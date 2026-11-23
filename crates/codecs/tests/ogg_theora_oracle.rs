//! Theora in Ogg (`.ogv`) through the media layer against ffmpeg's decode of the same file: every
//! frame equal, in order, backwards and after jumps; the file is recognised by content.
//!
//! The Ogg file is written here from the packet lists in `data/theora/*.bin` (see
//! `theora_oracle.rs`), one packet per page, so ffmpeg is needed only as a decoder.

mod common;

use std::sync::Arc;

use common::*;

fn packets(mut d: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while !d.is_empty() {
        let n = u32::from_le_bytes(d[..4].try_into().unwrap()) as usize;
        out.push(&d[4..4 + n]);
        d = &d[4 + n..];
    }
    out
}

/// One Ogg page of one whole packet (or two for the comment and setup headers).
fn page(out: &mut Vec<u8>, flags: u8, granule: i64, serial: u32, seq: u32, pkts: &[&[u8]]) {
    let mut lacing = Vec::new();
    for p in pkts {
        lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
        lacing.push((p.len() % 255) as u8);
    }
    assert!(lacing.len() <= 255, "packet too large for this writer");
    let start = out.len();
    out.extend_from_slice(b"OggS");
    out.push(0);
    out.push(flags);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&serial.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    for p in pkts {
        out.extend_from_slice(p);
    }
    let crc = cfa4_ogg::crc32(&out[start..]);
    out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
}

fn ogv(pk: &[&[u8]], kfgshift: u32) -> Vec<u8> {
    let mut out = Vec::new();
    page(&mut out, 0x02, 0, 7, 0, &[pk[0]]);
    page(&mut out, 0, 0, 7, 1, &[pk[1], pk[2]]);
    let frames = &pk[3..];
    let mut key = 0i64;
    for (i, p) in frames.iter().enumerate() {
        if p.first().is_some_and(|b| b & 0x40 == 0) {
            key = i as i64;
        }
        let granule = (key << kfgshift) + (i as i64 - key);
        let eos = if i + 1 == frames.len() { 0x04 } else { 0 };
        page(&mut out, eos, granule, 7, 2 + i as u32, &[p]);
    }
    out
}

#[test]
fn theora_in_ogg_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let cases: [(&str, (u32, u32), &str, &[u8]); 4] = [
        ("ogv_420", (96, 64), "yuv420p", include_bytes!("data/theora/theora_420.bin")),
        ("ogv_420_lowq", (96, 64), "yuv420p", include_bytes!("data/theora/theora_420_lowq.bin")),
        ("ogv_444", (80, 48), "yuv444p", include_bytes!("data/theora/theora_444.bin")),
        ("ogv_crop", (90, 66), "yuv420p", include_bytes!("data/theora/theora_crop.bin")),
    ];
    for (name, (w, h), pix, stream) in cases {
        let pk = packets(stream);
        // the identification header's keyframe granule shift (6 bits at bit 5 of byte 40)
        let shift = ((u32::from(pk[0][40]) << 8 | u32::from(pk[0][41])) >> 5) & 0x1f;
        let data = ogv(&pk, shift);
        let file = format!("{name}.ogv");
        let path = dir().join(&file);
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, &data).unwrap();

        let src = cfa4_codecs::open_bytes(&file, Arc::<[u8]>::from(data)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(src.info().container, "Ogg");
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        let (sx, sy) = if pix == "yuv420p" { (1, 1) } else { (0, 0) };
        let (cw, ch) = ((w as usize).div_ceil(1 << sx), (h as usize).div_ceil(1 << sy));
        let raw = ffmpeg_frames(&ff, &path, pix);
        let frame_bytes = (w * h) as usize + 2 * cw * ch;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), pk.len() - 3, "{name}: ffmpeg's frame count");
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert!(planes(&f) == raw_planes(want[i], w as usize, h as usize, cw, ch, 1), "{name}: frame {i} differs from ffmpeg's");
        }
    }
}
