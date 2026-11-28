//! `media.findCameraProxies` end to end: a GoPro-style `.LRV` next to a clip is found and attached;
//! a proxy of the wrong length is reported and skipped; clips without a proxy are skipped.

use cfa4_media::DemoScene;
use cfa4_project::MediaRef;
use serde_json::json;

use crate::media_test_util::{make_movie, session_with, tmp_dir};

const W: u32 = 160;
const H: u32 = 90;

#[test]
fn finds_and_attaches_a_gopro_lrv() {
    let root = tmp_dir("cam-proxy-ok");
    let clip = root.join("GX010001.mov");
    make_movie(&clip, DemoScene::OceanSunset, W, H, 12);
    // our own writer emits .mov; the GoPro rule needs `.MP4`, so name the clip like the camera does
    let mp4 = root.join("GX010001.MP4");
    std::fs::rename(&clip, &mp4).unwrap();
    make_movie(&root.join("tmp-proxy.mov"), DemoScene::OceanSunset, W / 2, H / 2, 12);
    std::fs::rename(root.join("tmp-proxy.mov"), root.join("GL010001.LRV")).unwrap();

    let (mut s, items, _) = session_with(&[&mp4]);
    let r = s.execute("media.findCameraProxies", json!({"items": [items[0].0]})).unwrap();
    assert_eq!(r["attached"], json!(true), "{r}");
    assert_eq!(r["found"][0]["camera"], "GoPro LRV", "{r}");
    let m = s.project.item(items[0]).unwrap().as_media().unwrap();
    match &m.proxy {
        Some(MediaRef::File { path }) => assert!(path.ends_with("GL010001.LRV"), "{path}"),
        other => panic!("no proxy attached: {other:?}"),
    }
}

#[test]
fn wrong_length_and_missing_proxies_are_skipped() {
    let root = tmp_dir("cam-proxy-skip");
    let a = root.join("DJI_0001.mov");
    make_movie(&a, DemoScene::OceanSunset, W, H, 12);
    let a_mp4 = root.join("DJI_0001.MP4");
    std::fs::rename(&a, &a_mp4).unwrap();
    make_movie(&root.join("short.mov"), DemoScene::OceanSunset, W / 2, H / 2, 4);
    std::fs::rename(root.join("short.mov"), root.join("DJI_0001.LRF")).unwrap();
    let b = root.join("lonely.mov");
    make_movie(&b, DemoScene::OceanSunset, W, H, 6);

    let (mut s, items, _) = session_with(&[&a_mp4, &b]);
    let r = s.execute("media.findCameraProxies", json!({"items": [items[0].0, items[1].0]})).unwrap();
    assert_eq!(r["attached"], json!(false), "{r}");
    assert_eq!(r["skipped"].as_array().unwrap().len(), 2, "{r}");
    assert!(s.project.item(items[0]).unwrap().as_media().unwrap().proxy.is_none());
}
