//! The real `mirabox-family` backend: AJAZZ / MiraBox decks through mirajazz (MPL-2.0) on
//! async-hid's hidraw backend (MIT).
//!
//! mirajazz is used as a library, unchanged. Its known panic paths are kept out of reach: the
//! protocol version is checked by [`Profile::problems`] before `Device::connect` (which asserts on
//! it), key number 0 is never subtracted from (`process_input`), and its `DeviceWatcher` (which
//! unwraps) is not used; plugging is detected by enumerating on the loop's scan tick instead.

use std::sync::Arc;
use std::time::Duration;

use async_hid::{DeviceId, DeviceInfo, HidBackend, HidError};
use futures_lite::StreamExt;
use mirajazz::device::Device;
use mirajazz::error::MirajazzError;
use mirajazz::state::{DeviceStateReader, DeviceStateUpdate};
use mirajazz::types::{DeviceInput, ImageFormat, ImageMirroring, ImageMode as MjMode, ImageRotation};
use serde::Serialize;

use crate::profile::{ImageMode, Mirror, Profile};
use crate::wire::ErrorCode;
use crate::{AdapterError, Backend, DeviceIo, Found, Opened, RawInput, RunOptions};

/// USB vendor ids of the MiraBox family seen in the references (AJAZZ / MiraBox 0x0300; the
/// mirajazz `n1` example 0x6603). `--probe` lists every HID device with one of these, or with the
/// family's vendor usage page, even when no profile matches it yet.
pub const FAMILY_VENDORS: [u16; 2] = [0x0300, 0x6603];
/// The vendor usage page the family's decks use for their key and image reports.
pub const FAMILY_USAGE_PAGE: u16 = 0xffa0;

/// The OS path of a HID device (`/dev/hidraw3`).
fn path_of(id: &DeviceId) -> String {
    #[allow(irrefutable_let_patterns)]
    if let DeviceId::DevPath(p) = id { p.display().to_string() } else { format!("{id:?}") }
}

fn kind_code(kind: std::io::ErrorKind) -> Option<ErrorCode> {
    match kind {
        std::io::ErrorKind::PermissionDenied => Some(ErrorCode::PermissionDenied),
        std::io::ErrorKind::ResourceBusy => Some(ErrorCode::DeviceBusy),
        std::io::ErrorKind::TimedOut => Some(ErrorCode::IoTimeout),
        _ => None,
    }
}

fn text_code(text: &str) -> ErrorCode {
    let t = text.to_ascii_lowercase();
    if t.contains("permission denied") || t.contains("eacces") || t.contains("eperm") {
        ErrorCode::PermissionDenied
    } else if t.contains("busy") {
        ErrorCode::DeviceBusy
    } else if t.contains("timed out") || t.contains("timeout") {
        ErrorCode::IoTimeout
    } else {
        ErrorCode::IoError
    }
}

fn hid_error(e: &HidError) -> AdapterError {
    let text = e.to_string();
    let code = match e {
        HidError::Disconnected | HidError::NotConnected => ErrorCode::IoError,
        HidError::Other(inner) => match inner.downcast_ref::<std::io::Error>().and_then(|io| kind_code(io.kind())) {
            Some(c) => c,
            None => text_code(&text),
        },
        HidError::Message(_) => text_code(&text),
    };
    let message = match code {
        ErrorCode::PermissionDenied => format!("{text} (is the udev `uaccess` rule for the device installed?)"),
        ErrorCode::DeviceBusy => format!("{text} (another program holds the device)"),
        _ => text,
    };
    AdapterError::new(code, message)
}

fn mj_error(e: MirajazzError) -> AdapterError {
    match e {
        MirajazzError::HidError(h) => hid_error(&h),
        MirajazzError::DeviceNotFoundError => AdapterError::new(ErrorCode::IoError, "the device is no longer connected"),
        MirajazzError::InvalidDeviceError | MirajazzError::UnrecognizedPID => {
            AdapterError::new(ErrorCode::FirmwareUnknown, format!("the device does not answer as the profile expects ({e})"))
        }
        MirajazzError::BadData => AdapterError::new(ErrorCode::ProtocolMismatch, "the device sent unexpected data"),
        MirajazzError::NoScreen | MirajazzError::InvalidKeyIndex | MirajazzError::UnsupportedOperation => {
            AdapterError::new(ErrorCode::Unsupported, format!("{e}"))
        }
        MirajazzError::ImageError(_) | MirajazzError::PoisonError | MirajazzError::WatcherAlreadyInitialized => {
            AdapterError::new(ErrorCode::Internal, format!("{e}"))
        }
    }
}

fn image_format(p: &Profile) -> ImageFormat {
    ImageFormat {
        mode: match p.image.mode {
            ImageMode::Jpeg => MjMode::JPEG,
            ImageMode::Bmp => MjMode::BMP,
        },
        size: (p.image.width as usize, p.image.height as usize),
        rotation: match p.image.rotation {
            90 => ImageRotation::Rot90,
            180 => ImageRotation::Rot180,
            270 => ImageRotation::Rot270,
            _ => ImageRotation::Rot0,
        },
        mirror: match p.image.mirror {
            Mirror::None => ImageMirroring::None,
            Mirror::X => ImageMirroring::X,
            Mirror::Y => ImageMirroring::Y,
            Mirror::Both => ImageMirroring::Both,
        },
    }
}

/// Longest key state vector `process_input` reports: any 1-based key number fits.
const STATE_LEN: usize = 256;

/// Maps one raw input report (1-based key number, state byte) to a key state vector. The device
/// numbers keys from 1; 0 carries no key and is ignored (mirajazz's own example subtracts 1
/// unchecked).
fn process_input(key: u8, state: u8) -> Result<DeviceInput, MirajazzError> {
    let Some(index) = key.checked_sub(1) else { return Ok(DeviceInput::NoData) };
    let mut buttons = vec![false; STATE_LEN];
    if let Some(b) = buttons.get_mut(usize::from(index)) {
        *b = state != 0;
    }
    Ok(DeviceInput::ButtonStateChange(buttons))
}

async fn enumerate() -> Result<Vec<async_hid::Device>, AdapterError> {
    let backend = HidBackend::default();
    let stream = backend.enumerate().await.map_err(|e| hid_error(&e))?;
    Ok(stream.collect::<Vec<_>>().await)
}

fn matches(d: &DeviceInfo, p: &Profile) -> bool {
    d.matches(p.usage_page, p.usage, p.vid, p.pid)
}

/// The mirajazz backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct MiraboxBackend;

impl Backend for MiraboxBackend {
    type Device = MiraboxDevice;

    async fn scan(&self, profiles: &[Profile]) -> Result<Vec<Found>, AdapterError> {
        let mut found = Vec::new();
        for d in enumerate().await? {
            if let Some(i) = profiles.iter().position(|p| matches(&d, p)) {
                found.push(Found { path: path_of(&d.id), profile: i });
            }
        }
        Ok(found)
    }

    async fn open(&self, found: &Found, profile: &Profile) -> Result<(MiraboxDevice, Opened), AdapterError> {
        // the profile list was checked by `profile::parse`; this guards mirajazz's `assert!`
        if !(1..=3).contains(&profile.protocol_version) {
            return Err(AdapterError::new(ErrorCode::Internal, format!("protocol_version {} is not 1…3", profile.protocol_version)));
        }
        let info: DeviceInfo = enumerate()
            .await?
            .into_iter()
            .find(|d| path_of(&d.id) == found.path && matches(d, profile))
            .map(|d| (*d).clone())
            .ok_or_else(|| AdapterError::new(ErrorCode::IoError, "the device is no longer connected"))?;
        let dev = Device::connect(&info, profile.protocol_version, profile.raw_keys as usize, 0).await.map_err(mj_error)?;
        let reader = dev.get_reader(process_input);
        let opened = Opened {
            firmware: dev.firmware_version.clone(),
            // pv1 units all report one hard-coded serial (mirajazz); `unique_serial = false` in
            // the profile tells the supervisor to bind to the USB path instead.
            serial: Some(dev.serial_number.clone()),
        };
        Ok((MiraboxDevice { dev, reader, format: image_format(profile) }, opened))
    }
}

/// One open deck.
pub struct MiraboxDevice {
    dev: Device,
    reader: Arc<DeviceStateReader>,
    format: ImageFormat,
}

impl DeviceIo for MiraboxDevice {
    async fn read(&mut self, timeout: Duration) -> Result<Vec<RawInput>, AdapterError> {
        let updates = self.reader.read(Some(timeout)).await.map_err(mj_error)?;
        Ok(updates
            .into_iter()
            .filter_map(|u| match u {
                DeviceStateUpdate::ButtonDown(i) => Some((i, true)),
                DeviceStateUpdate::ButtonUp(i) => Some((i, false)),
                _ => None,
            })
            .collect())
    }

    async fn set_image(&mut self, device_index: u8, image: image::RgbaImage) -> Result<(), AdapterError> {
        self.dev.set_button_image(device_index, self.format, image::DynamicImage::ImageRgba8(image)).await.map_err(mj_error)?;
        self.dev.flush().await.map_err(mj_error)
    }

    async fn clear(&mut self, device_index: Option<u8>) -> Result<(), AdapterError> {
        match device_index {
            Some(i) => self.dev.clear_button_image(i).await.map_err(mj_error)?,
            None => self.dev.clear_all_button_images().await.map_err(mj_error)?,
        }
        self.dev.flush().await.map_err(mj_error)
    }

    async fn set_brightness(&mut self, percent: u8) -> Result<(), AdapterError> {
        self.dev.set_brightness(percent).await.map_err(mj_error)
    }

    async fn sleep(&mut self) -> Result<(), AdapterError> {
        self.dev.sleep().await.map_err(mj_error)
    }

    async fn close(&mut self) {
        // blanks the keys and puts the deck to sleep; the device may already be gone
        let _ = self.dev.shutdown().await;
    }
}

/// One HID device `--probe` reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeEntry {
    pub path: String,
    pub vid: String,
    pub pid: String,
    pub usage_page: String,
    pub usage: u16,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_hash: Option<String>,
    /// The profile that matches the device, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Whether that profile's values were checked on a real unit.
    pub confirmed: bool,
}

/// Lists the HID devices of the family (and every device a profile matches), without opening
/// any. This is what the owner runs first on a new unit (plan §8.3, Ajazz addition).
pub async fn probe(profiles: &[Profile]) -> Result<Vec<ProbeEntry>, AdapterError> {
    let mut out = Vec::new();
    for d in enumerate().await? {
        let profile = profiles.iter().find(|p| matches(&d, p));
        if profile.is_none() && !FAMILY_VENDORS.contains(&d.vendor_id) && d.usage_page != FAMILY_USAGE_PAGE {
            continue;
        }
        out.push(ProbeEntry {
            path: path_of(&d.id),
            vid: format!("{:04x}", d.vendor_id),
            pid: format!("{:04x}", d.product_id),
            usage_page: format!("{:04x}", d.usage_page),
            usage: d.usage_id,
            name: d.name.clone(),
            serial_hash: d.serial_number.as_deref().map(crate::wire::serial_hash),
            profile: profile.map(|p| p.name.clone()),
            confirmed: profile.is_some_and(|p| p.confirmed),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.usage_page.cmp(&b.usage_page)));
    Ok(out)
}

/// Runs the adapter on stdin / stdout with the shipped profiles. Needs a multi-thread tokio
/// runtime (mirajazz encodes images in `block_in_place`).
pub async fn run_stdio(adapter_version: &str) -> Result<(), String> {
    let profiles = crate::profile::builtin()?;
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    crate::run(Arc::new(MiraboxBackend), adapter_version, profiles, input, tokio::io::stdout(), RunOptions::default())
        .await
        .map_err(|e| format!("mirabox-family adapter: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_zero_is_ignored_and_keys_are_one_based() {
        assert!(matches!(process_input(0, 1), Ok(DeviceInput::NoData)));
        let Ok(DeviceInput::ButtonStateChange(b)) = process_input(1, 1) else { panic!() };
        assert_eq!(b.len(), STATE_LEN);
        assert!(b[0] && b.iter().filter(|x| **x).count() == 1);
        let Ok(DeviceInput::ButtonStateChange(b)) = process_input(255, 0) else { panic!() };
        assert!(b.iter().all(|x| !x));
    }

    #[test]
    fn os_errors_get_their_codes() {
        let io = |k: std::io::ErrorKind| hid_error(&HidError::from_backend(std::io::Error::from(k))).code;
        assert_eq!(io(std::io::ErrorKind::PermissionDenied), ErrorCode::PermissionDenied);
        assert_eq!(io(std::io::ErrorKind::ResourceBusy), ErrorCode::DeviceBusy);
        assert_eq!(io(std::io::ErrorKind::TimedOut), ErrorCode::IoTimeout);
        assert_eq!(io(std::io::ErrorKind::BrokenPipe), ErrorCode::IoError);
        assert_eq!(hid_error(&HidError::message("EACCES: Permission denied")).code, ErrorCode::PermissionDenied);
        assert_eq!(hid_error(&HidError::message("EBUSY: Device or resource busy")).code, ErrorCode::DeviceBusy);
        assert!(hid_error(&HidError::message("Permission denied")).message.contains("uaccess"));
        assert_eq!(hid_error(&HidError::Disconnected).code, ErrorCode::IoError);
        assert_eq!(mj_error(MirajazzError::InvalidDeviceError).code, ErrorCode::FirmwareUnknown);
        assert_eq!(mj_error(MirajazzError::BadData).code, ErrorCode::ProtocolMismatch);
        assert_eq!(mj_error(MirajazzError::InvalidKeyIndex).code, ErrorCode::Unsupported);
    }

    #[test]
    fn the_profile_image_format_reaches_mirajazz() {
        let p = crate::profile::builtin().unwrap().remove(0);
        let f = image_format(&p);
        assert!(matches!(f.mode, MjMode::JPEG));
        assert_eq!(f.size, (85, 85));
        assert!(matches!(f.rotation, ImageRotation::Rot90));
        assert!(matches!(f.mirror, ImageMirroring::Both));
    }
}
