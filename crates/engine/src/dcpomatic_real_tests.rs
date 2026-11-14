//! Plan CFA4-DCPOMATIC-001, phase 3 acceptance, with the pinned DCP-o-matic itself: an encrypted
//! DCP (`dcpomatic.makeDcp` with `encrypt`), a KDM for a test certificate (`dcpomatic.makeKdm`),
//! and CFA4 playing the encrypted sound track after `dcp.loadKdm` with the test private key.
//!
//! Runs only when `CFA4_DCPOMATIC_BIN_DIR` names a DCP-o-matic build (the CI job
//! `dcpomatic.yml`) and `ffmpeg` and `openssl` are there; otherwise it says so and passes.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_project::ItemId;
use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

fn run(dir: &Path, program: &str, args: &[&str]) -> bool {
    Command::new(program).current_dir(dir).args(args).output().is_ok_and(|o| o.status.success())
}

/// One second of 2K Flat ProRes with a 1 kHz tone, as `file.exportMedia` would hand over.
fn master(dir: &Path) -> Option<PathBuf> {
    let out = dir.join("master.mov");
    let args = [
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1998x1080:rate=24:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:sample_rate=48000:duration=1",
        "-c:v",
        "prores_ks",
        "-c:a",
        "pcm_s24le",
        "master.mov",
    ];
    run(dir, "ffmpeg", &args).then_some(out)
}

/// A test recipient: an RSA key and a self-signed certificate standing in for a projector.
fn recipient(dir: &Path) -> bool {
    run(
        dir,
        "openssl",
        &[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "projector.key",
            "-out",
            "projector.pem",
            "-days",
            "30",
            "-subj",
            "/O=CFA4 test/OU=test/CN=CP.CFA4-TEST-PROJECTOR",
        ],
    )
}

/// The peak of the first 0.1 s over all channels of the sound track file (DCP-o-matic maps the
/// mono tone to the centre channel), or the error reading it.
fn sample(s: &Session, id: ItemId) -> Result<f32, String> {
    let src = s.media.source_for(&s.project, id, &*s.services).ok_or("no source")?;
    let a = src.audio(0, 4_800, 48_000).map_err(|e| e.to_string())?;
    Ok(a.channels.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs())))
}

#[test]
fn encrypted_dcp_and_kdm_from_dcpomatic_play_in_cfa4() {
    if std::env::var_os(cfa4_dcpomatic::ENV_BIN_DIR).is_none_or(|v| v.is_empty()) {
        eprintln!("skipped: {} is not set (no DCP-o-matic build)", cfa4_dcpomatic::ENV_BIN_DIR);
        return;
    }
    let dir = tmp_dir("dcpom3-real");
    let Some(master) = master(&dir) else {
        eprintln!("skipped: ffmpeg could not make the test master");
        return;
    };
    if !recipient(&dir) {
        eprintln!("skipped: openssl could not make the test certificate");
        return;
    }
    let p = |x: &Path| x.display().to_string();
    let mut s = Session::default();
    s.execute("dcp.clearKeys", json!({})).unwrap();

    let film = dir.join("film");
    let r = s
        .execute(
            "dcpomatic.makeDcp",
            json!({"content": [p(&master)], "output": p(&film), "name": "CFA4 DCPOM3", "resolution": "2k", "container": "flat",
                   "fps": 24, "encrypt": true, "configDir": p(&dir.join("config")), "wait": true}),
        )
        .unwrap();
    assert_eq!(r["encrypted"], true, "{r}");
    assert!(r["keepFilmDir"].is_string(), "the answer says to keep the film folder: {r}");
    let dcp = PathBuf::from(r["dcp"].as_str().unwrap());

    let r = s
        .execute(
            "dcpomatic.makeKdm",
            json!({"film": p(&film), "certificate": p(&dir.join("projector.pem")), "output": p(&dir.join("kdms")),
                   "cinema": "CFA4", "screen": "1", "validFor": {"count": 1, "unit": "days"}, "configDir": p(&dir.join("config"))}),
        )
        .unwrap();
    let kdm = r["kdms"][0].as_str().unwrap().to_string();

    let sound = std::fs::read_dir(&dcp)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|f| f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("pcm_")) && f.extension().is_some_and(|x| x == "mxf"));
    let sound = sound.expect("a sound track file in the DCP");
    let r = s.execute("file.import", json!({"paths": [p(&sound)]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap());
    let before = sample(&s, id);
    assert!(before.as_ref().is_ok_and(|v| *v == 0.0) || before.is_err(), "no key: an error or silence, never noise: {before:?}");

    let r = s.execute("dcp.loadKdm", json!({"kdm": kdm, "privateKey": p(&dir.join("projector.key"))})).unwrap();
    assert!(r["keys"].as_array().is_some_and(|k| k.len() >= 2), "picture and sound keys: {r}");
    let peak = sample(&s, id).unwrap();
    assert!(peak > 0.05, "the 1 kHz tone plays after the KDM: peak {peak}");

    s.execute("dcp.clearKeys", json!({})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
