//! OP-Atom clips over several files (Avid MediaFiles, P2 cards) import as one item.

use std::path::Path;

use cfa4_media::MediaKind;
use cfa4_mxf::{FrameInfo, MxfWriter, OpAtomPcm, PackageIds, Pattern, PictureCoding, PictureDesc, Rational, Umid, WriterConfig, write_opatom_pcm};
use cfa4_project::ItemId;
use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

/// Picture file and two mono sound files of one clip (shared material package).
fn write_clip(seed: &str, video: &Path, audio: &[&Path]) {
    let clip = PackageIds::from_seed(seed, seed);
    let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(25, 1), PackageIds { file: Umid::from_seed(format!("{seed}v").as_bytes()), ..clip.clone() });
    cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 64, 36));
    let mut w = MxfWriter::new(std::io::Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..10u8 {
        let mut f = vec![0x00, 0x00, 0x02, 0x80, 0x01];
        f.extend(std::iter::repeat_n(i, 600));
        w.push_picture(&f, FrameInfo::intra()).unwrap();
    }
    std::fs::write(video, w.finish().unwrap().into_inner()).unwrap();
    for (k, a) in audio.iter().enumerate() {
        let ids = PackageIds { file: Umid::from_seed(format!("{seed}a{k}").as_bytes()), ..clip.clone() };
        let opts = OpAtomPcm { sample_rate: 48_000, bits: 16, channels: 1, edit_rate: Some(Rational::new(25, 1)), ids, timecode: None };
        std::fs::write(a, write_opatom_pcm(&opts, &vec![1000 * (k as i32 + 1); 19_200]).unwrap()).unwrap();
    }
}

fn movie_channels(s: &Session, id: ItemId) -> (MediaKind, u32) {
    let src = s.media.source_for(&s.project, id, &*s.services).unwrap();
    (src.info().kind, src.info().audio().map_or(0, |a| a.channels))
}

#[test]
fn avid_media_files_import_as_one_clip() {
    let dir = tmp_dir("opatom-avid");
    let (v, a1, a2) = (dir.join("Interview V01.A1B2C3_1.mxf"), dir.join("Interview A01.A1B2C3_2.mxf"), dir.join("Interview A02.A1B2C3_3.mxf"));
    write_clip("interview", &v, &[&a1, &a2]);
    // another clip in the same folder stays separate
    let (ov, oa) = (dir.join("Other V01.mxf"), dir.join("Other A01.mxf"));
    write_clip("other", &ov, &[&oa]);
    let mut s = Session::default();
    let paths: Vec<String> = [&a2, &v, &a1].iter().map(|p| p.to_string_lossy().to_string()).collect();
    let r = s.execute("file.import", json!({"paths": paths})).unwrap();
    let items = r["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{r}");
    let id = ItemId(items[0].as_u64().unwrap());
    assert_eq!(movie_channels(&s, id), (MediaKind::Movie, 2));
    // the file asked for may be a sound file: the clip still has its picture
    let mut s = Session::default();
    let r = s.execute("file.import", json!({"paths": [oa.to_string_lossy()]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap());
    assert_eq!(movie_channels(&s, id), (MediaKind::Movie, 1));
}

#[test]
fn p2_card_clips_join_video_and_audio_folders() {
    let card = tmp_dir("opatom-p2");
    let (vdir, adir) = (card.join("CONTENTS").join("VIDEO"), card.join("CONTENTS").join("AUDIO"));
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::create_dir_all(&adir).unwrap();
    let (v, a0, a1) = (vdir.join("0001AB.MXF"), adir.join("0001AB00.MXF"), adir.join("0001AB01.MXF"));
    write_clip("p2clip", &v, &[&a0, &a1]);
    let mut s = Session::default();
    let r = s.execute("file.import", json!({"paths": [v.to_string_lossy()]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap());
    assert_eq!(movie_channels(&s, id), (MediaKind::Movie, 2));
    // channel order follows the file names: 00 then 01
    let src = s.media.source_for(&s.project, id, &*s.services).unwrap();
    let buf = src.audio(0, 2, 48_000).unwrap();
    assert!(buf.channels[0][0] < buf.channels[1][0]);
    // a lone OP-Atom file opens by itself
    let lone = tmp_dir("opatom-lone").join("lone.mxf");
    let ids = PackageIds::from_seed("lone", "lone");
    let opts = OpAtomPcm { sample_rate: 48_000, bits: 16, channels: 1, edit_rate: None, ids, timecode: None };
    std::fs::write(&lone, write_opatom_pcm(&opts, &[0; 4800]).unwrap()).unwrap();
    let r = s.execute("file.import", json!({"paths": [lone.to_string_lossy()]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap());
    assert_eq!(movie_channels(&s, id), (MediaKind::AudioOnly, 1));
}
