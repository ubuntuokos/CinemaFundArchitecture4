//! The adapter wire protocol (`cfa4-surface-adapter`, version 2): JSON lines between an adapter
//! child process and its supervisor (integration annex E2, E13). The shared description, which
//! every adapter process of CFA4 follows (control surfaces now, capture and playout hosts later),
//! is [`docs/adapter-process-protocol.md`](../../../docs/adapter-process-protocol.md).
//!
//! The child writes [`Event`]s on stdout and reads [`Command`]s on stdin, one JSON object per
//! line. The supervisor turns the events into `SurfaceEvent`s of `cfa4-surface-model`; keeping
//! this protocol separate lets the event model evolve without touching the child.
//!
//! The first line is always [`Event::Hello`]. The supervisor checks it with [`handshake`] before
//! it reads anything else: a child of another protocol version is stopped with a
//! `protocol_mismatch` error, never misread (READY-V4.4).

use serde::{Deserialize, Serialize};

/// The protocol name in [`Event::Hello`].
pub const PROTOCOL: &str = "cfa4-surface-adapter";
/// The protocol version in [`Event::Hello`] (2: `protocolVersion`, `family`, `kind`).
pub const PROTOCOL_VERSION: u32 = 2;
/// Largest key image a command may carry, per side, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 1024;

/// Machine-readable error codes (plan §5.3; AC-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The device node cannot be opened: the udev `uaccess` rule is missing (plan §5.5).
    PermissionDenied,
    /// Another program holds the device (plan §8.1: CFA4 does not take it).
    DeviceBusy,
    /// The firmware does not answer as the profile expects.
    FirmwareUnknown,
    /// The device answers, but not in the expected protocol.
    ProtocolMismatch,
    /// The device stopped answering.
    IoTimeout,
    /// Any other I/O error.
    IoError,
    /// A command line that is not valid JSON or not a known command.
    BadRequest,
    /// A command for a device this adapter does not have open.
    DeviceNotFound,
    /// A command the model cannot do (no such key, no display).
    Unsupported,
    /// A caught panic or another unexpected failure inside the adapter.
    Internal,
}

/// A group of controls of a connected device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlGroup {
    /// Control id within the model (`keys`, `buttons`), as in the catalogue.
    pub id: String,
    /// Control type of plan §3.3 (`lcd_key`, `key`).
    #[serde(rename = "type")]
    pub control_type: String,
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    /// Key image size in pixels, for controls with a display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<(u32, u32)>,
}

/// What an adapter process is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdapterKind {
    /// A control surface family (keys, faders, displays).
    Surface,
    /// A media input/output host (capture and playout devices).
    Io,
}

/// Child → supervisor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    Hello {
        /// [`PROTOCOL`].
        protocol: String,
        /// [`PROTOCOL_VERSION`] of the child.
        protocol_version: u32,
        /// The adapter family (`mirabox-family`).
        family: String,
        kind: AdapterKind,
        adapter_version: String,
        models: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Connected {
        /// Stable id while connected: `hid:<vid>:<pid>:<device path>`.
        device: String,
        /// Catalogue model id (`ajazz.akp153e`).
        model: String,
        vid: u16,
        pid: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        firmware: Option<String>,
        /// `sha256:<hex>` of the serial number; never the serial itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        serial_hash: Option<String>,
        /// False when every unit of the model reports the same serial (so the profile must
        /// bind to the USB path instead; plan §7.3).
        serial_unique: bool,
        controls: Vec<ControlGroup>,
    },
    Disconnected {
        device: String,
    },
    Input {
        device: String,
        control: String,
        index: u32,
        action: InputAction,
    },
    /// The answer to a command that carried a `seq`.
    Ack {
        seq: u64,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
        code: ErrorCode,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputAction {
    Press,
    Release,
}

/// Supervisor → child.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Command {
    /// Show an image on one key. `rgba` is base64 of `width × height × 4` bytes.
    #[serde(rename_all = "camelCase")]
    SetImage {
        #[serde(default)]
        seq: Option<u64>,
        device: String,
        control: String,
        index: u32,
        width: u32,
        height: u32,
        rgba: String,
    },
    /// Blank one key, or every key when `index` is absent.
    ClearImage {
        #[serde(default)]
        seq: Option<u64>,
        device: String,
        #[serde(default)]
        index: Option<u32>,
    },
    SetBrightness {
        #[serde(default)]
        seq: Option<u64>,
        device: String,
        percent: u8,
    },
    Sleep {
        #[serde(default)]
        seq: Option<u64>,
        device: String,
    },
    /// Close every device and end the process.
    Quit,
}

impl Command {
    pub fn seq(&self) -> Option<u64> {
        match self {
            Command::SetImage { seq, .. } | Command::ClearImage { seq, .. } | Command::SetBrightness { seq, .. } | Command::Sleep { seq, .. } => *seq,
            Command::Quit => None,
        }
    }

    pub fn device(&self) -> Option<&str> {
        match self {
            Command::SetImage { device, .. } | Command::ClearImage { device, .. } | Command::SetBrightness { device, .. } | Command::Sleep { device, .. } => {
                Some(device)
            }
            Command::Quit => None,
        }
    }
}

/// Decodes and checks the pixels of a [`Command::SetImage`].
pub fn decode_rgba(width: u32, height: u32, rgba: &str) -> Result<image::RgbaImage, String> {
    use base64::Engine as _;
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(format!("image size {width}×{height} is outside 1…{MAX_IMAGE_SIDE}"));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(rgba).map_err(|e| format!("rgba is not base64: {e}"))?;
    let want = (width as usize) * (height as usize) * 4;
    if bytes.len() != want {
        return Err(format!("rgba has {} bytes, {width}×{height} needs {want}", bytes.len()));
    }
    image::RgbaImage::from_raw(width, height, bytes).ok_or_else(|| "rgba does not fit the size".to_owned())
}

/// Encodes pixels for a [`Command::SetImage`] (supervisor side and tests).
pub fn encode_rgba(pixels: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(pixels)
}

/// `sha256:<hex>` of a serial number (plan §4.2: the repository and logs never see the serial).
pub fn serial_hash(serial: &str) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(serial.as_bytes());
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// What a supervisor learned from an accepted [`Event::Hello`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub family: String,
    pub kind: AdapterKind,
    pub adapter_version: String,
    pub models: Vec<String>,
}

/// Why a supervisor refused a child: send `quit`, stop the child, report `code` with `message`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub code: ErrorCode,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", serde_json::to_value(self.code).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default(), self.message)
    }
}

/// Checks a child's first line, the supervisor side of the handshake (READY-V4.4).
///
/// Accepts only an [`Event::Hello`] of [`PROTOCOL`] at [`PROTOCOL_VERSION`] whose kind is
/// `want`. Anything else is refused, never a panic: another version (version 1 children sent
/// `version` instead of `protocolVersion`) or another kind is `protocol_mismatch`; a line that is
/// not a hello at all is `bad_request`. The line is read as plain JSON first, so a hello of a
/// newer version with fields this version does not know is still recognised as a mismatch.
pub fn handshake(line: &str, want: AdapterKind) -> Result<Hello, Refused> {
    let refuse = |code, message: String| Refused { code, message };
    let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| refuse(ErrorCode::BadRequest, format!("the first line is not JSON: {e}")))?;
    if v.get("type").and_then(|t| t.as_str()) != Some("hello") {
        return Err(refuse(ErrorCode::BadRequest, "the first line is not a hello".into()));
    }
    let protocol = v.get("protocol").and_then(|p| p.as_str()).unwrap_or_default();
    if protocol != PROTOCOL {
        return Err(refuse(ErrorCode::ProtocolMismatch, format!("protocol `{protocol}`, expected `{PROTOCOL}`")));
    }
    let version = v.get("protocolVersion").or_else(|| v.get("version")).and_then(|n| n.as_u64());
    if version != Some(u64::from(PROTOCOL_VERSION)) {
        let got = version.map_or_else(|| "none".to_owned(), |n| n.to_string());
        return Err(refuse(ErrorCode::ProtocolMismatch, format!("protocol version {got}, this CFA4 speaks {PROTOCOL_VERSION}")));
    }
    match serde_json::from_value::<Event>(v) {
        Ok(Event::Hello { family, kind, adapter_version, models, .. }) if kind == want => Ok(Hello { family, kind, adapter_version, models }),
        Ok(Event::Hello { family, kind, .. }) => Err(refuse(ErrorCode::ProtocolMismatch, format!("`{family}` is a {kind:?} adapter, expected {want:?}"))),
        Ok(_) => Err(refuse(ErrorCode::BadRequest, "the first line is not a hello".into())),
        Err(e) => Err(refuse(ErrorCode::ProtocolMismatch, format!("hello does not match version {PROTOCOL_VERSION}: {e}"))),
    }
}
