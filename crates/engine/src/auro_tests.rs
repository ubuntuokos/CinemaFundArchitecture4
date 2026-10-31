//! Interpret Footage ▸ Auro-3D on / off (#323): an Auro-3D carrier plays its restored layout, or
//! its own channels as a player without the decoder would.

use cfa4_auro::cfa4::wav_carrier_ids;
use cfa4_auro::synth;
use cfa4_project::ItemId;
use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

/// A 24-bit 5.1 WAV whose low bits carry an Auro-3D 9.1 side channel.
fn write_carrier(path: &std::path::Path) -> String {
    let ids = wav_carrier_ids(6).unwrap();
    let planes: Vec<Vec<i32>> = ids.iter().enumerate().map(|(c, id)| synth::channel(c as u32 + 11, 40, 0.25, 30, id.0)).collect();
    let frames = planes[0].len();
    let data: Vec<u8> = (0..frames).flat_map(|i| planes.iter().flat_map(move |p| p[i].to_le_bytes()[..3].to_vec())).collect();
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&6u16.to_le_bytes());
    w.extend_from_slice(&48_000u32.to_le_bytes());
    w.extend_from_slice(&(48_000u32 * 18).to_le_bytes());
    w.extend_from_slice(&18u16.to_le_bytes());
    w.extend_from_slice(&24u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    std::fs::write(path, w).unwrap();
    path.to_string_lossy().into_owned()
}

fn channels(s: &Session, item: ItemId) -> (u32, usize) {
    let m = s.project.item(item).unwrap().as_media().unwrap();
    let src = s.media.full_res_source(&s.project, item, &*s.services).unwrap();
    (m.info.audio().unwrap().channels, src.audio(0, 100, 48_000).unwrap().channels.len())
}

#[test]
fn auro_decoding_can_be_turned_off_per_item() {
    let dir = tmp_dir("auro-toggle");
    let mut s = Session::default();
    let wav = write_carrier(&dir.join("carrier.wav"));
    let r = s.execute("file.import", json!({"paths": [wav]})).unwrap();
    let item = ItemId(r["items"][0].as_u64().unwrap());
    assert_eq!(channels(&s, item), (10, 10), "imported as the restored 9.1");
    let info = s.execute("media.auroInfo", json!({"item": item.0})).unwrap();
    assert_eq!((info["carrier"].clone(), info["decoding"].clone()), (json!(true), json!(true)), "{info}");
    // a channel map for the restored layout's heights goes when they do
    s.execute("project.select", json!({"items": [item.0]})).unwrap();
    s.execute("clip.audioChannels", json!({"format": "mono"})).unwrap();
    // off: the 5.1 carrier, its info and its source agree; the setting survives a save
    let r = s.execute("clip.interpretFootage", json!({"items": [item.0], "auroDecoding": false})).unwrap();
    assert_eq!(r["auroDecoding"], json!(false), "{r}");
    assert_eq!(channels(&s, item), (6, 6));
    let m = s.project.item(item).unwrap().as_media().unwrap();
    assert!(m.interpret.audio_channels.is_none());
    assert!(serde_json::to_string(&m.interpret).unwrap().contains("auro_as_carrier"));
    let info = s.execute("media.auroInfo", json!({"item": item.0})).unwrap();
    assert_eq!((info["carrier"].clone(), info["decoding"].clone()), (json!(true), json!(false)), "{info}");
    // undo brings the decoded layout back; on again likewise
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(channels(&s, item), (10, 10));
    s.execute("clip.interpretFootage", json!({"items": [item.0], "auroDecoding": false})).unwrap();
    s.execute("clip.interpretFootage", json!({"items": [item.0], "auroDecoding": true})).unwrap();
    assert_eq!(channels(&s, item), (10, 10));
    assert!(!serde_json::to_string(&s.project.item(item).unwrap().as_media().unwrap().interpret).unwrap().contains("auro_as_carrier"));
}

/// Footage without an Auro-3D carrier keeps its audio and its setting.
#[test]
fn auro_decoding_leaves_other_footage_alone() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let item = *s.project.items.iter().find(|(_, it)| it.as_media().is_some_and(|m| m.info.has_audio())).unwrap().0;
    let before = s.project.item(item).unwrap().as_media().unwrap().clone();
    s.execute("clip.interpretFootage", json!({"items": [item.0], "auroDecoding": false})).unwrap();
    assert_eq!(s.project.item(item).unwrap().as_media().unwrap(), &before);
    let info = s.execute("media.auroInfo", json!({"item": item.0})).unwrap();
    assert_eq!(info["carrier"], json!(false), "{info}");
}
