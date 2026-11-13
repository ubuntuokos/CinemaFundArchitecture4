//! The parameters of `dcpomatic.makeDcp`; running DCP-o-matic itself is tested in
//! `cfa4-dcpomatic` against fake tools.

use serde_json::json;

use super::*;

#[test]
fn params_map_onto_the_request() {
    let p = json!({
        "content": ["/m/master.mov", {"path": "/m/hi.wav", "channel": "HI", "gainDb": -2.5}, {"path": "/m/left.mov", "eye": "left"}],
        "output": "/out/My Film",
        "contentKind": "feature",
        "standard": "interop",
        "resolution": "4k",
        "container": "scope",
        "fps": 24,
        "audioChannels": 8,
        "videoBitRateMbps": 200,
        "threeD": true,
        "isdcfName": false,
        "threads": 4,
        "configDir": "/cfg",
    });
    let r = request_from_params("dcpomatic.makeDcp", &p).unwrap();
    assert_eq!(r.name, "My Film", "the name defaults to the output folder's");
    assert_eq!(r.film_dir, PathBuf::from("/out/My Film"));
    assert_eq!(r.content.len(), 3);
    assert_eq!(r.content[1].channel, Some(cfa4_dcpomatic::Channel::Hi));
    assert_eq!(r.content[1].gain_db, Some(-2.5));
    assert_eq!(r.content[2].eye, Some(cfa4_dcpomatic::Eye::Left));
    assert_eq!(r.content_kind, ContentKind::Feature);
    assert_eq!(r.standard, Standard::Interop);
    assert_eq!(r.resolution, Some(Resolution::FourK));
    assert_eq!(r.container, Some(Container::Scope));
    assert_eq!((r.frame_rate, r.audio_channels, r.video_bit_rate_mbps, r.threads), (Some(24), Some(8), Some(200), Some(4)));
    assert!(r.three_d && !r.isdcf_name);
    assert_eq!(r.config_dir, Some(PathBuf::from("/cfg")));

    let min = request_from_params("dcpomatic.makeDcp", &json!({"content": "/m/a.mov", "output": "/o/f", "name": "Short"})).unwrap();
    assert_eq!((min.name.as_str(), min.content_kind, min.standard, min.isdcf_name), ("Short", ContentKind::Test, Standard::Smpte, true));
}

#[test]
fn bad_params_are_refused() {
    let cmd = "dcpomatic.makeDcp";
    for (p, want) in [
        (json!({"output": "/o/f"}), "content"),
        (json!({"content": "/a.mov"}), "output"),
        (json!({"content": "/a.mov", "output": "/o/f", "standard": "dci"}), "standard"),
        (json!({"content": "/a.mov", "output": "/o/f", "container": "1.33"}), "container"),
        (json!({"content": "/a.mov", "output": "/o/f", "fps": "fast"}), "fps"),
        (json!({"content": [{"path": "/a.mov", "channel": "Q"}], "output": "/o/f"}), "content"),
    ] {
        let e = request_from_params(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
}

#[test]
fn commands_are_registered() {
    let ids: Vec<&str> = crate::commands::command_specs().iter().map(|c| c.id).collect();
    assert!(ids.contains(&"dcpomatic.status") && ids.contains(&"dcpomatic.makeDcp"));
    let mut s = crate::Session::default();
    let st = status(&mut s, &json!({})).unwrap();
    assert!(st["available"].is_boolean() && st["minimumVersion"] == "2.18.20", "{st}");
}
