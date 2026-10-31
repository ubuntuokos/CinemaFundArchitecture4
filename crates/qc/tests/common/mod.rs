//! Synthetic QC fixtures: small MP4 files written in memory with CFA4's own H.264 and AAC
//! encoders (no ffmpeg, no committed media). Every picture differs (a frame counter in the luma
//! and a moving bar), and the audio is a 1 kHz tone, so the known values (frame count, timing,
//! streams) are exact.

#![allow(dead_code)]

use std::io::Cursor;
use std::sync::Arc;

use cfa4_isobmff::{Brand, CodecConfig, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};

pub const W: u32 = 64;
pub const H: u32 = 64;
pub const FPS: u32 = 24;

#[derive(Clone)]
pub struct Spec {
    pub frames: u32,
    pub audio_channels: Option<u32>,
    /// Frame duration multiplier per frame index (for timing anomalies): `(index, multiple)`.
    pub stretch: Vec<(u32, u32)>,
    /// Write the video track with an unknown codec (no decoder can read it).
    pub unknown_codec: bool,
    /// Peak amplitude of the 1 kHz tone (0.2: about −14 LUFS in stereo).
    pub amplitude: f32,
    /// Seconds of silence in the middle of the audio: `(from, to)`.
    pub silence: Option<(f32, f32)>,
    /// The last channel carries no signal.
    pub mute_last_channel: bool,
    /// Frames rendered black (luma 16) and frames with a white bar above legal white (luma 255).
    pub black: Vec<u32>,
    pub white: Vec<u32>,
    /// Frames with the top quarter at luma 250: beyond legal white in limited range, legal in full.
    pub bright: Vec<u32>,
    /// The video track's `colr` box: `nclx` BT.709 with this full-range flag (the H.264 stream
    /// itself signals no range, so limited).
    pub colr_full_range: Option<bool>,
    /// The video edit list starts this many frames in (those frames are decoded, never shown).
    pub video_media_start: Option<i64>,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            frames: 24,
            audio_channels: Some(2),
            stretch: Vec::new(),
            unknown_codec: false,
            amplitude: 0.2,
            silence: None,
            mute_last_channel: false,
            black: Vec::new(),
            white: Vec::new(),
            bright: Vec::new(),
            colr_full_range: None,
            video_media_start: None,
        }
    }
}

fn picture(i: u32, spec: &Spec) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (W as usize, H as usize);
    if spec.black.contains(&i) {
        return (vec![16; w * h], vec![128; w * h / 4], vec![128; w * h / 4]);
    }
    let base = 40 + (i * 7 % 160) as u8;
    let mut y = vec![base; w * h];
    let bar = (i as usize * 3) % w;
    for row in 0..h {
        for x in bar..(bar + 4).min(w) {
            y[row * w + x] = 235;
        }
    }
    if spec.bright.contains(&i) {
        y.iter_mut().take(w * h / 4).for_each(|v| *v = 250);
    }
    if spec.white.contains(&i) {
        // the top quarter of the picture at code 255
        y.iter_mut().take(w * h / 4).for_each(|v| *v = 255);
    }
    (y, vec![128; w * h / 4], vec![128; w * h / 4])
}

pub fn mp4(spec: &Spec) -> Vec<u8> {
    let mut cfg = cfa4_h264enc::EncoderConfig::new(W, H, FPS, 1);
    cfg.format = cfa4_h264enc::PacketFormat::LengthPrefixed;
    cfg.aud = false;
    cfg.bframes = 0;
    cfg.keyint = 12;
    let mut enc = cfa4_h264enc::Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for i in 0..spec.frames {
        let (y, u, v) = picture(i, spec);
        let f = cfa4_h264enc::YuvFrame { y: &y, u: &u, v: &v, y_stride: W as usize, uv_stride: W as usize / 2 };
        packets.extend(enc.try_encode(&f, i64::from(i)).unwrap());
    }
    packets.extend(enc.flush());
    let avcc = cfa4_isobmff::AvcConfig::parse(&enc.avcc()).unwrap();
    let entry = if spec.unknown_codec {
        SampleEntry::video(FourCc(*b"zzzz"), CodecConfig::Unknown { fourcc: FourCc(*b"zzzz"), raw: Vec::new() }, W as u16, H as u16)
    } else {
        SampleEntry::avc(avcc, W as u16, H as u16)
    };
    let mut entry = entry;
    if let (Some(full_range), Some(v)) = (spec.colr_full_range, entry.video.as_mut()) {
        v.color = Some(cfa4_isobmff::ColorInfo::Nclx { primaries: 1, transfer: 1, matrix: 1, full_range });
    }
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let mut vcfg = TrackConfig::new(entry, FPS);
    vcfg.media_start = spec.video_media_start;
    let vt = mux.add_track(vcfg).unwrap();
    for (k, p) in packets.iter().enumerate() {
        let mult = spec.stretch.iter().find(|(i, _)| *i as usize == k).map_or(1, |(_, m)| *m);
        mux.write_sample(vt, WriteSample { data: &p.data, duration: mult, composition_offset: (p.pts - p.dts) as i32, is_sync: p.keyframe }).unwrap();
    }
    if let Some(ch) = spec.audio_channels {
        let rate = 48_000u32;
        let mut aenc = cfa4_aac::Encoder::new(cfa4_aac::EncoderConfig::cbr(rate, ch as usize, 128_000)).unwrap();
        let mut at = TrackConfig::new(SampleEntry::aac(aenc.audio_specific_config(), ch, rate), rate);
        at.media_start = Some(i64::from(aenc.priming_samples()));
        let a = mux.add_track(at).unwrap();
        let n = (rate as usize * spec.frames as usize) / FPS as usize;
        let mut tone: Vec<f32> = (0..n).map(|k| (k as f32 * 1000.0 * std::f32::consts::TAU / rate as f32).sin() * spec.amplitude).collect();
        if let Some((a, b)) = spec.silence {
            let (a, b) = ((a * rate as f32) as usize, ((b * rate as f32) as usize).min(n));
            tone[a..b].iter_mut().for_each(|x| *x = 0.0);
        }
        let mute = vec![0.0f32; n];
        let planes: Vec<&[f32]> = (0..ch).map(|c| if spec.mute_last_channel && c + 1 == ch { mute.as_slice() } else { tone.as_slice() }).collect();
        let mut aps = aenc.encode(&planes);
        aps.extend(aenc.flush());
        for p in &aps {
            mux.write_sample(a, WriteSample { data: p, duration: 1024, composition_offset: 0, is_sync: true }).unwrap();
        }
    }
    mux.finish().unwrap().into_inner()
}

pub fn reader(bytes: Vec<u8>) -> cfa4_media::SharedReader {
    let b: Arc<[u8]> = bytes.into();
    Arc::new(cfa4_media::reader::MemReader(b))
}

/// Offset and size of video sample `index` (to damage it).
pub fn video_sample(bytes: &[u8], index: usize) -> (usize, usize) {
    let f = cfa4_isobmff::open(bytes).unwrap();
    let t = f.track_of_kind(cfa4_isobmff::TrackKind::Video).unwrap();
    let s = f.tracks[t].samples[index];
    (s.offset as usize, s.size as usize)
}
