//! Read LTC timecode recorded on a clip's audio (plan TC-7).
//!
//! Cameras and recorders often get SMPTE LTC from a sync box on one audio channel. This finds it,
//! works out the timecode of the media's first frame, and sets it as the clip's timecode
//! (Clip ▸ Modify ▸ Timecode, undoable). Multi-camera sync by timecode then lines the clips up.
//!
//! | Command | |
//! |---|---|
//! | `clip.readLtcTimecode` | `{item?, stream?, seconds?, apply?}` → `{item, channel, timecode, startTimecode, at, applied}` |

use cfa4_tcsync::ltc::{LtcDecoded, LtcDecoder};
use cfa4_time::{FrameRate, Tick, format_timecode_frames};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, item_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "clip.readLtcTimecode";
/// Sample rate the audio is read at.
const SR: u32 = 48_000;
/// Consecutive frames that must agree before the timecode is trusted.
const AGREE: usize = 3;

/// LTC found on one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LtcFound {
    pub channel: usize,
    /// Timecode (frame count at the LTC rate) of the media's first frame, wrapped into one day.
    pub start_frame: i64,
    /// Media time (samples at [`SR`]) where the first trusted frame began.
    pub at_sample: u64,
    pub drop_frame: bool,
}

/// The LTC rate for media at `rate`: rates above 30 fps carry LTC at half the rate (50p → 25).
pub fn ltc_rate(rate: FrameRate) -> FrameRate {
    let r = rate.sane();
    if r.timecode_base() <= 30 {
        return r;
    }
    let half = if r.num % 2 == 0 { FrameRate { num: r.num / 2, den: r.den } } else { FrameRate { num: r.num, den: r.den.saturating_mul(2) } };
    half.sane()
}

/// Per-channel reader: frames that run on from each other, each giving the same start.
struct ChannelScan {
    dec: LtcDecoder,
    frames: Vec<LtcDecoded>,
    run: Vec<(i64, u64, bool)>,
}

impl ChannelScan {
    /// Feeds samples; returns the start once `AGREE` forward frames in a row agree.
    fn feed(&mut self, samples: &[f32], rate: FrameRate, day: i64) -> Option<(i64, u64, bool)> {
        self.frames.clear();
        self.dec.decode(samples, &mut self.frames);
        let frame = rate.frame_duration().0.max(1);
        for d in &self.frames {
            if d.reverse {
                self.run.clear();
                continue;
            }
            let n = d.frame.frame_number(rate);
            // the frame began `d.start` samples into the media: that many frames after the start
            let t = Tick::from_units(i64::try_from(d.start).unwrap_or(i64::MAX / 2), i64::from(SR));
            let k = rate.frame_at(Tick(t.0.saturating_add(frame / 2)));
            let start = (n - k).rem_euclid(day);
            let df = d.frame.drop_frame;
            if self.run.last().is_some_and(|&(s, _, _)| (s - start).rem_euclid(day).min((start - s).rem_euclid(day)) > 1) {
                self.run.clear();
            }
            self.run.push((start, d.start, df));
            if self.run.len() >= AGREE {
                return self.run.first().copied();
            }
        }
        None
    }
}

/// Scans `channels` channels of audio for LTC: `read(start, n)` gives `n` samples per channel
/// from sample `start` (at [`SR`]), `None` at the end. Stops at `limit` samples.
pub fn scan(rate: FrameRate, drop_frame: bool, channels: usize, limit: u64, mut read: impl FnMut(u64, usize) -> Option<Vec<Vec<f32>>>) -> Option<LtcFound> {
    let rate = rate.sane();
    let day = cfa4_time::fields_to_frames(24, 0, 0, 0, rate, drop_frame && rate.supports_drop_frame()).max(1);
    let mut scans: Vec<ChannelScan> =
        (0..channels.clamp(1, 64)).filter_map(|_| LtcDecoder::new(SR).ok().map(|dec| ChannelScan { dec, frames: Vec::new(), run: Vec::new() })).collect();
    let chunk = SR as usize;
    let mut at = 0u64;
    while at < limit {
        let n = chunk.min(usize::try_from(limit - at).unwrap_or(chunk));
        let block = read(at, n)?;
        if block.iter().all(Vec::is_empty) {
            return None;
        }
        for (channel, (scan, samples)) in scans.iter_mut().zip(&block).enumerate() {
            if let Some((start_frame, at_sample, drop_frame)) = scan.feed(samples, rate, day) {
                return Some(LtcFound { channel, start_frame, at_sample, drop_frame });
            }
        }
        at += n as u64;
    }
    None
}

fn read_ltc(s: &mut Session, p: &Value) -> Result<Value> {
    let item = item_p(p, "item")
        .or_else(|| s.state.project_selection.iter().copied().find(|i| s.project.item(*i).is_some_and(|it| it.as_media().is_some() && it.has_audio())))
        .ok_or_else(|| bad(CMD, "select a clip with audio"))?;
    let it = s.project.item(item).ok_or_else(|| bad(CMD, "no such item"))?;
    let m = it.as_media().ok_or_else(|| bad(CMD, "not a media clip"))?;
    if !it.has_audio() {
        return Err(bad(CMD, "the clip has no audio"));
    }
    // the media's own rate; audio-only media take the sequence's
    let media_rate = if it.has_video() { m.frame_rate() } else { s.sequence_rate() };
    let media_rate = media_rate.sane();
    let drop_frame = s.active_sequence().is_some_and(|q| q.settings.drop_frame);
    let rate = ltc_rate(media_rate);
    let stream = match p.get("stream") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < 256).ok_or_else(|| bad(CMD, "`stream` must be an audio stream index"))?,
    };
    let seconds = match p.get("seconds") {
        None | Some(Value::Null) => 60.0,
        Some(v) => v.as_f64().filter(|x| x.is_finite() && *x > 0.0).ok_or_else(|| bad(CMD, "`seconds` must be a positive number"))?.min(3600.0),
    };
    let apply = bool_p(p, "apply").unwrap_or(true);
    let duration = it.duration().to_units_floor(i64::from(SR)).max(0) as u64;
    let limit = ((seconds * f64::from(SR)) as u64).min(duration.max(1));
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let src = cfa4_render::SourceProvider::source(&provider, item).ok_or_else(|| EngineError::Other("the clip's media is offline".into()))?;
    let mut error = None;
    let found = scan(rate, drop_frame, 64, limit, |start, n| match src.audio_stream(stream, i64::try_from(start).ok()?, n, SR) {
        Ok(b) => Some(b.channels),
        Err(e) => {
            error = Some(e.to_string());
            None
        }
    });
    let Some(f) = found else {
        if let Some(e) = error {
            return Err(EngineError::Other(format!("reading the clip's audio: {e}")));
        }
        return Err(bad(CMD, format!("no LTC timecode found in the first {seconds:.0} s of the clip's audio")));
    };
    let df = f.drop_frame && rate.supports_drop_frame();
    let start_tick = rate.tick_of(f.start_frame);
    let half = media_rate.frame_duration().0 / 2;
    let start_media = media_rate.frame_at(Tick(start_tick.0 + half));
    let timecode = format_timecode_frames(f.start_frame, rate, df);
    let applied = if apply {
        s.execute("clip.modifyTimecode", json!({"item": item.0, "frame": start_media}))?;
        true
    } else {
        false
    };
    Ok(json!({
        "item": item.0,
        "channel": f.channel,
        "timecode": timecode,
        "dropFrame": df,
        "startTimecode": start_media,
        "at": Tick::from_units(i64::try_from(f.at_sample).unwrap_or(0), i64::from(SR)).0,
        "applied": applied,
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Read Timecode from LTC Audio",
        menu: &[],
        shortcut: None,
        params: r#"{"item":id?,"stream":int?,"seconds":number?,"apply":bool?}"#,
        enabled: crate::commands::always,
        run: read_ltc,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cfa4_tcsync::ltc::{LtcEncoder, LtcFrame};

    use super::*;

    /// `seconds` of audio: silence for `lead` s, then LTC from frame `first` on channel `ch` of 2.
    fn ltc_audio(rate: FrameRate, df: bool, first: i64, lead: f64, seconds: f64, ch: usize) -> Vec<Vec<f32>> {
        let total = (seconds * f64::from(SR)) as usize;
        let lead = (lead * f64::from(SR)) as usize;
        let mut enc = LtcEncoder::new(SR, rate, -12.0).unwrap();
        let mut ltc = Vec::new();
        let mut n = first;
        while ltc.len() < total - lead {
            enc.encode(&LtcFrame::from_frame_number(n, rate, df), &mut ltc);
            n += 1;
        }
        let mut out = vec![vec![0.0f32; total]; 2];
        out[ch][lead..].copy_from_slice(&ltc[..total - lead]);
        // the other channel has something that is not LTC
        let other = 1 - ch;
        for (i, x) in out[other].iter_mut().enumerate() {
            *x = 0.3 * (i as f32 * 0.05).sin();
        }
        out
    }

    fn reader(audio: &[Vec<f32>]) -> impl FnMut(u64, usize) -> Option<Vec<Vec<f32>>> + '_ {
        move |start, n| {
            let s = start as usize;
            Some(audio.iter().map(|c| c.get(s..(s + n).min(c.len())).unwrap_or(&[]).to_vec()).collect())
        }
    }

    #[test]
    fn finds_the_start_timecode_on_either_channel() {
        for (rate, df) in [(FrameRate::FPS_25, false), (FrameRate::FPS_29_97, true), (FrameRate::FPS_24, false)] {
            // LTC starts 1.5 s in, at 10:00:00:00 + 37 frames
            let first = cfa4_time::fields_to_frames(10, 0, 0, 0, rate, df) + 37;
            for ch in [0, 1] {
                let audio = ltc_audio(rate, df, first, 1.5, 6.0, ch);
                let f = scan(rate, df, 2, u64::MAX, reader(&audio)).unwrap_or_else(|| panic!("{rate:?} ch {ch}"));
                assert_eq!(f.channel, ch);
                assert_eq!(f.drop_frame, df);
                // 1.5 s before the first LTC frame
                let lead = rate.frame_at(Tick(Tick::from_units(3, 2).0 + rate.frame_duration().0 / 2));
                assert_eq!(f.start_frame, first - lead, "{rate:?}");
            }
        }
    }

    #[test]
    fn no_ltc_and_short_limits_find_nothing() {
        let quiet = vec![vec![0.0f32; SR as usize * 3]; 2];
        assert_eq!(scan(FrameRate::FPS_25, false, 2, u64::MAX, reader(&quiet)), None);
        let audio = ltc_audio(FrameRate::FPS_25, false, 0, 2.0, 4.0, 0);
        assert_eq!(scan(FrameRate::FPS_25, false, 2, u64::from(SR), reader(&audio)), None, "LTC starts after the limit");
        assert_eq!(scan(FrameRate::FPS_25, false, 2, u64::MAX, |_, _| None), None);
    }

    #[test]
    fn high_rates_read_ltc_at_half() {
        assert_eq!(ltc_rate(FrameRate { num: 50, den: 1 }), FrameRate::FPS_25);
        assert_eq!(ltc_rate(FrameRate { num: 60000, den: 1001 }), FrameRate::FPS_29_97);
        assert_eq!(ltc_rate(FrameRate::FPS_24), FrameRate::FPS_24);
    }

    #[test]
    fn the_command_sets_the_clip_timecode_undoably() {
        let mut s = Session::default();
        s.execute("file.newSequence", json!({"name": "ltc", "audio": 2, "video": 1, "fps": 25})).unwrap();
        let rate = s.sequence_rate();
        let first = cfa4_time::fields_to_frames(14, 30, 0, 0, rate, false);
        let audio = ltc_audio(rate, false, first, 0.4, 5.0, 1);
        let inter: Vec<f32> = (0..audio[0].len()).flat_map(|i| [audio[0][i], audio[1][i]]).collect();
        let bytes: Arc<[u8]> = crate::previews::write_wav_f32(&inter, SR).into();
        let item = crate::commands::import_bytes(&mut s, "/camera-a.wav", bytes, None).unwrap();
        let r = s.execute(CMD, json!({"item": item.0, "apply": false})).unwrap();
        assert_eq!(r["channel"], 1);
        assert_eq!(r["applied"], false);
        let lead = rate.frame_at(Tick(Tick::from_units(2, 5).0 + rate.frame_duration().0 / 2));
        assert_eq!(r["startTimecode"].as_i64(), Some(first - lead), "{r}");
        let n0 = s.history.undo.len();
        let r = s.execute(CMD, json!({"item": item.0})).unwrap();
        assert_eq!(r["applied"], true);
        assert_eq!(s.history.undo.len(), n0 + 1);
        let tc = |s: &Session| s.project.item(item).and_then(|i| i.as_media()).and_then(|m| m.info.start_timecode);
        assert_eq!(tc(&s), Some(first - lead));
        s.execute("edit.undo", json!({})).unwrap();
        assert_ne!(tc(&s), Some(first - lead));
        // a clip without LTC is an error, not a crash
        let quiet: Arc<[u8]> = crate::previews::write_wav_f32(&vec![0.0f32; SR as usize * 4], SR).into();
        let q = crate::commands::import_bytes(&mut s, "/quiet.wav", quiet, None).unwrap();
        let e = s.execute(CMD, json!({"item": q.0, "seconds": 2})).unwrap_err().to_string();
        assert!(e.contains("no LTC"), "{e}");
        assert!(s.execute(CMD, json!({"item": q.0, "seconds": -1})).is_err());
    }
}
