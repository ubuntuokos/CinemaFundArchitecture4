//! Encrypted DCPs (#933, owner decision 2026-11-12: playback and import only): the user gives a
//! KDM (ST 430-1) and the RSA private key of its recipient; the content keys are decrypted and put
//! into the in-memory key ring of `cfa4-mxf`, which then decrypts the ST 429-6 track files on read.
//!
//! Keys are never written anywhere: not into the project, the journal (these commands are not
//! journaled), settings or logs. They live until `dcp.clearKeys` or the end of the process.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p};
use crate::{Result, Session};

/// Largest KDM and private key files read.
const MAX_KDM: u64 = 4 << 20;
const MAX_KEY: u64 = 64 << 10;

fn read_text(s: &Session, cmd: &str, path: &str, limit: u64, what: &str) -> Result<String> {
    let size = s.services.file_size(path).map_err(|e| bad(cmd, format!("{what} {path}: {e}")))?;
    if size > limit {
        return Err(bad(cmd, format!("{what} {path} is {size} bytes; a {what} is at most {limit}")));
    }
    let b = s.services.read_file(path).map_err(|e| bad(cmd, format!("{what} {path}: {e}")))?;
    String::from_utf8(b).map_err(|_| bad(cmd, format!("{what} {path} is not text")))
}

fn uuid(u: &cfa4_dcp::Uuid) -> String {
    cfa4_mxf::crypt::uuid_text(&u.0)
}

/// Loads the content keys of a KDM; returns what it holds (never the keys themselves).
pub(crate) fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "dcp.loadKdm";
    let kdm_path = str_p(p, "kdm").ok_or_else(|| bad(CMD, "kdm: the KDM file path is required"))?;
    let key_path = str_p(p, "privateKey").ok_or_else(|| bad(CMD, "privateKey: the recipient's PEM private key file is required"))?;
    let xml = read_text(s, CMD, kdm_path, MAX_KDM, "KDM")?;
    let pem = read_text(s, CMD, key_path, MAX_KEY, "private key")?;
    let key = cfa4_dcp::kdm::private_key_from_pem(&pem).map_err(|e| bad(CMD, e.to_string()))?;
    drop(pem);
    let kdm = cfa4_dcp::kdm::open_kdm(&xml, &key, chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())).map_err(|e| bad(CMD, e.to_string()))?;
    let mut keys = Vec::new();
    for k in &kdm.keys {
        let content = match k.key_type.as_str() {
            // the MIC key of the content key with the same KeyId (ST 430-1 Table 2)
            "MDMK" => cfa4_mxf::crypt::ContentKey { cipher: [0; 16], mic: Some(k.key) },
            _ => cfa4_mxf::crypt::ContentKey { cipher: k.key, mic: None },
        };
        cfa4_mxf::crypt::add_key(k.key_id.0, content);
        keys.push(json!({"keyId": uuid(&k.key_id), "type": k.key_type, "notValidBefore": k.not_before, "notValidAfter": k.not_after}));
    }
    // sources opened before the keys were there reopen and decode now
    s.media.clear();
    Ok(json!({
        "title": kdm.content_title,
        "annotation": kdm.annotation,
        "composition": kdm.cpl_id.as_ref().map(uuid),
        "keys": keys,
        "loaded": cfa4_mxf::crypt::key_ids().len(),
    }))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "dcp.loadKdm",
            label: "Load KDM",
            menu: &[],
            shortcut: None,
            params: r#"{"kdm":str,"privateKey":str}"#,
            enabled: always,
            run: load,
            journal: false,
        },
        CommandSpec {
            id: "dcp.keys",
            label: "DCP Keys",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |_, _| {
                let ids: Vec<String> = cfa4_mxf::crypt::key_ids().iter().map(cfa4_mxf::crypt::uuid_text).collect();
                Ok(json!({"keyIds": ids}))
            },
            journal: false,
        },
        CommandSpec {
            id: "dcp.clearKeys",
            label: "Forget DCP Keys",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| {
                cfa4_mxf::crypt::clear_keys();
                s.media.clear();
                Ok(json!({"loaded": 0}))
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "kdm_tests.rs"]
mod tests;
