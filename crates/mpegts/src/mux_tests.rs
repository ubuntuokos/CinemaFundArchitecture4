use super::tests::{adts_frame, mpeg2_frame};
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
