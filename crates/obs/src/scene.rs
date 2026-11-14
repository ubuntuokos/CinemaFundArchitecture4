//! Scenes the host shows (CFA4-LIVE-STUDIO-001 phase S1).
//!
//! The engine keeps the scenes in the project (`cfa4_project::studio`) and sends the host the one
//! on the program as a [`HostScene`]: a list of layers, bottom first, each a picture and where to
//! put it on the canvas. The engine renders the pictures with CFA4's own code (colours, picture
//! files through CFA4's decoders, text through the title renderer) and hands them over as raw
//! RGBA files in a private directory, so the host needs no decoders and libobs no plugins.
//!
//! Both backends show the same layers with the same geometry, the one OBS Studio's scene items
//! use ([`Transform`]): the software backend composites them here ([`compose`]), libobs places
//! them as scene items of a CFA4 picture source.
//!
//! Moving pictures (S1b: a media file playing, the CFA4 live graphics program) are *feeds*: the
//! engine writes their frames (straight RGBA) into a frame ring of their own
//! ([`crate::ring::create_sized`]), and the host shows the newest one each program frame
//! ([`Feeds`], [`crate::StudioBackend::update_layer`]). A feed with no frame yet is transparent.
//!
//! Everything in a [`HostScene`] comes from another process: [`HostScene::load`] checks every
//! number and every file before anything is drawn, and a bad layer is an error, never a panic.

use crate::{Frame, PixelFormat, Result, StudioError, VideoSettings};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// Most layers in a scene (the project's limit, `cfa4_project::studio::MAX_SOURCES`).
pub const MAX_LAYERS: usize = 200;
/// Largest side of a layer picture.
pub const MAX_PICTURE_SIDE: u32 = 8192;
/// Most picture bytes one scene may load (1 GiB): eight 8K pictures.
pub const MAX_SCENE_BYTES: u64 = 1 << 30;
/// Largest absolute position, in canvas pixels.
pub const MAX_POSITION: f64 = 100_000.0;
/// Largest absolute scale factor.
pub const MAX_SCALE: f64 = 100.0;

/// The scene the host shows: layers bottom first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostScene {
    /// Increases with every scene the engine sends; the host reports the one it shows.
    pub revision: u64,
    /// The layers, bottom first.
    #[serde(default)]
    pub layers: Vec<Layer>,
}

/// One layer: a picture and where it goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    /// The project's source id (for logs and status).
    pub id: u64,
    pub picture: PictureRef,
    #[serde(default)]
    pub transform: Transform,
}

/// Where a layer's pixels come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PictureRef {
    /// A solid colour (sRGB-encoded straight RGBA).
    Color { rgba: [u8; 4], width: u32, height: u32 },
    /// A raw picture file: `width × height` straight RGBA8 pixels, rows packed, nothing else.
    Raw { path: String, width: u32, height: u32 },
    /// A moving picture: a frame ring of `width × height` RGBA8 frames the engine keeps writing.
    Feed { path: String, width: u32, height: u32 },
}

impl PictureRef {
    fn size(&self) -> (u32, u32) {
        match self {
            PictureRef::Color { width, height, .. } | PictureRef::Raw { width, height, .. } | PictureRef::Feed { width, height, .. } => (*width, *height),
        }
    }
}

/// OBS Studio's scene-item geometry: the picture, cut by `crop` (left, top, right, bottom
/// pixels), is scaled by `scale_x`/`scale_y` (negative flips), rotated by `rotation` degrees
/// clockwise on screen about its top-left corner, and that corner is put at `x`, `y`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
    #[serde(default)]
    pub crop: [u32; 4],
}

impl Default for Transform {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, scale_x: 1.0, scale_y: 1.0, rotation: 0.0, crop: [0; 4] }
    }
}

impl Transform {
    fn check(&self) -> Result<()> {
        let ok = |v: f64, max: f64| v.is_finite() && v.abs() <= max;
        if !(ok(self.x, MAX_POSITION) && ok(self.y, MAX_POSITION) && ok(self.scale_x, MAX_SCALE) && ok(self.scale_y, MAX_SCALE) && self.rotation.is_finite()) {
            return Err(StudioError::Invalid(format!("layer transform out of range: {self:?}")));
        }
        Ok(())
    }
}

/// A decoded layer picture: straight RGBA8, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// A layer ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedLayer {
    pub id: u64,
    /// The picture now: a still, or a feed's newest frame (transparent before its first).
    pub picture: Picture,
    pub transform: Transform,
    /// The frame ring of a moving picture ([`PictureRef::Feed`]).
    pub feed: Option<std::path::PathBuf>,
}

/// A scene whose pictures are loaded and checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadedScene {
    pub revision: u64,
    pub layers: Vec<LoadedLayer>,
}

fn check_side(what: &str, w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || w > MAX_PICTURE_SIDE || h > MAX_PICTURE_SIDE {
        return Err(StudioError::Invalid(format!("{what}: picture size {w}x{h} is outside 1..={MAX_PICTURE_SIDE}")));
    }
    Ok(())
}

/// Reads a raw RGBA picture file of exactly `width * height * 4` bytes.
pub fn read_raw(path: &Path, width: u32, height: u32) -> Result<Picture> {
    check_side(&path.display().to_string(), width, height)?;
    let want = u64::from(width) * u64::from(height) * 4;
    let meta = std::fs::metadata(path).map_err(|e| StudioError::Io(format!("layer picture {}", path.display()), e))?;
    if !meta.is_file() || meta.len() != want {
        return Err(StudioError::Invalid(format!("layer picture {}: {} bytes, expected {want} for {width}x{height} RGBA", path.display(), meta.len())));
    }
    let rgba = std::fs::read(path).map_err(|e| StudioError::Io(format!("layer picture {}", path.display()), e))?;
    if rgba.len() as u64 != want {
        return Err(StudioError::Invalid(format!("layer picture {} changed while it was read", path.display())));
    }
    Ok(Picture { width, height, rgba: Arc::new(rgba) })
}

/// Writes a raw RGBA picture file (owner-only) for [`read_raw`]. The engine side of the hand-over.
pub fn write_raw(path: &Path, picture: &Picture) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    check_side(&path.display().to_string(), picture.width, picture.height)?;
    if picture.rgba.len() as u64 != u64::from(picture.width) * u64::from(picture.height) * 4 {
        return Err(StudioError::Invalid("picture data does not match its size".into()));
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let io = |e| StudioError::Io(format!("writing {}", tmp.display()), e);
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(io)?;
    f.write_all(&picture.rgba).map_err(io)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| StudioError::Io(format!("writing {}", path.display()), e))
}

impl HostScene {
    /// Checks the scene and loads its pictures. Any bad layer makes the whole scene an error, so
    /// the host keeps showing the scene it had.
    pub fn load(&self) -> Result<LoadedScene> {
        if self.layers.len() > MAX_LAYERS {
            return Err(StudioError::Invalid(format!("{} layers; at most {MAX_LAYERS}", self.layers.len())));
        }
        // every number first, so nothing is read or allocated for a scene that will be refused
        let mut total = 0u64;
        for l in &self.layers {
            l.transform.check()?;
            let (w, h) = l.picture.size();
            check_side(&format!("layer {}", l.id), w, h)?;
            total = total.saturating_add(u64::from(w) * u64::from(h) * 4);
        }
        if total > MAX_SCENE_BYTES {
            return Err(StudioError::Invalid(format!("the scene's pictures exceed {MAX_SCENE_BYTES} bytes")));
        }
        let mut layers = Vec::with_capacity(self.layers.len());
        for l in &self.layers {
            let (picture, feed) = match &l.picture {
                PictureRef::Color { rgba, width, height } => (solid(*rgba, *width, *height), None),
                PictureRef::Raw { path, width, height } => (read_raw(Path::new(path), *width, *height)?, None),
                PictureRef::Feed { path, width, height } => {
                    let reader = crate::ring::RingReader::open(Path::new(path))?;
                    let newest = feed_picture(&reader, *width, *height)?;
                    (newest.map_or_else(|| solid([0; 4], *width, *height), |(_, p)| p), Some(std::path::PathBuf::from(path)))
                }
            };
            layers.push(LoadedLayer { id: l.id, picture, transform: l.transform, feed });
        }
        Ok(LoadedScene { revision: self.revision, layers })
    }
}

/// The newest frame of a feed as a picture, with its number; `None` before the first frame. A
/// frame of another size or format is an error (the feed is not the one the scene named).
fn feed_picture(reader: &crate::ring::RingReader, width: u32, height: u32) -> Result<Option<(u64, Picture)>> {
    let Some(f) = reader.latest()? else { return Ok(None) };
    let fr = &f.frame;
    if fr.format != PixelFormat::Rgba8 || fr.width != width || fr.height != height {
        return Err(StudioError::Invalid(format!("feed frame {}x{} {:?}, expected {width}x{height} rgba8", fr.width, fr.height, fr.format)));
    }
    let row = width as usize * 4;
    let rgba = if fr.stride as usize == row {
        f.frame.data
    } else {
        let mut out = Vec::with_capacity(row * height as usize);
        for r in 0..height as usize {
            let start = r * fr.stride as usize;
            out.extend_from_slice(fr.data.get(start..start + row).ok_or_else(|| StudioError::Ring("short feed frame".into()))?);
        }
        out
    };
    Ok(Some((f.seq, Picture { width, height, rgba: Arc::new(rgba) })))
}

/// The feeds of the shown scene: polled once per program frame for new pictures.
#[derive(Default)]
pub struct Feeds {
    /// (layer index, reader, size, last frame number shown)
    feeds: Vec<(usize, crate::ring::RingReader, (u32, u32), u64)>,
}

impl Feeds {
    /// Opens the feeds of `scene`'s layers. A feed that cannot be opened is skipped (its layer
    /// keeps the picture it was loaded with).
    pub fn of(scene: &LoadedScene) -> Self {
        let mut feeds = Vec::new();
        for (i, l) in scene.layers.iter().enumerate() {
            let Some(path) = &l.feed else { continue };
            match crate::ring::RingReader::open(path) {
                Ok(r) => {
                    let seq = r.latest_seq().unwrap_or(0);
                    feeds.push((i, r, (l.picture.width, l.picture.height), seq));
                }
                Err(e) => log::warn!("live studio: feed of layer {}: {e}", l.id),
            }
        }
        Self { feeds }
    }

    /// Whether the scene has feeds.
    pub fn is_empty(&self) -> bool {
        self.feeds.is_empty()
    }

    /// The layers whose feed has a frame not shown yet, with that frame. A broken frame is
    /// skipped (the layer keeps its picture).
    pub fn poll(&mut self) -> Vec<(usize, Picture)> {
        let mut out = Vec::new();
        for (index, reader, (w, h), shown) in &mut self.feeds {
            if reader.latest_seq().map_or(true, |seq| seq == *shown) {
                continue;
            }
            match feed_picture(reader, *w, *h) {
                Ok(Some((seq, pic))) => {
                    *shown = seq;
                    out.push((*index, pic));
                }
                Ok(None) => {}
                Err(e) => log::debug!("live studio: feed frame skipped: {e}"),
            }
        }
        out
    }
}

/// A `width × height` picture of one colour.
pub fn solid(rgba: [u8; 4], width: u32, height: u32) -> Picture {
    let n = width as usize * height as usize;
    let mut data = Vec::with_capacity(n * 4);
    for _ in 0..n {
        data.extend_from_slice(&rgba);
    }
    Picture { width, height, rgba: Arc::new(data) }
}

/// The affine map of a layer from its cropped picture (pixels) to the canvas, and its inverse.
#[derive(Clone, Copy, Debug)]
struct Placement {
    /// canvas = [a b; c d] · local + (tx, ty)
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
    /// Cropped size.
    w: f64,
    h: f64,
}

impl Placement {
    fn new(t: &Transform, pic_w: u32, pic_h: u32) -> Option<Self> {
        let w = f64::from(pic_w) - f64::from(t.crop[0]) - f64::from(t.crop[2]);
        let h = f64::from(pic_h) - f64::from(t.crop[1]) - f64::from(t.crop[3]);
        if w < 1.0 || h < 1.0 || t.scale_x == 0.0 || t.scale_y == 0.0 {
            return None;
        }
        let (s, c) = t.rotation.to_radians().sin_cos();
        Some(Self { a: c * t.scale_x, b: -s * t.scale_y, c: s * t.scale_x, d: c * t.scale_y, tx: t.x, ty: t.y, w, h })
    }

    fn forward(&self, u: f64, v: f64) -> (f64, f64) {
        (self.a * u + self.b * v + self.tx, self.c * u + self.d * v + self.ty)
    }

    /// Canvas bounds (x0, y0, x1, y1), whole pixels, clipped to the canvas.
    fn bounds(&self, cw: u32, ch: u32) -> Option<(usize, usize, usize, usize)> {
        let pts = [self.forward(0.0, 0.0), self.forward(self.w, 0.0), self.forward(0.0, self.h), self.forward(self.w, self.h)];
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let x0 = x0.floor().max(0.0);
        let y0 = y0.floor().max(0.0);
        let x1 = x1.ceil().min(f64::from(cw));
        let y1 = y1.ceil().min(f64::from(ch));
        (x1 > x0 && y1 > y0).then_some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }
}

/// Composites `scene` onto an opaque black canvas of `video`'s size, as BGRA8: each layer drawn
/// over the ones below (straight alpha, sRGB values, as libobs blends SDR scenes), sampled
/// bilinearly at pixel centres; a pixel belongs to a layer when its centre is inside it. The
/// arithmetic is 8-bit fixed point (1/256 sample weights, rounded blends) and rows are drawn in
/// parallel, so a full-HD scene with a playing video keeps up with 60 fps; a layer at a
/// whole-pixel position, scale 1 and no rotation takes its texels as they are.
pub fn compose(scene: &LoadedScene, video: &VideoSettings, pts_ns: u64) -> Frame {
    use rayon::prelude::*;
    let (cw, ch) = (video.width, video.height);
    let row = cw as usize * 4;
    let mut data = vec![0u8; row * ch as usize];
    data.par_chunks_mut(row).for_each(|r| {
        for px in r.chunks_exact_mut(4) {
            px[3] = 255;
        }
    });
    for layer in &scene.layers {
        draw_layer(&mut data, cw, ch, layer);
    }
    Frame { width: cw, height: ch, stride: video.stride(), format: PixelFormat::Bgra8, pts_ns, data }
}

/// Blends straight-alpha RGBA `s` over the BGRA canvas pixel `d` (rounded 8-bit arithmetic).
fn blend(d: &mut [u8], s: [u32; 4]) {
    let a = s[3];
    if a == 0 {
        return;
    }
    let mix = |src: u32, dst: u8| ((src * a + u32::from(dst) * (255 - a) + 127) / 255) as u8;
    let (r, g, b) = (mix(s[0], d[2]), mix(s[1], d[1]), mix(s[2], d[0]));
    d[0] = b;
    d[1] = g;
    d[2] = r;
    d[3] = (a + (u32::from(d[3]) * (255 - a) + 127) / 255).min(255) as u8;
}

fn draw_layer(canvas: &mut [u8], cw: u32, ch: u32, layer: &LoadedLayer) {
    use rayon::prelude::*;
    let pic = &layer.picture;
    let t = &layer.transform;
    let Some(pl) = Placement::new(t, pic.width, pic.height) else { return };
    let Some((x0, y0, x1, y1)) = pl.bounds(cw, ch) else { return };
    let det = pl.a * pl.d - pl.b * pl.c;
    if det.abs() < 1e-12 {
        return;
    }
    let (ia, ib, ic, id) = (pl.d / det, -pl.b / det, -pl.c / det, pl.a / det);
    let (cl, ct) = (f64::from(t.crop[0]), f64::from(t.crop[1]));
    let (pw, ph) = (pic.width as usize, pic.height as usize);
    // the cropped region's last pixel, for clamping the bilinear taps
    let (max_x, max_y) = (cl + pl.w - 1.0, ct + pl.h - 1.0);
    let src = &pic.rgba;
    let texel = |x: usize, y: usize| -> [u32; 4] {
        let i = (y * pw + x) * 4;
        match src.get(i..i + 4) {
            Some(p) => [u32::from(p[0]), u32::from(p[1]), u32::from(p[2]), u32::from(p[3])],
            None => [0; 4],
        }
    };
    // pixel centres land on texel centres: the taps are the texel itself
    let aligned = pl.a == 1.0 && pl.d == 1.0 && pl.b == 0.0 && pl.c == 0.0 && pl.tx.fract() == 0.0 && pl.ty.fract() == 0.0;
    let row = cw as usize * 4;
    canvas.par_chunks_mut(row).enumerate().skip(y0).take(y1 - y0).for_each(|(y, out)| {
        let cy = y as f64 + 0.5 - pl.ty;
        for x in x0..x1 {
            let cx = x as f64 + 0.5 - pl.tx;
            let u = ia * cx + ib * cy;
            let v = ic * cx + id * cy;
            if !(0.0..pl.w).contains(&u) || !(0.0..pl.h).contains(&v) {
                continue;
            }
            let Some(d) = out.get_mut(x * 4..x * 4 + 4) else { continue };
            if aligned {
                blend(d, texel((cl + u - 0.5) as usize, (ct + v - 0.5) as usize));
                continue;
            }
            // bilinear at the texel grid of the cropped picture (texel centres at +0.5)
            let sx = (cl + u - 0.5).clamp(cl, max_x);
            let sy = (ct + v - 0.5).clamp(ct, max_y);
            let (fx, fy) = (sx.floor(), sy.floor());
            let (wx, wy) = (((sx - fx) * 256.0) as u32, ((sy - fy) * 256.0) as u32);
            let (ix, iy) = (fx as usize, fy as usize);
            let (jx, jy) = ((ix + 1).min(max_x as usize).min(pw - 1), (iy + 1).min(max_y as usize).min(ph - 1));
            let (p00, p10, p01, p11) = (texel(ix, iy), texel(jx, iy), texel(ix, jy), texel(jx, jy));
            let mut s = [0u32; 4];
            for k in 0..4 {
                let top = p00[k] * (256 - wx) + p10[k] * wx;
                let bottom = p01[k] * (256 - wx) + p11[k] * wx;
                s[k] = (top * (256 - wy) + bottom * wy + 32768) >> 16;
            }
            blend(d, s);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(w: u32, h: u32) -> VideoSettings {
        VideoSettings { width: w, height: h, fps_num: 30, fps_den: 1 }
    }

    fn layer(id: u64, picture: Picture, transform: Transform) -> LoadedLayer {
        LoadedLayer { id, picture, transform, feed: None }
    }

    /// BGRA of the canvas pixel (x, y).
    fn at(f: &Frame, x: u32, y: u32) -> [u8; 4] {
        let i = (y * f.stride + x * 4) as usize;
        [f.data[i], f.data[i + 1], f.data[i + 2], f.data[i + 3]]
    }

    #[test]
    fn an_empty_scene_is_opaque_black() {
        let f = compose(&LoadedScene::default(), &video(32, 18), 7);
        assert_eq!((f.width, f.height, f.stride, f.pts_ns), (32, 18, 128, 7));
        assert!(f.data.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    }

    #[test]
    fn layers_are_placed_scaled_cropped_and_stacked() {
        let red = solid([255, 0, 0, 255], 10, 10);
        let blue = solid([0, 0, 255, 255], 4, 4);
        let scene = LoadedScene {
            revision: 1,
            layers: vec![
                layer(1, red, Transform { x: 5.0, y: 5.0, ..Default::default() }),
                layer(2, blue, Transform { x: 8.0, y: 8.0, scale_x: 2.0, scale_y: 2.0, crop: [1, 1, 1, 1], ..Default::default() }),
            ],
        };
        let f = compose(&scene, &video(32, 32), 0);
        assert_eq!(at(&f, 4, 4), [0, 0, 0, 255], "outside");
        assert_eq!(at(&f, 5, 5), [0, 0, 255, 255], "red's corner (BGRA)");
        assert_eq!(at(&f, 14, 14), [0, 0, 255, 255], "red's last pixel");
        assert_eq!(at(&f, 15, 15), [0, 0, 0, 255]);
        // blue: 2x2 after the crop, scaled 2x: 8..12
        assert_eq!(at(&f, 8, 8), [255, 0, 0, 255], "blue on top");
        assert_eq!(at(&f, 11, 11), [255, 0, 0, 255]);
        assert_eq!(at(&f, 12, 12), [0, 0, 255, 255], "red again past blue");
    }

    #[test]
    fn rotation_is_clockwise_about_the_top_left_corner() {
        // a 10x2 bar rotated 90° clockwise at (10, 5) runs down from there, to the left of x = 10
        let bar = solid([255, 255, 255, 255], 10, 2);
        let scene = LoadedScene { revision: 1, layers: vec![layer(1, bar, Transform { x: 10.0, y: 5.0, rotation: 90.0, ..Default::default() })] };
        let f = compose(&scene, &video(32, 32), 0);
        assert_eq!(at(&f, 9, 6), [255, 255, 255, 255]);
        assert_eq!(at(&f, 8, 14), [255, 255, 255, 255]);
        assert_eq!(at(&f, 10, 6), [0, 0, 0, 255], "right of the corner is empty");
        assert_eq!(at(&f, 9, 16), [0, 0, 0, 255], "the bar is 10 long");
        // a negative scale flips about the corner too
        let bar = solid([255, 255, 255, 255], 4, 1);
        let scene = LoadedScene { revision: 1, layers: vec![layer(1, bar, Transform { x: 10.0, y: 5.0, scale_x: -1.0, ..Default::default() })] };
        let f = compose(&scene, &video(32, 32), 0);
        assert_eq!(at(&f, 6, 5), [255, 255, 255, 255]);
        assert_eq!(at(&f, 10, 5), [0, 0, 0, 255]);
    }

    #[test]
    fn translucent_layers_blend_over_what_is_below() {
        let half = solid([255, 255, 255, 128], 4, 4);
        let scene = LoadedScene { revision: 1, layers: vec![layer(1, half, Transform::default())] };
        let f = compose(&scene, &video(16, 16), 0);
        assert_eq!(at(&f, 1, 1), [128, 128, 128, 255]);
        let clear = solid([255, 0, 0, 0], 4, 4);
        let f = compose(&LoadedScene { revision: 1, layers: vec![layer(1, clear, Transform::default())] }, &video(16, 16), 0);
        assert_eq!(at(&f, 1, 1), [0, 0, 0, 255], "fully transparent leaves black");
    }

    #[test]
    fn far_or_degenerate_layers_draw_nothing() {
        let p = solid([255, 255, 255, 255], 4, 4);
        for t in [
            Transform { x: -1000.0, ..Default::default() },
            Transform { scale_x: 0.0, ..Default::default() },
            Transform { crop: [2, 0, 2, 0], ..Default::default() },
            Transform { crop: [u32::MAX, 0, 0, 0], ..Default::default() },
        ] {
            let f = compose(&LoadedScene { revision: 1, layers: vec![layer(1, p.clone(), t)] }, &video(16, 16), 0);
            assert!(f.data.chunks_exact(4).all(|p| p == [0, 0, 0, 255]), "{t:?}");
        }
    }

    #[test]
    fn host_scenes_are_checked_and_loaded() {
        let dir = std::env::temp_dir().join(format!("cfa4-obs-scene-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("p.rgba");
        write_raw(&file, &solid([1, 2, 3, 4], 3, 2)).expect("write");
        let ok = HostScene {
            revision: 9,
            layers: vec![
                Layer { id: 1, picture: PictureRef::Color { rgba: [9, 9, 9, 255], width: 2, height: 2 }, transform: Transform::default() },
                Layer { id: 2, picture: PictureRef::Raw { path: file.to_string_lossy().into_owned(), width: 3, height: 2 }, transform: Transform::default() },
            ],
        };
        let loaded = ok.load().expect("load");
        assert_eq!(loaded.revision, 9);
        assert_eq!(loaded.layers[1].picture.rgba[..4], [1, 2, 3, 4]);
        let raw = |w: u32, h: u32| PictureRef::Raw { path: file.to_string_lossy().into_owned(), width: w, height: h };
        let bad = [
            Layer { id: 1, picture: raw(3, 3), transform: Transform::default() },
            Layer { id: 1, picture: raw(0, 2), transform: Transform::default() },
            Layer { id: 1, picture: PictureRef::Raw { path: "/nonexistent/x".into(), width: 1, height: 1 }, transform: Transform::default() },
            Layer { id: 1, picture: PictureRef::Raw { path: dir.to_string_lossy().into_owned(), width: 1, height: 1 }, transform: Transform::default() },
            Layer { id: 1, picture: PictureRef::Color { rgba: [0; 4], width: MAX_PICTURE_SIDE + 1, height: 1 }, transform: Transform::default() },
            Layer { id: 1, picture: PictureRef::Color { rgba: [0; 4], width: 1, height: 1 }, transform: Transform { x: f64::NAN, ..Default::default() } },
            Layer { id: 1, picture: PictureRef::Color { rgba: [0; 4], width: 1, height: 1 }, transform: Transform { scale_x: 1e9, ..Default::default() } },
        ];
        for l in bad {
            assert!(HostScene { revision: 1, layers: vec![l.clone()] }.load().is_err(), "{l:?}");
        }
        let too_many = HostScene { revision: 1, layers: vec![ok.layers[0].clone(); MAX_LAYERS + 1] };
        assert!(too_many.load().is_err());
        let too_big = HostScene {
            revision: 1,
            layers: vec![
                Layer {
                    id: 1,
                    picture: PictureRef::Color { rgba: [0; 4], width: MAX_PICTURE_SIDE, height: MAX_PICTURE_SIDE },
                    transform: Transform::default()
                };
                5
            ],
        };
        assert!(too_big.load().is_err(), "more than 1 GiB of pictures");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn feeds_show_their_newest_frame() {
        use crate::StudioBackend;
        let dir = std::env::temp_dir().join(format!("cfa4-obs-feed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let ring = crate::ring::create_sized(&dir, 17, 16).expect("ring");
        let mut w = crate::ring::RingWriter::open(&ring, 1).expect("writer");
        let frame = |v: u8| Frame { width: 17, height: 16, stride: 68, format: PixelFormat::Rgba8, pts_ns: 0, data: [v, 0, 0, 255].repeat(17 * 16) };
        let feed = |w: u32, h: u32| HostScene {
            revision: 1,
            layers: vec![Layer {
                id: 7,
                picture: PictureRef::Feed { path: ring.to_string_lossy().into_owned(), width: w, height: h },
                transform: Transform::default(),
            }],
        };
        // before the first frame: transparent
        let loaded = feed(17, 16).load().expect("load");
        assert!(loaded.layers[0].picture.rgba.iter().all(|&b| b == 0));
        assert_eq!(loaded.layers[0].feed.as_deref(), Some(ring.as_path()));
        w.write(&frame(10)).expect("write");
        let loaded = feed(17, 16).load().expect("load");
        assert_eq!(loaded.layers[0].picture.rgba[..4], [10, 0, 0, 255]);
        assert!(feed(18, 16).load().is_err(), "a feed of another size");
        assert!(
            HostScene {
                revision: 1,
                layers: vec![Layer {
                    id: 1,
                    picture: PictureRef::Feed { path: "/nonexistent".into(), width: 17, height: 16 },
                    transform: Transform::default()
                }]
            }
            .load()
            .is_err()
        );
        // the poller hands over each new frame once
        let mut feeds = Feeds::of(&loaded);
        assert!(!feeds.is_empty());
        assert!(feeds.poll().is_empty(), "the frame it was loaded with");
        w.write(&frame(200)).expect("write");
        let got = feeds.poll();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].0, got[0].1.rgba[..4].to_vec()), (0, vec![200, 0, 0, 255]));
        assert!(feeds.poll().is_empty());
        // the software backend recomposites with the new picture
        let video = VideoSettings { width: 32, height: 16, fps_num: 200, fps_den: 1 };
        let mut b = crate::software::SoftwareBackend::new(video).expect("backend");
        b.set_scene(&loaded).expect("scene");
        let f = b.next_frame(std::time::Duration::from_secs(1)).expect("ok").expect("frame");
        assert_eq!(at(&f, 1, 1), [0, 0, 10, 255], "BGRA of the loaded frame");
        b.update_layer(0, &got[0].1).expect("update");
        b.update_layer(5, &got[0].1).expect("an index the scene lacks is ignored");
        let f = b.next_frame(std::time::Duration::from_secs(1)).expect("ok").expect("frame");
        assert_eq!(at(&f, 1, 1), [0, 0, 200, 255]);
        assert_eq!(at(&f, 20, 1), [0, 0, 0, 255], "outside the 17-pixel layer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_wire_form_is_stable() {
        let s = HostScene {
            revision: 2,
            layers: vec![Layer { id: 5, picture: PictureRef::Color { rgba: [1, 2, 3, 4], width: 8, height: 6 }, transform: Transform::default() }],
        };
        let json = serde_json::to_string(&s).expect("json");
        assert_eq!(
            json,
            r#"{"revision":2,"layers":[{"id":5,"picture":{"type":"color","rgba":[1,2,3,4],"width":8,"height":6},"transform":{"x":0.0,"y":0.0,"scaleX":1.0,"scaleY":1.0,"rotation":0.0,"crop":[0,0,0,0]}}]}"#
        );
        assert_eq!(serde_json::from_str::<HostScene>(&json).ok(), Some(s));
    }
}
