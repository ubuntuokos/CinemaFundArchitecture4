//! The common part of the Indeo 4 / 5 decoders (`ivi.c`) with the codec specific headers
//! (`indeo4.c`, `indeo5.c`).

use thiserror::Error;

use crate::bits::{Bits, VLC_BITS, Vlc};
use crate::data::*;
use crate::dsp::{self, Dc, Inv};

/// Indeo decoding errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("Indeo: invalid picture size {0}x{1}")]
    Size(u32, u32),
    #[error("Indeo: frame damaged or inconsistent: {0}")]
    Invalid(&'static str),
    #[error("Indeo: unsupported feature: {0}")]
    Unsupported(&'static str),
}

fn inv<T>(why: &'static str) -> Result<T, Error> {
    Err(Error::Invalid(why))
}

/// Which codec a stream uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Indeo4,
    Indeo5,
}

/// A decoded picture: planar YUV 4:1:0, tightly packed (`Y`, `U`, `V`; chroma at a quarter of
/// the width and height, rounded up).
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Vec<u8>>,
    /// Whether this was a key (intra) frame.
    pub key: bool,
}

impl Picture {
    /// Width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if i == 0 { (w, h) } else { (w.div_ceil(4), h.div_ceil(4)) }
    }
}

// Indeo 4 frame types
const I4_INTRA: i32 = 0;
const I4_INTRA1: i32 = 1;
const I4_INTER: i32 = 2;
const I4_BIDIR: i32 = 3;
const I4_NULL_FIRST: i32 = 5;
const I4_NULL_LAST: i32 = 6;
// Indeo 5 frame types
const I5_INTRA: i32 = 0;
const I5_INTER: i32 = 1;
const I5_INTER_SCAL: i32 = 2;
const I5_INTER_NOREF: i32 = 3;
const I5_NULL: i32 = 4;

const IVI5_IS_PROTECTED: u8 = 0x20;

/// Largest picture accepted (FFmpeg bounds it by memory only).
const MAX_DIM: u32 = 4096;
const MAX_PIXELS: u32 = 4096 * 2304;

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
const VERTICAL_SCAN_8X8: [u8; 64] = [
    0, 8, 16, 24, 32, 40, 48, 56, 1, 9, 17, 25, 33, 41, 49, 57, 2, 10, 18, 26, 34, 42, 50, 58, 3, 11, 19, 27, 35, 43, 51, 59, 4, 12, 20, 28, 36, 44, 52, 60, 5,
    13, 21, 29, 37, 45, 53, 61, 6, 14, 22, 30, 38, 46, 54, 62, 7, 15, 23, 31, 39, 47, 55, 63,
];
const HORIZONTAL_SCAN_8X8: [u8; 64] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
];
const DIRECT_SCAN_4X4: [u8; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

fn scan_index_to_tab(i: usize) -> &'static [u8] {
    match i {
        0 | 4 => &ZIGZAG,
        1 => &IVI4_ALTERNATE_SCAN_8X8,
        2 => &HORIZONTAL_SCAN_8X8,
        3 => &VERTICAL_SCAN_8X8,
        5 | 9 => &DIRECT_SCAN_4X4,
        6 => &IVI4_ALTERNATE_SCAN_4X4,
        7 => &IVI4_VERTICAL_SCAN_4X4,
        8 => &IVI4_HORIZONTAL_SCAN_4X4,
        _ => &HORIZONTAL_SCAN_8X8,
    }
}

/// Indeo 4 transform table: inverse transform, DC transform, two-dimensional flag.
fn i4_transform(id: usize) -> Option<(Inv, Dc, bool)> {
    Some(match id {
        0 => (Inv::Haar8x8, Dc::Haar2d, true),
        1 => (Inv::RowHaar8, Dc::Haar2d, false),
        2 => (Inv::ColHaar8, Dc::Haar2d, false),
        3 => (Inv::PutPixels8x8, Dc::PutDcPixel8x8, true),
        4 => (Inv::Slant8x8, Dc::Slant2d, true),
        5 => (Inv::RowSlant8, Dc::RowSlant, true),
        6 => (Inv::ColSlant8, Dc::ColSlant, true),
        10 => (Inv::Haar4x4, Dc::Haar2d, true),
        11 => (Inv::Slant4x4, Dc::Slant2d, true),
        13 => (Inv::RowHaar4, Dc::Haar2d, false),
        14 => (Inv::ColHaar4, Dc::Haar2d, false),
        15 => (Inv::RowSlant4, Dc::RowSlant, false),
        16 => (Inv::ColSlant4, Dc::ColSlant, false),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Huffman tables

#[derive(Clone, Copy, Default)]
struct HuffDesc {
    num_rows: usize,
    xbits: [u8; 16],
}

const MB_HUFF_DESC: [HuffDesc; 8] = [
    HuffDesc { num_rows: 8, xbits: [0, 4, 5, 4, 4, 4, 6, 6, 0, 0, 0, 0, 0, 0, 0, 0] },
    HuffDesc { num_rows: 12, xbits: [0, 2, 2, 3, 3, 3, 3, 5, 3, 2, 2, 2, 0, 0, 0, 0] },
    HuffDesc { num_rows: 12, xbits: [0, 2, 3, 4, 3, 3, 3, 3, 4, 3, 2, 2, 0, 0, 0, 0] },
    HuffDesc { num_rows: 12, xbits: [0, 3, 4, 4, 3, 3, 3, 3, 3, 2, 2, 2, 0, 0, 0, 0] },
    HuffDesc { num_rows: 13, xbits: [0, 4, 4, 3, 3, 3, 3, 2, 3, 3, 2, 1, 1, 0, 0, 0] },
    HuffDesc { num_rows: 9, xbits: [0, 4, 4, 4, 4, 3, 3, 3, 2, 0, 0, 0, 0, 0, 0, 0] },
    HuffDesc { num_rows: 10, xbits: [0, 4, 4, 4, 4, 3, 3, 2, 2, 2, 0, 0, 0, 0, 0, 0] },
    HuffDesc { num_rows: 12, xbits: [0, 4, 4, 4, 3, 3, 2, 3, 2, 2, 2, 2, 0, 0, 0, 0] },
];

const BLK_HUFF_DESC: [HuffDesc; 8] = [
    HuffDesc { num_rows: 10, xbits: [1, 2, 3, 4, 4, 7, 5, 5, 4, 1, 0, 0, 0, 0, 0, 0] },
    HuffDesc { num_rows: 11, xbits: [2, 3, 4, 4, 4, 7, 5, 4, 3, 3, 2, 0, 0, 0, 0, 0] },
    HuffDesc { num_rows: 12, xbits: [2, 4, 5, 5, 5, 5, 6, 4, 4, 3, 1, 1, 0, 0, 0, 0] },
    HuffDesc { num_rows: 13, xbits: [3, 3, 4, 4, 5, 6, 6, 4, 4, 3, 2, 1, 1, 0, 0, 0] },
    HuffDesc { num_rows: 11, xbits: [3, 4, 4, 5, 5, 5, 6, 5, 4, 2, 2, 0, 0, 0, 0, 0] },
    HuffDesc { num_rows: 13, xbits: [3, 4, 5, 5, 5, 5, 6, 4, 3, 3, 2, 1, 1, 0, 0, 0] },
    HuffDesc { num_rows: 13, xbits: [3, 4, 5, 5, 5, 6, 5, 4, 3, 3, 2, 1, 1, 0, 0, 0] },
    HuffDesc { num_rows: 9, xbits: [3, 4, 4, 5, 5, 5, 6, 5, 5, 0, 0, 0, 0, 0, 0, 0] },
];

/// Generate a Huffman code from a descriptor (`ivi_create_huff_from_desc`).
fn create_huff(cb: &HuffDesc) -> Option<Vlc> {
    let mut codewords = Vec::with_capacity(256);
    let mut bits = Vec::with_capacity(256);
    for i in 0..cb.num_rows {
        let xb = u32::from(cb.xbits[i]);
        let codes_per_row = 1u64 << xb;
        let not_last_row = u32::from(i != cb.num_rows - 1);
        let prefix = (((1u64 << i) - 1) << (xb + not_last_row)) as u32;
        for j in 0..codes_per_row {
            if codewords.len() >= 256 {
                break;
            }
            let n = i as u32 + xb + not_last_row;
            if n > VLC_BITS {
                return None;
            }
            codewords.push((prefix | j as u32) as u16);
            bits.push(n.max(1) as u8);
        }
    }
    Vlc::new(&bits, &codewords)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum VlcRef {
    #[default]
    None,
    StaticMb(usize),
    StaticBlk(usize),
    CtxMb,
    CtxBlk,
    Band,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    CtxMb,
    CtxBlk,
    Band,
}

#[derive(Clone, Default)]
struct HuffTab {
    tab: VlcRef,
    cust_tab: Option<Vlc>,
}

/// Decode a Huffman codebook descriptor and select the table (`ff_ivi_dec_huff_desc`).
fn dec_huff_desc(gb: &mut Bits<'_>, desc_coded: bool, blk: bool, ht: &mut HuffTab, owner: Owner) -> Result<(), Error> {
    if !desc_coded {
        ht.tab = if blk { VlcRef::StaticBlk(7) } else { VlcRef::StaticMb(7) };
        return Ok(());
    }
    let sel = gb.get(3) as usize;
    if sel == 7 {
        let mut d = HuffDesc { num_rows: gb.get(4) as usize, xbits: [0; 16] };
        if d.num_rows == 0 {
            return inv("empty custom Huffman table");
        }
        for i in 0..d.num_rows {
            d.xbits[i] = gb.get(4) as u8;
        }
        ht.cust_tab = create_huff(&d);
        if ht.cust_tab.is_none() {
            return inv("custom Huffman table");
        }
        ht.tab = match owner {
            Owner::CtxMb => VlcRef::CtxMb,
            Owner::CtxBlk => VlcRef::CtxBlk,
            Owner::Band => VlcRef::Band,
        };
    } else {
        ht.tab = if blk { VlcRef::StaticBlk(sel) } else { VlcRef::StaticMb(sel) };
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Pictures, planes, bands, tiles

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct PicConf {
    pic_width: u32,
    pic_height: u32,
    chroma_width: u32,
    chroma_height: u32,
    tile_width: u32,
    tile_height: u32,
    luma_bands: u32,
    chroma_bands: u32,
}

#[derive(Clone, Copy, Default)]
struct Mb {
    xpos: i32,
    ypos: i32,
    buf_offs: usize,
    /// 0 intra, 1 inter, (Indeo 4) 2 backward, 3 bidirectional
    typ: u8,
    cbp: u8,
    q_delta: i8,
    mv_x: i8,
    mv_y: i8,
    b_mv_x: i8,
    b_mv_y: i8,
}

#[derive(Clone, Default)]
struct Tile {
    xpos: i32,
    ypos: i32,
    width: i32,
    height: i32,
    mb_size: i32,
    is_empty: bool,
    data_size: usize,
    num_mbs: usize,
    mbs: Vec<Mb>,
}

/// The header parameters of a band.
#[derive(Clone)]
struct BandParams {
    plane: usize,
    band_num: usize,
    width: i32,
    height: i32,
    aheight: i32,
    pitch: i32,
    bufsize: usize,
    is_empty: bool,
    mb_size: i32,
    blk_size: i32,
    is_halfpel: i32,
    inherit_mv: bool,
    inherit_qdelta: bool,
    qdelta_present: bool,
    quant_mat: usize,
    glob_quant: i32,
    scan: Option<&'static [u8]>,
    scan_size: i32,
    num_corr: usize,
    corr: [u8; 122],
    rvmap_sel: usize,
    inv_transform: Option<Inv>,
    dc_transform: Option<Dc>,
    transform_size: i32,
    is_2d_trans: bool,
    intra_base: Option<&'static [u16]>,
    inter_base: Option<&'static [u16]>,
    intra_scale: Option<&'static [u8]>,
    inter_scale: Option<&'static [u8]>,
}

impl Default for BandParams {
    fn default() -> Self {
        Self {
            plane: 0,
            band_num: 0,
            width: 0,
            height: 0,
            aheight: 0,
            pitch: 0,
            bufsize: 0,
            is_empty: false,
            mb_size: 0,
            blk_size: 0,
            is_halfpel: 0,
            inherit_mv: false,
            inherit_qdelta: false,
            qdelta_present: false,
            quant_mat: 0,
            glob_quant: 0,
            scan: None,
            scan_size: 0,
            num_corr: 0,
            corr: [0; 122],
            rvmap_sel: 0,
            inv_transform: None,
            dc_transform: None,
            transform_size: 0,
            is_2d_trans: false,
            intra_base: None,
            inter_base: None,
            intra_scale: None,
            inter_scale: None,
        }
    }
}

#[derive(Default)]
struct Band {
    p: BandParams,
    blk_vlc: HuffTab,
    tiles: Vec<Tile>,
    bufs: [Vec<i16>; 4],
    /// The buffer this band was last decoded into (the C `band->buf`).
    cur_buf: Option<usize>,
}

#[derive(Default)]
struct Plane {
    width: usize,
    height: usize,
    bands: Vec<Band>,
}

fn mbs_per_tile(w: i32, h: i32, mb: i32) -> usize {
    (((w + mb - 1) / mb) * ((h + mb - 1) / mb)) as usize
}

fn to_signed(v: u32) -> i32 {
    0u32.wrapping_sub((v >> 1) ^ 0u32.wrapping_sub(v & 1)) as i32
}

fn scale_mv(mv: i32, scale: i32) -> i32 {
    (mv + i32::from(mv > 0) + (scale - 1)) >> scale
}

fn check_size(w: u32, h: u32) -> Result<(), Error> {
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || w * h > MAX_PIXELS {
        return Err(Error::Size(w, h));
    }
    Ok(())
}

/// The Indeo 4 / 5 decoder of one stream.
pub struct Decoder {
    codec: Codec,
    mb_statics: Vec<Vlc>,
    blk_statics: Vec<Vlc>,
    rvmaps: Vec<RvMap>,

    frame_type: i32,
    prev_frame_type: i32,
    is_scalable: bool,
    inter_scal: bool,
    frame_flags: u8,
    pic_conf: PicConf,
    planes: [Plane; 3],

    buf_switch: usize,
    dst_buf: usize,
    ref_buf: usize,
    ref2_buf: usize,
    b_ref_buf: usize,

    mb_vlc: HuffTab,
    blk_vlc: HuffTab,

    in_q: bool,
    gop_flags: u8,
    gop_invalid: bool,
    buf_invalid: [bool; 4],
    p_frame: Option<Picture>,
}

impl Decoder {
    /// A decoder for `codec`.
    pub fn new(codec: Codec) -> Self {
        let mb_statics = MB_HUFF_DESC.iter().map(|d| create_huff(d).expect("static Huffman table")).collect();
        let blk_statics = BLK_HUFF_DESC.iter().map(|d| create_huff(d).expect("static Huffman table")).collect();
        let (dst_buf, ref_buf, b_ref_buf) = match codec {
            Codec::Indeo4 => (0, 1, 3),
            Codec::Indeo5 => (0, 0, 0),
        };
        Self {
            codec,
            mb_statics,
            blk_statics,
            rvmaps: RVMAPS.to_vec(),
            frame_type: 0,
            prev_frame_type: 0,
            is_scalable: false,
            inter_scal: false,
            frame_flags: 0,
            pic_conf: PicConf::default(),
            planes: Default::default(),
            buf_switch: 0,
            dst_buf,
            ref_buf,
            ref2_buf: 0,
            b_ref_buf,
            mb_vlc: HuffTab::default(),
            blk_vlc: HuffTab::default(),
            in_q: false,
            gop_flags: 0,
            gop_invalid: codec == Codec::Indeo5,
            buf_invalid: [false; 4],
            p_frame: None,
        }
    }

    /// Reset to the state of a new stream.
    pub fn reset(&mut self) {
        *self = Self::new(self.codec);
    }

    /// Decode one frame. `Ok(None)` when the frame carries no picture (null frames, and the
    /// frames Indeo 4 holds back for B-frame reordering).
    pub fn decode(&mut self, buf: &[u8]) -> Result<Option<Picture>, Error> {
        self.decode_frame(buf)
    }

    // -------------------------------------------------------------------------------------
    // planes and tiles

    fn init_planes(&mut self, cfg: &PicConf) -> Result<(), Error> {
        self.planes = Default::default();
        check_size(cfg.pic_width, cfg.pic_height)?;
        if cfg.luma_bands < 1 || cfg.chroma_bands < 1 {
            return inv("band count");
        }
        let (w, h) = (cfg.pic_width as usize, cfg.pic_height as usize);
        let dims = [(w, h, cfg.luma_bands), ((w + 3) >> 2, (h + 3) >> 2, cfg.chroma_bands), ((w + 3) >> 2, (h + 3) >> 2, cfg.chroma_bands)];
        for (p, &(pw, ph, nb)) in dims.iter().enumerate() {
            let nb = nb as usize;
            let (bw, bh) = if nb == 1 { (pw, ph) } else { ((pw + 1) >> 1, (ph + 1) >> 1) };
            let align = if p > 0 { 8 } else { 16 };
            let (wa, ha) = (bw.next_multiple_of(align), bh.next_multiple_of(align));
            let mut bands = Vec::with_capacity(nb);
            for b in 0..nb {
                let mut band = Band::default();
                band.p.plane = p;
                band.p.band_num = b;
                band.p.width = bw as i32;
                band.p.height = bh as i32;
                band.p.pitch = wa as i32;
                band.p.aheight = ha as i32;
                band.p.bufsize = wa * ha;
                bands.push(band);
            }
            self.planes[p] = Plane { width: pw, height: ph, bands };
        }
        Ok(())
    }

    fn init_tiles(&mut self, tile_width: u32, tile_height: u32) -> Result<(), Error> {
        let mut ref_nums: Vec<usize> = Vec::new();
        for p in 0..3 {
            let mut t_width = if p == 0 { tile_width as i32 } else { (tile_width as i32 + 3) >> 2 };
            let mut t_height = if p == 0 { tile_height as i32 } else { (tile_height as i32 + 3) >> 2 };
            if p == 0 && self.planes[0].bands.len() == 4 {
                if t_width % 2 != 0 || t_height % 2 != 0 {
                    return Err(Error::Unsupported("odd tiles"));
                }
                t_width >>= 1;
                t_height >>= 1;
            }
            if t_width <= 0 || t_height <= 0 {
                return inv("tile size");
            }
            for b in 0..self.planes[p].bands.len() {
                let band = &mut self.planes[p].bands[b];
                band.tiles.clear();
                let mb_size = band.p.mb_size;
                if mb_size <= 0 {
                    return inv("macroblock size");
                }
                let mut y = 0;
                while y < band.p.height {
                    let mut x = 0;
                    while x < band.p.width {
                        let width = (band.p.width - x).min(t_width);
                        let height = (band.p.height - y).min(t_height);
                        let num_mbs = mbs_per_tile(width, height, mb_size);
                        if p != 0 || b != 0 {
                            let idx = band.tiles.len();
                            if ref_nums.get(idx) != Some(&num_mbs) {
                                return inv("reference tile mismatch");
                            }
                        }
                        band.tiles.push(Tile {
                            xpos: x,
                            ypos: y,
                            width,
                            height,
                            mb_size,
                            is_empty: false,
                            data_size: 0,
                            num_mbs,
                            mbs: vec![Mb::default(); num_mbs],
                        });
                        x += t_width;
                    }
                    y += t_height;
                }
                if p == 0 && b == 0 {
                    ref_nums = band.tiles.iter().map(|t| t.num_mbs).collect();
                }
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------------------------------
    // Indeo 4 headers

    fn pic_hdr4(&mut self, gb: &mut Bits<'_>) -> Result<(), Error> {
        if gb.get(18) != 0x3FFF8 {
            return inv("picture start code");
        }
        self.prev_frame_type = self.frame_type;
        self.frame_type = gb.get(3) as i32;
        if self.frame_type == 7 {
            return inv("frame type");
        }
        let _has_transp = gb.get1();
        if gb.get1() {
            return inv("sync bit is set");
        }
        let _data_size = if gb.get1() { gb.get(24) } else { 0 };
        if self.frame_type >= I4_NULL_FIRST {
            return Ok(());
        }
        // key lock: ignored
        if gb.get1() {
            gb.skip(32);
        }
        let pic_size_indx = gb.get(3) as usize;
        let mut pc = PicConf::default();
        if pic_size_indx == 7 {
            pc.pic_height = gb.get(16);
            pc.pic_width = gb.get(16);
        } else {
            pc.pic_height = u32::from(IVI4_COMMON_PIC_SIZES[pic_size_indx * 2 + 1]);
            pc.pic_width = u32::from(IVI4_COMMON_PIC_SIZES[pic_size_indx * 2]);
        }
        let scale_tile = |def: u32, f: u32| if f == 15 { def } else { (f + 1) << 5 };
        if gb.get1() {
            pc.tile_height = scale_tile(pc.pic_height, gb.get(4));
            pc.tile_width = scale_tile(pc.pic_width, gb.get(4));
        } else {
            pc.tile_height = pc.pic_height;
            pc.tile_width = pc.pic_width;
        }
        if gb.get(2) != 0 {
            return Err(Error::Unsupported("picture format other than YVU9"));
        }
        pc.chroma_height = (pc.pic_height + 3) >> 2;
        pc.chroma_width = (pc.pic_width + 3) >> 2;
        let subdivision = |gb: &mut Bits<'_>| -> u32 {
            match gb.get(2) {
                3 => 1,
                2 => {
                    for _ in 0..4 {
                        if gb.get(2) != 3 {
                            return 0;
                        }
                    }
                    4
                }
                _ => 0,
            }
        };
        pc.luma_bands = subdivision(gb);
        pc.chroma_bands = if pc.luma_bands != 0 { subdivision(gb) } else { 0 };
        check_size(pc.pic_width, pc.pic_height)?;
        let scalable = pc.luma_bands != 1 || pc.chroma_bands != 1;
        self.is_scalable = scalable;
        if scalable && (pc.luma_bands != 4 || pc.chroma_bands != 1) {
            return Err(Error::Unsupported("scalability subdivision"));
        }
        if pc != self.pic_conf {
            if let Err(e) = self.init_planes(&pc) {
                self.pic_conf.luma_bands = 0;
                return Err(e);
            }
            self.pic_conf = pc;
            for p in 0..3 {
                let n = if p == 0 { pc.luma_bands } else { pc.chroma_bands } as usize;
                for i in 0..n {
                    let band = &mut self.planes[p].bands[i];
                    band.p.mb_size = if p == 0 { if !scalable { 16 } else { 8 } } else { 4 };
                    band.p.blk_size = if p == 0 { 8 } else { 4 };
                }
            }
            self.init_tiles(pc.tile_width, pc.tile_height)?;
        }
        let _frame_num = if gb.get1() { gb.get(20) } else { 0 };
        if gb.get1() {
            gb.skip(8);
        }
        let c = gb.get1();
        dec_huff_desc(gb, c, false, &mut self.mb_vlc, Owner::CtxMb)?;
        let c = gb.get1();
        dec_huff_desc(gb, c, true, &mut self.blk_vlc, Owner::CtxBlk)?;
        let _rvmap_sel = if gb.get1() { gb.get(3) } else { 8 };
        let _in_imf = gb.get1();
        self.in_q = gb.get1();
        let _pic_glob_quant = gb.get(5);
        let _unknown1 = if gb.get1() { gb.get(3) } else { 0 };
        let _checksum = if gb.get1() { gb.get(16) } else { 0 };
        while gb.get1() {
            if gb.left() < 10 {
                return inv("picture header extension");
            }
            gb.skip(8);
        }
        let _bad_blocks = gb.get1();
        gb.align();
        Ok(())
    }

    fn band_hdr4(&self, gb: &mut Bits<'_>, arg: &mut Band) -> Result<(), Error> {
        let frame_type = self.frame_type;
        let ctx_blk_tab = self.blk_vlc.tab;
        let mut band = arg.p.clone();
        let plane = gb.get(2) as usize;
        let band_num = gb.get(4) as usize;
        if band.plane != plane || band.band_num != band_num {
            return inv("band header sequence");
        }
        band.is_empty = gb.get1();
        if !band.is_empty {
            let old_blk_size = band.blk_size;
            if gb.get1() {
                gb.skip(16);
            }
            band.is_halfpel = gb.get(2) as i32;
            if band.is_halfpel >= 2 {
                return inv("motion vector resolution");
            }
            if gb.get1() {
                gb.skip(16); // checksum
            }
            let indx = gb.get(2) as i32;
            if indx == 3 {
                return inv("block size");
            }
            band.mb_size = 16 >> indx;
            band.blk_size = 8 >> (indx >> 1);
            band.inherit_mv = gb.get1();
            band.inherit_qdelta = gb.get1();
            band.glob_quant = gb.get(5) as i32;
            if !gb.get1() || frame_type == I4_INTRA {
                let transform_id = gb.get(5) as usize;
                let Some((inv_t, dc_t, is_2d)) = i4_transform(transform_id) else {
                    return Err(Error::Unsupported("transform"));
                };
                if transform_id < 10 && band.blk_size < 8 {
                    return inv("wrong transform size");
                }
                band.inv_transform = Some(inv_t);
                band.dc_transform = Some(dc_t);
                band.is_2d_trans = is_2d;
                band.transform_size = if transform_id < 10 { 8 } else { 4 };
                if band.blk_size != band.transform_size {
                    return inv("transform and block size mismatch");
                }
                let scan_indx = gb.get(4) as usize;
                if scan_indx == 15 {
                    return Err(Error::Unsupported("custom scan pattern"));
                }
                if scan_indx > 4 && scan_indx < 10 {
                    if band.blk_size != 4 {
                        return inv("mismatching scan table");
                    }
                } else if band.blk_size != 8 {
                    return inv("mismatching scan table");
                }
                band.scan = Some(scan_index_to_tab(scan_indx));
                band.scan_size = band.blk_size;
                let quant_mat = gb.get(5) as usize;
                if quant_mat == 31 {
                    return Err(Error::Unsupported("custom quant matrix"));
                }
                if quant_mat >= QUANT_INDEX_TO_TAB.len() {
                    return inv("quantization matrix");
                }
                band.quant_mat = quant_mat;
            } else if old_blk_size != band.blk_size {
                return inv("block size does not match the inherited configuration");
            }
            let q = usize::from(QUANT_INDEX_TO_TAB[band.quant_mat]);
            if q > 4 && band.blk_size == 4 {
                return inv("quant matrix for 4x4 blocks");
            }
            if band.scan_size != band.blk_size {
                return inv("mismatching scan table");
            }
            if band.transform_size == 8 && band.blk_size < 8 {
                return inv("mismatching transform size");
            }
            // block Huffman codebook
            if !gb.get1() {
                arg.blk_vlc.tab = ctx_blk_tab;
            } else {
                dec_huff_desc(gb, true, true, &mut arg.blk_vlc, Owner::Band)?;
            }
            band.rvmap_sel = if gb.get1() { gb.get(3) as usize } else { 8 };
            band.num_corr = 0;
            if gb.get1() {
                band.num_corr = gb.get(8) as usize;
                if band.num_corr > 61 {
                    return inv("too many corrections");
                }
                for i in 0..band.num_corr * 2 {
                    band.corr[i] = gb.get(8) as u8;
                }
            }
        }
        let q = usize::from(QUANT_INDEX_TO_TAB[band.quant_mat]);
        if band.blk_size == 8 {
            band.intra_base = Some(&IVI4_QUANT_8X8_INTRA[q][..]);
            band.inter_base = Some(&IVI4_QUANT_8X8_INTER[q][..]);
        } else {
            band.intra_base = Some(&IVI4_QUANT_4X4_INTRA[q.min(4)][..]);
            band.inter_base = Some(&IVI4_QUANT_4X4_INTER[q.min(4)][..]);
        }
        band.intra_scale = None;
        band.inter_scale = None;
        gb.align();
        if band.scan.is_none() {
            return inv("band scan not set");
        }
        arg.p = band;
        Ok(())
    }

    // -------------------------------------------------------------------------------------
    // Indeo 5 headers

    fn gop_header5(&mut self, gb: &mut Bits<'_>) -> Result<(), Error> {
        self.gop_flags = gb.get(8) as u8;
        let gop_flags = self.gop_flags;
        let _gop_hdr_size = if gop_flags & 1 != 0 { gb.get(16) } else { 0 };
        if gop_flags & IVI5_IS_PROTECTED != 0 {
            let _lock_word = gb.get(32);
        }
        let tile_size = if gop_flags & 0x40 != 0 { 64u32 << gb.get(2) } else { 0 };
        if tile_size > 256 {
            return inv("tile size");
        }
        let mut pc = PicConf { luma_bands: gb.get(2) * 3 + 1, chroma_bands: gb.get(1) * 3 + 1, ..PicConf::default() };
        let is_scalable = pc.luma_bands != 1 || pc.chroma_bands != 1;
        if is_scalable && (pc.luma_bands != 4 || pc.chroma_bands != 1) {
            return Err(Error::Unsupported("scalability subdivision"));
        }
        let pic_size_indx = gb.get(4) as usize;
        if pic_size_indx == 15 {
            pc.pic_height = gb.get(13);
            pc.pic_width = gb.get(13);
        } else {
            pc.pic_height = u32::from(IVI5_COMMON_PIC_SIZES[pic_size_indx * 2 + 1]) << 2;
            pc.pic_width = u32::from(IVI5_COMMON_PIC_SIZES[pic_size_indx * 2]) << 2;
        }
        if gop_flags & 2 != 0 {
            return Err(Error::Unsupported("YV12 picture format"));
        }
        pc.chroma_height = (pc.pic_height + 3) >> 2;
        pc.chroma_width = (pc.pic_width + 3) >> 2;
        if tile_size == 0 {
            pc.tile_height = pc.pic_height;
            pc.tile_width = pc.pic_width;
        } else {
            pc.tile_height = tile_size;
            pc.tile_width = tile_size;
        }
        let mut blk_size_changed;
        if pc != self.pic_conf || self.gop_invalid {
            self.init_planes(&pc)?;
            self.pic_conf = pc;
            self.is_scalable = is_scalable;
            blk_size_changed = true;
        } else {
            blk_size_changed = false;
        }
        for p in 0..=1usize {
            let n = if p == 0 { pc.luma_bands } else { pc.chroma_bands } as usize;
            for i in 0..n {
                let band = &mut self.planes[p].bands[i].p;
                band.is_halfpel = gb.get(1) as i32;
                let mb_size_bit = gb.get(1);
                let blk_size = 8 >> gb.get(1) as i32;
                let mb_size = blk_size << u32::from(mb_size_bit == 0);
                if p == 0 && blk_size == 4 {
                    return Err(Error::Unsupported("4x4 luma blocks"));
                }
                blk_size_changed = mb_size != band.mb_size || blk_size != band.blk_size;
                if blk_size_changed {
                    band.mb_size = mb_size;
                    band.blk_size = blk_size;
                }
                if gb.get1() {
                    return Err(Error::Unsupported("extended transform info"));
                }
                match (p << 2) + i {
                    0 => {
                        band.inv_transform = Some(Inv::Slant8x8);
                        band.dc_transform = Some(Dc::Slant2d);
                        band.scan = Some(&ZIGZAG);
                        band.transform_size = 8;
                    }
                    1 => {
                        band.inv_transform = Some(Inv::RowSlant8);
                        band.dc_transform = Some(Dc::RowSlant);
                        band.scan = Some(&VERTICAL_SCAN_8X8);
                        band.transform_size = 8;
                    }
                    2 => {
                        band.inv_transform = Some(Inv::ColSlant8);
                        band.dc_transform = Some(Dc::ColSlant);
                        band.scan = Some(&HORIZONTAL_SCAN_8X8);
                        band.transform_size = 8;
                    }
                    3 => {
                        band.inv_transform = Some(Inv::PutPixels8x8);
                        band.dc_transform = Some(Dc::PutDcPixel8x8);
                        band.scan = Some(&HORIZONTAL_SCAN_8X8);
                        band.transform_size = 8;
                    }
                    4 => {
                        band.inv_transform = Some(Inv::Slant4x4);
                        band.dc_transform = Some(Dc::Slant2d);
                        band.scan = Some(&DIRECT_SCAN_4X4);
                        band.transform_size = 4;
                    }
                    _ => {}
                }
                band.is_2d_trans = matches!(band.inv_transform, Some(Inv::Slant8x8 | Inv::Slant4x4));
                if band.transform_size != band.blk_size {
                    return inv("transform and block size mismatch");
                }
                let quant_mat = if p == 0 { if pc.luma_bands > 1 { i + 1 } else { 0 } } else { 5 };
                if band.blk_size == 8 {
                    if quant_mat >= 5 {
                        return inv("quant matrix too large");
                    }
                    band.intra_base = Some(&IVI5_BASE_QUANT_8X8_INTRA[quant_mat][..]);
                    band.inter_base = Some(&IVI5_BASE_QUANT_8X8_INTER[quant_mat][..]);
                    band.intra_scale = Some(&IVI5_SCALE_QUANT_8X8_INTRA[quant_mat][..]);
                    band.inter_scale = Some(&IVI5_SCALE_QUANT_8X8_INTER[quant_mat][..]);
                } else {
                    band.intra_base = Some(&IVI5_BASE_QUANT_4X4_INTRA[..]);
                    band.inter_base = Some(&IVI5_BASE_QUANT_4X4_INTER[..]);
                    band.intra_scale = Some(&IVI5_SCALE_QUANT_4X4_INTRA[..]);
                    band.inter_scale = Some(&IVI5_SCALE_QUANT_4X4_INTER[..]);
                }
                if gb.get(2) != 0 {
                    return inv("end marker missing");
                }
            }
        }
        // copy chroma parameters into the 2nd chroma plane
        for i in 0..pc.chroma_bands as usize {
            let b1 = self.planes[1].bands[i].p.clone();
            let b2 = &mut self.planes[2].bands[i].p;
            b2.width = b1.width;
            b2.height = b1.height;
            b2.mb_size = b1.mb_size;
            b2.blk_size = b1.blk_size;
            b2.is_halfpel = b1.is_halfpel;
            b2.intra_base = b1.intra_base;
            b2.inter_base = b1.inter_base;
            b2.intra_scale = b1.intra_scale;
            b2.inter_scale = b1.inter_scale;
            b2.scan = b1.scan;
            b2.inv_transform = b1.inv_transform;
            b2.dc_transform = b1.dc_transform;
            b2.is_2d_trans = b1.is_2d_trans;
            b2.transform_size = b1.transform_size;
        }
        if blk_size_changed {
            self.init_tiles(pc.tile_width, pc.tile_height)?;
        }
        if gop_flags & 8 != 0 {
            if gb.get(3) != 0 {
                return inv("alignment bits are not zero");
            }
            if gb.get1() {
                gb.skip(24);
            }
        }
        gb.align();
        gb.skip(23);
        if gb.get1() {
            loop {
                let i = gb.get(16);
                if i & 0x8000 == 0 {
                    break;
                }
            }
        }
        gb.align();
        Ok(())
    }

    fn skip_hdr_extension(gb: &mut Bits<'_>) -> Result<(), Error> {
        loop {
            let len = gb.get(8) as isize;
            if 8 * len > gb.left() {
                return inv("header extension");
            }
            gb.skip(8 * len as usize);
            if len == 0 {
                return Ok(());
            }
        }
    }

    fn pic_hdr5(&mut self, gb: &mut Bits<'_>) -> Result<(), Error> {
        if gb.get(5) != 0x1F {
            return inv("picture start code");
        }
        self.prev_frame_type = self.frame_type;
        self.frame_type = gb.get(3) as i32;
        if self.frame_type >= 5 {
            self.frame_type = I5_INTRA;
            return inv("frame type");
        }
        let _frame_num = gb.get(8);
        if self.frame_type == I5_INTRA {
            match self.gop_header5(gb) {
                Ok(()) => self.gop_invalid = false,
                Err(e) => {
                    self.gop_invalid = true;
                    return Err(e);
                }
            }
        }
        if self.frame_type == I5_INTER_SCAL && !self.is_scalable {
            self.frame_type = I5_INTER;
            return inv("scalable inter frame in a non scalable stream");
        }
        if self.frame_type != I5_NULL {
            self.frame_flags = gb.get(8) as u8;
            let flags = self.frame_flags;
            let _pic_hdr_size = if flags & 1 != 0 { gb.get(24) } else { 0 };
            let _checksum = if flags & 0x10 != 0 { gb.get(16) } else { 0 };
            if flags & 0x20 != 0 {
                let _ = Self::skip_hdr_extension(gb);
            }
            dec_huff_desc(gb, flags & 0x40 != 0, false, &mut self.mb_vlc, Owner::CtxMb)?;
            gb.skip(3);
        }
        gb.align();
        Ok(())
    }

    fn band_hdr5(&self, gb: &mut Bits<'_>, band: &mut Band) -> Result<(), Error> {
        let frame_flags = self.frame_flags;
        let band_flags = gb.get(8);
        if band_flags & 1 != 0 {
            band.p.is_empty = true;
            return Ok(());
        }
        let _data_size = if frame_flags & 0x80 != 0 { gb.get(24) } else { 0 };
        band.p.inherit_mv = band_flags & 2 != 0;
        band.p.inherit_qdelta = band_flags & 8 != 0;
        band.p.qdelta_present = band_flags & 4 != 0;
        if !band.p.qdelta_present {
            band.p.inherit_qdelta = true;
        }
        band.p.num_corr = 0;
        if band_flags & 0x10 != 0 {
            band.p.num_corr = gb.get(8) as usize;
            if band.p.num_corr > 61 {
                return inv("too many corrections");
            }
            for i in 0..band.p.num_corr * 2 {
                band.p.corr[i] = gb.get(8) as u8;
            }
        }
        band.p.rvmap_sel = if band_flags & 0x40 != 0 { gb.get(3) as usize } else { 8 };
        dec_huff_desc(gb, band_flags & 0x80 != 0, true, &mut band.blk_vlc, Owner::Band)?;
        if gb.get1() {
            gb.skip(16); // checksum
        }
        band.p.glob_quant = gb.get(5) as i32;
        if band_flags & 0x20 != 0 {
            gb.align();
            let _ = Self::skip_hdr_extension(gb);
        }
        gb.align();
        Ok(())
    }

    // -------------------------------------------------------------------------------------
    // frame decoding

    fn is_nonnull_frame(&self) -> bool {
        match self.codec {
            Codec::Indeo4 => self.frame_type < I4_NULL_FIRST,
            Codec::Indeo5 => self.frame_type != I5_NULL,
        }
    }

    fn switch_buffers(&mut self) {
        match self.codec {
            Codec::Indeo4 => {
                let is_ref = |t: i32| matches!(t, I4_INTRA | I4_INTRA1 | I4_INTER);
                let (is_prev_ref, is_cur_ref) = (is_ref(self.prev_frame_type), is_ref(self.frame_type));
                if is_prev_ref && is_cur_ref {
                    std::mem::swap(&mut self.dst_buf, &mut self.ref_buf);
                } else if is_prev_ref {
                    std::mem::swap(&mut self.ref_buf, &mut self.b_ref_buf);
                    std::mem::swap(&mut self.dst_buf, &mut self.ref_buf);
                }
            }
            Codec::Indeo5 => {
                match self.prev_frame_type {
                    I5_INTRA | I5_INTER => {
                        self.buf_switch ^= 1;
                        self.dst_buf = self.buf_switch;
                        self.ref_buf = self.buf_switch ^ 1;
                    }
                    I5_INTER_SCAL => {
                        if !self.inter_scal {
                            self.ref2_buf = 2;
                            self.inter_scal = true;
                        }
                        std::mem::swap(&mut self.dst_buf, &mut self.ref2_buf);
                        self.ref_buf = self.ref2_buf;
                    }
                    I5_INTER_NOREF => {}
                    _ => {}
                }
                match self.frame_type {
                    I5_INTRA | I5_INTER => {
                        if self.frame_type == I5_INTRA {
                            self.buf_switch = 0;
                        }
                        self.inter_scal = false;
                        self.dst_buf = self.buf_switch;
                        self.ref_buf = self.buf_switch ^ 1;
                    }
                    _ => {}
                }
            }
        }
    }

    fn decode_frame(&mut self, buf: &[u8]) -> Result<Option<Picture>, Error> {
        let mut gb = Bits::new(buf);
        match self.codec {
            Codec::Indeo4 => self.pic_hdr4(&mut gb)?,
            Codec::Indeo5 => self.pic_hdr5(&mut gb)?,
        }
        if self.gop_invalid {
            return inv("invalid GOP");
        }
        if self.codec == Codec::Indeo4 && self.frame_type == I4_NULL_LAST {
            return Ok(self.p_frame.take());
        }
        if self.gop_flags & IVI5_IS_PROTECTED != 0 {
            return Err(Error::Unsupported("password-protected clip"));
        }
        if self.planes[0].bands.is_empty() {
            return inv("color planes not initialized yet");
        }
        self.switch_buffers();
        if self.is_nonnull_frame() {
            self.buf_invalid[self.dst_buf] = true;
            for p in 0..3 {
                for b in 0..self.planes[p].bands.len() {
                    self.decode_band(&mut gb, p, b)?;
                }
            }
            self.buf_invalid[self.dst_buf] = false;
        } else {
            if self.is_scalable {
                return inv("null frame in a scalable stream");
            }
            for p in 0..3 {
                let band = &self.planes[p].bands[0];
                if band.cur_buf.is_none_or(|i| band.bufs[i].is_empty()) {
                    return inv("no picture yet");
                }
            }
        }
        if self.buf_invalid[self.dst_buf] {
            return inv("invalid buffer");
        }
        if !self.is_nonnull_frame() {
            return Ok(None);
        }

        let (w, h) = (self.planes[0].width, self.planes[0].height);
        let mut pic = Picture {
            width: w as u32,
            height: h as u32,
            planes: Vec::new(),
            key: matches!(self.frame_type, 0) || (self.codec == Codec::Indeo4 && self.frame_type == I4_INTRA1),
        };
        // luma
        if self.is_scalable {
            let bands = &self.planes[0].bands;
            let get = |i: usize| bands[i].cur_buf.map_or(&[][..], |c| &bands[i].bufs[c][..]);
            let slices = [get(0), get(1), get(2), get(3)];
            let dp = w.next_multiple_of(2);
            let mut tmp = vec![0u8; dp * h.next_multiple_of(2)];
            let pitch = bands[0].p.pitch as usize;
            if self.codec == Codec::Indeo4 {
                dsp::recompose_haar(slices, pitch, w, h, &mut tmp, dp);
            } else {
                dsp::recompose53(slices, pitch, w, h, &mut tmp, dp);
            }
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                out.extend_from_slice(&tmp[y * dp..y * dp + w]);
            }
            pic.planes.push(out);
        } else {
            pic.planes.push(self.output_plane(0));
        }
        pic.planes.push(self.output_plane(2));
        pic.planes.push(self.output_plane(1));

        // Indeo 4 sends the next I frame together with the following P frame
        if self.codec == Codec::Indeo4 && self.frame_type == I4_INTRA {
            while gb.get(8) != 0 {
                if gb.left() < 8 {
                    return inv("version string");
                }
            }
            let left = gb.count() & 0x18;
            gb.skip(64 - left);
            if gb.left() > 18 && gb.peek(21) == 0xBFFF8 {
                let start = (gb.count() >> 3).min(buf.len());
                self.p_frame = None;
                if let Ok(Some(p)) = self.decode_frame(&buf[start..]) {
                    self.p_frame = Some(p);
                }
            }
        }
        Ok(Some(pic))
    }

    /// Convert and output a plane: add back the bias of 128 and clip.
    fn output_plane(&self, p: usize) -> Vec<u8> {
        let plane = &self.planes[p];
        let (w, h) = (plane.width, plane.height);
        let band = &plane.bands[0];
        let Some(src) = band.cur_buf.map(|c| &band.bufs[c]) else {
            return vec![0; w * h];
        };
        let pitch = band.p.pitch as usize;
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            out.extend(src[y * pitch..y * pitch + w].iter().map(|&v| (i32::from(v) + 128).clamp(0, 255) as u8));
        }
        out
    }

    fn resolve<'a>(&'a self, band_tab: &'a HuffTab, r: VlcRef) -> Option<&'a Vlc> {
        match r {
            VlcRef::None => None,
            VlcRef::StaticMb(i) => self.mb_statics.get(i),
            VlcRef::StaticBlk(i) => self.blk_statics.get(i),
            VlcRef::CtxMb => self.mb_vlc.cust_tab.as_ref(),
            VlcRef::CtxBlk => self.blk_vlc.cust_tab.as_ref(),
            VlcRef::Band => band_tab.cust_tab.as_ref(),
        }
    }

    fn decode_band(&mut self, gb: &mut Bits<'_>, p: usize, b: usize) -> Result<(), Error> {
        let mut band = std::mem::take(&mut self.planes[p].bands[b]);
        let r = self.decode_band_inner(gb, &mut band, p, b);
        self.planes[p].bands[b] = band;
        r
    }

    fn decode_band_inner(&mut self, gb: &mut Bits<'_>, band: &mut Band, p: usize, b: usize) -> Result<(), Error> {
        let luma_bands = self.pic_conf.luma_bands;
        let prepare = |band: &mut Band, i: usize| -> bool {
            if luma_bands <= 1 && i == 2 {
                return false;
            }
            if band.bufs[i].is_empty() {
                band.bufs[i] = vec![0; band.p.bufsize];
            }
            true
        };
        let dst_i = self.dst_buf;
        if !prepare(band, dst_i) {
            return inv("band buffer points to no data");
        }
        band.cur_buf = Some(dst_i);
        let (ref_i, bref_i) = if self.codec == Codec::Indeo4 && self.frame_type == I4_BIDIR {
            let r = self.b_ref_buf;
            let br = self.ref_buf;
            if !prepare(band, r) || !prepare(band, br) {
                return inv("no reference buffer");
            }
            (r, Some(br))
        } else {
            let r = self.ref_buf;
            if !prepare(band, r) {
                return inv("no reference buffer");
            }
            (r, None)
        };

        match self.codec {
            Codec::Indeo4 => self.band_hdr4(gb, band)?,
            Codec::Indeo5 => self.band_hdr5(gb, band)?,
        }
        if band.p.is_empty {
            return inv("empty band");
        }
        let bp = band.p.clone();
        if bp.rvmap_sel >= self.rvmaps.len() {
            return inv("rvmap");
        }
        // the selected rvmap table with this band's corrections applied
        let mut rv = self.rvmaps[bp.rvmap_sel].clone();
        for i in 0..bp.num_corr {
            let (i1, i2) = (usize::from(bp.corr[i * 2]), usize::from(bp.corr[i * 2 + 1]));
            rv.run.swap(i1, i2);
            rv.val.swap(i1, i2);
            if i1 == usize::from(rv.eob) || i2 == usize::from(rv.eob) {
                rv.eob ^= (i1 ^ i2) as u8;
            }
            if i1 == usize::from(rv.esc) || i2 == usize::from(rv.esc) {
                rv.esc ^= (i1 ^ i2) as u8;
            }
        }
        let base_mb_size = if p == 0 && b == 0 { bp.mb_size } else { self.planes[0].bands[0].p.mb_size };
        let mv_scale = (base_mb_size >> 3) - (bp.mb_size >> 3);

        let mut dstv = std::mem::take(&mut band.bufs[dst_i]);
        let dst_alias = if ref_i == dst_i || bref_i == Some(dst_i) { Some(dstv.clone()) } else { None };
        let ref_slice: &[i16] = if ref_i == dst_i { dst_alias.as_deref().unwrap_or(&[]) } else { &band.bufs[ref_i] };
        let bref_slice: Option<&[i16]> = bref_i.map(|i| if i == dst_i { dst_alias.as_deref().unwrap_or(&[]) } else { &band.bufs[i][..] });

        let blk_vlc = self.resolve(&band.blk_vlc, band.blk_vlc.tab).ok_or(Error::Invalid("no block Huffman table"));
        let blk_vlc = match blk_vlc {
            Ok(v) => v,
            Err(e) => {
                band.bufs[dst_i] = dstv;
                return Err(e);
            }
        };
        let mb_vlc = self.resolve(&band.blk_vlc, self.mb_vlc.tab);

        let mut pos = gb.count();
        let mut result = Ok(());
        for t in 0..band.tiles.len() {
            if band.tiles[t].mb_size != bp.mb_size {
                result = inv("macroblock sizes mismatch");
                break;
            }
            let mut tile = std::mem::take(&mut band.tiles[t]);
            tile.is_empty = gb.get1();
            let ref_mbs: Option<Vec<Mb>> = if p == 0 && b == 0 { None } else { self.planes[0].bands[0].tiles.get(t).map(|t| t.mbs.clone()) };
            let step = (|| -> Result<(), Error> {
                if tile.is_empty {
                    process_empty_tile(&bp, &mut tile, ref_mbs.as_deref(), mv_scale, &mut dstv, ref_slice)
                } else {
                    tile.data_size = dec_tile_data_size(gb);
                    if tile.data_size == 0 {
                        return inv("tile data size is zero");
                    }
                    let mbv = mb_vlc.ok_or(Error::Invalid("no macroblock Huffman table"))?;
                    match self.codec {
                        Codec::Indeo4 => decode_mb_info4(gb, &bp, &mut tile, ref_mbs.as_deref(), mv_scale, self.frame_type, self.in_q, mbv)?,
                        Codec::Indeo5 => decode_mb_info5(gb, &bp, &mut tile, ref_mbs.as_deref(), mv_scale, self.frame_type, self.frame_flags, mbv)?,
                    }
                    decode_blocks(gb, &bp, &tile, &rv, blk_vlc, self.codec, &mut dstv, ref_slice, bref_slice)?;
                    if ((gb.count() - pos) >> 3) != tile.data_size {
                        return inv("tile data size mismatch");
                    }
                    pos += tile.data_size << 3;
                    Ok(())
                }
            })();
            band.tiles[t] = tile;
            if let Err(e) = step {
                result = Err(e);
                break;
            }
        }
        band.bufs[dst_i] = dstv;
        gb.align();
        result
    }
}

/// Size of the tile data: one byte, or `0xFF` and three more (`ivi_dec_tile_data_size`).
fn dec_tile_data_size(gb: &mut Bits<'_>) -> usize {
    let mut len = 0;
    if gb.get1() {
        len = gb.get(8) as usize;
        if len == 255 {
            len = gb.get(24) as usize;
        }
    }
    gb.align();
    len
}

fn mv_in_bounds(bp: &BandParams, mb: &Mb, mv_x: i32, mv_y: i32) -> bool {
    let half = bp.is_halfpel;
    let dmv_x = mv_x >> half;
    let dmv_y = mv_y >> half;
    let (cx, cy) = (mv_x & half, mv_y & half);
    mb.xpos + dmv_x >= 0 && mb.xpos + dmv_x + bp.mb_size + cx <= bp.pitch && mb.ypos + dmv_y >= 0 && mb.ypos + dmv_y + bp.mb_size + cy <= bp.aheight
}

/// Motion compensation of one block (`ivi_mc`).
#[allow(clippy::too_many_arguments)]
fn ivi_mc(
    bp: &BandParams,
    dst: &mut [i16],
    refb: &[i16],
    brefb: Option<&[i16]>,
    avg: bool,
    delta: bool,
    offs: i64,
    mv_x: i32,
    mv_y: i32,
    mv_x2: i32,
    mv_y2: i32,
    mc_type: i32,
    mc_type2: i32,
) -> Result<(), Error> {
    let pitch = i64::from(bp.pitch);
    let ref_offs = offs + i64::from(mv_y) * pitch + i64::from(mv_x);
    let buf_size = pitch * i64::from(bp.aheight);
    let blk = i64::from(bp.blk_size);
    let min_size = pitch * (blk - 1) + blk;
    let ref_size = i64::from(mc_type > 1) * pitch + i64::from(mc_type & 1);
    let size = bp.blk_size as usize;
    let _ = avg;

    if mc_type != -1 && (offs < 0 || ref_offs < 0 || buf_size - min_size < offs || buf_size - min_size - ref_size < ref_offs) {
        return inv("motion vector outside the reference");
    }
    if mc_type2 == -1 {
        if mc_type != -1 {
            dsp::mc(dst, offs as usize, pitch as usize, refb, ref_offs as usize, pitch as usize, mc_type, size, delta);
        }
    } else {
        let ref_offs2 = offs + i64::from(mv_y2) * pitch + i64::from(mv_x2);
        let ref_size2 = i64::from(mc_type2 > 1) * pitch + i64::from(mc_type2 & 1);
        let Some(bref) = brefb else { return inv("no second reference") };
        if offs < 0 || ref_offs2 < 0 || buf_size - min_size - ref_size2 < ref_offs2 || buf_size - min_size < offs {
            return inv("motion vector outside the reference");
        }
        if mc_type == -1 {
            dsp::mc(dst, offs as usize, pitch as usize, bref, ref_offs2 as usize, pitch as usize, mc_type2, size, delta);
        } else {
            dsp::mc_avg(dst, offs as usize, pitch as usize, refb, ref_offs as usize, bref, ref_offs2 as usize, mc_type, mc_type2, size, delta);
        }
    }
    Ok(())
}

/// Copy or motion-compensate an empty tile (`ivi_process_empty_tile`).
fn process_empty_tile(bp: &BandParams, tile: &mut Tile, ref_mbs: Option<&[Mb]>, mv_scale: i32, dst: &mut [i16], refb: &[i16]) -> Result<(), Error> {
    let clear_first = !bp.qdelta_present && bp.plane == 0 && bp.band_num == 0;
    let mb_size = bp.mb_size;
    let xend = tile.xpos + tile.width;
    let pitch = bp.pitch;
    if tile.num_mbs != mbs_per_tile(tile.width, tile.height, mb_size) {
        return inv("tile size mismatch");
    }
    let mut offs = (tile.ypos * pitch + tile.xpos) as usize;
    let row_offset = (mb_size * pitch) as usize;
    let mut need_mc = false;
    let mut mbi = 0;
    let mut y = tile.ypos;
    while y < tile.ypos + tile.height {
        let mut mb_offset = offs;
        let mut x = tile.xpos;
        while x < xend {
            let mb = &mut tile.mbs[mbi];
            mb.xpos = x;
            mb.ypos = y;
            mb.buf_offs = mb_offset;
            mb.typ = 1;
            mb.cbp = 0;
            if clear_first {
                mb.q_delta = bp.glob_quant as i8;
                mb.mv_x = 0;
                mb.mv_y = 0;
            }
            if let Some(refs) = ref_mbs {
                let ref_mb = refs.get(mbi).ok_or(Error::Invalid("reference macroblock"))?;
                if bp.inherit_qdelta {
                    mb.q_delta = ref_mb.q_delta;
                }
                if bp.inherit_mv {
                    if mv_scale != 0 {
                        mb.mv_x = scale_mv(i32::from(ref_mb.mv_x), mv_scale) as i8;
                        mb.mv_y = scale_mv(i32::from(ref_mb.mv_y), mv_scale) as i8;
                    } else {
                        mb.mv_x = ref_mb.mv_x;
                        mb.mv_y = ref_mb.mv_y;
                    }
                    need_mc |= mb.mv_x != 0 || mb.mv_y != 0;
                    if !mv_in_bounds(bp, mb, i32::from(mb.mv_x), i32::from(mb.mv_y)) {
                        return inv("motion vector out of bounds");
                    }
                }
            }
            mbi += 1;
            mb_offset += mb_size as usize;
            x += mb_size;
        }
        offs += row_offset;
        y += mb_size;
    }
    if bp.inherit_mv && need_mc {
        let num_blocks = if mb_size != bp.blk_size { 4 } else { 1 };
        for mbn in 0..tile.num_mbs {
            let mb = tile.mbs[mbn];
            let mut mv_x = i32::from(mb.mv_x);
            let mut mv_y = i32::from(mb.mv_y);
            let mc_type;
            if bp.is_halfpel == 0 {
                mc_type = 0;
            } else {
                mc_type = ((mv_y & 1) << 1) | (mv_x & 1);
                mv_x >>= 1;
                mv_y >>= 1;
            }
            for blk in 0..num_blocks {
                let o = mb.buf_offs as i64 + i64::from(bp.blk_size) * i64::from((blk & 1) + i32::from(blk & 2 != 0) * pitch);
                ivi_mc(bp, dst, refb, None, false, false, o, mv_x, mv_y, 0, 0, mc_type, -1)?;
            }
        }
    } else {
        // copy the reference tile
        let (w, h) = (tile.width as usize, tile.height as usize);
        let start = (tile.ypos * pitch + tile.xpos) as usize;
        for yy in 0..h {
            let o = start + yy * pitch as usize;
            let src = refb.get(o..o + w).ok_or(Error::Invalid("tile outside the reference"))?;
            dst.get_mut(o..o + w).ok_or(Error::Invalid("tile outside the band"))?.copy_from_slice(src);
        }
    }
    Ok(())
}

/// Indeo 4 macroblock information of a tile (`indeo4.c decode_mb_info`).
#[allow(clippy::too_many_arguments)]
fn decode_mb_info4(
    gb: &mut Bits<'_>,
    bp: &BandParams,
    tile: &mut Tile,
    ref_mbs: Option<&[Mb]>,
    mv_scale: i32,
    frame_type: i32,
    in_q: bool,
    mb_vlc: &Vlc,
) -> Result<(), Error> {
    let row_offset = (bp.mb_size * bp.pitch) as usize;
    let mut offs = (tile.ypos * bp.pitch + tile.xpos) as usize;
    let blks_per_mb = if bp.mb_size != bp.blk_size { 4 } else { 1 };
    let mb_type_bits = if frame_type == I4_BIDIR { 2 } else { 1 };
    let (mut mv_x, mut mv_y) = (0i32, 0i32);
    if (((tile.width + bp.mb_size - 1) / bp.mb_size) * ((tile.height + bp.mb_size - 1) / bp.mb_size)) as usize != tile.num_mbs {
        return inv("macroblock count mismatch");
    }
    let vlc = |gb: &mut Bits<'_>| mb_vlc.get(gb).ok_or(Error::Invalid("macroblock code"));
    let mut mbi = 0;
    let mut y = tile.ypos;
    while y < tile.ypos + tile.height {
        let mut mb_offset = offs;
        let mut x = tile.xpos;
        while x < tile.xpos + tile.width {
            let ref_mb = ref_mbs.and_then(|r| r.get(mbi)).copied();
            let mb = &mut tile.mbs[mbi];
            mb.xpos = x;
            mb.ypos = y;
            mb.buf_offs = mb_offset;
            mb.b_mv_x = 0;
            mb.b_mv_y = 0;
            if gb.left() < 1 {
                return inv("insufficient input for macroblock info");
            }
            if gb.get1() {
                if frame_type == I4_INTRA {
                    return inv("empty macroblock in an intra picture");
                }
                mb.typ = 1;
                mb.cbp = 0;
                mb.q_delta = 0;
                if bp.plane == 0 && bp.band_num == 0 && in_q {
                    mb.q_delta = to_signed(vlc(gb)?) as i8;
                }
                mb.mv_x = 0;
                mb.mv_y = 0;
                if bp.inherit_mv
                    && let Some(r) = ref_mb
                {
                    inherit_mv(mb, &r, mv_scale);
                }
            } else {
                if bp.inherit_mv {
                    let Some(r) = ref_mb else { return inv("reference macroblock unavailable") };
                    mb.typ = r.typ;
                } else if frame_type == I4_INTRA || frame_type == I4_INTRA1 {
                    mb.typ = 0;
                } else {
                    mb.typ = gb.get(mb_type_bits) as u8;
                }
                mb.cbp = gb.get(blks_per_mb) as u8;
                mb.q_delta = 0;
                if bp.inherit_qdelta {
                    if let Some(r) = ref_mb {
                        mb.q_delta = r.q_delta;
                    }
                } else if mb.cbp != 0 || (bp.plane == 0 && bp.band_num == 0 && in_q) {
                    mb.q_delta = to_signed(vlc(gb)?) as i8;
                }
                if mb.typ == 0 {
                    mb.mv_x = 0;
                    mb.mv_y = 0;
                } else {
                    if bp.inherit_mv {
                        if let Some(r) = ref_mb {
                            inherit_mv(mb, &r, mv_scale);
                        }
                    } else {
                        mv_y += to_signed(vlc(gb)?);
                        mv_x += to_signed(vlc(gb)?);
                        mb.mv_x = mv_x as i8;
                        mb.mv_y = mv_y as i8;
                        if mb.typ == 3 {
                            mv_y += to_signed(vlc(gb)?);
                            mv_x += to_signed(vlc(gb)?);
                            mb.b_mv_x = (-mv_x) as i8;
                            mb.b_mv_y = (-mv_y) as i8;
                        }
                    }
                    if mb.typ == 2 {
                        mb.b_mv_x = (-i32::from(mb.mv_x)) as i8;
                        mb.b_mv_y = (-i32::from(mb.mv_y)) as i8;
                        mb.mv_x = 0;
                        mb.mv_y = 0;
                    }
                }
            }
            if mb.typ != 0 && !mv_in_buffer(bp, x, y, mb) {
                return inv("motion vector outside the reference");
            }
            mbi += 1;
            mb_offset += bp.mb_size as usize;
            x += bp.mb_size;
        }
        offs += row_offset;
        y += bp.mb_size;
    }
    gb.align();
    Ok(())
}

fn inherit_mv(mb: &mut Mb, r: &Mb, mv_scale: i32) {
    if mv_scale != 0 {
        mb.mv_x = scale_mv(i32::from(r.mv_x), mv_scale) as i8;
        mb.mv_y = scale_mv(i32::from(r.mv_y), mv_scale) as i8;
    } else {
        mb.mv_x = r.mv_x;
        mb.mv_y = r.mv_y;
    }
}

/// The motion vector lies inside the band buffer (the check of `decode_mb_info`).
fn mv_in_buffer(bp: &BandParams, x: i32, y: i32, mb: &Mb) -> bool {
    let s = bp.is_halfpel;
    let (mx, my) = (i32::from(mb.mv_x), i32::from(mb.mv_y));
    let pitch = i64::from(bp.pitch);
    let first = i64::from(x + (mx >> s)) + i64::from(y + (my >> s)) * pitch;
    let last = i64::from(x + ((mx + s) >> s) + bp.mb_size - 1) + i64::from(y + bp.mb_size - 1 + ((my + s) >> s)) * pitch;
    first >= 0 && last < bp.bufsize as i64
}

/// Indeo 5 macroblock information of a tile (`indeo5.c decode_mb_info`).
#[allow(clippy::too_many_arguments)]
fn decode_mb_info5(
    gb: &mut Bits<'_>,
    bp: &BandParams,
    tile: &mut Tile,
    ref_mbs: Option<&[Mb]>,
    mv_scale: i32,
    frame_type: i32,
    frame_flags: u8,
    mb_vlc: &Vlc,
) -> Result<(), Error> {
    let row_offset = (bp.mb_size * bp.pitch) as usize;
    let mut offs = (tile.ypos * bp.pitch + tile.xpos) as usize;
    if ref_mbs.is_none() && ((bp.qdelta_present && bp.inherit_qdelta) || bp.inherit_mv) {
        return inv("no reference macroblocks");
    }
    if tile.num_mbs != mbs_per_tile(tile.width, tile.height, bp.mb_size) {
        return inv("tile size mismatch");
    }
    let (mut mv_x, mut mv_y) = (0i32, 0i32);
    let vlc = |gb: &mut Bits<'_>| mb_vlc.get(gb).ok_or(Error::Invalid("macroblock code"));
    let mut mbi = 0;
    let mut y = tile.ypos;
    while y < tile.ypos + tile.height {
        let mut mb_offset = offs;
        let mut x = tile.xpos;
        while x < tile.xpos + tile.width {
            let ref_mb = ref_mbs.and_then(|r| r.get(mbi)).copied();
            let mb = &mut tile.mbs[mbi];
            mb.xpos = x;
            mb.ypos = y;
            mb.buf_offs = mb_offset;
            if gb.get1() {
                if frame_type == I5_INTRA {
                    return inv("empty macroblock in an intra picture");
                }
                mb.typ = 1;
                mb.cbp = 0;
                mb.q_delta = 0;
                if bp.plane == 0 && bp.band_num == 0 && frame_flags & 8 != 0 {
                    mb.q_delta = to_signed(vlc(gb)?) as i8;
                }
                mb.mv_x = 0;
                mb.mv_y = 0;
                if bp.inherit_mv
                    && let Some(r) = ref_mb
                {
                    inherit_mv(mb, &r, mv_scale);
                }
            } else {
                if let (true, Some(r)) = (bp.inherit_mv, ref_mb) {
                    mb.typ = r.typ;
                } else if frame_type == I5_INTRA {
                    mb.typ = 0;
                } else {
                    mb.typ = gb.get(1) as u8;
                }
                let blks_per_mb = if bp.mb_size != bp.blk_size { 4 } else { 1 };
                mb.cbp = gb.get(blks_per_mb) as u8;
                mb.q_delta = 0;
                if bp.qdelta_present {
                    if bp.inherit_qdelta {
                        if let Some(r) = ref_mb {
                            mb.q_delta = r.q_delta;
                        }
                    } else if mb.cbp != 0 || (bp.plane == 0 && bp.band_num == 0 && frame_flags & 8 != 0) {
                        mb.q_delta = to_signed(vlc(gb)?) as i8;
                    }
                }
                if mb.typ == 0 {
                    mb.mv_x = 0;
                    mb.mv_y = 0;
                } else if let (true, Some(r)) = (bp.inherit_mv, ref_mb) {
                    inherit_mv(mb, &r, mv_scale);
                } else {
                    mv_y += to_signed(vlc(gb)?);
                    mv_x += to_signed(vlc(gb)?);
                    mb.mv_x = mv_x as i8;
                    mb.mv_y = mv_y as i8;
                }
            }
            if mb.typ != 0 && !mv_in_buffer(bp, x, y, mb) {
                return inv("motion vector outside the reference");
            }
            mbi += 1;
            mb_offset += bp.mb_size as usize;
            x += bp.mb_size;
        }
        offs += row_offset;
        y += bp.mb_size;
    }
    gb.align();
    Ok(())
}

/// Decode, dequantise and inverse transform the blocks of a tile (`ivi_decode_blocks`).
#[allow(clippy::too_many_arguments)]
fn decode_blocks(
    gb: &mut Bits<'_>,
    bp: &BandParams,
    tile: &Tile,
    rv: &RvMap,
    vlc: &Vlc,
    codec: Codec,
    dst: &mut [i16],
    refb: &[i16],
    brefb: Option<&[i16]>,
) -> Result<(), Error> {
    let mut prev_dc = 0i32;
    let blk_size = bp.blk_size;
    let num_blocks = if bp.mb_size != blk_size { 4 } else { 1 };
    let (mut mc_type, mut mc_type2) = (0i32, -1i32);
    let (mut mv_x, mut mv_y, mut mv_x2, mut mv_y2) = (0i32, 0i32, 0i32, 0i32);
    let pitch = i64::from(bp.pitch);

    for mb in &tile.mbs {
        let is_intra = mb.typ == 0;
        let mut cbp = mb.cbp;
        let mut buf_offs = mb.buf_offs as i64;
        let quant = bp.glob_quant + i32::from(mb.q_delta);
        let mut quant = match codec {
            Codec::Indeo4 => quant.clamp(0, 31),
            Codec::Indeo5 => quant.clamp(0, 23),
        } as u32;
        let scale_tab = if is_intra { bp.intra_scale } else { bp.inter_scale };
        if let Some(t) = scale_tab {
            quant = u32::from(*t.get(quant as usize).ok_or(Error::Invalid("quantiser"))?);
        }
        if !is_intra {
            mv_x = i32::from(mb.mv_x);
            mv_y = i32::from(mb.mv_y);
            mv_x2 = i32::from(mb.b_mv_x);
            mv_y2 = i32::from(mb.b_mv_y);
            if bp.is_halfpel != 0 {
                mc_type = ((mv_y & 1) << 1) | (mv_x & 1);
                mc_type2 = ((mv_y2 & 1) << 1) | (mv_x2 & 1);
                mv_x >>= 1;
                mv_y >>= 1;
                mv_x2 >>= 1;
                mv_y2 >>= 1;
            }
            if mb.typ == 2 {
                mc_type = -1;
            }
            if mb.typ != 2 && mb.typ != 3 {
                mc_type2 = -1;
            }
            if mb.typ != 0 && !mv_in_bounds(bp, mb, i32::from(mb.mv_x), i32::from(mb.mv_y)) {
                return inv("motion vector out of bounds");
            }
            if (mb.typ == 2 || mb.typ == 3) && !mv_in_bounds(bp, mb, i32::from(mb.b_mv_x), i32::from(mb.b_mv_y)) {
                return inv("motion vector out of bounds");
            }
        }
        for blk in 0..num_blocks {
            if blk & 1 != 0 {
                buf_offs += i64::from(blk_size);
            } else if blk == 2 {
                buf_offs -= i64::from(blk_size);
                buf_offs += i64::from(blk_size) * pitch;
            }
            if cbp & 1 != 0 {
                decode_coded_block(
                    gb,
                    bp,
                    rv,
                    vlc,
                    dst,
                    refb,
                    brefb,
                    (mv_x, mv_y, mv_x2, mv_y2),
                    &mut prev_dc,
                    is_intra,
                    (mc_type, mc_type2),
                    quant,
                    buf_offs,
                )?;
            } else {
                let buf_size = pitch * i64::from(bp.aheight) - buf_offs;
                let min_size = i64::from(blk_size - 1) * pitch + i64::from(blk_size);
                if min_size > buf_size || buf_offs < 0 {
                    return inv("block outside the band");
                }
                if is_intra {
                    let dc = bp.dc_transform.ok_or(Error::Invalid("no DC transform"))?;
                    dsp::dc_transform(dc, prev_dc, dst, buf_offs as usize, pitch as usize, blk_size as usize);
                } else {
                    ivi_mc(bp, dst, refb, brefb, false, false, buf_offs, mv_x, mv_y, mv_x2, mv_y2, mc_type, mc_type2)?;
                }
            }
            cbp >>= 1;
        }
    }
    gb.align();
    Ok(())
}

/// One coded block (`ivi_decode_coded_blocks`).
#[allow(clippy::too_many_arguments)]
fn decode_coded_block(
    gb: &mut Bits<'_>,
    bp: &BandParams,
    rv: &RvMap,
    vlc: &Vlc,
    dst: &mut [i16],
    refb: &[i16],
    brefb: Option<&[i16]>,
    mv: (i32, i32, i32, i32),
    prev_dc: &mut i32,
    is_intra: bool,
    mc: (i32, i32),
    quant: u32,
    offs: i64,
) -> Result<(), Error> {
    let base_tab = if is_intra { bp.intra_base } else { bp.inter_base }.ok_or(Error::Invalid("no quantisation matrix"))?;
    let blk_size = bp.blk_size;
    let num_coeffs = (blk_size * blk_size) as usize;
    let col_mask = (blk_size - 1) as usize;
    let pitch = i64::from(bp.pitch);
    let min_size = pitch * i64::from(bp.transform_size - 1) + i64::from(bp.transform_size);
    let buf_size = pitch * i64::from(bp.aheight) - offs;
    if min_size > buf_size || offs < 0 {
        return inv("block outside the band");
    }
    let scan = bp.scan.ok_or(Error::Invalid("scan pattern is not set"))?;
    let mut trvec = [0i32; 64];
    let mut col_flags = [0u8; 8];
    let mut scan_pos: i32 = -1;
    let mut sym: u32 = 0;
    let get = |gb: &mut Bits<'_>| vlc.get(gb).ok_or(Error::Invalid("block code"));
    while scan_pos <= num_coeffs as i32 {
        sym = get(gb)?;
        if sym == u32::from(rv.eob) {
            break;
        }
        let (run, mut val);
        if sym == u32::from(rv.esc) {
            run = get(gb)? as i32 + 1;
            let lo = get(gb)?;
            let hi = get(gb)?;
            val = to_signed((hi << 6) | lo);
        } else {
            if sym >= 256 {
                return inv("invalid symbol");
            }
            run = i32::from(rv.run[sym as usize]);
            val = i32::from(rv.val[sym as usize]);
        }
        scan_pos += run;
        if scan_pos >= num_coeffs as i32 || scan_pos < 0 {
            break;
        }
        let pos = usize::from(scan[scan_pos as usize]);
        let q = (u32::from(*base_tab.get(pos).ok_or(Error::Invalid("quantisation matrix"))?)).wrapping_mul(quant) >> 9;
        if q > 1 {
            let sign: i32 = if val > 0 { 1 } else { -1 };
            val = (val as u32).wrapping_mul(q).wrapping_add((sign as u32).wrapping_mul(((q ^ 1).wrapping_sub(1)) >> 1)) as i32;
        }
        trvec[pos] = val;
        col_flags[pos & col_mask] |= u8::from(val != 0);
    }
    if scan_pos < 0 || (scan_pos >= num_coeffs as i32 && sym != u32::from(rv.eob)) {
        return inv("corrupt block data");
    }
    if is_intra && bp.is_2d_trans {
        *prev_dc = prev_dc.wrapping_add(trvec[0]);
        trvec[0] = *prev_dc;
        col_flags[0] |= u8::from(*prev_dc != 0);
    }
    if bp.transform_size > bp.blk_size {
        return inv("transform too large");
    }
    let t = bp.inv_transform.ok_or(Error::Invalid("no inverse transform"))?;
    dsp::inverse(t, &trvec, dst, offs as usize, pitch as usize, &col_flags);
    if !is_intra {
        return ivi_mc(bp, dst, refb, brefb, false, true, offs, mv.0, mv.1, mv.2, mv.3, mc.0, mc.1);
    }
    Ok(())
}
