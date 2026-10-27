//! Picture buffers and per-macroblock state.

use std::sync::{Arc, OnceLock};

/// Storage type of decoded samples: `u8` for 8-bit pictures (half the memory and cache traffic
/// of 16-bit storage in prediction reads, filters and output), `u16` for High 10 and above.
/// Values are always within `0..1 << bit_depth`.
pub trait Sample: Copy + Default + PartialEq + Eq + std::fmt::Debug + Send + Sync + 'static {
    /// More than 8 bits: arithmetic that fits 16-bit lanes for 8-bit samples needs 32-bit ones.
    const WIDE: bool;
    /// Storage of a value already clipped to the bit depth.
    fn from_i32(v: i32) -> Self;
    fn to_i32(self) -> i32;
    /// The rows of `f` when they are stored with this type.
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<Self>>]>;
    /// A finished plane for [`crate::Plane`].
    fn wrap_plane(v: Vec<Self>) -> crate::Plane;
    /// The samples of `p` when it holds this type.
    fn plane_slice(p: &crate::Plane) -> Option<&[Self]>;
    /// An empty output plane with room for `n` samples (from the host's allocator).
    fn plane_buffer(n: usize) -> Vec<Self>;
}

impl Sample for u8 {
    const WIDE: bool = false;
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u8
    }
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<u8>>]> {
        match &f.rows {
            Rows::U8(r) => Some(r),
            Rows::U16(_) => None,
        }
    }
    fn wrap_plane(v: Vec<u8>) -> crate::Plane {
        crate::Plane::U8(v)
    }
    fn plane_slice(p: &crate::Plane) -> Option<&[u8]> {
        p.as_u8()
    }
    fn plane_buffer(n: usize) -> Vec<u8> {
        plane_allocator()(n)
    }
}

impl Sample for u16 {
    const WIDE: bool = true;
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u16
    }
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<u16>>]> {
        match &f.rows {
            Rows::U16(r) => Some(r),
            Rows::U8(_) => None,
        }
    }
    fn wrap_plane(v: Vec<u16>) -> crate::Plane {
        crate::Plane::U16(v)
    }
    fn plane_slice(p: &crate::Plane) -> Option<&[u16]> {
        p.as_u16()
    }
    fn plane_buffer(n: usize) -> Vec<u16> {
        Vec::with_capacity(n)
    }
}

/// Chroma sampling of a picture: ChromaArrayType (6.2) 0 = monochrome (stored as neutral 4:2:0
/// planes), 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chroma(pub u8);

impl Chroma {
    /// log2(SubWidthC) of the stored chroma planes.
    #[inline(always)]
    pub fn sx(self) -> usize {
        (self.0 != 3) as usize
    }
    /// log2(SubHeightC) of the stored chroma planes.
    #[inline(always)]
    pub fn sy(self) -> usize {
        (self.0 < 2) as usize
    }
    /// MbWidthC.
    #[inline(always)]
    pub fn mb_w(self) -> usize {
        16 >> self.sx()
    }
    /// MbHeightC.
    #[inline(always)]
    pub fn mb_h(self) -> usize {
        16 >> self.sy()
    }
    /// Chroma is coded (not monochrome).
    #[inline(always)]
    pub fn coded(self) -> bool {
        self.0 != 0
    }
}

/// Planar sample buffers (MB-aligned dimensions, stride == width).
#[derive(Clone)]
pub struct Planes<S> {
    pub y: Vec<S>,
    pub cb: Vec<S>,
    pub cr: Vec<S>,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    pub chroma: Chroma,
}

impl<S: Sample> Planes<S> {
    /// Buffers for a `width × height` picture of `bit_depth` bits: luma black, chroma neutral.
    pub fn new(width: usize, height: usize, bit_depth: u32, chroma: Chroma) -> Self {
        let (cw, ch) = (width >> chroma.sx(), height >> chroma.sy());
        let mid = S::from_i32(1 << bit_depth.saturating_sub(1));
        Planes { y: vec![S::default(); width * height], cb: vec![mid; cw * ch], cr: vec![mid; cw * ch], width, height, cwidth: cw, chroma }
    }
    /// Every plane neutral (a picture that could not be decoded).
    pub fn gray(width: usize, height: usize, bit_depth: u32, chroma: Chroma) -> Self {
        let mut p = Self::new(width, height, bit_depth, chroma);
        p.y.fill(S::from_i32(1 << bit_depth.saturating_sub(1)));
        p
    }
}

/// Macroblock coding category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MbKind {
    #[default]
    None,
    I4x4,
    I8x8,
    I16x16,
    IPcm,
    PSkip,
    BSkip,
    BDirect16x16,
    /// Any other P or B inter macroblock.
    Inter,
}

impl MbKind {
    #[inline]
    pub fn is_intra(self) -> bool {
        matches!(self, MbKind::I4x4 | MbKind::I8x8 | MbKind::I16x16 | MbKind::IPcm)
    }
}

/// Per-macroblock state kept for the whole picture (neighbour derivations, deblocking, co-located data).
#[derive(Clone, Copy)]
pub struct MbState {
    /// Slice number within the picture; `u32::MAX` = not (yet) decoded.
    pub slice_num: u32,
    pub kind: MbKind,
    pub transform_8x8: bool,
    /// coded_block_pattern: bits 0..3 luma, bits 4..5 chroma.
    pub cbp: u8,
    /// QPY, which high bit depths allow down to -QpBdOffsetY.
    pub qp: i8,
    /// QPc for Cb / Cr (used by deblocking).
    pub qpc: [i8; 2],
    pub intra_chroma_mode: u8,
    /// Intra4x4PredMode / Intra8x8PredMode per 4x4 block (raster order).
    pub intra_modes: [u8; 16],
    /// CAVLC: TotalCoeff per luma 4x4 block (raster). CABAC: coded_block_flag.
    pub nnz: [u8; 16],
    /// Same for the chroma blocks [Cb, Cr][raster 4x4 index (row * 4 + column) of the MbWidthC / 4 x
    /// MbHeightC / 4 grid]: AC blocks, or every block in 4:4:4.
    pub nnz_c: [[u8; 16]; 2],
    /// CABAC coded_block_flag of DC blocks: bit0 luma (I16x16), bit1 Cb, bit2 Cr.
    pub cbf_dc: u8,
    /// Luma 4x4 blocks (raster bit) with non-zero coefficients, for deblocking bS = 2.
    pub nz_mask: u16,
    pub ref_idx: [[i8; 4]; 2],
    /// Motion vectors per 4x4 block (raster order).
    pub mv: [[[i16; 2]; 16]; 2],
    /// |mvd| per 4x4 block (raster), saturated, for CABAC context selection.
    pub mvd: [[[u8; 2]; 16]; 2],
    /// Bit per 8x8 block predicted in direct mode.
    pub direct8x8: u8,
}

impl MbState {
    #[inline]
    pub fn kind_is_skip(&self) -> bool {
        matches!(self.kind, MbKind::PSkip | MbKind::BSkip)
    }
}

impl Default for MbState {
    fn default() -> Self {
        MbState {
            slice_num: u32::MAX,
            kind: MbKind::None,
            transform_8x8: false,
            cbp: 0,
            qp: 0,
            qpc: [0; 2],
            intra_chroma_mode: 0,
            intra_modes: [2; 16],
            nnz: [0; 16],
            nnz_c: [[0; 16]; 2],
            cbf_dc: 0,
            nz_mask: 0,
            ref_idx: [[-1; 4]; 2],
            mv: [[[0; 2]; 16]; 2],
            mvd: [[[0; 2]; 16]; 2],
            direct8x8: 0,
        }
    }
}

/// One published macroblock row of a decoded frame: final (deblocked) samples plus the motion data
/// needed as co-located information for direct prediction.
pub struct FrameRow<S> {
    /// 16 luma lines.
    pub y: Box<[S]>,
    /// MbHeightC lines per chroma component.
    pub cb: Box<[S]>,
    pub cr: Box<[S]>,
    /// Per 4x4 block: index mb_x * 16 + raster.
    pub mv: [Box<[[i16; 2]]>; 2],
    /// Per 8x8 block: index mb_x * 4 + b8.
    pub ref_idx: [Box<[i8]>; 2],
    /// Unique id of the referenced picture per 8x8 block (u32::MAX = none).
    pub ref_id: [Box<[u32]>; 2],
    pub intra: Box<[bool]>,
}

/// A decoded (or in-progress) frame as stored in the DPB and referenced by later pictures.
///
/// Rows are published in order by the thread decoding the picture; readers block (per row) until the
/// data they need is available, which lets several pictures decode concurrently.
pub struct Frame {
    pub id: u32,
    pub poc: i32,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    pub cheight: usize,
    pub chroma: Chroma,
    pub mb_w: usize,
    /// Bits per sample of both luma and chroma (8 for 8-bit storage, 9..=14 for 16-bit).
    pub bit_depth: u32,
    rows: Rows,
}

/// The published rows of a frame, in the storage type its bit depth needs.
pub enum Rows {
    U8(Box<[OnceLock<FrameRow<u8>>]>),
    U16(Box<[OnceLock<FrameRow<u16>>]>),
}

impl Rows {
    fn len(&self) -> usize {
        match self {
            Rows::U8(r) => r.len(),
            Rows::U16(r) => r.len(),
        }
    }
}

impl Frame {
    pub fn new(id: u32, poc: i32, mb_w: usize, mb_h: usize, bit_depth: u32, chroma: Chroma) -> Self {
        let rows = if bit_depth > 8 { Rows::U16((0..mb_h).map(|_| OnceLock::new()).collect()) } else { Rows::U8((0..mb_h).map(|_| OnceLock::new()).collect()) };
        Frame { id, poc, width: mb_w * 16, height: mb_h * 16, cwidth: mb_w * chroma.mb_w(), cheight: mb_h * chroma.mb_h(), chroma, mb_w, bit_depth, rows }
    }

    pub fn mb_h(&self) -> usize {
        self.rows.len()
    }

    /// Publish MB row `r` (ignored if already published, or if the frame stores another type).
    pub fn publish<S: Sample>(&self, r: usize, row: FrameRow<S>) {
        if let Some(slot) = S::rows(self).and_then(|rows| rows.get(r)) {
            let _ = slot.set(row);
        }
    }

    pub fn is_published(&self, r: usize) -> bool {
        match &self.rows {
            Rows::U8(rows) => rows.get(r).is_some_and(|s| s.get().is_some()),
            Rows::U16(rows) => rows.get(r).is_some_and(|s| s.get().is_some()),
        }
    }

    /// MB row `r` of a frame stored as `S`, waiting until it has been published.
    #[inline]
    pub fn row<S: Sample>(&self, r: usize) -> Option<&FrameRow<S>> {
        let slot = S::rows(self)?.get(r)?;
        #[cfg(feature = "threads")]
        {
            Some(slot.wait())
        }
        #[cfg(not(feature = "threads"))]
        {
            slot.get()
        }
    }

    /// Wait until the whole frame is available.
    pub fn wait_complete(&self) {
        for r in 0..self.rows.len() {
            match &self.rows {
                Rows::U8(_) => {
                    let _ = self.row::<u8>(r);
                }
                Rows::U16(_) => {
                    let _ = self.row::<u16>(r);
                }
            }
        }
    }

    /// Build a published row from a contiguous plane set (rows of MB row `r`) and motion data.
    pub fn make_row<S: Sample>(planes: &Planes<S>, r: usize, mbs: &[MbState], ref_ids: &dyn Fn(&MbState, usize, i8) -> u32) -> FrameRow<S> {
        let w = planes.width;
        let cw = planes.cwidth;
        let mbh = planes.chroma.mb_h();
        let mb_w = w / 16;
        let y = planes.y.get(r * 16 * w..(r + 1) * 16 * w).unwrap_or(&[]).into();
        let cb = planes.cb.get(r * mbh * cw..(r + 1) * mbh * cw).unwrap_or(&[]).into();
        let cr = planes.cr.get(r * mbh * cw..(r + 1) * mbh * cw).unwrap_or(&[]).into();
        let row_mbs = mbs.get(r * mb_w..(r + 1) * mb_w).unwrap_or(&[]);
        let mut mv = [vec![[0i16; 2]; mb_w * 16], vec![[0i16; 2]; mb_w * 16]];
        let mut ref_idx = [vec![-1i8; mb_w * 4], vec![-1i8; mb_w * 4]];
        let mut ref_id = [vec![u32::MAX; mb_w * 4], vec![u32::MAX; mb_w * 4]];
        let mut intra = vec![true; mb_w];
        for (x, st) in row_mbs.iter().enumerate() {
            if st.slice_num == u32::MAX {
                continue;
            }
            intra[x] = st.kind.is_intra();
            for l in 0..2 {
                mv[l][x * 16..x * 16 + 16].copy_from_slice(&st.mv[l]);
                for b in 0..4 {
                    let ri = st.ref_idx[l][b];
                    ref_idx[l][x * 4 + b] = ri;
                    if ri >= 0 {
                        ref_id[l][x * 4 + b] = ref_ids(st, l, ri);
                    }
                }
            }
        }
        let [mv0, mv1] = mv;
        let [ri0, ri1] = ref_idx;
        let [id0, id1] = ref_id;
        FrameRow { y, cb, cr, mv: [mv0.into(), mv1.into()], ref_idx: [ri0.into(), ri1.into()], ref_id: [id0.into(), id1.into()], intra: intra.into() }
    }

    /// A frame with every row published from `planes` and no motion (intra), e.g. "non-existing"
    /// frames inferred for frame_num gaps.
    pub fn from_planes<S: Sample>(id: u32, poc: i32, planes: &Planes<S>, bit_depth: u32) -> Self {
        let (mb_w, mb_h) = (planes.width / 16, planes.height / 16);
        let f = Frame::new(id, poc, mb_w, mb_h, bit_depth, planes.chroma);
        let blank = vec![MbState::default(); mb_w * mb_h];
        for r in 0..mb_h {
            f.publish(r, Frame::make_row(planes, r, &blank, &|_, _, _| u32::MAX));
        }
        f
    }

    /// Copy the w x h luma window at (x0, y0) (clamped to the picture) into `out` (stride `os`).
    #[inline]
    pub fn luma_window<S: Sample>(&self, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        let (width, height) = (self.width, self.height);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, height as i32 - 1) as usize;
            let Some(row) = self.row::<S>(yy >> 4) else { return };
            let Some(line) = row.y.get((yy & 15) * width..(yy & 15) * width + width) else { return };
            if let Some(dst) = out.get_mut(r * os..) {
                copy_line::<S, 24>(line, x0, dst, w);
            }
        }
    }

    /// Copy the w x h window of chroma component `c` at (x0, y0) (clamped) into `out`. When `w`
    /// is at most `N`, `out` must have room for `N` samples after the start of each line. `SHIFT`
    /// is log2(MbHeightC): 3 for 4:2:0, 4 for 4:2:2 and 4:4:4.
    #[inline]
    pub fn chroma_window<S: Sample, const N: usize, const SHIFT: u32>(&self, c: usize, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        let (cw, ch) = (self.cwidth, self.cheight);
        let (shift, mask) = (SHIFT, (1usize << SHIFT) - 1);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, ch as i32 - 1) as usize;
            let Some(row) = self.row::<S>(yy >> shift) else { return };
            let plane = if c == 0 { &row.cb } else { &row.cr };
            let Some(line) = plane.get((yy & mask) * cw..(yy & mask) * cw + cw) else { return };
            if let Some(dst) = out.get_mut(r * os..) {
                copy_line::<S, N>(line, x0, dst, w);
            }
        }
    }

    /// Crop and copy the frame into planar buffers (waits for completion).
    pub fn copy_cropped(&self, crop: (usize, usize, usize, usize)) -> (crate::Plane, crate::Plane, crate::Plane) {
        match &self.rows {
            Rows::U8(_) => self.copy_cropped_as::<u8>(crop),
            Rows::U16(_) => self.copy_cropped_as::<u16>(crop),
        }
    }

    /// The chroma rectangle (x, y, width, height) of the luma crop rectangle `crop`.
    pub fn chroma_crop(&self, crop: (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
        let (cx, cy, cw, ch) = crop;
        let (sx, sy) = (self.chroma.sx(), self.chroma.sy());
        (cx >> sx, cy >> sy, (cw + (1 << sx) - 1) >> sx, (ch + (1 << sy) - 1) >> sy)
    }

    fn copy_cropped_as<S: Sample>(&self, crop: (usize, usize, usize, usize)) -> (crate::Plane, crate::Plane, crate::Plane) {
        let (cx, cy, cw, ch) = crop;
        let mut y = S::plane_buffer(cw * ch);
        for r in cy..cy + ch {
            let Some(row) = self.row::<S>(r >> 4) else { break };
            y.extend_from_slice(row.y.get((r & 15) * self.width + cx..(r & 15) * self.width + cx + cw).unwrap_or(&[]));
        }
        let (ccx, ccy, ccw, cch) = self.chroma_crop(crop);
        let shift = 4 - self.chroma.sy();
        let mask = (1 << shift) - 1;
        let mut u = S::plane_buffer(ccw * cch);
        let mut v = S::plane_buffer(ccw * cch);
        for r in ccy..ccy + cch {
            let Some(row) = self.row::<S>(r >> shift) else { break };
            let o = (r & mask) * self.cwidth + ccx;
            u.extend_from_slice(row.cb.get(o..o + ccw).unwrap_or(&[]));
            v.extend_from_slice(row.cr.get(o..o + ccw).unwrap_or(&[]));
        }
        (S::wrap_plane(y), S::wrap_plane(u), S::wrap_plane(v))
    }
}

static PLANE_ALLOCATOR: std::sync::OnceLock<fn(usize) -> Vec<u8>> = std::sync::OnceLock::new();

/// Where output planes come from: an empty buffer with room for the given number of samples.
/// The host sets it once to recycle the planes of pictures it is done with; the first call wins.
pub fn set_plane_allocator(alloc: fn(usize) -> Vec<u8>) {
    let _ = PLANE_ALLOCATOR.set(alloc);
}

fn plane_allocator() -> fn(usize) -> Vec<u8> {
    PLANE_ALLOCATOR.get().copied().unwrap_or(Vec::with_capacity)
}

/// Copy `w` samples of `line` starting at x0 into `out`; when possible a fixed-size block of `N`
/// bytes is copied instead (cheaper than a variable-length copy; `out` must then have room for `N`).
#[inline(always)]
fn copy_line<S: Sample, const N: usize>(line: &[S], x0: i32, out: &mut [S], w: usize) {
    if w <= N
        && x0 >= 0
        && let Some(src) = line.get(x0 as usize..).and_then(|l| l.first_chunk::<N>())
        && let Some(dst) = out.first_chunk_mut::<N>()
    {
        *dst = *src;
    } else {
        copy_clamped(line, x0, &mut out[..w]);
    }
}

/// Copy `out.len()` samples of `line` starting at x0, replicating edge samples outside the line.
#[inline(always)]
fn copy_clamped<S: Sample>(line: &[S], x0: i32, out: &mut [S]) {
    let w = out.len();
    if x0 >= 0 && x0 as usize + w <= line.len() {
        if let Some(src) = line.get(x0 as usize..x0 as usize + w) {
            out.copy_from_slice(src);
        }
    } else {
        let max = line.len() as i32 - 1;
        for (c, o) in out.iter_mut().enumerate() {
            *o = line.get((x0 + c as i32).clamp(0, max) as usize).copied().unwrap_or_default();
        }
    }
}

pub type FrameRef = Arc<Frame>;

/// An entry of RefPicList0/1 for the current slice.
#[derive(Clone)]
pub struct RefPic {
    pub frame: FrameRef,
    pub long_term: bool,
}

impl RefPic {
    pub fn poc(&self) -> i32 {
        self.frame.poc
    }
    pub fn id(&self) -> u32 {
        self.frame.id
    }
}
