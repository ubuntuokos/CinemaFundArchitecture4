//! The three support dimensions of CFA4-HW-CONTROL-SURFACES-001 §3.1 and the control types of §3.3.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What CFA4 intends to support for a model (§3.1, written by hand in the catalogue).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SupportGoal {
    /// Every control and feature of the device.
    Full,
    /// As much as the protocol allows; gaps are acceptable.
    BestEffort,
    /// Depends on the concrete model or firmware within the family.
    ModelDependent,
    /// Only through a system driver that sends key events (§8.3); no images, LEDs or displays.
    Indirect,
}

/// The proven state of one feature of one control (§3.1).
///
/// The catalogue may declare every value except [`FeatureStatus::Verified`]: that one only comes
/// from a valid hardware test report (§4, AC-04).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeatureStatus {
    Verified,
    Partial,
    Untested,
    Unsupported,
    NotApplicable,
    Blocked,
}

/// The computed state of a model (§3.2). Never written by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelStatus {
    Planned,
    Experimental,
    Partial,
    Verified,
    Blocked,
    Unavailable,
}

/// The run-time state of one connected instance (§3.1), as the registry reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionState {
    Connected,
    Disconnected,
    PermissionDenied,
    DeviceBusy,
    FirmwareUnknown,
    Error,
}

/// Whether the USB ids of a model come from an adapter's source or a real device (§2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Identification {
    Confirmed,
    #[default]
    Unconfirmed,
}

/// How a device can reach CFA4 without its own adapter (§2 `system_path`, §8.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPath {
    #[default]
    None,
    KernelHidInput,
    UserspaceUinput,
}

/// The control types of §3.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
    LcdKey,
    Key,
    Encoder,
    JogWheel,
    Shuttle,
    Fader,
    TouchStrip,
    TouchScreen,
    InfoDisplay,
    Trackball,
    Axis,
    Pedal,
    Device,
}

impl ControlType {
    /// The snake_case name used in feature ids (`lcd_key.press`).
    pub fn as_str(self) -> &'static str {
        match self {
            ControlType::LcdKey => "lcd_key",
            ControlType::Key => "key",
            ControlType::Encoder => "encoder",
            ControlType::JogWheel => "jog_wheel",
            ControlType::Shuttle => "shuttle",
            ControlType::Fader => "fader",
            ControlType::TouchStrip => "touch_strip",
            ControlType::TouchScreen => "touch_screen",
            ControlType::InfoDisplay => "info_display",
            ControlType::Trackball => "trackball",
            ControlType::Axis => "axis",
            ControlType::Pedal => "pedal",
            ControlType::Device => "device",
        }
    }

    /// The features a control of this type can have (§3.3).
    pub fn features(self) -> &'static [&'static str] {
        match self {
            ControlType::LcdKey => &["press", "release", "image", "brightness"],
            ControlType::Key => &["press", "release", "led"],
            ControlType::Encoder => &["turn", "press", "release", "led"],
            ControlType::JogWheel => &["turn", "touch", "display"],
            ControlType::Shuttle => &["position", "direction"],
            ControlType::Fader => &["value", "touch", "motor"],
            ControlType::TouchStrip => &["tap", "drag", "image"],
            ControlType::TouchScreen => &["tap", "drag", "image", "multitouch"],
            ControlType::InfoDisplay => &["image", "text"],
            ControlType::Trackball => &["x", "y", "ring"],
            ControlType::Axis => &["translate", "rotate"],
            ControlType::Pedal => &["press", "release"],
            ControlType::Device => &["identify", "firmware_query", "sleep", "haptic", "hotplug"],
        }
    }
}

impl fmt::Display for ControlType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The milestone a model is planned for (§9), or `Planned` without one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Phase {
    M1,
    M2,
    M3,
    M4,
    M5,
    M6,
    M7,
    #[serde(rename = "PLANNED")]
    Planned,
}
