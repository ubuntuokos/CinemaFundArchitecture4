//! The adapter wire protocol (`cfa4-surface-adapter`, version 1): JSON lines between an adapter
//! child process and its supervisor (integration annex E2, E13).
//!
//! The child writes [`Event`]s on stdout and reads [`Command`]s on stdin, one JSON object per
//! line. The supervisor turns the events into `SurfaceEvent`s of `cfa4-surface-model`; keeping
//! this protocol separate lets the event model evolve without touching the child.

use serde::{Deserialize, Serialize};

/// The protocol name in [`Event::Hello`].
pub const PROTOCOL: &str = "cfa4-surface-adapter";
/// The protocol version in [`Event::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;
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

/// Child → supervisor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    Hello {
        protocol: String,
        version: u32,
        adapter: String,
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
