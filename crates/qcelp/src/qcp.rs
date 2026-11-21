//! The QCP (RFC 3625) file reader, a port of FFmpeg n7.1.1 `libavformat/qcp.c` (Copyright (c)
//! 2009 Kenan Gillet; LGPL-2.1-or-later), with FFmpeg's leniency: a rate byte it cannot map is
//! skipped, a short last packet is delivered as it is.

use crate::Error;

/// The first byte of the QCELP-13K codec GUID can be 0x41 or 0x42; these are the other 15.
const GUID_QCELP_13K_PART: [u8; 15] = [0x6d, 0x7f, 0x5e, 0x15, 0xb1, 0xd0, 0x11, 0xba, 0x91, 0x00, 0x80, 0x5f, 0xb4, 0xb9, 0x7e];
const GUID_EVRC: [u8; 16] = [0x8d, 0xd4, 0x89, 0xe6, 0x76, 0x90, 0xb5, 0x46, 0x91, 0xef, 0x73, 0x6a, 0x51, 0x00, 0xce, 0xb4];
const GUID_4GV: [u8; 16] = [0xca, 0x29, 0xfd, 0x3c, 0x53, 0xf6, 0xf5, 0x4e, 0x90, 0xe9, 0xf4, 0x23, 0x6d, 0x59, 0x9b, 0x61];
const GUID_SMV: [u8; 16] = [0x75, 0x2b, 0x7c, 0x8d, 0x97, 0xa7, 0x49, 0xed, 0x98, 0x5e, 0xd5, 0x3c, 0x8c, 0xc7, 0x5f, 0x84];

const MAX_MODE: usize = 4;

/// What a QCP file carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcpCodec {
    /// QCELP-13K, which this crate decodes.
    Qcelp13k,
    /// EVRC.
    Evrc,
    /// SMV.
    Smv,
    /// 4GV.
    FourGv,
}

/// A parsed QCP file: its codec, sample rate and packets (each is what FFmpeg's demuxer hands to
/// the decoder: the data after the mode byte).
#[derive(Debug, Clone)]
pub struct Qcp<'a> {
    pub codec: QcpCodec,
    pub bit_rate: u32,
    pub sample_rate: u32,
    pub packets: Vec<&'a [u8]>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }
    fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
    }
    /// A byte, 0 past the end (as `avio_r8`).
    fn u8(&mut self) -> u8 {
        let v = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        v
    }
    fn le(&mut self, n: usize) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            v |= u32::from(self.u8()) << (8 * i);
        }
        v
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.data.len());
        let end = self.pos.saturating_add(n).min(self.data.len());
        self.pos = self.pos.saturating_add(n);
        &self.data[start..end]
    }
}

/// True if `data` starts like a QCP file (`qcp_probe`).
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 16 && &data[0..4] == b"RIFF" && &data[8..16] == b"QLCMfmt "
}

/// Parses a QCP file (`qcp_read_header`, `qcp_read_packet`).
pub fn parse(data: &[u8]) -> Result<Qcp<'_>, Error> {
    if !probe(data) {
        return Err(Error::NotQcp);
    }
    let mut r = Reader { data, pos: 4 };
    // filesize + "QLCMfmt " + chunk-size + major-version + minor-version
    r.skip(4 + 8 + 4 + 1 + 1);
    let guid = r.take(16);
    if guid.len() != 16 {
        return Err(Error::Truncated);
    }
    let codec = if (guid[0] == 0x41 || guid[0] == 0x42) && guid[1..] == GUID_QCELP_13K_PART {
        QcpCodec::Qcelp13k
    } else if guid == GUID_EVRC {
        QcpCodec::Evrc
    } else if guid == GUID_SMV {
        QcpCodec::Smv
    } else if guid == GUID_4GV {
        QcpCodec::FourGv
    } else {
        return Err(Error::UnknownCodec);
    };
    r.skip(2 + 80); // codec-version + codec-name
    let bit_rate = r.le(2);
    let mut packet_size = r.le(2) as usize;
    r.skip(2); // block-size
    let sample_rate = r.le(2);
    r.skip(2); // sample-size

    let mut rates_per_mode: [i32; MAX_MODE + 1] = [-1; MAX_MODE + 1];
    let nb_rates = r.le(4).min(8) as usize;
    for _ in 0..nb_rates {
        let size = i32::from(r.u8());
        let mode = usize::from(r.u8());
        if mode <= MAX_MODE {
            rates_per_mode[mode] = size;
        }
    }
    r.skip(16 - 2 * nb_rates + 20); // empty entries of rate-map-table + reserved

    let mut packets = Vec::new();
    let mut data_size = 0u32;
    while !r.eof() {
        if data_size != 0 {
            let mode = usize::from(r.u8());
            let mut pkt_size;
            if packet_size != 0 {
                pkt_size = packet_size as i64 - 1;
            } else if mode > MAX_MODE || rates_per_mode[mode] < 0 {
                data_size -= 1;
                continue;
            } else {
                pkt_size = i64::from(rates_per_mode[mode]);
            }
            if i64::from(data_size) <= pkt_size {
                pkt_size = i64::from(data_size) - 1; // "Data chunk is too small."
            }
            let want = usize::try_from(pkt_size).unwrap_or(0);
            let pkt = r.take(want);
            data_size = data_size.wrapping_sub(u32::try_from(pkt_size + 1).unwrap_or(0));
            if !pkt.is_empty() {
                packets.push(pkt);
            }
            continue;
        }

        if r.pos & 1 == 1 {
            r.u8(); // padding
        }
        let tag = r.le(4);
        let chunk_size = r.le(4);
        match &tag.to_le_bytes() {
            b"vrat" => {
                if r.le(4) != 0 {
                    packet_size = 0; // var-rate-flag
                }
                r.skip(4); // size-in-packets
            }
            b"data" => data_size = chunk_size,
            _ => r.skip(chunk_size as usize),
        }
    }

    Ok(Qcp { codec, bit_rate, sample_rate, packets })
}
