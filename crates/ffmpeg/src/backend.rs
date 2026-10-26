//! What the linked FFmpeg is ([`probe`]) and what a file holds ([`discover`]), through rsmedia's
//! safe API. Shared by the media source.

use std::ffi::CString;
use std::path::Path;

use cfa4_time::Tick;
use rsmedia::stream::StreamInfo as FfStream;
use rsmedia::{MediaType, Reader, StreamReader};

use crate::{Error, MediaInfo, RESTRICTED, RestrictedStatus, Status, StreamInfo, StreamKind, status_from};

/// FFmpeg's `AV_TIME_BASE`: container durations and start times are counted in microseconds.
pub(crate) const AV_TIME_BASE: i64 = 1_000_000;
/// FFmpeg's `AV_NOPTS_VALUE`.
pub(crate) const NOPTS: i64 = i64::MIN;
/// Most streams looked at in one file (the count comes from the file and is never trusted).
const MAX_STREAMS: usize = 256;

pub(crate) fn probe() -> Status {
    let version = rsmedia::avutil::version_info().to_string_lossy().into_owned();
    let license = rsmedia::avutil::license().to_string_lossy().into_owned();
    let configuration = rsmedia::avutil::configuration().to_string_lossy().into_owned();
    let hwaccels = rsmedia::avutil::hwdevice_iterate_types()
        .filter_map(|t| rsmedia::avutil::hwdevice_get_type_name(t).map(|n| n.to_string_lossy().into_owned()))
        .take(64)
        .collect();
    let restricted =
        RESTRICTED.iter().map(|r| RestrictedStatus { encoder: r.encoder.into(), purpose: r.purpose.into(), available: has_encoder(r.encoder) }).collect();
    status_from(version, license, configuration, hwaccels, restricted)
}

fn has_encoder(name: &str) -> bool {
    CString::new(name).ok().is_some_and(|n| rsmpeg::avcodec::AVCodec::find_encoder_by_name(&n).is_some())
}

/// The container and its streams, as both [`discover`] and the media source need them.
pub(crate) struct Probed {
    pub(crate) container: String,
    /// Container duration, when known.
    pub(crate) duration: Option<Tick>,
    /// Container start time in microseconds (the origin of media time), 0 when unknown.
    pub(crate) start_us: i64,
    pub(crate) streams: Vec<FfStream>,
}

pub(crate) fn open_reader(path: &Path) -> Result<StreamReader, Error> {
    let text = path.to_str().ok_or_else(|| Error::InvalidInput("the path is not valid UTF-8".into()))?;
    StreamReader::new(text).map_err(|e| Error::Media(e.to_string()))
}

pub(crate) fn probe_file(path: &Path) -> Result<(StreamReader, Probed), Error> {
    let reader = open_reader(path)?;
    let input = reader.input();
    let container = input.iformat().name().to_string_lossy().into_owned();
    let duration = (input.duration > 0 && input.duration != NOPTS).then(|| Tick::from_units(input.duration, AV_TIME_BASE));
    let start_us = if input.start_time == NOPTS { 0 } else { input.start_time };
    let count = usize::try_from(input.nb_streams).unwrap_or(0).min(MAX_STREAMS);
    let mut streams = Vec::with_capacity(count);
    for index in 0..count {
        match FfStream::from_reader(&reader, index) {
            Ok(s) => streams.push(s),
            Err(e) => log::debug!("{}: stream {index} skipped: {e}", path.display()),
        }
    }
    Ok((reader, Probed { container, duration, start_us, streams }))
}

pub(crate) fn codec_name(s: &FfStream) -> String {
    rsmpeg::avcodec::AVCodec::find_decoder(s.codec_id).map(|c| c.name().to_string_lossy().into_owned()).unwrap_or_else(|| format!("codec {}", s.codec_id))
}

pub(crate) fn pixel_format_name(s: &FfStream) -> String {
    s.format.into_pixel().map(|p| p.get_pix_fmt_name()).unwrap_or_else(|| "unknown".into())
}

/// A positive rational, or `None`.
pub(crate) fn rational(num: i32, den: i32) -> Option<(i64, i64)> {
    (num > 0 && den > 0).then_some((i64::from(num), i64::from(den)))
}

pub(crate) fn discover(path: &Path) -> Result<MediaInfo, Error> {
    let (_reader, probed) = probe_file(path)?;
    let streams = probed
        .streams
        .iter()
        .map(|s| {
            let kind = if s.media_type == MediaType::VIDEO {
                StreamKind::Video {
                    width: u32::try_from(s.width).unwrap_or(0),
                    height: u32::try_from(s.height).unwrap_or(0),
                    pixel_format: pixel_format_name(s),
                    frame_rate: rational(s.avg_frame_rate.num, s.avg_frame_rate.den).or_else(|| rational(s.frame_rate.num, s.frame_rate.den)),
                }
            } else if s.media_type == MediaType::AUDIO {
                StreamKind::Audio { channels: u32::try_from(s.channel_layout.nb_channels).unwrap_or(0), sample_rate: u32::try_from(s.sample_rate).unwrap_or(0) }
            } else if s.media_type == MediaType::SUBTITLE {
                StreamKind::Subtitle
            } else {
                StreamKind::Other
            };
            StreamInfo { index: s.index, kind, codec: codec_name(s) }
        })
        .collect();
    Ok(MediaInfo { container: probed.container, duration: probed.duration, streams })
}
