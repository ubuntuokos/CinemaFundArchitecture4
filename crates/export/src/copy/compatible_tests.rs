use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::adapter::{ALLOWLIST, Adapter, AnnexB};
use super::compatible::{adts_equivalent, annexb_equivalent, check_arrival, edit_plan, mux_rate, source_timeline, stamps};
use super::*;
use crate::{ExportSettings, Format, Multiplexer, Progress};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-compat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An MP4 of `format` video and AAC written by our own exporter.
fn source(d: &Path, format: Format) -> PathBuf {
    let (p, seq, m) = crate::tests::project();
    let path = d.join(format!("src-{format:?}.mp4"));
    let s = ExportSettings { format, multiplexer: Multiplexer::Mp4, path: path.to_string_lossy().into_owned(), ..Default::default() };
    crate::export(&p, seq, &s, &m, &Progress::default()).unwrap();
    path
}

fn entries(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn h264_and_hevc_with_aac_copy_into_transport_streams() {
    let d = dir("ts");
    for (format, video, id) in
        [(Format::H264, cfa4_mpegts::Codec::H264, "h264_avcc_to_annexb"), (Format::Hevc, cfa4_mpegts::Codec::Hevc, "hevc_hvcc_to_annexb")]
    {
        let src = source(&d, format);
        for (container, ext, size) in [(CopyContainer::MpegTs, "ts", 188), (CopyContainer::M2ts, "m2ts", 192)] {
            let out = d.join(format!("out-{format:?}.{ext}"));
            let progress = Progress::default();
            let r = stream_copy_compatible(&StrictCopyRequest::new(&src, &out, container), &progress).unwrap();
            assert_eq!(r.mode, "stream_copy_compatible");
            assert_eq!(r.validation.result, "BITSTREAM_EQUIVALENT: PASS");
            assert_eq!(r.transformations.iter().map(|t| t.adapter.as_str()).collect::<Vec<_>>(), [id, "aac_raw_to_adts"]);
            assert_eq!(r.validation.packets, r.tracks.iter().map(|t| t.samples).sum::<u64>());
            assert_eq!(progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
            let bytes = std::fs::read(&out).unwrap();
            assert_eq!(bytes.len() % size, 0);
            assert_eq!(r.bytes, bytes.len() as u64);
            let f = cfa4_mpegts::open(&bytes).unwrap();
            assert!(f.warnings.is_empty(), "{:?}", f.warnings);
            let codecs: Vec<_> = f.streams.iter().map(|s| s.codec.clone()).collect();
            assert_eq!(codecs, [video.clone(), cfa4_mpegts::Codec::AacAdts]);
            // every video unit starts with a delimiter; the first carries the parameter sets
            let v = &f.streams[0];
            assert!(v.units[0].key);
            let first = f.read_unit(&bytes, 0, 0).unwrap();
            let nals = cfa4_bitstream::annexb_nals(&first);
            let sps = if format == Format::Hevc { 33 } else { 7 };
            let ty = |n: &[u8]| if format == Format::Hevc { (n[0] >> 1) & 0x3F } else { n[0] & 0x1F };
            assert_eq!(ty(nals[0]), if format == Format::Hevc { 35 } else { 9 });
            assert!(nals.iter().any(|n| ty(n) == sps));
        }
    }
    assert!(!entries(&d).iter().any(|e| e.starts_with(".cfa4-export-")));
}

#[test]
fn mp4_targets_are_a_strict_copy_and_other_codecs_are_refused() {
    let d = dir("other");
    let src = source(&d, Format::H264);
    let r = stream_copy_compatible(&StrictCopyRequest::new(&src, d.join("out.mp4"), CopyContainer::Mp4), &Progress::default()).unwrap();
    assert_eq!((r.mode.as_str(), r.validation.result.as_str()), ("stream_copy_compatible", "PAYLOAD_BIT_EXACT: PASS"));
    assert!(r.transformations.is_empty());

    // AV1 has no allowlisted transport stream adapter
    let (p, seq, m) = crate::tests::project();
    let av1 = d.join("av1.mp4");
    let s = ExportSettings { format: Format::Av1, path: av1.to_string_lossy().into_owned(), include_audio: false, ..Default::default() };
    crate::export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let out = d.join("av1.ts");
    let e = stream_copy_compatible(&StrictCopyRequest::new(&av1, &out, CopyContainer::MpegTs), &Progress::default()).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::CodecUnsupported, "{e}");
    assert!(!out.exists());
}

#[test]
fn a_cancelled_or_refused_copy_leaves_nothing() {
    let d = dir("cancel");
    let src = source(&d, Format::H264);
    let before = entries(&d);
    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Relaxed);
    let e = stream_copy_compatible(&StrictCopyRequest::new(&src, d.join("out.ts"), CopyContainer::MpegTs), &progress).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::Cancelled);
    std::fs::write(d.join("taken.ts"), b"keep").unwrap();
    let e = stream_copy_compatible(&StrictCopyRequest::new(&src, d.join("taken.ts"), CopyContainer::MpegTs), &Progress::default()).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::IoFailed);
    assert_eq!(std::fs::read(d.join("taken.ts")).unwrap(), b"keep");
    let mut want = before;
    want.push("taken.ts".into());
    want.sort();
    assert_eq!(entries(&d), want);
}

fn lp(nals: &[&[u8]]) -> Vec<u8> {
    nals.iter().flat_map(|n| (n.len() as u32).to_be_bytes().into_iter().chain(n.iter().copied())).collect()
}

#[test]
fn annex_b_adapter_inserts_only_what_it_must_and_the_validator_checks_it() {
    let a = AnnexB { hevc: false, length_size: 4, parameter_sets: vec![vec![0x67, 1, 2], vec![0x68, 3]] };
    let ad = Adapter::AnnexB(a.clone());
    let idr: &[u8] = &[0x65, 9, 9];
    let p: &[u8] = &[0x41, 7];
    // sync sample without parameter sets: delimiter, parameter sets, slice
    let src = lp(&[idr]);
    let out = ad.apply(&src, true).unwrap();
    assert_eq!(out, [&[0, 0, 0, 1, 0x09, 0xF0][..], &[0, 0, 0, 1, 0x67, 1, 2], &[0, 0, 0, 1, 0x68, 3], &[0, 0, 0, 1, 0x65, 9, 9]].concat());
    assert_eq!(annexb_equivalent(&a, &src, &out, true), Ok(()));
    // the validator refuses changed parameter sets, a missing delimiter, missing parameter sets
    let mut bad = out.clone();
    bad[10] ^= 1;
    assert!(annexb_equivalent(&a, &src, &bad, true).is_err());
    assert!(annexb_equivalent(&a, &src, &out[6..], true).is_err());
    let bare = [&[0, 0, 0, 1, 0x09, 0xF0][..], &[0, 0, 0, 1, 0x65, 9, 9]].concat();
    assert!(annexb_equivalent(&a, &src, &bare, true).is_err());
    // … and parameter sets put before a non-sync sample
    assert!(annexb_equivalent(&a, &src, &out, false).is_err());
    // a sample with its own delimiter and parameter sets keeps them and gets nothing inserted
    let own: [&[u8]; 4] = [&[0x09, 0x10], &[0x67, 1, 2], &[0x68, 3], idr];
    let src = lp(&own);
    assert_eq!(a.insertions(&a.source_nals(&src).unwrap(), true), (false, false));
    let out = ad.apply(&src, true).unwrap();
    assert_eq!(out, own.iter().flat_map(|n| [0, 0, 0, 1].iter().chain(n.iter()).copied()).collect::<Vec<_>>());
    assert_eq!(annexb_equivalent(&a, &src, &out, true), Ok(()));
    // an SPS without a PPS is not enough to start decoding: the sample entry's sets go in
    let src = lp(&[&[0x67, 1, 2], idr]);
    assert_eq!(a.insertions(&a.source_nals(&src).unwrap(), true), (true, true));
    let out = ad.apply(&src, true).unwrap();
    assert_eq!(annexb_equivalent(&a, &src, &out, true), Ok(()));
    // an in-band parameter set the sample entry does not have is refused, not overridden later
    let e = ad.apply(&lp(&[&[0x68, 4], idr]), true).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::BitstreamUnsupported);
    // a non-sync sample gets the delimiter only
    let src = lp(&[p]);
    let out = ad.apply(&src, false).unwrap();
    assert_eq!(out, [&[0, 0, 0, 1, 0x09, 0xF0][..], &[0, 0, 0, 1, 0x41, 7]].concat());
    assert_eq!(annexb_equivalent(&a, &src, &out, false), Ok(()));
    // NAL units Annex B could not carry unchanged are refused, not altered
    for nal in [&[0x41, 0][..], &[0x41, 0, 0, 1, 5], &[]] {
        let e = ad.apply(&lp(&[nal]), false).unwrap_err();
        assert_eq!(e.code, CopyErrorCode::BitstreamUnsupported);
    }
    assert!(ad.apply(&[0, 0, 0, 9, 1], false).is_err());
    assert!(ALLOWLIST.iter().any(|x| x.id == ad.allowed().id));

    // HEVC: VPS, SPS and PPS are needed; the delimiter carries the picture's TemporalId
    let h = AnnexB { hevc: true, length_size: 4, parameter_sets: vec![vec![0x40, 1, 7], vec![0x42, 1, 8], vec![0x44, 1, 9]] };
    let trail: &[u8] = &[0x02, 0x03, 5]; // TRAIL_R, TemporalId 2
    let src = lp(&[trail]);
    let out = Adapter::AnnexB(h.clone()).apply(&src, false).unwrap();
    assert_eq!(&out[..7], &[0, 0, 0, 1, 0x46, 0x03, 0x50]);
    assert_eq!(annexb_equivalent(&h, &src, &out, false), Ok(()));
    let src = lp(&[&[0x42, 1, 8], &[0x26, 1, 4]]);
    let out = Adapter::AnnexB(h.clone()).apply(&src, true).unwrap();
    assert_eq!(cfa4_bitstream::annexb_nals(&out).len(), 6);
    assert_eq!(annexb_equivalent(&h, &src, &out, true), Ok(()));
}

#[test]
fn adts_adapter_frames_exactly_what_adts_can_signal() {
    // AAC-LC, 48 kHz, stereo
    let asc = [0x11, 0x90];
    let cfg = cfa4_isobmff::CodecConfig::Aac(cfa4_isobmff::AacConfig::from_asc(asc.to_vec()));
    let a = Adapter::for_codec(&cfg).unwrap();
    let frame = [1u8, 2, 3, 4, 5];
    let out = a.apply(&frame, true).unwrap();
    assert_eq!(out.len(), 12);
    assert_eq!(&out[..2], &[0xFF, 0xF1]);
    assert!(adts_equivalent(&asc, &frame, &out));
    for (i, bit) in [(2, 0x40), (3, 0x40), (4, 0x01), (6, 0x01)] {
        let mut bad = out.clone();
        bad[i] ^= bit;
        assert!(!adts_equivalent(&asc, &frame, &bad), "byte {i}");
    }
    assert!(!adts_equivalent(&asc, &frame, &out[..11]));
    assert!(!adts_equivalent(&[0x12, 0x10], &frame, &out));
    // HE-AAC v2 (object type 29), 960-sample frames and a core coder dependency cannot be
    // signalled in ADTS
    for asc in [&[0xEB, 0x09, 0x88][..], &[0x12, 0x14], &[0x12, 0x12]] {
        let c = cfa4_isobmff::CodecConfig::Aac(cfa4_isobmff::AacConfig::from_asc(asc.to_vec()));
        assert_eq!(Adapter::for_codec(&c).unwrap_err().code, CopyErrorCode::TransformationNotAllowed, "{asc:02x?}");
    }
}

fn opened(path: &Path) -> cfa4_isobmff::Mp4File {
    cfa4_isobmff::open(std::fs::File::open(path).unwrap()).unwrap()
}

/// An edit showing track `t` of `f` from `media_time` to the end of its media.
fn edit_from(f: &cfa4_isobmff::Mp4File, t: usize, media_time: i64) -> cfa4_isobmff::Edit {
    let tr = &f.tracks[t];
    let end = tr.samples.iter().map(|s| s.pts + i64::from(s.duration)).max().unwrap();
    let segment_duration = ((end - media_time) as u128 * u128::from(f.timescale) / u128::from(tr.timescale)) as u64;
    cfa4_isobmff::Edit { segment_duration, media_time, media_rate: 0x1_0000 }
}

#[test]
fn edit_lists_that_hide_media_are_refused_and_aac_priming_is_kept_and_reported() {
    let d = dir("edits");
    let mut f = opened(&source(&d, Format::H264));
    let (v, a) = (0, 1);
    assert_eq!(f.tracks[a].kind, cfa4_isobmff::TrackKind::Audio);
    let first = |f: &cfa4_isobmff::Mp4File, t: usize| f.tracks[t].samples.iter().map(|s| s.pts).min().unwrap();
    let frame = i64::from(f.tracks[a].samples[0].duration);

    // video: an edit starting one frame in hides that frame; the copy would show it
    let vf = first(&f, v) + i64::from(f.tracks[v].samples[0].duration);
    f.tracks[v].edits = vec![edit_from(&f, v, vf)];
    let e = edit_plan(&f, v, &f.tracks[v], false).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::TimestampInvalid, "{e}");
    assert!(source_timeline(&f, &f.tracks[v], false).is_err());
    // … an edit from the first presented frame is fine
    f.tracks[v].edits = vec![edit_from(&f, v, first(&f, v))];
    assert_eq!(edit_plan(&f, v, &f.tracks[v], false).unwrap().priming, 0);

    // audio: one frame of encoder priming is kept and reported; four frames are not priming
    f.tracks[a].edits = vec![edit_from(&f, a, first(&f, a) + frame)];
    assert_eq!(edit_plan(&f, a, &f.tracks[a], true).unwrap().priming, frame);
    assert!(source_timeline(&f, &f.tracks[a], true).is_ok());
    f.tracks[a].edits = vec![edit_from(&f, a, first(&f, a) + 4 * frame)];
    assert!(edit_plan(&f, a, &f.tracks[a], true).is_err());
    assert!(source_timeline(&f, &f.tracks[a], true).is_err());

    // a cut-off end, another rate, several media edits, a hostile media time: refused, no panic
    let mut cut = edit_from(&f, v, first(&f, v));
    cut.segment_duration /= 2;
    let mut slow = edit_from(&f, v, first(&f, v));
    slow.media_rate = 0x8000;
    let whole = edit_from(&f, v, first(&f, v));
    for edits in [
        vec![cut],
        vec![slow],
        vec![whole, whole],
        vec![cfa4_isobmff::Edit { segment_duration: u64::MAX, media_time: i64::MIN + 2, media_rate: 0x1_0000 }],
        vec![cfa4_isobmff::Edit { segment_duration: u64::MAX, media_time: i64::MAX, media_rate: 0x1_0000 }],
        vec![cfa4_isobmff::Edit { segment_duration: 10, media_time: -1, media_rate: 0x1_0000 }],
    ] {
        f.tracks[v].edits = edits.clone();
        let e = edit_plan(&f, v, &f.tracks[v], false).unwrap_err();
        assert_eq!(e.code, CopyErrorCode::TimestampInvalid, "{edits:?}");
        assert!(source_timeline(&f, &f.tracks[v], false).is_err(), "{edits:?}");
    }
    // timestamps beyond the transport stream's range are refused, not wrapped
    f.tracks[v].edits = vec![cfa4_isobmff::Edit { segment_duration: u64::MAX / 2, media_time: -1, media_rate: 0x1_0000 }, whole];
    f.tracks[a].edits.clear();
    let plans = [edit_plan(&f, v, &f.tracks[v], false).unwrap(), edit_plan(&f, a, &f.tracks[a], true).unwrap()];
    assert_eq!(stamps(&plans, &f, &[v, a]).unwrap_err().code, CopyErrorCode::TimestampInvalid);
    f.tracks[v].edits.clear();
    f.tracks[v].samples[3].pts = i64::MAX;
    let plans = [edit_plan(&f, v, &f.tracks[v], false).unwrap(), edit_plan(&f, a, &f.tracks[a], true).unwrap()];
    assert_eq!(stamps(&plans, &f, &[v, a]).unwrap_err().code, CopyErrorCode::TimestampInvalid);
}

/// Mux units of `f`'s tracks sized at their upper bound at `rate` into a file and run the arrival
/// check on it.
fn mux_bound(d: &Path, f: &cfa4_isobmff::Mp4File, adapters: &[Adapter], ts: &[Vec<(i64, i64)>], rate: u64) -> Result<(), CopyError> {
    use std::io::Write;
    let path = d.join(format!("burst-{rate}.ts"));
    let file = std::fs::File::create(&path).unwrap();
    let codecs = [cfa4_mpegts::MuxCodec::H264, cfa4_mpegts::MuxCodec::AacAdts];
    let mut m = cfa4_mpegts::Muxer::new(std::io::BufWriter::new(file), cfa4_mpegts::MuxFormat::Ts, &codecs, rate).unwrap();
    let mut order: Vec<(i64, usize, usize)> = ts.iter().enumerate().flat_map(|(k, v)| v.iter().enumerate().map(move |(i, &(_, dts))| (dts, k, i))).collect();
    order.sort();
    for (_, k, i) in order {
        let s = f.tracks[k].samples[i];
        let n = match &adapters[k] {
            Adapter::AnnexB(_) => s.size as usize + 8,
            Adapter::Adts(_) => s.size as usize + 7,
        };
        let mut data = if k == 0 { vec![0, 0, 0, 1, 0x09, 0xF0] } else { vec![0xFF, 0xF1] };
        data.resize(n, 0x55);
        let (p, dts) = ts[k][i];
        m.write(k, &data, p, Some(dts), s.is_sync).unwrap();
    }
    m.finish().unwrap().flush().unwrap();
    check_arrival(&std::fs::File::open(&path).unwrap(), 188, &[0x100, 0x101])
}

#[test]
fn the_multiplex_rate_delivers_a_burst_across_a_second_boundary_in_time() {
    let d = dir("rate");
    let src = source(&d, Format::H264);
    let mut f = opened(&src);
    // 0.8 s of 150 kB pictures from 0.6 s on: the busiest calendar second holds only part of it
    let vts = i64::from(f.tracks[0].timescale);
    for s in &mut f.tracks[0].samples {
        if (vts * 6 / 10..vts * 14 / 10).contains(&s.dts) {
            s.size = 150_000;
        }
    }
    let tracks = [0, 1];
    let adapters: Vec<Adapter> = tracks.iter().map(|&t| Adapter::for_codec(f.tracks[t].codec().unwrap()).unwrap()).collect();
    let plans: Vec<_> = tracks.iter().map(|&t| edit_plan(&f, t, &f.tracks[t], t == 1).unwrap()).collect();
    let ts = stamps(&plans, &f, &tracks).unwrap();
    let rate = mux_rate(&f, &tracks, &adapters, &ts, cfa4_mpegts::MuxFormat::Ts).unwrap();
    mux_bound(&d, &f, &adapters, &ts, rate).unwrap();
    // the old estimate (busiest calendar second + 25 %) lets the clock fall behind; the check sees it
    let e = mux_bound(&d, &f, &adapters, &ts, rate / 2).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::CompatibleValidationFailed, "{e}");
    assert!(e.message.contains("multiplex rate is too low"), "{e}");
}

/// External oracle: ffmpeg-made H.264 (B-frames) / HEVC + AAC MP4s copied into transport streams
/// decode to the same pictures and samples as the source (skipped without ffmpeg or encoders).
#[test]
fn ffmpeg_decodes_the_copy_to_the_source_frames() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("copy::compatible_tests::ffmpeg_decodes_the_copy_to_the_source_frames") else { return };
    let d = dir("oracle");
    for enc in ["libx264", "libx265"] {
        if !cfa4_testkit::ffmpeg_has_encoder(&ffmpeg, enc) || !cfa4_testkit::ffmpeg_has_encoder(&ffmpeg, "aac") {
            eprintln!("skip {enc}: encoder missing");
            continue;
        }
        let src = d.join(format!("{enc}.mp4"));
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=2", "-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args(["-c:v", enc, "-g", "12", "-bf", "2", "-c:a", "aac", "-b:a", "128k"])
            .args(if enc == "libx265" { &["-x265-params", "log-level=error"][..] } else { &[] })
            .arg(&src)
            .status()
            .unwrap();
        assert!(ok.success());
        let out = d.join(format!("{enc}.ts"));
        let r = stream_copy_compatible(&StrictCopyRequest::new(&src, &out, CopyContainer::MpegTs), &Progress::default()).unwrap();
        assert_eq!(r.validation.result, "BITSTREAM_EQUIVALENT: PASS");
        let md5 = |p: &Path, map: &str| {
            let o = std::process::Command::new(&ffmpeg).args(["-v", "error", "-i"]).arg(p).args(["-map", map, "-f", "framemd5", "-"]).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.starts_with('#'))
                .map(|l| l.rsplit(',').next().unwrap_or("").trim().to_string())
                .collect::<Vec<_>>()
        };
        let (x, y) = (md5(&src, "0:v"), md5(&out, "0:v"));
        assert!(!x.is_empty());
        assert_eq!(x, y, "{enc} video");
        // the MP4 edit list hides the AAC encoder priming; the transport stream has no edit list
        // and keeps it, reported and stamped just before time zero: exactly those frames are extra
        let f = opened(&src);
        let tr = &f.tracks[1];
        let hidden = tr.edits.iter().find(|e| e.media_time >= 0).map_or(0, |e| e.media_time - tr.samples[0].pts);
        let primed =
            usize::try_from(hidden.div_euclid(i64::from(tr.samples[0].duration)) + i64::from(hidden % i64::from(tr.samples[0].duration) != 0)).unwrap();
        assert_eq!(r.transformations[1].notes.len(), usize::from(primed > 0), "{:?}", r.transformations);
        assert!(r.transformations[1].notes.iter().all(|n| n.starts_with("AAC_PRIMING_KEPT")));
        let (x, y) = (md5(&src, "0:a"), md5(&out, "0:a"));
        assert!(!x.is_empty());
        assert_eq!(y.len(), x.len() + primed, "{enc} audio");
        assert_eq!(&y[primed..], x.as_slice(), "{enc} audio");

        // a copy cut 1.3 s in keeps the GOP before the cut and hides it with its edit list: a
        // transport stream would show it, so that is refused
        let trim = d.join(format!("{enc}-trim.mp4"));
        let ok =
            std::process::Command::new(&ffmpeg).args(["-v", "error", "-y", "-ss", "1.3", "-i"]).arg(&src).args(["-c", "copy"]).arg(&trim).status().unwrap();
        assert!(ok.success());
        let out = d.join(format!("{enc}-trim.ts"));
        let e = stream_copy_compatible(&StrictCopyRequest::new(&trim, &out, CopyContainer::MpegTs), &Progress::default()).unwrap_err();
        assert_eq!(e.code, CopyErrorCode::TimestampInvalid, "{e}");
        assert!(!out.exists());
    }

    // lossless noise from 0.6 s to 1.4 s: a burst across a second boundary still arrives before
    // its decode times (the validator checks every packet against the PCR clock)
    if cfa4_testkit::ffmpeg_has_encoder(&ffmpeg, "libx264") {
        let src = d.join("burst.mp4");
        let ok = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=2.5,noise=alls=100:allf=t:enable='between(t,0.6,1.4)'"])
            .args(["-c:v", "libx264", "-qp", "0", "-g", "25"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(ok.success());
        let r = stream_copy_compatible(&StrictCopyRequest::new(&src, d.join("burst.ts"), CopyContainer::MpegTs), &Progress::default()).unwrap();
        assert_eq!(r.validation.result, "BITSTREAM_EQUIVALENT: PASS");
    }
}
