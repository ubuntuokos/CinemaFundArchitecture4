//! Sample parsing and reconstruction.

use cfa4_bitstream::BitReader;

use crate::tags::*;
use crate::vlc::{self, Peaks};
use crate::wavelet::{self, Post, Wavelet};
use crate::{EncodedFormat, Error, Flavor, Frame, Header, Plane, Result};

/// Largest accepted frame side.
const MAX_SIDE: u32 = 16384;
/// Largest accepted frame area (8K × 8K and a margin).
const MAX_PIXELS: u64 = 1 << 27;
/// Most wavelets a transform may have (`TRANSFORM_MAX_WAVELETS`).
const MAX_WAVELETS: usize = 8;
/// Most channels a sample may have (`TRANSFORM_MAX_CHANNELS`).
const MAX_CHANNELS: usize = 4;

/// One 32-bit tag/value pair.
#[derive(Clone, Copy, Debug)]
struct Tag {
    tag: u16,
    value: u16,
}

fn read_tag(data: &[u8], pos: usize) -> Option<Tag> {
    let b = data.get(pos..pos.checked_add(4)?)?;
    let raw = i16::from_be_bytes([*b.first()?, *b.get(1)?]);
    Some(Tag { tag: raw.unsigned_abs(), value: u16::from_be_bytes([*b.get(2)?, *b.get(3)?]) })
}

/// Header fields gathered from the tags.
#[derive(Clone, Copy, Debug, Default)]
struct Params {
    sample_type: u16,
    transform_type: u16,
    num_channels: u16,
    num_wavelets: u16,
    width: u16,
    height: u16,
    display_height: u16,
    encoded_format: u16,
    precision: u16,
    prescale: Option<u16>,
    version: Option<u16>,
}

impl Params {
    fn header(&self) -> Result<Header> {
        let format = EncodedFormat::from_tag(self.encoded_format)?;
        let (width, height) = (u32::from(self.width), u32::from(self.height));
        if width == 0 || height == 0 {
            return Err(Error::Invalid("missing frame size"));
        }
        if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Invalid("frame too large"));
        }
        let precision = match self.precision {
            0 => 8,
            p @ (8 | 10 | 12) => p as u8,
            _ => return Err(Error::Invalid("unsupported precision")),
        };
        let display_height = match u32::from(self.display_height) {
            0 => height,
            h => h.min(height),
        };
        let channels = match self.num_channels {
            0 => format.channels() as u8,
            n => n.min(255) as u8,
        };
        Ok(Header { width, height, display_height, format, precision, channels, intra: self.sample_type != SAMPLE_TYPE_GROUP, encoder_version: self.version })
    }

    /// Per-wavelet prescale shifts (`GetTransformPrescale`, overridden by `PRESCALE_TABLE`).
    fn prescale(&self, precision: u8) -> [u8; MAX_WAVELETS] {
        if let Some(v) = self.prescale {
            let mut p = [0u8; MAX_WAVELETS];
            for (i, s) in p.iter_mut().enumerate() {
                *s = ((v >> (14 - 2 * i)) & 3) as u8;
            }
            return p;
        }
        match precision {
            8 => [0; MAX_WAVELETS],
            10 => [0, 2, 0, 0, 0, 0, 0, 0],
            _ => [0, 2, 2, 0, 0, 0, 0, 0],
        }
    }
}

/// Header tags only (stops at the first coefficient data).
fn update_params(p: &mut Params, t: Tag) {
    match t.tag {
        SAMPLE => p.sample_type = if p.sample_type == 0 { t.value } else { p.sample_type },
        TRANSFORM_TYPE => p.transform_type = t.value,
        NUM_CHANNELS => p.num_channels = t.value,
        NUM_WAVELETS => p.num_wavelets = t.value,
        FRAME_WIDTH => p.width = t.value,
        FRAME_HEIGHT => p.height = t.value,
        FRAME_DISPLAY_HEIGHT => p.display_height = t.value,
        ENCODED_FORMAT => p.encoded_format = t.value,
        PRECISION => p.precision = t.value,
        PRESCALE_TABLE => p.prescale = Some(t.value),
        VERSION => p.version = Some(t.value),
        _ => {}
    }
}

/// Read the sample header: frame size, format and precision.
pub fn probe(data: &[u8]) -> Result<Header> {
    let mut p = Params::default();
    let mut pos = 0usize;
    while let Some(t) = read_tag(data, pos) {
        if t.tag == MARKER || t.tag == BAND_HEADER || t.tag == CHANNEL {
            break;
        }
        update_params(&mut p, t);
        pos += 4;
    }
    p.header()
}

/// Per-channel decoding state.
#[derive(Default)]
struct Channel {
    wavelets: Vec<Wavelet>,
}

impl Channel {
    fn wavelet(&mut self, index: usize, width: usize, height: usize) -> Result<&mut Wavelet> {
        let w = self.wavelets.get_mut(index).ok_or(Error::Invalid("subband out of range"))?;
        if w.bands.iter().all(Option::is_none) {
            w.width = width;
            w.height = height;
        } else if w.width != width || w.height != height {
            return Err(Error::Invalid("band size mismatch"));
        }
        Ok(w)
    }
}

/// The band about to be decoded (`BAND_*` tags).
#[derive(Clone, Copy, Debug, Default)]
struct BandParams {
    width: u16,
    height: u16,
    subband: u16,
    encoding: u16,
    quantization: u16,
    codebook: u8,
    difference: bool,
    peak_offset: u32,
    peak_base: Option<usize>,
    peak_level: Option<u16>,
}

/// Lowpass band parameters (`LOWPASS_*` tags).
#[derive(Clone, Copy, Debug, Default)]
struct LowpassParams {
    width: u16,
    height: u16,
    offset: u16,
    quantization: u16,
    depth: u16,
}

fn check_band(header: &Header, w: u16, h: u16) -> Result<(usize, usize)> {
    if w == 0 || h == 0 || u32::from(w) > header.width || u32::from(h) > header.height {
        return Err(Error::Invalid("bad band size"));
    }
    Ok((usize::from(w), usize::from(h)))
}

/// Decode one intra CineForm sample, matching the decoder of the encoder that wrote it (see
/// [`Flavor`]).
pub fn decode_frame(data: &[u8]) -> Result<Frame> {
    decode(data, None)
}

/// Decode one intra CineForm sample with the given rounding [`Flavor`].
pub fn decode_frame_as(data: &[u8], flavor: Flavor) -> Result<Frame> {
    decode(data, Some(flavor))
}

fn decode(data: &[u8], flavor: Option<Flavor>) -> Result<Frame> {
    let mut params = Params::default();
    let mut header: Option<Header> = None;
    let mut channels: Vec<Channel> = Vec::new();
    let mut current = 0usize;
    let mut band = BandParams::default();
    let mut lowpass = LowpassParams::default();
    let mut pos = 0usize;

    while let Some(t) = read_tag(data, pos) {
        pos += 4;
        if t.tag > LAST_NON_SIZED {
            if t.tag & CHUNK != 0 {
                let words = if t.tag & CHUNK24BIT != 0 { (u32::from(t.tag & 0xff) << 16) | u32::from(t.value) } else { u32::from(t.value) };
                pos = pos.saturating_add(words as usize * 4);
            } else if t.tag & 0xff00 == UNCOMPRESS {
                return Err(Error::Unsupported("uncompressed sample"));
            }
            continue;
        }
        if header.is_none() {
            update_params(&mut params, t);
        }
        match t.tag {
            INDEX => {
                // channel size table: `value` 32-bit words
                pos = pos.saturating_add(usize::from(t.value) * 4);
            }
            CHANNEL => {
                current = usize::from(t.value);
                if current >= channels.len() {
                    return Err(Error::Invalid("channel number out of range"));
                }
            }
            LOWPASS_WIDTH => lowpass.width = t.value,
            LOWPASS_HEIGHT => lowpass.height = t.value,
            PIXEL_OFFSET => lowpass.offset = t.value,
            QUANTIZATION => lowpass.quantization = t.value,
            PIXEL_DEPTH => lowpass.depth = t.value,
            BAND_WIDTH => band.width = t.value,
            BAND_HEIGHT => band.height = t.value,
            BAND_SUBBAND => band.subband = t.value,
            BAND_ENCODING => band.encoding = t.value,
            BAND_QUANTIZATION => band.quantization = t.value,
            BAND_CODING_FLAGS => {
                band.codebook = (t.value & 0xf) as u8;
                band.difference = (t.value >> 4) & 1 != 0;
            }
            PEAK_TABLE_OFFSET_L => {
                band.peak_offset = (band.peak_offset & !0xffff) | u32::from(t.value);
                band.peak_base = Some(pos);
                band.peak_level = None;
            }
            PEAK_TABLE_OFFSET_H => {
                band.peak_offset = (band.peak_offset & 0xffff) | (u32::from(t.value) << 16);
                band.peak_level = None;
            }
            PEAK_LEVEL => band.peak_level = Some(t.value),
            MARKER | BAND_HEADER => {
                let h = match header {
                    Some(h) => h,
                    None => {
                        let h = params.header()?;
                        start(&params, &h, &mut channels)?;
                        header = Some(h);
                        h
                    }
                };
                let ch = channels.get_mut(current).ok_or(Error::Invalid("channel number out of range"))?;
                if t.tag == MARKER {
                    if t.value == COEFFICIENT_START_CODE {
                        pos += decode_lowpass(data.get(pos..).unwrap_or(&[]), &h, &lowpass, ch)?;
                    }
                } else {
                    let rest = data.get(pos..).unwrap_or(&[]);
                    pos += decode_highpass(rest, data, &h, &band, ch)?;
                    band = BandParams::default();
                }
            }
            // everything else (versions, aspect ratio, metadata, trailers) does not affect decoding
            _ => {}
        }
    }
    let header = header.ok_or(Error::Invalid("no coefficients"))?;
    let flavor = flavor.unwrap_or(header.flavor());
    reconstruct(&params, &header, flavor, channels)
}

/// Check the transform structure once the header is complete and allocate the channels.
fn start(p: &Params, h: &Header, channels: &mut Vec<Channel>) -> Result<()> {
    if !h.intra || p.transform_type != TRANSFORM_TYPE_SPATIAL {
        return Err(Error::Unsupported("temporal (group of frames) transform"));
    }
    match h.format {
        EncodedFormat::Yuv422 | EncodedFormat::Rgb444 | EncodedFormat::Rgba4444 => {}
        EncodedFormat::Bayer => return Err(Error::Unsupported("Bayer encoding")),
        EncodedFormat::Yuva4444 => return Err(Error::Unsupported("YUVA 4:4:4:4 encoding")),
    }
    let n = usize::from(h.channels);
    if n == 0 || n > MAX_CHANNELS || n < h.format.channels() {
        return Err(Error::Invalid("bad channel count"));
    }
    let nw = usize::from(p.num_wavelets);
    if !(2..=MAX_WAVELETS).contains(&nw) {
        return Err(Error::Invalid("bad wavelet count"));
    }
    channels.clear();
    channels.resize_with(n, || Channel { wavelets: vec![Wavelet::default(); nw] });
    Ok(())
}

fn decode_lowpass(data: &[u8], h: &Header, lp: &LowpassParams, ch: &mut Channel) -> Result<usize> {
    let (w, ht) = check_band(h, lp.width, lp.height)?;
    let depth = u32::from(lp.depth);
    if !(1..=16).contains(&depth) {
        return Err(Error::Invalid("bad lowpass depth"));
    }
    let top = ch.wavelets.len().checked_sub(1).ok_or(Error::Invalid("no wavelets"))?;
    // a solid-colour lowpass band: 0xffffffff, colour, width, height (`DecodeSampleLowPassBand`)
    let word = |i: usize| data.get(i * 4..i * 4 + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes);
    if depth == 16 && word(0) == Some(u32::MAX) && word(2) == Some(w as u32) && word(3) == Some(ht as u32) {
        let color = word(1).unwrap_or(0) as u16 as i16;
        let wv = ch.wavelet(top, w, ht)?;
        wv.bands[0] = Some(vec![color; w * ht]);
        return Ok(16);
    }
    let mut br = BitReader::new(data);
    let mut v = Vec::with_capacity(w * ht);
    let (q, off) = (i32::from(lp.quantization), i32::from(lp.offset));
    for _ in 0..w * ht {
        let raw = br.read_bits(depth).map_err(|_| Error::Truncated)? as i32;
        v.push(raw.saturating_mul(q).saturating_add(off).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16);
    }
    let wv = ch.wavelet(top, w, ht)?;
    wv.bands[0] = Some(v);
    Ok(br.position().div_ceil(32) * 4)
}

fn decode_highpass(rest: &[u8], data: &[u8], h: &Header, b: &BandParams, ch: &mut Channel) -> Result<usize> {
    let (w, ht) = check_band(h, b.width, b.height)?;
    let nw = ch.wavelets.len();
    let s = usize::from(b.subband);
    if s == 0 || s > 3 * nw {
        return Err(Error::Invalid("bad subband"));
    }
    let index = nw - 1 - (s - 1) / 3;
    let band_index = (s - 1) % 3 + 1;
    let quant = i32::from(b.quantization.max(1));
    let mut coeffs = vec![0i16; w * ht];
    let used = match b.encoding {
        BAND_ENCODING_RUNLENGTHS => {
            let peaks = match (b.peak_level, b.peak_base) {
                (Some(level), Some(base)) if level > 0 => {
                    let start = base.saturating_add(b.peak_offset as usize);
                    Some(Peaks { level: i32::from(level), data: data.get(start..).unwrap_or(&[]) })
                }
                _ => None,
            };
            let stride = w.next_multiple_of(8);
            let mut padded = vec![0i16; stride * ht];
            let used = vlc::decode_band(rest, &mut padded, stride, w, b.codebook, quant, b.difference, peaks)?;
            for (dst, src) in coeffs.chunks_exact_mut(w).zip(padded.chunks_exact(stride)) {
                dst.copy_from_slice(src.get(..w).unwrap_or(&[]));
            }
            used
        }
        BAND_ENCODING_16BIT => {
            let bytes = rest.get(..w * ht * 2).ok_or(Error::Truncated)?;
            for (c, p) in coeffs.iter_mut().zip(bytes.chunks_exact(2)) {
                let v = i16::from_be_bytes([*p.first().unwrap_or(&0), *p.get(1).unwrap_or(&0)]);
                *c = (i32::from(v) * quant) as i16;
            }
            (w * ht * 2).div_ceil(4) * 4
        }
        _ => return Err(Error::Unsupported("band encoding")),
    };
    let wv = ch.wavelet(index, w, ht)?;
    if let Some(slot) = wv.bands.get_mut(band_index) {
        *slot = Some(coeffs);
    }
    Ok(used)
}

/// Run the inverse transforms of every channel and assemble the frame.
fn reconstruct(p: &Params, h: &Header, flavor: Flavor, channels: Vec<Channel>) -> Result<Frame> {
    let prescale = p.prescale(h.precision);
    let one = |(c, ch): (usize, Channel)| reconstruct_channel(h, flavor, &prescale, c, ch);
    #[cfg(feature = "threads")]
    let planes = {
        use rayon::prelude::*;
        channels.into_par_iter().enumerate().map(one).collect::<Result<Vec<Plane>>>()?
    };
    #[cfg(not(feature = "threads"))]
    let planes = channels.into_iter().enumerate().map(one).collect::<Result<Vec<Plane>>>()?;
    let mut planes = planes;
    // coded order Y Cr Cb / G R B A -> Y Cb Cr / R G B A
    match h.format {
        EncodedFormat::Yuv422 if planes.len() >= 3 => planes.swap(1, 2),
        EncodedFormat::Rgb444 | EncodedFormat::Rgba4444 if planes.len() >= 2 => planes.swap(0, 1),
        _ => {}
    }
    let display = h.display_height as usize;
    for pl in &mut planes {
        if display < pl.height {
            pl.data.truncate(display * pl.width);
            pl.height = display;
        }
    }
    Ok(Frame { width: h.width, height: h.display_height, format: h.format, bit_depth: h.precision, planes })
}

/// Inverse transforms of one channel: its output plane.
fn reconstruct_channel(h: &Header, flavor: Flavor, prescale: &[u8], c: usize, mut ch: Channel) -> Result<Plane> {
    let max = (1i32 << h.precision) - 1;
    let (pw, ph) = plane_size(h, c);
    let nw = ch.wavelets.len();
    for i in (1..nw).rev() {
        let wv = ch.wavelets.get_mut(i).ok_or(Error::Invalid("missing wavelet"))?;
        check_complete(wv)?;
        let shift = match prescale.get(i).copied().unwrap_or(0) {
            0 => None,
            1 => Some(0),
            // `InvertSpatialQuantDescale16s`: descale by 2 only at 10 bits and up
            _ => Some(u32::from(h.precision >= 10)),
        };
        let post = match (shift, flavor) {
            (None, _) => Post::Halve,
            (Some(s), Flavor::Sdk) => Post::Shift(s),
            (Some(s), Flavor::Ffmpeg) => Post::HalveShift(s),
        };
        let (ow, oh) = (2 * wv.width, 2 * wv.height);
        let out = wavelet::inverse_spatial(wv, post);
        let next = ch.wavelets.get_mut(i - 1).ok_or(Error::Invalid("missing wavelet"))?;
        if next.bands.iter().all(Option::is_none) {
            return Err(Error::Invalid("missing subband"));
        }
        next.bands[0] = Some(crop(&out, ow, oh, next.width, next.height)?);
    }
    let top = ch.wavelets.first().ok_or(Error::Invalid("missing wavelet"))?;
    check_complete(top)?;
    let full = wavelet::inverse_final(top, max);
    let mut data = crop(&full, 2 * top.width, 2 * top.height, pw, ph)?;
    if c == 3 && h.format == EncodedFormat::Rgba4444 && flavor == Flavor::Sdk {
        remove_alpha_curve(&mut data, h.precision);
    }
    Ok(Plane { width: pw, height: ph, data })
}

/// Every band of a wavelet must have been coded (the SDK's `BANDS_ALL_VALID`).
fn check_complete(w: &Wavelet) -> Result<()> {
    if w.bands.iter().any(Option::is_none) {
        return Err(Error::Invalid("missing subband"));
    }
    Ok(())
}

/// Undo the SDK encoder's alpha curve (`a' = 256 + a·223/256` for `0 < a < 4095` at 12 bits;
/// decoded as in the SDK's `Convert4444LinesToOutput`: `((a - 256) << 3) · 9400 >> 16`).
fn remove_alpha_curve(data: &mut [u16], precision: u8) {
    let up = 12u32.saturating_sub(u32::from(precision));
    let down = u32::from(precision).saturating_sub(12);
    for v in data {
        let a = (i32::from(*v) << up) >> down;
        let a = (((a - 256) << 3) * 9400) >> 16;
        *v = ((a.clamp(0, 4095) << down) >> up) as u16;
    }
}

fn plane_size(h: &Header, channel: usize) -> (usize, usize) {
    let (w, ht) = (h.width as usize, h.height as usize);
    match h.format {
        EncodedFormat::Yuv422 if channel > 0 => (w / 2, ht),
        _ => (w, ht),
    }
}

/// The top-left `w × h` part of a `sw × sh` image.
fn crop<T: Copy>(src: &[T], sw: usize, sh: usize, w: usize, h: usize) -> Result<Vec<T>> {
    if w > sw || h > sh || src.len() < sw * sh {
        return Err(Error::Invalid("band size mismatch"));
    }
    if w == sw {
        return Ok(src.get(..w * h).unwrap_or(&[]).to_vec());
    }
    Ok(src.chunks_exact(sw).take(h).flat_map(|r| r.iter().take(w).copied()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds samples tag by tag.
    struct Writer(Vec<u8>);

    impl Writer {
        fn tag(&mut self, tag: u16, value: u16) -> &mut Self {
            self.0.extend_from_slice(&tag.to_be_bytes());
            self.0.extend_from_slice(&value.to_be_bytes());
            self
        }
        fn optional(&mut self, tag: u16, value: u16) -> &mut Self {
            self.0.extend_from_slice(&(-(tag as i16)).to_be_bytes());
            self.0.extend_from_slice(&value.to_be_bytes());
            self
        }
        fn word(&mut self, w: u32) -> &mut Self {
            self.0.extend_from_slice(&w.to_be_bytes());
            self
        }
    }

    /// A three-wavelet 4:2:2 sample whose lowpass bands are solid and whose highpass bands are
    /// 16-bit coded zeros (`level_value` goes into the first coefficient of luma subband 4, in
    /// the prescaled middle level).
    fn flat_422(width: u16, height: u16, color: u16, sdk: bool, level_value: i16) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.tag(SAMPLE, SAMPLE_TYPE_IFRAME_TEST).tag(TRANSFORM_TYPE, 0).tag(NUM_CHANNELS, 3).tag(ENCODED_FORMAT, 1).tag(NUM_WAVELETS, 3);
        w.tag(FRAME_WIDTH, width).tag(FRAME_HEIGHT, height).tag(PRECISION, 10);
        if sdk {
            w.optional(VERSION, 0x9001);
        }
        for c in 0..3u16 {
            if c > 0 {
                w.tag(CHANNEL, c);
            }
            let cw = if c == 0 { width } else { width / 2 };
            let (lw, lh) = (cw / 8, height / 8);
            w.tag(LOWPASS_WIDTH, lw).tag(LOWPASS_HEIGHT, lh).tag(PIXEL_OFFSET, 0).tag(QUANTIZATION, 1).tag(PIXEL_DEPTH, 16);
            w.tag(MARKER, COEFFICIENT_START_CODE).word(u32::MAX).word(u32::from(color)).word(u32::from(lw)).word(u32::from(lh));
            for s in 1..=9u16 {
                let level = 3 - (s - 1) / 3;
                let (bw, bh) = (cw >> level, height >> level);
                w.tag(BAND_WIDTH, bw)
                    .tag(BAND_HEIGHT, bh)
                    .tag(BAND_SUBBAND, s)
                    .tag(BAND_ENCODING, BAND_ENCODING_16BIT)
                    .tag(BAND_QUANTIZATION, 1)
                    .tag(BAND_HEADER, 0);
                let n = usize::from(bw) * usize::from(bh);
                let mut data = vec![0u8; n * 2];
                if c == 0 && s == 4 {
                    data[..2].copy_from_slice(&level_value.to_be_bytes());
                }
                data.resize(data.len().div_ceil(4) * 4, 0);
                w.0.extend_from_slice(&data);
                w.tag(56, 0);
            }
        }
        w.0
    }

    const SAMPLE_TYPE_IFRAME_TEST: u16 = 9;

    #[test]
    fn solid_lowpass_and_16bit_bands_give_a_flat_frame() {
        // three levels each divide by 4, except the prescaled middle one: 8000 / 16 = 500
        let data = flat_422(64, 32, 8000, false, 0);
        let h = probe(&data).expect("probe");
        assert_eq!((h.width, h.height, h.format, h.precision, h.flavor()), (64, 32, EncodedFormat::Yuv422, 10, Flavor::Ffmpeg));
        let f = decode_frame(&data).expect("decode");
        assert_eq!(f.planes.len(), 3);
        assert_eq!((f.planes[1].width, f.planes[1].height), (32, 32));
        for p in &f.planes {
            assert!(p.data.iter().all(|&v| v == 500), "{:?}", &p.data[..8]);
        }
        let sdk = flat_422(64, 32, 8000, true, 0);
        assert_eq!(probe(&sdk).expect("probe").flavor(), Flavor::Sdk);
        assert_eq!(decode_frame(&sdk).expect("decode"), f);
    }

    #[test]
    fn flavors_differ_only_by_the_prescale_rounding() {
        let data = flat_422(64, 32, 8000, false, 3);
        let a = decode_frame_as(&data, Flavor::Ffmpeg).expect("decode");
        let b = decode_frame_as(&data, Flavor::Sdk).expect("decode");
        let max = a.planes[0].data.iter().zip(&b.planes[0].data).map(|(&x, &y)| (i32::from(x) - i32::from(y)).abs()).max();
        assert!(a != b && max <= Some(1), "max {max:?}");
        assert_eq!(a.planes[1], b.planes[1]);
    }

    #[test]
    fn alpha_curve_round_trips_the_extremes() {
        let mut a = vec![0u16, 256, 4095, 256 + 223 * 8];
        remove_alpha_curve(&mut a, 12);
        assert_eq!(&a[..3], &[0, 0, 4095]);
        assert!((i32::from(a[3]) - 2048).abs() <= 2, "{}", a[3]);
    }

    #[test]
    fn hostile_headers_are_errors() {
        let mut big = flat_422(64, 32, 8000, false, 0);
        // frame width tag -> 0xffff (wider than the limit)
        let pos = big.windows(2).position(|w| w == FRAME_WIDTH.to_be_bytes()).expect("tag");
        big[pos + 2..pos + 4].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(decode_frame(&big).is_err());
        assert!(decode_frame(&[]).is_err());
        assert!(probe(&[0, 1, 0, 9]).is_err());
        let data = flat_422(64, 32, 8000, false, 0);
        for cut in (0..data.len()).step_by(7) {
            let _ = decode_frame(&data[..cut]);
        }
        let mut x = 0x1234_5678u32;
        for _ in 0..300 {
            let mut d = data.clone();
            for _ in 0..4 {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let i = (x >> 8) as usize % d.len();
                d[i] ^= (x >> 24) as u8 | 1;
            }
            let _ = decode_frame(&d);
        }
    }
}
