//! RealMedia (`.rm`, `.rmvb`) demuxer.
//!
//! A port of the RealMedia part of FFmpeg's `libavformat/rmdec.c` (n7.1.1, LGPL-2.1-or-later):
//! the `.RMF` header chunks (`PROP`, `MDPR`, `CONT`, `DATA`, `INDX`), the `VIDO` type-specific
//! data of video streams, the data packet sync (`rm_sync`) and the reassembly of sliced video
//! frames (`rm_assemble_video_frame`). Video packets come out in the layout FFmpeg's RealVideo
//! decoders take: one byte with the slice count minus one, eight bytes per slice
//! (`1u32` LE, then the slice's LE byte offset) and the concatenated slice data.
//!
//! Audio streams are listed with their raw type-specific data and their packets are returned
//! as stored (no deinterleaving yet).
//!
//! Every number in the file is treated as hostile: sizes are capped and checked, a damaged
//! packet is skipped and the reader resynchronises on the next packet header.

use std::io::{ErrorKind, Read, Seek, SeekFrom};

/// Errors of the demuxer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a RealMedia file")]
    NotRealMedia,
    #[error("invalid RealMedia data: {0}")]
    Invalid(&'static str),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Largest type-specific data or video frame we accept (FFmpeg caps extradata at 16 MiB).
const MAX_ALLOC: usize = 1 << 24;
/// Most streams we keep.
const MAX_STREAMS: usize = 64;

/// What a stream carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamKind {
    /// A `VIDO` stream.
    Video {
        /// FourCC as stored (`RV10`, `RV20`, `RV30`, `RV40`, …), read little-endian like FFmpeg.
        fourcc: [u8; 4],
        width: u16,
        height: u16,
        /// Frame rate in 16.16 fixed point (0 when unknown).
        fps_16_16: u32,
        /// Codec extradata (the bytes after the frame rate).
        extradata: Vec<u8>,
    },
    /// An audio stream (`.ra\xfd` or `LSD:` header): its raw type-specific data.
    Audio { codec_data: Vec<u8> },
    /// Anything else (logical streams, unknown types).
    Other,
}

/// One `MDPR` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    /// The stream number used by data packets.
    pub id: u32,
    pub mime: String,
    pub start_time_ms: u32,
    pub duration_ms: u32,
    pub kind: StreamKind,
}

/// One `INDX` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub timestamp_ms: u32,
    /// File offset of the data packet.
    pub pos: u32,
}

/// The parsed file header.
#[derive(Debug, Clone, Default)]
pub struct Header {
    pub streams: Vec<Stream>,
    pub duration_ms: u32,
    /// File offset of the first data packet.
    pub data_start: u64,
    /// `INDX` entries per stream (same order as `streams`); empty without an index.
    pub index: Vec<Vec<IndexEntry>>,
}

/// One demuxed packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Index into [`Header::streams`].
    pub stream: usize,
    /// Timestamp of the data packet that started this packet, in milliseconds.
    pub timestamp_ms: u32,
    /// The data packet that started this packet was flagged as a keyframe.
    pub keyframe: bool,
    /// File offset of that data packet.
    pub pos: u64,
    /// A video frame whose first slices were not read (it began before the position the reader
    /// was moved to with [`Demuxer::seek`]).
    pub partial: bool,
    pub data: Vec<u8>,
}

/// Whether `data` starts like a RealMedia file.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 6 && &data[..4] == b".RMF" && data[4] == 0 && data[5] == 0
}

/// A byte reader with FFmpeg's `avio` conventions: reads past the end return zeros and set `eof`.
struct Io<R> {
    r: R,
    pos: u64,
    eof: bool,
}

impl<R: Read + Seek> Io<R> {
    fn new(mut r: R) -> Result<Self> {
        let pos = r.stream_position()?;
        Ok(Self { r, pos, eof: false })
    }

    fn fill(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            let Some(rest) = buf.get_mut(n..) else { break };
            match self.r.read(rest) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(k) => n += k,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.pos = self.pos.saturating_add(n as u64);
        if let Some(rest) = buf.get_mut(n..) {
            rest.fill(0);
        }
        Ok(n)
    }

    fn r8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.fill(&mut b)?;
        Ok(b[0])
    }

    fn rb16(&mut self) -> Result<u16> {
        let mut b = [0u8; 2];
        self.fill(&mut b)?;
        Ok(u16::from_be_bytes(b))
    }

    fn rb32(&mut self) -> Result<u32> {
        let mut b = [0u8; 4];
        self.fill(&mut b)?;
        Ok(u32::from_be_bytes(b))
    }

    fn rl32(&mut self) -> Result<u32> {
        let mut b = [0u8; 4];
        self.fill(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn seek(&mut self, pos: u64) -> Result<()> {
        self.pos = self.r.seek(SeekFrom::Start(pos))?;
        self.eof = false;
        Ok(())
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        let to = self.pos.saturating_add(n);
        self.seek(to)
    }

    fn read_vec(&mut self, n: usize) -> Result<Vec<u8>> {
        if n > MAX_ALLOC {
            return Err(Error::Invalid("chunk too large"));
        }
        let mut v = vec![0u8; n];
        let got = self.fill(&mut v)?;
        if got != n {
            return Err(Error::Invalid("truncated data"));
        }
        Ok(v)
    }

    /// `get_str8`.
    fn str8(&mut self) -> Result<String> {
        let n = usize::from(self.r8()?);
        let v = self.read_vec(n)?;
        let end = v.iter().position(|&b| b == 0).unwrap_or(v.len());
        Ok(String::from_utf8_lossy(v.get(..end).unwrap_or(&[])).into_owned())
    }
}

const fn tag(b: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*b)
}

/// `ff_rm_read_mdpr_codecdata` (and the first entry of `rm_read_multi`) on the type-specific data.
fn parse_codec_data(data: &[u8], mime: &str) -> StreamKind {
    if mime == "logical-fileinfo" {
        return StreamKind::Other;
    }
    let be16 = |o: usize| -> Option<u16> { Some(u16::from_be_bytes(data.get(o..o + 2)?.try_into().ok()?)) };
    let be32 = |o: usize| -> Option<u32> { Some(u32::from_be_bytes(data.get(o..o + 4)?.try_into().ok()?)) };
    let Some(head) = data.get(..4) else { return StreamKind::Other };
    if head == b"MLTI" {
        // Number of rule-to-substream entries, the entries, the substream count, then the first
        // substream's size and data (FFmpeg reads the others as data streams).
        let Some(n) = be16(4) else { return StreamKind::Other };
        let at = 6 + 2 * usize::from(n);
        let Some(size) = be32(at + 2) else { return StreamKind::Other };
        let start = at + 6;
        let end = start.saturating_add(size as usize);
        return match data.get(start..end) {
            Some(sub) if sub.get(..4) != Some(b"MLTI".as_slice()) => parse_codec_data(sub, mime),
            _ => StreamKind::Other,
        };
    }
    if head == b".ra\xfd" || head == b"LSD:" {
        return StreamKind::Audio { codec_data: data.to_vec() };
    }
    if data.get(4..8) != Some(b"VIDO".as_slice()) || data.len() < 26 {
        return StreamKind::Other;
    }
    let mut fourcc = [0u8; 4];
    fourcc.copy_from_slice(data.get(8..12).unwrap_or(&[0; 4]));
    StreamKind::Video {
        fourcc,
        width: be16(12).unwrap_or(0),
        height: be16(14).unwrap_or(0),
        fps_16_16: be32(22).unwrap_or(0),
        extradata: data.get(26..).unwrap_or(&[]).to_vec(),
    }
}

/// Parses the header chunks up to `DATA` (and the `INDX` chunks, when the file has them).
fn read_header<R: Read + Seek>(io: &mut Io<R>) -> Result<Header> {
    if io.rl32()? != tag(b".RMF") {
        return Err(Error::NotRealMedia);
    }
    let first = io.rb32()?;
    if !(8..=1 << 20).contains(&first) {
        return Err(Error::Invalid(".RMF chunk size"));
    }
    io.skip(u64::from(first) - 8)?;
    let mut h = Header::default();
    let mut indx_off = 0u32;
    loop {
        if io.eof {
            return Err(Error::Invalid("no DATA chunk"));
        }
        let t = io.rl32()?;
        let size = io.rb32()?;
        io.rb16()?;
        if size < 10 && t != tag(b"DATA") {
            return Err(Error::Invalid("chunk size"));
        }
        let chunk_start = io.pos.saturating_sub(10);
        match t {
            t if t == tag(b"PROP") => {
                for _ in 0..5 {
                    io.rb32()?;
                }
                h.duration_ms = io.rb32()?;
                io.rb32()?;
                indx_off = io.rb32()?;
                io.rb32()?; // data offset
                io.rb16()?;
                io.rb16()?;
            }
            t if t == tag(b"MDPR") => {
                let id = u32::from(io.rb16()?);
                for _ in 0..4 {
                    io.rb32()?;
                }
                let start = io.rb32()?;
                io.rb32()?;
                let duration = io.rb32()?;
                io.str8()?;
                let mime = io.str8()?;
                let size = usize::try_from(io.rb32()?).map_err(|_| Error::Invalid("codec data size"))?;
                // Like FFmpeg, continue right after the type-specific data.
                let data = io.read_vec(size)?;
                if h.streams.len() < MAX_STREAMS {
                    let kind = parse_codec_data(&data, &mime);
                    h.streams.push(Stream { id, mime, start_time_ms: start, duration_ms: duration, kind });
                }
            }
            t if t == tag(b"DATA") => break,
            _ => io.seek(chunk_start.saturating_add(u64::from(size)))?,
        }
    }
    io.rb32()?; // number of packets
    io.rb32()?; // next data header
    h.data_start = io.pos;
    h.index = vec![Vec::new(); h.streams.len()];
    if indx_off != 0 {
        // A damaged index is ignored: seeking then scans packets.
        if read_index(io, u64::from(indx_off), &mut h).is_err() {
            h.index.iter_mut().for_each(Vec::clear);
        }
        io.seek(h.data_start)?;
    }
    Ok(h)
}

/// `rm_read_index`.
fn read_index<R: Read + Seek>(io: &mut Io<R>, off: u64, h: &mut Header) -> Result<()> {
    io.seek(off)?;
    for _ in 0..MAX_STREAMS {
        if io.rl32()? != tag(b"INDX") {
            return Err(Error::Invalid("INDX"));
        }
        let size = io.rb32()?;
        if size < 20 {
            return Err(Error::Invalid("INDX size"));
        }
        io.rb16()?;
        let n = io.rb32()?;
        let id = u32::from(io.rb16()?);
        let next = io.rb32()?;
        if let Some(si) = h.streams.iter().position(|s| s.id == id) {
            let n = usize::try_from(n).map_err(|_| Error::Invalid("INDX count"))?;
            if n > MAX_ALLOC / 14 {
                return Err(Error::Invalid("INDX count"));
            }
            let raw = io.read_vec(n * 14)?;
            let list = h.index.get_mut(si).ok_or(Error::Invalid("INDX stream"))?;
            for e in raw.chunks_exact(14) {
                let ts = u32::from_be_bytes([e[2], e[3], e[4], e[5]]);
                let pos = u32::from_be_bytes([e[6], e[7], e[8], e[9]]);
                list.push(IndexEntry { timestamp_ms: ts, pos });
            }
        }
        if next == 0 {
            return Ok(());
        }
        if u64::from(next) < io.pos {
            return Err(Error::Invalid("non-linear index"));
        }
        io.seek(u64::from(next))?;
    }
    Ok(())
}

/// Reassembly state of one video stream (`RMStream`).
#[derive(Debug, Default)]
struct VideoState {
    buf: Vec<u8>,
    slices: usize,
    cur_slice: usize,
    bufpos: usize,
    curpic_num: Option<u8>,
    timestamp_ms: u32,
    keyframe: bool,
    pos: u64,
    partial: bool,
}

/// The data packet reader.
pub struct Demuxer<R> {
    io: Io<R>,
    header: Header,
    remaining_len: usize,
    current_stream: usize,
    current_ts: u32,
    current_flags: u8,
    current_pos: u64,
    video: Vec<VideoState>,
}

impl<R: Read + Seek> Demuxer<R> {
    /// Reads the header; the reader is then at the first data packet.
    pub fn new(r: R) -> Result<Self> {
        let mut io = Io::new(r)?;
        let header = read_header(&mut io)?;
        let video = header.streams.iter().map(|_| VideoState::default()).collect();
        Ok(Self { io, header, remaining_len: 0, current_stream: 0, current_ts: 0, current_flags: 0, current_pos: 0, video })
    }

    /// A reader over the same file with a header read before (no header parsing), positioned at
    /// the first data packet.
    pub fn from_header(r: R, header: Header) -> Result<Self> {
        let mut io = Io::new(r)?;
        io.seek(header.data_start)?;
        let video = header.streams.iter().map(|_| VideoState::default()).collect();
        Ok(Self { io, header, remaining_len: 0, current_stream: 0, current_ts: 0, current_flags: 0, current_pos: 0, video })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Continue reading at the data packet at file offset `pos` (from the index or from
    /// [`Packet::pos`]); partial frames are dropped.
    pub fn seek(&mut self, pos: u64) -> Result<()> {
        self.io.seek(pos)?;
        self.remaining_len = 0;
        for v in &mut self.video {
            *v = VideoState::default();
        }
        Ok(())
    }

    /// `rm_sync`: the next data packet header. Returns (stream index, payload length).
    fn sync(&mut self) -> Result<Option<(usize, usize)>> {
        let mut state: u32 = 0xFFFF_FFFF;
        while !self.io.eof {
            if self.remaining_len > 0 {
                let len = self.remaining_len;
                self.current_flags = 0;
                return Ok(Some((self.current_stream, len)));
            }
            let pos = self.io.pos.saturating_sub(3);
            state = (state << 8) | u32::from(self.io.r8()?);
            if self.io.eof {
                break;
            }
            if state == u32::from_be_bytes(*b"INDX") {
                let len = self.io.rb32()?;
                self.io.rb16()?;
                let n = self.io.rb32()?;
                let expected = 20u64.saturating_add(u64::from(n).saturating_mul(14));
                let len = if len == 20 { expected } else { u64::from(len) };
                if len >= 14 {
                    self.io.skip(len - 14)?;
                }
                state = 0xFFFF_FFFF;
                continue;
            }
            if state > 0xFFFF || state <= 12 {
                continue;
            }
            let len = (state - 12) as usize;
            state = 0xFFFF_FFFF;
            let num = u32::from(self.io.rb16()?);
            let ts = self.io.rb32()?;
            let group = i32::from(self.io.r8()? >> 1) - 1;
            let mlti = (group.max(0) as u32) << 16;
            let flags = self.io.r8()?;
            let id = mlti.wrapping_add(num);
            match self.header.streams.iter().position(|s| s.id == id) {
                Some(i) => {
                    self.current_ts = ts;
                    self.current_flags = flags;
                    self.current_pos = pos;
                    return Ok(Some((i, len)));
                }
                None => self.io.skip(len as u64)?,
            }
        }
        Ok(None)
    }

    /// The next packet, or `None` at the end of the data.
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        // Bounded so a file of nothing but damaged packets still ends.
        loop {
            let Some((si, len)) = self.sync()? else { return Ok(None) };
            let is_video = matches!(self.header.streams.get(si).map(|s| &s.kind), Some(StreamKind::Video { .. }));
            if !is_video {
                self.remaining_len = 0;
                let data = self.io.read_vec(len)?;
                return Ok(Some(Packet {
                    stream: si,
                    timestamp_ms: self.current_ts,
                    keyframe: self.current_flags & 2 != 0,
                    pos: self.current_pos,
                    partial: false,
                    data,
                }));
            }
            self.current_stream = si;
            match self.assemble(si, len) {
                Ok(Some(p)) => return Ok(Some(p)),
                Ok(None) => {}
                Err(Error::Io(e)) => return Err(Error::Io(e)),
                Err(_) => {
                    // Damaged packet: drop what is left of it and resynchronise.
                    let left = self.remaining_len;
                    self.remaining_len = 0;
                    self.io.skip(left as u64)?;
                }
            }
            if self.io.eof {
                return Ok(None);
            }
        }
    }

    /// `get_num`.
    fn get_num(&mut self, len: &mut i64) -> Result<u32> {
        let n = u32::from(self.io.rb16()?) & 0x7FFF;
        *len -= 2;
        if n >= 0x4000 {
            Ok(n - 0x4000)
        } else {
            let n1 = u32::from(self.io.rb16()?);
            *len -= 2;
            Ok((n << 16) | n1)
        }
    }

    /// `rm_assemble_video_frame`.
    fn assemble(&mut self, si: usize, len: usize) -> Result<Option<Packet>> {
        let mut len = len as i64;
        let hdr = self.io.r8()?;
        len -= 1;
        let ty = hdr >> 6;
        let mut seq = 0u8;
        let (mut len2, mut pos, mut pic_num) = (0u32, 0u32, 0u8);
        if ty != 3 {
            seq = self.io.r8()?;
            len -= 1;
        }
        if ty != 1 {
            len2 = self.get_num(&mut len)?;
            pos = self.get_num(&mut len)?;
            pic_num = self.io.r8()?;
            len -= 1;
        }
        if len < 0 {
            self.remaining_len = 0;
            return Err(Error::Invalid("video packet header"));
        }
        let mut remaining = len as usize;
        let ts = self.current_ts;
        let key = self.current_flags & 2 != 0;
        let ppos = self.current_pos;
        if ty & 1 == 1 {
            let mut n = remaining;
            let mut ts = ts;
            if ty == 3 {
                n = len2 as usize;
                ts = pos;
            }
            if remaining < n {
                self.remaining_len = remaining;
                return Err(Error::Invalid("frame larger than packet"));
            }
            remaining -= n;
            self.remaining_len = remaining;
            let body = self.io.read_vec(n)?;
            let mut data = Vec::with_capacity(n + 9);
            data.push(0);
            data.extend_from_slice(&1u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&body);
            return Ok(Some(Packet { stream: si, timestamp_ms: ts, keyframe: key, pos: ppos, partial: false, data }));
        }
        let v = self.video.get_mut(si).ok_or(Error::Invalid("stream"))?;
        if seq & 0x7F == 1 || v.curpic_num != Some(pic_num) {
            let len2 = len2 as usize;
            if len2 > MAX_ALLOC {
                self.remaining_len = remaining;
                return Err(Error::Invalid("video frame too large"));
            }
            v.slices = ((usize::from(hdr) & 0x3F) << 1) + 1;
            let size = len2 + 8 * v.slices + 1;
            v.buf = vec![0u8; size];
            v.bufpos = 8 * v.slices + 1;
            v.cur_slice = 0;
            v.curpic_num = Some(pic_num);
            v.timestamp_ms = ts;
            v.keyframe = key;
            v.pos = ppos;
            v.partial = seq & 0x7F != 1;
        }
        let mut n = remaining;
        if ty == 2 {
            n = n.min(pos as usize);
        }
        v.cur_slice += 1;
        if v.cur_slice > v.slices || v.buf.is_empty() {
            self.remaining_len = remaining;
            return Err(Error::Invalid("too many slices"));
        }
        let entry = 8 * v.cur_slice - 7;
        let off = (v.bufpos - 8 * v.slices - 1) as u32;
        if let Some(e) = v.buf.get_mut(entry..entry + 8) {
            e[..4].copy_from_slice(&1u32.to_le_bytes());
            e[4..].copy_from_slice(&off.to_le_bytes());
        }
        if v.bufpos + n > v.buf.len() {
            self.remaining_len = remaining;
            return Err(Error::Invalid("slice outside frame"));
        }
        let start = v.bufpos;
        let got = {
            let dst = v.buf.get_mut(start..start + n).ok_or(Error::Invalid("slice"))?;
            self.io.fill(dst)?
        };
        let v = self.video.get_mut(si).ok_or(Error::Invalid("stream"))?;
        if got != n {
            self.remaining_len = 0;
            return Err(Error::Invalid("truncated slice"));
        }
        v.bufpos += n;
        remaining -= n;
        self.remaining_len = remaining;
        if ty == 2 || v.bufpos == v.buf.len() {
            let mut data = std::mem::take(&mut v.buf);
            if let Some(b) = data.first_mut() {
                *b = (v.cur_slice - 1) as u8;
            }
            if v.slices != v.cur_slice {
                let from = 1 + 8 * v.slices;
                let to = 1 + 8 * v.cur_slice;
                data.copy_within(from..v.bufpos, to);
            }
            data.truncate(v.bufpos + 8 * v.cur_slice - 8 * v.slices);
            v.slices = 0;
            return Ok(Some(Packet { stream: si, timestamp_ms: v.timestamp_ms, keyframe: v.keyframe, pos: v.pos, partial: v.partial, data }));
        }
        Ok(None)
    }
}
