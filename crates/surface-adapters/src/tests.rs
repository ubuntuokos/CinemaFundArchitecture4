//! Tests of the wire protocol, the profiles and the adapter loop (with the fake backend).

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::task::JoinHandle;

use crate::profile::{self, Control, Profile};
use crate::testing::{FakeBackend, FakeHandle, FakeOp};
use crate::wire::{self, Command, ControlGroup, ErrorCode, Event, InputAction};
use crate::{RunOptions, run};

// ---- wire ----

#[test]
fn events_use_the_documented_json_shape() {
    let e = Event::Input { device: "hid:0300:1010:/dev/hidraw3".into(), control: "keys".into(), index: 4, action: InputAction::Press };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v, serde_json::json!({"type": "input", "device": "hid:0300:1010:/dev/hidraw3", "control": "keys", "index": 4, "action": "press"}));
    let e = Event::Error { device: None, seq: Some(3), code: ErrorCode::PermissionDenied, message: "x".into() };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v, serde_json::json!({"type": "error", "seq": 3, "code": "permission_denied", "message": "x"}));
    let c: Command = serde_json::from_str(r#"{"type":"setBrightness","seq":1,"device":"d","percent":40}"#).unwrap();
    assert_eq!(c, Command::SetBrightness { seq: Some(1), device: "d".into(), percent: 40 });
    let c: Command = serde_json::from_str(r#"{"type":"clearImage","device":"d"}"#).unwrap();
    assert_eq!(c, Command::ClearImage { seq: None, device: "d".into(), index: None });
    assert_eq!(serde_json::from_str::<Command>(r#"{"type":"quit"}"#).unwrap(), Command::Quit);
}

#[test]
fn every_event_round_trips() {
    let events = vec![
        Event::Hello { protocol: wire::PROTOCOL.into(), version: 1, adapter: "mirabox-family".into(), adapter_version: "0.1".into(), models: vec!["m".into()] },
        Event::Connected {
            device: "d".into(),
            model: "m".into(),
            vid: 0x300,
            pid: 0x1010,
            firmware: Some("v1".into()),
            serial_hash: Some(wire::serial_hash("s")),
            serial_unique: false,
            controls: vec![ControlGroup {
                id: "keys".into(),
                control_type: "lcd_key".into(),
                count: 15,
                rows: Some(3),
                columns: Some(5),
                image: Some((85, 85)),
            }],
        },
        Event::Disconnected { device: "d".into() },
        Event::Ack { seq: 9 },
    ];
    for e in events {
        let line = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), e, "{line}");
    }
}

#[test]
fn rgba_is_checked() {
    let px = wire::encode_rgba(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let img = wire::decode_rgba(2, 1, &px).unwrap();
    assert_eq!(img.get_pixel(1, 0).0, [5, 6, 7, 8]);
    assert!(wire::decode_rgba(2, 2, &px).is_err(), "wrong byte count");
    assert!(wire::decode_rgba(0, 1, "").is_err(), "empty size");
    assert!(wire::decode_rgba(2000, 1, &px).is_err(), "too large");
    assert!(wire::decode_rgba(2, 1, "not base64!").is_err());
}

#[test]
fn serial_hash_never_contains_the_serial() {
    let h = wire::serial_hash("355499441494");
    assert!(h.starts_with("sha256:") && h.len() == 7 + 64);
    assert!(!h.contains("355499441494"));
}

// ---- profiles ----

fn akp153e() -> Profile {
    let all = profile::builtin().unwrap();
    all.into_iter().find(|p| p.pid == 0x1010).unwrap()
}

#[test]
fn builtin_profiles_are_valid() {
    let all = profile::builtin().unwrap();
    assert_eq!(all.len(), 2);
    for p in &all {
        assert!(p.problems().is_empty(), "{:?}", p.problems());
        assert_eq!(p.model, "ajazz.akp153e");
        assert!(!p.confirmed, "nothing is confirmed before a CFA4 report on a real unit");
    }
}

#[test]
fn akp153e_layout() {
    let p = akp153e();
    assert_eq!((p.key_count(), p.button_count()), (15, 3));
    let c = p.controls();
    assert_eq!(c[0], ControlGroup { id: "keys".into(), control_type: "lcd_key".into(), count: 15, rows: Some(3), columns: Some(5), image: Some((85, 85)) });
    assert_eq!(c[1], ControlGroup { id: "buttons".into(), control_type: "key".into(), count: 3, rows: Some(3), columns: Some(1), image: None });
    // device 0 sits at display 4 (row 0, column 4): the fifth key with a screen
    assert_eq!(p.control_of_device(0), Some(Control::Key(4)));
    // the last column holds the keys without a screen
    assert_eq!(p.control_of_device(15), Some(Control::Button(0)));
    assert_eq!(p.control_of_device(16), Some(Control::Button(1)));
    assert_eq!(p.control_of_device(17), Some(Control::Button(2)));
    assert_eq!(p.control_of_device(18), None);
    assert_eq!(p.control_of_device(255), None);
    // second row starts at key 5
    assert_eq!(p.display_of(Control::Key(5)), Some(6));
    assert_eq!(p.device_index(Control::Key(0)), Some(12));
    assert_eq!(p.device_index(Control::Key(15)), None);
    assert_eq!(p.device_index(Control::Button(3)), None);
    // every control maps to a device index and back
    for i in 0..p.key_count() {
        let d = p.device_index(Control::Key(i)).unwrap();
        assert_eq!(p.control_of_device(d), Some(Control::Key(i)));
    }
    for i in 0..p.button_count() {
        let d = p.device_index(Control::Button(i)).unwrap();
        assert_eq!(p.control_of_device(d), Some(Control::Button(i)));
    }
}

const ONE: &str = r#"
[[profile]]
model = "x.y"
name = "X"
vid = 1
pid = 2
usage_page = 0xffa0
usage = 1
protocol_version = 1
raw_keys = 4
unique_serial = true
confirmed = false
image = { mode = "bmp", width = 72, height = 72, rotation = 0, mirror = "none" }
display_to_device = [0, 1, 2, 3]
grid = { rows = 2, columns = 2 }
lcd_columns = 2
"#;

#[test]
fn profile_problems_are_refused() {
    assert_eq!(profile::parse(ONE).unwrap().len(), 1);
    let bad = |from: &str, to: &str| profile::parse(&ONE.replace(from, to)).unwrap_err();
    assert!(bad("protocol_version = 1", "protocol_version = 0").contains("protocol_version 0"));
    assert!(bad("protocol_version = 1", "protocol_version = 4").contains("protocol_version 4"));
    assert!(bad("raw_keys = 4", "raw_keys = 5").contains("raw_keys"));
    assert!(bad("[0, 1, 2, 3]", "[0, 1, 1, 3]").contains("display_to_device"));
    assert!(bad("[0, 1, 2, 3]", "[0, 1, 2]").contains("display_to_device"));
    assert!(bad("rotation = 0", "rotation = 45").contains("rotation"));
    assert!(bad("width = 72", "width = 0").contains("image size"));
    assert!(bad("lcd_columns = 2", "lcd_columns = 3").contains("lcd_columns"));
    assert!(bad("lcd_columns = 2", "lcd_columns = 2\nextra = 1").contains("extra"), "unknown keys are refused");
    let twice = format!("{ONE}{ONE}");
    assert!(profile::parse(&twice).unwrap_err().contains("share a USB id"));
}

#[test]
fn a_grid_without_screen_keys_is_handled() {
    let p = &profile::parse(&ONE.replace("lcd_columns = 2", "lcd_columns = 0")).unwrap()[0];
    assert_eq!((p.key_count(), p.button_count()), (0, 4));
    assert_eq!(p.display_of(Control::Key(0)), None);
    assert_eq!(p.control_of_device(3), Some(Control::Button(3)));
}

// ---- the adapter loop ----

struct Harness {
    commands: DuplexStream,
    events: Lines<BufReader<DuplexStream>>,
    fake: FakeHandle,
    task: JoinHandle<std::io::Result<()>>,
}

const A: &str = "/dev/hidraw7";
const B: &str = "/dev/hidraw8";
const ID_A: &str = "hid:0300:1010:/dev/hidraw7";
const ID_B: &str = "hid:0300:1010:/dev/hidraw8";

fn opts() -> RunOptions {
    RunOptions { scan_every: Duration::from_millis(10), read_slice: Duration::from_millis(5), retry_after: Duration::from_millis(200) }
}

impl Harness {
    fn start() -> Harness {
        let (commands, cmd_in) = tokio::io::duplex(1 << 20);
        let (ev_out, ev_in) = tokio::io::duplex(1 << 20);
        let (backend, fake) = FakeBackend::new();
        let profiles = profile::builtin().unwrap();
        let task = tokio::spawn(run(backend, "test", profiles, BufReader::new(cmd_in), ev_out, opts()));
        Harness { commands, events: BufReader::new(ev_in).lines(), fake, task }
    }

    async fn next(&mut self) -> Event {
        let line = tokio::time::timeout(Duration::from_secs(5), self.events.next_line()).await.expect("no event in time").unwrap().expect("events ended");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
    }

    /// The next event that `pred` accepts; others are skipped.
    async fn until(&mut self, pred: impl Fn(&Event) -> bool) -> Event {
        loop {
            let e = self.next().await;
            if pred(&e) {
                return e;
            }
        }
    }

    async fn send(&mut self, line: &str) {
        self.commands.write_all(line.as_bytes()).await.unwrap();
        self.commands.write_all(b"\n").await.unwrap();
    }

    async fn hello_and_connect(&mut self, path: &str) -> Event {
        self.fake.plug(path, 0);
        self.until(|e| matches!(e, Event::Connected { .. })).await
    }

    async fn quit(mut self) -> FakeHandle {
        self.send(r#"{"type":"quit"}"#).await;
        tokio::time::timeout(Duration::from_secs(5), self.task).await.expect("run did not end").unwrap().unwrap();
        self.fake
    }
}

fn rgba_cmd(seq: u64, device: &str, control: &str, index: u32) -> String {
    let px = wire::encode_rgba(&[255; 2 * 2 * 4]);
    format!(r#"{{"type":"setImage","seq":{seq},"device":"{device}","control":"{control}","index":{index},"width":2,"height":2,"rgba":"{px}"}}"#)
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_comes_first() {
    let mut h = Harness::start();
    let Event::Hello { protocol, version, adapter, models, .. } = h.next().await else { panic!("not hello") };
    assert_eq!((protocol.as_str(), version, adapter.as_str()), (wire::PROTOCOL, 1, "mirabox-family"));
    assert_eq!(models, vec!["ajazz.akp153e".to_owned()]);
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugged_deck_connects_and_reports_keys() {
    let mut h = Harness::start();
    h.fake.set_serial("355499441494");
    let Event::Connected { device, model, vid, pid, firmware, serial_hash, serial_unique, controls } = h.hello_and_connect(A).await else { unreachable!() };
    assert_eq!((device.as_str(), model.as_str(), vid, pid), (ID_A, "ajazz.akp153e", 0x0300, 0x1010));
    assert_eq!(firmware.as_deref(), Some("fake-1.0"));
    assert_eq!(serial_hash, Some(wire::serial_hash("355499441494")));
    assert!(!serial_unique);
    assert_eq!(controls, akp153e().controls());

    h.fake.press(A, 0, true);
    h.fake.press(A, 0, false);
    h.fake.press(A, 17, true);
    let input = |control: &str, index: u32, action: InputAction| Event::Input { device: ID_A.into(), control: control.into(), index, action };
    assert_eq!(h.next().await, input("keys", 4, InputAction::Press));
    assert_eq!(h.next().await, input("keys", 4, InputAction::Release));
    assert_eq!(h.next().await, input("buttons", 2, InputAction::Press));
    // a device index outside the profile is dropped, not reported
    h.fake.press(A, 200, true);
    h.fake.press(A, 1, true);
    // device 1 sits at display 10 (row 1, column 4)
    assert_eq!(h.next().await, input("keys", 9, InputAction::Press));
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_reach_the_device_and_are_acked() {
    let mut h = Harness::start();
    h.hello_and_connect(A).await;
    h.send(&rgba_cmd(7, ID_A, "keys", 0)).await;
    assert_eq!(h.next().await, Event::Ack { seq: 7 });
    h.send(&format!(r#"{{"type":"clearImage","seq":8,"device":"{ID_A}","index":1}}"#)).await;
    assert_eq!(h.next().await, Event::Ack { seq: 8 });
    h.send(&format!(r#"{{"type":"clearImage","seq":9,"device":"{ID_A}"}}"#)).await;
    assert_eq!(h.next().await, Event::Ack { seq: 9 });
    h.send(&format!(r#"{{"type":"setBrightness","seq":10,"device":"{ID_A}","percent":150}}"#)).await;
    assert_eq!(h.next().await, Event::Ack { seq: 10 });
    // without a seq there is no answer; the sleep after it shows it was applied in order
    h.send(&format!(r#"{{"type":"sleep","device":"{ID_A}"}}"#)).await;
    h.send(&format!(r#"{{"type":"setBrightness","seq":11,"device":"{ID_A}","percent":5}}"#)).await;
    assert_eq!(h.next().await, Event::Ack { seq: 11 });
    let fake = h.quit().await;
    assert_eq!(
        fake.ops(A),
        vec![
            FakeOp::Open,
            FakeOp::SetImage(12, 2, 2),
            FakeOp::Clear(Some(9)),
            FakeOp::Clear(None),
            FakeOp::Brightness(100),
            FakeOp::Sleep,
            FakeOp::Brightness(5),
            FakeOp::Close,
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_commands_get_machine_readable_errors() {
    let mut h = Harness::start();
    h.hello_and_connect(A).await;
    let code_of = |e: Event| match e {
        Event::Error { code, seq, .. } => (code, seq),
        other => panic!("not an error: {other:?}"),
    };
    h.send("{not json").await;
    assert_eq!(code_of(h.next().await), (ErrorCode::BadRequest, None));
    h.send(r#"{"type":"explode"}"#).await;
    assert_eq!(code_of(h.next().await), (ErrorCode::BadRequest, None));
    h.send(&rgba_cmd(1, "hid:0300:1010:/dev/nothing", "keys", 0)).await;
    assert_eq!(code_of(h.next().await), (ErrorCode::DeviceNotFound, Some(1)));
    h.send(&rgba_cmd(2, ID_A, "buttons", 0)).await;
    assert_eq!(code_of(h.next().await), (ErrorCode::Unsupported, Some(2)));
    h.send(&rgba_cmd(3, ID_A, "keys", 15)).await;
    assert_eq!(code_of(h.next().await), (ErrorCode::Unsupported, Some(3)));
    h.send(&format!(r#"{{"type":"setImage","seq":4,"device":"{ID_A}","control":"keys","index":0,"width":3,"height":3,"rgba":"AAAA"}}"#)).await;
    assert_eq!(code_of(h.next().await), (ErrorCode::BadRequest, Some(4)));
    h.send(&format!(r#"{{"type":"clearImage","seq":5,"device":"{ID_A}","index":99}}"#)).await;
    assert_eq!(code_of(h.next().await), (ErrorCode::Unsupported, Some(5)));
    // the device still works
    h.send(&format!(r#"{{"type":"sleep","seq":6,"device":"{ID_A}"}}"#)).await;
    assert_eq!(h.next().await, Event::Ack { seq: 6 });
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panic_in_one_device_leaves_the_others_running() {
    let mut h = Harness::start();
    h.hello_and_connect(A).await;
    h.hello_and_connect(B).await;
    h.fake.panic_on_image(A);
    h.send(&rgba_cmd(1, ID_A, "keys", 0)).await;
    let e = h.until(|e| matches!(e, Event::Error { .. })).await;
    assert!(matches!(e, Event::Error { ref device, code: ErrorCode::Internal, .. } if device.as_deref() == Some(ID_A)), "{e:?}");
    assert_eq!(h.next().await, Event::Disconnected { device: ID_A.into() });
    h.send(&format!(r#"{{"type":"setBrightness","seq":2,"device":"{ID_B}","percent":30}}"#)).await;
    assert_eq!(h.until(|e| matches!(e, Event::Ack { .. })).await, Event::Ack { seq: 2 });
    // after the hold-off the deck that is still plugged in is opened again
    let e = h.until(|e| matches!(e, Event::Connected { .. })).await;
    assert!(matches!(e, Event::Connected { ref device, .. } if device == ID_A));
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unplugging_disconnects() {
    let mut h = Harness::start();
    h.hello_and_connect(A).await;
    h.fake.unplug(A);
    assert_eq!(h.until(|e| matches!(e, Event::Disconnected { .. })).await, Event::Disconnected { device: ID_A.into() });
    h.send(&rgba_cmd(1, ID_A, "keys", 0)).await;
    assert!(matches!(h.next().await, Event::Error { code: ErrorCode::DeviceNotFound, .. }));
    h.fake.plug(A, 0);
    assert!(matches!(h.until(|e| matches!(e, Event::Connected { .. })).await, Event::Connected { .. }));
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_failure_disconnects_and_retries() {
    let mut h = Harness::start();
    h.hello_and_connect(A).await;
    h.fake.fail_read(A);
    let e = h.next().await;
    assert!(matches!(e, Event::Error { code: ErrorCode::IoError, .. }), "{e:?}");
    assert_eq!(h.next().await, Event::Disconnected { device: ID_A.into() });
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_open_failure_is_reported_once_and_retried() {
    let mut h = Harness::start();
    h.next().await; // hello
    h.fake.fail_open(A, ErrorCode::PermissionDenied);
    h.fake.plug(A, 0);
    let e = h.next().await;
    assert!(matches!(e, Event::Error { code: ErrorCode::PermissionDenied, ref device, .. } if device.as_deref() == Some(ID_A)), "{e:?}");
    h.fake.allow_open(A);
    // nothing in between: the hold-off keeps the error from repeating on every scan
    assert!(matches!(h.next().await, Event::Connected { .. }));
    h.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn end_of_input_closes_every_device() {
    let mut h = Harness::start();
    h.hello_and_connect(A).await;
    drop(h.commands);
    tokio::time::timeout(Duration::from_secs(5), h.task).await.unwrap().unwrap().unwrap();
    assert_eq!(h.fake.ops(A).last(), Some(&FakeOp::Close));
}
