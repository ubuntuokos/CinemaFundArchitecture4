//! Musepack `.mpc` files read through the media source: ffmpeg's FATE samples (`inside-mp7.mpc`,
//! `inside-mp8.mpc` from `https://fate-suite.ffmpeg.org/musepack/`) in `$MUSEPACK_SAMPLES` or
//! `target/fixtures/musepack/fate` (optional local fixtures: a missing sample is not a missing oracle). Every sample, read front to back and from the middle, equals
//! the `cfa4-musepack` decode (which `crates/musepack/tests/oracle.rs` checks against ffmpeg).

use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn mpc_files_read_through_the_media_source() {
    let dir = std::env::var_os("MUSEPACK_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("musepack/fate"));
    for (name, version) in [("inside-mp7.mpc", 7), ("inside-mp8.mpc", 8)] {
        let Ok(data) = std::fs::read(dir.join(name)) else {
            eprintln!("{name}: optional FATE sample not present");
            continue;
        };
        let (_, planes) = cfa4_musepack::decode_file(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        let s = cfa4_codecs::open_bytes(name, Arc::from(data)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
        assert_eq!((info.sample_rate, info.channels), (44100, 2), "{name}");
        assert_eq!(info.codec, format!("Musepack SV{version}"), "{name}: codec label");
        let frames = planes[0].len();
        for from in [0, frames / 2 + 123] {
            let n = 5000.min(frames - from);
            let b = s.audio(from as i64, n, 44100).unwrap_or_else(|e| panic!("{name}: {e}"));
            for (c, p) in b.channels.iter().enumerate() {
                assert!(p[..n] == planes[c][from..from + n], "{name}: channel {c} from {from} differs");
            }
        }
    }
}
