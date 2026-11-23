//! The restricted-extras package is not installed (ADR 0013, VVC-TEST-022): VVC stays visible
//! and is switched off with a clear reason, and opening a VVC track is an error, not a crash.
//! A test binary of its own: it sets the helper environment variable for the whole process.
//! Needs no helper, FFmpeg or fixture, so it always runs.

use cfa4_codecs::vvc_external::{self, BACKEND_ID, RESTRICTED_PACKAGE};
use cfa4_codecs::{CodecError, make_video_decoder};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry, VvcConfig};

fn vvc_entry() -> SampleEntry {
    SampleEntry {
        format: FourCc(*b"vvc1"),
        data_reference_index: 1,
        codec: CodecConfig::Vvc(VvcConfig { length_size: 4, ..Default::default() }),
        video: None,
        audio: None,
        bitrate: None,
    }
}

#[test]
fn a_missing_helper_is_reported_and_refused_not_fatal() {
    // the report: visible, not available, with the reason and the package
    let r = vvc_external::report_for(None);
    assert_eq!(r.id, BACKEND_ID);
    assert!(r.compiled_in);
    assert!(!r.available, "{r:?}");
    assert!(r.problem.as_deref().is_some_and(|p| p.contains(RESTRICTED_PACKAGE)), "{r:?}");
    assert_eq!(r.restricted_package.as_deref(), Some(RESTRICTED_PACKAGE));
    assert_eq!(r.restricted.len(), 1);
    assert_eq!(r.restricted[0].name, "VVdeC decoder");
    assert!(!r.restricted[0].available);

    // a path that does not exist, and a program that is not the helper: reported, not fatal
    for bad in ["/nonexistent/cfa4-vvc-decode", "/bin/true"] {
        let path = std::path::Path::new(bad);
        let r = vvc_external::report_for(Some(path));
        assert!(!r.available && r.problem.is_some() && !r.restricted[0].available, "{bad}: {r:?}");
        match vvc_external::vvc_factory_for(Some(path), &vvc_entry()) {
            Some(Err(_)) => {}
            Some(Ok(_)) => panic!("{bad}: a decoder from a program that is not the helper"),
            None => panic!("{bad}: the factory ignored a VVC entry"),
        }
    }

    // no helper: the factory leaves the entry alone and the registry gives the error that
    // names the package
    assert!(vvc_external::vvc_factory_for(None, &vvc_entry()).is_none());

    // registered like any backend, so Settings ▸ Media Backends lists it (whatever this machine has)
    vvc_external::register_status();
    assert!(cfa4_media::backend_reports().iter().any(|b| b.id == BACKEND_ID));

    // the real registry on a machine without a helper (CI): the clear error
    if vvc_external::configured_helper().is_none() {
        match make_video_decoder(&vvc_entry()) {
            Err(CodecError::Unsupported(msg)) => assert!(msg.contains(RESTRICTED_PACKAGE), "{msg}"),
            Err(e) => panic!("unexpected error {e}"),
            Ok(_) => panic!("a decoder without a helper"),
        }
    }
}
