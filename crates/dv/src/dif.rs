//! DIF frame structure (IEC 61834-2, SMPTE 314M) and format detection.
//!
//! A frame is 10 (525/60) or 12 (625/50) DIF sequences per channel, each 150 blocks of 80
//! bytes: a header block, 2 subcode blocks, 3 VAUX blocks, then 9 groups of one audio block and
//! 15 video blocks. Every block starts with a 3-byte ID: section type (SCT) in the top 3 bits of
//! byte 0, the DIF sequence number in the top 4 bits of byte 1, the block number in byte 2.

/// Bytes per DIF block.
pub const BLOCK_SIZE: usize = 80;
/// DIF blocks per DIF sequence.
pub const BLOCKS_PER_SEQUENCE: usize = 150;
/// Bytes per DIF sequence.
pub const SEQUENCE_SIZE: usize = BLOCK_SIZE * BLOCKS_PER_SEQUENCE;

/// Section types (SCT).
pub mod sct {
    pub const HEADER: u8 = 0;
    pub const SUBCODE: u8 = 1;
    pub const VAUX: u8 = 2;
    pub const AUDIO: u8 = 3;
    pub const VIDEO: u8 = 4;
}

/// Section type of a DIF block.
pub fn block_sct(block: &[u8]) -> Option<u8> {
    block.first().map(|b| b >> 5)
}

/// DIF sequence number of a DIF block.
pub fn block_sequence(block: &[u8]) -> Option<u8> {
    block.get(1).map(|b| b >> 4)
}

/// Television system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum System {
    /// 525 lines, 29.97 frames/s (NTSC countries).
    Lines525,
    /// 625 lines, 25 frames/s (PAL / SECAM countries).
    Lines625,
}

/// DV family member, from the header's application ID and the VAUX source pack's signal type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// Consumer DV / DVCAM, 25 Mbit/s (IEC 61834-2): 4:1:1 at 525 lines, 4:2:0 at 625 lines.
    Dv25,
    /// DVCPRO 25 (SMPTE 314M): 4:1:1 at both line counts.
    Dvcpro25,
    /// DVCPRO 50 (SMPTE 314M), 4:2:2, two channels.
    Dvcpro50,
    /// DVCPRO HD / DV100 (SMPTE 370M), 4:2:2: 1080i in four channels (1280x1080 at 60 Hz,
    /// 1440x1080 at 50 Hz), 720p in two (960x720). Signal type 0x15 is recognised but not
    /// decoded.
    DvcproHd,
}

/// Chroma subsampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChromaFormat {
    /// Chroma at 1/4 horizontal, full vertical resolution.
    Yuv411,
    /// Chroma at 1/2 horizontal and vertical resolution.
    Yuv420,
    /// Chroma at 1/2 horizontal, full vertical resolution.
    Yuv422,
}

impl ChromaFormat {
    /// (horizontal, vertical) log2 subsampling.
    pub fn shifts(self) -> (u32, u32) {
        match self {
            Self::Yuv411 => (2, 0),
            Self::Yuv420 => (1, 1),
            Self::Yuv422 => (1, 0),
        }
    }
}

/// Display aspect ratio signalled in the VAUX source control pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aspect {
    Ratio4x3,
    Ratio16x9,
}

/// What a frame is, read from its first DIF sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub format: Format,
    pub system: System,
    pub chroma: ChromaFormat,
    /// Coded picture size.
    pub width: u32,
    pub height: u32,
    /// DIF sequences per channel.
    pub sequences: usize,
    /// Channels (1 for 25 Mbit/s, 2 for DVCPRO 50 and 720p, 4 for 1080i).
    pub channels: usize,
    pub aspect: Aspect,
    /// Application ID of the track (APT, header byte 4): 0 = IEC 61834, 1 = SMPTE 314M.
    pub apt: u8,
    /// Signal type from the VAUX source pack (0 = 25 Mbit/s, 4 = 50 Mbit/s, 0x14.. = HD).
    pub stype: u8,
}

impl Profile {
    /// Bytes per frame.
    pub fn frame_size(&self) -> usize {
        self.sequences * self.channels * SEQUENCE_SIZE
    }

    /// Frame rate as (numerator, denominator). A 720p DVCPRO HD frame is one progressive
    /// picture, so 720p runs at 59.94 or 50 frames/s.
    pub fn frame_rate(&self) -> (u32, u32) {
        match (self.system, self.is_720p()) {
            (System::Lines525, false) => (30000, 1001),
            (System::Lines625, false) => (25, 1),
            (System::Lines525, true) => (60000, 1001),
            (System::Lines625, true) => (50, 1),
        }
    }

    /// DVCPRO HD 720p (progressive).
    pub fn is_720p(&self) -> bool {
        self.format == Format::DvcproHd && self.height == 720
    }

    /// DVCPRO HD 1080i.
    pub fn is_1080i(&self) -> bool {
        self.format == Format::DvcproHd && self.height == 1080
    }

    /// Sample (pixel) aspect ratio of the coded picture for the signalled display aspect.
    pub fn sample_aspect(&self) -> (u32, u32) {
        let (dw, dh) = match self.aspect {
            Aspect::Ratio4x3 => (4, 3),
            Aspect::Ratio16x9 => (16, 9),
        };
        let (n, d) = (dw * self.height, dh * self.width);
        let g = gcd(n, d).max(1);
        (n / g, d / g)
    }

    /// True for the formats this crate decodes.
    pub fn is_supported(&self) -> bool {
        self.width != 0 && self.frame_size() != 0
    }

    /// Standard-definition DV is coded interlaced, bottom field first; DVCPRO HD 1080i is top
    /// field first.
    pub fn top_field_first(&self) -> bool {
        self.is_1080i()
    }

    /// DVCPRO HD 720p is progressive; everything else is interlaced.
    pub fn progressive(&self) -> bool {
        self.is_720p()
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Find a VAUX pack by its ID in the 3 VAUX blocks of the first DIF sequence (15 five-byte
/// packs per block, from byte 3).
pub fn vaux_pack(frame: &[u8], id: u8) -> Option<[u8; 5]> {
    for b in 3..6 {
        let block = frame.get(b * BLOCK_SIZE..(b + 1) * BLOCK_SIZE)?;
        if block_sct(block) != Some(sct::VAUX) {
            continue;
        }
        for p in 0..15 {
            let pack = block.get(3 + 5 * p..8 + 5 * p)?;
            if pack.first() == Some(&id) {
                let mut out = [0u8; 5];
                out.copy_from_slice(pack);
                return Some(out);
            }
        }
    }
    None
}

/// Identify a DV frame from its header block and VAUX packs. Returns `None` when `frame` does
/// not start with a DIF header block.
pub fn probe(frame: &[u8]) -> Option<Profile> {
    let h = frame.get(..BLOCK_SIZE)?;
    if block_sct(h) != Some(sct::HEADER) || block_sequence(h) != Some(0) || h.get(2) != Some(&0) {
        return None;
    }
    // the next blocks of a real DIF sequence are two subcode blocks
    if let Some(sc) = frame.get(BLOCK_SIZE..BLOCK_SIZE + 1)
        && block_sct(sc) != Some(sct::SUBCODE)
    {
        return None;
    }
    let dsf = h.get(3).map(|b| b >> 7)?;
    let apt = h.get(4).map(|b| b & 7)?;
    let system = if dsf == 0 { System::Lines525 } else { System::Lines625 };
    let sequences = if dsf == 0 { 10 } else { 12 };
    let stype = vaux_pack(frame, 0x60).map_or(0, |p| p[3] & 0x1f);
    let aspect = match vaux_pack(frame, 0x61).map(|p| p[2] & 7) {
        // DVCPRO HD is always 16:9
        _ if matches!(stype, 0x14 | 0x15 | 0x18) => Aspect::Ratio16x9,
        Some(2) | Some(7) => Aspect::Ratio16x9,
        _ => Aspect::Ratio4x3,
    };
    let (format, chroma, width, height, channels) = match stype {
        0 => {
            let chroma = if system == System::Lines625 && apt == 0 { ChromaFormat::Yuv420 } else { ChromaFormat::Yuv411 };
            let format = if apt == 0 { Format::Dv25 } else { Format::Dvcpro25 };
            (format, chroma, 720, if dsf == 0 { 480 } else { 576 }, 1)
        }
        4 => (Format::Dvcpro50, ChromaFormat::Yuv422, 720, if dsf == 0 { 480 } else { 576 }, 2),
        0x14 => (Format::DvcproHd, ChromaFormat::Yuv422, if dsf == 0 { 1280 } else { 1440 }, 1080, 4),
        0x18 => (Format::DvcproHd, ChromaFormat::Yuv422, 960, 720, 2),
        0x15 => (Format::DvcproHd, ChromaFormat::Yuv422, 0, 0, 0),
        _ => return None,
    };
    Some(Profile { format, system, chroma, width, height, sequences, channels, aspect, apt, stype })
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A minimal frame head: header block + 2 subcode + 3 VAUX blocks.
    pub(crate) fn head(dsf: u8, apt: u8, stype: u8, disp: u8) -> Vec<u8> {
        let mut f = vec![0xffu8; 6 * BLOCK_SIZE];
        for b in 0..6 {
            let s = match b {
                0 => sct::HEADER,
                1 | 2 => sct::SUBCODE,
                _ => sct::VAUX,
            };
            f[b * BLOCK_SIZE] = (s << 5) | 0x1f;
            f[b * BLOCK_SIZE + 1] = 0x07;
            f[b * BLOCK_SIZE + 2] = if b == 0 { 0 } else { ((b - 1) % 3) as u8 };
        }
        f[3] = (dsf << 7) | 0x3f;
        f[4] = 0xf8 | apt;
        let v = 3 * BLOCK_SIZE + 3;
        f[v..v + 5].copy_from_slice(&[0x60, 0xff, 0xff, 0xc0 | stype, 0xff]);
        f[v + 5..v + 10].copy_from_slice(&[0x61, 0x3f, 0xc8 | disp, 0xfc, 0xff]);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tests_support::head;

    #[test]
    fn formats() {
        let p = probe(&head(0, 1, 0, 0)).unwrap();
        assert_eq!((p.format, p.chroma, p.height, p.frame_size()), (Format::Dvcpro25, ChromaFormat::Yuv411, 480, 120_000));
        let p = probe(&head(1, 0, 0, 2)).unwrap();
        assert_eq!((p.format, p.chroma, p.height, p.frame_size(), p.aspect), (Format::Dv25, ChromaFormat::Yuv420, 576, 144_000, Aspect::Ratio16x9));
        assert_eq!(p.sample_aspect(), (64, 45));
        let p = probe(&head(1, 1, 0, 0)).unwrap();
        assert_eq!((p.format, p.chroma), (Format::Dvcpro25, ChromaFormat::Yuv411));
        assert_eq!(p.sample_aspect(), (16, 15));
        let p = probe(&head(0, 1, 4, 0)).unwrap();
        assert_eq!((p.format, p.frame_size(), p.is_supported()), (Format::Dvcpro50, 240_000, true));
        let p = probe(&head(1, 1, 0x14, 0)).unwrap();
        assert_eq!((p.format, p.width, p.height, p.frame_size(), p.sample_aspect()), (Format::DvcproHd, 1440, 1080, 576_000, (4, 3)));
        assert!(p.top_field_first() && !p.progressive());
        let p = probe(&head(0, 1, 0x14, 0)).unwrap();
        assert_eq!((p.width, p.frame_size(), p.sample_aspect(), p.frame_rate()), (1280, 480_000, (3, 2), (30000, 1001)));
        let p = probe(&head(1, 1, 0x18, 0)).unwrap();
        assert_eq!((p.width, p.height, p.frame_size(), p.frame_rate()), (960, 720, 288_000, (50, 1)));
        assert!(p.progressive());
        let p = probe(&head(0, 1, 0x15, 0)).unwrap();
        assert!(!p.is_supported());
        assert!(probe(&[0u8; 10]).is_none());
        assert!(probe(&vec![0u8; 6 * 80]).is_none());
    }
}
