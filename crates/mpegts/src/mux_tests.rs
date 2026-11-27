use super::tests::{ac3_frame, adts_frame, mp2_frame, mpeg2_frame};
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
    for bits in [16, 20, 24] {
        program_stream_with_lpcm(bits);
    }
}

fn program_stream_with_lpcm(bits: u32) {
    let pics = video(12);
    let lpcm = MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2, bits };
    let mut m = Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Mpeg2Video, lpcm], 10_080_000).unwrap();
    // 4 pictures: 7680 stereo sample frames, 3840 groups of two at 20 and 24 bits
    let block_len = 3840 * 2 * lpcm_unit_bytes(2, bits) / if bits == 16 { 1 } else { 2 };
    let mut pcm = Vec::new();
    for (k, p) in pics.iter().enumerate() {
        m.write(0, p, k as i64 * FRAME, None, k % 5 == 0).unwrap();
        if k % 4 == 3 {
            let start = (k as i64 - 3) * FRAME;
            let block: Vec<u8> = (0..block_len).map(|i| (i * 7 + k) as u8).collect();
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
    assert_eq!(f.streams[a].lpcm, Some(LpcmFormat { sample_rate: 48_000, channels: 2, bits }));
    for (k, p) in pics.iter().enumerate() {
        assert_eq!(&f.read_unit(&data, v, k).unwrap(), p);
    }
    let units = &f.streams[a].units;
    let got: Vec<u8> = (0..units.len()).flat_map(|k| f.read_unit(&data, a, k).unwrap()).collect();
    assert_eq!(got, pcm);
    // every packet holds whole frames (groups at 20 and 24 bits)
    assert!(units.iter().enumerate().all(|(k, _)| f.read_unit(&data, a, k).unwrap().len() % lpcm_unit_bytes(2, bits) == 0), "{bits} bits");
    // each block's first packet carries its time; continuation packets none
    let stamped: Vec<i64> = units.iter().filter_map(|u| u.pts).collect();
    assert_eq!(stamped, [0, 4, 8].map(|k| Muxer::<Vec<u8>>::DELAY + k * FRAME));
    // packs: a header before every PES packet, none over 2048 bytes
    let packs: Vec<usize> = data.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 1, 0xBA]).map(|(i, _)| i).collect();
    assert!(packs.windows(2).all(|w| w[1] - w[0] <= 2048));
}

/// Pictures and 1536-sample AC-3 syncframes at 48 kHz (2880 ticks apart), in `format`.
fn with_ac3(format: MuxFormat) -> (Vec<u8>, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let pics = video(12);
    // the 15 syncframes of the 12 pictures (0.48 s)
    let frames: Vec<Vec<u8>> = (0..15).map(|k| ac3_frame(k as u8 + 1)).collect();
    let mut m = Muxer::new(Vec::new(), format, &[MuxCodec::Mpeg2Video, MuxCodec::Ac3], 10_080_000).unwrap();
    let mut next = 0;
    for (k, p) in pics.iter().enumerate() {
        m.write(0, p, k as i64 * FRAME, None, k % 5 == 0).unwrap();
        // the syncframes that start before the next picture
        while next < frames.len() && next as i64 * 2880 < (k as i64 + 1) * FRAME {
            m.write(1, &frames[next], next as i64 * 2880, None, true).unwrap();
            next += 1;
        }
    }
    (m.finish().unwrap(), pics, frames)
}

#[test]
fn program_and_transport_streams_carry_ac3() {
    for format in [MuxFormat::Ps, MuxFormat::Ts, MuxFormat::M2ts] {
        let (data, pics, frames) = with_ac3(format);
        let f = open(&data).unwrap();
        assert!(f.warnings.is_empty(), "{format:?}: {:?}", f.warnings);
        let v = f.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
        let a = f.find(Kind::Audio, |c| *c == Codec::Ac3).unwrap();
        for (k, p) in pics.iter().enumerate() {
            assert_eq!(&f.read_unit(&data, v, k).unwrap(), p, "{format:?} picture {k}");
        }
        let units = &f.streams[a].units;
        assert_eq!(units.len(), frames.len(), "{format:?}");
        for (k, fr) in frames.iter().enumerate() {
            assert_eq!(&f.read_unit(&data, a, k).unwrap(), fr, "{format:?} syncframe {k}");
            assert_eq!(units[k].pts, Some(Muxer::<Vec<u8>>::DELAY + k as i64 * 2880), "{format:?} syncframe {k}");
        }
        match format {
            MuxFormat::Ps => assert_eq!(f.streams[a].id, StreamId::Ps { stream_id: 0xBD, sub_id: Some(0x80) }),
            _ => {
                assert_eq!(f.streams[a].stream_type, 0x81);
                // the PMT carries the `AC-3` registration descriptor
                assert!(data.windows(6).any(|w| w == [0x05, 4, b'A', b'C', b'-', b'3']), "{format:?}");
            }
        }
    }
}

#[test]
fn bad_input_is_an_error() {
    let lpcm = MuxCodec::LpcmDvd { sample_rate: 22_050, channels: 2, bits: 16 };
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Mpeg2Video, lpcm], 1_000_000).is_err());
    let lpcm = MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2, bits: 32 };
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Mpeg2Video, lpcm], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::AacAdts], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ts, &[MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 2, bits: 16 }], 1_000_000).is_err());
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
        // audio first: the PCR still goes on the video PID
        let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[MuxCodec::MpegAudio, codec], 20_000_000).unwrap();
        let mut units = Vec::new();
        for k in 0..30u8 {
            let (nal, pts) = match k % 3 {
                0 => (if k % 15 == 0 { idr } else { p }, i64::from(k) + 2),
                _ => (b, i64::from(k) - 1),
            };
            // unit 15 (an IDR) is over 64 KiB: PES_packet_length 0
            let size = if k == 15 { 90_000 } else { 200 + 1500 * usize::from(k) };
            let u = annexb_unit(hevc, nal, size, k);
            m.write(1, &u, pts * FRAME, Some((i64::from(k) - 1) * FRAME), k % 15 == 0).unwrap();
            units.push((u, pts * FRAME, (i64::from(k) - 1) * FRAME, k % 15 == 0, nal == b));
        }
        let audio: Vec<Vec<u8>> = (0..40u8).map(mp2_frame).collect();
        for (k, f) in audio.iter().enumerate() {
            m.write(0, f, k as i64 * 2160, None, true).unwrap();
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
        check_packets(&data, 0x101, units.iter().map(|u| (u.0.len(), u.3)).collect());
    }
}

/// Transport-level checks of video PID `pid` whose units have (size, key): the PMT names it as
/// PCR PID and the PCR on it never pauses over 50 ms (H.222.0: 100 ms; here a unit is 40 ms); each unit's first packet has the
/// random_access_indicator exactly when it is a key unit, and the PAT and PMT right before it then;
/// PES_packet_length is 0 exactly for units too long for it.
fn check_packets(data: &[u8], pid: u16, units: Vec<(usize, bool)>) {
    let packets: Vec<&[u8]> = data.chunks(188).collect();
    let pid_of = |p: &[u8]| u16::from(p[1] & 0x1F) << 8 | u16::from(p[2]);
    let pmt = packets.iter().find(|p| pid_of(p) == 0x1000).unwrap();
    assert_eq!(u16::from(pmt[13] & 0x1F) << 8 | u16::from(pmt[14]), pid, "PCR_PID");
    let mut starts = Vec::new();
    let mut last_pcr: Option<i64> = None;
    for (i, p) in packets.iter().enumerate() {
        if pid_of(p) != pid {
            continue;
        }
        let af = p[3] & 0x20 != 0 && p[4] > 0;
        if af && p[5] & 0x10 != 0 {
            let base = i64::from(p[6]) << 25 | i64::from(p[7]) << 17 | i64::from(p[8]) << 9 | i64::from(p[9]) << 1 | i64::from(p[10] >> 7);
            let t = base * 300 + (i64::from(p[10] & 1) << 8 | i64::from(p[11]));
            if let Some(l) = last_pcr {
                assert!(t - l <= 50 * 27_000, "PCR gap {} ms", (t - l) / 27_000);
            }
            last_pcr = Some(t);
        }
        if p[1] & 0x40 != 0 {
            let rai = af && p[5] & 0x40 != 0;
            let pes = if p[3] & 0x20 != 0 { 5 + usize::from(p[4]) } else { 4 };
            let len = u16::from(p[pes + 4]) << 8 | u16::from(p[pes + 5]);
            starts.push((i, rai, len));
        }
    }
    assert_eq!(starts.len(), units.len());
    for (k, ((i, rai, len), (size, key))) in starts.into_iter().zip(units).enumerate() {
        assert_eq!(rai, key, "unit {k}");
        assert_eq!(len == 0, size > 0xFFF0, "unit {k}: {size} bytes, length field {len}");
        if key {
            assert_eq!((pid_of(packets[i - 2]), pid_of(packets[i - 1])), (0, 0x1000), "unit {k}: tables first");
        }
    }
}

#[test]
fn audio_units_fit_a_pes_packet_in_transport_streams() {
    for codec in [MuxCodec::AacAdts, MuxCodec::MpegAudio] {
        let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[codec], 1_000_000).unwrap();
        assert!(m.write(0, &vec![0; 70_000], 0, None, true).is_err(), "{codec:?}");
    }
}

#[test]
fn h264_needs_annex_b_with_a_delimiter_and_a_transport_stream() {
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::H264], 1_000_000).is_err());
    assert!(Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Hevc], 1_000_000).is_err());
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
        let mut m = Muxer::new(Vec::new(), MuxFormat::Ts, &[codec, MuxCodec::MpegAudio], 10_000_000).unwrap();
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

/// AC-3 frame, 48 kHz 64 kb/s (256 bytes), stereo.
fn ac3_frame_64k(k: u8) -> Vec<u8> {
    let mut v = vec![0x0B, 0x77, 0, 0, 0x08, 0x40, 0x40];
    v.extend((7..256).map(|i| (i as u8) ^ k));
    v
}

#[test]
fn mpeg_audio_and_ac3_round_trip_in_transport_and_program_streams() {
    // Smart Cut copies the audio of MPEG-2 sources (WP-09c): MP2 every 24 ms, AC-3 every 32 ms
    let pics = video(10);
    for format in [MuxFormat::Ts, MuxFormat::Ps] {
        let mut m = Muxer::new(Vec::new(), format, &[MuxCodec::Mpeg2Video, MuxCodec::MpegAudio, MuxCodec::Ac3, MuxCodec::MpegAudio], 20_000_000).unwrap();
        let (mut mp2, mut ac3, mut mp2b) = (Vec::new(), Vec::new(), Vec::new());
        for (k, p) in pics.iter().enumerate() {
            m.write(0, p, k as i64 * FRAME, None, k % 5 == 0).unwrap();
        }
        for k in 0..15 {
            let f = mp2_frame(k as u8);
            m.write(1, &f, k * 2160, None, true).unwrap();
            mp2.push(f);
            let f = mp2_frame(100 + k as u8);
            m.write(3, &f, k * 2160, None, true).unwrap();
            mp2b.push(f);
        }
        for k in 0..11 {
            let f = ac3_frame_64k(k as u8);
            m.write(2, &f, k * 2880, None, true).unwrap();
            ac3.push(f);
        }
        let data = m.finish().unwrap();
        let f = open(&data).unwrap();
        assert!(f.warnings.is_empty(), "{format:?}: {:?}", f.warnings);
        let audio: Vec<usize> = (0..f.streams.len()).filter(|&s| f.streams[s].kind() == Kind::Audio).collect();
        assert_eq!(audio.len(), 3, "{format:?}: {:?}", f.streams.iter().map(|s| &s.codec).collect::<Vec<_>>());
        let read = |s: usize| (0..f.streams[s].units.len()).map(|k| f.read_unit(&data, s, k).unwrap()).collect::<Vec<_>>();
        let mpa: Vec<usize> = audio.iter().copied().filter(|&s| f.streams[s].codec == Codec::MpegAudio).collect();
        let a3 = f.find(Kind::Audio, |c| *c == Codec::Ac3).unwrap();
        assert_eq!(mpa.len(), 2, "{format:?}");
        let mut got = [read(mpa[0]), read(mpa[1])];
        got.sort();
        let mut want = [mp2.clone(), mp2b.clone()];
        want.sort();
        assert_eq!(got, want, "{format:?}");
        assert_eq!(read(a3), ac3, "{format:?}");
        let pts: Vec<i64> = f.streams[a3].units.iter().filter_map(|u| u.pts).collect();
        assert_eq!(pts[..3], [0, 2880, 5760].map(|t| Muxer::<Vec<u8>>::DELAY + t), "{format:?}");
    }
}

/// What a P-STD check found in a program stream.
struct PstdReport {
    /// Largest occupancy of any buffer relative to its size (1.0: full).
    peak: f64,
    overflows: usize,
    underflows: usize,
}

/// Replay a program stream through the P-STD (H.222.0 §2.5.2.4): the payload of every PES packet
/// arrives at the end of its pack (`rate` bytes per second after the SCR) and an access unit
/// leaves at its decode time. `size(stream_id)` is the buffer size in bytes; private stream 1 is
/// one buffer.
fn pstd_check(data: &[u8], rate: u64, size: impl Fn(u8) -> usize) -> PstdReport {
    // (arrival 27 MHz, removal 27 MHz, bytes, buffer id, first packet of its unit)
    let mut events: Vec<(i64, i64, usize, u8)> = Vec::new();
    let mut removal: std::collections::HashMap<u8, i64> = std::collections::HashMap::new();
    let mut at = 0;
    let mut scr = 0i64;
    let mut pack_start = 0;
    while at + 4 <= data.len() {
        let id = data[at + 3];
        if data[at..at + 3] != [0, 0, 1] {
            panic!("no start code at {at}");
        }
        match id {
            0xBA => {
                let b = &data[at + 4..at + 14];
                let base = (i64::from(b[0] >> 3 & 7) << 30)
                    | (i64::from(b[0] & 3) << 28)
                    | (i64::from(b[1]) << 20)
                    | (i64::from(b[2] >> 3) << 15)
                    | (i64::from(b[2] & 3) << 13)
                    | (i64::from(b[3]) << 5)
                    | i64::from(b[4] >> 3);
                let ext = i64::from(b[4] & 3) << 7 | i64::from(b[5] >> 1);
                scr = base * 300 + ext;
                pack_start = at;
                at += 14 + usize::from(b[9] & 7);
            }
            0xB9 => break,
            _ => {
                let len = usize::from(data[at + 4]) << 8 | usize::from(data[at + 5]);
                if id != 0xBB && id != 0xBE && id != 0xBF {
                    let hdl = usize::from(data[at + 8]);
                    let flags = data[at + 7];
                    let ts = |o: usize| {
                        let d = &data[at + o..at + o + 5];
                        (i64::from(d[0] >> 1 & 7) << 30)
                            | (i64::from(d[1]) << 22)
                            | (i64::from(d[2] >> 1) << 15)
                            | (i64::from(d[3]) << 7)
                            | i64::from(d[4] >> 1)
                    };
                    if flags & 0xC0 != 0 {
                        // video units leave at their DTS, audio at its PTS
                        let t = if flags & 0x40 != 0 { ts(14) } else { ts(9) };
                        removal.insert(id, t * 300);
                    }
                    let end = at + 6 + len;
                    let arrival = scr + ((end - pack_start) as i64) * 27_000_000 / rate as i64;
                    let buffer = if id == 0xBD { 0xBD } else { id };
                    events.push((arrival, removal.get(&id).copied().unwrap_or(0), len - 3 - hdl, buffer));
                }
                at += 6 + len;
            }
        }
    }
    let mut report = PstdReport { peak: 0.0, overflows: 0, underflows: 0 };
    // last arrival per unit (buffer, removal)
    let mut last: std::collections::HashMap<(u8, i64), i64> = std::collections::HashMap::new();
    for (arrival, rem, _, buffer) in &events {
        let e = last.entry((*buffer, *rem)).or_insert(0);
        *e = (*e).max(*arrival);
    }
    report.underflows = last.iter().filter(|((_, rem), arrival)| *arrival > rem).count();
    for (k, (arrival, _, _, buffer)) in events.iter().enumerate() {
        let held: usize = events[..=k].iter().filter(|e| e.3 == *buffer && e.1 > *arrival).map(|e| e.2).sum();
        let size = size(*buffer);
        report.peak = report.peak.max(held as f64 / size as f64);
        if held > size {
            report.overflows += 1;
        }
    }
    report
}

fn pstd_sizes(id: u8) -> usize {
    match id {
        0xE0..=0xEF => Muxer::<Vec<u8>>::DEFAULT_VIDEO_BUFFER,
        0xBD => 58 * 128,
        _ => 32 * 128,
    }
}

/// 60 big pictures (≈ 4 Mbit/s at 25 fps) with AC-3 beside them, sent into a program stream.
fn big_picture_program_stream(video_buffer: Option<usize>) -> Vec<u8> {
    let mut m = Muxer::new(Vec::new(), MuxFormat::Ps, &[MuxCodec::Mpeg2Video, MuxCodec::Ac3], 10_080_000).unwrap();
    if let Some(b) = video_buffer {
        m.set_video_buffer(b);
    }
    let mut a = 0i64;
    for k in 0..60i64 {
        let p = mpeg2_frame(if k % 12 == 0 { 1 } else { 2 }, (k % 5) as u16, false, 20_000);
        m.write(0, &p, k * FRAME, None, k % 12 == 0).unwrap();
        while a * 2880 < (k + 1) * FRAME {
            m.write(1, &ac3_frame(a as u8), a * 2880, None, true).unwrap();
            a += 1;
        }
    }
    m.finish().unwrap()
}

#[test]
fn program_stream_keeps_the_p_std_buffers() {
    let data = big_picture_program_stream(None);
    let r = pstd_check(&data, 10_080_000 / 8, pstd_sizes);
    assert_eq!(r.overflows, 0, "peak {}", r.peak);
    assert_eq!(r.underflows, 0);
    // 20 KB pictures sent up to 0.7 s early would have filled far more than 232 KB
    assert!(r.peak > 0.5, "the buffer is used: {}", r.peak);
    // the stream still demuxes to the same pictures
    let f = open(&data).unwrap();
    let v = f.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
    assert_eq!(f.streams[v].units.len(), 60);
}

#[test]
fn a_larger_video_buffer_is_announced_and_used() {
    let data = big_picture_program_stream(Some(1194 * 1024));
    // the system header announces 1194 × 1024 bytes: scale bit 1, 13-bit size
    let sys = data.windows(4).position(|w| w == [0, 0, 1, 0xBB]).unwrap();
    let e = &data[sys + 12..sys + 15];
    assert_eq!(e[0], 0xE0);
    assert_eq!(usize::from(e[1] & 0x1F) << 8 | usize::from(e[2]), 1194);
    let r = pstd_check(&data, 10_080_000 / 8, |id| if id == 0xE0 { 1194 * 1024 } else { pstd_sizes(id) });
    assert_eq!((r.overflows, r.underflows), (0, 0));
}

/// Walk a DVD VOB sector by sector: every sector is one pack whose packets end exactly at the
/// sector's end. Returns the (sector, kind) of the navigation packs.
fn dvd_sectors(data: &[u8]) -> Vec<usize> {
    assert_eq!(data.len() % 2048, 0, "whole sectors");
    let mut navs = Vec::new();
    for (n, sector) in data.chunks(2048).enumerate() {
        assert_eq!(&sector[..4], &[0, 0, 1, 0xBA], "sector {n} starts with a pack header");
        assert_eq!(sector[13] & 7, 0, "no pack stuffing");
        let mut at = 14;
        if sector[at..at + 4] == [0, 0, 1, 0xBB] {
            assert_eq!(&sector[at + 4..at + 6], &[0, 0x12]);
            at += 24;
        }
        let mut nav = false;
        while at < 2048 {
            assert_eq!(&sector[at..at + 3], &[0, 0, 1], "sector {n} offset {at}");
            let id = sector[at + 3];
            let len = usize::from(sector[at + 4]) << 8 | usize::from(sector[at + 5]);
            nav |= id == 0xBF;
            at += 6 + len;
        }
        assert_eq!(at, 2048, "sector {n}: packets end with the sector");
        if nav {
            assert_eq!(&sector[0x26..0x2D], &[0, 0, 1, 0xBF, 3, 0xD4, 0]);
            assert_eq!(&sector[0x400..0x407], &[0, 0, 1, 0xBF, 3, 0xFA, 1]);
            navs.push(n);
        }
    }
    navs
}

#[test]
fn dvd_vob_has_whole_sectors_and_a_navigation_pack_per_gop() {
    let lpcm = MuxCodec::LpcmDvd { sample_rate: 48_000, channels: 1, bits: 16 };
    let mut m = Muxer::new_dvd(Vec::new(), &[MuxCodec::Mpeg2Video, MuxCodec::Ac3, lpcm]).unwrap();
    let mut pics = Vec::new();
    let mut a = 0i64;
    for k in 0..45i64 {
        let p = mpeg2_frame(if k % 15 == 0 { 1 } else { 2 }, (k % 5) as u16, false, 1_000 + 37 * k as usize);
        m.write(0, &p, k * FRAME, None, k % 15 == 0).unwrap();
        pics.push(p);
        while a * 2880 < (k + 1) * FRAME {
            m.write(1, &ac3_frame(a as u8), a * 2880, None, true).unwrap();
            a += 1;
        }
        // 10 ms blocks (480 mono samples), the first of every third picture eight samples longer
        for j in 0..4i64 {
            m.write(2, &vec![k as u8; 960 + usize::from(k % 3 == 0 && j == 0) * 16], k * FRAME + j * 900, None, true).unwrap();
        }
    }
    let data = m.finish().unwrap();
    let navs = dvd_sectors(&data);
    assert_eq!(navs.len(), 3, "one per GOP");
    assert_eq!(navs[0], 0, "the first sector is a navigation pack");
    let f = open(&data).unwrap();
    let v = f.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
    for (k, p) in pics.iter().enumerate() {
        assert_eq!(&f.read_unit(&data, v, k).unwrap(), p, "picture {k}");
    }
    let r = pstd_check(&data, 10_080_000 / 8, pstd_sizes);
    assert_eq!((r.overflows, r.underflows), (0, 0));
    assert!(Muxer::new_dvd(Vec::new(), &[MuxCodec::Mpeg2Video, MuxCodec::AacAdts]).is_err());
}
