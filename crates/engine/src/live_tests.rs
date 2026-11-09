use std::io::{Read, Write};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;

const SHEET: &str = "Head,Body,Content Style,Page Style,Page Runtime\n\
                     ,THE LONG WAY HOME,Heading,Card,4s\n\
                     Directed by,Jordan Lee,Centered,Scroll,\n\
                     ,,,,\n\
                     Cast,Ann Smith,Gutter,,\n";

fn demo(dir: &str) -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.prefs_path = Some(tmp_dir(dir).join("prefs.json"));
    s.execute("playhead.set", json!({"seconds": 600})).unwrap();
    s
}

fn png_size(b64: &str) -> (u32, u32) {
    let png = cfa4_project::gtemplate::base64_decode(b64).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()))
}

fn add(s: &mut Session, p: Value) -> u64 {
    s.execute("live.cue.add", p).unwrap()["id"].as_u64().unwrap()
}

#[test]
fn take_step_hold_and_out() {
    let mut s = demo("live-take");
    let a = add(&mut s, json!({"csv": SHEET, "name": "End credits"}));
    let b = add(&mut s, json!({"csv": "Head,Body\nThanks,Everyone\n"}));
    let st = s.execute("live.status", json!({})).unwrap();
    assert_eq!(st["cues"].as_array().unwrap().len(), 2);
    assert_eq!(st["cues"][0]["pages"], 2);
    assert_eq!(st["preview"], a);
    assert_eq!(st["onAir"], false);
    // nothing on air: stepping is not offered
    assert!(s.execute("live.next", json!({})).is_err());

    let st = s.execute("live.take", json!({})).unwrap();
    assert_eq!((st["program"]["cue"].as_u64(), st["program"]["page"].as_u64()), (Some(a), Some(0)));
    assert_eq!(st["preview"], b, "the next cue comes into preview");
    let seq = s.live.store.seq();
    let t = s.execute("live.tick", json!({"at": 1.0})).unwrap();
    assert!(t["seq"].as_u64().unwrap() > seq && t["onAir"] == true);
    let f = s.execute("live.frame", json!({"which": "program", "width": 320, "at": 1.0})).unwrap();
    assert_eq!(f["empty"], false);
    assert_eq!(png_size(f["pngBase64"].as_str().unwrap()), (320, 180));
    // the page has drawn something (not all transparent)
    let png = s.live.store.png();
    assert!(png.len() > 200);

    s.execute("live.next", json!({})).unwrap();
    assert_eq!(s.execute("live.status", json!({})).unwrap()["program"]["page"], 1);
    s.execute("live.previous", json!({})).unwrap();
    assert_eq!(s.execute("live.status", json!({})).unwrap()["program"]["page"], 0);
    s.execute("live.previous", json!({})).unwrap();
    assert_eq!(s.execute("live.status", json!({})).unwrap()["program"]["page"], 0);

    // HOLD freezes the clock, CONTINUE goes on from there
    s.execute("live.hold", json!({})).unwrap();
    let e1 = s.execute("live.status", json!({})).unwrap()["program"]["elapsed"].as_f64().unwrap();
    std::thread::sleep(Duration::from_millis(40));
    let st = s.execute("live.status", json!({})).unwrap();
    assert_eq!(st["program"]["held"], true);
    assert_eq!(st["program"]["elapsed"].as_f64().unwrap(), e1);
    assert_eq!(s.live.store.status()["held"], true);
    s.execute("live.continue", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    assert!(s.execute("live.status", json!({})).unwrap()["program"]["elapsed"].as_f64().unwrap() > e1);

    // past the last page: the preview cue goes on air
    s.execute("live.next", json!({})).unwrap();
    s.execute("live.next", json!({})).unwrap();
    assert_eq!(s.execute("live.status", json!({})).unwrap()["program"]["cue"], b);
    // the on-air cue cannot be removed; OUT clears the picture
    assert!(s.execute("live.cue.remove", json!({"id": b})).is_err());
    s.execute("live.out", json!({})).unwrap();
    assert_eq!(s.execute("live.status", json!({})).unwrap()["onAir"], false);
    assert_eq!(&**s.live.store.png(), cfa4_live::store::CLEAR_PNG);
    assert_eq!(s.execute("live.frame", json!({})).unwrap()["empty"], true);
    s.execute("live.cue.remove", json!({"id": b})).unwrap();
    assert_eq!(s.execute("live.status", json!({})).unwrap()["cues"].as_array().unwrap().len(), 1);
    // live commands are not edits
    assert!(!s.journal.iter().any(|(id, _)| id.starts_with("live.")));
}

#[test]
fn cards_wait_rolls_run_out_and_advance() {
    let mut s = demo("live-run");
    let id = add(&mut s, json!({"csv": SHEET}));
    s.execute("live.take", json!({})).unwrap();
    // the card waits at its last full moment for NEXT, however long it has been on
    s.live.program.as_mut().unwrap().page_start = Instant::now() - Duration::from_secs(600);
    let t = s.execute("live.tick", json!({})).unwrap();
    assert_eq!((t["page"].as_u64(), t["finished"].as_bool()), (Some(0), Some(false)));
    // without holding, the card runs out into the roll, and the roll runs out to the end
    s.live.cues.iter_mut().find(|c| c.id == id).unwrap().hold_cards = false;
    s.live.program.as_mut().unwrap().page_start = Instant::now() - Duration::from_secs(5);
    let t = s.execute("live.tick", json!({})).unwrap();
    assert_eq!(t["page"], 1, "{t}");
    s.live.program.as_mut().unwrap().page_start = Instant::now() - Duration::from_secs(3600);
    let t = s.execute("live.tick", json!({})).unwrap();
    assert_eq!(t["finished"], true);
    assert_eq!(&**s.live.store.png(), cfa4_live::store::CLEAR_PNG);
    assert_eq!(s.live.store.status()["onAir"], false);
}

#[test]
fn update_reads_the_spreadsheet_again_on_air() {
    let mut s = demo("live-update");
    let dir = tmp_dir("live-update-sheet");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, "Head,Body,Page Style\nDirected by,Jordan Lee,Card\n").unwrap();
    let id = add(&mut s, json!({"path": sheet.to_string_lossy()}));
    assert_eq!(s.execute("live.status", json!({})).unwrap()["cues"][0]["name"], "credits");
    s.execute("live.take", json!({})).unwrap();
    std::fs::write(&sheet, "Head,Body,Page Style\nDirected by,Jordan Lee,Card\nMusic,Ann Smith,Card\n").unwrap();
    let u = s.execute("live.update", json!({})).unwrap();
    assert_eq!((u["id"].as_u64(), u["pages"].as_u64()), (Some(id), Some(2)));
    assert_eq!(s.execute("live.status", json!({})).unwrap()["onAir"], true);
    // new parameters replace the old ones
    let u = s.execute("live.update", json!({"id": id, "path": Value::Null, "csv": "Head,Body\nA,B\n"})).unwrap();
    assert_eq!(u["pages"], 1);
    // a broken spreadsheet is an error, and the cue stays as it was
    std::fs::write(&sheet, [0u8, 159, 146, 150]).unwrap();
    assert!(s.execute("live.update", json!({"csv": Value::Null, "path": sheet.to_string_lossy()})).is_err());
    assert_eq!(s.execute("live.status", json!({})).unwrap()["cues"][0]["pages"], 1);
}

#[test]
fn graphic_clips_go_live_and_other_clips_do_not() {
    let mut s = demo("live-clip");
    let r = s.execute("graphics.credits.import", json!({"csv": "Head,Body,Page Style\nDirected by,Jordan Lee,Card\n"})).unwrap();
    let clip = r["clips"][0]["clip"].as_u64().unwrap();
    let id = add(&mut s, json!({"clip": clip}));
    assert_eq!(s.execute("live.status", json!({})).unwrap()["cues"][0]["kind"], "clip");
    s.execute("live.preview", json!({"id": id})).unwrap();
    let f = s.execute("live.frame", json!({"which": "preview", "width": 200, "at": 1.0})).unwrap();
    assert_eq!(f["empty"], false);
    s.execute("live.take", json!({})).unwrap();
    assert!(s.execute("live.tick", json!({"at": 1.0})).unwrap()["onAir"] == true);
    // a video clip of the demo is not a graphic
    let video = s.active_sequence().unwrap().video_tracks.iter().flat_map(|t| t.items.iter()).find(|it| it.graphic.is_none()).map(|it| it.id.0).unwrap();
    assert!(s.execute("live.cue.add", json!({"clip": video})).is_err());
    assert!(s.execute("live.cue.add", json!({"clip": 999_999_999})).is_err());
}

#[test]
fn pictures_are_drawn_on_air() {
    let mut s = demo("live-pictures");
    let dir = tmp_dir("live-pictures-files");
    let red: Vec<u8> = [255u8, 0, 0, 255].repeat(16 * 16);
    std::fs::write(dir.join("logo.png"), cfa4_export::encode_png(red, 16, 16).unwrap()).unwrap();
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, "Head,Body,Page Style\n,{{Pic logo.png}},Card\n").unwrap();
    add(&mut s, json!({"path": sheet.to_string_lossy()}));
    s.execute("live.take", json!({})).unwrap();
    s.execute("live.tick", json!({"at": 2.0})).unwrap();
    let (rgba, w, h, _) = super::tick_program(&mut s, Some(Duration::from_secs(2))).unwrap().unwrap();
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    let reds = rgba.chunks(4).filter(|p| p[0] > 200 && p[1] < 60 && p[3] > 200).count();
    assert!(reds > 100, "the picture is drawn ({reds} red pixels)");
    // a missing picture is left out, not an error
    std::fs::remove_file(dir.join("logo.png")).unwrap();
    s.live.pictures.clear();
    assert!(s.execute("live.tick", json!({"at": 2.0})).is_ok());
}

#[test]
fn sound_modes_follow_the_cue() {
    let mut s = demo("live-sound");
    let dir = tmp_dir("live-sound-files");
    let wav = dir.join("music.wav");
    std::fs::write(&wav, b"RIFF....WAVE").unwrap();
    add(&mut s, json!({"csv": SHEET, "audio": wav.to_string_lossy()}));
    assert!(s.execute("live.cue.add", json!({"csv": SHEET, "audio": dir.join("none.wav").to_string_lossy()})).is_err());
    assert!(s.execute("live.audio", json!({"mode": "loud"})).is_err());
    s.execute("live.audio", json!({"mode": "separate"})).unwrap();
    s.execute("live.take", json!({})).unwrap();
    assert_eq!(s.live.store.audio().as_deref(), Some(wav.as_path()));
    assert_eq!(s.live.store.status()["audioMode"], "separate");
    s.execute("live.audio", json!({"mode": "ignore"})).unwrap();
    assert_eq!(s.live.store.audio(), None);
    s.execute("live.audio", json!({"mode": "beágyazott", "width": 99_999})).unwrap();
    assert_eq!(s.live.store.audio().as_deref(), Some(wav.as_path()));
    assert_eq!(s.execute("live.status", json!({})).unwrap()["width"], 3840);
}

#[test]
fn the_live_page_serves_the_program_on_localhost() {
    let mut s = demo("live-server");
    add(&mut s, json!({"csv": SHEET}));
    let st = s.execute("live.server.start", json!({})).unwrap();
    let port = st["server"]["port"].as_u64().unwrap() as u16;
    assert_eq!(st["server"]["url"], format!("http://127.0.0.1:{port}/"));
    // starting again keeps the same page
    assert_eq!(s.execute("live.server.start", json!({})).unwrap()["server"]["port"], port);
    s.execute("live.take", json!({})).unwrap();
    s.execute("live.tick", json!({"at": 1.0})).unwrap();
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(c, "GET /frame.png HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
    let mut out = Vec::new();
    c.read_to_end(&mut out).unwrap();
    let body = &out[out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4..];
    assert_eq!(body, &**s.live.store.png());
    assert!(body.len() > 200);
    s.execute("live.server.stop", json!({})).unwrap();
    assert!(s.execute("live.status", json!({})).unwrap()["server"].is_null());
    assert!(s.execute("live.server.start", json!({"port": 70_000})).is_err());
}

#[test]
fn obs_is_reached_on_this_machine_only_and_failures_are_errors() {
    let mut s = demo("live-obs");
    let e = s.execute("live.obs.connect", json!({"host": "192.168.1.5"})).unwrap_err().to_string();
    assert!(e.contains("this machine"), "{e}");
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert!(s.execute("live.obs.connect", json!({"port": free, "timeout": 0.3})).is_err());
    assert!(s.execute("live.obs.setup", json!({})).is_err());
    assert!(s.execute("live.obs.disconnect", json!({})).is_ok());
}

#[test]
fn hostile_parameters_are_errors() {
    let mut s = demo("live-hostile");
    assert!(s.execute("live.take", json!({})).is_err(), "nothing prepared");
    assert!(s.execute("live.cue.add", json!({"csv": ""})).is_err());
    assert!(s.execute("live.cue.add", json!({})).is_err());
    let id = add(&mut s, json!({"csv": SHEET, "name": "x".repeat(10_000)}));
    assert_eq!(s.execute("live.status", json!({})).unwrap()["cues"][0]["name"].as_str().unwrap().len(), 200);
    assert!(s.execute("live.preview", json!({"id": 424_242})).is_err());
    assert!(s.execute("live.cue.remove", json!({"id": 424_242})).is_err());
    assert!(s.execute("live.frame", json!({"which": "sideways"})).is_err());
    assert!(s.execute("live.frame", json!({"which": "preview", "page": 99, "at": -5, "width": 0})).is_ok());
    assert!(s.execute("live.tick", json!({"at": f64::MAX})).is_ok());
    assert!(s.execute("live.preview", json!({"next": true})).is_err(), "no cue after the only one");
    s.execute("live.cue.remove", json!({"id": id})).unwrap();
    assert!(s.execute("live.status", json!({})).unwrap()["preview"].is_null());
}
