// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-oxide 0.12.6.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use std::sync::Arc;

use crate::bitstream::{Bitstream, ContainerParser, ParseEvent};
use crate::common::{Bundle, Name};
use crate::frame::FrameContext;
use crate::image::BitDepth;
use crate::render::ImageBuffer;
use crate::render::ImageWithRegion;
use crate::render::Region;
use crate::render::{IndexedFrame, RenderContext};

pub use crate::color::{ColorEncodingWithProfile, ColorManagementSystem, NullCms, PreparedTransform};
pub use crate::frame::{Frame, FrameHeader};
pub use crate::grid::{AlignedGrid, AllocTracker};
pub use crate::image::color::{self, EnumColourEncoding, RenderingIntent};
pub use crate::image::{ExtraChannelType, ImageHeader};
pub use crate::threadpool::JxlThreadPool;

mod fb;

pub use fb::{FrameBuffer, FrameBufferSample, ImageStream};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

fn default_pool() -> JxlThreadPool {
    JxlThreadPool::none()
}

/// JPEG XL image decoder builder.
#[derive(Debug, Default)]
pub struct JxlImageBuilder {
    pool: Option<JxlThreadPool>,
    tracker: Option<AllocTracker>,
    force_wide_buffers: bool,
}

impl JxlImageBuilder {
    /// Sets a custom thread pool.
    pub fn pool(mut self, pool: JxlThreadPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Sets an allocation tracker.
    pub fn alloc_tracker(mut self, tracker: AllocTracker) -> Self {
        self.tracker = Some(tracker);
        self
    }

    /// Force 32-bit Modular buffers when decoding.
    pub fn force_wide_buffers(mut self, force_wide_buffers: bool) -> Self {
        self.force_wide_buffers = force_wide_buffers;
        self
    }

    /// Consumes the builder, and creates an empty, uninitialized JPEG XL image decoder.
    pub fn build_uninit(self) -> UninitializedJxlImage {
        UninitializedJxlImage {
            pool: self.pool.unwrap_or_else(default_pool),
            tracker: self.tracker,
            reader: ContainerParser::new(),
            buffer: Vec::new(),
            force_wide_buffers: self.force_wide_buffers,
        }
    }

    /// Consumes the builder, and creates a JPEG XL image decoder by reading image from the reader.
    pub fn read(self, mut reader: impl std::io::Read) -> Result<JxlImage> {
        let mut uninit = self.build_uninit();
        let mut buf = vec![0u8; 4096];
        let mut buf_valid = 0usize;
        let mut image = loop {
            let count = reader.read(&mut buf[buf_valid..])?;
            if count == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "reader ended before parsing image header").into());
            }
            buf_valid += count;
            let consumed = uninit.feed_bytes(&buf[..buf_valid])?;
            buf.copy_within(consumed..buf_valid, 0);
            buf_valid -= consumed;

            match uninit.try_init()? {
                InitializeResult::NeedMoreData(x) => {
                    uninit = x;
                }
                InitializeResult::Initialized(x) => {
                    break x;
                }
            }
        };

        while !image.inner.end_of_image {
            let count = reader.read(&mut buf[buf_valid..])?;
            if count == 0 {
                break;
            }
            buf_valid += count;
            let consumed = image.feed_bytes(&buf[..buf_valid])?;
            buf.copy_within(consumed..buf_valid, 0);
            buf_valid -= consumed;
        }

        buf.truncate(buf_valid);
        image.finalize()?;
        Ok(image)
    }

    /// Consumes the builder, and creates a JPEG XL image decoder by reading image from the file.
    pub fn open(self, path: impl AsRef<std::path::Path>) -> Result<JxlImage> {
        let file = std::fs::File::open(path)?;
        self.read(file)
    }
}

/// Empty, uninitialized JPEG XL image.
///
/// # Examples
/// ```no_run
/// # fn read_bytes() -> cfa4_jxl::Result<&'static [u8]> { Ok(&[]) }
/// # use cfa4_jxl::{JxlImage, InitializeResult};
/// # fn main() -> cfa4_jxl::Result<()> {
/// let mut uninit_image = JxlImage::builder().build_uninit();
/// let image = loop {
///     let buf = read_bytes()?;
///     uninit_image.feed_bytes(buf)?;
///     match uninit_image.try_init()? {
///         InitializeResult::NeedMoreData(uninit) => {
///             uninit_image = uninit;
///         }
///         InitializeResult::Initialized(image) => {
///             break image;
///         }
///     }
/// };
/// println!("{:?}", image.image_header());
/// # Ok(())
/// # }
/// ```
pub struct UninitializedJxlImage {
    pool: JxlThreadPool,
    tracker: Option<AllocTracker>,
    reader: ContainerParser,
    buffer: Vec<u8>,
    force_wide_buffers: bool,
}

impl UninitializedJxlImage {
    /// Feeds more data into the decoder.
    ///
    /// Returns total consumed bytes from the buffer.
    pub fn feed_bytes(&mut self, buf: &[u8]) -> Result<usize> {
        for event in self.reader.feed_bytes(buf) {
            match event? {
                ParseEvent::BitstreamKind(_) => {}
                ParseEvent::Codestream(buf) => {
                    self.buffer.extend_from_slice(buf);
                }
                // auxiliary boxes (Exif, XMP, JPEG reconstruction data) are not read
                _ => {}
            }
        }
        Ok(self.reader.previous_consumed_bytes())
    }

    /// Returns the internal reader.
    #[inline]
    pub fn reader(&self) -> &ContainerParser {
        &self.reader
    }

    /// Try to initialize an image with the data fed into so far.
    ///
    /// # Returns
    /// - `Ok(InitializeResult::Initialized(_))` if the initialization was successful,
    /// - `Ok(InitializeResult::NeedMoreData(_))` if the data was not enough, and
    /// - `Err(_)` if there was a decode error during the initialization, meaning invalid bitstream
    ///   was given.
    pub fn try_init(mut self) -> Result<InitializeResult> {
        let mut bitstream = Bitstream::new(&self.buffer);
        let image_header = match ImageHeader::parse(&mut bitstream, ()) {
            Ok(x) => x,
            Err(e) if e.unexpected_eof() => {
                return Ok(InitializeResult::NeedMoreData(self));
            }
            Err(e) => {
                return Err(e.into());
            }
        };

        let embedded_icc = if image_header.metadata.colour_encoding.want_icc() {
            let icc = match crate::color::icc::read_icc(&mut bitstream) {
                Ok(x) => x,
                Err(e) if e.unexpected_eof() => {
                    return Ok(InitializeResult::NeedMoreData(self));
                }
                Err(e) => {
                    return Err(e.into());
                }
            };
            crate::tracing::debug!("Image has an embedded ICC profile");
            let icc = crate::color::icc::decode_icc(&icc)?;
            Some(icc)
        } else {
            None
        };
        bitstream.zero_pad_to_byte()?;

        let image_header = Arc::new(image_header);
        let skip_bytes = if image_header.metadata.preview.is_some() {
            let frame = match Frame::parse(
                &mut bitstream,
                FrameContext { image_header: image_header.clone(), tracker: self.tracker.as_ref(), pool: self.pool.clone() },
            ) {
                Ok(x) => x,
                Err(e) if e.unexpected_eof() => {
                    return Ok(InitializeResult::NeedMoreData(self));
                }
                Err(e) => {
                    return Err(e.into());
                }
            };

            let bytes_read = bitstream.num_read_bits() / 8;
            let x = frame.toc().total_byte_size();
            if self.buffer.len() < bytes_read + x {
                return Ok(InitializeResult::NeedMoreData(self));
            }

            x
        } else {
            0usize
        };

        let bytes_read = bitstream.num_read_bits() / 8 + skip_bytes;
        self.buffer.drain(..bytes_read);

        let render_spot_color = !image_header.metadata.grayscale();

        let mut builder = RenderContext::builder().pool(self.pool.clone());
        if let Some(icc) = embedded_icc {
            builder = builder.embedded_icc(icc);
        }
        if let Some(tracker) = self.tracker {
            builder = builder.alloc_tracker(tracker);
        }
        builder = builder.force_wide_buffers(self.force_wide_buffers);
        let ctx = builder.build(image_header.clone())?;

        let mut image = JxlImage {
            pool: self.pool.clone(),
            reader: self.reader,
            image_header,
            ctx: Box::new(ctx),
            render_spot_color,
            inner: JxlImageInner { end_of_image: false, buffer: Vec::new(), buffer_offset: bytes_read, frame_offsets: Vec::new() },
        };
        image.inner.feed_bytes_inner(&mut image.ctx, &self.buffer)?;

        Ok(InitializeResult::Initialized(image))
    }
}

/// Initialization result from [`UninitializedJxlImage::try_init`].
pub enum InitializeResult {
    /// The data was not enough. Feed more data into the returned image.
    NeedMoreData(UninitializedJxlImage),
    /// The image is successfully initialized.
    Initialized(JxlImage),
}

/// JPEG XL image.
#[derive(Debug)]
pub struct JxlImage {
    pool: JxlThreadPool,
    reader: ContainerParser,
    image_header: Arc<ImageHeader>,
    ctx: Box<RenderContext>,
    render_spot_color: bool,
    inner: JxlImageInner,
}

/// # Constructors and data-feeding methods
impl JxlImage {
    /// Creates a decoder builder with default options.
    pub fn builder() -> JxlImageBuilder {
        JxlImageBuilder::default()
    }

    /// Reads an image from the reader with default options.
    pub fn read_with_defaults(reader: impl std::io::Read) -> Result<JxlImage> {
        Self::builder().read(reader)
    }

    /// Opens an image in the filesystem with default options.
    pub fn open_with_defaults(path: impl AsRef<std::path::Path>) -> Result<JxlImage> {
        Self::builder().open(path)
    }

    /// Feeds more data into the decoder.
    ///
    /// Returns total consumed bytes from the buffer.
    pub fn feed_bytes(&mut self, buf: &[u8]) -> Result<usize> {
        for event in self.reader.feed_bytes(buf) {
            match event? {
                ParseEvent::BitstreamKind(_) => {}
                ParseEvent::Codestream(buf) => {
                    self.inner.feed_bytes_inner(&mut self.ctx, buf)?;
                }
                _ => {}
            }
        }
        Ok(self.reader.previous_consumed_bytes())
    }

    /// Signals the end of bitstream.
    ///
    /// This is automatically done if `open()` or `read()` is used to decode the image.
    pub fn finalize(&mut self) -> Result<()> {
        Ok(())
    }
}

/// # Image and decoder metadata accessors
impl JxlImage {
    /// Returns the image header.
    #[inline]
    pub fn image_header(&self) -> &ImageHeader {
        &self.image_header
    }

    /// Returns the image width with orientation applied.
    #[inline]
    pub fn width(&self) -> u32 {
        self.image_header.width_with_orientation()
    }

    /// Returns the image height with orientation applied.
    #[inline]
    pub fn height(&self) -> u32 {
        self.image_header.height_with_orientation()
    }

    /// Returns the *original* ICC profile embedded in the image.
    #[inline]
    pub fn original_icc(&self) -> Option<&[u8]> {
        self.ctx.embedded_icc()
    }

    /// Returns the ICC profile that describes rendered images.
    ///
    /// The returned profile will change if different color encoding is specified using
    /// [`request_icc`][Self::request_icc] or
    /// [`request_color_encoding`][Self::request_color_encoding].
    pub fn rendered_icc(&self) -> Vec<u8> {
        let encoding = self.ctx.requested_color_encoding();
        match encoding.encoding() {
            color::ColourEncoding::Enum(encoding) => crate::color::icc::colour_encoding_to_icc(encoding),
            color::ColourEncoding::IccProfile(_) => encoding.icc_profile().to_vec(),
        }
    }

    /// Returns the CICP tag of the color encoding of rendered images, if there's any.
    #[inline]
    pub fn rendered_cicp(&self) -> Option<[u8; 4]> {
        let encoding = self.ctx.requested_color_encoding();
        encoding.encoding().cicp()
    }

    /// Returns the pixel format of the rendered image.
    pub fn pixel_format(&self) -> PixelFormat {
        let encoding = self.ctx.requested_color_encoding();
        let is_grayscale = encoding.is_grayscale();
        let has_black = encoding.is_cmyk();
        let mut has_alpha = false;
        for ec_info in &self.image_header.metadata.ec_info {
            if ec_info.is_alpha() {
                has_alpha = true;
            }
        }

        match (is_grayscale, has_black, has_alpha) {
            (false, false, false) => PixelFormat::Rgb,
            (false, false, true) => PixelFormat::Rgba,
            (false, true, false) => PixelFormat::Cmyk,
            (false, true, true) => PixelFormat::Cmyka,
            (true, _, false) => PixelFormat::Gray,
            (true, _, true) => PixelFormat::Graya,
        }
    }

    /// Returns what HDR transfer function the image uses, if there's any.
    ///
    /// Returns `None` if the image is not HDR one.
    pub fn hdr_type(&self) -> Option<HdrType> {
        self.ctx.suggested_hdr_tf().and_then(|tf| match tf {
            color::TransferFunction::Pq => Some(HdrType::Pq),
            color::TransferFunction::Hlg => Some(HdrType::Hlg),
            _ => None,
        })
    }

    /// Returns whether the spot color channels will be rendered.
    #[inline]
    pub fn render_spot_color(&self) -> bool {
        self.render_spot_color
    }

    /// Sets whether the spot colour channels will be rendered.
    #[inline]
    pub fn set_render_spot_color(&mut self, render_spot_color: bool) -> &mut Self {
        if render_spot_color && self.image_header.metadata.grayscale() {
            crate::tracing::warn!("Spot colour channels are not rendered on grayscale images");
            return self;
        }
        self.render_spot_color = render_spot_color;
        self
    }

    /// Returns the number of currently loaded keyframes.
    #[inline]
    pub fn num_loaded_keyframes(&self) -> usize {
        self.ctx.loaded_keyframes()
    }

    /// Returns the number of currently loaded frames, including frames that are not displayed
    /// directly.
    #[inline]
    pub fn num_loaded_frames(&self) -> usize {
        self.ctx.loaded_frames()
    }

    /// Returns whether the image is loaded completely, without missing animation keyframes or
    /// partially loaded frames.
    #[inline]
    pub fn is_loading_done(&self) -> bool {
        self.inner.end_of_image
    }

    /// Returns frame data by keyframe index.
    pub fn frame_by_keyframe(&self, keyframe_index: usize) -> Option<&IndexedFrame> {
        self.ctx.keyframe(keyframe_index)
    }

    /// Returns the frame header for the given keyframe index, or `None` if the keyframe does not
    /// exist.
    pub fn frame_header(&self, keyframe_index: usize) -> Option<&FrameHeader> {
        let frame = self.ctx.keyframe(keyframe_index)?;
        Some(frame.header())
    }

    /// Returns frame data by frame index, including frames that are not displayed directly.
    ///
    /// There are some situations where a frame is not displayed directly:
    /// - It may be marked as reference only, and meant to be only used by other frames.
    /// - It may contain LF image (which is 8x downsampled version) of another VarDCT frame.
    /// - Zero duration frame that is not the last frame of image is blended with following frames
    ///   and displayed together.
    pub fn frame(&self, frame_idx: usize) -> Option<&IndexedFrame> {
        self.ctx.frame(frame_idx)
    }

    /// Returns the offset of frame within codestream, in bytes.
    pub fn frame_offset(&self, frame_index: usize) -> Option<usize> {
        self.inner.frame_offsets.get(frame_index).copied()
    }

    /// Returns the thread pool used by the renderer.
    #[inline]
    pub fn pool(&self) -> &JxlThreadPool {
        &self.pool
    }

    /// Returns the internal reader.
    pub fn reader(&self) -> &ContainerParser {
        &self.reader
    }
}

/// # Color management methods
impl JxlImage {
    /// Sets color management system implementation to be used by the renderer.
    #[inline]
    pub fn set_cms(&mut self, cms: impl ColorManagementSystem + Send + Sync + 'static) {
        self.ctx.set_cms(cms);
    }

    /// Requests the decoder to render in specific color encoding, described by an ICC profile.
    ///
    /// # Errors
    /// This function will return an error if it cannot parse the ICC profile.
    pub fn request_icc(&mut self, icc_profile: &[u8]) -> Result<()> {
        self.ctx.request_color_encoding(ColorEncodingWithProfile::with_icc(icc_profile)?);
        Ok(())
    }

    /// Requests the decoder to render in specific color encoding, described by
    /// `EnumColourEncoding`.
    pub fn request_color_encoding(&mut self, color_encoding: EnumColourEncoding) {
        self.ctx.request_color_encoding(ColorEncodingWithProfile::new(color_encoding))
    }
}

/// # Rendering to image buffers
impl JxlImage {
    /// Renders the given keyframe.
    pub fn render_frame(&self, keyframe_index: usize) -> Result<Render> {
        self.render_frame_cropped(keyframe_index)
    }

    /// Renders the given keyframe with optional cropping region.
    pub fn render_frame_cropped(&self, keyframe_index: usize) -> Result<Render> {
        let image = self.ctx.render_keyframe(keyframe_index)?;

        let image_region = self.ctx.image_region().apply_orientation(&self.image_header);
        let frame = self.ctx.keyframe(keyframe_index).ok_or("keyframe out of range")?;
        let frame_header = frame.header();
        let target_frame_region = image_region.translate(-frame_header.x0, -frame_header.y0);

        let is_cmyk = self.ctx.requested_color_encoding().is_cmyk();
        let result = Render {
            keyframe_index,
            name: frame_header.name.clone(),
            duration: frame_header.duration,
            orientation: self.image_header.metadata.orientation,
            image,
            extra_channels: self.convert_ec_info(),
            target_frame_region,
            color_bit_depth: self.image_header.metadata.bit_depth,
            is_cmyk,
            render_spot_color: self.render_spot_color,
        };
        Ok(result)
    }

    /// Renders the currently loading keyframe.
    pub fn render_loading_frame(&mut self) -> Result<Render> {
        self.render_loading_frame_cropped()
    }

    /// Renders the currently loading keyframe with optional cropping region.
    pub fn render_loading_frame_cropped(&mut self) -> Result<Render> {
        let (frame, image) = self.ctx.render_loading_keyframe()?;
        let frame_header = frame.header();
        let name = frame_header.name.clone();
        let duration = frame_header.duration;

        let image_region = self.ctx.image_region().apply_orientation(&self.image_header);
        let frame = self.ctx.frame(self.ctx.loaded_frames()).or_else(|| self.ctx.frame(self.ctx.loaded_frames().checked_sub(1)?)).ok_or("no loaded frame")?;
        let frame_header = frame.header();
        let target_frame_region = image_region.translate(-frame_header.x0, -frame_header.y0);

        let is_cmyk = self.ctx.requested_color_encoding().is_cmyk();
        let result = Render {
            keyframe_index: self.ctx.loaded_keyframes(),
            name,
            duration,
            orientation: self.image_header.metadata.orientation,
            image,
            extra_channels: self.convert_ec_info(),
            target_frame_region,
            color_bit_depth: self.image_header.metadata.bit_depth,
            is_cmyk,
            render_spot_color: self.render_spot_color,
        };
        Ok(result)
    }

    /// Returns current cropping region (region of interest).
    pub fn current_image_region(&self) -> CropInfo {
        let region = self.ctx.image_region();
        region.into()
    }

    /// Sets the cropping region (region of interest).
    ///
    /// Subsequent rendering methods will crop the image buffer according to the region.
    pub fn set_image_region(&mut self, region: CropInfo) -> &mut Self {
        self.ctx.request_image_region(region.into());
        self
    }
}

/// # Private methods
impl JxlImage {
    fn convert_ec_info(&self) -> Vec<ExtraChannel> {
        self.image_header
            .metadata
            .ec_info
            .iter()
            .map(|ec_info| ExtraChannel { ty: ec_info.ty, name: ec_info.name.clone(), bit_depth: ec_info.bit_depth })
            .collect()
    }
}

#[derive(Debug)]
struct JxlImageInner {
    end_of_image: bool,
    buffer: Vec<u8>,
    buffer_offset: usize,
    frame_offsets: Vec<usize>,
}

impl JxlImageInner {
    fn feed_bytes_inner(&mut self, ctx: &mut RenderContext, mut buf: &[u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }

        if self.end_of_image {
            self.buffer.extend_from_slice(buf);
            return Ok(());
        }

        if let Some(loading_frame) = ctx.current_loading_frame() {
            debug_assert!(self.buffer.is_empty());
            let len = buf.len();
            buf = loading_frame.feed_bytes(buf)?;
            let count = len - buf.len();
            self.buffer_offset += count;

            if loading_frame.is_loading_done() {
                let is_last = loading_frame.header().is_last;
                ctx.finalize_current_frame();
                if is_last {
                    self.end_of_image = true;
                    self.buffer = buf.to_vec();
                    return Ok(());
                }
            }
            if buf.is_empty() {
                return Ok(());
            }
        }

        self.buffer.extend_from_slice(buf);
        let mut buf = &*self.buffer;
        while !buf.is_empty() {
            let mut bitstream = Bitstream::new(buf);
            let frame = match ctx.load_frame_header(&mut bitstream) {
                Ok(x) => x,
                Err(e) if e.unexpected_eof() => {
                    self.buffer = buf.to_vec();
                    return Ok(());
                }
                Err(e) => {
                    return Err(e.into());
                }
            };
            let frame_index = frame.index();
            assert_eq!(self.frame_offsets.len(), frame_index);
            self.frame_offsets.push(self.buffer_offset);

            let read_bytes = bitstream.num_read_bits() / 8;
            buf = &buf[read_bytes..];
            let len = buf.len();
            buf = frame.feed_bytes(buf)?;
            let read_bytes = read_bytes + (len - buf.len());
            self.buffer_offset += read_bytes;

            if frame.is_loading_done() {
                let is_last = frame.header().is_last;
                ctx.finalize_current_frame();
                if is_last {
                    self.end_of_image = true;
                    self.buffer = buf.to_vec();
                    return Ok(());
                }
            }
        }

        self.buffer.clear();
        Ok(())
    }
}

/// Pixel format of the rendered image.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum PixelFormat {
    /// Grayscale, single channel
    Gray,
    /// Grayscale with alpha, two channels
    Graya,
    /// RGB, three channels
    Rgb,
    /// RGB with alpha, four channels
    Rgba,
    /// CMYK, four channels
    Cmyk,
    /// CMYK with alpha, five channels
    Cmyka,
}

impl PixelFormat {
    /// Returns the number of channels of the image.
    #[inline]
    pub fn channels(self) -> usize {
        match self {
            PixelFormat::Gray => 1,
            PixelFormat::Graya => 2,
            PixelFormat::Rgb => 3,
            PixelFormat::Rgba => 4,
            PixelFormat::Cmyk => 4,
            PixelFormat::Cmyka => 5,
        }
    }

    /// Returns whether the image is grayscale.
    #[inline]
    pub fn is_grayscale(self) -> bool {
        matches!(self, Self::Gray | Self::Graya)
    }

    /// Returns whether the image has an alpha channel.
    #[inline]
    pub fn has_alpha(self) -> bool {
        matches!(self, PixelFormat::Graya | PixelFormat::Rgba | PixelFormat::Cmyka)
    }

    /// Returns whether the image has a black channel.
    #[inline]
    pub fn has_black(self) -> bool {
        matches!(self, PixelFormat::Cmyk | PixelFormat::Cmyka)
    }
}

/// HDR transfer function type, returned by [`JxlImage::hdr_type`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HdrType {
    /// Perceptual quantizer.
    Pq,
    /// Hybrid log-gamma.
    Hlg,
}

/// The result of loading the keyframe.
#[derive(Debug)]
pub enum LoadResult {
    /// The frame is loaded with the given keyframe index.
    Done(usize),
    /// More data is needed to fully load the frame.
    NeedMoreData,
    /// No more frames are present.
    NoMoreFrames,
}

/// The result of loading and rendering the keyframe.
#[derive(Debug)]
pub enum RenderResult {
    /// The frame is rendered.
    Done(Render),
    /// More data is needed to fully render the frame.
    NeedMoreData,
    /// No more frames are present.
    NoMoreFrames,
}

/// The result of rendering a keyframe.
#[derive(Debug)]
pub struct Render {
    keyframe_index: usize,
    name: Name,
    duration: u32,
    orientation: u32,
    image: Arc<ImageWithRegion>,
    extra_channels: Vec<ExtraChannel>,
    target_frame_region: Region,
    color_bit_depth: BitDepth,
    is_cmyk: bool,
    render_spot_color: bool,
}

impl Render {
    /// Returns the keyframe index.
    #[inline]
    pub fn keyframe_index(&self) -> usize {
        self.keyframe_index
    }

    /// Returns the name of the frame.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns how many ticks this frame is presented.
    #[inline]
    pub fn duration(&self) -> u32 {
        self.duration
    }

    /// Returns the orientation of the image.
    #[inline]
    pub fn orientation(&self) -> u32 {
        self.orientation
    }

    /// Creates a stream that writes to borrowed buffer.
    ///
    /// The stream will include black and alpha channels, if exist, in addition to color channels.
    /// Orientation is applied.
    pub fn stream(&self) -> ImageStream<'_> {
        ImageStream::from_render(self, false)
    }

    /// Creates a stream that writes to borrowed buffer.
    ///
    /// The stream will include black channels if exist, but not alpha channels. Orientation is
    /// applied.
    pub fn stream_no_alpha(&self) -> ImageStream<'_> {
        ImageStream::from_render(self, true)
    }

    /// Creates a buffer with interleaved channels, with orientation applied.
    ///
    /// All extra channels are included. Use [`stream`](Render::stream) if only color, black and
    /// alpha channels are needed.
    #[inline]
    pub fn image_all_channels(&self) -> FrameBuffer {
        let fb: Vec<_> = self.image.buffer().iter().collect();
        let mut bit_depth = vec![self.color_bit_depth; self.image.color_channels()];
        for ec in &self.extra_channels {
            bit_depth.push(ec.bit_depth);
        }
        let regions: Vec<_> = self.image.regions_and_shifts().iter().map(|(region, _)| *region).collect();

        FrameBuffer::from_grids(&fb, &bit_depth, &regions, self.target_frame_region, self.orientation)
    }

    /// Creates a separate buffer by channel, with orientation applied.
    ///
    /// All extra channels are included.
    pub fn image_planar(&self) -> Vec<FrameBuffer> {
        let grids = self.image.buffer();
        let bit_depth_it = std::iter::repeat_n(self.color_bit_depth, self.image.color_channels()).chain(self.extra_channels.iter().map(|ec| ec.bit_depth));
        let region_it = self.image.regions_and_shifts().iter().map(|(region, _)| *region);

        bit_depth_it
            .zip(region_it)
            .zip(grids)
            .map(|((bit_depth, region), x)| FrameBuffer::from_grids(&[x], &[bit_depth], &[region], self.target_frame_region, self.orientation))
            .collect()
    }

    /// Returns the color channels.
    ///
    /// Orientation is not applied.
    #[inline]
    pub fn color_channels(&self) -> &[ImageBuffer] {
        let color_channels = self.image.color_channels();
        &self.image.buffer()[..color_channels]
    }

    /// Returns the extra channels, potentially including alpha and black channels.
    ///
    /// Orientation is not applied.
    #[inline]
    pub fn extra_channels(&self) -> (&[ExtraChannel], &[ImageBuffer]) {
        let color_channels = self.image.color_channels();
        (&self.extra_channels, &self.image.buffer()[color_channels..])
    }
}

/// Extra channel of the image.
#[derive(Debug)]
pub struct ExtraChannel {
    ty: ExtraChannelType,
    name: Name,
    bit_depth: BitDepth,
}

impl ExtraChannel {
    /// Returns the type of the extra channel.
    #[inline]
    pub fn ty(&self) -> ExtraChannelType {
        self.ty
    }

    /// Returns the name of the channel.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns `true` if the channel is a black channel of CMYK image.
    #[inline]
    pub fn is_black(&self) -> bool {
        matches!(self.ty, ExtraChannelType::Black)
    }

    /// Returns `true` if the channel is an alpha channel.
    #[inline]
    pub fn is_alpha(&self) -> bool {
        matches!(self.ty, ExtraChannelType::Alpha { .. })
    }

    /// Returns `true` if the channel is a spot colour channel.
    #[inline]
    pub fn is_spot_colour(&self) -> bool {
        matches!(self.ty, ExtraChannelType::SpotColour { .. })
    }
}

/// Cropping region information.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct CropInfo {
    pub width: u32,
    pub height: u32,
    pub left: u32,
    pub top: u32,
}

impl From<CropInfo> for crate::render::Region {
    fn from(value: CropInfo) -> Self {
        Self { left: value.left as i32, top: value.top as i32, width: value.width, height: value.height }
    }
}

impl From<crate::render::Region> for CropInfo {
    fn from(value: crate::render::Region) -> Self {
        Self { left: value.left.max(0) as u32, top: value.top.max(0) as u32, width: value.width, height: value.height }
    }
}
