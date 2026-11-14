//! Encrypted DCPs (#933, owner decision 2026-11-12: playback and import only): the user gives a
//! KDM (ST 430-1), or a Key Delivery Bundle of KDMs (ST 430-9, #1144), and the RSA private key of
//! its recipient; the content keys are decrypted and put into the in-memory key ring of
//! `cfa4-mxf`, which then decrypts the ST 429-6 track files on read. From a bundle, every KDM made
//! for that key loads (each under the signature rule below); those for other recipients are
//! skipped.
//!
//! Keys are never written anywhere: not into the project, the journal (these commands are not
//! journaled), settings or logs. They live until `dcp.clearKeys` or the end of the process.
//!
//! The KDM's XML signature and its signer's certificate chain are checked (#1028, ST 430-1 / 430-2
//! / 430-3). An unsigned KDM, or one whose signed content does not check out (changed after
//! signing), is always refused. One whose content is intact but whose certificate chain does not
//! keep ST 430-2 (a test KDM, an expired or non-conforming certificate) is refused unless the
//! caller passes `allowUnverified` (owner decision 2026-11-13); the answer always says how the
//! check went.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p};
use crate::{Result, Session};

/// Largest KDM and private key files read.
const MAX_KDM: u64 = 4 << 20;
const MAX_KEY: u64 = 64 << 10;

fn read_bytes(s: &Session, cmd: &str, path: &str, limit: u64, what: &str) -> Result<Vec<u8>> {
    let size = s.services.file_size(path).map_err(|e| bad(cmd, format!("{what} {path}: {e}")))?;
    if size > limit {
        return Err(bad(cmd, format!("{what} {path} is {size} bytes; a {what} is at most {limit}")));
    }
    s.services.read_file(path).map_err(|e| bad(cmd, format!("{what} {path}: {e}")))
}

fn text(cmd: &str, path: &str, b: Vec<u8>, what: &str) -> Result<String> {
    String::from_utf8(b).map_err(|_| bad(cmd, format!("{what} {path} is not text")))
}

fn uuid(u: &cfa4_dcp::Uuid) -> String {
    cfa4_mxf::crypt::uuid_text(&u.0)
}

fn signature_json(sig: &cfa4_dcp::kdm::Signature) -> Value {
    match sig {
        cfa4_dcp::kdm::Signature::Valid { chain } => json!({"valid": true, "chain": chain}),
        cfa4_dcp::kdm::Signature::Untrusted { chain, problem } => json!({"valid": false, "overridable": true, "chain": chain, "problem": problem}),
        cfa4_dcp::kdm::Signature::Missing => json!({"valid": false, "overridable": false, "problem": "the KDM is not signed"}),
        cfa4_dcp::kdm::Signature::Invalid(e) => json!({"valid": false, "overridable": false, "problem": e}),
    }
}

/// Why the keys of `kdm` may not be loaded, under the signature rule (owner decision 2026-11-13).
fn refusal(kdm: &cfa4_dcp::kdm::Kdm, allow_unverified: bool) -> Option<String> {
    let sig = signature_json(&kdm.signature);
    let problem = sig["problem"].as_str().unwrap_or_default();
    if !kdm.signature.is_valid() && !kdm.signature.is_overridable() {
        return Some(format!("the KDM's signed content does not check out ({problem}); its keys are not loaded"));
    }
    if kdm.signature.is_overridable() && !allow_unverified {
        return Some(format!("the KDM's certificate chain does not check out ({problem}); pass allowUnverified to load its keys anyway"));
    }
    None
}

/// Puts the keys of `kdm` into the key ring; what was loaded (never the keys themselves).
fn add_keys(kdm: &cfa4_dcp::kdm::Kdm) -> Vec<Value> {
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
    keys
}

fn describe(kdm: &cfa4_dcp::kdm::Kdm, keys: Vec<Value>) -> Value {
    json!({
        "title": kdm.content_title,
        "annotation": kdm.annotation,
        "composition": kdm.cpl_id.as_ref().map(uuid),
        "keys": keys,
        "signature": signature_json(&kdm.signature),
    })
}

/// Loads the content keys of a KDM, or of every KDM of a Key Delivery Bundle (ST 430-9) made for
/// the private key; returns what was loaded (never the keys themselves).
pub(crate) fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "dcp.loadKdm";
    let kdm_path = str_p(p, "kdm").ok_or_else(|| bad(CMD, "kdm: the KDM file path is required"))?;
    let key_path = str_p(p, "privateKey").ok_or_else(|| bad(CMD, "privateKey: the recipient's PEM private key file is required"))?;
    let allow = bool_p(p, "allowUnverified").unwrap_or(false);
    let data = read_bytes(s, CMD, kdm_path, cfa4_dcp::kdb::MAX_BUNDLE, "KDM")?;
    let pem = text(CMD, key_path, read_bytes(s, CMD, key_path, MAX_KEY, "private key")?, "private key")?;
    let key = cfa4_dcp::kdm::private_key_from_pem(&pem).map_err(|e| bad(CMD, e.to_string()))?;
    drop(pem);
    let now = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now());
    if cfa4_dcp::kdb::is_ustar(&data) {
        return load_bundle(s, &data, &key, now, allow);
    }
    if data.len() as u64 > MAX_KDM {
        return Err(bad(CMD, format!("KDM {kdm_path} is {} bytes; a KDM is at most {MAX_KDM}", data.len())));
    }
    let xml = text(CMD, kdm_path, data, "KDM")?;
    let kdm = cfa4_dcp::kdm::open_kdm(&xml, &key, now).map_err(|e| bad(CMD, e.to_string()))?;
    if let Some(why) = refusal(&kdm, allow) {
        return Err(bad(CMD, why));
    }
    let keys = add_keys(&kdm);
    // sources opened before the keys were there reopen and decode now
    s.media.clear();
    let mut r = describe(&kdm, keys);
    r["loaded"] = json!(cfa4_mxf::crypt::key_ids().len());
    Ok(r)
}

/// The KDMs of a bundle made for `key`: each under the signature rule on its own. KDMs for other
/// recipients are skipped; a bundle where none loads is an error saying why.
fn load_bundle(s: &mut Session, tar: &[u8], key: &cfa4_dcp::kdm::PrivateKey, now: chrono::DateTime<chrono::Utc>, allow: bool) -> Result<Value> {
    const CMD: &str = "dcp.loadKdm";
    let b = cfa4_dcp::kdb::read_bundle(tar).map_err(|e| bad(CMD, e.to_string()))?;
    let (mut loaded, mut refused, mut other_recipient) = (Vec::new(), Vec::new(), 0usize);
    let mut keys = Vec::new();
    for k in &b.kdms {
        match cfa4_dcp::kdm::open_kdm(&k.xml, key, now) {
            Err(cfa4_dcp::kdm::KdmError::WrongRecipient) => other_recipient += 1,
            Err(e) => refused.push(json!({"path": k.path, "problem": e.to_string()})),
            Ok(kdm) => match refusal(&kdm, allow) {
                Some(why) => refused.push(json!({"path": k.path, "problem": why})),
                None => {
                    let got = add_keys(&kdm);
                    keys.extend(got.iter().cloned());
                    let mut d = describe(&kdm, got);
                    d["path"] = json!(k.path);
                    loaded.push(d);
                }
            },
        }
    }
    if loaded.is_empty() {
        let why: Vec<String> =
            refused.iter().map(|r| format!("{}: {}", r["path"].as_str().unwrap_or_default(), r["problem"].as_str().unwrap_or_default())).collect();
        let mut msg = format!("no KDM of the bundle was loaded ({} for other recipients", other_recipient);
        if !why.is_empty() {
            msg.push_str(&format!("; {}", why.join("; ")));
        }
        msg.push(')');
        return Err(bad(CMD, msg));
    }
    s.media.clear();
    // one KDM's signature stands for the reply when there is one, as for a single KDM
    let signature = if loaded.len() == 1 { loaded.first().map(|d| d["signature"].clone()).unwrap_or(Value::Null) } else { Value::Null };
    Ok(json!({
        "title": b.annotation,
        "bundle": {"id": b.id, "creator": b.creator, "kdms": b.kdms.len(), "issues": b.issues},
        "kdms": loaded,
        "keys": keys,
        "otherRecipients": other_recipient,
        "refused": refused,
        "loaded": cfa4_mxf::crypt::key_ids().len(),
        "signature": signature,
    }))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "dcp.loadKdm",
            label: "Load KDM",
            menu: &[],
            shortcut: None,
            params: r#"{"kdm":str,"privateKey":str,"allowUnverified":bool?}"#,
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
