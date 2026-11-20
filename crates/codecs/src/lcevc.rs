//! LCEVC enhancement layers (MPEG-5 Part 2) in imported files, found and decoded by
//! `cfa4-lcevc`.
//!
//! - **T.35 SEI in an H.264 / HEVC base** (MP4 / MOV, Matroska, MPEG-TS / PS / elementary
//!   streams): decoded. The base decoder is wrapped in an [`LcevcDecoder`] that takes the LCEVC
//!   access unit out of each sample and applies it to the picture with the same timestamp; the
//!   stream reports the enhanced size and its codec label gains [`DECODED_SUFFIX`].
//! - **A separate `lvc1` track or a TS `stream_type` 0x36 stream**: not decoded yet. The base
//!   layer plays and the label gains [`LABEL_SUFFIX`].

use std::collections::BTreeMap;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_lcevc::{BaseCodec, Framing, Picture};

use crate::Result;
use crate::video::{DecodedFrame, VideoDecoder};

/// Samples scanned at open for an LCEVC T.35 SEI (the enhancement rides in every picture, so
/// the first few decide).
pub(crate) const SCAN_SAMPLES: usize = 8;

/// What the codec label gains when the file carries LCEVC that CFA4 does not decode.
pub const LABEL_SUFFIX: &str = " + LCEVC (enhancement not decoded)";

/// What the codec label gains when the LCEVC enhancement is decoded.
pub const DECODED_SUFFIX: &str = " + LCEVC";

/// LCEVC access units waiting for their base picture; more than this means the stream's
/// timestamps don't line up, and the oldest are dropped.
const MAX_PENDING: usize = 64;

/// Mark a stream as carrying LCEVC that is not decoded: label and a warning in the log.
pub(crate) fn mark(name: &str, codec: &mut String, how: &str) {
    if !codec.ends_with(LABEL_SUFFIX) {
        codec.push_str(LABEL_SUFFIX);
    }
    log::warn!("{name}: LCEVC enhancement layer ({how}) found; CFA4 does not decode this LCEVC carriage yet, the base layer plays");
}

/// Mark a stream whose LCEVC enhancement is decoded.
pub(crate) fn mark_decoded(name: &str, codec: &mut String, (w, h): (u32, u32)) {
    if !codec.ends_with(DECODED_SUFFIX) {
        codec.push_str(DECODED_SUFFIX);
    }
    log::info!("{name}: LCEVC enhancement layer (T.35 SEI) decoded to {w}×{h}");
}

/// The base codec and NAL framing of an MP4 / Matroska sample entry, for the SEI scan.
pub(crate) fn base_of(codec: &cfa4_isobmff::CodecConfig) -> Option<(BaseCodec, Framing)> {
    match codec {
        cfa4_isobmff::CodecConfig::Avc(c) => Some((BaseCodec::H264, Framing::LengthPrefixed(c.length_size))),
        cfa4_isobmff::CodecConfig::Hevc(c) => Some((BaseCodec::Hevc, Framing::LengthPrefixed(c.length_size))),
        _ => None,
    }
}

/// LCEVC in a base track's T.35 SEI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Carriage {
    pub base: BaseCodec,
    pub framing: Framing,
    /// The enhanced picture size, when one of the scanned samples carries a global
    /// configuration; without it the enhancement is not decoded.
    pub size: Option<(u32, u32)>,
}

impl Carriage {
    /// Whether the enhancement can be decoded (see [`Carriage::size`]).
    pub fn decodable(&self) -> Option<(BaseCodec, Framing)> {
        self.size.map(|_| (self.base, self.framing))
    }
}

/// Scan a base track's first samples for LCEVC T.35 SEI.
pub(crate) fn scan(samples: impl IntoIterator<Item = Vec<u8>>, base: BaseCodec, framing: Framing) -> Option<Carriage> {
    let mut found = None;
    for s in samples.into_iter().take(SCAN_SAMPLES) {
        if let Some(nal) = cfa4_lcevc::extract_lcevc(&s, base, framing) {
            let size = cfa4_lcevc::Decoder::probe(&nal);
            let c = found.get_or_insert(Carriage { base, framing, size: None });
            if c.size.is_none() {
                c.size = size;
            }
        }
    }
    found
}

/// Mark a stream after [`scan`]: decoded at the enhanced size, or not decoded. Returns the
/// base codec and framing to wrap the decoder with, when decoded.
pub(crate) fn apply(name: &str, c: Carriage, codec: &mut String, size: &mut (u32, u32)) -> Option<(BaseCodec, Framing)> {
    match c.size {
        Some(s) => {
            *size = s;
            mark_decoded(name, codec, s);
            c.decodable()
        }
        None => {
            mark(name, codec, "T.35 SEI without a configuration in the first pictures");
            None
        }
    }
}

/// A base H.264 / HEVC decoder with the LCEVC enhancement applied to its pictures.
pub struct LcevcDecoder {
    inner: Box<dyn VideoDecoder>,
    base: BaseCodec,
    framing: Framing,
    lcevc: cfa4_lcevc::Decoder,
    /// LCEVC access units by the timestamp of the sample that carried them.
    pending: BTreeMap<i64, Vec<u8>>,
    warned: bool,
}

impl LcevcDecoder {
    pub fn new(inner: Box<dyn VideoDecoder>, base: BaseCodec, framing: Framing) -> Self {
        Self { inner, base, framing, lcevc: cfa4_lcevc::Decoder::new(), pending: BTreeMap::new(), warned: false }
    }

    /// Wrap `inner` when the stream carries decodable LCEVC.
    pub(crate) fn wrap(inner: Box<dyn VideoDecoder>, lcevc: Option<(BaseCodec, Framing)>) -> Box<dyn VideoDecoder> {
        match lcevc {
            Some((base, framing)) => Box::new(Self::new(inner, base, framing)),
            None => inner,
        }
    }

    fn enhance(&mut self, mut f: DecodedFrame) -> DecodedFrame {
        // pictures come out in presentation order: older access units have no picture left
        let nal = self.pending.remove(&f.pts);
        self.pending = self.pending.split_off(&f.pts);
        let Some(base) = to_picture(&f.frame) else { return f };
        let out = match nal {
            Some(nal) => self.lcevc.decode(&nal, &base).or_else(|e| {
                if !self.warned {
                    log::warn!("LCEVC enhancement not applied ({e}); the upscaled base picture is shown");
                    self.warned = true;
                }
                self.lcevc.upscale_only(&base)
            }),
            None => self.lcevc.upscale_only(&base),
        };
        if let Some(frame) = out.ok().and_then(|d| from_decoded(&f.frame, &d)) {
            f.frame = frame;
        }
        f
    }
}

impl VideoDecoder for LcevcDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if let Some(nal) = cfa4_lcevc::extract_lcevc(sample, self.base, self.framing) {
            self.pending.insert(pts, nal);
            while self.pending.len() > MAX_PENDING {
                self.pending.pop_first();
            }
        }
        let frames = self.inner.decode(sample, pts)?;
        Ok(frames.into_iter().map(|f| self.enhance(f)).collect())
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        let frames = self.inner.flush();
        frames.into_iter().map(|f| self.enhance(f)).collect()
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.lcevc.reset();
        self.pending.clear();
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.inner.is_random_access(sample)
    }
    // every picture updates the LCEVC temporal buffer: none can be left out
    fn is_disposable(&self, _sample: &[u8]) -> bool {
        false
    }
    fn set_draft(&mut self, on: bool) {
        self.inner.set_draft(on);
    }
    fn thread_limited(&self) -> bool {
        self.inner.thread_limited()
    }
}

fn lcevc_chroma(c: Chroma) -> cfa4_lcevc::Chroma {
    match c {
        Chroma::C420 => cfa4_lcevc::Chroma::C420,
        Chroma::C422 => cfa4_lcevc::Chroma::C422,
        Chroma::C444 => cfa4_lcevc::Chroma::C444,
    }
}

/// A base picture for the LCEVC decoder (planar Y'CbCr only).
fn to_picture(f: &VideoFrame) -> Option<Picture> {
    let (planes, chroma, bit_depth) = match &f.data {
        PixelData::Yuv8 { planes, chroma, .. } => (planes.iter().map(|p| p.iter().map(|&v| u16::from(v)).collect()).collect(), *chroma, 8),
        PixelData::Yuv16 { planes, chroma, bits, .. } => (planes.iter().map(|p| p.to_vec()).collect(), *chroma, *bits),
        _ => return None,
    };
    Some(Picture { width: f.width, height: f.height, bit_depth, chroma: lcevc_chroma(chroma), planes })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The enhanced picture as a frame: the conformance window cut, the base frame's colour,
/// timing and layout, its pixel aspect ratio scaled by the change of shape.
fn from_decoded(base: &VideoFrame, d: &cfa4_lcevc::Decoded) -> Option<VideoFrame> {
    let p = &d.picture;
    let [l, r, t, b] = d.crop;
    let ow = p.width.checked_sub(l.checked_add(r)?)?;
    let oh = p.height.checked_sub(t.checked_add(b)?)?;
    if ow == 0 || oh == 0 {
        return None;
    }
    let (chroma, (sx, sy)) = match p.chroma {
        cfa4_lcevc::Chroma::Mono | cfa4_lcevc::Chroma::C420 => (Chroma::C420, (1, 1)),
        cfa4_lcevc::Chroma::C422 => (Chroma::C422, (1, 0)),
        cfa4_lcevc::Chroma::C444 => (Chroma::C444, (0, 0)),
    };
    let neutral = 1u16 << p.bit_depth.clamp(1, 16).saturating_sub(1);
    let mut planes: Vec<Vec<u16>> = Vec::with_capacity(3);
    for k in 0..3 {
        let (shx, shy) = if k == 0 { (0, 0) } else { (sx, sy) };
        let (cw, ch) = (ow.div_ceil(1 << shx) as usize, oh.div_ceil(1 << shy) as usize);
        let Some(src) = p.planes.get(k) else {
            planes.push(vec![neutral; cw.checked_mul(ch)?]);
            continue;
        };
        let (pw, ph) = p.plane_size(k);
        let (x0, y0) = ((l >> shx) as usize, (t >> shy) as usize);
        let (pw, ph) = (pw as usize, ph as usize);
        if x0.checked_add(cw)? > pw || y0.checked_add(ch)? > ph {
            return None;
        }
        let mut out = Vec::with_capacity(cw.checked_mul(ch)?);
        for y in y0..y0 + ch {
            out.extend_from_slice(src.get(y * pw + x0..y * pw + x0 + cw)?);
        }
        planes.push(out);
    }
    let [y, u, v]: [Vec<u16>; 3] = planes.try_into().ok()?;
    let data = if p.bit_depth == 8 {
        let n = |p: Vec<u16>| std::sync::Arc::new(p.into_iter().map(|v| v.min(255) as u8).collect::<Vec<u8>>());
        PixelData::Yuv8 { planes: [n(y), n(u), n(v)], chroma, alpha: None }
    } else {
        let n = std::sync::Arc::new;
        PixelData::Yuv16 { planes: [n(y), n(u), n(v)], chroma, bits: p.bit_depth, alpha: None }
    };
    // the base's pixel aspect ratio, scaled by how much wider than tall the picture grew
    let (pn, pd) = (u64::from(base.par.0) * u64::from(base.width) * u64::from(oh), u64::from(base.par.1) * u64::from(base.height) * u64::from(ow));
    let g = gcd(pn, pd).max(1);
    let par = (u32::try_from(pn / g).ok()?, u32::try_from(pd / g).ok()?);
    let par = if par.0 == 0 || par.1 == 0 { (1, 1) } else { par };
    Some(VideoFrame { width: ow, height: oh, data, color: base.color, par, pts: base.pts, layout: base.layout })
}
