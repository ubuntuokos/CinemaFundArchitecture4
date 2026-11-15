//! A Lagarith AVI read through the media source: every frame is what `cfa4_lagarith` decodes
//! from its chunk, read in order and out of order. (The decoder itself is checked against
//! ffmpeg in `crates/lagarith/tests/oracle.rs`.)

/// A minimal AVI with one `LAGS` video stream of `frames`.
fn avi(w: u32, h: u32, bit_count: u16, frames: &[Vec<u8>]) -> Vec<u8> {
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
    let strh = [b"vidsLAGS".as_slice(), &words(&[0, 0, 0, 1, 25, 0, n, 0, u32::MAX, 0]), &[0u8; 8]].concat();
    let strf = [words(&[40, w, h]), [1u16.to_le_bytes(), bit_count.to_le_bytes()].concat(), b"LAGS".to_vec(), words(&[0; 5])].concat();
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat())].concat());
    let (mut movi, mut idx) = (Vec::new(), Vec::new());
    for f in frames {
        idx.extend_from_slice(&[b"00dc".as_slice(), &words(&[0x10, 4 + movi.len() as u32, f.len() as u32])].concat());
        movi.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi), chunk(b"idx1", &idx)].concat())
}

/// An RGB24 frame of three uncompressed residual planes.
fn raw_rgb_frame(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let n = (w * h) as usize;
    let plane = |k: u32| -> Vec<u8> { [4u8].into_iter().chain((0..n as u32).map(|i| ((i * 7 + k * 31 + seed * 13) % 11) as u8)).collect() };
    let (a, b, c) = (plane(0), plane(1), plane(2));
    let o1 = 9 + a.len() as u32;
    let o2 = o1 + b.len() as u32;
    [vec![4u8], o1.to_le_bytes().to_vec(), o2.to_le_bytes().to_vec(), a, b, c].concat()
}

#[test]
fn lagarith_avi_reads_through_the_media_source() {
    let (w, h) = (40u32, 24u32);
    let frames: Vec<Vec<u8>> = (0..6).map(|k| if k % 3 == 2 { vec![6, 10 * k as u8, 100, 200] } else { raw_rgb_frame(w, h, k) }).collect();
    let mut dec = cfa4_lagarith::Decoder::new(w, h, 24).unwrap();
    let want: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| {
            let p = dec.decode(f).unwrap();
            (0..(w * h) as usize).flat_map(|i| [p.planes[0][i], p.planes[1][i], p.planes[2][i], 255]).collect()
        })
        .collect();
    let src = cfa4_codecs::open_bytes("clip.avi", avi(w, h, 24, &frames).into()).unwrap();
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (w, h, "Lagarith"));
    for i in (0..frames.len()).rev().chain(0..frames.len()) {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("RGBA8 frame") };
        assert!(px.as_slice() == want[i].as_slice(), "frame {i}");
    }
}
