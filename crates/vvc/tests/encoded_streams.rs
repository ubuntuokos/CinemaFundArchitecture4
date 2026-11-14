//! Streams from a real encoder: FFmpeg with `libvvenc` (VVenC) writes a short Annex B stream into
//! `target/fixtures/vvc/`, and the parser must find every access unit, the IRAP pictures and the
//! SPS geometry. `libvvenc` is a restricted component (`cfa4-restricted-extras`, ADR 0013), so an
//! FFmpeg without it skips this test even with `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_bitstream::annexb_nals;
use cfa4_testkit::fixtures::generate;
use cfa4_testkit::{ffmpeg_has_encoder, ffmpeg_or_skip, fixtures_dir};
use cfa4_vvc::{NalHeader, NalType, Sps, access_units, annexb_access_units, au_info};

const FRAMES: usize = 24;
/// Not a multiple of the 8-sample coding granularity: the SPS needs a conformance window.
const WIDTH: u32 = 330;
const HEIGHT: u32 = 202;

fn fixture(ff: &Path) -> Option<PathBuf> {
    let out = fixtures_dir("vvc").join(format!("testsrc-{WIDTH}x{HEIGHT}-{FRAMES}f-ip8.266"));
    generate(&out, |tmp| {
        Command::new(ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=24"))
            .args(["-frames:v", &FRAMES.to_string(), "-pix_fmt", "yuv420p10le", "-c:v", "libvvenc", "-preset", "faster", "-qp", "32"])
            .args(["-vvenc-params", "IntraPeriod=8", "-f", "vvc"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn vvenc_stream_access_units_and_sps() {
    let Some(ff) = ffmpeg_or_skip("vvc::encoded_streams") else { return };
    if !ffmpeg_has_encoder(&ff, "libvvenc") {
        eprintln!("SKIPPED (vvc::encoded_streams): this ffmpeg has no libvvenc (restricted component)");
        return;
    }
    let path = fixture(&ff).expect("ffmpeg with libvvenc failed to encode the fixture");
    let data = std::fs::read(&path).unwrap();

    let nals = annexb_nals(&data);
    let units = access_units(&nals);
    assert_eq!(units.len(), FRAMES, "one access unit per frame");
    let infos: Vec<_> = units.iter().map(|r| au_info(&nals[r.clone()])).collect();
    assert!(infos.iter().all(|i| i.pictures == 1));
    assert!(infos[0].irap && infos[0].has_sps);
    let iraps = infos.iter().filter(|i| i.irap).count();
    assert!((2..FRAMES).contains(&iraps), "IntraPeriod=8 gives several, but not only, IRAP pictures: {iraps}");
    // Every IRAP unit repeats the parameter sets, so decoding can start there.
    assert!(infos.iter().filter(|i| i.irap).all(|i| i.has_sps));

    let sps_nal = nals.iter().find(|n| NalHeader::parse(n).is_ok_and(|h| h.nal_type == NalType::Sps)).unwrap();
    let sps = Sps::parse(sps_nal).unwrap();
    assert_eq!((sps.width, sps.height, sps.bit_depth, sps.chroma_format_idc), (WIDTH, HEIGHT, 10, 1));
    assert!(sps.coded_width >= WIDTH && sps.coded_height >= HEIGHT);
    assert_eq!(sps.profile_idc, Some(1), "Main 10");

    let bytes = annexb_access_units(&data);
    assert_eq!(bytes.len(), FRAMES);
    assert_eq!(bytes[0].0, 0);
    for w in bytes.windows(2) {
        assert_eq!(w[0].0 + w[0].1, w[1].0, "byte ranges are contiguous");
    }
    let last = bytes[FRAMES - 1];
    assert_eq!(last.0 + last.1, data.len());
    // Each byte range re-splits into exactly one access unit with the same NAL units.
    for (r, &(off, len)) in units.iter().zip(&bytes) {
        let part = annexb_nals(&data[off..off + len]);
        assert_eq!(part.len(), r.len());
        assert_eq!(access_units(&part).len(), 1);
    }
}
