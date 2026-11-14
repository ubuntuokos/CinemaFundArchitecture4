//! Out-of-process video decoders (ADR 0010 for AV2, ADR 0013 for H.266/VVC): a helper program
//! wraps a C or C++ decoder library and speaks the CFA4 frame protocol on stdin/stdout. A crash,
//! hang or garbage output of the decoder becomes a [`CodecError`] here, never a crash of CFA4
//! (AGENTS.md § 0). Every reply is checked: message sizes are capped, a frame header must
//! describe exactly the bytes that follow, and each request has a time limit.
//!
//! The codec modules ([`crate::av2_external`], [`crate::vvc_external`]) add what is codec
//! specific: the helper's hello tag, how samples are split and which samples are random access
//! points.
//!
//! # Frame protocol v1
//!
//! Integers are little-endian; every message is `tag[4] len[u32] payload[len]`.
//!
//! | Direction | Message | Payload |
//! |---|---|---|
//! | helper → | hello | 8-byte tag (`"CFA4AV2D"`, `"CFA4VVCD"`) `u32 version u32 n` + decoder version (`n` bytes), sent once, unframed |
//! | → helper | `DATA` | `i64 pts`, then one sample (an AV2 temporal unit, a VVC access unit; Annex B) |
//! | → helper | `FLSH` | empty: hand over every held picture (end of stream) |
//! | → helper | `RSET` | empty: forget all state (before a seek) |
//! | helper → | `FRME` | 24-byte header (`u32 w, u32 h, u8 layout, u8 bit depth, u8 matrix, u8 transfer, u8 primaries, u8 full range, u8 colour present, u8 0, i64 pts`), then the planes |
//! | helper → | `ERRO` | UTF-8 message: the request failed to decode |
//! | helper → | `DONE` | empty: the reply to the request is complete |
//!
//! Layout is 0 = 4:0:0 (Y plane only), 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4. Planes are packed
//! without padding, one byte per sample at 8 bits, otherwise two bytes little-endian (the same
//! packing as Y4M planes, so oracle hashes apply to them unchanged). Colour code points are
//! ITU-T H.273.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use cfa4_frame::{Chroma, PixelData, VideoFrame};

use crate::video::DecodedFrame;
use crate::{CodecError, Result};

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

/// What tells one helper from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelperKind {
    /// The first 8 bytes of the helper's hello.
    pub magic: [u8; 8],
    /// Codec name in messages (`"AV2"`, `"VVC"`).
    pub label: &'static str,
    /// Decoder library name for [`ExternalDecoder::name`] (`"dav2d"`, `"VVdeC"`).
    pub decoder: &'static str,
    /// Name of the thread that reads the helper's replies.
    pub thread_name: &'static str,
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
        let bad = |m: &str| CodecError::Decode(format!("decoder helper: {m}"));
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
            frame: VideoFrame { width: self.width, height: self.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() },
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
fn read_hello(r: &mut impl Read, kind: &HelperKind) -> std::result::Result<String, String> {
    let mut hello = [0u8; 16];
    r.read_exact(&mut hello).map_err(|e| format!("no hello: {e}"))?;
    if hello[..8] != kind.magic {
        return Err(format!("not a CFA4 {} decoder helper", kind.label));
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
    fn spawn(path: &Path, threads: usize, kind: &'static HelperKind) -> Result<Helper> {
        let err = |m: String| CodecError::Unsupported(format!("{} decoder helper {}: {m}", kind.label, path.display()));
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
        let reader = std::thread::Builder::new().name(kind.thread_name.into()).spawn(move || {
            let hello = read_hello(&mut stdout, kind);
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

/// A decoder running in a helper process; restarted after a failure (the stream then continues
/// from the next random access point).
pub struct ExternalDecoder {
    kind: &'static HelperKind,
    path: PathBuf,
    threads: usize,
    helper: Option<Helper>,
    name: String,
    timeout: Duration,
}

impl ExternalDecoder {
    /// Start the helper at `path` (`threads` 0 = the decoder's choice).
    pub fn new(path: &Path, threads: usize, kind: &'static HelperKind) -> Result<ExternalDecoder> {
        let helper = Helper::spawn(path, threads, kind)?;
        let name = format!("{} {} (external)", kind.decoder, helper.version);
        Ok(ExternalDecoder { kind, path: path.to_path_buf(), threads, helper: Some(helper), name, timeout: REQUEST_TIMEOUT })
    }

    /// Time the helper gets to answer one request (default one minute).
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
        if let Some(h) = self.helper.as_mut() {
            h.timeout = timeout;
        }
    }

    /// `"<decoder> <version> (external)"`.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn call(&mut self, tag: &[u8; 4], payload: &[&[u8]]) -> Result<Vec<DecodedFrame>> {
        let label = self.kind.label;
        if self.helper.is_none() {
            let mut helper = Helper::spawn(&self.path, self.threads, self.kind)?;
            helper.timeout = self.timeout;
            self.helper = Some(helper);
        }
        let Some(helper) = self.helper.as_mut() else {
            return Err(CodecError::Decode(format!("{label} helper not running")));
        };
        match helper.request(tag, payload) {
            Ok((pictures, None)) => Ok(pictures.into_iter().map(RawPicture::into_decoded).collect()),
            Ok((_, Some(e))) => Err(CodecError::Decode(format!("{label}: {e}"))),
            Err(e) => {
                self.helper = None;
                Err(CodecError::Decode(format!("{label} decoder helper failed: {e}")))
            }
        }
    }

    /// Decode one sample (`DATA`); an empty sample decodes to nothing.
    pub fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if sample.is_empty() {
            return Ok(Vec::new());
        }
        self.call(b"DATA", &[&pts.to_le_bytes(), sample])
    }

    /// Hand over the held pictures (`FLSH`).
    pub fn flush(&mut self) -> Vec<DecodedFrame> {
        self.call(b"FLSH", &[]).unwrap_or_default()
    }

    /// Forget all state (`RSET`); a helper that fails it is restarted on the next request.
    pub fn reset(&mut self) {
        if self.call(b"RSET", &[]).is_err() {
            self.helper = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KIND: HelperKind = HelperKind { magic: *b"CFA4AV2D", label: "test", decoder: "test", thread_name: "cfa4-test-helper" };

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
        assert!(read_hello(&mut b"CFA4AV2D\x02\0\0\0\0\0\0\0".as_slice(), &TEST_KIND).is_err());
        assert!(read_hello(&mut b"CFA4XXXX\x01\0\0\0\0\0\0\0".as_slice(), &TEST_KIND).is_err());
        assert_eq!(read_hello(&mut b"CFA4AV2D\x01\0\0\0\x02\0\0\0ok".as_slice(), &TEST_KIND).unwrap(), "ok");
    }

    #[test]
    fn a_missing_or_wrong_helper_is_an_error() {
        static KIND: HelperKind = TEST_KIND;
        assert!(ExternalDecoder::new(Path::new("/nonexistent/cfa4-test-decode"), 0, &KIND).is_err());
        // a program that is not a helper: no hello
        assert!(ExternalDecoder::new(Path::new("/bin/true"), 0, &KIND).is_err());
    }
}
