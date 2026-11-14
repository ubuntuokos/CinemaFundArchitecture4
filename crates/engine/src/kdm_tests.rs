//! #933: an encrypted D-Cinema sound track file imports, stays silent-with-an-error until its KDM
//! and private key are loaded, then plays; the keys never reach the project or the journal. The
//! KDM is built with the openssl command line and signed with xmlsec1 (oracles; skipped when
//! missing). Unsigned and changed KDMs are always refused; one whose certificate chain does not
//! keep ST 430-2 loads only with `allowUnverified` (owner decision 2026-11-13, #1028).

use std::path::Path;
use std::process::Command;

use base64::Engine as _;
use cfa4_mxf::{MxfWriter, PackageIds, Pattern, Rational, SoundDesc, Umid, WriterConfig};
use cfa4_project::ItemId;
use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

const KEY_ID: [u8; 16] = [0x5A; 16];
const CPL: [u8; 16] = [0x11; 16];

/// One second of 5.1 24-bit PCM in an encrypted D-Cinema sound track file.
fn write_encrypted(path: &Path, key: &[u8; 16]) {
    write_encrypted_with(path, key, &KEY_ID);
}

fn write_encrypted_with(path: &Path, key: &[u8; 16], key_id: &[u8; 16]) {
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
    std::fs::write(path, cfa4_mxf::testing::encrypt_track_file(&plain, key, key_id, &[0xC0; 16], 0, None)).unwrap();
}

fn openssl(dir: &Path, args: &[&str]) -> bool {
    Command::new("openssl").current_dir(dir).args(args).output().is_ok_and(|o| o.status.success())
}

/// A recipient key (`key.pem`) and a KDM (`kdm.xml`) with the content key for it.
fn write_kdm(dir: &Path, key: &[u8; 16]) -> bool {
    if !openssl(dir, &["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", "key.pem"]) {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "openssl not found");
        eprintln!("SKIPPED: openssl not found");
        return false;
    }
    // signed by an ST 430-2 chain, and by a plain X.509 one; the key blocks name the first
    let (Some(good), Some(plain)) = (cfa4_dcp::testing::Signed::new("engine-kdm"), cfa4_dcp::testing::Signed::non_conforming("engine-kdm-plain")) else {
        return false;
    };
    let mut p = cfa4_dcp::kdm::STRUCTURE_ID.to_vec();
    p.extend_from_slice(&good.signer_thumbprint());
    p.extend_from_slice(&CPL);
    p.extend_from_slice(b"MDAK");
    p.extend_from_slice(&KEY_ID);
    p.extend_from_slice(b"2020-01-01T00:00:00+00:00");
    p.extend_from_slice(b"2099-12-31T23:59:59+00:00");
    p.extend_from_slice(key);
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
<DCinemaSecurityMessage xmlns="http://www.smpte-ra.org/schemas/430-3/2006/ETM" xmlns:dsig="http://www.w3.org/2000/09/xmldsig#">
<AuthenticatedPublic Id="ID_AuthenticatedPublic"><MessageId>urn:uuid:22222222-2222-2222-2222-222222222222</MessageId><AnnotationText>Reel 1 KDM</AnnotationText>
<RequiredExtensions><KDMRequiredExtensions xmlns="http://www.smpte-ra.org/schemas/430-1/2006/KDM"><CompositionPlaylistId>urn:uuid:11111111-1111-1111-1111-111111111111</CompositionPlaylistId><ContentTitleText>Encrypted Test</ContentTitleText></KDMRequiredExtensions></RequiredExtensions></AuthenticatedPublic>
<AuthenticatedPrivate Id="ID_AuthenticatedPrivate"><enc:EncryptedKey xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:CipherData><enc:CipherValue>{b64}</enc:CipherValue></enc:CipherData></enc:EncryptedKey></AuthenticatedPrivate>
</DCinemaSecurityMessage>"#
    );
    std::fs::write(dir.join("kdm.xml"), &xml).unwrap();
    let template = xml.replace("</DCinemaSecurityMessage>", &format!("{}\n</DCinemaSecurityMessage>", cfa4_dcp::testing::TEMPLATE_SIGNATURE));
    let signed = good.sign(&template);
    std::fs::write(dir.join("signed.xml"), &signed).unwrap();
    std::fs::write(dir.join("changed.xml"), signed.replace("Reel 1 KDM", "Reel 2 KDM")).unwrap();
    std::fs::write(dir.join("plain.xml"), plain.sign(&template)).unwrap();
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
    // made at run time: no fixed key in the source
    let key = cfa4_mxf::testing::test_key("content");
    if !write_kdm(&dir, &key) {
        return;
    }
    let mxf = dir.join("enc_audio.mxf");
    write_encrypted(&mxf, &key);
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
    let load = |s: &mut Session, kdm: &str, allow: bool| {
        s.execute("dcp.loadKdm", json!({"kdm": s_path(&dir.join(kdm)), "privateKey": s_path(&dir.join("key.pem")), "allowUnverified": allow}))
    };
    let none_loaded = |s: &mut Session| s.execute("dcp.keys", json!({})).unwrap()["keyIds"].as_array().unwrap().is_empty();
    // the right key, but unsigned, or changed after signing: refused, even when asked to accept
    for (kdm, why) in [("kdm.xml", "not signed"), ("changed.xml", "ID_AuthenticatedPublic")] {
        for allow in [false, true] {
            let e = load(&mut s, kdm, allow).unwrap_err().to_string();
            assert!(e.contains(why) && e.contains("not loaded") && !e.contains("allowUnverified"), "{kdm} {allow}: {e}");
            assert!(none_loaded(&mut s), "{kdm}: nothing loaded");
        }
    }
    // a chain that does not keep ST 430-2: refused unless the user accepts it
    let e = load(&mut s, "plain.xml", false).unwrap_err().to_string();
    assert!(e.contains("certificate chain") && e.contains("ST 430-2") && e.contains("allowUnverified"), "{e}");
    assert!(none_loaded(&mut s));
    let r = load(&mut s, "plain.xml", true).unwrap();
    assert_eq!(r["signature"]["valid"], false);
    assert_eq!(r["signature"]["overridable"], true);
    assert_eq!(r["signature"]["chain"][0], "CS.signer");
    s.execute("dcp.clearKeys", json!({})).unwrap();
    // a conforming chain loads as it is
    let r = load(&mut s, "signed.xml", false).unwrap();
    assert_eq!(r["signature"]["valid"], true);
    assert_eq!(r["signature"]["chain"], json!(["CS.signer", ".inter", ".root"]));
    assert_eq!(r["title"], "Encrypted Test");
    assert_eq!(r["keys"][0]["type"], "MDAK");
    assert_eq!(r["keys"][0]["keyId"], "5a5a5a5a-5a5a-5a5a-5a5a-5a5a5a5a5a5a");
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(key);
    assert!(!r.to_string().contains(&hex) && !r.to_string().contains(&b64), "the key itself is never returned");
    // frame 10, channel 1: (10 * 3 + 100 000) mod 8 000 000
    let want = 100_030.0 / 8_388_608.0;
    assert_eq!(first_sample(&s, id).unwrap(), want);
    assert_eq!(s.execute("dcp.keys", json!({})).unwrap()["keyIds"][0], "5a5a5a5a-5a5a-5a5a-5a5a-5a5a5a5a5a5a");
    // nothing of the key in the saved project
    let saved = serde_json::to_string(&*s.project).unwrap();
    assert!(!saved.contains(&hex) && !saved.contains(&b64));
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

#[test]
fn an_encrypted_clip_without_its_key_says_which_key_it_needs() {
    // #1053: a Key ID no other test loads (the key ring is shared by the whole process)
    let key_id = [0x6B; 16];
    let dir = tmp_dir("kdm-needs-key");
    let mxf = dir.join("enc_needs_key.mxf");
    write_encrypted_with(&mxf, &cfa4_mxf::testing::test_key("needs-key"), &key_id);
    let mut s = Session::default();
    let r = s.execute("file.import", json!({"paths": [mxf.to_string_lossy()]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap());
    s.execute("source.open", json!({"item": id.0})).unwrap();
    let inspect = s.execute("source.inspect", json!({})).unwrap();
    assert_eq!(inspect["needsKey"], json!(["6b6b6b6b-6b6b-6b6b-6b6b-6b6b6b6b6b6b"]));
    assert_eq!(s.media.missing_keys(id), vec!["6b6b6b6b-6b6b-6b6b-6b6b-6b6b6b6b6b6b".to_string()]);
    // a clip that is not encrypted needs no key
    let plain = dir.join("plain.wav");
    let data = vec![0u8; 4_800 * 4];
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0002_0001, 48_000, 192_000, 0x0010_0004] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(&plain, wav).unwrap();
    let r = s.execute("file.import", json!({"paths": [plain.to_string_lossy()]})).unwrap();
    let wav = ItemId(r["items"][0].as_u64().unwrap());
    s.execute("source.open", json!({"item": wav.0})).unwrap();
    assert_eq!(s.execute("source.inspect", json!({})).unwrap()["needsKey"], json!([]));
    assert!(s.media.missing_keys(wav).is_empty());
}
