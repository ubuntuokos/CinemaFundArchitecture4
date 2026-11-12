//! obs-websocket 5 client: connect (with the password OBS asks for), then point a Browser
//! Source at CFA4's live page, or add a media source for the separate sound. CFA4 drives OBS
//! only this far; OBS never drives CFA4.
//!
//! Protocol: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>
//! (Hello op 0, Identify op 1, Identified op 2, Request op 6, RequestResponse op 7).

use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ws::{WsClient, WsError};

/// OBS's default obs-websocket port.
pub const DEFAULT_PORT: u16 = 4455;
/// Most messages read while waiting for one answer (events in between are skipped).
const MAX_SKIPPED: usize = 10_000;

#[derive(Debug)]
pub enum ObsError {
    Connect(WsError),
    /// OBS wants a password (none given) or refused the one given.
    Auth(String),
    /// A request OBS answered with a failure.
    Request {
        request: String,
        code: i64,
        comment: String,
    },
    Protocol(String),
}

impl std::fmt::Display for ObsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObsError::Connect(e) => write!(f, "cannot reach OBS: {e}"),
            ObsError::Auth(m) => write!(f, "OBS refused the connection: {m}"),
            ObsError::Request { request, code, comment } => write!(f, "OBS could not do {request} ({code}): {comment}"),
            ObsError::Protocol(m) => write!(f, "unexpected answer from OBS: {m}"),
        }
    }
}

impl std::error::Error for ObsError {}

impl From<WsError> for ObsError {
    fn from(e: WsError) -> Self {
        ObsError::Connect(e)
    }
}

/// The authentication string for `password` (obs-websocket 5: base64 SHA-256 of the
/// base64 SHA-256 of password + salt, followed by the challenge).
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret = b64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    b64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

pub struct ObsClient {
    ws: WsClient,
    next_id: u64,
    /// OBS's obs-websocket version (from its Hello).
    pub version: String,
}

/// What [`ObsClient::point_browser_source`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum SourceSetup {
    Created { scene: String },
    Updated,
}

impl ObsClient {
    pub fn connect(host: &str, port: u16, password: Option<&str>, timeout: Duration) -> Result<ObsClient, ObsError> {
        let mut ws = WsClient::connect(host, port, timeout)?;
        let hello = parse(&ws.read_text()?)?;
        if hello["op"] != 0 {
            return Err(ObsError::Protocol(format!("expected Hello, got op {}", hello["op"])));
        }
        let d = &hello["d"];
        let version = d["obsWebSocketVersion"].as_str().unwrap_or_default().to_string();
        let mut identify = json!({"rpcVersion": 1, "eventSubscriptions": 0});
        if let Some(a) = d.get("authentication").filter(|a| a.is_object()) {
            let Some(pw) = password.filter(|p| !p.is_empty()) else {
                return Err(ObsError::Auth("OBS asks for the WebSocket server password (Tools ▸ WebSocket Server Settings)".into()));
            };
            identify["authentication"] = json!(auth_string(pw, a["salt"].as_str().unwrap_or_default(), a["challenge"].as_str().unwrap_or_default()));
        }
        ws.send_text(&json!({"op": 1, "d": identify}).to_string())?;
        match ws.read_text() {
            Ok(t) => {
                let v = parse(&t)?;
                if v["op"] != 2 {
                    return Err(ObsError::Protocol(format!("expected Identified, got op {}", v["op"])));
                }
            }
            Err(WsError::Closed | WsError::Io(_)) => return Err(ObsError::Auth("wrong password, or OBS closed the connection".into())),
            Err(e) => return Err(e.into()),
        }
        Ok(ObsClient { ws, next_id: 1, version })
    }

    /// Send a request and wait for its answer (`responseData`, or null).
    pub fn request(&mut self, request_type: &str, data: Value) -> Result<Value, ObsError> {
        let id = format!("cfa4-{}", self.next_id);
        self.next_id += 1;
        let mut d = json!({"requestType": request_type, "requestId": id});
        if !data.is_null() {
            d["requestData"] = data;
        }
        self.ws.send_text(&json!({"op": 6, "d": d}).to_string())?;
        for _ in 0..MAX_SKIPPED {
            let v = parse(&self.ws.read_text()?)?;
            if v["op"] != 7 || v["d"]["requestId"] != id.as_str() {
                continue;
            }
            let st = &v["d"]["requestStatus"];
            if st["result"].as_bool() == Some(true) {
                return Ok(v["d"].get("responseData").cloned().unwrap_or(Value::Null));
            }
            return Err(ObsError::Request {
                request: request_type.to_string(),
                code: st["code"].as_i64().unwrap_or(0),
                comment: st["comment"].as_str().unwrap_or_default().to_string(),
            });
        }
        Err(ObsError::Protocol(format!("no answer to {request_type}")))
    }

    fn input_exists(&mut self, name: &str) -> Result<bool, ObsError> {
        let list = self.request("GetInputList", Value::Null)?;
        Ok(list["inputs"].as_array().is_some_and(|a| a.iter().any(|i| i["inputName"] == name)))
    }

    fn scene(&mut self, scene: Option<&str>) -> Result<String, ObsError> {
        if let Some(s) = scene.filter(|s| !s.trim().is_empty()) {
            return Ok(s.to_string());
        }
        let r = self.request("GetCurrentProgramScene", Value::Null)?;
        r["currentProgramSceneName"]
            .as_str()
            .or_else(|| r["sceneName"].as_str())
            .map(str::to_string)
            .ok_or_else(|| ObsError::Protocol("OBS named no program scene".into()))
    }

    /// Point the Browser Source `name` at `url` (`width`×`height`), creating it in `scene` (the
    /// program scene by default) when OBS has none of that name. `obs_audio`: OBS takes the
    /// page's sound ("Control audio via OBS"), for the embedded audio mode.
    pub fn point_browser_source(
        &mut self,
        name: &str,
        url: &str,
        width: u32,
        height: u32,
        scene: Option<&str>,
        obs_audio: bool,
    ) -> Result<SourceSetup, ObsError> {
        let settings = json!({"url": url, "width": width, "height": height, "reroute_audio": obs_audio, "shutdown": false, "restart_when_active": false});
        if self.input_exists(name)? {
            self.request("SetInputSettings", json!({"inputName": name, "inputSettings": settings, "overlay": true}))?;
            // load the page again (a new port, say); a refusal leaves the source as set
            let _ = self.request("PressInputPropertiesButton", json!({"inputName": name, "propertyName": "refreshnocache"}));
            return Ok(SourceSetup::Updated);
        }
        let scene = self.scene(scene)?;
        self.request(
            "CreateInput",
            json!({"sceneName": scene, "inputName": name, "inputKind": "browser_source", "inputSettings": settings, "sceneItemEnabled": true}),
        )?;
        Ok(SourceSetup::Created { scene })
    }

    /// A media source playing `url` (the separate sound), created in `scene` when missing.
    pub fn point_media_source(&mut self, name: &str, url: &str, scene: Option<&str>) -> Result<SourceSetup, ObsError> {
        let settings = json!({"is_local_file": false, "input": url, "restart_on_activate": true, "close_when_inactive": true});
        if self.input_exists(name)? {
            self.request("SetInputSettings", json!({"inputName": name, "inputSettings": settings, "overlay": true}))?;
            return Ok(SourceSetup::Updated);
        }
        let scene = self.scene(scene)?;
        self.request(
            "CreateInput",
            json!({"sceneName": scene, "inputName": name, "inputKind": "ffmpeg_source", "inputSettings": settings, "sceneItemEnabled": true}),
        )?;
        Ok(SourceSetup::Created { scene })
    }

    /// Restart a media source (a new cue's sound from its start).
    pub fn restart_media(&mut self, name: &str) -> Result<(), ObsError> {
        self.request("TriggerMediaInputAction", json!({"inputName": name, "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART"})).map(|_| ())
    }

    pub fn close(&mut self) {
        self.ws.close();
    }
}

fn parse(text: &str) -> Result<Value, ObsError> {
    serde_json::from_str(text).map_err(|e| ObsError::Protocol(format!("not JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_string_matches_obs_websocket() {
        // computed independently: secret = b64(sha256("supersecretpasswordsalt")), then
        // b64(sha256(secret + "challenge"))
        let secret = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(b"pwsalt"));
        let want = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(format!("{secret}ch").as_bytes()));
        assert_eq!(auth_string("pw", "salt", "ch"), want);
        assert_ne!(auth_string("pw", "salt", "ch"), auth_string("pw2", "salt", "ch"));
    }
}
