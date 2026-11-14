//! RTP fixed header (IETF RFC 3550 §5.1) with the ST 2110 constraints: version 2, dynamic payload
//! types, big-endian fields, header extensions (RFC 8285) tolerated and skipped (ST 2110-10:2022).

use crate::Error;

/// Size of the fixed RTP header without CSRCs or extensions.
pub const RTP_HEADER_LEN: usize = 12;

/// The fields of an RTP header that ST 2110 senders set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpHeader {
    /// Marker bit (for -20 video: last packet of a frame or field).
    pub marker: bool,
    /// Payload type, 0–127 (ST 2110 uses the dynamic range 96–127).
    pub payload_type: u8,
    /// Low 16 bits of the packet sequence counter.
    pub sequence: u16,
    /// RTP timestamp.
    pub timestamp: u32,
    /// Synchronisation source.
    pub ssrc: u32,
}

impl RtpHeader {
    /// Append the 12-byte header (no padding, no extension, no CSRCs) to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(0x80);
        out.push(((self.marker as u8) << 7) | (self.payload_type & 0x7F));
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.ssrc.to_be_bytes());
    }
}

/// A parsed RTP packet borrowing its payload from the datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpPacket<'a> {
    /// The header fields.
    pub header: RtpHeader,
    /// Number of CSRC identifiers that were present (and skipped).
    pub csrc_count: u8,
    /// The header extension, if present: (profile-defined 16-bit value, extension data).
    pub extension: Option<(u16, &'a [u8])>,
    /// The payload, with any RTP padding removed.
    pub payload: &'a [u8],
}

impl<'a> RtpPacket<'a> {
    /// Parse a datagram. Every length is checked; malformed input is an error, never a panic.
    pub fn parse(buf: &'a [u8]) -> Result<Self, Error> {
        if buf.len() < RTP_HEADER_LEN {
            return Err(Error::Truncated("RTP header"));
        }
        let b0 = buf[0];
        if b0 >> 6 != 2 {
            return Err(Error::Invalid("RTP version is not 2"));
        }
        let padding = b0 & 0x20 != 0;
        let has_ext = b0 & 0x10 != 0;
        let csrc_count = b0 & 0x0F;
        let header = RtpHeader {
            marker: buf[1] & 0x80 != 0,
            payload_type: buf[1] & 0x7F,
            sequence: u16::from_be_bytes([buf[2], buf[3]]),
            timestamp: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
            ssrc: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
        };
        let mut pos = RTP_HEADER_LEN + 4 * csrc_count as usize;
        if buf.len() < pos {
            return Err(Error::Truncated("RTP CSRC list"));
        }
        let mut extension = None;
        if has_ext {
            let Some(ext_head) = buf.get(pos..pos + 4) else {
                return Err(Error::Truncated("RTP header extension"));
            };
            let profile = u16::from_be_bytes([ext_head[0], ext_head[1]]);
            let words = u16::from_be_bytes([ext_head[2], ext_head[3]]) as usize;
            let start = pos + 4;
            let end = start + 4 * words;
            let Some(data) = buf.get(start..end) else {
                return Err(Error::Truncated("RTP header extension"));
            };
            extension = Some((profile, data));
            pos = end;
        }
        let mut end = buf.len();
        if padding {
            let pad = buf[end - 1] as usize;
            if pad == 0 || pad > end - pos {
                return Err(Error::Invalid("RTP padding length"));
            }
            end -= pad;
        }
        Ok(RtpPacket { header, csrc_count, extension, payload: &buf[pos..end] })
    }
}

/// Join the 16 high-order bits (ST 2110-20 Extended Sequence Number) and the RTP header's 16
/// low-order bits into the 32-bit extended sequence number.
pub fn extended_sequence(high: u16, low: u16) -> u32 {
    (u32::from(high) << 16) | u32::from(low)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = RtpHeader { marker: true, payload_type: 112, sequence: 0xBEEF, timestamp: 0x0102_0304, ssrc: 0xCAFE_F00D };
        let mut out = Vec::new();
        h.write(&mut out);
        out.extend_from_slice(b"data");
        assert_eq!(out.len(), 16);
        let p = RtpPacket::parse(&out).unwrap();
        assert_eq!(p.header, h);
        assert_eq!(p.payload, b"data");
        assert_eq!(p.extension, None);
    }

    #[test]
    fn extension_csrc_and_padding_are_skipped() {
        let mut out = vec![0x80 | 0x20 | 0x10 | 0x01, 96, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        out.extend_from_slice(&[9, 9, 9, 9]); // one CSRC
        out.extend_from_slice(&[0xBE, 0xDE, 0, 1, 0x10, 0xAA, 0, 0]); // RFC 8285 one-byte header, 1 word
        out.extend_from_slice(b"xy");
        out.extend_from_slice(&[0, 0, 3]); // 3 bytes of padding
        let p = RtpPacket::parse(&out).unwrap();
        assert_eq!(p.csrc_count, 1);
        assert_eq!(p.extension, Some((0xBEDE, &[0x10, 0xAA, 0, 0][..])));
        assert_eq!(p.payload, b"xy");
    }

    #[test]
    fn malformed_packets_are_errors() {
        assert!(RtpPacket::parse(&[0x80; 11]).is_err());
        assert!(RtpPacket::parse(&[0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err()); // version 1
        assert!(RtpPacket::parse(&[0x8F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err()); // 15 CSRCs missing
        assert!(RtpPacket::parse(&[0x90, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9]).is_err()); // extension too long
        assert!(RtpPacket::parse(&[0xA0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5]).is_err()); // padding longer than payload
        assert!(RtpPacket::parse(&[0xA0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err()); // zero padding count
    }

    #[test]
    fn extended_sequence_joins_halves() {
        assert_eq!(extended_sequence(0x0001, 0xFFFF), 0x0001_FFFF);
    }
}
