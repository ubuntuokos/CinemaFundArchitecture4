//! E-AC-3 7.1 (an independent 5.1 substream plus a dependent substream, A/52:2012 §E2.3.1) against
//! ffmpeg. ffmpeg encodes a 5.1 stream and a 2/2 stream, each 2/2 syncframe becomes a dependent
//! substream with a custom channel map ([`make_dependent_substream`]), and ffmpeg's decode of the
//! combined stream is the reference.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::*;

/// Ls, Rs and the Lrs / Rrs pair (Table E2.5 bits 3, 4 and 6, bit 0 being the MSB).
const CHANMAP: u16 = 0x1A00;

const CORE: &str =
    "aevalsrc=exprs=0.3*sin(2*PI*300*t)|0.3*sin(2*PI*400*t)|0.3*sin(2*PI*500*t)|0.3*sin(2*PI*60*t)|0.2*sin(2*PI*700*t)|0.2*sin(2*PI*800*t):s=48000:c=5.1";
const EXTRA: &str = "aevalsrc=exprs=0.25*sin(2*PI*900*t)|0.25*sin(2*PI*1000*t)|0.25*sin(2*PI*1100*t)|0.25*sin(2*PI*1200*t):s=48000:c=quad";

fn encode(ff: &Path, name: &str, input: &str, kbps: &str) -> PathBuf {
    let out = cfa4_testkit::fixtures_dir("ac3").join(format!("{name}.eac3"));
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", input, "-t", "1", "-c:a", "eac3", "-b:a", kbps, "-f", "eac3"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(x) if x.success())
    })
    .unwrap_or_else(|| panic!("could not generate {name}"))
}

fn frames(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    while let Ok(h) = parse_header(&data[p..]) {
        if p + h.frame_bytes > data.len() {
            break;
        }
        out.push(&data[p..p + h.frame_bytes]);
        p += h.frame_bytes;
    }
    out
}

/// The program's locations: 7.1 in output order.
fn d_locations() -> Vec<Location> {
    use Location::*;
    vec![L, R, C, Lfe, Lrs, Rrs, Ls, Rs]
}

fn seven_one(ff: &Path) -> Vec<u8> {
    let core = std::fs::read(encode(ff, "eac3_71_core", CORE, "384k")).unwrap();
    let extra = std::fs::read(encode(ff, "eac3_71_extra", EXTRA, "256k")).unwrap();
    let (core, extra) = (frames(&core), frames(&extra));
    assert_eq!(core.len(), extra.len());
    core.iter().zip(&extra).flat_map(|(c, e)| c.iter().copied().chain(make_dependent_substream(e, CHANMAP).unwrap())).collect()
}

#[test]
fn dependent_substream_extends_51_to_71() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let data = seven_one(&ff);
    let file = cfa4_testkit::fixtures_dir("ac3").join("eac3_71_dependent.eac3");
    std::fs::write(&file, &data).unwrap();

    // the headers alone give the layout
    assert_eq!(program_locations(&data).unwrap(), d_locations());
    assert_eq!(program_locations(&data[..parse_header(&data).unwrap().frame_bytes]).unwrap().len(), 6);

    // ours, one program (independent + dependent syncframe) at a time
    let mut dec = Decoder::new();
    let mut ours: Vec<Vec<f32>> = vec![Vec::new(); 8];
    let mut p = 0;
    while p < data.len() {
        let (d, used) = dec.decode_program(&data[p..]).unwrap();
        assert_eq!(d.locations, d_locations());
        for (o, c) in ours.iter_mut().zip(d.channels) {
            o.extend(c);
        }
        p += used;
    }

    let o = Command::new(&ff).args(["-v", "error", "-i"]).arg(&file).args(["-f", "f32le", "-"]).output().unwrap();
    let probe = Command::new(&ff).args(["-hide_banner", "-i"]).arg(&file).output().unwrap();
    let info = String::from_utf8_lossy(&probe.stderr);
    assert!(info.contains("7.1"), "ffmpeg sees no 7.1 program: {info}");
    let want: Vec<f32> = o.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let n = want.len() / 8;
    assert_eq!(n, ours[0].len(), "sample count");

    // ffmpeg orders 7.1 as FL FR FC LFE BL BR SL SR, with E-AC-3's Lrs / Rrs as its back and Ls /
    // Rs as its side channels: the same order as ours
    const FFMPEG: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    for (k, ch) in ours.iter().enumerate() {
        let (mut sig, mut err) = (0f64, 0f64);
        for i in 0..n {
            let (x, y) = (f64::from(ch[i]), f64::from(want[i * 8 + FFMPEG[k]]));
            sig += y * y;
            err += (x - y) * (x - y);
        }
        let db = 10.0 * (sig / err.max(1e-30)).log10();
        println!("{:?}: {db:.1} dB against ffmpeg", d_locations()[k]);
        assert!(db > 60.0, "{:?}: {db:.1} dB", d_locations()[k]);
    }
    // the dependent channels replaced the core's surrounds: Ls carries 900 Hz, not the core's 700
    let at = |ch: &[f32], hz: f64| {
        let (mut re, mut im) = (0f64, 0f64);
        for (i, &x) in ch.iter().enumerate().take(48_000) {
            let w = 2.0 * std::f64::consts::PI * hz * i as f64 / 48_000.0;
            re += f64::from(x) * w.cos();
            im += f64::from(x) * w.sin();
        }
        (re * re + im * im).sqrt() / 24_000.0
    };
    assert!(at(&ours[6], 900.0) > 0.2 && at(&ours[6], 700.0) < 0.01);
    assert!(at(&ours[5], 1200.0) > 0.2);
}

/// Through `cfa4-codecs`' packet path the 7.1 program arrives as eight channels too (an MP4 /
/// Matroska access unit holds the independent and dependent syncframes), and a damaged dependent
/// substream keeps the layout: the core's channels stay, its own locations go silent.
#[test]
fn damaged_dependent_substream_keeps_the_layout() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let data = seven_one(&ff);
    let units: Vec<&[u8]> = {
        let f = frames(&data);
        f.chunks(2).map(|c| &data[c[0].as_ptr() as usize - data.as_ptr() as usize..][..c[0].len() + c[1].len()]).collect()
    };
    let mut x = 0x5EED_1234u32;
    let mut dec = Decoder::new();
    for (i, u) in units.iter().enumerate().take(12) {
        let core_len = parse_header(u).unwrap().frame_bytes;
        for k in 0..40 {
            let mut m = u.to_vec();
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            // damage the dependent syncframe's audio, never its header and bsi
            let bit = core_len * 8 + 120 + x as usize % ((m.len() - core_len) * 8 - 160);
            m[bit / 8] ^= 0x80 >> (bit % 8);
            if k % 5 == 4 {
                m.truncate(core_len + 40);
            }
            if let Ok((d, _)) = dec.decode_program(&m) {
                assert!(d.channels.len() == 8 || d.channels.len() == 6, "unit {i}: {} channels", d.channels.len());
                assert!(d.channels.iter().all(|c| c.len() == 1536 && c.iter().all(|s| s.is_finite())));
            }
        }
    }
}
