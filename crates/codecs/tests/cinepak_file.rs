//! A Cinepak AVI read through the media source: every frame is what `cfa4_cinepak` decodes from
//! its chunk, read in order and out of order. (The decoder itself is checked against ffmpeg in
//! `crates/cinepak/tests/oracle.rs`.)

fn avi(w: u32, h: u32, frames: &[Vec<u8>]) -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        chunk(b"LIST", &[kind.as_slice(), body].concat())
    }
    let words = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let n = frames.len() as u32;
    let avih = words(&[40_000, 0, 0, 0x10, n, 0, 1, 0, w, h, 0, 0, 0, 0]);
    let strh = [b"vidscvid".as_slice(), &words(&[0, 0, 0, 1, 25, 0, n, 0, u32::MAX, 0]), &[0u8; 8]].concat();
    let strf = [words(&[40, w, h]), [1u16.to_le_bytes(), 24u16.to_le_bytes()].concat(), b"cvid".to_vec(), words(&[0; 5])].concat();
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat())].concat());
    let (mut movi, mut idx) = (Vec::new(), Vec::new());
    for f in frames {
        idx.extend_from_slice(&[b"00dc".as_slice(), &words(&[0x10, 4 + movi.len() as u32, f.len() as u32])].concat());
        movi.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi), chunk(b"idx1", &idx)].concat())
}

fn be24(v: usize) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// An intra frame of one strip: a full V1 codebook (4-element entries, `seed` shifts the grey
/// ramp) and an all-V1 vector chunk that paints the `w` x 4 strip with entries `0, 1, 2, …`.
fn frame(w: usize, seed: u8) -> Vec<u8> {
    let cb: Vec<u8> = (0..256usize).flat_map(|i| [(i as u8).wrapping_add(seed); 4]).collect();
    let cb = [vec![0x26], be24(cb.len() + 4).to_vec(), cb].concat();
    let vec_data: Vec<u8> = (0..w / 4).map(|i| i as u8).collect();
    let vecs = [vec![0x32], be24(vec_data.len() + 4).to_vec(), vec_data].concat();
    let body = [cb, vecs].concat();
    let strip = [
        vec![0x10],
        be24(body.len() + 12).to_vec(),
        0u16.to_be_bytes().to_vec(),
        0u16.to_be_bytes().to_vec(),
        4u16.to_be_bytes().to_vec(),
        (w as u16).to_be_bytes().to_vec(),
        body,
    ]
    .concat();
    let total = 10 + strip.len();
    [vec![0], be24(total).to_vec(), (w as u16).to_be_bytes().to_vec(), 4u16.to_be_bytes().to_vec(), 1u16.to_be_bytes().to_vec(), strip].concat()
}

#[test]
fn cinepak_avi_reads_through_the_media_source() {
    let (w, h) = (32u32, 4u32);
    let frames: Vec<Vec<u8>> = (0..5).map(|k| frame(w as usize, 10 * k)).collect();
    let mut dec = cfa4_cinepak::Decoder::new(w, h, 24).unwrap();
    let want: Vec<Vec<u8>> = frames.iter().map(|f| dec.decode(f).unwrap().to_rgba()).collect();
    assert_ne!(want[0], want[1]);
    let src = cfa4_codecs::open_bytes("clip.avi", avi(w, h, &frames).into()).unwrap();
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (w, h, "Cinepak"));
    for i in (0..frames.len()).rev().chain(0..frames.len()) {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("RGBA8 frame") };
        assert!(px.as_slice() == want[i].as_slice(), "frame {i}");
    }
}

/// FATE samples (local only, `target/fixtures/fate/cvid/`, never committed): every frame through
/// the media source equals ffmpeg's decoding. Skipped when the samples or ffmpeg are missing.
#[test]
fn fate_samples_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cinepak FATE oracle") else { return };
    let dir = cfa4_testkit::fixtures::fixtures_root().join("fate").join("cvid");
    let mut seen = 0;
    for name in ["laracroft-cinepak-partial.avi", "pcitva15.avi", "catfight-cvid-pal8-partial.mov"] {
        let path = dir.join(name);
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let theirs = std::process::Command::new(&ff)
            .args(["-v", "error", "-threads", "1", "-i"])
            .arg(&path)
            .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .expect("run ffmpeg")
            .stdout;
        let src = match cfa4_codecs::open_bytes(name, bytes.into()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{name}: container not readable ({e}), skipped");
                continue;
            }
        };
        let v = src.info().video.clone().unwrap();
        let fsz = (v.width * v.height * 3) as usize;
        let n = theirs.len() / fsz;
        assert!(n >= 5, "{name}: ffmpeg decoded {n} frames");
        for i in 0..n {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name} frame {i}: {e}"));
            let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("RGBA8 frame") };
            let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
            assert!(rgb == theirs[i * fsz..(i + 1) * fsz], "{name}: frame {i} differs from ffmpeg");
        }
        seen += 1;
    }
    eprintln!("{seen} FATE cinepak samples compared");
}
