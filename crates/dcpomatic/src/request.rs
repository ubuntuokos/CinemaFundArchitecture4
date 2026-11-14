//! What to make, checked and turned into the two argument arrays before anything starts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::DcpomaticError;

/// The ISDCF content kind (`dcpomatic2_create -c`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    Feature,
    Short,
    Trailer,
    #[default]
    Test,
    Transitional,
    Rating,
    Teaser,
    Policy,
    Psa,
    Advertisement,
}

impl ContentKind {
    pub fn isdcf(self) -> &'static str {
        match self {
            ContentKind::Feature => "FTR",
            ContentKind::Short => "SHR",
            ContentKind::Trailer => "TLR",
            ContentKind::Test => "TST",
            ContentKind::Transitional => "XSN",
            ContentKind::Rating => "RTG",
            ContentKind::Teaser => "TSR",
            ContentKind::Policy => "POL",
            ContentKind::Psa => "PSA",
            ContentKind::Advertisement => "ADV",
        }
    }
}

/// SMPTE (the default) or the older Interop packaging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Standard {
    #[default]
    Smpte,
    Interop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    #[serde(rename = "2k")]
    TwoK,
    #[serde(rename = "4k")]
    FourK,
}

/// The DCP container. DCP-o-matic 2.18.51 has no 1.90 (full frame) container: `--container-ratio`
/// takes only the ids below, and it silently ignores any other value (DCPOM-01 audit §3), so the
/// list is closed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Container {
    /// 1.85 (DCI Flat).
    Flat,
    /// 2.39 (DCI Scope).
    Scope,
    /// 1.19.
    #[serde(rename = "1.19")]
    R119,
    /// 1.33 (4:3).
    #[serde(rename = "1.33")]
    R133,
    /// 1.37 (Academy).
    #[serde(rename = "1.37")]
    R137,
    /// 1.38.
    #[serde(rename = "1.38")]
    R138,
    /// 1.66.
    #[serde(rename = "1.66")]
    R166,
    /// 1.78 (16:9).
    #[serde(rename = "1.78")]
    R178,
}

impl Container {
    /// DCP-o-matic's ratio id (`--container-ratio`).
    pub fn ratio_id(self) -> &'static str {
        match self {
            Container::Flat => "185",
            Container::Scope => "239",
            Container::R119 => "119",
            Container::R133 => "133",
            Container::R137 => "137",
            Container::R138 => "138",
            Container::R166 => "166",
            Container::R178 => "178",
        }
    }
}

/// The eye a piece of content is for, in a 3D DCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Eye {
    Left,
    Right,
}

/// The DCP channel a piece of audio-only content goes to (`--channel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    L,
    R,
    C,
    Lfe,
    Ls,
    Rs,
    BsL,
    BsR,
    #[serde(rename = "HI")]
    Hi,
    #[serde(rename = "VI")]
    Vi,
}

impl Channel {
    fn arg(self) -> &'static str {
        match self {
            Channel::L => "L",
            Channel::R => "R",
            Channel::C => "C",
            Channel::Lfe => "Lfe",
            Channel::Ls => "Ls",
            Channel::Rs => "Rs",
            Channel::BsL => "BsL",
            Channel::BsR => "BsR",
            Channel::Hi => "HI",
            Channel::Vi => "VI",
        }
    }

    /// The channel's index (DCP-o-matic needs at least index + 1 channels).
    fn index(self) -> u32 {
        match self {
            Channel::L => 0,
            Channel::R => 1,
            Channel::C => 2,
            Channel::Lfe => 3,
            Channel::Ls => 4,
            Channel::Rs => 5,
            Channel::Hi => 6,
            Channel::Vi => 7,
            Channel::BsL => 10,
            Channel::BsR => 11,
        }
    }
}

/// One content file (or image-sequence folder), in playback order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub path: PathBuf,
    /// For a 3D DCP: the eye this content is for (left and right come as separate files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eye: Option<Eye>,
    /// For mono audio files: the channel they go to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<Channel>,
    /// Audio gain in dB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
}

impl Content {
    pub fn new(path: impl Into<PathBuf>) -> Content {
        Content { path: path.into(), eye: None, channel: None, gain_db: None }
    }
}

/// The frame rates DCP-o-matic offers for DCPs.
pub const FRAME_RATES: &[u32] = &[24, 25, 30, 48, 50, 60];

/// A DCP to make.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DcpRequest {
    pub content: Vec<Content>,
    /// The DCP-o-matic film directory to create (must not exist); the DCP is written inside it.
    pub film_dir: PathBuf,
    /// The film name; the DCP folder gets the ISDCF name built from it unless `isdcf_name` is off.
    pub name: String,
    #[serde(default)]
    pub content_kind: ContentKind,
    #[serde(default)]
    pub standard: Standard,
    /// Unset: DCP-o-matic picks from the content.
    #[serde(default)]
    pub resolution: Option<Resolution>,
    #[serde(default)]
    pub container: Option<Container>,
    /// Unset: guessed from the content.
    #[serde(default)]
    pub frame_rate: Option<u32>,
    /// Even, 2..=16. Unset: 6, or more when a content channel needs it.
    #[serde(default)]
    pub audio_channels: Option<u32>,
    /// JPEG 2000 bit rate in Mbit/s (at least 10).
    #[serde(default)]
    pub video_bit_rate_mbps: Option<u32>,
    #[serde(default)]
    pub three_d: bool,
    /// Encrypt the track files (ST 429-6). The content keys stay in the film directory's
    /// `metadata.xml`, so keep that directory: KDMs ([`crate::KdmRequest`]) are made from it.
    #[serde(default)]
    pub encrypt: bool,
    /// At most one subtitle file (`.srt`, `.ssa`, `.ass`, or DCP subtitle `.xml`): a subtitle track
    /// of the DCP from the start of the composition, not burnt into the picture (D5). More are
    /// refused, since `dcpomatic2_create` places each one after the end of the previous one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitles: Vec<PathBuf>,
    /// `false`: the folder is named exactly `name`.
    #[serde(default = "yes")]
    pub isdcf_name: bool,
    /// Local encoding threads (unset: DCP-o-matic's configuration).
    #[serde(default)]
    pub threads: Option<u32>,
    /// DCP-o-matic's configuration directory (`config.xml`, the signing certificate chain).
    /// Unset: `${XDG_CONFIG_HOME:-~/.config}/cfa4/dcpomatic`, so packages are signed with the
    /// same chain every time and the user's own DCP-o-matic settings are not touched.
    #[serde(default)]
    pub config_dir: Option<PathBuf>,
}

fn yes() -> bool {
    true
}

impl DcpRequest {
    pub fn new(content: Vec<Content>, film_dir: impl Into<PathBuf>, name: impl Into<String>) -> DcpRequest {
        DcpRequest {
            content,
            film_dir: film_dir.into(),
            name: name.into(),
            content_kind: ContentKind::default(),
            standard: Standard::default(),
            resolution: None,
            container: None,
            frame_rate: None,
            audio_channels: None,
            video_bit_rate_mbps: None,
            three_d: false,
            encrypt: false,
            subtitles: Vec::new(),
            isdcf_name: true,
            threads: None,
            config_dir: None,
        }
    }

    /// Checks the request and builds both argument arrays.
    pub(crate) fn plan(&self) -> Result<Plan, DcpomaticError> {
        let bad = |m: String| Err(DcpomaticError::Invalid(m));
        if self.content.is_empty() {
            return bad("no content files".into());
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().any(char::is_control) || name.contains('/') {
            return bad(format!("the name {:?} is empty or has a slash or control character", self.name));
        }
        if self.film_dir.as_os_str().is_empty() {
            return bad("no film directory".into());
        }
        let film_dir = absolute(&self.film_dir)?;
        if film_dir.exists() {
            return bad(format!("{} exists already; DCP-o-matic needs a new film directory", film_dir.display()));
        }
        match film_dir.parent() {
            Some(p) if p.is_dir() => {}
            _ => return bad(format!("the parent folder of {} does not exist", film_dir.display())),
        }
        if let Some(r) = self.frame_rate
            && !FRAME_RATES.contains(&r)
        {
            return bad(format!("{r} fps is not a DCP frame rate ({FRAME_RATES:?})"));
        }
        if let Some(b) = self.video_bit_rate_mbps
            && !(10..=1000).contains(&b)
        {
            return bad(format!("the video bit rate must be at least 10 Mbit/s, not {b}"));
        }
        if self.threads == Some(0) {
            return bad("threads must be at least 1".into());
        }
        let needed = self.content.iter().filter_map(|c| c.channel).map(|c| c.index() + 1).max().unwrap_or(0);
        if let Some(a) = self.audio_channels {
            if !(2..=16).contains(&a) || a % 2 != 0 {
                return bad(format!("audio channels must be even, 2 to 16, not {a}"));
            }
            if a < needed {
                return bad(format!("{a} audio channels cannot hold the requested channel mapping (needs {needed})"));
            }
        }
        let eyes = self.content.iter().filter(|c| c.eye.is_some()).count();
        if eyes > 0 && !self.three_d {
            return bad("a content file has an eye, but the DCP is not 3D".into());
        }

        let config_dir = match &self.config_dir {
            Some(d) => Some(absolute(d)?),
            None => default_config_dir(),
        };

        let mut create: Vec<String> = Vec::new();
        let mut push = |a: &str| create.push(a.to_string());
        if let Some(cfg) = &config_dir {
            push("--config");
            push(&text(cfg)?);
        }
        push("--output");
        push(&text(&film_dir)?);
        push("--name");
        push(name);
        push(if self.encrypt { "--encrypt" } else { "--no-encrypt" });
        push("--dcp-content-type");
        push(self.content_kind.isdcf());
        push("--standard");
        push(match self.standard {
            Standard::Smpte => "SMPTE",
            Standard::Interop => "interop",
        });
        if !self.isdcf_name {
            push("--no-use-isdcf-name");
        }
        match self.resolution {
            Some(Resolution::TwoK) => push("--twok"),
            Some(Resolution::FourK) => push("--fourk"),
            None => {}
        }
        if let Some(c) = self.container {
            push("--container-ratio");
            push(c.ratio_id());
        }
        if let Some(r) = self.frame_rate {
            push("--dcp-frame-rate");
            push(&r.to_string());
        }
        if let Some(a) = self.audio_channels {
            push("--audio-channels");
            push(&a.to_string());
        }
        if let Some(b) = self.video_bit_rate_mbps {
            push("--video-bit-rate");
            push(&b.to_string());
        }
        push(if self.three_d { "--threed" } else { "--twod" });
        for c in &self.content {
            let path = absolute(&c.path)?;
            if !path.exists() {
                return bad(format!("content {} does not exist", path.display()));
            }
            match c.eye {
                Some(Eye::Left) => push("--left-eye"),
                Some(Eye::Right) => push("--right-eye"),
                None => {}
            }
            if let Some(ch) = c.channel {
                push("--channel");
                push(ch.arg());
            }
            if let Some(g) = c.gain_db {
                if !g.is_finite() {
                    return bad(format!("the gain of {} is not a number", path.display()));
                }
                push("--gain");
                push(&g.to_string());
            }
            // absolute, so it starts with `/` and is never read as an option
            push(&text(&path)?);
        }
        if self.subtitles.len() > 1 {
            return bad("only one subtitle file is supported (dcpomatic2_create would place each after the end of the previous one)".into());
        }
        for s in &self.subtitles {
            let path = absolute(s)?;
            let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("srt" | "ssa" | "ass" | "xml")) {
                return bad(format!("subtitle {} is not .srt, .ssa, .ass or DCP subtitle .xml", path.display()));
            }
            if !path.is_file() {
                return bad(format!("subtitle {} does not exist", path.display()));
            }
            push(&text(&path)?);
        }

        let mut cli: Vec<String> = Vec::new();
        if let Some(cfg) = &config_dir {
            cli.push("--config".into());
            cli.push(text(cfg)?);
        }
        // never send frames to network encoding servers the configuration may list
        cli.push("--no-remote".into());
        if let Some(t) = self.threads {
            cli.push("--threads".into());
            cli.push(t.to_string());
        }
        cli.push("make-dcp".into());
        cli.push(text(&film_dir)?);

        Ok(Plan { film_dir, config_dir, create_args: create, cli_args: cli, encrypt: self.encrypt })
    }
}

/// The checked request.
#[derive(Debug)]
pub(crate) struct Plan {
    pub film_dir: PathBuf,
    pub config_dir: Option<PathBuf>,
    pub create_args: Vec<String>,
    pub cli_args: Vec<String>,
    pub encrypt: bool,
}

/// Puts `value` after `flag` in `args` (the flag is there: the request built it).
pub(crate) fn set_value(args: &mut [String], flag: &str, value: String) {
    if let Some(i) = args.iter().position(|a| a == flag)
        && let Some(v) = args.get_mut(i + 1)
    {
        *v = value;
    }
}

pub(crate) fn absolute(p: &Path) -> Result<PathBuf, DcpomaticError> {
    std::path::absolute(p).map_err(|e| DcpomaticError::Invalid(format!("{}: {e}", p.display())))
}

/// DCP-o-matic takes paths as text; a path that is not UTF-8 is refused instead of mangled.
pub(crate) fn text(p: &Path) -> Result<String, DcpomaticError> {
    p.to_str().map(str::to_string).ok_or_else(|| DcpomaticError::Invalid(format!("{} is not a UTF-8 path", p.display())))
}

pub(crate) fn default_config_dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config"))).map(|c| c.join("cfa4/dcpomatic"))
}
