// Port of the MPEG-4 Simple Studio Profile parts of FFmpeg n7.1.1 (libavcodec/mpeg4videodec.c,
// h263dec.c, ituh263dec.c, simple_idct_template.c, vlc.c), LGPL-2.1-or-later
// (`COPYING.LGPL-2.1`); see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for the **MPEG-4 Simple Studio Profile** (SStP, ISO/IEC 14496-2 Amd. 1),
//! the codec of Sony's HDCAM SR and SRMASTER ("Sony SR"), as FFmpeg's `mpeg4` decoder reads
//! it: intra-only 10-bit 4:2:2, 4:4:4 or RGB pictures of DCT macroblocks (studio VLCs, 32-bit
//! simple IDCT) or lossless DPCM macroblocks, in slices.
//!
//! [`Decoder::decode`] takes one access unit (the headers before a picture may come with it, or
//! with an earlier one) and gives the picture as planar 10-bit samples. Every value from the
//! stream is hostile: a damaged picture is an [`Error`], never a panic. In strict mode
//! ([`Decoder::set_strict`]) the decoder also refuses pictures whose decoding FFmpeg leaves
//! undefined or conceals (a read past the end, an integer overflow, a macroblock no slice
//! covers), so that what it accepts decodes exactly as FFmpeg's does.

#[allow(clippy::all)]
mod tables;

use std::sync::OnceLock;

/// Errors of a picture.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid MPEG-4 studio stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported MPEG-4 stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid<T>(what: &'static str) -> Result<T> {
    Err(Error::Invalid(what))
}

const VOS_STARTCODE: u32 = 0x1B0;
const USER_DATA_STARTCODE: u32 = 0x1B2;
const GOP_STARTCODE: u32 = 0x1B3;
const VISUAL_OBJ_STARTCODE: u32 = 0x1B5;
const VOP_STARTCODE: u32 = 0x1B6;
const SLICE_STARTCODE: u32 = 0x1B7;
const EXT_STARTCODE: u32 = 0x1B8;
const QUANT_MATRIX_EXT_ID: u32 = 3;
const PROFILE_SIMPLE_STUDIO: u32 = 14;
const SIMPLE_STUDIO_VO_TYPE: u32 = 14;
const CORE_STUDIO_VO_TYPE: u32 = 15;
/// The largest picture side decoded (SStP allows 16383; Sony SR is at most 4096).
const MAX_SIDE: usize = 8192;

/// How the picture's components are sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    /// Y'CbCr 4:2:2.
    Yuv422,
    /// Y'CbCr 4:4:4.
    Yuv444,
    /// G, B, R planes (FFmpeg's `gbrp10`).
    Rgb,
}

/// A decoded picture: three planes of 10-bit samples (`Y`, `Cb`, `Cr`, or `G`, `B`, `R`),
/// `width` × `height` luma, chroma `(width + 1) / 2` wide in 4:2:2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub chroma: Chroma,
    pub planes: [Vec<u16>; 3],
}

impl Picture {
    /// The width of plane `i`.
    pub fn plane_width(&self, i: usize) -> usize {
        if i > 0 && self.chroma == Chroma::Yuv422 { self.width.div_ceil(2) } else { self.width }
    }
}

/// An MSB-first bit reader with FFmpeg's checked-reader semantics: bits past the end read as
/// zero, and the position stops 8 bits past it.
#[derive(Clone)]
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    size: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, size: data.len() * 8 }
    }

    fn bit_at(&self, p: usize) -> u32 {
        self.data.get(p / 8).map_or(0, |b| u32::from(b >> (7 - (p % 8)) & 1))
    }

    fn show(&self, n: u32) -> u32 {
        (0..n as usize).fold(0u32, |v, i| (v << 1) | self.bit_at(self.pos + i))
    }

    fn skip(&mut self, n: u32) {
        self.pos = (self.pos + n as usize).min(self.size + 8);
    }

    fn get(&mut self, n: u32) -> u32 {
        let v = self.show(n.min(32));
        self.skip(n);
        v
    }

    fn get1(&mut self) -> bool {
        self.get(1) != 0
    }

    fn count(&self) -> usize {
        self.pos
    }

    fn left(&self) -> isize {
        self.size as isize - self.pos as isize
    }

    fn align(&mut self) {
        let r = self.pos % 8;
        if r != 0 {
            self.skip(8 - r as u32);
        }
    }

    /// `get_xbits`: `n` bits, the top one the sign (0 negative), `n` ≥ 1.
    fn xbits(&mut self, n: u32) -> i32 {
        let v = self.get(n) as i64;
        let r = if v >> (n - 1) != 0 { v } else { v - ((1i64 << n) - 1) };
        r as i32
    }

    /// `get_unary(gb, 1, len)`: zero bits before a one, at most `len`.
    fn unary(&mut self, len: u32) -> u32 {
        let mut i = 0;
        while i < len && !self.get1() {
            i += 1;
        }
        i
    }

    /// `next_start_code_studio`: align, then skip bytes up to a `00 00 01` prefix.
    fn next_start_code_studio(&mut self) {
        self.align();
        while self.left() >= 24 && self.show(24) != 1 {
            self.skip(8);
        }
    }
}

/// A single-level prefix code table: `(symbol, length)` for every `bits`-bit prefix, length 0
/// for a code that is not in the table (FFmpeg answers -1).
struct Vlc {
    bits: u32,
    table: Vec<(i8, u8)>,
}

impl Vlc {
    /// `ff_vlc_init_from_lengths`: `(symbol, length)` pairs in code order; a negative length
    /// takes its code space without a symbol, a zero length is skipped.
    fn new(pairs: &[i8]) -> Self {
        let bits = pairs.chunks(2).filter_map(|p| p.get(1)).map(|&l| u32::from(l.unsigned_abs())).max().unwrap_or(1).clamp(1, 16);
        let mut table = vec![(-1i8, 0u8); 1 << bits];
        let mut code: u64 = 0;
        for p in pairs.chunks(2) {
            let (Some(&sym), Some(&len)) = (p.first(), p.get(1)) else { continue };
            if len == 0 {
                continue;
            }
            let l = u32::from(len.unsigned_abs()).min(bits);
            if len > 0 {
                let start = ((code >> (32 - l)) as usize) << (bits - l);
                for e in table.iter_mut().skip(start).take(1 << (bits - l)) {
                    *e = (sym, l as u8);
                }
            }
            code += 1u64 << (32 - l);
        }
        Self { bits, table }
    }

    /// The symbol, or -1 for a code that is not in the table.
    fn read(&self, b: &mut Bits<'_>) -> i32 {
        let (sym, len) = self.table.get(b.show(self.bits) as usize).copied().unwrap_or((-1, 0));
        if len == 0 {
            return -1;
        }
        b.skip(u32::from(len));
        i32::from(sym)
    }
}

struct Tables {
    dc_luma: Vlc,
    dc_chroma: Vlc,
    intra: Vec<Vlc>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| Tables {
        dc_luma: Vlc::new(&tables::STUDIO_DC_LUMA),
        dc_chroma: Vlc::new(&tables::STUDIO_DC_CHROMA),
        intra: tables::STUDIO_INTRA.chunks(48).map(Vlc::new).collect(),
    })
}

/// `ac_state_tab`: additional code length and next code table of each AC group.
const AC_STATE: [(u32, usize); 22] = [
    (0, 0),
    (0, 1),
    (1, 1),
    (2, 1),
    (3, 1),
    (4, 1),
    (5, 1),
    (1, 2),
    (2, 2),
    (3, 2),
    (4, 2),
    (5, 2),
    (6, 2),
    (1, 3),
    (2, 4),
    (3, 5),
    (4, 6),
    (5, 7),
    (6, 8),
    (7, 9),
    (8, 10),
    (0, 11),
];

/// The video object layer (sequence) parameters.
#[derive(Clone, Copy, Default)]
struct Vol {
    rgb: bool,
    /// 2: 4:2:2, 3: 4:4:4.
    chroma_format: u32,
    width: usize,
    height: usize,
    mpeg_quant: bool,
    par: (u32, u32),
    frame_rate: Option<(u32, u32)>,
}

/// A slice or macroblock outcome that is not an error.
#[derive(PartialEq, Eq)]
enum Mb {
    Ok,
    SliceEnd,
}

/// An MPEG-4 Simple Studio Profile decoder (the stream's headers persist between pictures).
#[derive(Clone)]
pub struct Decoder {
    studio: bool,
    profile: Option<u32>,
    vol: Option<Vol>,
    bits_per_raw_sample: u32,
    // picture parameters
    alternate_scan: bool,
    dct_precision: u32,
    intra_dc_precision: u32,
    q_scale_type: bool,
    intra_matrix: [u16; 64],
    chroma_intra_matrix: [u16; 64],
    qscale: i32,
    last_dc: [i32; 3],
    strict: bool,
    /// Set when the strict mode would refuse what was decoded so far.
    undefined: bool,
    mbs_decoded: usize,
    bits_used: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            studio: false,
            profile: None,
            vol: None,
            bits_per_raw_sample: 0,
            alternate_scan: false,
            dct_precision: 0,
            intra_dc_precision: 0,
            q_scale_type: false,
            intra_matrix: tables::DEFAULT_INTRA_MATRIX,
            chroma_intra_matrix: tables::DEFAULT_INTRA_MATRIX,
            qscale: 0,
            last_dc: [0; 3],
            strict: false,
            undefined: false,
            mbs_decoded: 0,
            bits_used: 0,
        }
    }

    /// Refuse pictures whose decoding FFmpeg leaves undefined or conceals.
    pub fn set_strict(&mut self, on: bool) {
        self.strict = on;
    }

    /// Macroblocks the last [`Decoder::decode`] decoded in order from the first, before
    /// anything the strict mode refuses.
    pub fn mbs_decoded(&self) -> usize {
        self.mbs_decoded
    }

    /// The bit position right after the last macroblock counted by [`Decoder::mbs_decoded`].
    pub fn bits_used(&self) -> usize {
        self.bits_used
    }

    /// The picture size and sampling once a video object layer header was read.
    pub fn format(&self) -> Option<(usize, usize, Chroma)> {
        let v = self.vol?;
        Some((v.width, v.height, chroma_of(&v)))
    }

    /// The stream's description once a video object layer header was read.
    pub fn stream_info(&self) -> Option<StreamInfo> {
        let v = self.vol?;
        Some(StreamInfo { width: v.width, height: v.height, chroma: chroma_of(&v), par: v.par, frame_rate: v.frame_rate })
    }

    fn flag_undefined(&mut self) -> Result<()> {
        self.undefined = true;
        if self.strict { invalid("undefined decoding") } else { Ok(()) }
    }

    /// Reads the headers in `data` (a VOS, visual object or VOL without a picture, such as
    /// codec configuration data).
    pub fn parse_headers(&mut self, data: &[u8]) -> Result<()> {
        let mut b = Bits::new(data);
        self.picture_header(&mut b, true).map(|_| ())
    }

    /// Decodes one access unit into a picture.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Picture> {
        self.undefined = false;
        self.mbs_decoded = 0;
        self.bits_used = 0;
        let mut b = Bits::new(packet);
        if !self.picture_header(&mut b, false)? {
            return invalid("no picture");
        }
        let vol = self.vol.ok_or(Error::Invalid("missing video object layer header"))?;
        if vol.width == 0 || vol.height == 0 {
            return invalid("no picture size");
        }
        if vol.width > MAX_SIDE || vol.height > MAX_SIDE {
            return Err(Error::Unsupported("picture larger than 8192 pixels"));
        }
        let mb_w = vol.width.div_ceil(16);
        let mb_h = vol.height.div_ceil(16);
        let mb_num = mb_w * mb_h;
        if (mb_num / 2) as isize > b.left() {
            return invalid("picture too short");
        }
        let (cw, ch) = match vol.chroma_format {
            2 => (8usize, 16usize),
            _ => (16, 16),
        };
        let mut pic = Frame { mb_w, luma: vec![0u16; mb_w * 16 * mb_h * 16], cb: vec![0u16; mb_w * cw * mb_h * ch], cr: vec![0u16; mb_w * cw * mb_h * ch], cw };
        let mut pos = (0usize, 0usize);
        let mut expected = 0usize;
        let slice_ok = self.decode_slice(&mut b, &vol, &mut pic, &mut pos, &mut expected);
        let mut damaged = slice_ok.is_err();
        while pos.1 < mb_h {
            // ff_h263_resync: the next slice start code
            b.align();
            while b.left() >= 32 && b.show(32) != SLICE_STARTCODE {
                b.skip(8);
            }
            if !(b.left() >= 32 && b.show(32) == SLICE_STARTCODE) {
                break;
            }
            if self.decode_slice(&mut b, &vol, &mut pic, &mut pos, &mut expected).is_err() {
                damaged = true;
            }
        }
        if damaged || expected < mb_num || b.left() < 0 {
            self.flag_undefined()?;
        }
        Ok(pic.crop(&vol))
    }

    /// `ff_mpeg4_decode_picture_header` for the studio profile: the start codes up to a VOP
    /// (true) or the end of the data (false, only with `header`), then the VOP header.
    fn picture_header(&mut self, b: &mut Bits<'_>, header: bool) -> Result<bool> {
        b.align();
        let mut startcode: u32 = 0xFF;
        let mut vol_seen = false;
        loop {
            if b.count() >= b.size {
                if header && b.count() == b.size {
                    return Ok(false);
                }
                return invalid("end of stream");
            }
            let v = b.get(8);
            startcode = (startcode << 8) | v;
            if startcode & 0xFFFF_FF00 != 0x100 {
                continue;
            }
            match startcode {
                0x120..=0x12F => {
                    if !vol_seen {
                        vol_seen = true;
                        self.vol_header(b)?;
                    }
                }
                USER_DATA_STARTCODE => {
                    // decode_user_data: up to 255 bytes before a start code prefix
                    let mut i = 0;
                    while i < 255 && b.count() < b.size {
                        if b.show(23) == 0 {
                            break;
                        }
                        b.skip(8);
                        i += 1;
                    }
                }
                GOP_STARTCODE => {
                    // mpeg4_decode_gop_header: time code and flags
                    if b.show(23) != 0 {
                        b.skip(5 + 6 + 1 + 6 + 1 + 1);
                    }
                }
                VOS_STARTCODE => {
                    let profile = b.get(4);
                    let level = b.get(4);
                    if profile == PROFILE_SIMPLE_STUDIO && (1..9).contains(&level) {
                        self.studio = true;
                        b.next_start_code_studio();
                        self.extension_and_user_data(b, 0);
                    } else if self.studio {
                        return Err(Error::Unsupported("mix of studio and non-studio profiles"));
                    }
                    self.profile = Some(profile);
                }
                VISUAL_OBJ_STARTCODE => {
                    if self.studio {
                        b.skip(4);
                        if b.get(4) != 1 {
                            return Err(Error::Unsupported("visual object type"));
                        }
                        b.next_start_code_studio();
                        self.extension_and_user_data(b, 1);
                    }
                }
                VOP_STARTCODE => break,
                _ => {}
            }
            b.align();
            startcode = 0xFF;
        }
        if !self.studio {
            return Err(Error::Unsupported("not the MPEG-4 Simple Studio Profile"));
        }
        if self.bits_per_raw_sample == 0 {
            return invalid("missing video object layer header");
        }
        self.vop_header(b)?;
        Ok(true)
    }

    fn vol_header(&mut self, b: &mut Bits<'_>) -> Result<()> {
        b.skip(1); // random_accessible_vol
        let vo_type = b.get(8);
        if vo_type != CORE_STUDIO_VO_TYPE && vo_type != SIMPLE_STUDIO_VO_TYPE {
            if self.studio {
                return Err(Error::Unsupported("non-studio video object layer in a studio stream"));
            }
            return Err(Error::Unsupported("not the MPEG-4 Simple Studio Profile"));
        }
        if self.profile.is_some_and(|p| p != PROFILE_SIMPLE_STUDIO) {
            return invalid("video object type against the profile");
        }
        self.studio = true;
        self.profile = Some(PROFILE_SIMPLE_STUDIO);
        b.skip(4); // video_object_layer_verid
        let shape = b.get(2);
        b.skip(4); // shape extension
        b.skip(1); // progressive_sequence
        if shape != 0 {
            return Err(Error::Unsupported("non-rectangular studio shape"));
        }
        let mut vol = self.vol.unwrap_or_default();
        let rgb = b.get1();
        let chroma_format = b.get(2);
        if chroma_format == 0 || chroma_format == 1 || (rgb && chroma_format == 2) {
            return invalid("chroma format");
        }
        let bits = b.get(4);
        if bits != 10 {
            return Err(Error::Unsupported("studio bit depth other than 10"));
        }
        self.bits_per_raw_sample = bits;
        vol.rgb = rgb;
        vol.chroma_format = chroma_format;
        b.skip(1);
        let width = b.get(14) as usize;
        b.skip(1);
        let height = b.get(14) as usize;
        b.skip(1);
        if width != 0 && height != 0 {
            vol.width = width;
            vol.height = height;
        }
        // aspect_ratio_info (ISO/IEC 14496-2 Table 6-12: pixel aspect ratios)
        vol.par = match b.get(4) {
            2 => (12, 11),
            3 => (10, 11),
            4 => (16, 11),
            5 => (40, 33),
            15 => match (b.get(8), b.get(8)) {
                (0, _) | (_, 0) => (1, 1),
                (w, h) => (w, h),
            },
            _ => (1, 1),
        };
        // frame_rate_code (the MPEG-2 frame rates, Table 6-4 of ISO/IEC 13818-2)
        vol.frame_rate = match b.get(4) {
            1 => Some((24000, 1001)),
            2 => Some((24, 1)),
            3 => Some((25, 1)),
            4 => Some((30000, 1001)),
            5 => Some((30, 1)),
            6 => Some((50, 1)),
            7 => Some((60000, 1001)),
            8 => Some((60, 1)),
            _ => None,
        };
        b.skip(15 + 1 + 15 + 1 + 15 + 1 + 3 + 11 + 1 + 15 + 1);
        b.skip(1); // low_delay
        vol.mpeg_quant = b.get1();
        self.vol = Some(vol);
        b.next_start_code_studio();
        self.extension_and_user_data(b, 2);
        Ok(())
    }

    fn vop_header(&mut self, b: &mut Bits<'_>) -> Result<()> {
        if b.left() <= 32 {
            // FFmpeg keeps the previous picture parameters
            return self.flag_undefined();
        }
        b.skip(16 + 1 + 16 + 1 + 16 + 1 + 16 + 1 + 4); // SMPTE time code
        b.skip(10); // temporal_reference
        b.skip(2); // vop_structure
        let coding_type = b.get(2);
        if b.get1() {
            b.skip(2); // top_field_first, repeat_first_field
            b.skip(1); // progressive_frame
        }
        if coding_type == 0 && b.get1() {
            self.reset_dc();
        }
        self.alternate_scan = b.get1();
        b.skip(1); // frame_pred_frame_dct
        self.dct_precision = b.get(2);
        self.intra_dc_precision = b.get(2);
        self.q_scale_type = b.get1();
        self.intra_matrix = tables::DEFAULT_INTRA_MATRIX;
        self.chroma_intra_matrix = tables::DEFAULT_INTRA_MATRIX;
        b.next_start_code_studio();
        self.extension_and_user_data(b, 4);
        if coding_type != 0 {
            return Err(Error::Unsupported("studio P or B picture"));
        }
        Ok(())
    }

    fn extension_and_user_data(&mut self, b: &mut Bits<'_>, id: u32) {
        let startcode = b.show(32);
        if (id == 2 || id == 4) && startcode == EXT_STARTCODE {
            b.skip(32);
            if b.get(4) == QUANT_MATRIX_EXT_ID {
                self.quant_matrix_ext(b);
            }
        }
    }

    fn quant_matrix_ext(&mut self, b: &mut Bits<'_>) {
        for k in 0..4 {
            if b.get1() {
                if b.left() < 64 * 8 {
                    return;
                }
                for i in 0..64 {
                    let v = b.get(8) as u16;
                    let j = usize::from(tables::ZIGZAG_DIRECT[i]);
                    match k {
                        0 => {
                            self.intra_matrix[j] = v;
                            self.chroma_intra_matrix[j] = v;
                        }
                        2 => self.chroma_intra_matrix[j] = v,
                        _ => {}
                    }
                }
            }
        }
        b.next_start_code_studio();
    }

    fn reset_dc(&mut self) {
        let s = (self.bits_per_raw_sample + self.dct_precision + self.intra_dc_precision).clamp(1, 31) - 1;
        self.last_dc = [1 << s; 3];
    }

    fn get_qscale(&self, b: &mut Bits<'_>) -> i32 {
        let q = b.get(5) as usize;
        if self.q_scale_type { i32::from(tables::NON_LINEAR_QSCALE[q]) } else { (q as i32) << 1 }
    }

    /// `decode_slice` of `h263dec.c` with the studio slice header.
    fn decode_slice(&mut self, b: &mut Bits<'_>, vol: &Vol, pic: &mut Frame, pos: &mut (usize, usize), expected: &mut usize) -> Result<()> {
        let mb_w = pic.mb_w;
        let mb_h = pic.luma.len() / (mb_w * 256).max(1);
        let mb_num = mb_w * mb_h;
        // ff_mpeg4_decode_studio_slice_header
        if !(b.left() >= 32 && b.get(32) == SLICE_STARTCODE) {
            return invalid("slice start code");
        }
        let vlc_len = usize::BITS - (mb_num.max(1)).leading_zeros();
        let n = b.get(vlc_len) as usize & 0xFFFF;
        if n >= mb_num {
            return invalid("slice position");
        }
        *pos = (n % mb_w, n / mb_w);
        if n != *expected {
            self.flag_undefined()?;
        }
        self.qscale = self.get_qscale(b);
        if b.get1() {
            b.skip(1 + 1 + 6);
            while b.get1() {
                b.skip(8);
            }
        }
        self.reset_dc();
        while pos.1 < mb_h {
            while pos.0 < mb_w {
                self.decode_mb(b, vol, pic, *pos)?;
                if b.left() < 0 {
                    self.flag_undefined()?;
                }
                if !self.undefined && pos.1 * mb_w + pos.0 == *expected {
                    *expected += 1;
                    self.mbs_decoded = *expected;
                    self.bits_used = b.count();
                }
                let r = Self::slice_end(b);
                pos.0 += 1;
                if r == Mb::SliceEnd {
                    if pos.0 >= mb_w {
                        pos.0 = 0;
                        pos.1 += 1;
                    }
                    return Ok(());
                }
            }
            pos.0 = 0;
            pos.1 += 1;
        }
        Ok(())
    }

    /// `mpeg4_decode_studio_mb` and its reconstruction (`ff_mpeg4_decode_studio`).
    fn decode_mb(&mut self, b: &mut Bits<'_>, vol: &Vol, pic: &mut Frame, (mx, my): (usize, usize)) -> Result<()> {
        if b.get1() {
            // DCT
            if !b.get1() {
                b.skip(1);
                self.qscale = self.get_qscale(b);
            }
            let count = if vol.chroma_format == 2 { 8 } else { 12 };
            let mut blocks = [[0i32; 64]; 12];
            for (n, blk) in blocks.iter_mut().enumerate().take(count) {
                self.studio_block(b, vol, blk, n)?;
            }
            for (n, blk) in blocks.iter_mut().enumerate().take(count) {
                let (plane, x, y) = match n {
                    0..=3 => (0, mx * 16 + (n & 1) * 8, my * 16 + (n >> 1) * 8),
                    _ => {
                        let k = n - 4;
                        (1 + (k & 1), mx * pic.cw + (k >> 2) * 8, my * 16 + ((k >> 1) & 1) * 8)
                    }
                };
                pic.idct_put(plane, x, y, blk);
            }
        } else {
            // DPCM
            b.skip(1); // marker
            let reverse = b.get1();
            for i in 0..3 {
                let mb = self.dpcm_block(b, vol, i)?;
                let (w, h) = if i > 0 && vol.chroma_format == 2 { (8, 16) } else { (16, 16) };
                let (x0, y0) = if i == 0 { (mx * 16, my * 16) } else { (mx * pic.cw, my * 16) };
                for r in 0..h {
                    for c in 0..w {
                        let v = mb.get(r * w + c).copied().unwrap_or(0);
                        let (x, y) = if reverse { (x0 + w - 1 - c, y0 + h - 1 - r) } else { (x0 + c, y0 + r) };
                        pic.put(i, x, y, v);
                    }
                }
            }
        }
        Ok(())
    }

    /// The end of `mpeg4_decode_studio_mb`: whether the slice ends after this macroblock.
    fn slice_end(b: &mut Bits<'_>) -> Mb {
        if b.left() >= 24 && b.show(23) == 0 {
            b.next_start_code_studio();
            return Mb::SliceEnd;
        }
        if b.left() == 0 {
            return Mb::SliceEnd;
        }
        if b.left() > 0 && b.left() < 8 && b.show(b.left() as u32) == 0 {
            return Mb::SliceEnd;
        }
        Mb::Ok
    }

    /// `mpeg4_decode_studio_block`: the dequantised coefficients of block `n` in raster order.
    fn studio_block(&mut self, b: &mut Bits<'_>, vol: &Vol, block: &mut [i32; 64], n: usize) -> Result<()> {
        let tb = tables();
        let bits = self.bits_per_raw_sample;
        let min = -(1i32 << (bits + 6));
        let max = (1i32 << (bits + 6)) - 1;
        let shift = 3u32.saturating_sub(self.dct_precision);
        let scan: &[u8; 64] = if self.alternate_scan { &tables::ALTERNATE_VERTICAL_SCAN } else { &tables::ZIGZAG_DIRECT };
        let (cc, size, matrix) = if n < 4 {
            (0, tb.dc_luma.read(b), self.intra_matrix)
        } else {
            let dc = if vol.rgb { &tb.dc_luma } else { &tb.dc_chroma };
            ((n & 1) + 1, dc.read(b), self.chroma_intra_matrix)
        };
        if size < 0 {
            return invalid("DC size code");
        }
        let mut diff = 0;
        if size > 0 {
            diff = b.xbits(size as u32);
            if size > 8 && !b.get1() {
                return invalid("marker after the DC difference");
            }
        }
        let dc = self.last_dc[cc].checked_add(diff);
        if dc.is_none() {
            self.flag_undefined()?;
        }
        self.last_dc[cc] = dc.unwrap_or_else(|| self.last_dc[cc].wrapping_add(diff));
        let mul = if vol.mpeg_quant { 8 >> self.intra_dc_precision } else { (8 >> self.intra_dc_precision) * (8 >> self.dct_precision) };
        let v = i64::from(self.last_dc[cc]) * i64::from(mul);
        if i32::try_from(v).is_err() {
            self.flag_undefined()?;
        }
        block[0] = (v as i32).clamp(min, max);
        let mut mismatch = 1 ^ block[0];
        let mut idx = 1usize;
        let mut vlc = &tb.intra[0];
        loop {
            let group = vlc.read(b);
            let Ok(g) = usize::try_from(group) else { return invalid("AC coefficient group code") };
            let (acl, next) = AC_STATE.get(g).copied().ok_or(Error::Invalid("AC coefficient group"))?;
            vlc = tb.intra.get(next).ok_or(Error::Invalid("AC table"))?;
            let j;
            match g {
                0 => break,
                1..=6 => {
                    let mut run = 1usize << acl;
                    if acl > 0 {
                        run += b.get(acl) as usize;
                    }
                    idx += run;
                    continue;
                }
                7..=12 => {
                    let code = b.get(acl);
                    let sign = code & 1;
                    let run = (1usize << (acl - 1)) + (code >> 1) as usize;
                    idx += run;
                    if idx > 63 {
                        return invalid("AC run past the block");
                    }
                    j = usize::from(scan[idx]);
                    idx += 1;
                    block[j] = if sign != 0 { 1 } else { -1 };
                }
                13..=20 => {
                    if idx > 63 {
                        return invalid("AC level past the block");
                    }
                    j = usize::from(scan[idx]);
                    idx += 1;
                    block[j] = b.xbits(acl);
                }
                _ => {
                    if idx > 63 {
                        return invalid("AC escape past the block");
                    }
                    j = usize::from(scan[idx]);
                    idx += 1;
                    let len = bits + self.dct_precision + 4;
                    let flc = i64::from(b.get(len));
                    block[j] = if flc >> (len - 1) != 0 { (flc - (1i64 << len)) as i32 } else { flc as i32 };
                }
            }
            let prod = i64::from(block[j]) * i64::from(matrix[j]) * i64::from(self.qscale);
            if i32::try_from(prod).is_err() || i32::try_from(prod << shift).is_err() {
                self.flag_undefined()?;
            }
            let p = (block[j].wrapping_mul(i32::from(matrix[j])).wrapping_mul(self.qscale)).wrapping_mul(1 << shift);
            block[j] = (p / 16).clamp(min, max);
            mismatch ^= block[j];
        }
        block[63] ^= mismatch & 1;
        Ok(())
    }

    /// `mpeg4_decode_dpcm_macroblock`: component `n`'s samples, rows of 16 (8 for 4:2:2 chroma).
    fn dpcm_block(&mut self, b: &mut Bits<'_>, vol: &Vol, n: usize) -> Result<Vec<u16>> {
        let bits = self.bits_per_raw_sample;
        let (w, h) = if n > 0 && vol.chroma_format == 2 { (8usize, 16usize) } else { (16, 16) };
        let block_mean = b.get(bits) as i32;
        if block_mean == 0 {
            return invalid("DPCM block mean");
        }
        self.last_dc[n] = block_mean << (self.dct_precision + self.intra_dc_precision);
        let mut rice = b.get(4);
        if rice == 0 {
            return invalid("DPCM Rice parameter");
        }
        if rice == 15 {
            rice = 0;
        }
        if rice > 11 {
            return invalid("DPCM Rice parameter");
        }
        let half = 1i32 << (bits - 1);
        let mask = (1i32 << bits) - 1;
        let mut out = vec![0u16; w * h];
        let mut idx = 0usize;
        for i in 0..h {
            let mut output = half;
            let mut top = half;
            for _ in 0..w {
                let left = output;
                let topleft = top;
                let prefix = b.unary(12);
                let mut residual = if prefix == 11 {
                    b.get(bits) as i32
                } else {
                    if prefix == 12 {
                        return invalid("DPCM Rice prefix");
                    }
                    let suffix = if rice > 0 { b.get(rice) } else { 0 };
                    ((prefix << rice) + suffix) as i32
                };
                residual = if residual & 1 != 0 { (-residual) >> 1 } else { residual >> 1 };
                if i != 0 {
                    top = i32::from(out[idx - w]);
                }
                let mut p = left + top - topleft;
                let min_lt = left.min(top);
                let max_lt = left.max(top);
                p = p.clamp(min_lt, max_lt.max(min_lt));
                let mut p2 = (min_lt.min(topleft) + max_lt.max(topleft)) >> 1;
                if p2 == p {
                    p2 = block_mean;
                }
                if p2 > p {
                    residual = -residual;
                }
                output = (residual + p) & mask;
                out[idx] = output as u16;
                idx += 1;
            }
        }
        Ok(out)
    }
}

fn chroma_of(v: &Vol) -> Chroma {
    if v.rgb {
        Chroma::Rgb
    } else if v.chroma_format == 2 {
        Chroma::Yuv422
    } else {
        Chroma::Yuv444
    }
}

/// What a video object layer header says about a studio stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub width: usize,
    pub height: usize,
    pub chroma: Chroma,
    /// Pixel aspect ratio (width, height).
    pub par: (u32, u32),
    /// Frames per second as (numerator, denominator), when the header names a rate.
    pub frame_rate: Option<(u32, u32)>,
}

/// Recognises a Simple Studio Profile elementary stream from its first bytes: the headers
/// before the first picture must name a studio video object layer.
pub fn probe(head: &[u8]) -> Option<StreamInfo> {
    if !head.starts_with(&[0, 0, 1]) {
        return None;
    }
    let vop = head.windows(4).position(|w| w == [0, 0, 1, 0xB6])?;
    let mut d = Decoder::new();
    d.parse_headers(head.get(..vop)?).ok()?;
    d.stream_info()
}

/// Splits an elementary stream into access units: a picture with the headers before it.
/// Feed the stream in order; every start code that begins a new access unit is reported
/// by its offset.
#[derive(Clone, Debug, Default)]
pub struct AccessUnits {
    seen_vop: bool,
    /// The last three bytes fed, for start codes across chunks.
    tail: u32,
}

impl AccessUnits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans `chunk`, which starts at `offset` in the stream, and pushes the offsets of the
    /// access units that start in it (after the first one, which starts at 0).
    pub fn feed(&mut self, offset: u64, chunk: &[u8], starts: &mut Vec<u64>) {
        for (i, &c) in chunk.iter().enumerate() {
            if self.tail & 0xFF_FFFF == 1 {
                if self.seen_vop && (c == 0xB0 || c == 0xB3 || c == 0xB6 || c <= 0x2F) {
                    starts.push((offset + i as u64).saturating_sub(3));
                    self.seen_vop = false;
                }
                if c == 0xB6 {
                    self.seen_vop = true;
                }
            }
            self.tail = (self.tail << 8) | u32::from(c);
        }
    }
}

/// The macroblock-aligned picture being decoded.
struct Frame {
    mb_w: usize,
    luma: Vec<u16>,
    cb: Vec<u16>,
    cr: Vec<u16>,
    /// Chroma samples per macroblock row (8 or 16).
    cw: usize,
}

impl Frame {
    fn stride(&self, plane: usize) -> usize {
        if plane == 0 { self.mb_w * 16 } else { self.mb_w * self.cw }
    }

    fn put(&mut self, plane: usize, x: usize, y: usize, v: u16) {
        let s = self.stride(plane);
        let p = match plane {
            0 => &mut self.luma,
            1 => &mut self.cb,
            _ => &mut self.cr,
        };
        if let Some(d) = p.get_mut(y * s + x) {
            *d = v;
        }
    }

    /// `ff_simple_idct_put_int32_10bit` at (`x`, `y`) of `plane`.
    fn idct_put(&mut self, plane: usize, x: usize, y: usize, block: &mut [i32; 64]) {
        const W1: u32 = 22725;
        const W2: u32 = 21407;
        const W3: u32 = 19265;
        const W4: u32 = 16384;
        const W5: u32 = 12873;
        const W6: u32 = 8867;
        const W7: u32 = 4520;
        const ROW_SHIFT: u32 = 13;
        const COL_SHIFT: u32 = 21;
        let m = |a: u32, b: i32| a.wrapping_mul(b as u32);
        let neg = |a: u32| a.wrapping_neg();
        for r in 0..8 {
            let row = &mut block[r * 8..r * 8 + 8];
            let mut a0 = m(W4, row[0]).wrapping_add(1 << (ROW_SHIFT - 1));
            let (mut a1, mut a2, mut a3) = (a0, a0, a0);
            a0 = a0.wrapping_add(m(W2, row[2]));
            a1 = a1.wrapping_add(m(W6, row[2]));
            a2 = a2.wrapping_sub(m(W6, row[2]));
            a3 = a3.wrapping_sub(m(W2, row[2]));
            let mut b0 = m(W1, row[1]).wrapping_add(m(W3, row[3]));
            let mut b1 = m(W3, row[1]).wrapping_add(m(neg(W7), row[3]));
            let mut b2 = m(W5, row[1]).wrapping_add(m(neg(W1), row[3]));
            let mut b3 = m(W7, row[1]).wrapping_add(m(neg(W5), row[3]));
            if row[4] | row[5] | row[6] | row[7] != 0 {
                a0 = a0.wrapping_add(m(W4, row[4]).wrapping_add(m(W6, row[6])));
                a1 = a1.wrapping_add(m(neg(W4), row[4]).wrapping_sub(m(W2, row[6])));
                a2 = a2.wrapping_add(m(neg(W4), row[4]).wrapping_add(m(W2, row[6])));
                a3 = a3.wrapping_add(m(W4, row[4]).wrapping_sub(m(W6, row[6])));
                b0 = b0.wrapping_add(m(W5, row[5])).wrapping_add(m(W7, row[7]));
                b1 = b1.wrapping_add(m(neg(W1), row[5])).wrapping_add(m(neg(W5), row[7]));
                b2 = b2.wrapping_add(m(W7, row[5])).wrapping_add(m(W3, row[7]));
                b3 = b3.wrapping_add(m(W3, row[5])).wrapping_add(m(neg(W1), row[7]));
            }
            let sh = |v: u32| (v as i32) >> ROW_SHIFT;
            row[0] = sh(a0.wrapping_add(b0));
            row[7] = sh(a0.wrapping_sub(b0));
            row[1] = sh(a1.wrapping_add(b1));
            row[6] = sh(a1.wrapping_sub(b1));
            row[2] = sh(a2.wrapping_add(b2));
            row[5] = sh(a2.wrapping_sub(b2));
            row[3] = sh(a3.wrapping_add(b3));
            row[4] = sh(a3.wrapping_sub(b3));
        }
        for c in 0..8 {
            let col = |k: usize| block[k * 8 + c];
            let mut a0 = m(W4, col(0).wrapping_add(((1 << (COL_SHIFT - 1)) / W4) as i32));
            let (mut a1, mut a2, mut a3) = (a0, a0, a0);
            a0 = a0.wrapping_add(m(W2, col(2)));
            a1 = a1.wrapping_add(m(W6, col(2)));
            a2 = a2.wrapping_add(m(neg(W6), col(2)));
            a3 = a3.wrapping_add(m(neg(W2), col(2)));
            let mut b0 = m(W1, col(1));
            let mut b1 = m(W3, col(1));
            let mut b2 = m(W5, col(1));
            let mut b3 = m(W7, col(1));
            b0 = b0.wrapping_add(m(W3, col(3)));
            b1 = b1.wrapping_add(m(neg(W7), col(3)));
            b2 = b2.wrapping_add(m(neg(W1), col(3)));
            b3 = b3.wrapping_add(m(neg(W5), col(3)));
            a0 = a0.wrapping_add(m(W4, col(4)));
            a1 = a1.wrapping_add(m(neg(W4), col(4)));
            a2 = a2.wrapping_add(m(neg(W4), col(4)));
            a3 = a3.wrapping_add(m(W4, col(4)));
            b0 = b0.wrapping_add(m(W5, col(5)));
            b1 = b1.wrapping_add(m(neg(W1), col(5)));
            b2 = b2.wrapping_add(m(W7, col(5)));
            b3 = b3.wrapping_add(m(W3, col(5)));
            a0 = a0.wrapping_add(m(W6, col(6)));
            a1 = a1.wrapping_add(m(neg(W2), col(6)));
            a2 = a2.wrapping_add(m(W2, col(6)));
            a3 = a3.wrapping_add(m(neg(W6), col(6)));
            b0 = b0.wrapping_add(m(W7, col(7)));
            b1 = b1.wrapping_add(m(neg(W5), col(7)));
            b2 = b2.wrapping_add(m(W3, col(7)));
            b3 = b3.wrapping_add(m(neg(W1), col(7)));
            let px = |v: u32| ((v as i32) >> COL_SHIFT).clamp(0, 1023) as u16;
            let out = [
                px(a0.wrapping_add(b0)),
                px(a1.wrapping_add(b1)),
                px(a2.wrapping_add(b2)),
                px(a3.wrapping_add(b3)),
                px(a3.wrapping_sub(b3)),
                px(a2.wrapping_sub(b2)),
                px(a1.wrapping_sub(b1)),
                px(a0.wrapping_sub(b0)),
            ];
            for (k, &v) in out.iter().enumerate() {
                self.put(plane, x + c, y + k, v);
            }
        }
    }

    /// The `width` × `height` picture.
    fn crop(self, vol: &Vol) -> Picture {
        let (w, h) = (vol.width, vol.height);
        let cw = if vol.chroma_format == 2 { w.div_ceil(2) } else { w };
        let cut = |p: &[u16], stride: usize, pw: usize| -> Vec<u16> {
            let mut out = Vec::with_capacity(pw * h);
            for r in 0..h {
                out.extend(p.get(r * stride..r * stride + pw).unwrap_or(&[]));
            }
            out
        };
        let (sl, sc) = (self.stride(0), self.stride(1));
        Picture { width: w, height: h, chroma: chroma_of(vol), planes: [cut(&self.luma, sl, w), cut(&self.cb, sc, cw), cut(&self.cr, sc, cw)] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_tables_are_complete() {
        let tb = tables();
        let kraft = |p: &[i8]| p.chunks(2).map(|x| x[1]).filter(|&l| l != 0).map(|l| 2f64.powi(-i32::from(l.unsigned_abs()))).sum::<f64>();
        assert_eq!(kraft(&tables::STUDIO_DC_LUMA), 1.0);
        assert_eq!(kraft(&tables::STUDIO_DC_CHROMA), 1.0);
        for t in tables::STUDIO_INTRA.chunks(48) {
            assert_eq!(kraft(t), 1.0);
        }
        assert_eq!(tb.intra.len(), 12);
    }

    #[test]
    fn idct_of_a_dc_block_is_flat() {
        let mut f = Frame { mb_w: 1, luma: vec![0; 256], cb: vec![0; 128], cr: vec![0; 128], cw: 8 };
        let mut b = [0i32; 64];
        // a DC of 32768 gives mid-grey 512 (the row pass scales by 2, the column pass by 1/128)
        b[0] = 32768;
        f.idct_put(0, 0, 0, &mut b);
        assert!(f.luma[..8].iter().all(|&v| v == 512), "{:?}", &f.luma[..8]);
    }

    #[test]
    fn random_packets_do_not_panic() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        };
        // a 64x32 4:2:2 VOL followed by random pictures
        let vol = [0x00, 0x00, 0x01, 0x20, 0x07, 0xA1, 0xE5, 0x50, 0x80, 0x10, 0x00, 0x08, 0x02, 0x00, 0x00, 0x00, 0x01, 0xB6];
        for _ in 0..300 {
            let mut p = vol.to_vec();
            p.extend((0..400).map(|_| rnd()));
            let mut d = Decoder::new();
            let _ = d.decode(&p);
            d.set_strict(true);
            let _ = d.decode(&p);
        }
        let mut d = Decoder::new();
        for _ in 0..300 {
            let p: Vec<u8> = (0..300).map(|_| rnd()).collect();
            let _ = d.decode(&p);
        }
    }
}
