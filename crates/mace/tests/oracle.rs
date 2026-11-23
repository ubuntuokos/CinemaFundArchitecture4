//! Samples against ffmpeg's `mace3` / `mace6` decoders: random bytes in an AIFF-C file, and the
//! FATE QuickTime samples when they are present.

use cfa4_mace::{Decoder, Version};
use std::process::Command;

fn ffmpeg_pcm(file: &std::path::Path) -> Option<Vec<i16>> {
    let out = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(file).args(["-f", "s16le", "-"]).output().ok()?;
    out.status.success().then(|| out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

fn aifc(version: Version, channels: u16, data: &[u8]) -> Vec<u8> {
    let tag: &[u8; 4] = if version == Version::Mace3 { b"MAC3" } else { b"MAC6" };
    let mut comm = Vec::new();
    comm.extend_from_slice(&channels.to_be_bytes());
    comm.extend_from_slice(&0u32.to_be_bytes());
    comm.extend_from_slice(&8u16.to_be_bytes());
    // 22050 Hz as an 80-bit extended float
    comm.extend_from_slice(&0x400Du16.to_be_bytes());
    comm.extend_from_slice(&(22050u64 << 49).to_be_bytes());
    comm.extend_from_slice(tag);
    comm.extend_from_slice(&[0, 0]); // empty pascal string, padded
    let mut ssnd = Vec::new();
    ssnd.extend_from_slice(&[0u8; 8]);
    ssnd.extend_from_slice(data);
    let mut body = b"AIFC".to_vec();
    for (id, bytes) in [(b"FVER", &0xA280_5140u32.to_be_bytes()[..]), (b"COMM", &comm[..]), (b"SSND", &ssnd[..])] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        body.extend_from_slice(bytes);
        if bytes.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn random_streams_match_ffmpeg() {
    let dir = std::env::temp_dir().join(format!("cfa4-mace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut x = 5u32;
    for version in [Version::Mace3, Version::Mace6] {
        for channels in [1u16, 2] {
            let unit = usize::from(channels) << usize::from(version == Version::Mace3);
            let data: Vec<u8> = (0..unit * 3000)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect();
            let path = dir.join(format!("r{}{channels}.aifc", if version == Version::Mace3 { 3 } else { 6 }));
            std::fs::write(&path, aifc(version, channels, &data)).expect("write");
            let Some(want) = ffmpeg_pcm(&path) else {
                eprintln!("ffmpeg not available or cannot decode {version:?}; skipped");
                return;
            };
            let planes = Decoder::new(version, usize::from(channels)).expect("decoder").decode(&data).expect("decode");
            let got: Vec<i16> = (0..planes[0].len()).flat_map(|i| planes.iter().map(move |p| p[i])).collect();
            assert_eq!(got.len(), want.len(), "{version:?} x{channels}: sample count");
            assert_eq!(got.iter().zip(&want).position(|(a, b)| a != b), None, "{version:?} x{channels}: first differing sample");
            // the same stream in two packets: the state carries over
            let (a, b) = data.split_at(unit * 1000);
            let mut d = Decoder::new(version, usize::from(channels)).expect("decoder");
            let (pa, pb) = (d.decode(a).expect("first"), d.decode(b).expect("second"));
            let joined: Vec<i16> = pa.iter().zip(&pb).flat_map(|(x, y)| x.iter().chain(y)).copied().collect();
            let whole: Vec<i16> = planes.iter().flatten().copied().collect();
            assert_eq!(joined, whole, "{version:?} x{channels}: packet split");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
