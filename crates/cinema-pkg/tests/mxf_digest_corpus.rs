//! The MXF-DIGEST memo's own test file (cinecert/mxf-digest, commit
//! `512e8a9723b8ffd4181a4e2e443fb7e371cc17ce`, `test/oneframe.pcm.mxf`; BSD-2-Clause). It is not
//! committed: `bash scripts/dev/setup-cinema-oracles.sh --corpus` fetches it into the corpus
//! directory, which `CFA4_CINEMA_CORPUS` names. Without it the test passes with a note.

use std::path::PathBuf;

use cfa4_cinema_pkg::mxf_digest;

const EXPECTED: &str = "urn:smpte:mxf-digest:64pMA4dgr8iLqAEkiMpfJv2JHLubLY9wpDUeAcr3pto3gKGsszyCqr9ofBk668EJrVNagTW7WujyYZV9YEUqCRGE";

#[test]
fn memo_reference_file() {
    let Some(dir) = std::env::var_os("CFA4_CINEMA_CORPUS") else {
        eprintln!("skipped: CFA4_CINEMA_CORPUS is not set (scripts/dev/setup-cinema-oracles.sh --corpus)");
        return;
    };
    let path = PathBuf::from(dir).join("oneframe.pcm.mxf");
    let Ok(file) = std::fs::File::open(&path) else {
        eprintln!("skipped: {} is missing", path.display());
        return;
    };
    let (digest, packets) = mxf_digest(std::io::BufReader::new(file)).unwrap();
    assert!(packets > 0);
    assert_eq!(digest.to_urn(), EXPECTED);
}
