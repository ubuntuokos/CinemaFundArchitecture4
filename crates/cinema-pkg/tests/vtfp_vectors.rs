//! The test compositions of the IMF virtual track fingerprint memo (cinecert/imf-vtfp, commit
//! `3528343079a5ec9565a4bf4aa17674c6cdbb1955`, `test/`; BSD-2-Clause, `tests/data/imf-vtfp/LICENSE.md`).
//! Each file carries its expected fingerprint in `Annotation`: files 1–4 lay out the same
//! `MainImage` timeline in four ways, file 5 is a stereoscopic track.

use std::path::Path;

use cfa4_cinema_pkg::vtfp::{self, urns_match};

fn load(n: u32) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/data/imf-vtfp/vt-fp-test-{n}.cpl.xml"));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn annotation(xml: &str) -> &str {
    let start = xml.find("<Annotation>").unwrap() + "<Annotation>".len();
    let end = xml[start..].find("</Annotation>").unwrap() + start;
    &xml[start..end]
}

#[test]
fn memo_test_compositions() {
    for n in 1..=5 {
        let xml = load(n);
        let tracks = vtfp::virtual_tracks(&xml).unwrap();
        assert_eq!(tracks.len(), 1, "file {n}: {tracks:?}");
        assert_eq!(tracks[0].kind, "MainImageSequence");
        let fp = vtfp::fingerprint(&xml, &tracks[0].track_id).unwrap();
        assert_eq!(fp.to_urn(), annotation(&xml), "file {n}");
    }
}

#[test]
fn same_timeline_same_fingerprint() {
    let urns: Vec<String> = (1..=4)
        .map(|n| {
            let xml = load(n);
            let id = vtfp::virtual_tracks(&xml).unwrap()[0].track_id.clone();
            vtfp::fingerprint(&xml, &format!("urn:uuid:{id}")).unwrap().to_urn()
        })
        .collect();
    assert!(urns.iter().all(|u| u == "urn:smpte:imf-vtfp:11cbefc227319bf4708a6f0cc228a968ecf7c65b"), "{urns:?}");
}

#[test]
fn abbreviated_urns() {
    let full = "urn:smpte:imf-vtfp:190fb72cddad682227cb3f075db9dfb558447a03";
    assert!(urns_match(full, "urn:smpte:imf-vtfp:190fb72cdd").unwrap());
    assert!(urns_match("urn:smpte:imf-vtfp:190f", full).unwrap());
    assert!(!urns_match(full, "urn:smpte:imf-vtfp:190fb72cde").unwrap());
    // Too short, upper case, wrong prefix.
    assert!(urns_match(full, "urn:smpte:imf-vtfp:190").is_err());
    assert!(urns_match(full, "urn:smpte:imf-vtfp:190FB72CDD").is_err());
    assert!(urns_match(full, "urn:smpte:mxf-digest:190fb72cdd").is_err());
}
