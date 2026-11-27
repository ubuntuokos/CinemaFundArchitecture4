//! ST 2110-20:2022 §6: active video into RTP packets and back.
//!
//! An *image* is one progressive frame, or one field (segment) of an interlaced (PsF) frame, as
//! its transmitted sample rows back to back: each row is [`row_bytes`] long (whole pgroups, the
//! last one zero-filled), and for 4:2:0 a transmitted row holds two image rows (§6.2.5).
//!
//! Packet layout (§6.1.4): RTP header, 16-bit Extended Sequence Number, one to three 6-byte
//! Sample Row Data headers (length, F + row number, C + pixel offset), then the data segments.

use std::collections::VecDeque;

use crate::format::VideoFormat;
use crate::rtp::{RTP_HEADER_LEN, RtpHeader, RtpPacket};
use crate::video::{Depth, PGroup, PackingMode, Sampling, row_bytes};
use crate::{Error, UDP_HEADER_LEN};

/// Bytes of the Extended Sequence Number field.
const EXT_SEQ_LEN: usize = 2;
/// Bytes of one Sample Row Data header.
const SRD_LEN: usize = 6;
/// At most three SRD headers per packet (§6.2.1).
const MAX_SRDS: usize = 3;
/// Block Packing Mode: 7 blocks of 180 octets of sample data per packet (§6.3.3).
pub const BPM_PAYLOAD: usize = 7 * 180;

#[derive(Debug, Clone, Copy)]
struct Segment {
    row: usize,
    byte_offset: usize,
    len: usize,
}

/// Turns images into RTP packets in General or Block Packing Mode.
#[derive(Debug, Clone)]
pub struct Packetizer {
    format: VideoFormat,
    pg: PGroup,
    row_len: usize,
    payload_type: u8,
    ssrc: u32,
    /// Next extended (32-bit) sequence number.
    sequence: u32,
}

impl Packetizer {
    /// A packetizer for `format` (validated), starting at extended sequence number `first_seq`.
    pub fn new(format: VideoFormat, payload_type: u8, ssrc: u32, first_seq: u32) -> Result<Self, Error> {
        let pg = format.validate()?;
        if !(96..=127).contains(&payload_type) {
            return Err(Error::Invalid("payload type must be dynamic (96..=127)"));
        }
        if format.packing == PackingMode::Block && !BPM_PAYLOAD.is_multiple_of(pg.octets as usize) {
            return Err(Error::Unsupported(format!("Block Packing Mode with {}-octet pgroups", pg.octets)));
        }
        let row_len = row_bytes(format.width, pg);
        Ok(Packetizer { format, pg, row_len, payload_type, ssrc, sequence: first_seq })
    }

    /// Bytes of one image (frame or field) in the transmitted-row layout.
    pub fn image_len(&self, second_field: bool) -> usize {
        self.format.rows_per_image(second_field) as usize * self.row_len
    }

    /// The next extended sequence number.
    pub fn next_sequence(&self) -> u32 {
        self.sequence
    }

    /// Packetize one image with RTP timestamp `timestamp`, handing each packet to `emit`.
    /// Returns the number of packets; the last one carries the marker bit.
    pub fn packetize(&mut self, image: &[u8], timestamp: u32, second_field: bool, mut emit: impl FnMut(&[u8])) -> Result<usize, Error> {
        if second_field && !self.format.interlace {
            return Err(Error::Invalid("second field of a progressive format"));
        }
        let rows = self.format.rows_per_image(second_field) as usize;
        if image.len() != rows * self.row_len {
            return Err(Error::Invalid("image size does not match the format"));
        }
        let packets = self.plan(rows)?;
        let count = packets.len();
        let mut buf = Vec::with_capacity(self.format.udp_limit());
        for (i, segs) in packets.iter().enumerate() {
            buf.clear();
            RtpHeader { marker: i + 1 == count, payload_type: self.payload_type, sequence: self.sequence as u16, timestamp, ssrc: self.ssrc }.write(&mut buf);
            buf.extend_from_slice(&((self.sequence >> 16) as u16).to_be_bytes());
            for (k, s) in segs.iter().enumerate() {
                let row_no = if self.format.sampling.is_420() { 2 * s.row } else { s.row } as u16;
                let pixel = (s.byte_offset / self.pg.octets as usize * self.pg.pixels as usize) as u16;
                let cont = k + 1 < segs.len();
                buf.extend_from_slice(&(s.len as u16).to_be_bytes());
                buf.extend_from_slice(&(((second_field as u16) << 15) | row_no).to_be_bytes());
                buf.extend_from_slice(&(((cont as u16) << 15) | pixel).to_be_bytes());
            }
            for s in segs {
                let start = s.row * self.row_len + s.byte_offset;
                buf.extend_from_slice(&image[start..start + s.len]);
            }
            emit(&buf);
            self.sequence = self.sequence.wrapping_add(1);
        }
        Ok(count)
    }

    /// Split `rows` rows into packets of segments.
    fn plan(&self, rows: usize) -> Result<Vec<Vec<Segment>>, Error> {
        let octets = self.pg.octets as usize;
        let max_rtp = self.format.udp_limit() - UDP_HEADER_LEN;
        let block = self.format.packing == PackingMode::Block;
        let mut packets = Vec::new();
        let (mut row, mut pos) = (0usize, 0usize);
        while row < rows {
            let mut segs: Vec<Segment> = Vec::with_capacity(MAX_SRDS);
            let mut data = 0usize;
            loop {
                let headers = RTP_HEADER_LEN + EXT_SEQ_LEN + SRD_LEN * (segs.len() + 1);
                let room = if block { BPM_PAYLOAD - data } else { max_rtp.saturating_sub(headers + data) / octets * octets };
                if room == 0 {
                    break;
                }
                let len = room.min(self.row_len - pos);
                segs.push(Segment { row, byte_offset: pos, len });
                data += len;
                pos += len;
                if pos < self.row_len {
                    break; // the packet is full
                }
                row += 1;
                pos = 0;
                if row == rows {
                    break;
                }
                if segs.len() == MAX_SRDS {
                    if block && data < BPM_PAYLOAD {
                        return Err(Error::Unsupported("Block Packing Mode with rows this short needs more than 3 SRD headers".into()));
                    }
                    break;
                }
            }
            if segs.is_empty() {
                return Err(Error::Invalid("UDP size limit too small for one pgroup"));
            }
            packets.push(segs);
        }
        Ok(packets)
    }
}

/// One received image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub timestamp: u32,
    pub second_field: bool,
    /// Transmitted rows back to back; missing parts are zero.
    pub data: Vec<u8>,
    /// Sample data bytes received (counting each packet once).
    pub received: usize,
    /// Every byte arrived and the marker packet was seen.
    pub complete: bool,
}

/// Reassembles images from -20 packets. Malformed packets are rejected with an error and leave the
/// image in progress untouched; lost packets leave zeros and mark the image incomplete.
#[derive(Debug, Clone)]
pub struct Depacketizer {
    format: VideoFormat,
    pg: PGroup,
    row_len: usize,
    current: Option<Image>,
    last_sequence: Option<u32>,
    /// Byte position after the last data written into the current image: sample data arrives
    /// top to bottom, left to right (§6.1.4–6.1.5), so a segment starting before it is refused.
    write_end: usize,
    lost_packets: u64,
    ready: VecDeque<Image>,
}

impl Depacketizer {
    pub fn new(format: VideoFormat) -> Result<Self, Error> {
        let pg = format.validate()?;
        let row_len = row_bytes(format.width, pg);
        Ok(Depacketizer { format, pg, row_len, current: None, last_sequence: None, write_end: 0, lost_packets: 0, ready: VecDeque::new() })
    }

    /// Packets missing from the sequence so far.
    pub fn lost_packets(&self) -> u64 {
        self.lost_packets
    }

    /// The next finished image, oldest first.
    pub fn pop(&mut self) -> Option<Image> {
        self.ready.pop_front()
    }

    /// Hand over an image still in progress (end of stream).
    pub fn flush(&mut self) {
        if let Some(mut img) = self.current.take() {
            img.complete = false;
            self.ready.push_back(img);
        }
    }

    /// Feed one RTP datagram.
    pub fn push(&mut self, datagram: &[u8]) -> Result<(), Error> {
        let p = RtpPacket::parse(datagram)?;
        let segs = self.parse_segments(p.payload)?;
        let low = p.header.sequence;
        let high = u16::from_be_bytes([p.payload[0], p.payload[1]]);
        let seq = crate::rtp::extended_sequence(high, low);
        if let Some(last) = self.last_sequence {
            let d = seq.wrapping_sub(last);
            if d == 0 || d > u32::MAX / 2 {
                return Ok(()); // duplicate or late packet: already counted as lost
            }
            if d > 1 {
                self.lost_packets += u64::from(d - 1);
            }
        }
        self.last_sequence = Some(seq);

        let second_field = segs.first().is_some_and(|s| s.0);
        if self.current.as_ref().is_some_and(|img| img.timestamp != p.header.timestamp || img.second_field != second_field) {
            // The marker packet of the previous image was lost.
            self.flush();
        }
        if self.current.is_none() {
            self.write_end = 0;
        }
        let mut end = self.write_end;
        for &(_, seg) in &segs {
            let start = seg.row * self.row_len + seg.byte_offset;
            if start < end {
                return Err(Error::Invalid("SRD out of order or overlapping"));
            }
            end = start + seg.len;
        }
        self.write_end = end;
        let rows = self.format.rows_per_image(second_field) as usize;
        let img = self.current.get_or_insert_with(|| Image {
            timestamp: p.header.timestamp,
            second_field,
            data: vec![0; rows * self.row_len],
            received: 0,
            complete: false,
        });
        let mut at = EXT_SEQ_LEN + SRD_LEN * segs.len();
        for &(_, seg) in &segs {
            let start = seg.row * self.row_len + seg.byte_offset;
            let (Some(dst), Some(src)) = (img.data.get_mut(start..start + seg.len), p.payload.get(at..at + seg.len)) else {
                return Err(Error::Invalid("SRD outside the image"));
            };
            dst.copy_from_slice(src);
            img.received += seg.len;
            at += seg.len;
        }
        if p.header.marker
            && let Some(mut img) = self.current.take()
        {
            // Segments never overlap (checked above), so every byte counted is a distinct one.
            img.complete = img.received == img.data.len();
            self.ready.push_back(img);
        }
        Ok(())
    }

    /// Parse and bound-check the SRD headers: (field bit, segment) per header.
    fn parse_segments(&self, payload: &[u8]) -> Result<Vec<(bool, Segment)>, Error> {
        let octets = self.pg.octets as usize;
        let pixels = self.pg.pixels as usize;
        let mut out = Vec::with_capacity(MAX_SRDS);
        let mut at = EXT_SEQ_LEN;
        loop {
            let h = payload.get(at..at + SRD_LEN).ok_or(Error::Truncated("SRD header"))?;
            let len = u16::from_be_bytes([h[0], h[1]]) as usize;
            let field = h[2] & 0x80 != 0;
            let row_no = (u16::from_be_bytes([h[2], h[3]]) & 0x7FFF) as usize;
            let cont = h[4] & 0x80 != 0;
            let pixel = (u16::from_be_bytes([h[4], h[5]]) & 0x7FFF) as usize;
            at += SRD_LEN;
            if field && !self.format.interlace {
                return Err(Error::Invalid("F bit set in a progressive stream"));
            }
            let row = if self.format.sampling.is_420() {
                if !row_no.is_multiple_of(2) {
                    return Err(Error::Invalid("odd SRD row number in 4:2:0"));
                }
                row_no / 2
            } else {
                row_no
            };
            if !len.is_multiple_of(octets) || !pixel.is_multiple_of(pixels) {
                return Err(Error::Invalid("SRD not aligned to pgroups"));
            }
            let byte_offset = pixel / pixels * octets;
            if row >= self.format.rows_per_image(field) as usize || byte_offset + len > self.row_len {
                return Err(Error::Invalid("SRD outside the image"));
            }
            if len == 0 && (cont || !out.is_empty()) {
                return Err(Error::Invalid("zero SRD length with more than one header"));
            }
            out.push((field, Segment { row, byte_offset, len }));
            if !cont {
                break;
            }
            if out.len() == MAX_SRDS {
                return Err(Error::Invalid("more than three SRD headers"));
            }
        }
        if out.iter().any(|(f, _)| *f != out[0].0) {
            return Err(Error::Invalid("SRDs of both fields in one packet"));
        }
        let data: usize = out.iter().map(|(_, s)| s.len).sum();
        if payload.len() < at + data {
            return Err(Error::Truncated("SRD data"));
        }
        Ok(out)
    }
}

/// Image planes with one `u16` per sample (the low `depth` bits are used): `[Y, Cb, Cr]` for
/// Y′C′BC′R and ICTCP (as `[I, CT, CP]`), `[R, G, B]`, `[X, Y, Z]`, or `[K]`. 4:2:2 colour planes
/// are half width (rounded up).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planes {
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Vec<u16>>,
}

/// Pack progressive planes into transmitted rows (the layout [`Packetizer::packetize`] takes).
/// 4:2:0 and 16f are not converted here (send their pgroup bytes directly).
pub fn planes_to_rows(format: &VideoFormat, p: &Planes) -> Result<Vec<u8>, Error> {
    let pg = format.validate()?;
    let order = sample_order(format)?;
    if format.interlace || p.width != format.width || p.height != format.height {
        return Err(Error::Invalid("planes do not match a progressive format"));
    }
    let cw = chroma_width(format.sampling, format.width) as usize;
    let w = format.width as usize;
    let expect = |i: usize| if format.sampling == Sampling::Key || i == 0 || cw == w { w } else { cw };
    if p.planes.len() != order.planes || p.planes.iter().enumerate().any(|(i, pl)| pl.len() != expect(i) * format.height as usize) {
        return Err(Error::Invalid("plane sizes do not match the format"));
    }
    let bits = format.depth.bits();
    let mask = ((1u32 << bits) - 1) as u16;
    let row_len = row_bytes(format.width, pg);
    let mut out = Vec::with_capacity(row_len * format.height as usize);
    for y in 0..format.height as usize {
        let mut bw = BitWriter::new(&mut out);
        for x0 in (0..w).step_by(pg.pixels as usize) {
            for &(plane, dx, chroma) in order.pgroup {
                let x = if chroma { x0 / 2 + dx } else { x0 + dx };
                let width = expect(plane);
                let v = if (chroma && x0 + 2 * dx < w) || (!chroma && x < w) { p.planes[plane][y * width + x] & mask } else { 0 };
                bw.put(v, bits);
            }
        }
    }
    Ok(out)
}

/// Unpack transmitted rows (progressive) into planes; the inverse of [`planes_to_rows`].
pub fn rows_to_planes(format: &VideoFormat, rows: &[u8]) -> Result<Planes, Error> {
    let pg = format.validate()?;
    let order = sample_order(format)?;
    if format.interlace {
        return Err(Error::Invalid("rows_to_planes takes a progressive frame"));
    }
    let row_len = row_bytes(format.width, pg);
    let h = format.height as usize;
    if rows.len() != row_len * h {
        return Err(Error::Invalid("row data does not match the format"));
    }
    let w = format.width as usize;
    let cw = chroma_width(format.sampling, format.width) as usize;
    let width_of = |i: usize| if format.sampling == Sampling::Key || i == 0 || cw == w { w } else { cw };
    let mut planes: Vec<Vec<u16>> = (0..order.planes).map(|i| vec![0; width_of(i) * h]).collect();
    let bits = format.depth.bits();
    for y in 0..h {
        let mut br = BitReader::new(&rows[y * row_len..(y + 1) * row_len]);
        for x0 in (0..w).step_by(pg.pixels as usize) {
            for &(plane, dx, chroma) in order.pgroup {
                let v = br.get(bits);
                let x = if chroma { x0 / 2 + dx } else { x0 + dx };
                let inside = if chroma { x0 + 2 * dx < w } else { x < w };
                if inside {
                    let width = width_of(plane);
                    planes[plane][y * width + x] = v;
                }
            }
        }
    }
    Ok(Planes { width: format.width, height: format.height, planes })
}

struct Order {
    planes: usize,
    /// Per sample in pgroup order: (plane, pixel or chroma index within the pgroup, is chroma).
    pgroup: &'static [(usize, usize, bool)],
}

fn sample_order(format: &VideoFormat) -> Result<Order, Error> {
    use Sampling::*;
    if format.depth == Depth::D16f {
        return Err(Error::Unsupported("plane conversion of 16f samples".into()));
    }
    // Planes: 0 = Y/I/R/X/K, 1 = Cb/CT/G/Y, 2 = Cr/CP/B/Z.
    const YCC1: [(usize, usize, bool); 3] = [(1, 0, false), (0, 0, false), (2, 0, false)];
    const YCC2: [(usize, usize, bool); 6] = [(1, 0, false), (0, 0, false), (2, 0, false), (1, 1, false), (0, 1, false), (2, 1, false)];
    const YCC4: [(usize, usize, bool); 12] = [
        (1, 0, false),
        (0, 0, false),
        (2, 0, false),
        (1, 1, false),
        (0, 1, false),
        (2, 1, false),
        (1, 2, false),
        (0, 2, false),
        (2, 2, false),
        (1, 3, false),
        (0, 3, false),
        (2, 3, false),
    ];
    const RGB1: [(usize, usize, bool); 3] = [(0, 0, false), (1, 0, false), (2, 0, false)];
    const RGB2: [(usize, usize, bool); 6] = [(0, 0, false), (1, 0, false), (2, 0, false), (0, 1, false), (1, 1, false), (2, 1, false)];
    const RGB4: [(usize, usize, bool); 12] = [
        (0, 0, false),
        (1, 0, false),
        (2, 0, false),
        (0, 1, false),
        (1, 1, false),
        (2, 1, false),
        (0, 2, false),
        (1, 2, false),
        (2, 2, false),
        (0, 3, false),
        (1, 3, false),
        (2, 3, false),
    ];
    const C422: [(usize, usize, bool); 4] = [(1, 0, true), (0, 0, false), (2, 0, true), (0, 1, false)];
    const K1: [(usize, usize, bool); 1] = [(0, 0, false)];
    const K2: [(usize, usize, bool); 2] = [(0, 0, false), (0, 1, false)];
    const K4: [(usize, usize, bool); 4] = [(0, 0, false), (0, 1, false), (0, 2, false), (0, 3, false)];
    let px = crate::video::pgroup(format.sampling, format.depth)?.pixels;
    let pick3 = |a: &'static [(usize, usize, bool)], b: &'static [(usize, usize, bool)], c: &'static [(usize, usize, bool)]| match px {
        1 => a,
        2 => b,
        _ => c,
    };
    Ok(match format.sampling {
        YCbCr444 | ClYCbCr444 | ICtCp444 => Order { planes: 3, pgroup: pick3(&YCC1, &YCC2, &YCC4) },
        Rgb | Xyz => Order { planes: 3, pgroup: pick3(&RGB1, &RGB2, &RGB4) },
        YCbCr422 | ClYCbCr422 | ICtCp422 => Order { planes: 3, pgroup: &C422 },
        Key => Order { planes: 1, pgroup: pick3(&K1, &K2, &K4) },
        YCbCr420 | ClYCbCr420 | ICtCp420 => return Err(Error::Unsupported("plane conversion of 4:2:0".into())),
    })
}

fn chroma_width(s: Sampling, width: u32) -> u32 {
    if matches!(s, Sampling::YCbCr422 | Sampling::ClYCbCr422 | Sampling::ICtCp422) { width.div_ceil(2) } else { width }
}

struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    acc: u32,
    n: u32,
}

impl<'a> BitWriter<'a> {
    fn new(out: &'a mut Vec<u8>) -> Self {
        BitWriter { out, acc: 0, n: 0 }
    }

    /// Append `bits` bits of `v`, most significant first (pgroups always end on a byte).
    fn put(&mut self, v: u16, bits: u32) {
        self.acc = (self.acc << bits) | u32::from(v);
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1 << self.n) - 1;
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, acc: 0, n: 0 }
    }

    fn get(&mut self, bits: u32) -> u16 {
        while self.n < bits {
            self.acc = (self.acc << 8) | u32::from(self.data.get(self.pos).copied().unwrap_or(0));
            self.pos += 1;
            self.n += 8;
        }
        self.n -= bits;
        let v = (self.acc >> self.n) & ((1 << bits) - 1);
        self.acc &= (1 << self.n) - 1;
        v as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{Colorimetry, FrameRate};

    fn fmt(s: Sampling, d: Depth, w: u32, h: u32) -> VideoFormat {
        let c = if s == Sampling::Key { Colorimetry::Alpha } else { Colorimetry::Bt709 };
        VideoFormat::progressive(s, d, w, h, FrameRate::new(50, 1).unwrap(), c)
    }

    fn pattern(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn round_trip(f: &VideoFormat) -> (Vec<Vec<u8>>, Image) {
        let mut tx = Packetizer::new(f.clone(), 98, 7, 0xFFFF_FFF0).unwrap();
        let mut img = pattern(tx.image_len(false), f.width);
        // Zero the fill of the last pgroup, as a sender must.
        let pg = crate::video::pgroup(f.sampling, f.depth).unwrap();
        let rl = row_bytes(f.width, pg);
        if !f.width.is_multiple_of(pg.pixels) {
            for r in 0..img.len() / rl {
                img[r * rl + rl - pg.octets as usize..(r + 1) * rl].fill(0);
            }
        }
        let mut packets = Vec::new();
        tx.packetize(&img, 1234, false, |p| packets.push(p.to_vec())).unwrap();
        let mut rx = Depacketizer::new(f.clone()).unwrap();
        for p in &packets {
            rx.push(p).unwrap();
        }
        let out = rx.pop().unwrap();
        assert_eq!(out.data, img);
        assert!(out.complete);
        assert_eq!(rx.lost_packets(), 0);
        (packets, out)
    }

    #[test]
    fn gpm_round_trip_1080p_422_10() {
        let f = fmt(Sampling::YCbCr422, Depth::D10, 1920, 1080);
        let (packets, _) = round_trip(&f);
        assert!(packets.iter().all(|p| p.len() + UDP_HEADER_LEN <= crate::STANDARD_UDP_LIMIT));
        // Full packets: 1452 − 12 − 2 − 6 = 1432 → 1430 bytes (286 pgroups) per single-row packet.
        assert_eq!(packets[0].len(), 12 + 2 + 6 + 1430);
        assert_eq!(packets.iter().filter(|p| p[1] & 0x80 != 0).count(), 1);
        assert!(packets.last().unwrap()[1] & 0x80 != 0);
        // The extended sequence number wrapped from 0xFFFF_FFFF to 0 inside the frame.
        assert!(packets.len() > 16);
    }

    #[test]
    fn gpm_packs_short_rows_with_continuation() {
        let f = fmt(Sampling::YCbCr422, Depth::D8, 64, 8); // 128-byte rows
        let (packets, _) = round_trip(&f);
        assert_eq!(packets.len(), 3); // 3 rows per packet (3 SRD limit), 8 rows
        assert_eq!(packets[0][14 + 4] & 0x80, 0x80); // first SRD has C=1
    }

    #[test]
    fn bpm_uses_1260_byte_payloads() {
        let mut f = fmt(Sampling::YCbCr422, Depth::D10, 1920, 1080);
        f.packing = PackingMode::Block;
        let (packets, _) = round_trip(&f);
        let data_len = |p: &Vec<u8>| {
            let mut at = 14;
            let mut n = 0;
            loop {
                n += u16::from_be_bytes([p[at], p[at + 1]]) as usize;
                let c = p[at + 4] & 0x80 != 0;
                at += 6;
                if !c {
                    break n;
                }
            }
        };
        assert!(packets[..packets.len() - 1].iter().all(|p| data_len(p) == BPM_PAYLOAD));
        assert_eq!(packets.len(), (4800 * 1080usize).div_ceil(BPM_PAYLOAD));
    }

    #[test]
    fn bpm_refuses_pgroups_that_do_not_divide_1260() {
        let mut f = fmt(Sampling::YCbCr422, Depth::D16, 1920, 1080);
        f.packing = PackingMode::Block;
        assert!(Packetizer::new(f, 98, 1, 0).is_err());
    }

    #[test]
    fn other_formats_round_trip() {
        round_trip(&fmt(Sampling::Rgb, Depth::D10, 1922, 4)); // partial last pgroup
        round_trip(&fmt(Sampling::Rgb, Depth::D8, 640, 4));
        round_trip(&fmt(Sampling::YCbCr420, Depth::D10, 64, 8));
        round_trip(&fmt(Sampling::Key, Depth::D12, 99, 3));
        let mut ext = fmt(Sampling::YCbCr422, Depth::D10, 3840, 4);
        ext.max_udp = Some(8960);
        let (packets, _) = round_trip(&ext);
        assert!(packets.iter().all(|p| p.len() + UDP_HEADER_LEN <= 8960));
        assert!(packets.len() <= 5, "38 400 bytes in jumbo packets: {}", packets.len());
    }

    #[test]
    fn interlaced_fields() {
        let mut f = fmt(Sampling::YCbCr422, Depth::D10, 720, 487);
        f.interlace = true;
        let mut tx = Packetizer::new(f.clone(), 98, 7, 0).unwrap();
        let a = pattern(tx.image_len(false), 1);
        let b = pattern(tx.image_len(true), 2);
        let mut packets = Vec::new();
        tx.packetize(&a, 100, false, |p| packets.push(p.to_vec())).unwrap();
        tx.packetize(&b, 100, true, |p| packets.push(p.to_vec())).unwrap();
        let mut rx = Depacketizer::new(f).unwrap();
        for p in &packets {
            rx.push(p).unwrap();
        }
        let (f1, f2) = (rx.pop().unwrap(), rx.pop().unwrap());
        assert_eq!((f1.second_field, f1.data.len(), f1.complete), (false, 244 * 1800, true));
        assert_eq!((f2.second_field, f2.data == b, f2.complete), (true, true, true));
        assert_eq!(f1.data, a);
    }

    #[test]
    fn loss_marks_the_image_incomplete() {
        let f = fmt(Sampling::YCbCr422, Depth::D10, 1920, 16);
        let mut tx = Packetizer::new(f.clone(), 98, 7, 0).unwrap();
        let img = pattern(tx.image_len(false), 3);
        let mut packets = Vec::new();
        tx.packetize(&img, 1, false, |p| packets.push(p.to_vec())).unwrap();
        tx.packetize(&img, 2, false, |p| packets.push(p.to_vec())).unwrap();
        let n = packets.len() / 2;
        let mut rx = Depacketizer::new(f).unwrap();
        for (i, p) in packets.iter().enumerate() {
            if i == 3 || i == n - 1 {
                continue; // a middle packet of image 1 and its marker packet
            }
            rx.push(p).unwrap();
        }
        let first = rx.pop().unwrap();
        assert!(!first.complete && first.timestamp == 1);
        let second = rx.pop().unwrap();
        assert!(second.complete && second.data == img);
        assert_eq!(rx.lost_packets(), 2);
        // A duplicate is ignored.
        rx.push(&packets[2 * n - 1]).unwrap();
        assert!(rx.pop().is_none());
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        let f = fmt(Sampling::YCbCr422, Depth::D10, 64, 4);
        let mut rx = Depacketizer::new(f).unwrap();
        let mut pkt = Vec::new();
        RtpHeader { marker: true, payload_type: 98, sequence: 0, timestamp: 0, ssrc: 0 }.write(&mut pkt);
        pkt.extend_from_slice(&[0, 0]);
        let with = |srd: &[u8], data: usize| {
            let mut p = pkt.clone();
            p.extend_from_slice(srd);
            p.extend(std::iter::repeat_n(0u8, data));
            p
        };
        assert!(rx.push(&with(&[0, 5, 0, 9, 0, 0], 5)).is_err()); // row 9 of 4
        assert!(rx.push(&with(&[0, 4, 0, 0, 0, 0], 4)).is_err()); // not a pgroup multiple
        assert!(rx.push(&with(&[0, 5, 0, 0, 0, 1], 5)).is_err()); // odd pixel offset
        assert!(rx.push(&with(&[0, 165, 0, 0, 0, 0], 165)).is_err()); // past the 160-byte row
        assert!(rx.push(&with(&[0, 10, 0, 0, 0, 0], 5)).is_err()); // data truncated
        assert!(rx.push(&with(&[0, 5, 0x80, 0, 0, 0], 5)).is_err()); // F bit, progressive
        assert!(rx.push(&with(&[0, 5, 0, 0, 0x80, 0, 0, 5, 0, 1, 0x80, 0, 0, 5, 0, 2, 0x80, 0], 15)).is_err()); // 4th SRD
        assert!(rx.push(&pkt).is_err()); // no SRD at all
        let mut ok = |seq: u16, srd: &[u8]| {
            let mut p = Vec::new();
            RtpHeader { marker: false, payload_type: 98, sequence: seq, timestamp: 5, ssrc: 0 }.write(&mut p);
            p.extend_from_slice(&[0, 0]);
            p.extend_from_slice(srd);
            p.extend(std::iter::repeat_n(0u8, 10));
            rx.push(&p)
        };
        assert!(ok(10, &[0, 10, 0, 1, 0, 0]).is_ok()); // row 1, pixels 0..4
        assert!(ok(11, &[0, 10, 0, 1, 0, 2]).is_err()); // overlaps it
        assert!(ok(12, &[0, 10, 0, 0, 0, 0]).is_err()); // goes back to row 0
        assert!(rx.push(&with(&[0, 0, 0, 0, 0, 0], 0)).is_ok()); // a single empty SRD is allowed
        assert_eq!(rx.lost_packets(), 0);
    }

    #[test]
    fn planes_round_trip() {
        for (s, d, w) in [
            (Sampling::YCbCr422, Depth::D10, 1921),
            (Sampling::YCbCr422, Depth::D8, 16),
            (Sampling::YCbCr444, Depth::D10, 7),
            (Sampling::Rgb, Depth::D12, 5),
            (Sampling::Rgb, Depth::D16, 3),
            (Sampling::Key, Depth::D10, 9),
        ] {
            let f = fmt(s, d, w, 3);
            let cw = if s == Sampling::YCbCr422 { w.div_ceil(2) } else { w };
            let n = if s == Sampling::Key { 1 } else { 3 };
            let mask = ((1u32 << d.bits()) - 1) as u16;
            let planes: Vec<Vec<u16>> = (0..n)
                .map(|i| {
                    let pw = if i == 0 { w } else { cw };
                    pattern(pw as usize * 6, i as u32 + w).chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]]) & mask).collect()
                })
                .collect();
            let p = Planes { width: w, height: 3, planes };
            let rows = planes_to_rows(&f, &p).unwrap();
            assert_eq!(rows.len(), 3 * row_bytes(w, crate::video::pgroup(s, d).unwrap()));
            assert_eq!(rows_to_planes(&f, &rows).unwrap(), p, "{s:?} {d:?} {w}");
        }
    }

    #[test]
    fn uyvp_bit_layout() {
        // One 4:2:2 10-bit pgroup: Cb=0x3FF, Y0=0, Cr=0x155, Y1=0x2AA → 40 bits MSB first.
        let f = fmt(Sampling::YCbCr422, Depth::D10, 2, 1);
        let p = Planes { width: 2, height: 1, planes: vec![vec![0, 0x2AA], vec![0x3FF], vec![0x155]] };
        assert_eq!(planes_to_rows(&f, &p).unwrap(), [0xFF, 0xC0, 0x05, 0x56, 0xAA]);
    }
}
