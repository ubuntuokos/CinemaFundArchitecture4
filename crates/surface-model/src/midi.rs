//! MIDI messages as bytes: what the MIDI Monitor shows and which [`SurfaceInput`] a message is.
//!
//! The input is one complete message as the ALSA sequencer decodes it (status byte first, no
//! running status). Anything shorter than its kind needs is described as incomplete, never read
//! past its end.

use crate::{ControlId, MidiKind, SurfaceInput};

/// The 1-based channel of a channel message (as controllers and manuals number them).
fn ch(status: u8) -> u8 {
    (status & 0x0f) + 1
}

/// The two 7-bit data bytes after the status, if the message has them.
fn two(raw: &[u8]) -> Option<(u8, u8)> {
    Some((*raw.get(1)? & 0x7f, *raw.get(2)? & 0x7f))
}

/// A system real-time message that arrives many times a second (clock, active sensing): the
/// monitor leaves these out unless asked.
pub fn is_chatter(raw: &[u8]) -> Option<Chatter> {
    match raw.first() {
        Some(0xf8) => Some(Chatter::Clock),
        Some(0xfe) => Some(Chatter::Sense),
        _ => None,
    }
}

/// The two kinds of [`is_chatter`] messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chatter {
    Clock,
    Sense,
}

/// A short English description of one message, such as `Ch 1 CC 16 = 64`.
pub fn describe(raw: &[u8]) -> String {
    let Some(&status) = raw.first() else { return "(empty)".into() };
    let incomplete = || format!("incomplete message ({} bytes)", raw.len());
    match status & 0xf0 {
        0x80 => two(raw).map_or_else(incomplete, |(n, v)| format!("Ch {} Note Off {n} vel {v}", ch(status))),
        0x90 => two(raw).map_or_else(incomplete, |(n, v)| {
            if v == 0 { format!("Ch {} Note Off {n} vel 0", ch(status)) } else { format!("Ch {} Note On {n} vel {v}", ch(status)) }
        }),
        0xa0 => two(raw).map_or_else(incomplete, |(n, v)| format!("Ch {} Poly Aftertouch {n} = {v}", ch(status))),
        0xb0 => two(raw).map_or_else(incomplete, |(n, v)| format!("Ch {} CC {n} = {v}", ch(status))),
        0xc0 => raw.get(1).map_or_else(incomplete, |p| format!("Ch {} Program {}", ch(status), p & 0x7f)),
        0xd0 => raw.get(1).map_or_else(incomplete, |p| format!("Ch {} Aftertouch {}", ch(status), p & 0x7f)),
        0xe0 => two(raw).map_or_else(incomplete, |(l, m)| format!("Ch {} Pitch Bend {}", ch(status), u16::from(l) | (u16::from(m) << 7))),
        0xf0 => match status {
            0xf0 => format!("SysEx ({} bytes)", raw.len()),
            0xf1 => "MTC Quarter Frame".into(),
            0xf2 => two(raw).map_or_else(incomplete, |(l, m)| format!("Song Position {}", u16::from(l) | (u16::from(m) << 7))),
            0xf3 => "Song Select".into(),
            0xf6 => "Tune Request".into(),
            0xf8 => "Clock".into(),
            0xfa => "Start".into(),
            0xfb => "Continue".into(),
            0xfc => "Stop".into(),
            0xfe => "Active Sensing".into(),
            0xff => "Reset".into(),
            _ => format!("System 0x{status:02X}"),
        },
        _ => format!("data byte 0x{status:02X} without a status"),
    }
}

/// A short English name of a control, such as `Ch 1 CC 16` (channels counted from 1).
pub fn control_label(c: &ControlId) -> String {
    match c {
        ControlId::Midi { channel, kind, number } => {
            let ch = u16::from(*channel) + 1;
            match kind {
                MidiKind::Cc => format!("Ch {ch} CC {number}"),
                MidiKind::Cc14 => format!("Ch {ch} CC {number} (14-bit)"),
                MidiKind::Note => format!("Ch {ch} Note {number}"),
                MidiKind::Rpn => format!("Ch {ch} RPN {number}"),
                MidiKind::Nrpn => format!("Ch {ch} NRPN {number}"),
            }
        }
        ControlId::Hid { name, .. } => name.clone(),
        ControlId::Evdev { ev_type, code } => format!("evdev {ev_type}:{code}"),
        ControlId::Osc { path } => path.clone(),
    }
}

/// The bytes as the monitor shows them: `B0 10 40`.
pub fn hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// The input a message is: notes (press and release), controllers (7-bit absolute; 14-bit pairs
/// and encoders are a profile's reading, later), pitch bend, program change and SysEx;
/// [`SurfaceInput::Raw`] for everything else and for incomplete messages.
pub fn input(raw: &[u8]) -> SurfaceInput {
    let Some(&status) = raw.first() else { return SurfaceInput::Raw };
    let channel = status & 0x0f;
    let midi = |kind, number: u8| ControlId::Midi { channel, kind, number: u16::from(number) };
    let got = match status & 0xf0 {
        0x80 => two(raw).map(|(n, _)| SurfaceInput::Button { control: midi(MidiKind::Note, n), pressed: false }),
        0x90 => two(raw).map(|(n, v)| SurfaceInput::Button { control: midi(MidiKind::Note, n), pressed: v > 0 }),
        0xb0 => two(raw).map(|(n, v)| SurfaceInput::Absolute { control: midi(MidiKind::Cc, n), value: u16::from(v), bits: 7 }),
        0xc0 => raw.get(1).map(|p| SurfaceInput::ProgramChange { channel, program: p & 0x7f }),
        0xe0 => two(raw).map(|(l, m)| SurfaceInput::PitchBend { channel, value: u16::from(l) | (u16::from(m) << 7) }),
        0xf0 if status == 0xf0 => Some(SurfaceInput::SysEx { len: u32::try_from(raw.len()).unwrap_or(u32::MAX) }),
        _ => None,
    };
    got.unwrap_or(SurfaceInput::Raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions() {
        assert_eq!(describe(&[0xb0, 16, 64]), "Ch 1 CC 16 = 64");
        assert_eq!(describe(&[0x9f, 60, 100]), "Ch 16 Note On 60 vel 100");
        assert_eq!(describe(&[0x90, 60, 0]), "Ch 1 Note Off 60 vel 0", "note on with velocity 0 is a release");
        assert_eq!(describe(&[0x81, 60, 10]), "Ch 2 Note Off 60 vel 10");
        assert_eq!(describe(&[0xe0, 0x00, 0x40]), "Ch 1 Pitch Bend 8192");
        assert_eq!(describe(&[0xc3, 5]), "Ch 4 Program 5");
        assert_eq!(describe(&[0xd0, 40]), "Ch 1 Aftertouch 40");
        assert_eq!(describe(&[0xa0, 60, 3]), "Ch 1 Poly Aftertouch 60 = 3");
        assert_eq!(describe(&[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7]), "SysEx (6 bytes)");
        assert_eq!(describe(&[0xf8]), "Clock");
        assert_eq!(describe(&[0xfc]), "Stop");
        assert_eq!(describe(&[0xb0, 16]), "incomplete message (2 bytes)");
        assert_eq!(describe(&[0x40]), "data byte 0x40 without a status");
        assert_eq!(describe(&[]), "(empty)");
    }

    #[test]
    fn control_labels() {
        assert_eq!(control_label(&ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 16 }), "Ch 1 CC 16");
        assert_eq!(control_label(&ControlId::Midi { channel: 9, kind: MidiKind::Note, number: 36 }), "Ch 10 Note 36");
        assert_eq!(control_label(&ControlId::hid_named("button3")), "button3");
    }

    #[test]
    fn chatter_and_hex() {
        assert_eq!(is_chatter(&[0xf8]), Some(Chatter::Clock));
        assert_eq!(is_chatter(&[0xfe]), Some(Chatter::Sense));
        assert_eq!(is_chatter(&[0xb0, 1, 2]), None);
        assert_eq!(hex(&[0xb0, 0x10, 0x40]), "B0 10 40");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn inputs() {
        let cc = ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: 16 };
        assert_eq!(input(&[0xb0, 16, 64]), SurfaceInput::Absolute { control: cc, value: 64, bits: 7 });
        let note = ControlId::Midi { channel: 2, kind: MidiKind::Note, number: 36 };
        assert_eq!(input(&[0x92, 36, 90]), SurfaceInput::Button { control: note.clone(), pressed: true });
        assert_eq!(input(&[0x92, 36, 0]), SurfaceInput::Button { control: note.clone(), pressed: false });
        assert_eq!(input(&[0x82, 36, 64]), SurfaceInput::Button { control: note, pressed: false });
        assert_eq!(input(&[0xe1, 0x7f, 0x7f]), SurfaceInput::PitchBend { channel: 1, value: 16383 });
        assert_eq!(input(&[0xc0, 1]), SurfaceInput::ProgramChange { channel: 0, program: 1 });
        assert_eq!(input(&[0xf0, 1, 2, 0xf7]), SurfaceInput::SysEx { len: 4 });
        assert_eq!(input(&[0xf8]), SurfaceInput::Raw);
        assert_eq!(input(&[0xb0, 1]), SurfaceInput::Raw, "incomplete");
        assert_eq!(input(&[]), SurfaceInput::Raw);
    }
}
