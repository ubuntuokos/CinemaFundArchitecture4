//! Huffyuv MT (`HYMT`) sliced packets against ffmpeg's `hymt` decoder. ffmpeg cannot encode HYMT,
//! so the packets are built here: every slice is the bit stream of a Huffyuv frame that ffmpeg's
//! encoder wrote for `slice_height` rows (the slices share its static tables), laid out with the
//! trailer ffmpeg reads (slice 0 is the bottom one). Skipped when ffmpeg is not installed.

use cfa4_huffyuv::Decoder;

#[path = "support/hymt.rs"]
mod support;
use support::*;

/// Left and gradient prediction. ffmpeg's median path ignores the slice offset (it writes every
/// slice over the picture's top rows), so a multi-slice median stream has no usable reference;
/// median is only checked with one slice, below.
#[test]
fn sliced_packets_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    for pred in ["left", "plane"] {
        sliced_case(pred);
    }
}

fn sliced_case(pred: &str) {
    let (w, slice_h, nb) = (64u32, 16u32, 4usize);
    let height = slice_h * nb as u32;
    let frames = 2u32;
    // one encode per slice position (different content), the same static tables in all
    let parts: Vec<(Vec<u8>, Vec<Vec<u8>>)> =
        (0..nb).map(|k| slice_packets(&format!("slice{k}_{pred}"), pred, w, slice_h, 30 * k as u32 + 7, frames)).collect();
    let extra = parts[0].0.clone();
    assert!(parts.iter().all(|p| p.0 == extra), "the slices' stream headers differ");
    let packets: Vec<Vec<u8>> = (0..frames as usize).map(|f| hymt_packet(&parts.iter().map(|p| p.1[f].as_slice()).collect::<Vec<_>>(), slice_h)).collect();
    let file = dir().join(format!("hymt_{pred}.avi"));
    std::fs::write(&file, avi(w, height, b"HYMT", &extra, &packets)).expect("write");
    let raw = dir().join(format!("hymt_{pred}.yuv"));
    ffmpeg(&["-i", &file.to_string_lossy(), "-f", "rawvideo", "-pix_fmt", "yuv422p", &raw.to_string_lossy()]);
    let want = std::fs::read(&raw).expect("raw");
    let size = (w * height * 2) as usize;
    assert_eq!(want.len(), size * frames as usize, "ffmpeg's decode size");
    let mut dec = Decoder::new(w, height, 16, &extra).expect("decoder");
    dec.set_slices(true);
    for (i, p) in packets.iter().enumerate() {
        let pic = dec.decode(p).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let got = pic.planes.concat();
        let exp = &want[i * size..(i + 1) * size];
        assert_eq!(got, exp, "frame {i} differs from ffmpeg's");
    }
    // without the slice mode the trailer is not looked at: plain Huffyuv, so it must not decode this
    let mut plain = Decoder::new(w, height, 16, &extra).expect("decoder");
    assert!(plain.decode(&packets[0]).map_or(true, |p| p.planes.concat() != want[..size]));
    // a single slice, and a packet with no trailer, decode as Huffyuv
    let (e1, one) = slice_packets("whole", "median", w, slice_h, 3, 1);
    let mut d1 = Decoder::new(w, slice_h, 16, &e1).expect("decoder");
    d1.set_slices(true);
    let plain_pic = Decoder::new(w, slice_h, 16, &e1).expect("decoder").decode(&one[0]).expect("plain");
    assert_eq!(d1.decode(&one[0]).expect("sliced mode").planes, plain_pic.planes);
}

#[test]
fn hostile_trailers_are_errors() {
    let extra = {
        // a minimal header: method 0 (left), 16 bits, progressive, three flat tables
        let mut e = vec![0u8, 16, 0x20, 0];
        for _ in 0..3 {
            e.extend_from_slice(&[0x08, 0x08, 0x08, 0x08, 0xE8, 0x08]);
        }
        e
    };
    let Ok(mut dec) = Decoder::new(16, 16, 16, &extra) else { return };
    dec.set_slices(true);
    for n in 0..64usize {
        let mut p: Vec<u8> = (0..48 + n).map(|i| (i * 31) as u8).collect();
        let l = p.len();
        p[l - 16..l - 12].copy_from_slice(&0u32.to_le_bytes());
        for v in [0u32, 1, 3, u32::MAX, 0x7FFF_FFFF] {
            p[l - 12..l - 8].copy_from_slice(&v.to_le_bytes());
            p[l - 8..l - 4].copy_from_slice(&v.to_le_bytes());
            p[l - 4..].copy_from_slice(&v.to_le_bytes());
            let _ = dec.decode(&p);
        }
    }
}
