//! The GStreamer-backed implementation (feature `gstreamer`). Safe code only: everything goes
//! through the gstreamer-rs bindings.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_pbutils as pbutils;
use gstreamer_pbutils::prelude::*;

use crate::{CATALOGUE, ElementStatus, Error, MediaInfo, Status, StreamInfo, StreamKind, frame_rate, ticks_from_nanos};

/// Initialises GStreamer once per process; later calls return the first outcome.
pub(crate) fn init() -> Result<(), Error> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| gst::init().map_err(|e| e.to_string())).clone().map_err(|e| Error::Unavailable(format!("initialisation failed: {e}")))
}

pub(crate) fn probe() -> Result<Status, Error> {
    init()?;
    let (major, minor, micro, nano) = gst::version();
    let version = if nano == 0 { format!("{major}.{minor}.{micro}") } else { format!("{major}.{minor}.{micro}.{nano}") };
    let elements = CATALOGUE
        .iter()
        .map(|spec| {
            let factory = gst::ElementFactory::find(spec.factory);
            let plugin = factory.as_ref().and_then(|f| f.plugin());
            ElementStatus {
                spec: *spec,
                available: factory.is_some(),
                plugin: plugin.as_ref().map(|p| p.plugin_name().to_string()),
                license: plugin.as_ref().map(|p| p.license().to_string()),
                package: plugin.as_ref().map(|p| p.package().to_string()),
            }
        })
        .collect();
    Ok(Status { version, elements })
}

pub(crate) fn discover(path: &Path, timeout: Duration) -> Result<MediaInfo, Error> {
    init()?;
    let uri = gst::glib::filename_to_uri(path, None).map_err(|e| Error::InvalidInput(format!("{}: {e}", path.display())))?.to_string();
    let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
    let discoverer =
        pbutils::Discoverer::new(gst::ClockTime::from_nseconds(nanos)).map_err(|e| Error::Unavailable(format!("cannot create a discoverer: {e}")))?;
    let info = discoverer.discover_uri(&uri).map_err(|e| Error::Discover { message: e.to_string(), missing_plugins: Vec::new() })?;

    match info.result() {
        pbutils::DiscovererResult::Ok => {}
        pbutils::DiscovererResult::MissingPlugins => {
            let missing = info.missing_elements_installer_details().iter().map(|s| s.to_string()).collect();
            return Err(Error::Discover { message: "required GStreamer plugins are not installed".into(), missing_plugins: missing });
        }
        other => {
            return Err(Error::Discover { message: format!("discovery ended with {other:?}"), missing_plugins: Vec::new() });
        }
    }

    let container =
        info.stream_info().and_then(|top| top.downcast::<pbutils::DiscovererContainerInfo>().ok()).and_then(|c| c.caps()).map(|caps| codec_description(&caps));

    let mut streams = Vec::new();
    for v in info.video_streams() {
        let rate = v.framerate();
        streams.push(StreamInfo {
            kind: StreamKind::Video {
                width: v.width(),
                height: v.height(),
                frame_rate: if v.is_image() { None } else { frame_rate(rate.numer(), rate.denom()) },
                depth: v.depth(),
                par: {
                    let par = v.par();
                    match (u32::try_from(par.numer()), u32::try_from(par.denom())) {
                        (Ok(n), Ok(d)) if n > 0 && d > 0 => Some((n, d)),
                        _ => None,
                    }
                },
                is_image: v.is_image(),
            },
            codec: v.caps().map(|c| codec_description(&c)),
            caps: v.caps().map(|c| c.to_string()),
        });
    }
    for a in info.audio_streams() {
        streams.push(StreamInfo {
            kind: StreamKind::Audio { channels: a.channels(), sample_rate: a.sample_rate(), language: a.language().map(|l| l.to_string()) },
            codec: a.caps().map(|c| codec_description(&c)),
            caps: a.caps().map(|c| c.to_string()),
        });
    }
    for s in info.subtitle_streams() {
        streams.push(StreamInfo {
            kind: StreamKind::Subtitle { language: s.language().map(|l| l.to_string()) },
            codec: s.caps().map(|c| codec_description(&c)),
            caps: s.caps().map(|c| c.to_string()),
        });
    }

    Ok(MediaInfo { uri, container, duration: info.duration().map(|d| ticks_from_nanos(d.nseconds())), seekable: info.is_seekable(), streams })
}

/// GStreamer's human-readable codec / container name for `caps`, or the caps' media type.
fn codec_description(caps: &gst::Caps) -> String {
    let described = pbutils::pb_utils_get_codec_description(caps).to_string();
    if !described.is_empty() {
        return described;
    }
    caps.structure(0).map(|s| s.name().to_string()).unwrap_or_default()
}

/// `Ok` when GStreamer is initialised and the element factory `name` is installed.
pub(crate) fn require_element(name: &str) -> Result<(), Error> {
    init()?;
    if gst::ElementFactory::find(name).is_some() { Ok(()) } else { Err(Error::Unavailable(format!("the GStreamer element `{name}` is not installed"))) }
}
