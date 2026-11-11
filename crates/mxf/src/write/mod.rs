//! Clean-room MXF writer: OP1a (ST 378, frame-wrapped generic container) and OP-Atom (ST 390,
//! Avid-style: one clip-wrapped essence per file).
//!
//! The writer streams to any `Write + Seek` sink: [`MxfWriter::new`] writes the header partition
//! (open, incomplete) with the header metadata, then a body partition; essence follows as it is
//! pushed. [`MxfWriter::finish`] writes the footer partition (closed, complete header metadata with
//! the final durations, index table segments), the random index pack, and rewrites the header and
//! body partition packs and the header metadata in place (same size) so the file's header is closed
//! and complete too.
//!
//! Layout written:
//!
//! ```text
//! header partition pack · primer · preface · identification · content storage · essence container
//!   data · material package (+ tracks) · file package (+ tracks) · descriptor(s) · fill
//! body partition pack (BodySID 1)
//!   OP1a: one content package per edit unit: picture element, then one element per sound track
//!   OP-Atom: one clip-wrapped essence element
//! footer partition pack (IndexSID 2) · header metadata · index table segments
//! random index pack
//! ```
//!
//! Essence mappings: VC-3 / DNxHD / DNxHR (ST 2019-4), Apple ProRes (RDD 44), AVC byte stream
//! (ST 381-3; Annex B access units with in-band SPS / PPS) and MPEG-2 video (ST 381-1, MPEG video
//! descriptor) with long-GOP index entries (temporal offsets, key-frame offsets, prediction
//! flags), JPEG 2000 (ST 422, RGBA descriptor with the
//! JPEG 2000 picture sub-descriptor), type D-10 / IMX (ST 386: SDTI-CP compatible content
//! packages of system item, fixed-size picture and 8-channel AES3 element on a 512-byte KAG,
//! constant-size index) and Broadcast Wave PCM (ST 382, 8–32 bit).
//!
//! [`Pattern::DCinema`] writes D-Cinema track files (ST 429-3): OP-Atom with one frame-wrapped
//! essence track, the file package as the primary package, the closed and complete header
//! metadata only in the header partition and the index only in the footer; with
//! [`PictureCoding::J2k`] and [`ColorSpace::DcdmXyz`] the picture track file of ST 429-4, with
//! 24-bit PCM and [`WriterConfig::channel_assignment`] the sound track file of ST 429-2, with
//! [`WriterConfig::iab`] the immersive audio track file of ST 429-18 (one IABitstream frame per
//! data element, [`MxfWriter::push_data`]).

mod labels;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::io::{self, Seek, SeekFrom, Write};

use crate::Rational;
use labels::*;

/// A SMPTE ST 330 basic UMID (32 bytes): package identifiers.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Umid(pub [u8; 32]);

impl std::fmt::Debug for Umid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl Umid {
    /// The package UID of a D-Cinema track file (ST 429-3 9.3.2): a basic UMID of unidentified
    /// material type whose material number is the track file's UUID.
    pub fn dcinema(uuid: [u8; 16]) -> Umid {
        let mut u = [0u8; 32];
        u[..16].copy_from_slice(&[0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0F, 0x20, 0x13, 0x00, 0x00, 0x00]);
        u[16..].copy_from_slice(&uuid);
        Umid(u)
    }
    /// A basic UMID (universal label `06 0A 2B 34 01 01 01 05 01 01 0D 20`, length 0x13, instance
    /// 0) whose 16-byte material number is a hash of `seed` (callers pass something unique: path,
    /// time, track). The same seed always gives the same UMID.
    pub fn from_seed(seed: &[u8]) -> Umid {
        let mut u = [0u8; 32];
        u[..16].copy_from_slice(&[0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0D, 0x20, 0x13, 0x00, 0x00, 0x00]);
        u[16..].copy_from_slice(&hash128(seed));
        Umid(u)
    }
    /// The material number (bytes 16..32).
    pub fn material_number(&self) -> [u8; 16] {
        self.0[16..].try_into().unwrap_or([0; 16])
    }
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 32]
    }
}

fn hash128(seed: &[u8]) -> [u8; 16] {
    let mut a = 0xcbf2_9ce4_8422_2325u64;
    let mut b = 0x84222325_cbf29ce4u64 ^ seed.len() as u64;
    for &x in seed {
        a = (a ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
        b = (b ^ x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23);
    }
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&mix(a ^ b.rotate_left(17)).to_be_bytes());
    out[8..].copy_from_slice(&mix(b ^ a.rotate_left(41)).to_be_bytes());
    out
}

/// Operational pattern of the written file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// OP1a: one material package playing one file package; all essence interleaved per edit
    /// unit (frame wrapping).
    Op1a,
    /// OP-Atom: exactly one essence track (a picture or one sound track), clip-wrapped.
    OpAtom,
    /// D-Cinema track file (ST 429-3): OP-Atom with exactly one frame-wrapped essence track, the
    /// file package as the primary package, header metadata in the header partition only.
    DCinema,
    /// IMF track file (SMPTE ST 2067-5 as constrained by ST 2067-2 and, for JPEG 2000 pictures,
    /// ST 2067-21): OP1a with one essence track, the file package as the primary package,
    /// 8 KiB of fill after the header metadata and a random index pack. Pictures are frame-wrapped
    /// in essence partitions of about 60 s, each followed by an index partition; sound is 24-bit
    /// PCM of 48 or 96 kHz, clip-wrapped, at an edit rate equal to the sample rate, followed by an
    /// index partition with a constant-rate index table. Give the file package a D-Cinema style
    /// UMID ([`Umid::dcinema`], ST 2067-2 5.1.5).
    Imf,
}

/// Picture essence coding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureCoding {
    /// SMPTE VC-3 (DNxHD / DNxHR) with its compression id (e.g. 1272 = DNxHR HQ).
    Vc3 { cid: u32 },
    /// Apple ProRes; `profile` as in the RDD 44 coding label: 1 Proxy, 2 LT, 3 422, 4 HQ, 5 4444,
    /// 6 4444 XQ.
    ProRes { profile: u8 },
    /// H.264 / AVC Annex B byte stream; `profile_idc` from the SPS (66, 77, 100, …).
    Avc { profile_idc: u8, intra: bool },
    /// JPEG 2000 codestreams (ISO/IEC 15444-1); `rsiz` is the codestream's Rsiz: 3 / 4 for the
    /// 2K / 4K Digital Cinema profiles (ST 429-4 coding labels), anything else is written as an
    /// unconstrained Part 1 codestream.
    J2k { rsiz: u16 },
    /// MPEG-2 video elementary stream (ST 381-1), one coded picture per edit unit in coded order,
    /// with the MPEG video descriptor. Only the High and High-1440 levels, whose coding labels
    /// RDD 9 Table 6 gives: `profile_and_level` 0x44 (MP@HL), 0x46 (MP@H-14), 0x82 (422P@HL).
    /// `max_gop` 1 is intra-only; `b_pictures` the most B pictures between anchors; `bit_rate`
    /// the stream's maximum in bit/s (0: unknown).
    Mpeg2 { profile_and_level: u8, max_gop: u16, b_pictures: u8, closed_gop: bool, bit_rate: u32 },
    /// Uncompressed Y'CbCr pictures (ST 384) in one of the labelled layouts of ST 377-1 G.2.25.
    Uncompressed { layout: crate::UncompressedLayout },
    /// Type D-10 / IMX (ST 386, ST 356): intra-only MPEG-2 4:2:2P@ML at `mbps` 30, 40 or 50
    /// Mbit/s with 720 × 608 (`lines` 625, 25 Hz) or 720 × 512 (525, 30000/1001 Hz) stored
    /// lines, one SDTI-CP compatible content package per frame (ST 385 system item, the picture
    /// padded to its fixed size, one 8-channel AES3 sound element of ST 331), KAG 512 and a
    /// constant-size index. OP1a only; at most one sound track of 1-8 channels, 48 kHz, 16 or
    /// 24 bit.
    D10 { mbps: u8, lines: u16 },
}

/// The fixed geometry of a D-10 stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct D10Geometry {
    /// ST 386 Table 1 byte 15 and the coding label's byte 16.
    code: u8,
    /// Picture element bytes (SMPTE 356 frame size: bit rate × frame period / 8).
    frame_bytes: usize,
    /// Stored lines per field, display offset and height per field, the video line map.
    field_lines: u32,
    display_offset: u32,
    display_lines: u32,
    line_map: (u32, u32),
    edit_rate: Rational,
}

impl D10Geometry {
    fn of(mbps: u8, lines: u16) -> Option<D10Geometry> {
        let rate = match mbps {
            50 => 0,
            40 => 1,
            30 => 2,
            _ => return None,
        };
        match lines {
            625 => Some(D10Geometry {
                code: 1 + 2 * rate,
                frame_bytes: [250_000, 200_000, 150_000][rate as usize],
                field_lines: 304,
                display_offset: 16,
                display_lines: 288,
                line_map: (7, 320),
                edit_rate: Rational::new(25, 1),
            }),
            525 => Some(D10Geometry {
                code: 2 + 2 * rate,
                frame_bytes: [208_541, 166_833, 125_125][rate as usize],
                field_lines: 256,
                display_offset: 13,
                display_lines: 243,
                line_map: (7, 270),
                edit_rate: Rational::new(30000, 1001),
            }),
            _ => None,
        }
    }
}

impl PictureCoding {
    /// D-10 only: the fixed picture element size in bytes (coded pictures are padded to it and
    /// must not exceed it), `None` for an unsupported rate or line standard.
    pub fn d10_frame_bytes(&self) -> Option<usize> {
        match *self {
            PictureCoding::D10 { mbps, lines } => D10Geometry::of(mbps, lines).map(|g| g.frame_bytes),
            _ => None,
        }
    }
    fn intra_only(&self) -> bool {
        match self {
            PictureCoding::Avc { intra, .. } => *intra,
            PictureCoding::Mpeg2 { max_gop, .. } => *max_gop <= 1,
            _ => true,
        }
    }
    /// Picture essence coding label.
    fn coding_label(&self) -> [u8; 16] {
        match *self {
            PictureCoding::Vc3 { cid } => {
                let code = if (1235..=1234 + 0x7F).contains(&cid) { (cid - 1234) as u8 } else { 0 };
                [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x04, 0x01, 0x02, 0x02, 0x71, code, 0x00, 0x00]
            }
            PictureCoding::ProRes { profile } => {
                [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x04, 0x01, 0x02, 0x02, 0x03, 0x06, profile.clamp(1, 6), 0x00]
            }
            PictureCoding::Avc { profile_idc, intra } => {
                let (b13, b14) = if intra {
                    (0x32, 0x20)
                } else {
                    (
                        0x31,
                        match profile_idc {
                            66 => 0x10,
                            77 => 0x20,
                            88 => 0x30,
                            110 => 0x50,
                            122 => 0x60,
                            244 => 0x70,
                            _ => 0x40,
                        },
                    )
                };
                [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x04, 0x01, 0x02, 0x02, 0x01, b13, b14, 0x01]
            }
            PictureCoding::J2k { rsiz } => {
                // ST 422 Table 3: Part 1 codestreams, byte 16 the D-Cinema restrictions (ST 429-4);
                // the IMF profiles (Rsiz 04xxh-09xxh) in byte 15, their levels in byte 16
                if let Some(imf) = imf_j2k_label(rsiz) {
                    return imf;
                }
                let (version, last) = match rsiz {
                    3 | 4 => (0x09, rsiz as u8),
                    _ => (0x07, 0x7F),
                };
                [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, version, 0x04, 0x01, 0x02, 0x02, 0x03, 0x01, 0x01, last]
            }
            PictureCoding::Mpeg2 { profile_and_level, max_gop, .. } => {
                // RDD 9 Table 6: byte 14 the profile and level, byte 15 03h long GOP
                let pl = match profile_and_level {
                    0x82 => 0x04,
                    0x46 => 0x05,
                    _ => 0x03,
                };
                [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x03, 0x04, 0x01, 0x02, 0x02, 0x01, pl, if max_gop <= 1 { 0x02 } else { 0x03 }, 0x00]
            }
            PictureCoding::Uncompressed { layout } => layout.label(),
            PictureCoding::D10 { mbps, lines } => {
                // RP 224: MPEG-2 422P@ML I-frame, the D-10 rate and line standard in byte 16
                let code = D10Geometry::of(mbps, lines).map_or(0x01, |g| g.code);
                [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x04, 0x01, 0x02, 0x02, 0x01, 0x02, 0x01, code]
            }
        }
    }
    /// (essence container label, element type) for frame or clip wrapping.
    fn container(&self, clip: bool) -> ([u8; 16], u8) {
        let w = if clip { 0x02 } else { 0x01 };
        match self {
            PictureCoding::Vc3 { .. } => (essence_container(0x0A, 0x11, w, 0x00), if clip { 0x0D } else { 0x0C }),
            PictureCoding::ProRes { .. } => (essence_container(0x0D, 0x1C, w, 0x00), if clip { 0x18 } else { 0x17 }),
            PictureCoding::Avc { .. } => (essence_container(0x0A, 0x10, 0x60, w), if clip { 0x16 } else { 0x15 }),
            // ST 422 Table 2 ("FU" frame wrapping / "Cn" clip wrapping) and Table 1
            PictureCoding::J2k { .. } => (essence_container(0x07, 0x0C, w, 0x00), if clip { 0x09 } else { 0x08 }),
            // ST 381-1 Tables 5, 6 (MPEG ES, stream_id E0h bits 6..0) and Table 1 (05h / 06h)
            PictureCoding::Mpeg2 { .. } => (essence_container(0x02, 0x04, 0x60, w), if clip { 0x06 } else { 0x05 }),
            // ST 384 Tables 5-7: lines and rate from the descriptor (7Fh); element types 02h / 03h
            PictureCoding::Uncompressed { .. } => (essence_container(0x01, 0x05, 0x7F, w), if clip { 0x03 } else { 0x02 }),
            // ST 386 Tables 1 and 2: the defined template, the 422P@ML picture element (01h)
            PictureCoding::D10 { mbps, lines } => (essence_container(0x01, 0x01, D10Geometry::of(*mbps, *lines).map_or(0x01, |g| g.code), 0x01), 0x01),
        }
    }
}

/// Colour description of the picture (transfer, coding equations, primaries labels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSpace {
    #[default]
    Rec709,
    /// BT.2020 primaries and matrix, SMPTE ST 2084 transfer.
    Rec2020Pq,
    /// BT.2020 primaries and matrix, ARIB STD-B67 (HLG) transfer.
    Rec2020Hlg,
    /// D-Cinema X'Y'Z' (ST 428-1: 12-bit, gamma 2.6): an RGBA descriptor with the DCDM pixel
    /// layout (ST 429-4).
    DcdmXyz,
}

/// The JPEG 2000 picture sub-descriptor (ST 422 Table 7): the SIZ values shared by every
/// codestream of the track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct J2kSubDescriptor {
    pub rsiz: u16,
    pub xsiz: u32,
    pub ysiz: u32,
    pub xosiz: u32,
    pub yosiz: u32,
    pub xtsiz: u32,
    pub ytsiz: u32,
    pub xtosiz: u32,
    pub ytosiz: u32,
    /// (Ssiz, XRsiz, YRsiz) per component.
    pub components: Vec<(u8, u8, u8)>,
    /// Coding Style Default: Scod, SGcod and SPcod of the COD marker, when the same in every
    /// codestream.
    pub coding_style: Option<Vec<u8>>,
    /// Quantization Default: Sqcd and the SPqcd of the QCD marker.
    pub quantization: Option<Vec<u8>>,
    /// J2CLayout (ST 377-1 RGBALayout): component codes and depths in codestream order.
    pub layout: Option<[u8; 16]>,
}

impl J2kSubDescriptor {
    /// One tile covering a `width`×`height` image of `components` components of `depth` bits,
    /// unsigned and not subsampled (the D-Cinema profiles: 3 components of 12 bits).
    pub fn single_tile(rsiz: u16, width: u32, height: u32, components: usize, depth: u8) -> Self {
        J2kSubDescriptor {
            rsiz,
            xsiz: width,
            ysiz: height,
            xosiz: 0,
            yosiz: 0,
            xtsiz: width,
            ytsiz: height,
            xtosiz: 0,
            ytosiz: 0,
            components: vec![(depth.saturating_sub(1), 1, 1); components],
            coding_style: None,
            quantization: None,
            layout: None,
        }
    }

    /// The values of a codestream's main header: SIZ, and the COD and QCD bodies as the coding
    /// and quantization defaults. `None` when the header is not a JPEG 2000 main header.
    pub fn from_codestream(cs: &[u8]) -> Option<Self> {
        if cs.get(..2)? != [0xFF, 0x4F] {
            return None;
        }
        let mut at = 2usize;
        let mut d: Option<J2kSubDescriptor> = None;
        let (mut cod, mut qcd) = (None, None);
        while let Some(m) = cs.get(at..at + 2) {
            let marker = u16::from_be_bytes([m[0], m[1]]);
            if marker == 0xFF90 || marker == 0xFF93 {
                break;
            }
            let len = usize::from(u16::from_be_bytes(cs.get(at + 2..at + 4)?.try_into().ok()?));
            let body = cs.get(at + 4..at + 2 + len.max(2))?;
            let be32 = |o: usize| -> Option<u32> { Some(u32::from_be_bytes(body.get(o..o + 4)?.try_into().ok()?)) };
            match marker {
                0xFF51 => {
                    let n = usize::from(u16::from_be_bytes(body.get(34..36)?.try_into().ok()?));
                    let comps = body.get(36..36 + 3 * n)?.chunks_exact(3).map(|c| (c[0], c[1], c[2])).collect();
                    d = Some(J2kSubDescriptor {
                        rsiz: u16::from_be_bytes(body.get(..2)?.try_into().ok()?),
                        xsiz: be32(2)?,
                        ysiz: be32(6)?,
                        xosiz: be32(10)?,
                        yosiz: be32(14)?,
                        xtsiz: be32(18)?,
                        ytsiz: be32(22)?,
                        xtosiz: be32(26)?,
                        ytosiz: be32(30)?,
                        components: comps,
                        coding_style: None,
                        quantization: None,
                        layout: None,
                    });
                }
                0xFF52 => cod = Some(body.to_vec()),
                0xFF5C => qcd = Some(body.to_vec()),
                _ => {}
            }
            at = at.checked_add(2 + len)?;
        }
        let mut d = d?;
        d.coding_style = cod;
        d.quantization = qcd;
        Some(d)
    }
}

/// The picture track.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureDesc {
    pub coding: PictureCoding,
    pub width: u32,
    pub height: u32,
    /// Display aspect ratio of the frame (e.g. 16/9).
    pub aspect: Rational,
    /// Bits per component.
    pub depth: u32,
    /// Chroma subsampling: (2, 1) 4:2:2, (2, 2) 4:2:0, (1, 1) 4:4:4.
    pub subsampling: (u32, u32),
    pub color: ColorSpace,
    /// JPEG 2000 only: the picture sub-descriptor.
    pub j2k: Option<J2kSubDescriptor>,
    /// [`Pattern::Imf`]: R'G'B' components (an RGBA descriptor) instead of Y'C'BC'R (CDCI).
    pub rgb: bool,
    /// [`Pattern::Imf`] R'G'B': full-range quantization (ST 2067-21 QE.2) instead of the
    /// narrow range of QE.1.
    pub full_range: bool,
}

impl PictureDesc {
    /// Square pixels; 8-bit 4:2:0 for AVC, 10-bit 4:2:2 for ProRes, 8-bit 4:2:2 for VC-3
    /// (10-bit for DNxHR HQX / 444 and the 10-bit DNxHD ids).
    pub fn new(coding: PictureCoding, width: u32, height: u32) -> Self {
        let g = gcd(width.max(1), height.max(1));
        let (depth, subsampling) = match coding {
            PictureCoding::Avc { .. } => (8, (2, 2)),
            PictureCoding::ProRes { profile } => (if profile >= 5 { 12 } else { 10 }, if profile >= 5 { (1, 1) } else { (2, 1) }),
            PictureCoding::Vc3 { cid } => {
                (if matches!(cid, 1235 | 1241 | 1250 | 1256 | 1270 | 1271) { 10 } else { 8 }, if matches!(cid, 1256 | 1270) { (1, 1) } else { (2, 1) })
            }
            PictureCoding::J2k { .. } => (12, (1, 1)),
            PictureCoding::Mpeg2 { profile_and_level, .. } => (8, if profile_and_level & 0x70 == 0 { (2, 1) } else { (2, 2) }),
            PictureCoding::Uncompressed { layout } => (layout.depth(), layout.subsampling()),
            PictureCoding::D10 { .. } => (8, (2, 1)),
        };
        PictureDesc {
            coding,
            width,
            height,
            aspect: Rational::new((width.max(1) / g) as i32, (height.max(1) / g) as i32),
            depth,
            subsampling,
            color: ColorSpace::Rec709,
            j2k: None,
            rgb: false,
            full_range: false,
        }
    }

    /// A D-Cinema picture track (ST 429-4): JPEG 2000 of the 2K (`rsiz` 3) or 4K (4) profile,
    /// 12-bit X'Y'Z' in one tile.
    pub fn dcinema(rsiz: u16, width: u32, height: u32) -> Self {
        let mut d = PictureDesc::new(PictureCoding::J2k { rsiz }, width, height);
        d.color = ColorSpace::DcdmXyz;
        d.j2k = Some(J2kSubDescriptor::single_tile(rsiz, width, height, 3, 12));
        d
    }
}

/// The picture essence coding label of a JPEG 2000 IMF profile codestream (ST 422 Table 3, byte
/// 15 the profile; byte 16 the mainlevel and sublevel as the SMPTE label register numbers them:
/// mainlevel M starts at 2 + the sublevel counts of the levels below it, max(1, M - 2) + 1
/// each, and adds the sublevel). `None` for other Rsiz values.
fn imf_j2k_label(rsiz: u16) -> Option<[u8; 16]> {
    let profile = (rsiz >> 8) as u8;
    if !(0x04..=0x09).contains(&profile) {
        return None;
    }
    let (mainlevel, sublevel) = ((rsiz & 0x0F) as u8, ((rsiz >> 4) & 0x0F) as u8);
    let level = if (1..=10).contains(&mainlevel) && sublevel <= mainlevel.saturating_sub(2).max(1) {
        let base: u8 = (1..mainlevel).map(|m| m.saturating_sub(2).max(1) + 1).sum::<u8>() + 2;
        base + sublevel
    } else {
        0
    };
    Some([0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x04, 0x01, 0x02, 0x02, 0x03, 0x01, profile - 2, level])
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// A PCM sound track (Broadcast Wave mapping, little-endian interleaved samples).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundDesc {
    pub sample_rate: u32,
    pub channels: u32,
    /// 8, 16, 24 or 32.
    pub bits: u32,
}

impl SoundDesc {
    pub fn block_align(&self) -> u32 {
        self.channels.max(1) * self.bits.div_ceil(8)
    }
}

/// Start timecode of the packages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartTimecode {
    /// Frame count of the first frame (at the timecode rate).
    pub frames: i64,
    /// Timecode frame rate (the track's edit rate; 30000/1001 for 29.97).
    pub rate: Rational,
    pub drop_frame: bool,
}

impl StartTimecode {
    pub fn rounded_base(&self) -> u16 {
        if !self.rate.is_valid() {
            return 25;
        }
        ((self.rate.num as i64 + self.rate.den as i64 - 1) / self.rate.den as i64) as u16
    }
}

/// Unix seconds → (proleptic Gregorian year, the same wall-clock instant moved into chrono's range
/// by whole 400-year cycles). The Gregorian calendar repeats every 400 years (146_097 days), so
/// month, day and time of day come from chrono and only the year is carried, which keeps every
/// `i64` exact instead of collapsing out-of-range years to the epoch.
fn utc_parts(secs: i64) -> (i64, chrono::NaiveDateTime) {
    use chrono::Datelike;
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` (1970-01-01 is CE day 719_163) is always a valid CE day, and `sod` < 86_400.
    let date = i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
    let time = u32::try_from(sod).ok().and_then(|s| chrono::NaiveTime::from_num_seconds_from_midnight_opt(s, 0));
    match (date, time) {
        (Some(d), Some(t)) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), d.and_time(t))
        }
        _ => (1970, chrono::NaiveDateTime::default()), // unreachable
    }
}

/// An MXF timestamp (UTC).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
}

impl Timestamp {
    /// From seconds since the Unix epoch (proleptic Gregorian, UTC). Years far outside chrono's own
    /// range are preserved (the old hand-rolled conversion kept them) with month, day and time
    /// intact; only the `year` is clamped to the MXF `u16` field, exactly as the old code did.
    pub fn from_unix(secs: i64) -> Timestamp {
        use chrono::{Datelike, Timelike};
        let (y, t) = utc_parts(secs);
        Timestamp {
            year: y.clamp(0, 65535) as u16,
            month: t.month() as u8,
            day: t.day() as u8,
            hour: t.hour() as u8,
            minute: t.minute() as u8,
            second: t.second() as u8,
            millisecond: 0,
        }
    }
    fn bytes(&self) -> [u8; 8] {
        let y = self.year.to_be_bytes();
        [y[0], y[1], self.month, self.day, self.hour, self.minute, self.second, (self.millisecond / 4) as u8]
    }
}

/// Package identifiers and names. The material package is what an AAF composition (or an NLE)
/// references; the file package describes the stored essence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageIds {
    pub material: Umid,
    pub file: Umid,
    pub material_name: String,
    pub file_name: String,
}

impl PackageIds {
    /// UMIDs derived from `seed` (material and file differ).
    pub fn from_seed(seed: &str, name: &str) -> Self {
        PackageIds {
            material: Umid::from_seed(format!("material:{seed}").as_bytes()),
            file: Umid::from_seed(format!("file:{seed}").as_bytes()),
            material_name: name.to_string(),
            file_name: name.to_string(),
        }
    }
}

/// The immersive audio data essence of a D-Cinema IAB track file (ST 429-18 Tables 3 and 5):
/// values for the sub-descriptor, taken from the IAFrames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IabDesc {
    /// IAFrame Version.
    pub version: u8,
    /// Most bed channels in a frame.
    pub max_channels: u16,
    /// Most objects in a frame.
    pub max_objects: u16,
    /// Edit unit aligned with the first frame of action of the picture.
    pub first_frame: u32,
    /// IAFrame SampleRate (48000 or 96000).
    pub sample_rate: u32,
}

/// Everything the writer needs up front.
#[derive(Clone, Debug, PartialEq)]
pub struct WriterConfig {
    pub pattern: Pattern,
    /// Edit rate of the essence tracks (the frame rate; OP-Atom sound files may use the sample
    /// rate, one edit unit per sample).
    pub edit_rate: Rational,
    pub picture: Option<PictureDesc>,
    pub sound: Vec<SoundDesc>,
    pub timecode: Option<StartTimecode>,
    pub ids: PackageIds,
    /// Track id of the first essence track (the timecode track is 1). OP-Atom files that share a
    /// material package give their tracks distinct ids (Avid style: V1 = 2, A1 = 3, …).
    pub first_track_id: u32,
    pub company: String,
    pub product: String,
    pub version: String,
    pub modified: Timestamp,
    /// Sound tracks: the Wave descriptor's channel assignment label (ST 429-2 Annex A).
    pub channel_assignment: Option<[u8; 16]>,
    /// [`Pattern::DCinema`] only: an immersive audio track file (no picture, no sound).
    pub iab: Option<IabDesc>,
    /// Sound tracks: multichannel audio labels (ST 377-4), the sound descriptor's sub-descriptors.
    pub mca: Vec<McaLabel>,
    /// SMPTE RDD 9 (XDCAM HD422 style) OP1a: MPEG-2 long GOP pictures in content packages with an
    /// SDTI-CP compatible system item (ST 385) and one frame-wrapped AES3 element per audio
    /// channel (ST 382; 2, 4 or 8 elements), every element on a 512-byte KAG, a new body
    /// partition at the first key frame after five seconds whose index table segment indexes the
    /// previous partition. Needs [`PictureCoding::Mpeg2`] and at most one sound track (48 kHz,
    /// 16 or 24 bit, 1-8 channels).
    pub rdd9: bool,
    /// Sound tracks: the edit rate of the pictures the sound was made for (ST 2067-2 Annex E).
    pub reference_image_edit_rate: Option<Rational>,
    /// Sound tracks: the alignment level in dBFS, e.g. -20 (ST 2067-2 Annex E).
    pub reference_audio_alignment_level: Option<i8>,
}

/// One ST 377-4 multichannel audio label: a soundfield group (`channel` is `None`) or an audio
/// channel, optionally linked to its group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McaLabel {
    /// MCA label dictionary id (the label's UL, e.g. ST 428-12 `chL` or ST 430-12 `FSKSync`).
    pub dictionary: [u8; 16],
    /// MCA link id: this label's UUID.
    pub link: [u8; 16],
    pub symbol: String,
    pub name: String,
    /// 1-based channel in the track (audio channel labels only).
    pub channel: Option<u32>,
    /// RFC 5646 spoken language.
    pub language: Option<String>,
    /// The soundfield group's link id (audio channel labels in a group).
    pub group: Option<[u8; 16]>,
}

impl WriterConfig {
    pub fn new(pattern: Pattern, edit_rate: Rational, ids: PackageIds) -> Self {
        WriterConfig {
            pattern,
            edit_rate,
            picture: None,
            sound: Vec::new(),
            timecode: None,
            ids,
            first_track_id: 2,
            company: "CFA4".into(),
            product: "CFA4".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            modified: Timestamp::default(),
            channel_assignment: None,
            iab: None,
            mca: Vec::new(),
            rdd9: false,
            reference_image_edit_rate: None,
            reference_audio_alignment_level: None,
        }
    }
}

/// How a picture was coded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CodedKind {
    #[default]
    Intra,
    /// Forward predicted (P).
    Predicted,
    /// Bidirectionally predicted (B).
    Bidirectional,
}

/// Index information for one picture, in stored (decode) order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameInfo {
    /// A decoder can start here (IDR / intra picture with its parameter sets).
    pub key: bool,
    pub kind: CodedKind,
    /// Presentation position (edit units from the start); `None` = the stored position.
    pub display: Option<i64>,
}

impl FrameInfo {
    /// An independently decodable picture presented in stored order.
    pub fn intra() -> Self {
        FrameInfo { key: true, kind: CodedKind::Intra, display: None }
    }
}

/// One index entry under construction.
#[derive(Clone, Debug)]
struct Entry {
    /// Stream offset of the edit unit in the essence container.
    offset: u64,
    /// Offsets of slices 1.. from the edit unit start (OP1a sound elements).
    slices: Vec<u32>,
    info: FrameInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackKind {
    Picture,
    Sound(usize),
    /// Immersive audio data (ST 429-18).
    Iab,
    /// The 8-channel AES3 sound element of a D-10 content package (ST 331 7.1).
    Aes3,
    /// One channel of an RDD 9 sound track: a frame-wrapped AES3 element (ST 382).
    Channel(usize),
}

/// The sound of an RDD 9 content package.
#[derive(Clone, Copy, Debug)]
struct Rdd9Layout {
    /// Channels pushed and AES3 elements written (4 or 8; the extra ones are silent).
    channels: usize,
    elements: usize,
    /// Bytes per sample (2 or 3).
    bps: usize,
}

/// An RDD 9 body partition after the first: its offset, index byte count and essence offset
/// (the pack is rewritten with the footer offset at the end).
#[derive(Clone, Copy, Debug)]
struct BodyPartition {
    this: u64,
    prev: u64,
    index_bytes: u64,
    body_offset: u64,
}

/// Edit units of an RDD 9 body partition before a new one starts at the next key frame
/// (RDD 9 8.3.1 keeps a partition under ten seconds), in seconds.
const RDD9_PARTITION_SECONDS: i64 = 5;

/// The SDTI-CP content package rate byte (ST 326): the rate code (1: 24, 2: 25, 3: 30, 4: 48,
/// 5: 50, 6: 60 Hz) shifted left, bit 0 the 1.001 divisor.
fn cp_rate(r: Rational) -> u8 {
    let (fps, ntsc) = match r.den {
        1 => (r.num, false),
        1001 => (r.num / 1000, true),
        _ => return 0,
    };
    let code = match fps {
        24 => 1,
        25 => 2,
        30 => 3,
        48 => 4,
        50 => 5,
        60 => 6,
        _ => 0,
    };
    (code << 1) | u8::from(ntsc)
}

/// The system item of an SDTI-CP compatible content package (ST 385 5.2, RDD 9 5.2): the 57-byte
/// system metadata pack (`bitmap`, rate, continuity count `i`, the container label, the creation
/// stamp when bit 5 is set, the time code as the user stamp) and an empty package metadata set,
/// filled to `size` bytes.
fn system_item(out: &mut Vec<u8>, bitmap: u8, rate: Rational, i: u64, label: &[u8; 16], stamp: &[u8; 17], size: usize) {
    let start = out.len();
    let mut pack = Vec::with_capacity(57);
    pack.push(bitmap);
    pack.push(cp_rate(rate));
    pack.extend_from_slice(&[0, 0, 0]);
    pack.extend_from_slice(&((i % 65_536) as u16).to_be_bytes());
    pack.extend_from_slice(label);
    pack.extend_from_slice(if bitmap & 0x20 != 0 { stamp } else { &[0; 17] });
    pack.extend_from_slice(stamp);
    klv(out, &SYSTEM_METADATA_PACK, &pack);
    klv(out, &PACKAGE_METADATA_SET, &[]);
    fill_to(out, start + size);
}

/// Item sizes of a D-10 content package (each a multiple of the 512-byte KAG).
#[derive(Clone, Copy, Debug)]
struct D10Layout {
    geo: D10Geometry,
    label: [u8; 16],
    system: usize,
    picture: usize,
    sound: usize,
    /// Channels of the sound track pushed (0: none, the element's channels are all invalid).
    channels: usize,
    bits: u32,
}

const D10_KAG: usize = 512;

/// `raw` bytes plus the fill item that ends the item on the KAG (a fill item is at least 20
/// bytes: key and a 4-byte length).
fn kag_aligned(raw: usize) -> usize {
    let a = raw.next_multiple_of(D10_KAG);
    if a > raw && a - raw < 20 { a + D10_KAG } else { a }
}

impl D10Layout {
    fn content_package(&self) -> usize {
        self.system + self.picture + self.sound
    }
    /// 48 kHz sample frames in content package `i` (1920 at 25 Hz; 1601 or 1602 at 30000/1001).
    fn samples(&self, i: u64) -> usize {
        let r = self.geo.edit_rate;
        let at = |n: u64| (n as u128 * 48_000 * r.den as u128 / r.num.max(1) as u128) as usize;
        at(i + 1) - at(i)
    }
    /// The 5-sequence count of the AES3 element header (ST 331 7.1): 0 at 25 Hz; at 30000/1001
    /// the 1602, 1601, 1602, 1601, 1602 sequence numbered 1 to 5. Package 0 holds 1601 samples
    /// (number 2), so package `i` is number `(i + 1) mod 5 + 1`.
    fn sequence(&self, i: u64) -> u8 {
        if self.geo.edit_rate.den == 1 { 0 } else { ((i + 1) % 5 + 1) as u8 }
    }
}

/// An 8-channel AES3 element (ST 331 7.1): header, sample count, channel valid flags, then for
/// every sample one 32-bit word per channel 1-8 (channel number in bits 0-2, the 24-bit AES3
/// audio word left-justified in bits 4-27, no V/U/C/P). `pcm` holds `channels` interleaved
/// little-endian samples of `bits` (16 or 24).
fn aes3_element(pcm: &[u8], channels: usize, bits: u32, samples: usize, sequence: u8) -> Vec<u8> {
    let bps = (bits / 8) as usize;
    let mut v = Vec::with_capacity(4 + samples * 32);
    v.push(sequence & 0x07);
    v.extend_from_slice(&(samples.min(0xFFFF) as u16).to_le_bytes());
    v.push(((1u16 << channels.min(8)) - 1) as u8);
    for s in 0..samples {
        for c in 0..8usize {
            let at = (s * channels + c) * bps;
            let word = match (c < channels, pcm.get(at..at + bps)) {
                (true, Some(&[a, b])) => u32::from_le_bytes([0, a, b, 0]),
                (true, Some(&[a, b, d])) => u32::from_le_bytes([a, b, d, 0]),
                _ => 0,
            };
            v.extend_from_slice(&((c as u32) | (word << 4)).to_le_bytes());
        }
    }
    v
}

/// A 17-byte date/time stamp of the system metadata pack (ST 385 5.2.3.3): type 81h, the SMPTE
/// ST 12-1 time code of `frames` (BCD frames, seconds, minutes, hours, LSB first; drop frame in
/// bit 6 of the frames byte), the binary groups and the rest zero.
fn timecode_stamp(frames: i64, base: u16, drop: bool) -> [u8; 17] {
    let base = i64::from(base.max(1));
    let mut f = frames.rem_euclid(24 * 3600 * base);
    if drop && base == 30 {
        // 2 frame numbers dropped every minute except every tenth (17 982 frames per 10 minutes)
        let (d, m) = (f / 17_982, f % 17_982);
        f += 18 * d + if m > 1 { 2 * ((m - 2) / 1798) } else { 0 };
    }
    let bcd = |x: i64| (((x / 10) << 4) | (x % 10)) as u8;
    let mut t = [0u8; 17];
    t[0] = 0x81;
    t[1] = bcd(f % base) | if drop { 0x40 } else { 0 };
    t[2] = bcd(f / base % 60);
    t[3] = bcd(f / (base * 60) % 60);
    t[4] = bcd(f / (base * 3600) % 24);
    t
}

#[derive(Clone, Debug)]
struct TrackW {
    id: u32,
    kind: TrackKind,
    element_key: [u8; 16],
    container: [u8; 16],
}

/// Header region: header metadata plus reserved fill, rewritten in place by `finish`.
const HEADER_RESERVE: usize = 4096;
/// IMF track files: at least 8 KiB of fill after the header metadata (ST 2067-5 5.1.1), on top
/// of the usual room for the metadata growing by `finish`.
const IMF_HEADER_RESERVE: usize = HEADER_RESERVE + 8192;

/// An IMF body partition after the first: essence (`index_bytes` 0) or an index table.
#[derive(Clone, Copy, Debug)]
struct ImfPartition {
    this: u64,
    prev: u64,
    index_bytes: u64,
    body_offset: u64,
}
const INDEX_SID: u32 = 2;
const BODY_SID: u32 = 1;
/// Entries per index table segment (the entry array's 2-byte local length limits a segment).
const MAX_SEGMENT_BYTES: usize = 60_000;

/// Streaming MXF writer. See the [module docs](self).
pub struct MxfWriter<W: Write + Seek> {
    w: W,
    base: u64,
    /// Bytes written so far (relative to `base`).
    pos: u64,
    cfg: WriterConfig,
    tracks: Vec<TrackW>,
    header_region: usize,
    body_partition: u64,
    /// File offset (relative) of the first byte of the essence container.
    essence_start: u64,
    entries: Vec<Entry>,
    /// Type D-10: SDTI-CP compatible content packages.
    d10: Option<D10Layout>,
    /// RDD 9: content packages with a system item and one AES3 element per channel.
    rdd9: Option<Rdd9Layout>,
    /// Essence container bytes written (the stream offset of the next edit unit).
    stream: u64,
    /// RDD 9: offsets of the body partitions after the first, and the first edit unit of the
    /// current one.
    partitions: Vec<BodyPartition>,
    segment_start: usize,
    /// IMF: the essence and index partitions after the first body partition.
    imf_parts: Vec<ImfPartition>,
    // OP1a
    pending: VecDeque<(Vec<u8>, FrameInfo)>,
    sound_buf: Vec<Vec<u8>>,
    // OP-Atom
    clip_len: u64,
    clip_kl: u64,
    samples: u64,
}

fn ber4(n: usize) -> [u8; 4] {
    let b = (n as u32).to_be_bytes();
    [0x83, b[1], b[2], b[3]]
}

fn klv(out: &mut Vec<u8>, key: &[u8; 16], v: &[u8]) {
    out.extend_from_slice(key);
    if v.len() < 1 << 24 {
        out.extend_from_slice(&ber4(v.len()));
    } else {
        out.push(0x88);
        out.extend_from_slice(&(v.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(v);
}

fn fill(out: &mut Vec<u8>, total: usize) {
    if total < 20 {
        // too small for a fill item: zero-length fill items are not possible; caller avoids this
        out.extend(std::iter::repeat_n(0u8, total));
        return;
    }
    out.extend_from_slice(&KLV_FILL);
    out.extend_from_slice(&ber4(total - 20));
    out.extend(std::iter::repeat_n(0u8, total - 20));
}

/// Fill `out` up to `len` bytes (nothing when it is already that long).
fn fill_to(out: &mut Vec<u8>, len: usize) {
    let n = len.saturating_sub(out.len());
    fill(out, n);
}

fn batch(items: &[&[u8]], item_size: usize) -> Vec<u8> {
    let mut v = (items.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(&(item_size as u32).to_be_bytes());
    for i in items {
        v.extend_from_slice(i);
    }
    v
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

fn rational(r: Rational) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&r.num.to_be_bytes());
    b[4..].copy_from_slice(&r.den.to_be_bytes());
    b
}

/// A header metadata local set under construction.
struct Set {
    key: [u8; 16],
    v: Vec<u8>,
}

impl Set {
    fn new(kind: u8, uid: [u8; 16]) -> Set {
        let mut s = Set { key: set_key(kind), v: Vec::new() };
        s.p(0x3C0A, &uid);
        s
    }
    fn p(&mut self, tag: u16, v: &[u8]) -> &mut Set {
        self.v.extend_from_slice(&tag.to_be_bytes());
        self.v.extend_from_slice(&(v.len().min(0xFFFF) as u16).to_be_bytes());
        self.v.extend_from_slice(&v[..v.len().min(0xFFFF)]);
        self
    }
    fn u8(&mut self, tag: u16, x: u8) -> &mut Set {
        self.p(tag, &[x])
    }
    fn u16(&mut self, tag: u16, x: u16) -> &mut Set {
        self.p(tag, &x.to_be_bytes())
    }
    fn u32(&mut self, tag: u16, x: u32) -> &mut Set {
        self.p(tag, &x.to_be_bytes())
    }
    fn i64(&mut self, tag: u16, x: i64) -> &mut Set {
        self.p(tag, &x.to_be_bytes())
    }
    fn rat(&mut self, tag: u16, r: Rational) -> &mut Set {
        self.p(tag, &rational(r))
    }
    fn refs(&mut self, tag: u16, uids: &[[u8; 16]]) -> &mut Set {
        let items: Vec<&[u8]> = uids.iter().map(|u| &u[..]).collect();
        self.p(tag, &batch(&items, 16))
    }
    fn write(&self, out: &mut Vec<u8>) {
        klv(out, &self.key, &self.v);
    }
}

/// An MCA label sub-descriptor (ST 377-4): an audio channel label (6Bh) or a soundfield group
/// label (6Ch).
fn mca_label(out: &mut Vec<u8>, uid: [u8; 16], m: &McaLabel) {
    let mut s = Set::new(if m.channel.is_some() { 0x6B } else { 0x6C }, uid);
    s.p(TAG_MCA, &m.dictionary).p(TAG_MCA + 1, &m.link).p(TAG_MCA + 2, &utf16(&m.symbol)).p(TAG_MCA + 3, &utf16(&m.name));
    if let Some(c) = m.channel {
        s.u32(TAG_MCA + 4, c);
    }
    if let Some(l) = &m.language {
        s.p(TAG_MCA + 5, l.as_bytes());
    }
    if let Some(g) = m.group {
        s.p(TAG_MCA + 6, &g);
    }
    s.write(out);
}

/// The JPEG 2000 picture sub-descriptor (ST 422 Table 7): SIZ values, the coding and
/// quantization defaults when known, and the J2CLayout (`layout`, IMF only).
fn j2k_sub_descriptor(out: &mut Vec<u8>, uid: [u8; 16], j: &J2kSubDescriptor, layout: Option<[u8; 16]>) {
    let mut s = Set::new(0x5A, uid);
    let sizing: Vec<[u8; 3]> = j.components.iter().map(|&(a, b, c)| [a, b, c]).collect();
    let items: Vec<&[u8]> = sizing.iter().map(|c| &c[..]).collect();
    s.u16(TAG_J2K_RSIZ, j.rsiz)
        .u32(TAG_J2K_RSIZ + 1, j.xsiz)
        .u32(TAG_J2K_RSIZ + 2, j.ysiz)
        .u32(TAG_J2K_RSIZ + 3, j.xosiz)
        .u32(TAG_J2K_RSIZ + 4, j.yosiz)
        .u32(TAG_J2K_RSIZ + 5, j.xtsiz)
        .u32(TAG_J2K_RSIZ + 6, j.ytsiz)
        .u32(TAG_J2K_RSIZ + 7, j.xtosiz)
        .u32(TAG_J2K_RSIZ + 8, j.ytosiz)
        .u16(TAG_J2K_RSIZ + 9, j.components.len() as u16)
        .p(TAG_J2K_RSIZ + 10, &batch(&items, 3));
    if let Some(cod) = &j.coding_style {
        s.p(TAG_J2K_RSIZ + 11, cod);
    }
    if let Some(qcd) = &j.quantization {
        s.p(TAG_J2K_RSIZ + 12, qcd);
    }
    if let Some(l) = layout {
        s.p(TAG_J2K_RSIZ + 13, &l);
    }
    s.write(out);
}

/// Final (or provisional, -1 = unknown) durations for the metadata.
#[derive(Clone, Copy, Debug)]
struct Durations {
    /// Edit units of the essence tracks.
    essence: i64,
}

impl<W: Write + Seek> MxfWriter<W> {
    /// Start a file: writes the header partition and the body partition pack.
    pub fn new(mut w: W, cfg: WriterConfig) -> io::Result<Self> {
        if !cfg.edit_rate.is_valid() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "MXF edit rate must be positive"));
        }
        // OP1a pads every sound track to a whole edit unit on flush. Bound that unit before a
        // low but technically positive edit rate can turn padding into a multi-gigabyte Vec.
        const MAX_PCM_EDIT_UNIT_BYTES: u128 = 64 << 20;
        for sound in &cfg.sound {
            let samples = (u128::from(sound.sample_rate) * cfg.edit_rate.den as u128).div_ceil(cfg.edit_rate.num as u128);
            let bytes = samples.saturating_mul(u128::from(sound.block_align()));
            if bytes > MAX_PCM_EDIT_UNIT_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "MXF PCM edit unit exceeds the allocation budget"));
            }
        }
        let n_essence = cfg.picture.is_some() as usize + cfg.sound.len() + cfg.iab.is_some() as usize;
        if cfg.iab.is_some() && cfg.pattern != Pattern::DCinema {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "immersive audio data is only written as a D-Cinema track file"));
        }
        if n_essence == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "MXF file without essence tracks"));
        }
        if matches!(cfg.pattern, Pattern::OpAtom | Pattern::DCinema | Pattern::Imf) && n_essence != 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "OP-Atom and IMF track files hold exactly one essence track"));
        }
        if cfg.pattern == Pattern::Imf {
            let invalid = |m: &str| io::Error::new(io::ErrorKind::InvalidInput, m.to_string());
            if cfg.rdd9 || matches!(cfg.picture.as_ref().map(|p| p.coding), Some(PictureCoding::D10 { .. })) {
                return Err(invalid("IMF track files are neither D-10 nor RDD 9"));
            }
            if let Some(sd) = cfg.sound.first() {
                // ST 2067-2 5.3.2: 24-bit, 48 or 96 kHz; ST 2067-5 Table 9: edit rate = sample rate
                if sd.bits != 24 || !matches!(sd.sample_rate, 48_000 | 96_000) {
                    return Err(invalid("IMF sound is 24-bit PCM at 48 or 96 kHz"));
                }
                if cfg.edit_rate != Rational::new(sd.sample_rate as i32, 1) {
                    return Err(invalid("IMF sound track files run at the sample rate"));
                }
            }
        }
        if cfg.sound.iter().any(|s| s.sample_rate == 0 || s.channels == 0 || !(1..=32).contains(&s.bits)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid PCM sound description"));
        }
        // clip wrapping: OP-Atom, and IMF sound (ST 2067-5 5.1.8)
        let clip = cfg.pattern == Pattern::OpAtom || cfg.pattern == Pattern::Imf && cfg.picture.is_none();
        let d10 = match cfg.picture.as_ref().map(|p| p.coding) {
            Some(coding @ PictureCoding::D10 { mbps, lines }) => {
                let invalid = |m: &str| io::Error::new(io::ErrorKind::InvalidInput, m.to_string());
                let geo = D10Geometry::of(mbps, lines).ok_or_else(|| invalid("D-10 is 30, 40 or 50 Mbit/s with 625 or 525 lines"))?;
                if cfg.pattern != Pattern::Op1a {
                    return Err(invalid("D-10 is written as OP1a"));
                }
                if cfg.edit_rate != geo.edit_rate {
                    return Err(invalid("D-10 625 runs at 25 Hz, 525 at 30000/1001 Hz"));
                }
                if cfg.sound.len() > 1 || cfg.sound.iter().any(|s| s.sample_rate != 48_000 || s.channels > 8 || !matches!(s.bits, 16 | 24)) {
                    return Err(invalid("D-10 carries one sound track of 1-8 channels, 48 kHz, 16 or 24 bit"));
                }
                let max_samples = if geo.edit_rate.den == 1 { 1920 } else { 1602 };
                Some(D10Layout {
                    geo,
                    label: coding.container(false).0,
                    system: kag_aligned(20 + 57 + 20),
                    picture: kag_aligned(20 + geo.frame_bytes),
                    sound: kag_aligned(20 + 4 + 32 * max_samples),
                    channels: cfg.sound.first().map_or(0, |s| s.channels as usize),
                    bits: cfg.sound.first().map_or(16, |s| s.bits),
                })
            }
            _ => None,
        };
        let rdd9 = if cfg.rdd9 {
            let invalid = |m: &str| io::Error::new(io::ErrorKind::InvalidInput, m.to_string());
            if cfg.pattern != Pattern::Op1a || !matches!(cfg.picture.as_ref().map(|p| p.coding), Some(PictureCoding::Mpeg2 { .. })) {
                return Err(invalid("RDD 9 is MPEG-2 video in OP1a"));
            }
            if cfg.sound.len() > 1 || cfg.sound.iter().any(|s| s.sample_rate != 48_000 || s.channels > 8 || !matches!(s.bits, 16 | 24)) {
                return Err(invalid("RDD 9 carries one sound track of 1-8 channels, 48 kHz, 16 or 24 bit"));
            }
            let channels = cfg.sound.first().map_or(0, |s| s.channels as usize);
            Some(Rdd9Layout {
                channels,
                // XDCAM HD422 equipment takes four or eight channels; the extra ones are silent
                elements: if channels <= 4 { 4 } else { 8 },
                bps: cfg.sound.first().map_or(3, |s| s.bits as usize / 8),
            })
        } else {
            None
        };
        let mut tracks = Vec::new();
        let mut id = cfg.first_track_id.max(2);
        if let Some(l) = &rdd9 {
            // RDD 9 Tables 2-5: picture element number 0, one AES3 frame-wrapped element per channel
            let Some(p) = &cfg.picture else { return Err(io::Error::new(io::ErrorKind::InvalidInput, "RDD 9 needs a picture")) };
            tracks.push(TrackW { id, kind: TrackKind::Picture, element_key: element_key(0x15, 1, 0x05, 0), container: p.coding.container(false).0 });
            id += 1;
            for k in 0..l.elements {
                tracks.push(TrackW { id, kind: TrackKind::Channel(k), element_key: element_key(0x16, l.elements as u8, 0x03, k as u8), container: EC_AES3 });
                id += 1;
            }
        } else if let Some(l) = &d10 {
            // ST 386 Tables 2, 3: SDTI-CP compatible picture (05h) and sound (06h) items
            tracks.push(TrackW { id, kind: TrackKind::Picture, element_key: element_key(0x05, 1, 0x01, 1), container: l.label });
            tracks.push(TrackW { id: id + 1, kind: TrackKind::Aes3, element_key: element_key(0x06, 1, 0x10, 1), container: l.label });
            id += 2;
        } else if let Some(p) = &cfg.picture {
            let (container, et) = p.coding.container(clip);
            tracks.push(TrackW { id, kind: TrackKind::Picture, element_key: element_key(0x15, 1, et, 1), container });
            id += 1;
        }
        if cfg.iab.is_some() {
            tracks.push(TrackW { id, kind: TrackKind::Iab, element_key: IAB_ELEMENT, container: EC_IAB });
            id += 1;
        }
        let ns = cfg.sound.len() as u8;
        for k in 0..if d10.is_some() || rdd9.is_some() { 0 } else { cfg.sound.len() } {
            let container = essence_container(0x01, 0x06, if clip { 0x02 } else { 0x01 }, 0x00);
            tracks.push(TrackW { id, kind: TrackKind::Sound(k), element_key: element_key(0x16, ns, if clip { 0x02 } else { 0x01 }, k as u8 + 1), container });
            id += 1;
        }
        let base = w.stream_position()?;
        let nsound = cfg.sound.len();
        let mut me = MxfWriter {
            w,
            base,
            pos: 0,
            cfg,
            tracks,
            header_region: 0,
            body_partition: 0,
            essence_start: 0,
            entries: Vec::new(),
            d10,
            rdd9,
            stream: 0,
            partitions: Vec::new(),
            segment_start: 0,
            imf_parts: Vec::new(),
            pending: VecDeque::new(),
            sound_buf: vec![Vec::new(); nsound],
            clip_len: 0,
            clip_kl: 0,
            samples: 0,
        };
        let meta = me.metadata(Durations { essence: -1 });
        me.header_region = meta.len() + if me.cfg.pattern == Pattern::Imf { IMF_HEADER_RESERVE } else { HEADER_RESERVE };
        let pack = me.partition_pack(2, 1, 0, 0, 0, me.header_region as u64, 0, 0, 0);
        if me.kag() > 1 {
            // ST 386 6.1, RDD 9 8.1: the essence partition starts on the KAG
            me.header_region = (pack.len() + me.header_region).next_multiple_of(D10_KAG) - pack.len();
        }
        let mut out = me.partition_pack(2, 1, 0, 0, 0, me.header_region as u64, 0, 0, 0);
        out.extend_from_slice(&meta);
        fill(&mut out, me.header_region - meta.len());
        me.put(&out)?;
        me.body_partition = me.pos;
        let mut body = me.partition_pack(3, 4, me.pos, 0, 0, 0, 0, 0, BODY_SID);
        if me.kag() > 1 {
            // the first content package starts on the KAG of its partition
            let n = body.len();
            fill(&mut body, kag_aligned(n) - n);
        }
        me.put(&body)?;
        me.essence_start = me.pos;
        if clip {
            // clip-wrapped element: key + 9-byte BER length, patched by finish()
            me.clip_kl = me.pos;
            let mut kl = me.tracks[0].element_key.to_vec();
            kl.push(0x88);
            kl.extend_from_slice(&[0; 8]);
            me.put(&kl)?;
        }
        Ok(me)
    }

    /// The configuration (package UMIDs and names included).
    pub fn config(&self) -> &WriterConfig {
        &self.cfg
    }

    /// The instance UIDs of the MCA label sub-descriptors, in [`WriterConfig::mca`] order (a CPL
    /// repeats them in its MCA metadata).
    pub fn mca_instance_ids(&self) -> Vec<[u8; 16]> {
        (0..self.cfg.mca.len()).map(|k| self.uid(3200 + k as u32)).collect()
    }

    /// Edit units written so far (content packages, or OP-Atom pictures).
    pub fn edit_units(&self) -> u64 {
        self.entries.len() as u64
    }

    fn put(&mut self, b: &[u8]) -> io::Result<()> {
        self.w.write_all(b)?;
        self.pos += b.len() as u64;
        Ok(())
    }

    fn uid(&self, n: u32) -> [u8; 16] {
        let mut u = self.cfg.ids.file.material_number();
        u[6] = (u[6] & 0x0F) | 0x40;
        u[8] = (u[8] & 0x3F) | 0x80;
        u[12..].copy_from_slice(&n.to_be_bytes());
        u
    }

    /// The KLV alignment grid: 512 bytes for SDTI-CP style content packages, else none.
    fn kag(&self) -> usize {
        if self.d10.is_some() || self.rdd9.is_some() { D10_KAG } else { 1 }
    }

    fn op_label(&self) -> [u8; 16] {
        match self.cfg.pattern {
            Pattern::Op1a => op1a(self.tracks.len() > 1),
            // ST 2067-5 5.2.1: internal, streamable, uni-track
            Pattern::Imf => op1a(false),
            Pattern::OpAtom | Pattern::DCinema => OP_ATOM,
        }
    }

    fn containers(&self) -> Vec<[u8; 16]> {
        let mut v: Vec<[u8; 16]> = Vec::new();
        for t in &self.tracks {
            if !v.contains(&t.container) {
                v.push(t.container);
            }
        }
        // D-Cinema track files list the multiple-mappings label too, as the reference
        // implementation's do (readers that take single-container OP-Atom as clip-wrapped
        // then read the frames, ffmpeg among them)
        // D-10 content packages are one essence container (ST 386 5.1)
        if (self.tracks.len() > 1 || self.cfg.pattern == Pattern::DCinema) && self.d10.is_none() {
            v.push(EC_MULTIPLE);
        }
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn partition_pack(&self, kind: u8, status: u8, this: u64, prev: u64, footer: u64, hbc: u64, ibc: u64, index_sid: u32, body_sid: u32) -> Vec<u8> {
        self.partition_pack_at(kind, status, this, prev, footer, hbc, ibc, index_sid, body_sid, 0)
    }

    /// A partition pack whose essence starts at stream offset `body_offset`.
    #[allow(clippy::too_many_arguments)]
    fn partition_pack_at(
        &self,
        kind: u8,
        status: u8,
        this: u64,
        prev: u64,
        footer: u64,
        hbc: u64,
        ibc: u64,
        index_sid: u32,
        body_sid: u32,
        body_offset: u64,
    ) -> Vec<u8> {
        let mut v = Vec::with_capacity(160);
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&3u16.to_be_bytes());
        v.extend_from_slice(&(self.kag() as u32).to_be_bytes()); // KAG size
        v.extend_from_slice(&this.to_be_bytes());
        v.extend_from_slice(&prev.to_be_bytes());
        v.extend_from_slice(&footer.to_be_bytes());
        v.extend_from_slice(&hbc.to_be_bytes());
        v.extend_from_slice(&ibc.to_be_bytes());
        v.extend_from_slice(&index_sid.to_be_bytes());
        v.extend_from_slice(&body_offset.to_be_bytes());
        v.extend_from_slice(&body_sid.to_be_bytes());
        v.extend_from_slice(&self.op_label());
        let cs = self.containers();
        let items: Vec<&[u8]> = cs.iter().map(|c| &c[..]).collect();
        v.extend_from_slice(&batch(&items, 16));
        let mut out = Vec::new();
        klv(&mut out, &partition_key(kind, status), &v);
        out
    }

    /// Sound sample frames of edit unit `i` of OP1a track `k`.
    fn unit_samples(&self, k: usize, i: u64) -> u64 {
        let sr = self.cfg.sound[k].sample_rate as u128;
        let r = self.cfg.edit_rate;
        let at = |n: u64| (n as u128 * sr * r.den as u128 / r.num as u128) as u64;
        at(i + 1) - at(i)
    }

    /// Duration of an OP-Atom sound track in edit units.
    fn sound_units(&self) -> i64 {
        let sr = self.cfg.sound.first().map_or(1, |s| s.sample_rate) as u128;
        let r = self.cfg.edit_rate;
        let num = self.samples as u128 * r.num as u128;
        let den = sr * r.den as u128;
        num.div_ceil(den.max(1)) as i64
    }

    fn essence_units(&self) -> i64 {
        match (self.cfg.pattern, self.cfg.picture.is_some()) {
            (Pattern::OpAtom | Pattern::Imf, false) => self.sound_units(),
            _ => self.entries.len() as i64,
        }
    }

    /// Push one picture (stored order). OP1a: content packages are written as soon as the sound
    /// of their edit unit is there too.
    pub fn push_picture(&mut self, data: &[u8], info: FrameInfo) -> io::Result<()> {
        if self.cfg.picture.is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "this MXF file has no picture track"));
        }
        if let Some(l) = &self.d10
            && data.len() > l.geo.frame_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("D-10 picture of {} bytes exceeds the {}-byte frame", data.len(), l.geo.frame_bytes),
            ));
        }
        match self.cfg.pattern {
            Pattern::Op1a | Pattern::DCinema => {
                self.pending.push_back((data.to_vec(), info));
                self.emit(false)
            }
            Pattern::OpAtom => {
                self.entries.push(Entry { offset: self.clip_len, slices: Vec::new(), info });
                self.put(data)?;
                self.clip_len += data.len() as u64;
                Ok(())
            }
            Pattern::Imf => self.push_imf_picture(data, info),
        }
    }

    /// Edit units in an IMF essence partition: 60 s (ST 2067-2 5.1.3).
    fn imf_partition_units(&self) -> usize {
        let r = self.cfg.edit_rate;
        usize::try_from((60 * i64::from(r.num) + i64::from(r.den) / 2) / i64::from(r.den.max(1))).unwrap_or(usize::MAX).max(1)
    }

    /// IMF picture: a frame-wrapped element; every 60 s an index partition for the essence
    /// partition just finished, then a new essence partition (ST 2067-2 5.1.3, 5.1.4).
    fn push_imf_picture(&mut self, data: &[u8], info: FrameInfo) -> io::Result<()> {
        if self.entries.len() - self.segment_start >= self.imf_partition_units() {
            self.imf_index_partition()?;
            let prev = self.imf_parts.last().map_or(self.body_partition, |p| p.this);
            let part = ImfPartition { this: self.pos, prev, index_bytes: 0, body_offset: self.stream };
            let pack = self.imf_pack(part, 0);
            self.put(&pack)?;
            self.imf_parts.push(part);
            self.segment_start = self.entries.len();
        }
        let mut el = Vec::with_capacity(data.len() + 20);
        klv(&mut el, &self.tracks[0].element_key, data);
        self.entries.push(Entry { offset: self.stream, slices: Vec::new(), info });
        self.put(&el)?;
        self.stream += el.len() as u64;
        Ok(())
    }

    /// An IMF index partition: the index table segments of the essence written since the last
    /// one (pictures), or the constant-rate index of the clip (sound).
    fn imf_index_partition(&mut self) -> io::Result<()> {
        let mut index = Vec::new();
        if self.cfg.picture.is_some() {
            let uid_base = 5000 + 64 * self.imf_parts.len() as u32;
            self.segment_index(&mut index, self.segment_start..self.entries.len(), uid_base);
        } else {
            index = self.index();
        }
        let prev = self.imf_parts.last().map_or(self.body_partition, |p| p.this);
        let part = ImfPartition { this: self.pos, prev, index_bytes: index.len() as u64, body_offset: 0 };
        let mut out = self.imf_pack(part, 0);
        out.extend_from_slice(&index);
        self.put(&out)?;
        self.imf_parts.push(part);
        Ok(())
    }

    /// The pack of an IMF body partition (`footer`: 0 while writing).
    fn imf_pack(&self, p: ImfPartition, footer: u64) -> Vec<u8> {
        if p.index_bytes > 0 {
            self.partition_pack_at(3, 4, p.this, p.prev, footer, 0, p.index_bytes, INDEX_SID, 0, 0)
        } else {
            self.partition_pack_at(3, 4, p.this, p.prev, footer, 0, 0, 0, BODY_SID, p.body_offset)
        }
    }

    /// Replace the immersive audio sub-descriptor values (known once every frame is written);
    /// [`MxfWriter::finish`] writes them.
    pub fn set_iab(&mut self, d: IabDesc) -> io::Result<()> {
        if self.cfg.iab.is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "this MXF file has no immersive audio track"));
        }
        self.cfg.iab = Some(d);
        Ok(())
    }

    /// Push one IABitstream frame (an immersive audio track file's edit unit).
    pub fn push_data(&mut self, data: &[u8]) -> io::Result<()> {
        if self.cfg.iab.is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "this MXF file has no immersive audio track"));
        }
        self.pending.push_back((data.to_vec(), FrameInfo::intra()));
        self.emit(false)
    }

    /// Whether the edit units are pictures or IAB frames (one pushed element each).
    fn framed(&self) -> bool {
        self.cfg.picture.is_some() || self.cfg.iab.is_some()
    }

    /// Push interleaved little-endian PCM of sound track `track` (whole sample frames).
    pub fn push_sound(&mut self, track: usize, pcm: &[u8]) -> io::Result<()> {
        let Some(sd) = self.cfg.sound.get(track).copied() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no sound track {track}")));
        };
        let ba = sd.block_align() as usize;
        if !pcm.len().is_multiple_of(ba) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "PCM data is not a whole number of sample frames"));
        }
        match self.cfg.pattern {
            Pattern::Op1a | Pattern::DCinema => {
                self.sound_buf[track].extend_from_slice(pcm);
                self.emit(false)
            }
            Pattern::OpAtom | Pattern::Imf => {
                self.put(pcm)?;
                self.clip_len += pcm.len() as u64;
                self.samples += (pcm.len() / ba) as u64;
                Ok(())
            }
        }
    }

    /// Write every complete OP1a content package (`flush`: pad the sound of the pending pictures
    /// with silence, or, without a picture track, write the remaining sound).
    fn emit(&mut self, flush: bool) -> io::Result<()> {
        if let Some(l) = self.d10 {
            return self.emit_d10(l, flush);
        }
        if let Some(l) = self.rdd9 {
            return self.emit_rdd9(l, flush);
        }
        let has_pic = self.framed();
        loop {
            let i = self.entries.len() as u64;
            if has_pic && self.pending.is_empty() {
                return Ok(());
            }
            let need: Vec<usize> = (0..self.cfg.sound.len()).map(|k| self.unit_samples(k, i) as usize * self.cfg.sound[k].block_align() as usize).collect();
            let enough = need.iter().zip(&self.sound_buf).all(|(n, b)| b.len() >= *n);
            if !enough && !flush {
                return Ok(());
            }
            if !has_pic && self.sound_buf.iter().all(Vec::is_empty) {
                return Ok(());
            }
            let pic = if has_pic { self.pending.pop_front() } else { None };
            let mut cp = Vec::new();
            let mut slices = Vec::new();
            if let Some((data, _)) = &pic {
                klv(&mut cp, &self.tracks[0].element_key, data);
            }
            let first_sound = has_pic as usize;
            for (k, n) in need.iter().enumerate() {
                let take = (*n).min(self.sound_buf[k].len());
                let mut chunk: Vec<u8> = self.sound_buf[k].drain(..take).collect();
                chunk.resize(*n, 0);
                if k > 0 || has_pic {
                    slices.push(cp.len() as u32);
                }
                klv(&mut cp, &self.tracks[first_sound + k].element_key, &chunk);
            }
            let info = pic.map(|p| p.1).unwrap_or_else(FrameInfo::intra);
            self.entries.push(Entry { offset: self.pos - self.essence_start, slices, info });
            self.stream += cp.len() as u64;
            self.put(&cp)?;
        }
    }

    /// Write every complete D-10 content package: system item (system metadata pack, empty
    /// package metadata set), the picture padded to the fixed frame size, the AES3 element, each
    /// item filled to the KAG (ST 386 5, 6.1).
    fn emit_d10(&mut self, l: D10Layout, flush: bool) -> io::Result<()> {
        let ba = self.cfg.sound.first().map_or(0, |s| s.block_align() as usize);
        loop {
            if self.pending.is_empty() {
                return Ok(());
            }
            let i = self.entries.len() as u64;
            let n = l.samples(i);
            if l.channels > 0 && self.sound_buf.first().map_or(0, Vec::len) < n * ba && !flush {
                return Ok(());
            }
            let Some((mut data, info)) = self.pending.pop_front() else { return Ok(()) };
            let mut cp = Vec::with_capacity(l.content_package());
            // system metadata pack (ST 385 5.2.3): UL, creation and user time stamps, picture
            // and sound items present
            let tc = self.cfg.timecode.unwrap_or(StartTimecode { frames: 0, rate: self.cfg.edit_rate, drop_frame: false });
            let stamp = timecode_stamp(tc.frames.saturating_add(i as i64), tc.rounded_base(), tc.drop_frame);
            system_item(&mut cp, 0x7C, l.geo.edit_rate, i, &l.label, &stamp, l.system);
            data.resize(l.geo.frame_bytes, 0);
            klv(&mut cp, &self.tracks[0].element_key, &data);
            fill_to(&mut cp, l.system + l.picture);
            let mut chunk = Vec::new();
            if let Some(buf) = self.sound_buf.first_mut() {
                let take = (n * ba).min(buf.len());
                chunk = buf.drain(..take).collect();
                chunk.resize(n * ba, 0);
            }
            let aes = aes3_element(&chunk, l.channels, l.bits, n, l.sequence(i));
            klv(&mut cp, &self.tracks[1].element_key, &aes);
            fill_to(&mut cp, l.content_package());
            self.entries.push(Entry { offset: self.stream, slices: Vec::new(), info });
            self.stream += cp.len() as u64;
            self.put(&cp)?;
        }
    }

    /// Write every complete RDD 9 content package: system item, picture, one AES3 element per
    /// channel, each filled to the KAG; at a key frame five seconds into a body partition, a new
    /// body partition with the index table segment of the previous one first.
    fn emit_rdd9(&mut self, l: Rdd9Layout, flush: bool) -> io::Result<()> {
        let ba = l.channels * l.bps;
        let rate = self.cfg.edit_rate;
        let interval = (RDD9_PARTITION_SECONDS * i64::from(rate.num) / i64::from(rate.den.max(1))).max(1) as usize;
        loop {
            let Some(key) = self.pending.front().map(|p| p.1.key) else { return Ok(()) };
            let i = self.entries.len() as u64;
            let n = self.unit_samples_48k(i);
            if l.channels > 0 && self.sound_buf.first().map_or(0, Vec::len) < n * ba && !flush {
                return Ok(());
            }
            if key && self.entries.len() - self.segment_start >= interval {
                self.new_partition()?;
            }
            let Some((data, info)) = self.pending.pop_front() else { return Ok(()) };
            let tc = self.cfg.timecode.unwrap_or(StartTimecode { frames: 0, rate, drop_frame: false });
            let stamp = timecode_stamp(tc.frames.saturating_add(i as i64), tc.rounded_base(), tc.drop_frame);
            let mut cp = Vec::new();
            // RDD 9 5.2.2: picture and sound items, the user stamp, no creation stamp
            system_item(&mut cp, 0x5C, rate, i, &EC_MULTIPLE, &stamp, D10_KAG);
            klv(&mut cp, &self.tracks[0].element_key, &data);
            let picture_end = kag_aligned(cp.len());
            fill_to(&mut cp, picture_end);
            let slice = cp.len() as u32;
            let mut pcm = Vec::new();
            if let Some(buf) = self.sound_buf.first_mut() {
                let take = (n * ba).min(buf.len());
                pcm = buf.drain(..take).collect();
                pcm.resize(n * ba, 0);
            }
            let el = self.rdd9_element_size(l);
            for k in 0..l.elements {
                let mut ch = vec![0u8; n * l.bps];
                if k < l.channels {
                    for (s, out) in ch.chunks_exact_mut(l.bps).enumerate() {
                        let at = s * ba + k * l.bps;
                        if let Some(src) = pcm.get(at..at + l.bps) {
                            out.copy_from_slice(src);
                        }
                    }
                }
                let at = cp.len();
                klv(&mut cp, &self.tracks[1 + k].element_key, &ch);
                fill_to(&mut cp, at + el);
            }
            self.entries.push(Entry { offset: self.stream, slices: vec![slice], info });
            self.stream += cp.len() as u64;
            self.put(&cp)?;
        }
    }

    /// Bytes of each RDD 9 AES3 element item (the KLV of the largest content package's samples
    /// and its fill to the KAG), so that every content package has the same sound slice.
    fn rdd9_element_size(&self, l: Rdd9Layout) -> usize {
        let r = self.cfg.edit_rate;
        let max = (48_000u64 * r.den.max(1) as u64).div_ceil(r.num.max(1) as u64) as usize;
        kag_aligned(20 + max * l.bps)
    }

    /// 48 kHz sample frames in edit unit `i`.
    fn unit_samples_48k(&self, i: u64) -> usize {
        let r = self.cfg.edit_rate;
        let at = |n: u64| (n as u128 * 48_000 * r.den.max(1) as u128 / r.num.max(1) as u128) as usize;
        at(i + 1) - at(i)
    }

    /// RDD 9 8.3.1: a body partition whose index table segment indexes the previous partition's
    /// edit units, then the essence of this one.
    fn new_partition(&mut self) -> io::Result<()> {
        let mut index = Vec::new();
        let uid_base = 5000 + 64 * self.partitions.len() as u32;
        self.segment_index(&mut index, self.segment_start..self.entries.len(), uid_base);
        let n = index.len();
        fill(&mut index, kag_aligned(n) - n);
        let bp = BodyPartition {
            this: self.pos,
            prev: self.partitions.last().map_or(self.body_partition, |b| b.this),
            index_bytes: index.len() as u64,
            body_offset: self.stream,
        };
        let mut out = self.body_pack(bp, 0);
        let n = out.len();
        fill(&mut out, kag_aligned(n) - n);
        out.extend_from_slice(&index);
        self.put(&out)?;
        self.partitions.push(bp);
        self.segment_start = self.entries.len();
        Ok(())
    }

    /// The pack of an RDD 9 body partition after the first (`footer`: 0 while writing).
    fn body_pack(&self, b: BodyPartition, footer: u64) -> Vec<u8> {
        self.partition_pack_at(3, 4, b.this, b.prev, footer, 0, b.index_bytes, INDEX_SID, BODY_SID, b.body_offset)
    }

    /// The index table segments of the content packages `range` (RDD 9 8.3: one per body
    /// partition), segment instance UIDs from `uid_base`.
    fn segment_index(&self, out: &mut Vec<u8>, range: std::ops::Range<usize>, uid_base: u32) {
        let rate = self.cfg.edit_rate;
        let mut seg_uid = uid_base;
        let mut segment = |out: &mut Vec<u8>, f: &mut dyn FnMut(&mut Set)| {
            let mut s = Set { key: INDEX_KEY, v: Vec::new() };
            s.p(0x3C0A, &self.uid(seg_uid));
            seg_uid += 1;
            s.rat(0x3F0B, rate);
            f(&mut s);
            s.u32(0x3F06, INDEX_SID).u32(0x3F07, BODY_SID);
            s.write(out);
        };
        self.entry_segments(out, &self.entries, range, &mut segment);
    }

    /// Header metadata (primer pack and sets).
    fn metadata(&self, d: Durations) -> Vec<u8> {
        let cfg = &self.cfg;
        let mut out = Vec::new();
        let iab_primer = if cfg.iab.is_some() { IAB_PRIMER } else { &[] };
        let mca_primer = if cfg.mca.is_empty() { &[] } else { MCA_PRIMER };
        let mpeg_primer = if matches!(cfg.picture.as_ref().map(|p| p.coding), Some(PictureCoding::Mpeg2 { .. })) { MPEG_PRIMER } else { &[] };
        let d10_primer = if self.d10.is_some() { D10_PRIMER } else { &[] };
        let primer: Vec<Vec<u8>> = PRIMER
            .iter()
            .chain(iab_primer)
            .chain(mca_primer)
            .chain(mpeg_primer)
            .chain(d10_primer)
            .map(|(t, ul)| {
                let mut e = t.to_be_bytes().to_vec();
                e.extend_from_slice(ul);
                e
            })
            .collect();
        let items: Vec<&[u8]> = primer.iter().map(Vec::as_slice).collect();
        klv(&mut out, &PRIMER_KEY, &batch(&items, 18));
        let ts = cfg.modified.bytes();
        let containers = self.containers();
        // Preface
        let mut s = Set::new(0x2F, self.uid(1));
        s.p(0x3B02, &ts).u16(0x3B05, 0x0103).u32(0x3B07, 1).refs(0x3B06, &[self.uid(2)]).p(0x3B03, &self.uid(3)).p(0x3B09, &self.op_label());
        if matches!(cfg.pattern, Pattern::DCinema | Pattern::Imf) {
            // ST 429-3 6.2, ST 2067-5 Table 9: the top-level file package is the primary package
            s.p(0x3B08, &self.uid(2000));
        }
        let ec: Vec<&[u8]> = containers.iter().map(|c| &c[..]).collect();
        s.p(0x3B0A, &batch(&ec, 16)).p(0x3B0B, &batch(&[], 16));
        s.write(&mut out);
        // Identification
        let mut s = Set::new(0x30, self.uid(2));
        s.p(0x3C09, &self.uid(100))
            .p(0x3C01, &utf16(&cfg.company))
            .p(0x3C02, &utf16(&cfg.product))
            .p(0x3C04, &utf16(&cfg.version))
            .p(0x3C05, &hash128(b"CFA4 MXF writer"))
            .p(0x3C06, &ts);
        s.write(&mut out);
        // Content storage, essence container data
        let mut s = Set::new(0x18, self.uid(3));
        s.refs(0x1901, &[self.uid(1000), self.uid(2000)]).refs(0x1902, &[self.uid(4)]);
        s.write(&mut out);
        let mut s = Set::new(0x23, self.uid(4));
        s.p(0x2701, &cfg.ids.file.0).u32(0x3F06, INDEX_SID).u32(0x3F07, BODY_SID);
        s.write(&mut out);
        // Packages
        for (pkg, base) in [(0u8, 1000u32), (1, 2000)] {
            let material = pkg == 0;
            let n = 1 + self.tracks.len() as u32;
            let track_uids: Vec<[u8; 16]> = (0..n).map(|k| self.uid(base + 1 + 3 * k)).collect();
            let mut s = Set::new(if material { 0x36 } else { 0x37 }, self.uid(base));
            let (umid, name) = if material { (cfg.ids.material, &cfg.ids.material_name) } else { (cfg.ids.file, &cfg.ids.file_name) };
            s.p(0x4401, &umid.0);
            if !name.is_empty() {
                s.p(0x4402, &utf16(name));
            }
            s.p(0x4405, &ts).p(0x4404, &ts).refs(0x4403, &track_uids);
            if !material {
                s.p(0x4701, &self.uid(3000));
            }
            s.write(&mut out);
            // timecode track (id 1)
            let tc = cfg.timecode.unwrap_or(StartTimecode { frames: 0, rate: cfg.edit_rate, drop_frame: false });
            let tc_rate = if cfg.picture.is_some() || !tc.rate.is_valid() { cfg.edit_rate } else { tc.rate };
            let tc_dur = if d.essence < 0 {
                -1
            } else {
                let er = cfg.edit_rate;
                {
                    let (a, b) = (d.essence as i128 * tc_rate.num as i128 * er.den as i128, (tc_rate.den as i128 * er.num as i128).max(1));
                    ((a + b - 1) / b) as i64
                }
            };
            self.track_sets(&mut out, base + 1, 1, 0, tc_rate, DEF_TIMECODE, tc_dur, if material { Some("TC1") } else { None }, |s| {
                s.u16(0x1502, tc.rounded_base()).i64(0x1501, tc.frames).u8(0x1503, tc.drop_frame as u8);
            });
            let mut vi = 0;
            let mut ai = 0;
            for (k, t) in self.tracks.iter().enumerate() {
                let (def, label) = match t.kind {
                    TrackKind::Picture => {
                        vi += 1;
                        (DEF_PICTURE, format!("V{vi}"))
                    }
                    TrackKind::Sound(_) | TrackKind::Aes3 | TrackKind::Channel(_) => {
                        ai += 1;
                        (DEF_SOUND, format!("A{ai}"))
                    }
                    TrackKind::Iab => (DEF_DATA, "D1".to_string()),
                };
                let number = if material { 0 } else { u32::from_be_bytes(t.element_key[12..16].try_into().unwrap_or([0; 4])) };
                let file = cfg.ids.file;
                self.track_sets(&mut out, base + 4 + 3 * k as u32, t.id, number, cfg.edit_rate, def, d.essence, material.then_some(label.as_str()), |s| {
                    let (src, src_track) = if material { (file, t.id) } else { (Umid([0; 32]), 0) };
                    s.i64(0x1201, 0).p(0x1101, &src.0).u32(0x1102, src_track);
                });
            }
        }
        // Descriptors
        let multiple = self.tracks.len() > 1;
        let mut desc_uids = Vec::new();
        for (k, t) in self.tracks.iter().enumerate() {
            let uid = if multiple { self.uid(3001 + k as u32) } else { self.uid(3000) };
            desc_uids.push(uid);
            match t.kind {
                TrackKind::Picture => {
                    // Picture tracks exist only with a picture configuration.
                    let Some(p) = cfg.picture.as_ref() else { continue };
                    if cfg.pattern == Pattern::Imf {
                        self.imf_picture_descriptor(&mut out, uid, t, p, d);
                        continue;
                    }
                    if p.color == ColorSpace::DcdmXyz {
                        self.rgba_descriptor(&mut out, uid, t, p, d);
                        continue;
                    }
                    let mpeg = match p.coding {
                        PictureCoding::Mpeg2 { profile_and_level, max_gop, b_pictures, closed_gop, bit_rate } => {
                            Some((profile_and_level, max_gop, b_pictures, closed_gop, bit_rate))
                        }
                        _ => None,
                    };
                    // the MPEG video descriptor extends the CDCI descriptor (ST 381-1 8.1)
                    let mut s = Set::new(if mpeg.is_some() { 0x51 } else { 0x28 }, uid);
                    if multiple {
                        s.u32(0x3006, t.id);
                    }
                    s.rat(0x3001, cfg.edit_rate);
                    if d.essence >= 0 {
                        s.i64(0x3002, d.essence);
                    }
                    let (tcl, ce, cp) = match p.color {
                        ColorSpace::Rec709 | ColorSpace::DcdmXyz => (TC_709, CE_709, CP_709),
                        ColorSpace::Rec2020Pq => (TC_PQ, CE_2020, CP_2020),
                        ColorSpace::Rec2020Hlg => (TC_HLG, CE_2020, CP_2020),
                    };
                    let depth = p.depth.clamp(8, 16);
                    let black = 16u32 << (depth - 8);
                    let white = 235u32 << (depth - 8);
                    // ST 377-1 G.2.20: the chroma range, (240 - 16) · 2^(n - 8) + 1
                    let mut range = (224u32 << (depth - 8)) + 1;
                    s.p(0x3004, &t.container);
                    if let Some(l) = &self.d10 {
                        // ST 386 Annex A Table A.1: separate fields of 304 (256) stored lines, the
                        // active picture below the vertical interval lines, ITU-R BT.601
                        let g = l.geo;
                        range = 225;
                        s.u8(0x3215, 1)
                            .u8(0x320C, 1)
                            .u32(0x3203, 720)
                            .u32(0x3202, g.field_lines)
                            .u32(0x3205, 720)
                            .u32(0x3204, g.field_lines)
                            .u32(0x3209, 720)
                            .u32(0x3208, g.display_lines)
                            .u32(0x320B, g.display_offset)
                            .rat(0x320E, p.aspect)
                            .p(0x320D, &batch(&[&g.line_map.0.to_be_bytes(), &g.line_map.1.to_be_bytes()], 4))
                            .p(0x3201, &p.coding.coding_label())
                            .p(0x3210, &TC_470)
                            .p(0x321A, &CE_601)
                            .u8(0x3212, 1)
                            .u8(0x3303, 4);
                    } else {
                        s.u8(0x320C, 0)
                            .u32(0x3203, p.width)
                            .u32(0x3202, p.height)
                            .u32(0x3205, p.width)
                            .u32(0x3204, p.height)
                            .u32(0x3209, p.width)
                            .u32(0x3208, p.height)
                            .rat(0x320E, p.aspect)
                            .p(0x320D, &batch(&[&0u32.to_be_bytes(), &0u32.to_be_bytes()], 4))
                            .p(0x3201, &p.coding.coding_label())
                            .p(0x3210, &tcl)
                            .p(0x321A, &ce)
                            .p(0x3219, &cp);
                    }
                    s.u32(0x3301, depth).u32(0x3302, p.subsampling.0).u32(0x3308, p.subsampling.1).u32(0x3304, black).u32(0x3305, white).u32(0x3306, range);
                    if let Some((pl, max_gop, b, closed, bit_rate)) = mpeg {
                        s.u8(TAG_MPEG, 1)
                            .u8(TAG_MPEG + 1, 0)
                            .u8(TAG_MPEG + 2, 1)
                            .u8(TAG_MPEG + 3, u8::from(b == 0))
                            .u8(TAG_MPEG + 4, u8::from(closed))
                            .u8(TAG_MPEG + 5, 0)
                            .u16(TAG_MPEG + 6, max_gop)
                            .u16(TAG_MPEG + 7, u16::from(b))
                            .u8(TAG_MPEG + 8, pl);
                        if bit_rate > 0 {
                            s.u32(TAG_MPEG + 9, bit_rate);
                        }
                    }
                    s.write(&mut out);
                }
                TrackKind::Aes3 => {
                    // ST 386 Annex A Table A.2: generic sound descriptor, 4 or 8 channels
                    let Some(l) = &self.d10 else { continue };
                    let mut s = Set::new(0x42, uid);
                    s.u32(0x3006, t.id).rat(0x3001, cfg.edit_rate);
                    if d.essence >= 0 {
                        s.i64(0x3002, d.essence);
                    }
                    s.p(0x3004, &t.container)
                        .rat(0x3D03, Rational::new(48_000, 1))
                        .u8(0x3D02, 1)
                        .u32(0x3D07, if l.channels > 4 { 8 } else { 4 })
                        .u32(0x3D01, l.bits);
                    s.write(&mut out);
                }
                TrackKind::Channel(_) => {
                    // RDD 9 Table 14 (bmx): an AES3 audio descriptor per mono channel track
                    let Some(l) = &self.rdd9 else { continue };
                    let mut s = Set::new(0x47, uid);
                    s.u32(0x3006, t.id).rat(0x3001, cfg.edit_rate);
                    if d.essence >= 0 {
                        s.i64(0x3002, d.essence);
                    }
                    let bps = l.bps as u32;
                    s.p(0x3004, &t.container)
                        .rat(0x3D03, Rational::new(48_000, 1))
                        .u8(0x3D02, 1)
                        .u32(0x3D07, 1)
                        .u32(0x3D01, bps * 8)
                        .u16(0x3D0A, bps as u16)
                        .u32(0x3D09, bps * 48_000);
                    s.write(&mut out);
                }
                TrackKind::Iab => {
                    let Some(ia) = cfg.iab else { continue };
                    self.iab_descriptor(&mut out, uid, t, ia, d);
                }
                TrackKind::Sound(i) => {
                    let sd = cfg.sound[i];
                    let mut s = Set::new(0x48, uid);
                    if multiple || cfg.pattern == Pattern::DCinema {
                        s.u32(0x3006, t.id);
                    }
                    s.rat(0x3001, cfg.edit_rate);
                    if d.essence >= 0 {
                        s.i64(0x3002, d.essence);
                    }
                    s.p(0x3004, &t.container)
                        .rat(0x3D03, Rational::new(sd.sample_rate as i32, 1))
                        .u8(0x3D02, 1)
                        .u32(0x3D07, sd.channels)
                        .u32(0x3D01, sd.bits)
                        .u16(0x3D0A, sd.block_align() as u16)
                        .u32(0x3D09, sd.block_align() * sd.sample_rate);
                    // ST 2067-2 5.3.4.2: IMF track files signal the MCA framework
                    if let Some(ca) = cfg.channel_assignment.or((cfg.pattern == Pattern::Imf).then_some(CA_IMF_MCA)) {
                        s.p(0x3D32, &ca);
                    }
                    if cfg.pattern == Pattern::Imf {
                        if let Some(r) = cfg.reference_image_edit_rate {
                            s.rat(TAG_REF_IMAGE_EDIT_RATE, r);
                        }
                        if let Some(l) = cfg.reference_audio_alignment_level {
                            s.p(TAG_REF_IMAGE_EDIT_RATE + 1, &l.to_be_bytes());
                        }
                    }
                    let mca_uids = self.mca_instance_ids();
                    if !mca_uids.is_empty() {
                        s.refs(TAG_SUB_DESCRIPTORS, &mca_uids);
                    }
                    s.write(&mut out);
                    for (m, uid) in cfg.mca.iter().zip(mca_uids) {
                        mca_label(&mut out, uid, m);
                    }
                }
            }
        }
        if multiple {
            let mut s = Set::new(0x44, self.uid(3000));
            s.rat(0x3001, cfg.edit_rate);
            if d.essence >= 0 {
                s.i64(0x3002, d.essence);
            }
            s.p(0x3004, &self.d10.map_or(EC_MULTIPLE, |l| l.label)).refs(0x3F01, &desc_uids);
            s.write(&mut out);
        }
        out
    }

    /// The immersive audio data essence descriptor (ST 429-18 Table 3, a generic data essence
    /// descriptor) and its sub-descriptor (Table 5).
    fn iab_descriptor(&self, out: &mut Vec<u8>, uid: [u8; 16], t: &TrackW, ia: IabDesc, d: Durations) {
        let sub_uid = self.uid(3100);
        let mut s = Set { key: IAB_DESCRIPTOR, v: Vec::new() };
        s.p(0x3C0A, &uid).refs(TAG_SUB_DESCRIPTORS, &[sub_uid]).u32(0x3006, t.id).rat(0x3001, self.cfg.edit_rate);
        if d.essence >= 0 {
            s.i64(0x3002, d.essence);
        }
        s.p(0x3004, &t.container).p(0x3E01, &IAB_CODING);
        s.write(out);
        let mut s = Set { key: IAB_SUB_DESCRIPTOR, v: Vec::new() };
        s.p(0x3C0A, &sub_uid)
            .u8(TAG_IAB_VERSION, ia.version)
            .u16(TAG_IAB_VERSION + 1, ia.max_channels)
            .u16(TAG_IAB_VERSION + 2, ia.max_objects)
            .u32(TAG_IAB_VERSION + 3, ia.first_frame)
            .rat(TAG_IAB_VERSION + 4, Rational::new(ia.sample_rate as i32, 1));
        s.write(out);
    }

    /// The RGBA picture essence descriptor of a D-Cinema X'Y'Z' track (ST 429-4 Table 4) and its
    /// JPEG 2000 picture sub-descriptor (ST 422 Table 7).
    fn rgba_descriptor(&self, out: &mut Vec<u8>, uid: [u8; 16], t: &TrackW, p: &PictureDesc, d: Durations) {
        let sub_uid = self.uid(3100);
        let mut s = Set::new(0x29, uid);
        s.u32(0x3006, t.id).rat(0x3001, self.cfg.edit_rate);
        if d.essence >= 0 {
            s.i64(0x3002, d.essence);
        }
        let max = (1u32 << p.depth.clamp(1, 16)) - 1;
        s.p(0x3004, &t.container)
            .u8(0x320C, 0)
            .u32(0x3203, p.width)
            .u32(0x3202, p.height)
            .rat(0x320E, p.aspect)
            .p(0x320D, &batch(&[&0u32.to_be_bytes(), &0u32.to_be_bytes()], 4))
            .p(0x3201, &p.coding.coding_label())
            .p(0x3210, &TC_DCDM)
            .u32(0x3406, max)
            .u32(0x3407, 0)
            // X', Y', Z' (D8h, D9h, DAh) of `depth` bits each
            .p(0x3401, &[0xD8, p.depth as u8, 0xD9, p.depth as u8, 0xDA, p.depth as u8, 0, 0]);
        if p.j2k.is_some() {
            s.refs(TAG_SUB_DESCRIPTORS, &[sub_uid]);
        }
        s.write(out);
        if let Some(j) = &p.j2k {
            j2k_sub_descriptor(out, sub_uid, j, None);
        }
    }

    /// The picture descriptor of an IMF image track file (ST 2067-21 7.2-7.5): RGBA for R'G'B'
    /// with the component reference levels of the quantization, CDCI for Y'C'BC'R with the
    /// black, white and colour range levels; the colour labels; the JPEG 2000 sub-descriptor
    /// with the J2CLayout.
    fn imf_picture_descriptor(&self, out: &mut Vec<u8>, uid: [u8; 16], t: &TrackW, p: &PictureDesc, d: Durations) {
        let sub_uid = self.uid(3100);
        let depth = p.depth.clamp(8, 16);
        let (tcl, ce, cp) = match p.color {
            ColorSpace::Rec709 | ColorSpace::DcdmXyz => (TC_709, CE_709, CP_709),
            ColorSpace::Rec2020Pq => (TC_PQ, CE_2020, CP_2020),
            ColorSpace::Rec2020Hlg => (TC_HLG, CE_2020, CP_2020),
        };
        let mut s = Set::new(if p.rgb { 0x29 } else { 0x28 }, uid);
        s.u32(0x3006, t.id).rat(0x3001, self.cfg.edit_rate);
        if d.essence >= 0 {
            s.i64(0x3002, d.essence);
        }
        s.p(0x3004, &t.container)
            .u8(0x320C, 0)
            .u32(0x3203, p.width)
            .u32(0x3202, p.height)
            .u32(0x3209, p.width)
            .u32(0x3208, p.height)
            .rat(0x320E, p.aspect)
            .p(0x320D, &batch(&[&0u32.to_be_bytes(), &0u32.to_be_bytes()], 4))
            .p(0x3201, &p.coding.coding_label())
            .p(0x3210, &tcl)
            .p(0x3219, &cp);
        let n = depth as u8;
        let layout = if p.rgb {
            // Table 10: QE.1 narrow or QE.2 full range
            let (min, max) = if p.full_range { (0, (1u32 << depth) - 1) } else { (16u32 << (depth - 8), 235u32 << (depth - 8)) };
            s.u32(0x3406, max).u32(0x3407, min).u8(0x3405, 0).p(0x3401, &[b'R', n, b'G', n, b'B', n, 0, 0]);
            [b'R', n, b'G', n, b'B', n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        } else {
            // Table 12 (COLOR.1-5, 7, 8)
            s.p(0x321A, &ce)
                .u32(0x3301, depth)
                .u32(0x3302, p.subsampling.0)
                .u32(0x3308, 1)
                .u8(0x3303, 0)
                .u32(0x3304, 16 << (depth - 8))
                .u32(0x3305, 235 << (depth - 8))
                .u32(0x3306, (224 << (depth - 8)) + 1);
            [b'Y', n, b'U', n, b'V', n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        };
        if p.j2k.is_some() {
            s.refs(TAG_SUB_DESCRIPTORS, &[sub_uid]);
        }
        s.write(out);
        if let Some(j) = &p.j2k {
            j2k_sub_descriptor(out, sub_uid, j, Some(j.layout.unwrap_or(layout)));
        }
    }

    /// A track, its sequence and its one component (`component` adds the component-specific
    /// properties: source clip or timecode).
    #[allow(clippy::too_many_arguments)]
    fn track_sets(
        &self,
        out: &mut Vec<u8>,
        uid0: u32,
        id: u32,
        number: u32,
        rate: Rational,
        def: [u8; 16],
        duration: i64,
        name: Option<&str>,
        component: impl FnOnce(&mut Set),
    ) {
        let (tu, su, cu) = (self.uid(uid0), self.uid(uid0 + 1), self.uid(uid0 + 2));
        let mut t = Set::new(0x3B, tu);
        t.u32(0x4801, id).u32(0x4804, number);
        if let Some(n) = name {
            t.p(0x4802, &utf16(n));
        }
        t.rat(0x4B01, rate).i64(0x4B02, 0).p(0x4803, &su);
        t.write(out);
        let mut s = Set::new(0x0F, su);
        s.p(0x0201, &def).i64(0x0202, duration).refs(0x1001, &[cu]);
        s.write(out);
        let mut c = Set::new(if def == DEF_TIMECODE { 0x14 } else { 0x11 }, cu);
        c.p(0x0201, &def).i64(0x0202, duration);
        component(&mut c);
        c.write(out);
    }

    /// Index table segments for the essence written.
    fn index(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let rate = self.cfg.edit_rate;
        let mut seg_uid = 4000;
        let mut segment = |out: &mut Vec<u8>, f: &mut dyn FnMut(&mut Set)| {
            let mut s = Set { key: INDEX_KEY, v: Vec::new() };
            s.p(0x3C0A, &self.uid(seg_uid));
            seg_uid += 1;
            s.rat(0x3F0B, rate);
            f(&mut s);
            s.u32(0x3F06, INDEX_SID).u32(0x3F07, BODY_SID);
            s.write(out);
        };
        // D-10: constant content packages, delta entries for the system, picture and sound items
        // (ST 386 6.2, Table 5) and no index entries
        if let Some(l) = self.d10 {
            let units = self.essence_units();
            let deltas: Vec<[u8; 6]> = [0, l.system, l.system + l.picture]
                .iter()
                .map(|&o| {
                    let b = (o as u32).to_be_bytes();
                    [0, 0, b[0], b[1], b[2], b[3]]
                })
                .collect();
            let items: Vec<&[u8]> = deltas.iter().map(|d| &d[..]).collect();
            let delta = batch(&items, 6);
            segment(&mut out, &mut |s| {
                s.i64(0x3F0C, 0).i64(0x3F0D, units).u32(0x3F05, l.content_package() as u32).u8(0x3F08, 0).u8(0x3F0E, 0).p(0x3F09, &delta);
            });
            return out;
        }
        // RDD 9: the footer indexes the last body partition
        if self.rdd9.is_some() {
            self.segment_index(&mut out, self.segment_start..self.entries.len(), 4000);
            return out;
        }
        // OP-Atom sound at one sample per edit unit: constant bytes per edit unit
        if matches!(self.cfg.pattern, Pattern::OpAtom | Pattern::Imf) && self.cfg.picture.is_none() {
            let sd = self.cfg.sound[0];
            let per_unit = (sd.sample_rate as i64 * rate.den as i64) % rate.num as i64 == 0 && (sd.sample_rate as i64 * rate.den as i64) / rate.num as i64 == 1;
            if per_unit {
                let units = self.essence_units();
                segment(&mut out, &mut |s| {
                    s.i64(0x3F0C, 0).i64(0x3F0D, units).u32(0x3F05, sd.block_align()).u8(0x3F08, 0).u8(0x3F0E, 0);
                });
                return out;
            }
            // other edit rates: one entry per edit unit from the sample counts
            let units = self.essence_units().max(0) as u64;
            let ba = sd.block_align() as u64;
            let entries: Vec<Entry> = (0..units)
                .map(|i| {
                    let s = (i as u128 * sd.sample_rate as u128 * rate.den as u128 / rate.num as u128) as u64;
                    Entry { offset: s * ba, slices: Vec::new(), info: FrameInfo::intra() }
                })
                .collect();
            self.entry_segments(&mut out, &entries, 0..entries.len(), &mut segment);
            return out;
        }
        self.entry_segments(&mut out, &self.entries, 0..self.entries.len(), &mut segment);
        out
    }

    /// Index table segments of `entries[range]` (temporal offsets from all of `entries`).
    fn entry_segments(
        &self,
        out: &mut Vec<u8>,
        entries: &[Entry],
        range: std::ops::Range<usize>,
        segment: &mut dyn FnMut(&mut Vec<u8>, &mut dyn FnMut(&mut Set)),
    ) {
        let n = entries.len();
        let nslices = entries.iter().map(|e| e.slices.len()).max().unwrap_or(0);
        // temporal offsets: display d is stored at d + TO[d]
        let mut stored_of = vec![usize::MAX; n];
        let mut valid = true;
        for (s, e) in entries.iter().enumerate() {
            let d = e.info.display.unwrap_or(s as i64);
            if d < 0 || d as usize >= n || stored_of[d as usize] != usize::MAX {
                valid = false;
                break;
            }
            stored_of[d as usize] = s;
        }
        let intra = self.cfg.picture.as_ref().is_none_or(|p| p.coding.intra_only());
        // AVC and MPEG-2 random access points carry their sequence header (flag bit 6, ST 381-1 A.2)
        let avc = matches!(self.cfg.picture.as_ref().map(|p| p.coding), Some(PictureCoding::Avc { .. } | PictureCoding::Mpeg2 { .. }));
        let mut rows: Vec<Vec<u8>> = Vec::with_capacity(n);
        let mut last_key = 0usize;
        for (s, e) in entries.iter().enumerate() {
            let to = if valid { (stored_of[s] as i64 - s as i64).clamp(-128, 127) as i8 } else { 0 };
            if e.info.key || intra {
                last_key = s;
            }
            let kfo = (last_key as i64 - s as i64).max(-128) as i8;
            let flags: u8 = if e.info.key || intra {
                if avc { 0xC0 } else { 0x80 }
            } else {
                match e.info.kind {
                    CodedKind::Intra => 0x00,
                    CodedKind::Predicted => 0x22,
                    CodedKind::Bidirectional => 0x33,
                }
            };
            let mut r = vec![to as u8, kfo as u8, flags];
            r.extend_from_slice(&e.offset.to_be_bytes());
            for k in 0..nslices {
                r.extend_from_slice(&e.slices.get(k).copied().unwrap_or(0).to_be_bytes());
            }
            rows.push(r);
        }
        // delta entries: element k of the content package is in slice k (picture in slice 0)
        let n_el = if self.cfg.pattern == Pattern::Op1a { self.tracks.len() } else { 1 };
        let delta_entry = |pos: i8, slice: u8, delta: usize| {
            let b = (delta as u32).to_be_bytes();
            [pos as u8, slice, b[0], b[1], b[2], b[3]]
        };
        let deltas: Vec<[u8; 6]> = match self.rdd9 {
            // RDD 9 8.3.2 (as bmx writes it): the system item and the picture (reordered) in
            // slice 0, the AES3 elements in slice 1
            Some(l) => {
                let el = self.rdd9_element_size(l);
                let mut v = vec![delta_entry(0, 0, 0), delta_entry(-1, 0, D10_KAG)];
                v.extend((0..l.elements).map(|k| delta_entry(0, 1, k * el)));
                v
            }
            None => (0..n_el).map(|k| delta_entry(0, k.min(nslices) as u8, 0)).collect(),
        };
        let delta_items: Vec<&[u8]> = deltas.iter().map(|d| &d[..]).collect();
        let delta = batch(&delta_items, 6);
        let entry_size = 11 + 4 * nslices;
        let per_seg = (MAX_SEGMENT_BYTES / entry_size).max(1);
        let n = range.end.min(n);
        let mut start = range.start.min(n);
        loop {
            let end = (start + per_seg).min(n);
            let items: Vec<&[u8]> = rows[start..end].iter().map(Vec::as_slice).collect();
            let arr = batch(&items, entry_size);
            segment(out, &mut |s: &mut Set| {
                s.i64(0x3F0C, start as i64)
                    .i64(0x3F0D, (end - start) as i64)
                    .u32(0x3F05, 0)
                    .u8(0x3F08, nslices as u8)
                    .u8(0x3F0E, 0)
                    .p(0x3F09, &delta)
                    .p(0x3F0A, &arr);
            });
            start = end;
            if start >= n {
                break;
            }
        }
    }

    /// Finish the file: footer partition with the final header metadata and the index, random
    /// index pack, and the closed header. Returns the sink (positioned at the end).
    pub fn finish(mut self) -> io::Result<W> {
        let imf = self.cfg.pattern == Pattern::Imf;
        let imf_picture = imf && self.cfg.picture.is_some();
        if matches!(self.cfg.pattern, Pattern::Op1a | Pattern::DCinema) || imf_picture {
            if !imf {
                self.emit(true)?;
            }
        } else {
            // patch the clip element length
            let end = self.pos;
            self.w.seek(SeekFrom::Start(self.base + self.clip_kl + 16))?;
            let mut l = vec![0x88];
            l.extend_from_slice(&self.clip_len.to_be_bytes());
            self.w.write_all(&l)?;
            self.w.seek(SeekFrom::Start(self.base + end))?;
        }
        let d = Durations { essence: self.essence_units() };
        let meta = self.metadata(d);
        // IMF: the index partition follows the last essence partition (ST 2067-2 5.1.4); the
        // footer holds neither metadata nor index
        if imf {
            self.imf_index_partition()?;
        }
        let index = if imf { Vec::new() } else { self.index() };
        let footer = self.pos;
        // D-Cinema: header metadata in the header partition only (ST 429-3 9.1)
        let footer_meta: &[u8] = if matches!(self.cfg.pattern, Pattern::DCinema | Pattern::Imf) { &[] } else { &meta };
        let last_body = self.imf_parts.last().map(|p| p.this).or(self.partitions.last().map(|b| b.this)).unwrap_or(self.body_partition);
        let index_sid = if index.is_empty() { 0 } else { INDEX_SID };
        let pack = self.partition_pack(4, 4, footer, last_body, footer, footer_meta.len() as u64, index.len() as u64, index_sid, 0);
        let mut out = pack;
        out.extend_from_slice(footer_meta);
        out.extend_from_slice(&index);
        // random index pack: (BodySID, offset) per partition, then the pack's length
        let mut rip = Vec::new();
        let mut parts = vec![(0u32, 0u64), (BODY_SID, self.body_partition)];
        parts.extend(self.partitions.iter().map(|b| (BODY_SID, b.this)));
        parts.extend(self.imf_parts.iter().map(|p| (if p.index_bytes > 0 { 0 } else { BODY_SID }, p.this)));
        parts.push((0, footer));
        for (sid, off) in parts {
            rip.extend_from_slice(&sid.to_be_bytes());
            rip.extend_from_slice(&off.to_be_bytes());
        }
        rip.extend_from_slice(&((16 + 4 + rip.len() + 4) as u32).to_be_bytes());
        klv(&mut out, &RIP_KEY, &rip);
        self.put(&out)?;
        let end = self.pos;
        // closed, complete header partition (same size as the provisional one)
        if meta.len() + 20 <= self.header_region {
            let mut h = self.partition_pack(2, 4, 0, 0, footer, self.header_region as u64, 0, 0, 0);
            h.extend_from_slice(&meta);
            fill(&mut h, self.header_region - meta.len());
            self.w.seek(SeekFrom::Start(self.base))?;
            self.w.write_all(&h)?;
        } else {
            // cannot happen (metadata sizes do not depend on the durations); keep the header open
            let h = self.partition_pack(2, 1, 0, 0, footer, self.header_region as u64, 0, 0, 0);
            self.w.seek(SeekFrom::Start(self.base))?;
            self.w.write_all(&h)?;
        }
        let body = self.partition_pack(3, 4, self.body_partition, 0, footer, 0, 0, 0, BODY_SID);
        self.w.seek(SeekFrom::Start(self.base + self.body_partition))?;
        self.w.write_all(&body)?;
        for b in &self.partitions {
            let pack = self.body_pack(*b, footer);
            self.w.seek(SeekFrom::Start(self.base + b.this))?;
            self.w.write_all(&pack)?;
        }
        for p in &self.imf_parts {
            let pack = self.imf_pack(*p, footer);
            self.w.seek(SeekFrom::Start(self.base + p.this))?;
            self.w.write_all(&pack)?;
        }
        self.w.seek(SeekFrom::Start(self.base + end))?;
        self.w.flush()?;
        Ok(self.w)
    }
}

/// Interleave planar f32 samples as little-endian signed PCM of `bits` (16, 24 or 32; 8-bit is
/// unsigned as in WAVE).
pub fn encode_pcm(planar: &[Vec<f32>], bits: u32) -> Vec<u8> {
    let n = planar.iter().map(Vec::len).min().unwrap_or(0);
    let bps = bits.div_ceil(8).clamp(1, 4) as usize;
    let mut out = Vec::with_capacity(n * planar.len() * bps);
    for i in 0..n {
        for c in planar {
            // f32 products, as the WAV / QuickTime PCM writers quantise
            let s = c[i].clamp(-1.0, 1.0);
            match bps {
                1 => out.push(((s * 127.0).round() as i32 + 128) as u8),
                2 => out.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes()),
                3 => out.extend_from_slice(&((s * 8_388_607.0).round() as i32).to_le_bytes()[..3]),
                _ => out.extend_from_slice(&((s as f64 * 2_147_483_647.0).round() as i32).to_le_bytes()),
            }
        }
    }
    out
}

/// Options of [`write_opatom_pcm`].
#[derive(Clone, Debug, PartialEq)]
pub struct OpAtomPcm {
    pub sample_rate: u32,
    /// 16 or 24 (any 8–32).
    pub bits: u32,
    pub channels: u32,
    /// Edit rate of the track (`None` = the sample rate: one edit unit per sample frame).
    pub edit_rate: Option<Rational>,
    pub ids: PackageIds,
    pub timecode: Option<StartTimecode>,
}

/// A complete OP-Atom PCM file in memory. `samples` are interleaved integer sample values in the
/// range of `bits` (e.g. -32768..=32767 for 16 bits).
pub fn write_opatom_pcm(opts: &OpAtomPcm, samples: &[i32]) -> io::Result<Vec<u8>> {
    let sd = SoundDesc { sample_rate: opts.sample_rate, channels: opts.channels.max(1), bits: opts.bits };
    let rate = opts.edit_rate.unwrap_or(Rational::new(opts.sample_rate as i32, 1));
    let mut cfg = WriterConfig::new(Pattern::OpAtom, rate, opts.ids.clone());
    cfg.sound = vec![sd];
    cfg.timecode = opts.timecode;
    let mut w = MxfWriter::new(io::Cursor::new(Vec::new()), cfg)?;
    let bps = sd.bits.div_ceil(8).clamp(1, 4) as usize;
    let frames = samples.len() / sd.channels as usize;
    let mut pcm = Vec::with_capacity(frames * sd.channels as usize * bps);
    for &s in &samples[..frames * sd.channels as usize] {
        if bps == 1 {
            pcm.push((s + 128) as u8);
        } else {
            pcm.extend_from_slice(&s.to_le_bytes()[..bps]);
        }
    }
    w.push_sound(0, &pcm)?;
    Ok(w.finish()?.into_inner())
}
