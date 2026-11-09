use super::tests::{adts_frame, mp2_frame, mpeg2_frame};
use super::*;

/// 25 fps pictures (I then P), 90 kHz apart.
const FRAME: i64 = 3600;

fn video(n: u16) -> Vec<Vec<u8>> {
    (0..n).map(|k| mpeg2_frame(if k % 5 == 0 { 1 } else { 2 }, k % 5, false, 300 + 997 * usize::from(k))).collect()
}

/// A transport stream of `pics` pictures and 1024-sample ADTS frames at 48 kHz, the audio of each
/// group of four pictures queued after their video (as the exporter does), starting 1920 ticks
/// early (encoder priming).
fn ts(format: MuxFormat, pics: &[Vec<u8>]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut m = Muxer::new(Vec::new(), format, &[MuxCodec::Mpeg2Video, MuxCodec::AacAdts], 20_000_000).unwrap();
    let mut audio = Vec::new();
    let mut a = 0i64;
    for (k, p) in pics.iter().enumerate() {
        m.write(0, p, k as i64 * FRAME, None, k % 5 == 0).unwrap();
        if k % 4 == 3 || k + 1 == pics.len() {
            let end = (k as i64 + 1) * FRAME;
            while -1920 + a * 1920 < end {
                let f = adts_frame(a as u8);
                m.write(1, &f, -1920 + a * 1920, None, true).unwrap();
                audio.push(f);
                a += 1;
            }
        }
    }
    m.append_to_last(0, &[0, 0, 1, 0xB7]).unwrap();
    (m.finish().unwrap(), audio)
}

#[test]
fn transport_streams_demux_to_what_was_written() {
    let pics = video(23);
    for (format, size) in [(MuxFormat::Ts, 188), (MuxFormat::M2ts, 192)] {
        let (data, audio) = ts(format, &pics);
        assert_eq!(data.len() % size, 0);
        let f = open(&data).unwrap();
        assert!(matches!(f.format, Format::Ts { packet_size, .. } if packet_size == size), "{:?}", f.format);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert_eq!(f.streams.len(), 2);
        let v = f.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
        let a = f.find(Kind::Audio, |c| *c == Codec::AacAdts).unwrap();
        assert_eq!(f.streams[v].units.len(), pics.len());
        for (k, u) in f.streams[v].units.iter().enumerate() {
            assert_eq!(u.pts, Some(Muxer::<Vec<u8>>::DELAY + k as i64 * FRAME));
            assert_eq!(u.key, k % 5 == 0);
            let mut want = pics[k].clone();
            if k + 1 == pics.len() {
                want.extend([0, 0, 1, 0xB7]);
            }
            assert_eq!(f.read_unit(&data, v, k).unwrap(), want, "picture {k}");
        }
        assert_eq!(f.streams[a].units.len(), audio.len());
        for (k, u) in f.streams[a].units.iter().enumerate() {
            assert_eq!(u.pts, Some(Muxer::<Vec<u8>>::DELAY - 1920 + k as i64 * 1920));
            assert_eq!(f.read_unit(&data, a, k).unwrap(), audio[k]);
        }
        // the clock (27 MHz) starts near zero (after the tables and the first audio) and stays
        // ahead of the data
        let (p0, p1) = f.pcr_range.unwrap();
        assert!(p0 < 27_000, "{p0}");
        assert!(p1 < 300 * (Muxer::<Vec<u8>>::DELAY + 22 * FRAME), "{p1}");
    }
}

#[test]
fn units_are_interleaved_by_decode_time() {
    let (data, _) = ts(MuxFormat::Ts, &video(12));
    let f = open(&data).unwrap();
    // every unit is in the file after the units of either stream that decode earlier
    let mut all: Vec<(i64, u64)> = f.streams.iter().flat_map(|s| s.units.iter().map(|u| (u.dts.or(u.pts).unwrap(), u.pos))).collect();
    all.sort();
    assert!(all.windows(2).all(|w| w[0].1 <= w[1].1), "{all:?}");
}

#[test]
fn program_streams_carry_video_and_dvd_lpcm() {
    let pics = video(12);
    let mut m = Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Mpeg2Video, MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2 }], 10_080_000).unwrap();
    let mut pcm = Vec::new();
    for (k, p) in pics.iter().enumerate() {
        m.write(0, p, k as i64 * FRAME, None, k % 5 == 0).unwrap();
        if k % 4 == 3 {
            // 4 pictures: 7680 stereo 16-bit samples
            let start = (k as i64 - 3) * FRAME;
            let block: Vec<u8> = (0..7680 * 4).map(|i| (i * 7 + k) as u8).collect();
            m.write(1, &block, start, None, true).unwrap();
            pcm.extend(block);
        }
    }
    let data = m.finish().unwrap();
    assert_eq!(&data[data.len() - 4..], &[0, 0, 1, 0xB9]);
    let f = open(&data).unwrap();
    assert_eq!(f.format, Format::Ps { mpeg1: false });
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    let v = f.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
    let a = f.find(Kind::Audio, |c| *c == Codec::LpcmDvd).unwrap();
    assert_eq!(f.streams[a].lpcm, Some(LpcmFormat { sample_rate: 48_000, channels: 2, bits: 16 }));
    for (k, p) in pics.iter().enumerate() {
        assert_eq!(&f.read_unit(&data, v, k).unwrap(), p);
    }
    let units = &f.streams[a].units;
    let got: Vec<u8> = (0..units.len()).flat_map(|k| f.read_unit(&data, a, k).unwrap()).collect();
    assert_eq!(got, pcm);
    // each block's first packet carries its time; continuation packets none
    let stamped: Vec<i64> = units.iter().filter_map(|u| u.pts).collect();
    assert_eq!(stamped, [0, 4, 8].map(|k| Muxer::<Vec<u8>>::DELAY + k * FRAME));
    // packs: a header before every PES packet, none over 2048 bytes
    let packs: Vec<usize> = data.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 1, 0xBA]).map(|(i, _)| i).collect();
    assert!(packs.windows(2).all(|w| w[1] - w[0] <= 2048));
}

#[test]
fn bad_input_is_an_error() {
    let lpcm = MuxCodec::LpcmDvd { sample_rate: 22_050, channels: 2 };
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Mpeg2Video, lpcm], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::AacAdts], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ts, &[MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2 }], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ts, &[], 1_000_000).is_err());
    let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[MuxCodec::Mpeg2Video], 0).unwrap();
    assert!(m.write(1, &[1], 0, None, true).is_err());
    assert!(m.write(0, &[1], -Muxer::<Vec<u8>>::DELAY - 1, None, true).is_err());
    assert!(m.write(0, &[1], 0, Some(1), true).is_err());
    assert!(m.write(0, &[1], i64::MAX, None, true).is_err());
    m.write(0, &[1], 100, None, true).unwrap();
    assert!(m.write(0, &[1], 50, None, true).is_err());
    assert!(m.append_to_last(0, &[2]).is_ok());
}

/// An Annex B access unit: delimiter, then `slices` of NAL header `nal` and `size` bytes of
/// emulation-free filler.
fn annexb_unit(hevc: bool, nal: u8, size: usize, k: u8) -> Vec<u8> {
    let mut v = if hevc { vec![0, 0, 0, 1, 0x46, 0x01, 0x50] } else { vec![0, 0, 0, 1, 0x09, 0xF0] };
    v.extend_from_slice(&[0, 0, 1, nal]);
    if hevc {
        v.push(0x01);
    }
    v.extend((0..size).map(|i| 0x40 | ((i as u8) ^ k) & 0x3F));
    v
}

/// I P B B … in decode order with B-frame timestamps (pts ≠ dts), and MPEG audio beside it.
#[test]
fn h264_and_hevc_access_units_go_out_unchanged() {
    for hevc in [false, true] {
        // H.264: IDR 0x65, reference P 0x41, non-reference B 0x01; HEVC: IDR_W_RADL 19, TRAIL_R 1, TRAIL_N 0
        let (idr, p, b) = if hevc { (19 << 1, 1 << 1, 0) } else { (0x65, 0x41, 0x01) };
        let codec = if hevc { MuxCodec::Hevc } else { MuxCodec::H264 };
        let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[codec, MuxCodec::MpegAudio { lsf: false }], 20_000_000).unwrap();
        let mut units = Vec::new();
        for k in 0..30u8 {
            let (nal, pts) = match k % 3 {
                0 => (if k % 15 == 0 { idr } else { p }, i64::from(k) + 2),
                _ => (b, i64::from(k) - 1),
            };
            let u = annexb_unit(hevc, nal, 200 + 1500 * usize::from(k), k);
            m.write(0, &u, pts * FRAME, Some((i64::from(k) - 1) * FRAME), k % 15 == 0).unwrap();
            units.push((u, pts * FRAME, (i64::from(k) - 1) * FRAME, k % 15 == 0, nal == b));
        }
        let audio: Vec<Vec<u8>> = (0..40u8).map(mp2_frame).collect();
        for (k, f) in audio.iter().enumerate() {
            m.write(1, f, k as i64 * 2160, None, true).unwrap();
        }
        let data = m.finish().unwrap();
        let f = open(&data).unwrap();
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        let want = if hevc { Codec::Hevc } else { Codec::H264 };
        let v = f.find(Kind::Video, |c| *c == want).unwrap();
        let a = f.find(Kind::Audio, |c| *c == Codec::MpegAudio).unwrap();
        assert_eq!(f.streams[v].units.len(), units.len());
        let d = Muxer::<Vec<u8>>::DELAY;
        for (k, (u, pts, dts, key, disposable)) in units.iter().enumerate() {
            let got = &f.streams[v].units[k];
            assert_eq!((got.pts, got.dts.or(got.pts), got.key, got.disposable), (Some(pts + d), Some(dts + d), *key, *disposable), "unit {k}");
            assert_eq!(&f.read_unit(&data, v, k).unwrap(), u, "unit {k} payload");
        }
        assert_eq!(f.streams[a].units.len(), audio.len());
        for (k, want) in audio.iter().enumerate() {
            assert_eq!(&f.read_unit(&data, a, k).unwrap(), want);
        }
        // the tables go out before every random-access point: one PAT per IDR at least
        let pats = data.chunks(188).filter(|p| p[1] & 0x1F == 0 && p[2] == 0).count();
        assert!(pats >= 2, "{pats}");
    }
}

#[test]
fn h264_needs_annex_b_with_a_delimiter_and_a_transport_stream() {
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::H264], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::MpegAudio { lsf: false }], 1_000_000).is_err());
    let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[MuxCodec::H264, MuxCodec::Hevc], 1_000_000).unwrap();
    // length-prefixed (MP4) samples and units without a delimiter are refused, not converted
    assert!(m.write(0, &[0, 0, 0, 5, 0x65, 1, 2, 3, 4], 0, None, true).is_err());
    assert!(m.write(0, &[0, 0, 0, 1, 0x65, 1, 2, 3], 0, None, true).is_err());
    assert!(m.write(1, &annexb_unit(false, 0x65, 10, 0), 0, None, true).is_err());
    m.write(0, &annexb_unit(false, 0x65, 10, 0), 0, None, true).unwrap();
    m.write(1, &annexb_unit(true, 19 << 1, 10, 0), 0, None, true).unwrap();
}

/// External oracle: H.264 / HEVC + MPEG audio transport streams made by ffmpeg, demuxed by us and
/// muxed again, decode to the same frames (skipped without ffmpeg or its encoders).
#[test]
fn ffmpeg_decodes_a_remux_to_the_same_frames() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("mpegts::mux_tests::ffmpeg_decodes_a_remux_to_the_same_frames") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-mpegts-remux-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (enc, codec, want) in [("libx264", MuxCodec::H264, Codec::H264), ("libx265", MuxCodec::Hevc, Codec::Hevc)] {
        if !cfa4_testkit::ffmpeg_has_encoder(&ffmpeg, enc) || !cfa4_testkit::ffmpeg_has_encoder(&ffmpeg, "mp2") {
            eprintln!("skip {enc}: encoder missing");
            continue;
        }
        let src = dir.join(format!("{enc}.ts"));
        let aud = if enc == "libx264" { "h264_metadata=aud=insert" } else { "hevc_metadata=aud=insert" };
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=2", "-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args(["-c:v", enc, "-g", "12", "-bf", "2", "-bsf:v", aud, "-c:a", "mp2", "-b:a", "128k"])
            .args(if enc == "libx265" { &["-x265-params", "log-level=error"][..] } else { &[] })
            .arg(&src)
            .status()
            .unwrap();
        assert!(ok.success());
        let bytes = std::fs::read(&src).unwrap();
        let f = open(&bytes).unwrap();
        let v = f.find(Kind::Video, |c| *c == want).unwrap();
        let a = f.find(Kind::Audio, |c| *c == Codec::MpegAudio).unwrap();
        let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[codec, MuxCodec::MpegAudio { lsf: false }], 10_000_000).unwrap();
        let base = f.streams.iter().flat_map(|s| s.units.iter().filter_map(|u| u.dts.or(u.pts))).min().unwrap();
        // units without a timestamp (several audio frames in one PES packet) travel with the
        // stamped unit before them, as one PES packet again
        let mut groups: Vec<(i64, usize, i64, Option<i64>, bool, Vec<u8>)> = Vec::new();
        for (out, &s) in [v, a].iter().enumerate() {
            for (i, u) in f.streams[s].units.iter().enumerate() {
                let data = f.read_unit(&bytes, s, i).unwrap();
                match u.pts {
                    Some(pts) => groups.push((u.dts.unwrap_or(pts), out, pts, u.dts, u.key, data)),
                    None => groups.iter_mut().rev().find(|g| g.1 == out).unwrap().5.extend(data),
                }
            }
        }
        // both streams in decode order, interleaved
        groups.sort_by_key(|g| (g.0, g.1));
        for (_, out, pts, dts, key, data) in groups {
            m.write(out, &data, pts - base, dts.map(|d| d - base), key).unwrap();
        }
        let remux = dir.join(format!("{enc}-remux.ts"));
        std::fs::write(&remux, m.finish().unwrap()).unwrap();
        let md5 = |p: &std::path::Path, map: &str| {
            let o = std::process::Command::new(&ffmpeg).args(["-v", "error", "-i"]).arg(p).args(["-map", map, "-f", "framemd5", "-"]).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            // the hashes only: timestamps start elsewhere after the remux
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.starts_with('#'))
                .map(|l| l.rsplit(',').next().unwrap_or("").trim().to_string())
                .collect::<Vec<_>>()
        };
        for map in ["0:v", "0:a"] {
            let (x, y) = (md5(&src, map), md5(&remux, map));
            assert!(!x.is_empty());
            assert_eq!(x, y, "{enc} {map}");
        }
    }
}
