//! SVQ3 through the media source on the FATE samples in `target/fixtures/fate/svq3/` (local only,
//! never committed; skipped without them or ffmpeg): the QuickTime files read directly (the
//! decoder takes its sequence header from the sample entry) and remuxed to AVI (from the extradata
//! after the `BITMAPINFOHEADER`), their frames in a jumping order against ffmpeg's decoding of the
//! AVI. The decoder itself is checked against ffmpeg in `crates/svq3/tests/oracle.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Sample {
    name: &'static str,
    /// The QuickTime file's sample times are usable (the regression sample's are all zero).
    timed: bool,
    /// Frames to ask for, in this order (cold seeks, backwards, around B pictures).
    frames: &'static [usize],
}

const SAMPLES: &[Sample] = &[
    Sample { name: "Vertical400kbit.sorenson3.mov", timed: true, frames: &[0, 300, 7, 1000, 301, 3, 1307] },
    Sample { name: "svq3_watermark.mov", timed: true, frames: &[0, 5, 2, 9, 3, 1] },
    Sample { name: "svq3_decoding_regression.mov", timed: false, frames: &[0, 11, 4, 19, 6] },
];

fn remux(ff: &Path, src: &Path, out: &Path) -> bool {
    Command::new(ff)
        .args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args(["-an", "-map", "0:v:0", "-c:v", "copy", "-tag:v", "SVQ3", "-r", "25"])
        .arg(out)
        .status()
        .is_ok_and(|s| s.success())
}

fn rawvideo(ff: &Path, file: &Path) -> Vec<u8> {
    Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(file)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuvj420p", "-"])
        .output()
        .expect("run ffmpeg")
        .stdout
}

fn hash(planes: &[std::sync::Arc<Vec<u8>>; 3]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in planes {
        p.hash(&mut h);
    }
    h.finish()
}

/// The picture shown at the time of sample `i` (QuickTime and AVI time the samples in decoding
/// order, so with B pictures that is not the `i`-th picture in display order).
fn frame_hash(source: &dyn cfa4_media::MediaSource, i: usize, what: &str) -> u64 {
    let v = source.info().video.clone().unwrap();
    let f = source.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{what} frame {i}: {e}"));
    let cfa4_frame::PixelData::Yuv8 { planes, chroma, .. } = &f.data else { panic!("YUV 8-bit frame") };
    let (w, h) = (v.width as usize, v.height as usize);
    assert_eq!((*chroma, planes[0].len(), planes[1].len()), (cfa4_frame::Chroma::C420, w * h, w.div_ceil(2) * h.div_ceil(2)), "{what}");
    hash(planes)
}

#[test]
fn svq3_reads_through_the_media_source() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq3 file") else { return };
    let root = cfa4_testkit::fixtures::fixtures_root();
    let dir = root.join("svq3-file");
    std::fs::create_dir_all(&dir).unwrap();
    let mut checked = 0;
    for s in SAMPLES {
        let src: PathBuf = root.join("fate/svq3").join(s.name);
        if !src.exists() {
            eprintln!("no FATE sample svq3/{}, skipping", s.name);
            continue;
        }
        let avi = dir.join(format!("{}.avi", s.name));
        assert!(remux(&ff, &src, &avi), "remux {}", s.name);
        let oracle = rawvideo(&ff, &avi);
        let avi_bytes = std::fs::read(&avi).unwrap();
        let mov_bytes = std::fs::read(&src).unwrap();
        for (label, file, bytes) in [("mov", s.name.to_string(), mov_bytes), ("avi", format!("{}.avi", s.name), avi_bytes)] {
            if label == "mov" && !s.timed {
                continue;
            }
            let open = || cfa4_codecs::open_bytes(&file, bytes.clone().into());
            let source = match open() {
                Ok(s) => s,
                // QuickTime files of that era often compress their `moov` (`cmov`), which the
                // container reader does not read; the AVI remux of the same stream is checked
                Err(e) if label == "mov" && e.to_string().contains("cmov") => {
                    eprintln!("{}: {e}, only the AVI remux is checked", s.name);
                    continue;
                }
                Err(e) => panic!("{} {label}: {e}", s.name),
            };
            let v = source.info().video.clone().unwrap();
            assert_eq!(v.codec.as_str(), "Sorenson Video 3", "{} {label}", s.name);
            let (w, h) = (v.width as usize, v.height as usize);
            let fsize = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
            let n = oracle.len() / fsize;
            let what = format!("{} {label}", s.name);
            // sequentially: every picture ffmpeg shows, once
            let seq: Vec<u64> = (0..n).map(|i| frame_hash(&*source, i, &what)).collect();
            let mut theirs: Vec<u64> = oracle
                .chunks(fsize)
                .map(|f| {
                    let (y, c) = f.split_at(w * h);
                    let (u, vv) = c.split_at(c.len() / 2);
                    hash(&[y.to_vec().into(), u.to_vec().into(), vv.to_vec().into()])
                })
                .collect();
            let mut ours = seq.clone();
            ours.sort_unstable();
            theirs.sort_unstable();
            let missing = theirs.iter().filter(|h| !ours.contains(h)).count();
            assert!(missing == 0, "{what}: {missing} of {n} pictures differ from ffmpeg's");
            // in a jumping order (cold seeks, backwards): the same pictures
            let cold = open().unwrap();
            for &i in s.frames.iter().filter(|&&i| i < n) {
                assert_eq!(frame_hash(&*cold, i, &what), seq[i], "{what}: frame {i} after a seek");
                checked += 1;
            }
        }
    }
    eprintln!("{checked} seeks checked");
}
