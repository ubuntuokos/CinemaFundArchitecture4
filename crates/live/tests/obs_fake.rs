//! The obs-websocket client against a stand-in OBS (handshake, password, requests, events in
//! between, failures, a server that hangs up).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cfa4_live::obs::{ObsClient, ObsError, SourceSetup, auth_string};
use cfa4_live::ws::accept_key;
use serde_json::{Value, json};

fn write_frame(s: &mut TcpStream, text: &str) {
    let b = text.as_bytes();
    let mut f = vec![0x81u8];
    if b.len() < 126 {
        f.push(b.len() as u8);
    } else {
        f.push(126);
        f.extend_from_slice(&(b.len() as u16).to_be_bytes());
    }
    f.extend_from_slice(b);
    s.write_all(&f).unwrap();
}

fn read_frame(r: &mut impl Read) -> Option<String> {
    loop {
        match read_one(r)? {
            (0xA, _) => continue, // the client's pong
            (_, t) => return Some(t),
        }
    }
}

fn read_one(r: &mut impl Read) -> Option<(u8, String)> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h).ok()?;
    assert!(h[1] & 0x80 != 0, "client frames are masked");
    let mut len = u64::from(h[1] & 0x7f);
    if len == 126 {
        let mut b = [0u8; 2];
        r.read_exact(&mut b).ok()?;
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).ok()?;
        len = u64::from_be_bytes(b);
    }
    let mut m = [0u8; 4];
    r.read_exact(&mut m).ok()?;
    let mut p = vec![0u8; len as usize];
    r.read_exact(&mut p).ok()?;
    for (i, x) in p.iter_mut().enumerate() {
        *x ^= m[i % 4];
    }
    if h[0] & 0x0f == 8 {
        return None;
    }
    Some((h[0] & 0x0f, String::from_utf8(p).unwrap()))
}

/// A stand-in OBS: password "pw", one scene "Show", inputs created on request; every request it
/// gets is logged.
fn fake_obs(existing: &'static [&'static str]) -> (u16, Arc<Mutex<Vec<Value>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let mut s = conn.unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut key = String::new();
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.strip_prefix("Sec-WebSocket-Key:") {
                    key = v.trim().to_string();
                }
            }
            write!(s, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key))
                .unwrap();
            write_frame(
                &mut s,
                &json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1, "authentication": {"challenge": "ch", "salt": "sa"}}}).to_string(),
            );
            let Some(id) = read_frame(&mut r) else { continue };
            let id: Value = serde_json::from_str(&id).unwrap();
            if id["d"]["authentication"] != auth_string("pw", "sa", "ch").as_str() {
                let _ = s.write_all(&[0x88, 0]);
                continue;
            }
            write_frame(&mut s, &json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string());
            let mut inputs: Vec<String> = existing.iter().map(|x| x.to_string()).collect();
            while let Some(t) = read_frame(&mut r) {
                let v: Value = serde_json::from_str(&t).unwrap();
                log2.lock().unwrap().push(v.clone());
                let d = &v["d"];
                let rid = d["requestId"].clone();
                // an event and a ping first: the client must skip and answer them
                write_frame(&mut s, &json!({"op": 5, "d": {"eventType": "CurrentProgramSceneChanged"}}).to_string());
                s.write_all(&[0x89, 0]).unwrap();
                let (ok, data) = match d["requestType"].as_str().unwrap() {
                    "GetInputList" => (true, json!({"inputs": inputs.iter().map(|n| json!({"inputName": n})).collect::<Vec<_>>()})),
                    "GetCurrentProgramScene" => (true, json!({"currentProgramSceneName": "Show"})),
                    "CreateInput" => {
                        inputs.push(d["requestData"]["inputName"].as_str().unwrap().to_string());
                        (true, json!({"sceneItemId": 7}))
                    }
                    "SetInputSettings" | "PressInputPropertiesButton" | "TriggerMediaInputAction" => (true, Value::Null),
                    _ => (false, Value::Null),
                };
                let mut resp = json!({"op": 7, "d": {"requestType": d["requestType"], "requestId": rid, "requestStatus": {"result": ok, "code": if ok { 100 } else { 204 }, "comment": if ok { "" } else { "unknown request" }}}});
                if !data.is_null() {
                    resp["d"]["responseData"] = data;
                }
                write_frame(&mut s, &resp.to_string());
            }
        }
    });
    (port, log)
}

const T: Duration = Duration::from_secs(5);

#[test]
fn creates_then_updates_the_browser_source() {
    let (port, log) = fake_obs(&[]);
    let mut c = ObsClient::connect("127.0.0.1", port, Some("pw"), T).unwrap();
    assert_eq!(c.version, "5.5.0");
    let r = c.point_browser_source("CFA4 Live", "http://127.0.0.1:9/", 1920, 1080, None, true).unwrap();
    assert_eq!(r, SourceSetup::Created { scene: "Show".into() });
    let created = log.lock().unwrap().iter().find(|v| v["d"]["requestType"] == "CreateInput").cloned().unwrap();
    let d = &created["d"]["requestData"];
    assert_eq!((d["sceneName"].as_str(), d["inputKind"].as_str()), (Some("Show"), Some("browser_source")));
    assert_eq!((d["inputSettings"]["url"].as_str(), d["inputSettings"]["reroute_audio"].as_bool()), (Some("http://127.0.0.1:9/"), Some(true)));
    // again: the same source, pointed anew and reloaded
    assert_eq!(c.point_browser_source("CFA4 Live", "http://127.0.0.1:10/", 1280, 720, None, false).unwrap(), SourceSetup::Updated);
    assert_eq!(c.point_media_source("CFA4 Live Audio", "http://127.0.0.1:10/audio", Some("Other")).unwrap(), SourceSetup::Created { scene: "Other".into() });
    c.restart_media("CFA4 Live Audio").unwrap();
    match c.request("NoSuchRequest", json!({})) {
        Err(ObsError::Request { code: 204, .. }) => {}
        other => panic!("{other:?}"),
    }
    c.close();
}

#[test]
fn password_problems_are_errors() {
    let (port, _) = fake_obs(&[]);
    assert!(matches!(ObsClient::connect("127.0.0.1", port, None, T), Err(ObsError::Auth(_))));
    assert!(matches!(ObsClient::connect("127.0.0.1", port, Some("wrong"), T), Err(ObsError::Auth(_))));
    assert!(ObsClient::connect("127.0.0.1", port, Some("pw"), T).is_ok());
}

#[test]
fn nothing_listening_or_not_websocket() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert!(matches!(ObsClient::connect("127.0.0.1", free, None, Duration::from_millis(500)), Err(ObsError::Connect(_))));
    // an HTTP server that is not OBS
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let mut c = c.unwrap();
            let mut buf = [0u8; 1024];
            let _ = c.read(&mut buf);
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    assert!(matches!(ObsClient::connect("127.0.0.1", port, None, T), Err(ObsError::Connect(_))));
}
