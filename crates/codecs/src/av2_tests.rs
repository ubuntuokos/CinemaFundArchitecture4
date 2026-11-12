//! AV2 identification through the real open paths (AV2 plan X/4): an AV2 file is recognised and
//! answered as unsupported, never opened as something else, and never crashes.

use std::sync::Arc;

use cfa4_isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_media::{FrameRequest, MediaError};
use cfa4_time::Tick;

/// A temporal delimiter, a sequence header OBU and a closed-loop key OBU (Annex B framing).
const ANNEX_B: &[u8] = &[0x01, 0x08, 0x04, 0x04, 0xaa, 0xbb, 0xcc, 0x03, 0x10, 0x01, 0x02];

fn unsupported(r: Result<cfa4_media::SharedSource, MediaError>) -> String {
    match r {
        Err(MediaError::Unsupported(m)) => m,
        Err(e) => panic!("expected Unsupported, got {e}"),
        Ok(_) => panic!("expected Unsupported, the file opened"),
    }
}

#[test]
fn annex_b_and_ivf_files_are_answered_as_unsupported_av2() {
    let m = unsupported(crate::open_bytes("clip.obu", ANNEX_B.into()));
    assert!(m.contains("AV2") && m.contains("Annex B"), "{m}");
    let reader: cfa4_media::SharedReader = Arc::new(cfa4_media::reader::MemReader(ANNEX_B.into()));
    let m = unsupported(cfa4_media::reader::open_reader("clip.obu", reader, &crate::reader_openers(), &crate::openers()));
    assert!(m.contains("AV2"), "{m}");

    let mut ivf = b"DKIF\0\0\x20\0AV02".to_vec();
    ivf.resize(32, 0);
    ivf.extend_from_slice(&[11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    ivf.extend_from_slice(ANNEX_B);
    let m = unsupported(crate::open_bytes("clip.ivf", ivf.into()));
    assert!(m.contains("AV2") && m.contains("IVF"), "{m}");
}

#[test]
fn mp4_av02_track_is_identified_and_its_frames_are_unsupported() {
    let mut mux = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("writer");
    let vt = mux.add_track(TrackConfig::new(SampleEntry::av2(vec![0x81], 64, 48), 24)).expect("track");
    mux.write_sample(vt, WriteSample { data: &ANNEX_B[2..], duration: 1, composition_offset: 0, is_sync: true }).expect("sample");
    let bytes: Arc<[u8]> = mux.finish().expect("finish").into_inner().into();
    let src = crate::open_bytes("clip.mp4", bytes).expect("an MP4 with an AV2 track still opens (its audio, its info)");
    let v = src.info().video.as_ref().expect("video info");
    assert_eq!(v.codec, "AV2 (unsupported)");
    assert_eq!((v.width, v.height), (64, 48));
    match src.video_frame(FrameRequest::full(Tick::ZERO)) {
        Err(e) => assert!(e.to_string().contains("AV2"), "{e}"),
        Ok(_) => panic!("an AV2 frame decoded without an AV2 decoder"),
    }
}

/// The AVM v1.0.0 corpus bitstreams (`target/fixtures/av2/vectors/*.obu`, made by the AV2 oracle
/// harness), when they have been generated; otherwise nothing to check.
#[test]
fn avm_corpus_vectors_are_recognised() {
    let dir = cfa4_testkit::fixtures::fixtures_root().join("av2").join("vectors");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("SKIPPED (av2_tests::avm_corpus_vectors_are_recognised): no AV2 corpus in {}", dir.display());
        return;
    };
    for path in entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "obu")) {
        let bytes: Arc<[u8]> = std::fs::read(&path).expect("read vector").into();
        assert_eq!(crate::av2::sniff(&bytes), Some(crate::av2::Framing::AnnexB), "{}", path.display());
        let m = unsupported(crate::open_bytes("vector.obu", bytes));
        assert!(m.contains("AV2"), "{}: {m}", path.display());
    }
}
