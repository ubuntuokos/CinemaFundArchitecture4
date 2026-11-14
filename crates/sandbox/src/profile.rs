//! The profile of every CFA4 process kind (plan IV.2), as data.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The CFA4 process kinds of plan IV.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessKind {
    /// The Qt 6 GUI (Python).
    QtGui,
    /// The engine (`cfa4-cli serve`).
    Engine,
    /// `cfa4-media-host`: FFmpeg and GStreamer, one opened file (D2, phase B4).
    MediaHost,
    /// `cfa4-studio-host`: the live studio (LIVE-STUDIO D2, phase B6).
    StudioHost,
    /// The browser source inside the studio host: confined by Chromium's sandbox and AppArmor.
    BrowserSource,
    /// `cfa4-fetch`: downloads the user asked for (D5).
    Fetch,
    /// A native plug-in (OFX, LV2/VST3…) in its own process (D7).
    PluginHost,
    /// The one-shot pickle → safetensors model converter (D7).
    ModelConverter,
}

impl ProcessKind {
    /// Every kind, in the order of plan IV.2.
    pub const ALL: [ProcessKind; 8] = [
        ProcessKind::QtGui,
        ProcessKind::Engine,
        ProcessKind::MediaHost,
        ProcessKind::StudioHost,
        ProcessKind::BrowserSource,
        ProcessKind::Fetch,
        ProcessKind::PluginHost,
        ProcessKind::ModelConverter,
    ];

    /// Stable id (serde name), for `security.status` and logs.
    pub fn id(self) -> &'static str {
        match self {
            ProcessKind::QtGui => "qtGui",
            ProcessKind::Engine => "engine",
            ProcessKind::MediaHost => "mediaHost",
            ProcessKind::StudioHost => "studioHost",
            ProcessKind::BrowserSource => "browserSource",
            ProcessKind::Fetch => "fetch",
            ProcessKind::PluginHost => "pluginHost",
            ProcessKind::ModelConverter => "modelConverter",
        }
    }

    /// English label for Settings ▸ Security.
    pub fn label(self) -> &'static str {
        match self {
            ProcessKind::QtGui => "Qt GUI",
            ProcessKind::Engine => "Engine",
            ProcessKind::MediaHost => "Media host (FFmpeg, GStreamer)",
            ProcessKind::StudioHost => "Live studio host",
            ProcessKind::BrowserSource => "Browser source",
            ProcessKind::Fetch => "Downloader",
            ProcessKind::PluginHost => "Plug-in host",
            ProcessKind::ModelConverter => "Model converter",
        }
    }

    /// Parse an id from [`ProcessKind::id`].
    pub fn from_id(id: &str) -> Option<ProcessKind> {
        ProcessKind::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// What network a process may use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Network {
    /// No IPv4/IPv6 sockets at all (seccomp) and no TCP ports (Landlock ABI ≥ 4). Unix-domain sockets
    /// stay available for the channel to the parent.
    None,
    /// Bind and connect only these TCP ports (meant for 127.0.0.1: the control channel, MCP, the live
    /// page). Landlock rules are per port, not per address; binding to loopback only is enforced in code
    /// and by the Semgrep rule of `cfa4-security.yml`.
    Loopback { ports: Vec<u16> },
    /// Connect (not bind) only to these TCP ports (the downloader: HTTPS).
    Outbound { ports: Vec<u16> },
    /// Unrestricted (the live studio's streaming targets and network inputs).
    Any,
}

impl Network {
    /// Whether enforcing this needs Landlock's TCP port rules (ABI ≥ 4).
    pub fn port_limited(&self) -> bool {
        matches!(self, Network::None | Network::Loopback { .. } | Network::Outbound { .. })
    }
}

/// The paths and ports a profile is built from. Every field is optional: a missing directory simply
/// grants nothing. Paths are used as given (a rule on a path covers everything beneath it).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Context {
    /// Directories of the open project (read-write for the engine, read for the GUI).
    pub project_dirs: Vec<PathBuf>,
    /// Directories or files of the media the user added to the project (read).
    pub media: Vec<PathBuf>,
    /// The one media file a media host opens (read).
    pub media_file: Option<PathBuf>,
    /// The cache directory (read-write: proxies, previews, peaks).
    pub cache_dir: Option<PathBuf>,
    /// The settings directory (read-write).
    pub config_dir: Option<PathBuf>,
    /// Export targets (files, read-write: see [`Profile::outputs`]).
    pub export_targets: Vec<PathBuf>,
    /// Program files of this process: the Python virtual environment, `/opt/cfa4` (read).
    pub app_dirs: Vec<PathBuf>,
    /// Extra font directories (`CFA4_FONTS_DIR`, `~/.local/share/fonts`) (read).
    pub font_dirs: Vec<PathBuf>,
    /// `$XDG_RUNTIME_DIR` (Wayland, PipeWire, D-Bus sockets) for processes with a display or audio.
    pub runtime_dir: Option<PathBuf>,
    /// The helper programs this process may start (the engine for the GUI; `cfa4-media-host`,
    /// `cfa4-fetch` for the engine; the Chromium helper for the studio host). Empty: no program start.
    pub helper_exes: Vec<PathBuf>,
    /// Local TCP ports of the control channel, MCP and the live page.
    pub loopback_ports: Vec<u16>,
    /// Live studio: the scene's sources (read) and the recording target (a file, read-write).
    pub studio_sources: Vec<PathBuf>,
    pub recording_target: Option<PathBuf>,
    /// Downloader: the target directory (read-write) and the ports it may connect to (default 443).
    pub download_target: Option<PathBuf>,
    pub fetch_ports: Vec<u16>,
    /// Plug-in host: the plug-in's own files (read).
    pub plugin_dirs: Vec<PathBuf>,
    /// Model converter: the source model (read) and the safetensors output (a file, read-write).
    pub model_source: Option<PathBuf>,
    pub model_output: Option<PathBuf>,
}

/// One process's rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub kind: ProcessKind,
    /// Read (files and directory listings) beneath these paths, on top of [`SYSTEM_READ`].
    pub read: Vec<PathBuf>,
    /// Read and write beneath these directories (create, remove, truncate). One that does not exist
    /// grants nothing.
    pub write: Vec<PathBuf>,
    /// Single files this process writes (an export, a recording, a model): one that does not exist
    /// yet is created empty just before confinement, and only the file itself is granted, never its
    /// directory.
    pub outputs: Vec<PathBuf>,
    /// The only programs this process may start. Empty: `execve` is refused (seccomp).
    pub exec: Vec<PathBuf>,
    pub network: Network,
    /// GPU device nodes (`/dev/dri`, `/dev/nvidia*`) read-write with device ioctls.
    pub gpu: bool,
}

/// System locations every process reads: libraries, configuration, locale, fonts, CPU/GPU topology.
/// Not `$HOME`, not other processes' `/proc` entries.
pub const SYSTEM_READ: &[&str] = &[
    "/usr",
    "/lib",
    "/lib64",
    "/lib32",
    "/etc",
    "/opt/cfa4",
    "/sys",
    "/proc/self",
    "/proc/thread-self",
    "/proc/cpuinfo",
    "/proc/meminfo",
    "/proc/stat",
    "/proc/filesystems",
    "/proc/sys",
    "/var/cache/fontconfig",
];

/// Device nodes every process reads and writes.
pub const SYSTEM_DEVICES: &[&str] = &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom", "/dev/full"];

/// GPU device nodes (directories and the prefix of the NVIDIA nodes in `/dev`).
pub const GPU_DEVICES: &[&str] = &["/dev/dri", "/dev/nvidia-caps", "/dev/nvidia-uvm", "/dev/nvidia-uvm-tools", "/dev/nvidiactl", "/dev/nvidia-modeset"];

fn some(p: &Option<PathBuf>) -> Vec<PathBuf> {
    p.iter().cloned().collect()
}

impl Profile {
    /// The profile of `kind` (plan IV.2), filled with the paths of `ctx`.
    pub fn for_process(kind: ProcessKind, ctx: &Context) -> Profile {
        let mut read = ctx.app_dirs.clone();
        let mut write = Vec::new();
        let mut outputs = Vec::new();
        let mut exec = Vec::new();
        let (network, gpu) = match kind {
            ProcessKind::QtGui => {
                read.extend(ctx.project_dirs.iter().cloned());
                read.extend(ctx.font_dirs.iter().cloned());
                write.extend(some(&ctx.config_dir));
                write.extend(some(&ctx.runtime_dir));
                exec.extend(ctx.helper_exes.iter().cloned());
                (Network::Loopback { ports: ctx.loopback_ports.clone() }, true)
            }
            ProcessKind::Engine => {
                read.extend(ctx.media.iter().cloned());
                read.extend(ctx.font_dirs.iter().cloned());
                write.extend(ctx.project_dirs.iter().cloned());
                write.extend(some(&ctx.cache_dir));
                // the engine stores preferences (`prefs.*` commands)
                write.extend(some(&ctx.config_dir));
                outputs.extend(ctx.export_targets.iter().cloned());
                write.extend(some(&ctx.runtime_dir));
                exec.extend(ctx.helper_exes.iter().cloned());
                (Network::Loopback { ports: ctx.loopback_ports.clone() }, true)
            }
            ProcessKind::MediaHost => {
                // only the opened file; frames go back over shared memory passed as a descriptor
                read.extend(some(&ctx.media_file));
                (Network::None, true)
            }
            ProcessKind::StudioHost => {
                read.extend(ctx.studio_sources.iter().cloned());
                read.extend(ctx.font_dirs.iter().cloned());
                outputs.extend(some(&ctx.recording_target));
                write.extend(some(&ctx.runtime_dir));
                exec.extend(ctx.helper_exes.iter().cloned());
                (Network::Any, true)
            }
            ProcessKind::BrowserSource => (Network::Any, true),
            ProcessKind::Fetch => {
                write.extend(some(&ctx.download_target));
                let ports = if ctx.fetch_ports.is_empty() { vec![443] } else { ctx.fetch_ports.clone() };
                (Network::Outbound { ports }, false)
            }
            ProcessKind::PluginHost => {
                read.extend(ctx.plugin_dirs.iter().cloned());
                (Network::None, true)
            }
            ProcessKind::ModelConverter => {
                read.extend(some(&ctx.model_source));
                outputs.extend(some(&ctx.model_output));
                (Network::None, false)
            }
        };
        Profile { kind, read, write, outputs, exec, network, gpu }
    }

    /// Whether `path` falls under one of this profile's own read or write paths (not the system ones).
    /// Lexical: does not resolve symbolic links (the kernel rules do).
    pub fn covers(&self, path: &Path) -> bool {
        self.read.iter().chain(&self.write).any(|p| path.starts_with(p)) || self.outputs.iter().any(|p| path == p)
    }
}
