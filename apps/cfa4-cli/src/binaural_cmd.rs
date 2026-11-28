//! `cfa4-cli binaural`: a whole file heard through headphones, written as a stereo FLAC
//! (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` §3.5, work package B-05). It renders exactly what the
//! Source monitor plays with headphones on (`serve_monitor`): object audio (Dolby TrueHD Atmos,
//! IAB) object by object from its own positions, any other layout as virtual loudspeakers. With the
//! same HRTF set, room and LFE level the file is the monitor's sound sample for sample, before the
//! 24-bit rounding.
//!
//! The FLAC's VORBIS_COMMENT says what it is (`COMMENT=binaural, HRTF: <set>`). The file is written
//! next to the output under a temporary name and renamed when complete, so a failure or a kill
//! never leaves a partial file under the final name. Nothing here panics: a set that cannot be
//! made, a file without sound or a block that does not decode is an error message.

use std::io::{Seek, SeekFrom, Write};

use serde_json::{Value, json};

use crate::serve_monitor::{self, Binaural};

/// Samples per channel rendered at a time (the monitor's renderers keep their history across
/// blocks, so the size changes nothing in the result).
const BLOCK: usize = 8192;

/// How long the HRTF set may take to make (a large SOFA file is a few seconds).
const LOAD_SECONDS: u64 = 120;

/// `binaural`'s options besides the two paths.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// `builtin` (the default) or a `.maudhrtf` / `.sofa` file.
    pub hrtf: Option<String>,
    /// 0 (no room) to 1; the monitor's default when absent.
    pub room: Option<f64>,
    /// −30 to 0 dB; the monitor's default when absent.
    pub lfe_db: Option<f64>,
    /// The output rate; the file's own rate when absent.
    pub rate: Option<u32>,
    /// −30 to +12 dB applied to the ears before the 24-bit rounding (0 by default, the monitor's
    /// level). A dense object mix can pass full scale; the summary counts the samples clipped.
    pub gain_db: Option<f64>,
}

/// The headphone monitor `opt` asks for, checked as `audio.render {monitor}` checks it.
pub fn monitor(opt: &Options) -> Result<Binaural, String> {
    let mut m = json!({"mode": "binaural", "hrtf": opt.hrtf.as_deref().unwrap_or("builtin")});
    if let Some(room) = opt.room {
        m["room"] = json!(room);
    }
    if let Some(lfe) = opt.lfe_db {
        m["lfe_db"] = json!(lfe);
    }
    let parsed = serve_monitor::parse(&json!({"monitor": m}))
        .map_err(|e| e.replace("`monitor.hrtf`", "--hrtf").replace("`monitor.room`", "--room").replace("`monitor.lfe_db`", "--lfe-db"))?;
    parsed.ok_or_else(|| "no headphone monitor".to_owned())
}

/// Renders `input` to the stereo FLAC `output`; the summary printed as JSON.
pub fn run(input: &str, output: &str, opt: &Options) -> Result<Value, String> {
    if !output.to_ascii_lowercase().ends_with(".flac") {
        return Err(format!("{output}: the binaural output is a .flac file"));
    }
    let b = monitor(opt)?;
    let bytes = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let native = cfa4_codecs::open_bytes(input, bytes.into());
    let src = cfa4_media::open_with_fallback(std::path::Path::new(input), native).map_err(|e| format!("{input}: {e}"))?;
    let info = src.info();
    let Some(stream) = info.audio_streams.first() else { return Err(format!("{input}: no sound")) };
    let rate = opt.rate.unwrap_or(stream.sample_rate);
    if !(8_000..=192_000).contains(&rate) {
        return Err(format!("--rate {rate}: 8000 to 192000 Hz"));
    }
    let total = usize::try_from(info.duration.to_units_floor(i64::from(rate)).max(0)).map_err(|_| "the file is too long".to_owned())?;
    let gain_db = opt.gain_db.unwrap_or(0.0);
    if !(gain_db.is_finite() && (-30.0..=12.0).contains(&gain_db)) {
        return Err("--gain-db must be a number in -30..=12".into());
    }
    let gain = 10f32.powf(gain_db as f32 / 20.0);
    let set = serve_monitor::load(&b.set, rate, LOAD_SECONDS)?;

    let mut enc = cfa4_flac::Encoder::new(cfa4_flac::EncoderConfig::new(rate, 2, 24)).map_err(|e| format!("FLAC: {e}"))?;
    let part = format!("{output}.part");
    let (mut peak, mut clipped) = (0f32, 0u64);
    let written = (|| -> Result<(Value, u64), String> {
        let file = std::fs::File::create(&part).map_err(|e| format!("{part}: {e}"))?;
        let mut w = std::io::BufWriter::new(file);
        let io = |e: std::io::Error| format!("{part}: {e}");
        w.write_all(&enc.header_with_comments(&[format!("COMMENT=binaural, HRTF: {set}")])).map_err(io)?;
        let mut heard = Value::Null;
        let mut start = 0usize;
        while start < total {
            let frames = BLOCK.min(total - start);
            let (ears, meta) = block(&*src, rate, start as i64, frames, &b)?;
            if heard.is_null() {
                heard = meta;
            }
            let mut ints: Vec<Vec<i32>> = Vec::with_capacity(2);
            for ear in &ears {
                ints.push(
                    ear.iter()
                        .map(|&s| {
                            let s = if gain == 1.0 { s } else { s * gain };
                            peak = peak.max(s.abs());
                            clipped += u64::from(s.abs() > 1.0);
                            cfa4_export::SampleFormat::Int24.quantise(s)
                        })
                        .collect(),
                );
            }
            let refs: Vec<&[i32]> = ints.iter().map(Vec::as_slice).collect();
            w.write_all(&enc.encode(&refs).map_err(|e| format!("FLAC: {e}"))?).map_err(io)?;
            start += frames;
        }
        w.write_all(&enc.finish()).map_err(io)?;
        w.seek(SeekFrom::Start(cfa4_flac::STREAMINFO_OFFSET)).map_err(io)?;
        w.write_all(&enc.streaminfo()).map_err(io)?;
        let file = w.into_inner().map_err(|e| format!("{part}: {}", e.error()))?;
        file.sync_all().map_err(io)?;
        Ok((heard, file.metadata().map(|m| m.len()).unwrap_or(0)))
    })();
    let (heard, size) = match written {
        Ok(done) => done,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&part, output) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{output}: {e}"));
    }
    Ok(json!({
        "output": output, "bytes": size, "sampleRate": rate, "frames": total,
        "seconds": total as f64 / f64::from(rate), "hrtf": set, "monitor": heard,
        "gainDb": gain_db, "peakDb": if peak > 0.0 { json!(20.0 * f64::from(peak).log10()) } else { Value::Null },
        "clippedSamples": clipped,
    }))
}

/// One block of the two ears, as the Source monitor renders it: the objects when the file has
/// them and they decode here, otherwise its channels (folded to stereo first when the headphone
/// monitor has no virtual loudspeakers for the layout).
pub fn block(src: &dyn cfa4_media::MediaSource, rate: u32, start: i64, frames: usize, b: &Binaural) -> Result<([Vec<f32>; 2], Value), String> {
    let (ears, meta) = match serve_monitor::render_objects(0, rate, start, frames, src, b) {
        Some((ears, meta)) => (Vec::from(ears), meta),
        None => {
            let sound = src.audio(start, frames, rate).map_err(|e| format!("sample {start}: {e}"))?;
            let (ears, meta) = crate::serve::heard("source", start, sound, frames, Some(b), cfa4_audio_dsp::channels::Mixdown::default());
            let meta = meta.unwrap_or(Value::Null);
            if let Some(e) = meta.get("error").and_then(Value::as_str) {
                return Err(e.to_owned());
            }
            if meta.get("mode").and_then(Value::as_str) != Some("binaural") {
                return Err("the headphone monitor did not render this block".into());
            }
            (ears, meta)
        }
    };
    let mut ears = ears.into_iter().map(|mut c| {
        c.resize(frames, 0.0);
        c
    });
    let (Some(l), Some(r)) = (ears.next(), ears.next()) else { return Err("the headphone monitor gave no ears".into()) };
    Ok(([l, r], meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_are_checked_as_the_monitor_checks_them() {
        let b = monitor(&Options::default()).unwrap();
        assert_eq!(b.set, serve_monitor::SetId::Builtin);
        let e = monitor(&Options { room: Some(2.0), ..Options::default() }).unwrap_err();
        assert!(e.contains("--room"), "{e}");
        let e = monitor(&Options { hrtf: Some("x.wav".into()), ..Options::default() }).unwrap_err();
        assert!(e.contains("--hrtf"), "{e}");
        let e = run("/no/such/file.wav", "x.flac", &Options { gain_db: Some(20.0), ..Options::default() }).unwrap_err();
        assert!(e.contains("/no/such") || e.contains("--gain-db"), "{e}");
    }

    #[test]
    fn bad_paths_are_errors_and_leave_no_file() {
        let dir = std::env::temp_dir().join(format!("cfa4-binaural-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.flac");
        let out = out.to_str().unwrap();
        assert!(run("/no/such/file.wav", out, &Options::default()).unwrap_err().contains("/no/such/file.wav"));
        assert!(run("/no/such/file.wav", "out.wav", &Options::default()).unwrap_err().contains(".flac"));
        assert!(!std::path::Path::new(out).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A 5.1 file rendered whole by `cfa4_hrtf::ChannelMonitor` in one call is what the command
    /// writes block by block (24-bit rounding aside), and the FLAC says which set made it.
    #[test]
    fn the_file_is_the_monitor_sound_sample_for_sample() {
        let dir = std::env::temp_dir().join(format!("cfa4-binaural-51-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rate = 48_000u32;
        let n = 3 * BLOCK + 1234;
        let interleaved: Vec<f32> = (0..n)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                (0..6).map(move |c| 0.2 * (std::f32::consts::TAU * (220.0 + 110.0 * c as f32) * t).sin())
            })
            .collect();
        let wav = cfa4_export::write_wav(&interleaved, 6, rate, cfa4_export::SampleFormat::Float32).unwrap();
        let input = dir.join("in.wav");
        std::fs::write(&input, &wav).unwrap();
        let out = dir.join("out.flac");
        let summary = run(input.to_str().unwrap(), out.to_str().unwrap(), &Options::default()).unwrap();
        assert_eq!(summary["monitor"]["mode"], "binaural");
        assert_eq!(summary["monitor"]["layout"], 6);
        assert_eq!(summary["frames"], n);
        assert_eq!(summary["clippedSamples"], 0);
        assert!(summary["peakDb"].as_f64().is_some_and(|p| p < 0.0), "{summary}");

        let src = cfa4_codecs::open_bytes("in.wav", wav.into()).unwrap();
        let sound = src.audio(0, n, rate).unwrap();
        let set = std::sync::Arc::new(cfa4_hrtf::HrtfSet::builtin(rate).unwrap());
        let mut m = cfa4_hrtf::ChannelMonitor::new(set.clone(), cfa4_hrtf::MonitorConfig::default()).unwrap();
        let inputs: Vec<&[f32]> = sound.channels.iter().map(Vec::as_slice).collect();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        m.process(0, &inputs, &mut l, &mut r).unwrap();

        let flac = std::fs::read(&out).unwrap();
        let text = String::from_utf8_lossy(&flac[..512.min(flac.len())]).into_owned();
        assert!(text.contains(&format!("COMMENT=binaural, HRTF: {}", set.name())), "{text}");
        let got = cfa4_codecs::open_bytes("out.flac", flac.into()).unwrap().audio(0, n, rate).unwrap();
        assert_eq!(got.channels.len(), 2);
        let q = |s: f32| cfa4_export::SampleFormat::Int24.quantise(s);
        let scale = 8_388_608.0;
        for (ear, want) in got.channels.iter().zip([&l, &r]) {
            assert_eq!(ear.len(), n);
            for (i, (&g, &w)) in ear.iter().zip(want.iter()).enumerate() {
                assert!(((g * scale).round() as i32 - q(w)).abs() <= 1, "sample {i}: {g} vs {w}");
            }
        }
        assert!(l.iter().any(|s| s.abs() > 0.01), "the reference is silent");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
