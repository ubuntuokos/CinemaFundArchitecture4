//! Device-independent control surface types (CFA4-CONTROL-SURFACE-001, [ADR 0012]).
//!
//! Every physical input (HID, MIDI, later OSC) is normalised by the device I/O crate
//! (`cfa4-surface`, L5) into a [`SurfaceEvent`]; the engine (L4) maps it to a command. Feedback
//! flows the other way as [`SurfaceFeedback`]. This crate holds only the shared types and the pure
//! [`midi`] byte-stream parser: no I/O, no clock, no threads, so L4 and L5 can both depend on it.
//!
//! Normalisation never drops what a device needs to work: MIDI channel, controller number,
//! message kind, raw value and its resolution, press and release, the sign of a relative value and
//! the original HID or evdev code all survive, and the raw message is kept in [`SurfaceEvent::raw`]
//! for the MIDI Monitor.
//!
//! [ADR 0012]: ../../../docs/adr/ADR-0012-control-surface.md

pub mod midi;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Longest raw message kept with an event (enough for any channel message and short SysEx).
pub const MAX_RAW: usize = 64;

/// A stable device identity: `hid:<vid>:<pid>:<physical path>` or `midi:<client>:<port>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

impl DeviceId {
    /// The id of a HID device from its USB ids and its physical path (`HID_PHYS`).
    pub fn hid(vid: u16, pid: u16, phys: &str) -> Self {
        DeviceId(format!("hid:{vid:04x}:{pid:04x}:{phys}"))
    }

    /// A version of the id that is safe as a file name (`active-<device>.json`).
    pub fn file_stem(&self) -> String {
        self.0.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A monotonic timestamp in nanoseconds, taken by the I/O crate when the input was read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonoNs(pub u64);

/// Where an event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceProtocol {
    Hid,
    Midi,
    Osc,
}

/// One normalised input from one device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceEvent {
    pub device: DeviceId,
    pub at: MonoNs,
    pub protocol: SurfaceProtocol,
    pub input: SurfaceInput,
    /// Strictly increasing per device; a gap means events were lost (counted, never silent).
    pub seq: u64,
    /// The raw report or message, at most [`MAX_RAW`] bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<u8>,
}

impl SurfaceEvent {
    /// Whether the event is a trigger: never coalesced and never dropped (F0 spec §3).
    pub fn is_trigger(&self) -> bool {
        self.input.is_trigger()
    }
}

/// The MIDI message kinds a control can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidiKind {
    Cc,
    /// 14-bit controller: MSB on `number` (0–31), LSB on `number + 32`.
    Cc14,
    Note,
    Rpn,
    Nrpn,
}

/// The physical control an input belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlId {
    Midi {
        channel: u8,
        kind: MidiKind,
        number: u16,
    },
    /// A HID control by its driver-level name (`button1`…, `jog`, `shuttle`) and, when the report
    /// descriptor gives them, its usage page and usage.
    #[serde(rename_all = "camelCase")]
    Hid {
        name: String,
        usage_page: u16,
        usage: u16,
    },
    Evdev {
        ev_type: u16,
        code: u16,
    },
    Osc {
        path: String,
    },
}

impl ControlId {
    /// A HID control known only by its driver-level name.
    pub fn hid_named(name: &str) -> Self {
        ControlId::Hid { name: name.to_string(), usage_page: 0, usage: 0 }
    }
}

/// How a relative (endless) control encodes its steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelEncoding {
    /// Two's complement: 1..=63 up, 127..=65 down (profile mode `encoder-2`).
    TwosComplement,
    /// Bit 6 is the sign: 1..=63 up, 65..=127 down (`encoder-left`).
    SignMagnitude,
    /// 64 is zero: 65.. up, ..63 down (`encoder-right`).
    Offset64,
    /// Only the direction: 1 up, 127 down (`encoder-binary`).
    Binary,
}

impl RelEncoding {
    /// The signed step a 7-bit value means; 0 for a value that carries no movement.
    pub fn delta(self, value: u8) -> i32 {
        let v = i32::from(value & 0x7f);
        match self {
            RelEncoding::TwosComplement => {
                if v < 64 {
                    v
                } else {
                    v - 128
                }
            }
            RelEncoding::SignMagnitude => {
                if v & 0x40 == 0 {
                    v
                } else {
                    -(v & 0x3f)
                }
            }
            RelEncoding::Offset64 => v - 64,
            RelEncoding::Binary => match v {
                0 => 0,
                1..=63 => 1,
                _ => -1,
            },
        }
    }
}

/// A normalised input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SurfaceInput {
    /// Press and release are separate events.
    Button {
        control: ControlId,
        pressed: bool,
    },
    /// An absolute value with its resolution (7 or 14 bits); `value` is the raw value.
    Absolute {
        control: ControlId,
        value: u16,
        bits: u8,
    },
    Relative {
        control: ControlId,
        delta: i32,
        encoding: RelEncoding,
    },
    /// Jog wheel steps (ShuttlePRO: one click is one step).
    Jog {
        control: ControlId,
        delta: i32,
    },
    /// Shuttle ring position `-range..=range`, 0 = rest.
    Shuttle {
        control: ControlId,
        position: i8,
        range: i8,
    },
    Touch {
        control: ControlId,
        touched: bool,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    /// 14-bit, 8192 = centre.
    PitchBend {
        channel: u8,
        value: u16,
    },
    /// The bytes are in [`SurfaceEvent::raw`]; SysEx maps only to triggers.
    SysEx {
        len: u32,
    },
    /// Anything else: seen by the Monitor and Learn only.
    Raw,
}

impl SurfaceInput {
    /// Triggers are never coalesced or dropped; continuous values may be coalesced (last value wins).
    pub fn is_trigger(&self) -> bool {
        !matches!(self, SurfaceInput::Absolute { .. } | SurfaceInput::Relative { .. } | SurfaceInput::PitchBend { .. })
    }

    /// The control the input belongs to, when it has one.
    pub fn control(&self) -> Option<&ControlId> {
        match self {
            SurfaceInput::Button { control, .. }
            | SurfaceInput::Absolute { control, .. }
            | SurfaceInput::Relative { control, .. }
            | SurfaceInput::Jog { control, .. }
            | SurfaceInput::Shuttle { control, .. }
            | SurfaceInput::Touch { control, .. } => Some(control),
            _ => None,
        }
    }
}

/// What a feedback value is shown on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum FeedbackTarget {
    Led { control: ControlId },
    Fader { control: ControlId },
    Ring { control: ControlId },
    Display { index: u16 },
    Timecode,
    Meter { index: u16 },
    DeviceState,
}

/// A feedback value; the I/O crate turns it into the device's own protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum FeedbackValue {
    Bool(bool),
    U14(u16),
    Text(String),
    /// dBFS.
    Level(f32),
    /// `cfa4_time::Tick` units (254 016 000 000 per second).
    Tick(i64),
}

/// How urgently feedback goes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FeedbackPriority {
    Immediate,
    Coalesce,
}

/// Feedback produced by the engine for one device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceFeedback {
    pub device: DeviceId,
    pub target: FeedbackTarget,
    pub value: FeedbackValue,
    pub priority: FeedbackPriority,
    pub at: MonoNs,
}

/// The state of a device as the I/O crate reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceState {
    Online,
    Offline,
    Error,
    Disabled,
}

/// Per-device counters (F0 spec §3), as `surface.state` reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCounters {
    pub received: u64,
    pub dispatched: u64,
    pub coalesced: u64,
    pub stalls: u64,
    pub gaps: u64,
    pub errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_encodings() {
        assert_eq!(RelEncoding::TwosComplement.delta(1), 1);
        assert_eq!(RelEncoding::TwosComplement.delta(127), -1);
        assert_eq!(RelEncoding::TwosComplement.delta(65), -63);
        assert_eq!(RelEncoding::SignMagnitude.delta(3), 3);
        assert_eq!(RelEncoding::SignMagnitude.delta(65), -1);
        assert_eq!(RelEncoding::Offset64.delta(66), 2);
        assert_eq!(RelEncoding::Offset64.delta(63), -1);
        assert_eq!(RelEncoding::Binary.delta(1), 1);
        assert_eq!(RelEncoding::Binary.delta(127), -1);
        assert_eq!(RelEncoding::Binary.delta(0), 0);
        // the top bit of a corrupt byte is ignored, not a panic
        assert_eq!(RelEncoding::TwosComplement.delta(0x81), 1);
    }

    #[test]
    fn triggers_and_continuous() {
        let c = ControlId::hid_named("jog");
        assert!(SurfaceInput::Jog { control: c.clone(), delta: 1 }.is_trigger());
        assert!(SurfaceInput::Shuttle { control: c.clone(), position: 0, range: 7 }.is_trigger());
        assert!(SurfaceInput::Button { control: c.clone(), pressed: false }.is_trigger());
        assert!(!SurfaceInput::Absolute { control: c.clone(), value: 3, bits: 7 }.is_trigger());
        assert!(!SurfaceInput::PitchBend { channel: 0, value: 8192 }.is_trigger());
        assert_eq!(SurfaceInput::Raw.control(), None);
    }

    #[test]
    fn device_ids() {
        let d = DeviceId::hid(0x0b33, 0x0030, "usb-0000:00:14.0-3/input0");
        assert_eq!(d.0, "hid:0b33:0030:usb-0000:00:14.0-3/input0");
        assert_eq!(d.file_stem(), "hid_0b33_0030_usb-0000_00_14.0-3_input0");
    }

    #[test]
    fn events_round_trip_through_json() {
        let e = SurfaceEvent {
            device: DeviceId("midi:nanoKONTROL2:0".into()),
            at: MonoNs(42),
            protocol: SurfaceProtocol::Midi,
            input: SurfaceInput::Relative {
                control: ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 16 },
                delta: -2,
                encoding: RelEncoding::TwosComplement,
            },
            seq: 7,
            raw: vec![0xb0, 16, 126],
        };
        let j = serde_json::to_value(&e).unwrap_or_default();
        assert_eq!(j["input"]["type"], "relative");
        assert_eq!(j["input"]["encoding"], "twos-complement");
        assert_eq!(j["device"], "midi:nanoKONTROL2:0");
        let back: Option<SurfaceEvent> = serde_json::from_value(j).ok();
        assert_eq!(back, Some(e));
        let f = SurfaceFeedback {
            device: DeviceId("x".into()),
            target: FeedbackTarget::Timecode,
            value: FeedbackValue::Tick(254_016_000_000),
            priority: FeedbackPriority::Coalesce,
            at: MonoNs(1),
        };
        let fj = serde_json::to_value(&f).unwrap_or_default();
        assert_eq!(fj["value"]["type"], "tick");
        let fb: Option<SurfaceFeedback> = serde_json::from_value(fj).ok();
        assert_eq!(fb, Some(f));
    }
}
