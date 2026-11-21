//! mirabox-family device profiles (`profiles/mirabox.toml`) and the key layout they describe.

use serde::Deserialize;

use crate::wire::ControlGroup;

/// The shipped profiles.
pub const BUILTIN: &str = include_str!("../profiles/mirabox.toml");

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    Jpeg,
    Bmp,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mirror {
    None,
    X,
    Y,
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSpec {
    pub mode: ImageMode,
    pub width: u32,
    pub height: u32,
    /// Clockwise degrees: 0, 90, 180 or 270.
    pub rotation: u32,
    pub mirror: Mirror,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub rows: u32,
    pub columns: u32,
}

/// One device model at one USB id.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Catalogue model id.
    pub model: String,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// mirajazz protocol version, 1…3 (mirajazz asserts on anything else, so it is checked here).
    pub protocol_version: usize,
    /// Inputs the device reports (keys with and without a screen).
    pub raw_keys: u32,
    pub unique_serial: bool,
    /// The values were checked on a real unit (a CFA4 report exists).
    pub confirmed: bool,
    pub image: ImageSpec,
    /// Device index (0-based) of each display position.
    pub display_to_device: Vec<u8>,
    pub grid: Grid,
    /// The first `lcd_columns` columns of the grid are keys with a screen; the rest are plain keys.
    pub lcd_columns: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default, rename = "profile")]
    profiles: Vec<Profile>,
}

/// A control as the catalogue names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// A key with a screen (`keys`, `lcd_key`).
    Key(u32),
    /// A key without a screen (`buttons`, `key`).
    Button(u32),
}

impl Control {
    pub fn group(self) -> &'static str {
        match self {
            Control::Key(_) => "keys",
            Control::Button(_) => "buttons",
        }
    }

    pub fn index(self) -> u32 {
        match self {
            Control::Key(i) | Control::Button(i) => i,
        }
    }
}

impl Profile {
    fn extra_columns(&self) -> u32 {
        self.grid.columns.saturating_sub(self.lcd_columns)
    }

    pub fn key_count(&self) -> u32 {
        self.grid.rows * self.lcd_columns
    }

    pub fn button_count(&self) -> u32 {
        self.grid.rows * self.extra_columns()
    }

    /// The display position of a control.
    pub fn display_of(&self, c: Control) -> Option<u32> {
        match c {
            Control::Key(i) if i < self.key_count() && self.lcd_columns > 0 => Some((i / self.lcd_columns) * self.grid.columns + i % self.lcd_columns),
            Control::Button(i) if i < self.button_count() => {
                let extra = self.extra_columns();
                Some((i / extra) * self.grid.columns + self.lcd_columns + i % extra)
            }
            _ => None,
        }
    }

    /// The control at a display position.
    pub fn control_at(&self, display: u32) -> Option<Control> {
        if self.grid.columns == 0 || display >= self.grid.rows * self.grid.columns {
            return None;
        }
        let (row, col) = (display / self.grid.columns, display % self.grid.columns);
        if col < self.lcd_columns {
            Some(Control::Key(row * self.lcd_columns + col))
        } else {
            Some(Control::Button(row * self.extra_columns() + col - self.lcd_columns))
        }
    }

    /// The device index (0-based) a key image is written to.
    pub fn device_index(&self, c: Control) -> Option<u8> {
        let d = self.display_of(c)?;
        self.display_to_device.get(d as usize).copied()
    }

    /// The control at a device index (0-based; the device itself reports it 1-based).
    pub fn control_of_device(&self, device: u8) -> Option<Control> {
        let display = self.display_to_device.iter().position(|&x| x == device)?;
        self.control_at(display as u32)
    }

    /// The control groups announced in [`crate::wire::Event::Connected`].
    pub fn controls(&self) -> Vec<ControlGroup> {
        let mut out = vec![ControlGroup {
            id: "keys".into(),
            control_type: "lcd_key".into(),
            count: self.key_count(),
            rows: Some(self.grid.rows),
            columns: Some(self.lcd_columns),
            image: Some((self.image.width, self.image.height)),
        }];
        if self.button_count() > 0 {
            out.push(ControlGroup {
                id: "buttons".into(),
                control_type: "key".into(),
                count: self.button_count(),
                rows: Some(self.grid.rows),
                columns: Some(self.extra_columns()),
                image: None,
            });
        }
        out
    }

    /// Every problem with the profile; empty when it is usable.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        let who = &self.name;
        if !(1..=3).contains(&self.protocol_version) {
            p.push(format!("{who}: protocol_version {} is not 1…3", self.protocol_version));
        }
        let cells = self.grid.rows * self.grid.columns;
        if cells != self.raw_keys {
            p.push(format!("{who}: grid {}×{} has {cells} positions, raw_keys is {}", self.grid.rows, self.grid.columns, self.raw_keys));
        }
        if self.lcd_columns > self.grid.columns {
            p.push(format!("{who}: lcd_columns {} > columns {}", self.lcd_columns, self.grid.columns));
        }
        let mut seen = self.display_to_device.clone();
        seen.sort_unstable();
        let want: Vec<u8> = (0..self.raw_keys).filter_map(|i| u8::try_from(i).ok()).collect();
        if seen != want || self.display_to_device.len() != self.raw_keys as usize {
            p.push(format!("{who}: display_to_device must hold each of 0…{} once", self.raw_keys.saturating_sub(1)));
        }
        if self.image.width == 0 || self.image.height == 0 || self.image.width > 1024 || self.image.height > 1024 {
            p.push(format!("{who}: image size {}×{}", self.image.width, self.image.height));
        }
        if ![0, 90, 180, 270].contains(&self.image.rotation) {
            p.push(format!("{who}: rotation {} is not 0, 90, 180 or 270", self.image.rotation));
        }
        p
    }
}

/// Parses profiles and refuses any that is not usable.
pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    let file: File = toml::from_str(text).map_err(|e| format!("mirabox profiles: {e}"))?;
    let mut problems: Vec<String> = file.profiles.iter().flat_map(Profile::problems).collect();
    let mut ids: Vec<(u16, u16)> = file.profiles.iter().map(|p| (p.vid, p.pid)).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        problems.push("two profiles share a USB id".into());
    }
    if problems.is_empty() { Ok(file.profiles) } else { Err(problems.join("; ")) }
}

/// The shipped profiles.
pub fn builtin() -> Result<Vec<Profile>, String> {
    parse(BUILTIN)
}
