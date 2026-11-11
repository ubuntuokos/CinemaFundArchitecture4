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
}

impl Default for Spec {
    fn default() -> Self {
        Spec { frames: 24, audio_channels: Some(2), stretch: Vec::new(), unknown_codec: false }
    }
}

fn picture(i: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (W as usize, H as usize);
    let base = 40 + (i * 7 % 160) as u8;
    let mut y = vec![base; w * h];
    let bar = (i as usize * 3) % w;
    for row in 0..h {
        for x in bar..(bar + 4).min(w) {
            y[row * w + x] = 235;
        }
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
        let (y, u, v) = picture(i);
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
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let vt = mux.add_track(TrackConfig::new(entry, FPS)).unwrap();
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
        let tone: Vec<f32> = (0..n).map(|k| (k as f32 * 1000.0 * std::f32::consts::TAU / rate as f32).sin() * 0.25).collect();
        let planes: Vec<&[f32]> = (0..ch).map(|_| tone.as_slice()).collect();
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
