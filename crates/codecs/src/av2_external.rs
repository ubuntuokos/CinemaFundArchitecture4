//! Early AV2 decoding through an external helper process (ADR 0010, A track; plan X/6).
//!
//! Behind the `av2-external` cargo feature and the `CFA4_AV2_DECODER` environment variable
//! (the path of `cfa4-av2-decode`, built by `scripts/dev/av2-decoder.py`). Without either, AV2
//! stays `Unsupported` ([`crate::av2`]). This is AV2-LAB level: the router registration (plan
//! X/7) waits for G1.
//!
//! The helper (`packaging/av2-decode/cfa4-av2-decode.c`) wraps dav2d. It runs in its own
//! process, so a crash, hang or garbage output of the C decoder becomes a [`CodecError`] here,
//! never a crash of CFA4 (AGENTS.md § 0). Every reply is checked: message sizes are capped, a
//! frame header must describe exactly the bytes that follow, and each request has a time limit.
//!
//! # Frame protocol v1
//!
//! Integers are little-endian; every message is `tag[4] len[u32] payload[len]`.
//!
//! | Direction | Message | Payload |
//! |---|---|---|
//! | helper → | hello | `"CFA4AV2D" u32 version u32 n` + decoder version (`n` bytes), sent once, unframed |
//! | → helper | `DATA` | `i64 pts`, then one temporal unit (Annex B OBUs, temporal delimiter first) |
//! | → helper | `FLSH` | empty: hand over every held picture (end of stream) |
//! | → helper | `RSET` | empty: forget all state (before a seek) |
//! | helper → | `FRME` | 24-byte header (`u32 w, u32 h, u8 layout, u8 bit depth, u8 matrix, u8 transfer, u8 primaries, u8 full range, u8 colour present, u8 0, i64 pts`), then the planes |
//! | helper → | `ERRO` | UTF-8 message: the request failed to decode |
//! | helper → | `DONE` | empty: the reply to the request is complete |
//!
//! Layout is 0 = 4:0:0 (Y plane only), 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4. Planes are packed
//! without padding, one byte per sample at 8 bits, otherwise two bytes little-endian (the same
//! packing as the AVM oracle's Y4M planes, so the oracle hashes apply to them unchanged).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use cfa4_av2::obu::{ObuType, annexb_obus};
use cfa4_frame::{Chroma, PixelData, VideoFrame};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The environment variable naming the helper executable.
pub const DECODER_ENV: &str = "CFA4_AV2_DECODER";
/// Protocol version this adapter speaks.
pub const PROTOCOL_VERSION: u32 = 1;
/// Length of the `FRME` header before the planes.
pub const FRAME_HEADER_LEN: usize = 24;
/// Largest message accepted from the helper (an 8K 4:4:4 16-bit picture is about 200 MB).
const MAX_MESSAGE: u32 = 512 * 1024 * 1024;
/// Largest picture side accepted from the helper.
const MAX_SIDE: u32 = 16384;
/// Time the helper gets to start, and to answer one request.
const START_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Replies in flight between the reader thread and the decoder (bounded: back-pressure).
const CHANNEL_DEPTH: usize = 4;

/// The helper configured in [`DECODER_ENV`], if any.
pub fn configured_helper() -> Option<PathBuf> {
    std::env::var_os(DECODER_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// A decoded picture as the helper sends it, before conversion to a [`VideoFrame`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPicture {
    pub width: u32,
    pub height: u32,
    /// 0 = 4:0:0, 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4.
    pub layout: u8,
    pub bit_depth: u8,
    pub matrix: u8,
    pub transfer: u8,
    pub primaries: u8,
    pub full_range: bool,
    pub colour_present: bool,
    pub pts: i64,
    /// Y, U, V (Y only for 4:0:0), packed as described in the module docs.
    pub planes: Vec<Vec<u8>>,
}

impl RawPicture {
    /// Parse and check a `FRME` payload: the header must describe exactly the bytes that follow.
    pub fn parse(payload: &[u8]) -> Result<RawPicture> {
        let bad = |m: &str| CodecError::Decode(format!("AV2 helper: {m}"));
        let header = payload.get(..FRAME_HEADER_LEN).ok_or_else(|| bad("frame header truncated"))?;
        let u32_at = |o: usize| header.get(o..o + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0);
        let (width, height) = (u32_at(0), u32_at(4));
        let byte = |o: usize| header.get(o).copied().unwrap_or(0);
        let (layout, bit_depth) = (byte(8), byte(9));
        let pts = header.get(16..24).and_then(|b| b.try_into().ok()).map(i64::from_le_bytes).unwrap_or(0);
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(bad(&format!("picture size {width}x{height} out of range")));
        }
        if !matches!(bit_depth, 8 | 10 | 12) {
            return Err(bad(&format!("bit depth {bit_depth}")));
        }
        let (ss_x, ss_y, planes) = match layout {
            0 => (0, 0, 1),
            1 => (1, 1, 3),
            2 => (1, 0, 3),
            3 => (0, 0, 3),
            l => return Err(bad(&format!("pixel layout {l}"))),
        };
        let bps = if bit_depth > 8 { 2usize } else { 1 };
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = ((w + ss_x) >> ss_x, (h + ss_y) >> ss_y);
        let sizes: Vec<usize> = (0..planes).map(|p| if p == 0 { w * h * bps } else { cw * ch * bps }).collect();
        let body = payload.get(FRAME_HEADER_LEN..).unwrap_or(&[]);
        if body.len() != sizes.iter().sum::<usize>() {
            return Err(bad(&format!("frame of {} bytes for a {width}x{height} layout {layout} {bit_depth}-bit picture", body.len())));
        }
        let mut out = Vec::with_capacity(planes);
        let mut pos = 0usize;
        for size in sizes {
            out.push(body.get(pos..pos + size).ok_or_else(|| bad("plane out of range"))?.to_vec());
            pos += size;
        }
        Ok(RawPicture {
            width,
            height,
            layout,
            bit_depth,
            matrix: byte(10),
            transfer: byte(11),
            primaries: byte(12),
            full_range: byte(13) != 0,
            colour_present: byte(14) != 0,
            pts,
            planes: out,
        })
    }

    /// Convert to the engine's frame type (4:0:0 gets neutral chroma planes, like the AV1 path).
    pub fn into_decoded(self) -> DecodedFrame {
        let (w, h) = (self.width as usize, self.height as usize);
        let color = if self.colour_present {
            crate::hw_frame::av1_color(self.width, self.height, self.matrix, self.transfer, self.primaries, self.full_range)
        } else {
            crate::hw_frame::av1_color(self.width, self.height, 2, 2, 2, false)
        };
        let (chroma, cw, ch) = match self.layout {
            2 => (Chroma::C422, w.div_ceil(2), h),
            3 => (Chroma::C444, w, h),
            _ => (Chroma::C420, w.div_ceil(2), h.div_ceil(2)),
        };
        let mut planes = self.planes.into_iter();
        let y = planes.next().unwrap_or_default();
        let (u, v) = (planes.next(), planes.next());
        let data = if self.bit_depth == 8 {
            let neutral = || vec![128u8; cw * ch];
            PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u.unwrap_or_else(neutral)), Arc::new(v.unwrap_or_else(neutral))], chroma, alpha: None }
        } else {
            let to16 = |b: Vec<u8>| b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<u16>>();
            let neutral = vec![1u16 << (self.bit_depth - 1); cw * ch];
            PixelData::Yuv16 {
                planes: [Arc::new(to16(y)), Arc::new(u.map(to16).unwrap_or_else(|| neutral.clone())), Arc::new(v.map(to16).unwrap_or(neutral))],
                chroma,
                bits: u32::from(self.bit_depth),
                alpha: None,
            }
        };
        DecodedFrame {
            pts: self.pts,
            frame: VideoFrame { width: self.width, height: self.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO },
            draft: false,
        }
    }
}

/// One message from the helper.
#[derive(Debug)]
enum Reply {
    Frame(Vec<u8>),
    Error(String),
    Done,
}

/// Read one tagged message (`None` at a clean end of stream).
fn read_message(r: &mut impl Read) -> std::result::Result<Option<Reply>, String> {
    let mut head = [0u8; 8];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    if len > MAX_MESSAGE {
        return Err(format!("message of {len} bytes refused"));
    }
    let mut payload = Vec::new();
    r.take(u64::from(len)).read_to_end(&mut payload).map_err(|e| e.to_string())?;
    if payload.len() != len as usize {
        return Err("message truncated".into());
    }
    match &head[..4] {
        b"FRME" => Ok(Some(Reply::Frame(payload))),
        b"ERRO" => Ok(Some(Reply::Error(String::from_utf8_lossy(&payload).into_owned()))),
        b"DONE" if payload.is_empty() => Ok(Some(Reply::Done)),
        t => Err(format!("unknown message {:?}", String::from_utf8_lossy(t))),
    }
}

/// Read and check the hello: returns the decoder version string.
fn read_hello(r: &mut impl Read) -> std::result::Result<String, String> {
    let mut hello = [0u8; 16];
    r.read_exact(&mut hello).map_err(|e| format!("no hello: {e}"))?;
    if &hello[..8] != b"CFA4AV2D" {
        return Err("not a CFA4 AV2 decoder helper".into());
    }
    let version = u32::from_le_bytes([hello[8], hello[9], hello[10], hello[11]]);
    if version != PROTOCOL_VERSION {
        return Err(format!("protocol version {version}, expected {PROTOCOL_VERSION}"));
    }
    let n = u32::from_le_bytes([hello[12], hello[13], hello[14], hello[15]]).min(256);
    let mut v = Vec::new();
    r.take(u64::from(n)).read_to_end(&mut v).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&v).into_owned())
}

/// A running helper process.
struct Helper {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<std::result::Result<Reply, String>>,
    version: String,
    timeout: Duration,
}

impl Helper {
    fn spawn(path: &Path, threads: usize) -> Result<Helper> {
        let err = |m: String| CodecError::Unsupported(format!("AV2 decoder helper {}: {m}", path.display()));
        let mut child = Command::new(path)
            .arg("--threads")
            .arg(threads.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| err(e.to_string()))?;
        let (Some(stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err("no pipes".into()));
        };
        let (hello_tx, hello_rx) = sync_channel(1);
        let (tx, replies) = sync_channel(CHANNEL_DEPTH);
        let reader = std::thread::Builder::new().name("cfa4-av2-helper".into()).spawn(move || {
            let hello = read_hello(&mut stdout);
            let ok = hello.is_ok();
            if hello_tx.send(hello).is_err() || !ok {
                return;
            }
            loop {
                let msg = match read_message(&mut stdout) {
                    Ok(Some(m)) => Ok(m),
                    Ok(None) => Err("the helper exited".to_string()),
                    Err(e) => Err(e),
                };
                let stop = msg.is_err();
                if tx.send(msg).is_err() || stop {
                    return;
                }
            }
        });
        if let Err(e) = reader {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err(e.to_string()));
        }
        match hello_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(version)) => Ok(Helper { child, stdin, replies, version, timeout: REQUEST_TIMEOUT }),
            Ok(Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(err(e))
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(err("did not start in time".into()))
            }
        }
    }

    /// Send one request and collect its reply.
    fn request(&mut self, tag: &[u8; 4], payload: &[&[u8]]) -> std::result::Result<(Vec<RawPicture>, Option<String>), String> {
        let len: usize = payload.iter().map(|p| p.len()).sum();
        let len = u32::try_from(len).map_err(|_| "request too large".to_string())?;
        let mut head = [0u8; 8];
        head[..4].copy_from_slice(tag);
        head[4..].copy_from_slice(&len.to_le_bytes());
        let send = |stdin: &mut ChildStdin| -> std::io::Result<()> {
            stdin.write_all(&head)?;
            for p in payload {
                stdin.write_all(p)?;
            }
            stdin.flush()
        };
        send(&mut self.stdin).map_err(|e| format!("the helper is gone ({e})"))?;
        let mut pictures = Vec::new();
        let mut error = None;
        loop {
            match self.replies.recv_timeout(self.timeout) {
                Ok(Ok(Reply::Frame(f))) => pictures.push(RawPicture::parse(&f).map_err(|e| e.to_string())?),
                Ok(Ok(Reply::Error(e))) => error = Some(e),
                Ok(Ok(Reply::Done)) => return Ok((pictures, error)),
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Timeout) => return Err("the helper did not answer in time".into()),
                Err(RecvTimeoutError::Disconnected) => return Err("the helper exited".into()),
            }
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // Closing stdin ends the helper; kill it in case it hangs. The reader thread ends with
        // the pipe.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// AV2 decoding through `cfa4-av2-decode`. Samples are temporal units (Annex B OBUs).
pub struct ExternalAv2Decoder {
    path: PathBuf,
    threads: usize,
    helper: Option<Helper>,
    name: String,
    timeout: Duration,
}

impl ExternalAv2Decoder {
    /// Start the helper at `path` (`threads` 0 = the decoder's choice).
    pub fn new(path: &Path, threads: usize) -> Result<ExternalAv2Decoder> {
        let helper = Helper::spawn(path, threads)?;
        let name = format!("dav2d {} (external)", helper.version);
        Ok(ExternalAv2Decoder { path: path.to_path_buf(), threads, helper: Some(helper), name, timeout: REQUEST_TIMEOUT })
    }

    /// Time the helper gets to answer one request (default one minute).
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
        if let Some(h) = self.helper.as_mut() {
            h.timeout = timeout;
        }
    }

    /// The helper configured in [`DECODER_ENV`].
    pub fn from_env() -> Result<ExternalAv2Decoder> {
        let path = configured_helper().ok_or_else(|| CodecError::Unsupported(format!("AV2: {DECODER_ENV} is not set")))?;
        Self::new(&path, 0)
    }

    fn call(&mut self, tag: &[u8; 4], payload: &[&[u8]]) -> Result<Vec<DecodedFrame>> {
        if self.helper.is_none() {
            // restarted after a failure (the stream continues from the next random access point)
            let mut helper = Helper::spawn(&self.path, self.threads)?;
            helper.timeout = self.timeout;
            self.helper = Some(helper);
        }
        let Some(helper) = self.helper.as_mut() else {
            return Err(CodecError::Decode("AV2 helper not running".into()));
        };
        match helper.request(tag, payload) {
            Ok((pictures, None)) => Ok(pictures.into_iter().map(RawPicture::into_decoded).collect()),
            Ok((_, Some(e))) => Err(CodecError::Decode(format!("AV2: {e}"))),
            Err(e) => {
                self.helper = None;
                Err(CodecError::Decode(format!("AV2 decoder helper failed: {e}")))
            }
        }
    }
}

impl VideoDecoder for ExternalAv2Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if sample.is_empty() {
            return Ok(Vec::new());
        }
        self.call(b"DATA", &[&pts.to_le_bytes(), sample])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.call(b"FLSH", &[]).unwrap_or_default()
    }
    fn reset(&mut self) {
        if self.call(b"RSET", &[]).is_err() {
            self.helper = None;
        }
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(is_closed_loop_key_unit(sample))
    }
}

/// Whether a temporal unit holds a closed-loop key frame (decoding can start there).
pub fn is_closed_loop_key_unit(tu: &[u8]) -> bool {
    annexb_obus(tu).map_while(|o| o.ok()).any(|o| o.header.obu_type == ObuType::ClosedLoopKey)
}

/// Split an Annex B stream into temporal units (each starts at a temporal delimiter):
/// `(offset, length)` pairs. Stops at the first malformed OBU.
pub fn temporal_units(data: &[u8]) -> Vec<(usize, usize)> {
    let mut units: Vec<(usize, usize)> = Vec::new();
    let mut end = 0usize;
    for obu in annexb_obus(data) {
        let Ok(obu) = obu else { break };
        let obu_end = obu.payload.as_ptr() as usize - data.as_ptr() as usize + obu.payload.len();
        if obu.header.obu_type == ObuType::TemporalDelimiter || units.is_empty() {
            if let Some(last) = units.last_mut() {
                last.1 = obu.offset - last.0;
            }
            units.push((obu.offset, 0));
        }
        end = obu_end;
    }
    if let Some(last) = units.last_mut() {
        last.1 = end - last.0;
    }
    units
}

/// One temporal unit of an Annex B file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Unit {
    offset: u64,
    len: usize,
    key: bool,
}

/// Longest temporal unit read from a file (the helper refuses more than 256 MiB too).
const MAX_UNIT: usize = 256 * 1024 * 1024;

/// Index the temporal units of an Annex B file by reading only the OBU headers.
fn index_units(reader: &dyn cfa4_media::reader::ByteReader) -> Vec<Unit> {
    let len = reader.len();
    let mut units: Vec<Unit> = Vec::new();
    let mut pos = 0u64;
    let mut head = [0u8; 10]; // the longest leb128 (8) and OBU header (2)
    while pos < len {
        let n = usize::try_from((len - pos).min(head.len() as u64)).unwrap_or(0);
        let Some(buf) = head.get_mut(..n) else { break };
        if reader.read_at(pos, buf).is_err() {
            break;
        }
        let Ok((size, leb)) = cfa4_av2::obu::leb128(buf) else { break };
        let Ok((header, _)) = cfa4_av2::obu::ObuHeader::parse(buf.get(leb..).unwrap_or(&[])) else { break };
        let obu_len = leb as u64 + u64::from(size);
        if size == 0 || pos + obu_len > len {
            break;
        }
        let starts_unit = header.obu_type == ObuType::TemporalDelimiter || units.is_empty();
        if starts_unit {
            units.push(Unit { offset: pos, len: 0, key: false });
        }
        let Some(u) = units.last_mut() else { break };
        u.len = usize::try_from(pos + obu_len - u.offset).unwrap_or(usize::MAX);
        u.key |= header.obu_type == ObuType::ClosedLoopKey;
        if u.len > MAX_UNIT {
            units.pop();
            break;
        }
        pos += obu_len;
    }
    units
}

/// An AV2 Annex B file (`.obu`) decoded through the helper. Frame rate: Annex B carries no timing,
/// so 24 fps like the other raw elementary streams.
pub struct Av2AnnexBSource {
    info: cfa4_media::MediaInfo,
    bytes: crate::Src,
    units: Vec<Unit>,
    helper: PathBuf,
    rate: cfa4_time::FrameRate,
    video: crate::gop::GopCache,
}

impl Av2AnnexBSource {
    pub fn open(name: &str, reader: cfa4_media::SharedReader, helper: PathBuf) -> Result<Self> {
        use cfa4_media::{MediaInfo, MediaKind, VideoStreamInfo};
        let units = index_units(&*reader);
        let first = units.first().ok_or_else(|| CodecError::Container("AV2: no temporal units".into()))?;
        let bytes = cfa4_media::reader::read_range(&*reader, first.offset, first.len).map_err(|e| CodecError::Container(e.to_string()))?;
        let stream = cfa4_av2::probe_annexb(&bytes).map_err(|e| CodecError::Unsupported(format!("AV2: {e}")))?;
        let sh = stream.sequence_header;
        let (width, height) = sh.cropped_size().ok_or_else(|| CodecError::Container("AV2: cropping window larger than the frame".into()))?;
        let chroma = match sh.chroma_format {
            cfa4_av2::seq::ChromaFormat::Monochrome => "Monochrome",
            cfa4_av2::seq::ChromaFormat::Yuv420 => "YUV 4:2:0",
            cfa4_av2::seq::ChromaFormat::Yuv422 => "YUV 4:2:2",
            cfa4_av2::seq::ChromaFormat::Yuv444 => "YUV 4:4:4",
        };
        let rate = cfa4_time::FrameRate::FPS_24;
        let duration = cfa4_time::Tick::from_rational(units.len() as i64 * rate.den, 1, rate.num);
        let color = crate::hw_frame::av1_color(width, height, 2, 2, 2, false);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width,
                height,
                frame_rate: rate,
                par: (1, 1),
                codec: format!("AV2 {} (external decoder)", sh.profile.label()),
                pixel_format: format!("{chroma} {}-bit", sh.bit_depth),
                color,
                has_alpha: false,
                bitrate: None,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "AV2 Annex B".into(),
            start_timecode: None,
            file_size: Some(reader.len()),
        };
        Ok(Av2AnnexBSource { info, bytes: crate::Src(reader), units, helper, rate, video: crate::gop::GopCache::new(None) })
    }
}

struct Av2Video<'a>(&'a Av2AnnexBSource);

impl crate::gop::VideoSamples for Av2Video<'_> {
    fn count(&self) -> usize {
        self.0.units.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        let i = i.min(self.0.units.len().saturating_sub(1));
        self.0.units.get(..=i).and_then(|u| u.iter().rposition(|u| u.key)).unwrap_or(0)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.units.len().checked_sub(1)?;
        Some(usize::try_from(t.max(0)).unwrap_or(usize::MAX).min(last))
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let u = self.0.units.get(i).ok_or_else(|| CodecError::Container("sample index out of range".into()))?;
        cfa4_media::reader::read_range(&*self.0.bytes.0, u.offset, u.len).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(ExternalAv2Decoder::new(&self.0.helper, 0)?))
    }
}

impl cfa4_media::MediaSource for Av2AnnexBSource {
    fn info(&self) -> &cfa4_media::MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: cfa4_media::FrameRequest) -> std::result::Result<Arc<VideoFrame>, cfa4_media::MediaError> {
        let frame_idx = self.rate.frame_at(req.time.max(cfa4_time::Tick::ZERO));
        Ok(self.video.frame(&Av2Video(self), frame_idx)?)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<cfa4_frame::AudioBuffer, cfa4_media::MediaError> {
        Err(cfa4_media::MediaError::NoStream("audio"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, layout: u8, bits: u8, body: usize) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&w.to_le_bytes());
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(&[layout, bits, 1, 1, 1, 0, 1, 0]);
        f.extend_from_slice(&7i64.to_le_bytes());
        f.extend(std::iter::repeat_n(0x40u8, body));
        f
    }

    #[test]
    fn frames_are_checked_against_their_header() {
        // 4x2 4:2:0 8 bit: 8 + 2 + 2
        let p = RawPicture::parse(&frame(4, 2, 1, 8, 12)).unwrap();
        assert_eq!((p.planes.len(), p.planes[0].len(), p.planes[1].len(), p.pts), (3, 8, 2, 7));
        // odd size 5x3 4:2:0 10 bit: (15 + 2*3*2) * 2
        assert!(RawPicture::parse(&frame(5, 3, 1, 10, (15 + 12) * 2)).is_ok());
        // monochrome: luma only
        assert_eq!(RawPicture::parse(&frame(4, 2, 0, 8, 8)).unwrap().planes.len(), 1);
        for bad in
            [frame(4, 2, 1, 8, 11), frame(4, 2, 1, 8, 13), frame(0, 2, 1, 8, 0), frame(4, 2, 4, 8, 12), frame(4, 2, 1, 9, 12), frame(MAX_SIDE + 1, 1, 0, 8, 0)]
        {
            assert!(RawPicture::parse(&bad).is_err());
        }
        assert!(RawPicture::parse(&[0; 10]).is_err());
    }

    #[test]
    fn conversion_fills_monochrome_chroma() {
        let d = RawPicture::parse(&frame(4, 2, 0, 10, 16)).unwrap().into_decoded();
        match d.frame.data {
            PixelData::Yuv16 { planes, chroma, bits, .. } => {
                assert_eq!((chroma, bits, planes[1].len(), planes[1][0]), (Chroma::C420, 10, 2, 512));
                assert_eq!(planes[0][0], 0x4040);
            }
            _ => panic!("expected 16-bit planes"),
        }
    }

    #[test]
    fn messages_are_bounded() {
        let mut m = b"DONE".to_vec();
        m.extend_from_slice(&0u32.to_le_bytes());
        assert!(matches!(read_message(&mut m.as_slice()), Ok(Some(Reply::Done))));
        let mut big = b"FRME".to_vec();
        big.extend_from_slice(&(MAX_MESSAGE + 1).to_le_bytes());
        assert!(read_message(&mut big.as_slice()).is_err());
        let mut short = b"FRME".to_vec();
        short.extend_from_slice(&10u32.to_le_bytes());
        short.extend_from_slice(&[0; 4]);
        assert!(read_message(&mut short.as_slice()).is_err());
        let mut unknown = b"XXXX".to_vec();
        unknown.extend_from_slice(&0u32.to_le_bytes());
        assert!(read_message(&mut unknown.as_slice()).is_err());
        assert!(matches!(read_message(&mut [].as_slice()), Ok(None)));
        assert!(read_hello(&mut b"CFA4AV2D\x02\0\0\0\0\0\0\0".as_slice()).is_err());
        assert_eq!(read_hello(&mut b"CFA4AV2D\x01\0\0\0\x02\0\0\0ok".as_slice()).unwrap(), "ok");
    }

    #[test]
    fn temporal_units_start_at_delimiters() {
        // TD, seq header (2 bytes), tile group | TD, tile group
        let s = [0x01, 0x08, 0x03, 0x04, 0xaa, 0xbb, 0x02, 0x1c, 0x01, 0x01, 0x08, 0x02, 0x10, 0x00];
        assert_eq!(temporal_units(&s), vec![(0, 9), (9, 5)]);
        // closed-loop key (type 4) in the second unit
        assert!(!is_closed_loop_key_unit(&s[..9]));
        assert!(is_closed_loop_key_unit(&s[9..]));
        assert!(temporal_units(&[]).is_empty());
        // a broken tail ends the last unit at the last good OBU
        assert_eq!(temporal_units(&s[..13]), vec![(0, 9), (9, 2)]);
    }

    #[test]
    fn a_missing_or_wrong_helper_is_an_error() {
        assert!(ExternalAv2Decoder::new(Path::new("/nonexistent/cfa4-av2-decode"), 0).is_err());
        // a program that is not the helper: no hello
        assert!(ExternalAv2Decoder::new(Path::new("/bin/true"), 0).is_err());
    }
}
