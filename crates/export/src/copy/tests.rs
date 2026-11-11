use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::*;
use crate::atomic::Overwrite;
use crate::{ExportSettings, Format, Multiplexer, Progress};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-copy-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An H.264 + AAC source written by our own exporter.
fn source(d: &Path, mux: Multiplexer) -> PathBuf {
    let (p, seq, m) = crate::tests::project();
    let ext = if mux == Multiplexer::Mov { "mov" } else { "mp4" };
    let path = d.join(format!("src.{ext}"));
    let s = ExportSettings { format: Format::H264, multiplexer: mux, path: path.to_string_lossy().into_owned(), ..Default::default() };
    crate::export(&p, seq, &s, &m, &Progress::default()).unwrap();
    path
}

fn entries(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn open(p: &Path) -> cfa4_isobmff::Mp4File {
    cfa4_isobmff::open(std::fs::File::open(p).unwrap()).unwrap()
}

/// Every payload of `a`'s track `t` equals `b`'s track `k`.
fn same_payloads(a: &Path, t: usize, b: &Path, k: usize) {
    let (fa, fb) = (std::fs::File::open(a).unwrap(), std::fs::File::open(b).unwrap());
    let (ma, mb) = (cfa4_isobmff::open(&fa).unwrap(), cfa4_isobmff::open(&fb).unwrap());
    assert_eq!(ma.tracks[t].samples.len(), mb.tracks[k].samples.len());
    for i in 0..ma.tracks[t].samples.len() {
        assert_eq!(ma.read_sample(&fa, t, i).unwrap(), mb.read_sample(&fb, k, i).unwrap(), "track {t} sample {i}");
    }
}

#[test]
fn mp4_and_mov_copy_bit_exact_into_mp4_and_mov() {
    let d = dir("exact");
    for src_mux in [Multiplexer::Mp4, Multiplexer::Mov] {
        let src = source(&d, src_mux);
        for (container, ext, faststart) in [(CopyContainer::Mp4, "mp4", false), (CopyContainer::Mp4, "fast.mp4", true), (CopyContainer::Mov, "mov", true)] {
            let out = d.join(format!("out-{src_mux:?}.{ext}"));
            let mut req = StrictCopyRequest::new(&src, &out, container);
            req.faststart = faststart;
            let progress = Progress::default();
            let r = stream_copy_strict(&req, &progress).unwrap();
            assert_eq!(r.mode, "stream_copy_strict");
            assert_eq!(r.validation.result, "PAYLOAD_BIT_EXACT: PASS");
            assert!(r.validation.not_run.iter().any(|w| w.starts_with("VALIDATION_DECODE_NOT_AVAILABLE")));
            assert_eq!(r.tracks.iter().map(|t| t.kind.as_str()).collect::<Vec<_>>(), ["video", "audio"]);
            assert_eq!(
                r.tracks.iter().map(|t| t.codec.as_str()).collect::<Vec<_>>(),
                [open(&src).tracks[0].codec().unwrap().name(), open(&src).tracks[1].codec().unwrap().name()]
            );
            assert_eq!(r.validation.packets, r.tracks.iter().map(|t| t.samples).sum::<u64>());
            assert_eq!(r.validation.payload_bytes, r.tracks.iter().map(|t| t.payload_bytes).sum::<u64>());
            assert_eq!(r.bytes, std::fs::metadata(&out).unwrap().len());
            assert_eq!(progress.done.load(Ordering::Relaxed), progress.total.load(Ordering::Relaxed));
            for t in 0..2 {
                same_payloads(&src, t, &out, t);
                let (a, b) = (open(&src), open(&out));
                let (x, y) = (&a.tracks[t], &b.tracks[t]);
                assert_eq!((x.timescale, x.edit_offset, &x.language), (y.timescale, y.edit_offset, &y.language));
                assert_eq!(
                    x.samples.iter().map(|s| (s.dts, s.pts, s.duration, s.is_sync)).collect::<Vec<_>>(),
                    y.samples.iter().map(|s| (s.dts, s.pts, s.duration, s.is_sync)).collect::<Vec<_>>()
                );
            }
        }
    }
    assert!(!entries(&d).iter().any(|e| e.starts_with(".cfa4-export-")), "no temp file left: {:?}", entries(&d));
}

#[test]
fn selected_tracks_are_copied_in_the_requested_order() {
    let d = dir("select");
    let src = source(&d, Multiplexer::Mp4);
    let out = d.join("audio-first.mp4");
    let mut req = StrictCopyRequest::new(&src, &out, CopyContainer::Mp4);
    req.tracks = Some(vec![1, 0]);
    let r = stream_copy_strict(&req, &Progress::default()).unwrap();
    assert_eq!(r.tracks.iter().map(|t| t.source_track).collect::<Vec<_>>(), [1, 0]);
    same_payloads(&src, 1, &out, 0);
    same_payloads(&src, 0, &out, 1);

    let out = d.join("video-only.mp4");
    let mut req = StrictCopyRequest::new(&src, &out, CopyContainer::Mp4);
    req.tracks = Some(vec![0]);
    let r = stream_copy_strict(&req, &Progress::default()).unwrap();
    assert_eq!(r.tracks.len(), 1);
    assert_eq!(open(&out).tracks.len(), 1);

    for (tracks, code) in [(vec![7], CopyErrorCode::ContainerUnsupported), (vec![0, 0], CopyErrorCode::ContainerUnsupported)] {
        let out = d.join("bad.mp4");
        let mut req = StrictCopyRequest::new(&src, &out, CopyContainer::Mp4);
        req.tracks = Some(tracks);
        assert_eq!(stream_copy_strict(&req, &Progress::default()).unwrap_err().code, code);
        assert!(!out.exists());
    }
}

/// `src` with the video `tkhd` matrix turned 90° and the audio track disabled, in alternate
/// group 1 at half volume (our writer's `tkhd` is version 0).
fn reheadered(src: &Path, to: &Path) {
    let mut b = std::fs::read(src).unwrap();
    let at: Vec<usize> = b.windows(4).enumerate().filter(|(_, w)| *w == b"tkhd").map(|(i, _)| i).collect();
    assert_eq!(at.len(), 2);
    assert!(at.iter().all(|&i| b[i + 4] == 0), "tkhd version 0");
    let m = at[0] + 44;
    for (i, v) in [0i32, 0x1_0000, 0, -0x1_0000, 0, 0, 0, 0, 0x4000_0000].into_iter().enumerate() {
        b[m + 4 * i..m + 4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    let a = at[1];
    b[a + 7] &= !1; // track_enabled
    b[a + 38..a + 40].copy_from_slice(&1u16.to_be_bytes());
    b[a + 40..a + 42].copy_from_slice(&0x80i16.to_be_bytes());
    std::fs::write(to, b).unwrap();
}

#[test]
fn the_track_header_is_kept_and_the_source_is_never_replaced() {
    let d = dir("header");
    let src = source(&d, Multiplexer::Mp4);
    let re = d.join("reheadered.mp4");
    reheadered(&src, &re);
    let f = open(&re);
    assert_ne!(f.tracks[0].matrix, open(&src).tracks[0].matrix);
    assert_eq!((f.tracks[1].enabled, f.tracks[1].alternate_group, f.tracks[1].volume), (false, 1, 0x80));
    for (container, ext) in [(CopyContainer::Mp4, "mp4"), (CopyContainer::Mov, "mov")] {
        let out = d.join(format!("out.{ext}"));
        let r = stream_copy_strict(&StrictCopyRequest::new(&re, &out, container), &Progress::default()).unwrap();
        assert_eq!(r.validation.result, "PAYLOAD_BIT_EXACT: PASS");
        let g = open(&out);
        for t in 0..2 {
            let (x, y) = (&f.tracks[t], &g.tracks[t]);
            assert_eq!((x.matrix, x.enabled, x.alternate_group, x.volume), (y.matrix, y.enabled, y.alternate_group, y.volume), "{ext} track {t}");
        }
    }

    // the output never replaces the source, even when replacing is approved
    let mut req = StrictCopyRequest::new(&src, &src, CopyContainer::Mp4);
    req.overwrite = Overwrite::Replace;
    let before = std::fs::read(&src).unwrap();
    assert_eq!(stream_copy_strict(&req, &Progress::default()).unwrap_err().code, CopyErrorCode::ContainerUnsupported);
    assert_eq!(std::fs::read(&src).unwrap(), before);
}

#[test]
fn a_transport_stream_target_needs_a_payload_change_and_is_refused() {
    let d = dir("ts");
    let src = source(&d, Multiplexer::Mp4);
    for c in [CopyContainer::MpegTs, CopyContainer::M2ts] {
        let out = d.join("out.ts");
        let e = stream_copy_strict(&StrictCopyRequest::new(&src, &out, c), &Progress::default()).unwrap_err();
        assert_eq!(e.code, CopyErrorCode::PayloadModificationRequired);
        assert!(e.to_string().starts_with("COPY_PAYLOAD_MODIFICATION_REQUIRED: "), "{e}");
        assert!(!out.exists());
    }
    let e = stream_copy_strict(&StrictCopyRequest::new(&src, d.join("out.mkv"), CopyContainer::Matroska), &Progress::default()).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::ContainerUnsupported);
}

#[test]
fn an_existing_output_is_kept_unless_replacing_is_approved() {
    let d = dir("overwrite");
    let src = source(&d, Multiplexer::Mp4);
    let out = d.join("out.mp4");
    std::fs::write(&out, b"keep me").unwrap();
    let e = stream_copy_strict(&StrictCopyRequest::new(&src, &out, CopyContainer::Mp4), &Progress::default()).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::IoFailed);
    assert_eq!(std::fs::read(&out).unwrap(), b"keep me");
    let mut req = StrictCopyRequest::new(&src, &out, CopyContainer::Mp4);
    req.overwrite = Overwrite::Replace;
    stream_copy_strict(&req, &Progress::default()).unwrap();
    assert_eq!(open(&out).tracks.len(), 2);
}

#[test]
fn a_cancelled_copy_leaves_nothing_behind() {
    let d = dir("cancel");
    let src = source(&d, Multiplexer::Mp4);
    let before = entries(&d);
    let out = d.join("out.mp4");
    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Relaxed);
    let e = stream_copy_strict(&StrictCopyRequest::new(&src, &out, CopyContainer::Mp4), &progress).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::Cancelled);
    assert_eq!(entries(&d), before);
    assert!(matches!(crate::ExportError::from(e), crate::ExportError::Cancelled));
}

#[test]
fn unreadable_and_corrupt_sources_are_errors_not_panics() {
    let d = dir("bad");
    let out = d.join("out.mp4");
    let missing = stream_copy_strict(&StrictCopyRequest::new(d.join("nope.mp4"), &out, CopyContainer::Mp4), &Progress::default()).unwrap_err();
    assert_eq!(missing.code, CopyErrorCode::IoFailed);
    let text = d.join("text.mp4");
    std::fs::write(&text, b"this is not an mp4 file at all").unwrap();
    let e = stream_copy_strict(&StrictCopyRequest::new(&text, &out, CopyContainer::Mp4), &Progress::default()).unwrap_err();
    assert_eq!(e.code, CopyErrorCode::ContainerUnsupported);

    // every truncation and a spread of flipped bytes of a real file: an error or a valid copy
    let src = source(&d, Multiplexer::Mp4);
    let bytes = std::fs::read(&src).unwrap();
    let hurt = d.join("hurt.mp4");
    let step = (bytes.len() / 97).max(1);
    for cut in (0..bytes.len()).step_by(step) {
        std::fs::write(&hurt, &bytes[..cut]).unwrap();
        let _ = std::fs::remove_file(&out);
        let _ = stream_copy_strict(&StrictCopyRequest::new(&hurt, &out, CopyContainer::Mp4), &Progress::default());
    }
    for at in (0..bytes.len()).step_by(step) {
        let mut b = bytes.clone();
        b[at] ^= 0xA5;
        std::fs::write(&hurt, &b).unwrap();
        let _ = std::fs::remove_file(&out);
        if let Ok(r) = stream_copy_strict(&StrictCopyRequest::new(&hurt, &out, CopyContainer::Mp4), &Progress::default()) {
            assert_eq!(r.validation.result, "PAYLOAD_BIT_EXACT: PASS");
        }
    }
    assert!(!entries(&d).iter().any(|e| e.starts_with(".cfa4-export-")));
}

#[test]
fn error_codes_are_the_stable_sce_001_names() {
    assert_eq!(CopyErrorCode::AtomicCommitFailed.code(), "EXPORT_ATOMIC_COMMIT_FAILED");
    assert_eq!(CopyErrorCode::StrictValidationFailed.code(), "COPY_STRICT_VALIDATION_FAILED");
    assert_eq!(serde_json::to_string(&CopyContainer::MpegTs).unwrap(), "\"mpegTs\"");
}

/// The external oracle reads the copy as the same streams (skipped when ffprobe is missing).
#[test]
fn ffprobe_reads_the_copy() {
    let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("copy::ffprobe_reads_the_copy") else { return };
    let d = dir("ffprobe");
    let src = source(&d, Multiplexer::Mp4);
    let out = d.join("out.mov");
    stream_copy_strict(&StrictCopyRequest::new(&src, &out, CopyContainer::Mov), &Progress::default()).unwrap();
    let o =
        std::process::Command::new(ffprobe).args(["-v", "error", "-show_entries", "stream=codec_name,nb_frames", "-of", "csv=p=0"]).arg(&out).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("h264") && s.contains("aac"), "{s}");
}
