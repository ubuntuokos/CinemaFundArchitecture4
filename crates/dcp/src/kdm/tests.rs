//! KDM tests. The KDMs are built with the openssl command line (RSA key generation and RSA-OAEP
//! SHA-1 encryption: an oracle independent of the `rsa` crate), skipped when it is missing.

use super::*;
use std::process::Command;

const CPL: [u8; 16] = [0x11; 16];
const KEY_ID: [u8; 16] = [0x5A; 16];
const CONTENT_KEY: [u8; 16] = *b"0123456789abcdef";

fn plaintext(structure: [u8; 16], cpl: [u8; 16], kind: &str, key_id: [u8; 16], from: &str, to: &str) -> Vec<u8> {
    let mut p = structure.to_vec();
    p.extend_from_slice(&[0x77; 20]);
    p.extend_from_slice(&cpl);
    p.extend_from_slice(kind.as_bytes());
    p.extend_from_slice(&key_id);
    p.extend_from_slice(from.as_bytes());
    p.extend_from_slice(to.as_bytes());
    p.extend_from_slice(&CONTENT_KEY);
    p
}

const FROM: &str = "2026-01-01T00:00:00+00:00";
const TO: &str = "2030-12-31T23:59:59+00:00";

#[test]
fn cipher_data_fields() {
    let k = cipher_data(&plaintext(STRUCTURE_ID, CPL, "MDIK", KEY_ID, FROM, TO)).unwrap();
    assert_eq!((k.key_id.0, k.cpl_id.0, k.key_type.as_str(), k.key), (KEY_ID, CPL, "MDIK", CONTENT_KEY));
    assert_eq!((k.not_before.as_str(), k.not_after.as_str()), (FROM, TO));
    assert!(!format!("{k:?}").contains("0123456789abcdef"), "Debug never shows the key");
    assert!(cipher_data(&plaintext([0; 16], CPL, "MDIK", KEY_ID, FROM, TO)).is_err(), "wrong Structure ID");
    let p = plaintext(STRUCTURE_ID, CPL, "MDIK", KEY_ID, FROM, TO);
    for n in 0..p.len() {
        assert!(cipher_data(&p[..n]).is_err());
    }
}

struct Ossl {
    dir: std::path::PathBuf,
}

impl Ossl {
    fn new(name: &str) -> Option<Ossl> {
        let ok = Command::new("openssl").arg("version").output().is_ok_and(|o| o.status.success());
        if !ok {
            assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "openssl not found");
            eprintln!("SKIPPED: openssl not found");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("cfa4-kdm-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Some(Ossl { dir })
    }
    fn run(&self, args: &[&str]) {
        let out = Command::new("openssl").current_dir(&self.dir).args(args).output().unwrap();
        assert!(out.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    /// A new 2048-bit RSA key in PKCS#8 PEM (`key.pem`) and PKCS#1 PEM (`key1.pem`).
    fn key(&self) -> (String, String) {
        self.run(&["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", "key.pem"]);
        self.run(&["rsa", "-in", "key.pem", "-traditional", "-out", "key1.pem"]);
        let read = |f: &str| std::fs::read_to_string(self.dir.join(f)).unwrap();
        (read("key.pem"), read("key1.pem"))
    }
    /// RSA-OAEP (SHA-1, MGF1 SHA-1) encryption of `p` for `key.pem`, base64.
    fn wrap(&self, p: &[u8]) -> String {
        std::fs::write(self.dir.join("p.bin"), p).unwrap();
        self.run(&[
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
        ]);
        base64::engine::general_purpose::STANDARD.encode(std::fs::read(self.dir.join("c.bin")).unwrap())
    }
}

impl Drop for Ossl {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A KDM in the ST 430-1 layout carrying `blocks` (base64 RSA blocks).
fn kdm_xml(cpl: &str, blocks: &[String]) -> String {
    let keys: String = blocks
        .iter()
        .map(|b| {
            format!(
                "<enc:EncryptedKey xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\"><enc:EncryptionMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#rsa-oaep-mgf1p\"><ds:DigestMethod xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\" Algorithm=\"http://www.w3.org/2000/09/xmldsig#sha1\"/></enc:EncryptionMethod><enc:CipherData><enc:CipherValue>{}\n{}</enc:CipherValue></enc:CipherData></enc:EncryptedKey>",
                &b[..64],
                &b[64..]
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<DCinemaSecurityMessage xmlns="http://www.smpte-ra.org/schemas/430-3/2006/ETM" xmlns:dsig="http://www.w3.org/2000/09/xmldsig#">
  <AuthenticatedPublic Id="ID_AuthenticatedPublic">
    <MessageId>urn:uuid:22222222-2222-2222-2222-222222222222</MessageId>
    <MessageType>http://www.smpte-ra.org/430-1/2006/KDM#kdm-key-type</MessageType>
    <AnnotationText>Test KDM</AnnotationText>
    <IssueDate>2026-11-12T00:00:00+00:00</IssueDate>
    <RequiredExtensions>
      <KDMRequiredExtensions xmlns="http://www.smpte-ra.org/schemas/430-1/2006/KDM">
        <CompositionPlaylistId>urn:uuid:{cpl}</CompositionPlaylistId>
        <ContentTitleText>Test Feature</ContentTitleText>
        <ContentKeysNotValidBefore>{FROM}</ContentKeysNotValidBefore>
        <ContentKeysNotValidAfter>{TO}</ContentKeysNotValidAfter>
      </KDMRequiredExtensions>
    </RequiredExtensions>
  </AuthenticatedPublic>
  <AuthenticatedPrivate Id="ID_AuthenticatedPrivate">{keys}</AuthenticatedPrivate>
</DCinemaSecurityMessage>"#
    )
}

fn now() -> DateTime<Utc> {
    time("2026-11-12T08:00:00+00:00").unwrap()
}

#[test]
fn openssl_made_kdm_opens() {
    let Some(o) = Ossl::new("open") else { return };
    let (pkcs8, pkcs1) = o.key();
    let blocks = [o.wrap(&plaintext(STRUCTURE_ID, CPL, "MDIK", KEY_ID, FROM, TO)), o.wrap(&plaintext(STRUCTURE_ID, CPL, "MDAK", [0x6B; 16], FROM, TO))];
    let xml = kdm_xml("11111111-1111-1111-1111-111111111111", &blocks);
    for pem in [&pkcs8, &pkcs1] {
        let key = private_key_from_pem(pem).unwrap();
        let k = open_kdm(&xml, &key, now()).unwrap();
        assert_eq!(k.content_title.as_deref(), Some("Test Feature"));
        assert_eq!(k.cpl_id, Some(Uuid(CPL)));
        let got: Vec<(&str, [u8; 16], [u8; 16])> = k.keys.iter().map(|k| (k.key_type.as_str(), k.key_id.0, k.key)).collect();
        assert_eq!(got, [("MDIK", KEY_ID, CONTENT_KEY), ("MDAK", [0x6B; 16], CONTENT_KEY)]);
    }
    // outside the validity window
    let key = private_key_from_pem(&pkcs8).unwrap();
    assert!(matches!(open_kdm(&xml, &key, time("2025-01-01T00:00:00Z").unwrap()), Err(KdmError::NotYetValid(_))));
    assert!(matches!(open_kdm(&xml, &key, time("2031-01-01T00:00:00Z").unwrap()), Err(KdmError::Expired(_))));
    // the public part naming another composition
    let other = kdm_xml("33333333-3333-3333-3333-333333333333", &blocks);
    assert!(matches!(open_kdm(&other, &key, now()), Err(KdmError::Invalid(_))));
    // another recipient's key
    let (stranger, _) = o.key();
    let stranger = private_key_from_pem(&stranger).unwrap();
    assert!(matches!(open_kdm(&xml, &stranger, now()), Err(KdmError::WrongRecipient)));
}

#[test]
fn bad_inputs_are_errors() {
    assert!(matches!(private_key_from_pem("not a key"), Err(KdmError::PrivateKey(_))));
    assert!(matches!(private_key_from_pem("-----BEGIN ENCRYPTED PRIVATE KEY-----\nAA==\n-----END ENCRYPTED PRIVATE KEY-----"), Err(KdmError::PrivateKey(_))));
    let Some(o) = Ossl::new("bad") else { return };
    let (pem, _) = o.key();
    let key = private_key_from_pem(&pem).unwrap();
    assert!(matches!(open_kdm("<CompositionPlaylist/>", &key, now()), Err(KdmError::NotKdm(_))));
    assert!(open_kdm("<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"b\">]><x/>", &key, now()).is_err(), "DTDs are refused");
    let block = o.wrap(&plaintext([0; 16], CPL, "MDIK", KEY_ID, FROM, TO));
    assert!(matches!(open_kdm(&kdm_xml("11111111-1111-1111-1111-111111111111", &[block]), &key, now()), Err(KdmError::Invalid(_))));
    let xml = kdm_xml("11111111-1111-1111-1111-111111111111", &["A".repeat(344)]);
    assert!(open_kdm(&xml, &key, now()).is_err());
}

/// The KDM's signature: an xmlsec1-signed KDM checks out with its chain; unsigned and changed
/// ones say so, and still open (the caller decides).
#[test]
fn kdm_signatures_are_checked() {
    let Some(o) = Ossl::new("sig") else { return };
    let Some(signer) = crate::xmldsig::tests::Signed::new("kdm") else { return };
    let (pem, _) = o.key();
    let key = private_key_from_pem(&pem).unwrap();
    let blocks = [o.wrap(&plaintext(STRUCTURE_ID, CPL, "MDIK", KEY_ID, FROM, TO))];
    let unsigned = kdm_xml("11111111-1111-1111-1111-111111111111", &blocks);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let now = DateTime::from_timestamp(secs, 0).unwrap();
    assert_eq!(open_kdm(&unsigned, &key, now).unwrap().signature, Signature::Missing);
    let template = unsigned.replace("</DCinemaSecurityMessage>", &format!("{}\n</DCinemaSecurityMessage>", crate::xmldsig::tests::TEMPLATE_SIGNATURE));
    let signed = signer.sign(&template);
    let k = open_kdm(&signed, &key, now).unwrap();
    assert_eq!(k.signature, Signature::Valid { chain: vec!["CS.signer".into(), ".inter".into(), ".root".into()] });
    assert!(k.signature.is_valid());
    let changed = open_kdm(&signed.replace("Test Feature", "Other Feature"), &key, now).unwrap();
    assert!(matches!(&changed.signature, Signature::Invalid(e) if e.contains("ID_AuthenticatedPublic")), "{:?}", changed.signature);
    assert_eq!(changed.content_title.as_deref(), Some("Other Feature"));
    // a signature that leaves the private part out
    let partial = signer.sign(&unsigned.replace(
        "</DCinemaSecurityMessage>",
        &format!("{}\n</DCinemaSecurityMessage>", crate::xmldsig::tests::TEMPLATE_SIGNATURE.replace("<dsig:Reference URI=\"#ID_AuthenticatedPrivate\"><dsig:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/><dsig:DigestValue/></dsig:Reference>", "")),
    ));
    let p = open_kdm(&partial, &key, now).unwrap();
    assert!(matches!(&p.signature, Signature::Invalid(e) if e.contains("AuthenticatedPrivate")), "{:?}", p.signature);
}
