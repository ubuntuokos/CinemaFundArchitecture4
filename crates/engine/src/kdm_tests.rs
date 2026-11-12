//! #933: an encrypted D-Cinema sound track file imports, stays silent-with-an-error until its KDM
//! and private key are loaded, then plays; the keys never reach the project or the journal. The
//! KDM is built with the openssl command line (oracle; skipped when missing).

use std::path::Path;
use std::process::Command;

use base64::Engine as _;
use cfa4_mxf::{MxfWriter, PackageIds, Pattern, Rational, SoundDesc, Umid, WriterConfig};
use cfa4_project::ItemId;
use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

const KEY: [u8; 16] = *b"0123456789abcdef";
const KEY_ID: [u8; 16] = [0x5A; 16];
const CPL: [u8; 16] = [0x11; 16];

/// One second of 5.1 24-bit PCM in an encrypted D-Cinema sound track file.
fn write_encrypted(path: &Path) {
    let mut ids = PackageIds::from_seed("enc-snd", "Reel 1");
    ids.file = Umid::dcinema([0x22; 16]);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 6, bits: 24 }];
    let mut w = MxfWriter::new(std::io::Cursor::new(Vec::new()), cfg).unwrap();
    let mut pcm = Vec::new();
    for k in 0..48_000i32 {
        for c in 0..6i32 {
            pcm.extend_from_slice(&((k * 3 + c * 100_000) % 8_000_000).to_le_bytes()[..3]);
        }
    }
    w.push_sound(0, &pcm).unwrap();
    let plain = w.finish().unwrap().into_inner();
    std::fs::write(path, cfa4_mxf::testing::encrypt_track_file(&plain, &KEY, &KEY_ID, &[0xC0; 16], 0, None)).unwrap();
}

fn openssl(dir: &Path, args: &[&str]) -> bool {
    Command::new("openssl").current_dir(dir).args(args).output().is_ok_and(|o| o.status.success())
}

/// A recipient key (`key.pem`) and a KDM (`kdm.xml`) with the content key for it.
fn write_kdm(dir: &Path) -> bool {
    if !openssl(dir, &["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", "key.pem"]) {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "openssl not found");
        eprintln!("SKIPPED: openssl not found");
        return false;
    }
    let mut p = cfa4_dcp::kdm::STRUCTURE_ID.to_vec();
    p.extend_from_slice(&[0x77; 20]);
    p.extend_from_slice(&CPL);
    p.extend_from_slice(b"MDAK");
    p.extend_from_slice(&KEY_ID);
    p.extend_from_slice(b"2020-01-01T00:00:00+00:00");
    p.extend_from_slice(b"2099-12-31T23:59:59+00:00");
    p.extend_from_slice(&KEY);
    std::fs::write(dir.join("p.bin"), &p).unwrap();
    let wrap = [
        "pkeyutl",
        "-encrypt",
        "-inkey",
        "key.pem",
        "-pkeyopt",
        "rsa_padding_mode:oaep",
        "-pkeyopt",
        "rsa_oaep_md:sha1",
        "-pkeyopt",
        "rsa_mgf1_md:sha1",
        "-in",
        "p.bin",
        "-out",
        "c.bin",
    ];
    assert!(openssl(dir, &wrap));
    let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(dir.join("c.bin")).unwrap());
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DCinemaSecurityMessage xmlns="http://www.smpte-ra.org/schemas/430-3/2006/ETM">
<AuthenticatedPublic Id="ID_AuthenticatedPublic"><MessageId>urn:uuid:22222222-2222-2222-2222-222222222222</MessageId><AnnotationText>Reel 1 KDM</AnnotationText>
<RequiredExtensions><KDMRequiredExtensions xmlns="http://www.smpte-ra.org/schemas/430-1/2006/KDM"><CompositionPlaylistId>urn:uuid:11111111-1111-1111-1111-111111111111</CompositionPlaylistId><ContentTitleText>Encrypted Test</ContentTitleText></KDMRequiredExtensions></RequiredExtensions></AuthenticatedPublic>
<AuthenticatedPrivate Id="ID_AuthenticatedPrivate"><enc:EncryptedKey xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:CipherData><enc:CipherValue>{b64}</enc:CipherValue></enc:CipherData></enc:EncryptedKey></AuthenticatedPrivate>
</DCinemaSecurityMessage>"#
    );
    std::fs::write(dir.join("kdm.xml"), xml).unwrap();
    true
}

fn first_sample(s: &Session, id: ItemId) -> Result<f32, String> {
    let src = s.media.source_for(&s.project, id, &*s.services).ok_or("no source")?;
    let a = src.audio(0, 100, 48_000).map_err(|e| e.to_string())?;
    Ok(a.channels.get(1).and_then(|c| c.get(10).copied()).unwrap_or(f32::NAN))
}

#[test]
fn encrypted_dcp_audio_plays_after_its_kdm() {
    let dir = tmp_dir("kdm-engine");
    if !write_kdm(&dir) {
        return;
    }
    let mxf = dir.join("enc_audio.mxf");
    write_encrypted(&mxf);
    let mut s = Session::default();
    let s_path = |p: &Path| p.to_string_lossy().to_string();
    s.execute("dcp.clearKeys", json!({})).unwrap();
    let r = s.execute("file.import", json!({"paths": [s_path(&mxf)]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap());
    let before = first_sample(&s, id);
    assert!(before.as_ref().is_ok_and(|v| *v == 0.0) || before.is_err(), "no key: an error or silence, never noise: {before:?}");
    // a wrong private key
    assert!(openssl(&dir, &["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", "other.pem"]));
    let e = s.execute("dcp.loadKdm", json!({"kdm": s_path(&dir.join("kdm.xml")), "privateKey": s_path(&dir.join("other.pem"))})).unwrap_err();
    assert!(e.to_string().contains("not made for this private key"), "{e}");
    // the right one
    let r = s.execute("dcp.loadKdm", json!({"kdm": s_path(&dir.join("kdm.xml")), "privateKey": s_path(&dir.join("key.pem"))})).unwrap();
    assert_eq!(r["title"], "Encrypted Test");
    assert_eq!(r["keys"][0]["type"], "MDAK");
    assert_eq!(r["keys"][0]["keyId"], "5a5a5a5a-5a5a-5a5a-5a5a-5a5a5a5a5a5a");
    assert!(!r.to_string().contains("3031323334"), "the key itself is never returned");
    // frame 10, channel 1: (10 * 3 + 100 000) mod 8 000 000
    let want = 100_030.0 / 8_388_608.0;
    assert_eq!(first_sample(&s, id).unwrap(), want);
    assert_eq!(s.execute("dcp.keys", json!({})).unwrap()["keyIds"][0], "5a5a5a5a-5a5a-5a5a-5a5a-5a5a5a5a5a5a");
    // nothing of the key in the saved project
    let saved = serde_json::to_string(&*s.project).unwrap();
    assert!(!saved.contains("0123456789abcdef"));
    s.execute("dcp.clearKeys", json!({})).unwrap();
    assert!(s.execute("dcp.keys", json!({})).unwrap()["keyIds"].as_array().unwrap().is_empty());
    let after = first_sample(&s, id);
    assert!(after.as_ref().is_ok_and(|v| *v == 0.0) || after.is_err(), "{after:?}");
}

#[test]
fn load_kdm_params_are_checked() {
    let mut s = Session::default();
    assert!(s.execute("dcp.loadKdm", json!({})).is_err());
    assert!(s.execute("dcp.loadKdm", json!({"kdm": "/nonexistent/kdm.xml", "privateKey": "/nonexistent/key.pem"})).is_err());
}
