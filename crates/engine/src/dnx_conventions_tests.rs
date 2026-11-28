//! Interpret Footage ▸ DNx conventions (CFA4-DNX-SPEC-001 R5, D2): an item's DNx video decodes as
//! detected from its container (FFmpeg's conventions for video FFmpeg encoded), by SMPTE
//! ST 2019-1, or as FFmpeg reads it; the choice is an undoable project setting.

use cfa4_isobmff::{Brand, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_media::FrameRequest;
use cfa4_project::{DnxConventions, ItemId};
use cfa4_time::Tick;
use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

/// A one-frame MOV of RGB DNxHR 444 that FFmpeg's encoder wrote (FFmpeg's signalling, CLF = 0,
/// and its compressor name).
fn write_ffmpeg_rgb_444(path: &std::path::Path) -> String {
    let (w, h) = (64u32, 32u32);
    let mut f = cfa4_dnx::Frame::new(w, h, cfa4_dnx::ChromaFormat::Yuv444, 10, false);
    f.rgb = true;
    let mut cfg = cfa4_dnx::EncoderConfig::new(cfa4_dnx::Profile::R444, w, h);
    cfg.bit_depth = 10;
    cfg.ffmpeg_compat = true;
    let au = cfa4_dnx::Encoder::with_config(cfg).unwrap().encode(&f).unwrap();
    let mut entry = SampleEntry::dnx(FourCc(*b"AVdh"), w as u16, h as u16);
    entry.video.as_mut().unwrap().compressor_name = "Lavc63.24.100 dnxhd".into();
    let mut mux = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).unwrap();
    let t = mux.add_track(TrackConfig::new(entry, 25)).unwrap();
    mux.write_sample(t, WriteSample { data: &au, duration: 1, composition_offset: 0, is_sync: true }).unwrap();
    std::fs::write(path, mux.finish().unwrap().into_inner()).unwrap();
    path.to_string_lossy().into_owned()
}

fn decodes_rgb(s: &Session, item: ItemId) -> bool {
    let src = s.media.full_res_source(&s.project, item, &*s.services).unwrap();
    let f = src.video_frame(FrameRequest::full(Tick::ZERO)).unwrap();
    matches!(f.data, cfa4_frame::PixelData::Rgb16 { .. })
}

#[test]
fn dnx_conventions_are_chosen_per_item_and_undone() {
    let dir = tmp_dir("dnx-conventions");
    let mut s = Session::default();
    let mov = write_ffmpeg_rgb_444(&dir.join("ffmpeg_rgb444.mov"));
    let r = s.execute("file.import", json!({"paths": [mov]})).unwrap();
    let item = ItemId(r["items"][0].as_u64().unwrap());
    let info = s.execute("media.dnxInfo", json!({"item": item.0})).unwrap();
    assert_eq!(info, json!({"dnx": true, "setting": "auto", "detected": "ffmpeg", "effective": "ffmpeg"}));
    assert!(decodes_rgb(&s, item), "FFmpeg's RGB as FFmpeg reads it");

    // by the standard: CLF = 0 is Y′CbCr
    let r = s.execute("clip.interpretFootage", json!({"items": [item.0], "dnxConventions": "smpte"})).unwrap();
    assert_eq!(r["dnxConventions"], "smpte", "{r}");
    let info = s.execute("media.dnxInfo", json!({"item": item.0})).unwrap();
    assert_eq!((info["setting"].clone(), info["detected"].clone(), info["effective"].clone()), (json!("smpte"), json!("ffmpeg"), json!("smpte")));
    assert!(!decodes_rgb(&s, item));
    assert_eq!(s.project.item(item).unwrap().as_media().unwrap().interpret.dnx_conventions, Some(DnxConventions::Smpte));

    // the same choice again changes nothing (no undo step), undo goes back to the detection
    s.execute("clip.interpretFootage", json!({"items": [item.0], "dnxConventions": "smpte"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.item(item).unwrap().as_media().unwrap().interpret.dnx_conventions, None);
    assert!(decodes_rgb(&s, item));

    // explicit FFmpeg, then auto again
    s.execute("clip.interpretFootage", json!({"items": [item.0], "dnxConventions": "ffmpeg"})).unwrap();
    assert!(decodes_rgb(&s, item));
    let r = s.execute("clip.interpretFootage", json!({"items": [item.0], "dnxConventions": "auto"})).unwrap();
    assert_eq!(r["dnxConventions"], "auto");
    assert!(s.execute("clip.interpretFootage", json!({"items": [item.0], "dnxConventions": "avid"})).is_err());
}

#[test]
fn dnx_info_of_other_media() {
    let mut s = Session::default();
    let id = s.execute("file.newColorMatte", json!({"width": 64, "height": 36, "color": "#ff0000"})).unwrap()["item"].as_u64().unwrap();
    let info = s.execute("media.dnxInfo", json!({"item": id})).unwrap();
    assert_eq!(info["dnx"], false, "{info}");
    assert_eq!(info["effective"], serde_json::Value::Null);
}
