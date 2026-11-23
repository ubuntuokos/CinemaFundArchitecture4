//! Builders shared by the Huffyuv MT tests (here and in `cfa4-codecs`): an AVI writer and HYMT
//! packets made from ffmpeg-encoded slices.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

pub fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

pub fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/huffyuv-mt");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

pub fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

pub fn list(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    chunk(b"LIST", &[kind.as_slice(), data].concat())
}

/// A one-stream AVI of `frames` with the given `BITMAPINFOHEADER` extradata and FourCC.
pub fn avi(w: u32, h: u32, fourcc: &[u8; 4], extra: &[u8], frames: &[Vec<u8>]) -> Vec<u8> {
    let mut avih = Vec::new();
    for v in [100_000u32, 0, 0, 0x10, frames.len() as u32, 0, 1, 0, w, h, 0, 0, 0, 0] {
        avih.extend_from_slice(&v.to_le_bytes());
    }
    let mut strh = b"vids".to_vec();
    strh.extend_from_slice(fourcc);
    for v in [0u32, 0, 0, 1, 10, 0, frames.len() as u32, 0, u32::MAX, 0] {
        strh.extend_from_slice(&v.to_le_bytes());
    }
    strh.extend_from_slice(&[0u8; 8]);
    let mut strf = Vec::new();
    for v in [(40 + extra.len()) as u32, w, h] {
        strf.extend_from_slice(&v.to_le_bytes());
    }
    strf.extend_from_slice(&1u16.to_le_bytes());
    strf.extend_from_slice(&16u16.to_le_bytes());
    strf.extend_from_slice(fourcc);
    strf.extend_from_slice(&[0u8; 20]);
    strf.extend_from_slice(extra);
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat())].concat());
    let mut movi = Vec::new();
    let mut idx = Vec::new();
    for f in frames {
        idx.extend_from_slice(b"00dc");
        for v in [0x10u32, 4 + movi.len() as u32, f.len() as u32] {
            idx.extend_from_slice(&v.to_le_bytes());
        }
        movi.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi), chunk(b"idx1", &idx)].concat())
}

/// The packets and extradata of the single-frame Huffyuv AVI ffmpeg wrote for `slice_h` rows.
pub fn slice_packets(name: &str, pred: &str, w: u32, slice_h: u32, seed: u32, frames: u32) -> (Vec<u8>, Vec<Vec<u8>>) {
    let path = dir().join(format!("{name}.avi"));
    let src = format!("testsrc2=size={w}x{slice_h}:rate=10,noise=alls=40:allf=t:all_seed={seed},hue=h={seed}");
    ffmpeg(&["-f", "lavfi", "-i", &src, "-frames:v", &frames.to_string(), "-pix_fmt", "yuv422p", "-c:v", "huffyuv", "-pred", pred, &path.to_string_lossy()]);
    let bytes = std::fs::read(&path).expect("read");
    let file = cfa4_avi::open(&bytes).expect("avi");
    let (si, s) = file.streams.iter().enumerate().find(|(_, s)| s.video.is_some()).expect("video");
    let extra = s.video.as_ref().expect("bih").extra.clone();
    (extra, (0..s.chunks.len()).map(|i| file.read_chunk(&bytes, si, i).expect("chunk")).collect())
}

/// One HYMT packet from `slices` (slice 0 first, the bottom one).
pub fn hymt_packet(slices: &[&[u8]], slice_h: u32) -> Vec<u8> {
    let mut p = Vec::new();
    let mut table = Vec::new();
    for s in slices {
        assert_eq!(p.len() % 4, 0);
        table.extend_from_slice(&(p.len() as u32).to_le_bytes());
        table.extend_from_slice(&(s.len() as u32).to_le_bytes());
        p.extend_from_slice(s);
        while p.len() % 4 != 0 {
            p.push(0);
        }
    }
    let info = p.len() as u32;
    p.extend_from_slice(&table);
    for v in [0u32, slices.len() as u32, slice_h, info] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p
}
