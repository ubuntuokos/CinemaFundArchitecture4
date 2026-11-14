//! XML Signature tests: KDM-shaped documents signed by xmlsec1 with a three-certificate chain made
//! by the openssl command line (`crate::testing`; both oracles only, skipped when missing),
//! checked as signed, after tampering and with chains that do not keep ST 430-2.

use super::*;
use crate::testing::Signed;

const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DCinemaSecurityMessage xmlns="http://www.smpte-ra.org/schemas/430-3/2006/ETM" xmlns:dsig="http://www.w3.org/2000/09/xmldsig#" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <AuthenticatedPublic Id="ID_AuthenticatedPublic">
    <MessageId>urn:uuid:11111111-2222-3333-4444-555555555555</MessageId>
    <!-- a comment the references leave out -->
    <AnnotationText xml:lang="hu">Próba &amp; teszt</AnnotationText>
    <Signer><dsig:X509IssuerName>CN=.inter,O=test</dsig:X509IssuerName><dsig:X509SerialNumber>12345678901234567890</dsig:X509SerialNumber></Signer>
  </AuthenticatedPublic>
  <AuthenticatedPrivate Id="ID_AuthenticatedPrivate"><enc:EncryptedKey xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:CipherData><enc:CipherValue>AAEC
AwQ=</enc:CipherValue></enc:CipherData></enc:EncryptedKey></AuthenticatedPrivate>
  SIGNATURE
</DCinemaSecurityMessage>"#;

fn check(xml: &str, serial: Option<&str>, now: DateTime<Utc>) -> Result<Verified, String> {
    let doc = Document::parse(xml).map_err(|e| e.to_string())?;
    let sig = doc.root_element().children().find(|n| n.tag_name().name() == "Signature").ok_or("no Signature")?;
    verify(&doc, sig, serial, now)
}

#[test]
fn xmlsec1_signed_documents_verify_and_tampering_fails() {
    let Some(s) = Signed::new("verify") else { return };
    let signed = s.sign(&DOC.replace("SIGNATURE", crate::testing::TEMPLATE_SIGNATURE));
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let now = DateTime::from_timestamp(secs, 0).unwrap();
    let v = check(&signed, Some("12345678901234567890"), now).unwrap();
    assert_eq!(v.references, ["ID_AuthenticatedPublic", "ID_AuthenticatedPrivate"]);
    assert_eq!(v.chain_problem, None);
    // every certificate keeps ST 430-2 6.2
    for c in &v.chain {
        assert_eq!(c.st430_2_problem(), None, "{}", name(c));
        assert_eq!(c.attribute(false, crate::x509::OID_DN_QUALIFIER), Some(c.public_key_thumbprint().as_str()));
    }
    assert_eq!(v.chain[0].rsa_size().unwrap(), (2048, Some(65537)));
    assert_eq!(v.chain.iter().map(name).collect::<Vec<_>>(), ["CS.signer", ".inter", ".root"]);
    assert_eq!(v.chain[0].serial_decimal(), "12345678901234567890");
    assert!(v.chain[1].ca && v.chain[2].ca && !v.chain[0].ca);
    // without the Signer's serial the leaf is the certificate that signs no other
    assert_eq!(name(&check(&signed, None, now).unwrap().chain[0]), "CS.signer");
    // tampering anywhere the signature covers
    let e = check(&signed.replace("Próba", "Proba"), None, now).unwrap_err();
    assert!(e.contains("ID_AuthenticatedPublic"), "{e}");
    let e = check(&signed.replace("AAEC\n", "AAED\n"), None, now).unwrap_err();
    assert!(e.contains("ID_AuthenticatedPrivate"), "{e}");
    // a comment inside a reference is not signed content; one in SignedInfo is
    assert!(check(&signed.replace("a comment the references", "another comment the references"), None, now).is_ok());
    let e = check(&signed.replace("<dsig:SignedInfo>", "<dsig:SignedInfo><!-- x -->"), None, now).unwrap_err();
    assert!(e.contains("signature value"), "{e}");
    // a different SignatureValue, an unknown signer, a time outside the validity
    let sv = signed.find("<dsig:SignatureValue>").unwrap() + "<dsig:SignatureValue>".len();
    let mut bad = signed.clone();
    let c = if &bad[sv..sv + 1] == "A" { "B" } else { "A" };
    bad.replace_range(sv..sv + 1, c);
    assert!(check(&bad, None, now).is_err());
    assert!(check(&signed, Some("99"), now).unwrap_err().contains("not in KeyInfo"));
    // a time outside the validity: the content is intact, the chain is not
    let early = check(&signed, None, now - chrono::Duration::days(2)).unwrap();
    assert!(early.chain_problem.as_deref().is_some_and(|e| e.contains("not valid at")), "{:?}", early.chain_problem);
    // a chain without its root
    let first = signed.find("<dsig:X509Certificate>").unwrap();
    let mut certs: Vec<(usize, usize)> = Vec::new();
    let mut at = first;
    while let Some(i) = signed[at..].find("<dsig:X509Certificate>") {
        let a = at + i;
        let b = a + signed[a..].find("</dsig:X509Certificate>").unwrap() + "</dsig:X509Certificate>".len();
        certs.push((a, b));
        at = b;
    }
    assert_eq!(certs.len(), 3);
    let root_cert = certs.iter().find(|(a, b)| {
        let text = format!("<x xmlns:dsig=\"{DSIG_NS}\">{}</x>", &signed[*a..*b]);
        let doc = Document::parse(&text).unwrap();
        let der = b64(doc.root_element().first_element_child(), "c").unwrap();
        name(&Certificate::from_der(&der).unwrap()) == ".root"
    });
    let (a, b) = *root_cert.unwrap();
    let mut no_root = signed.clone();
    no_root.replace_range(a..b, "");
    let nr = check(&no_root, None, now).unwrap();
    assert!(nr.chain_problem.as_deref().is_some_and(|e| e.contains("issuer")), "missing root: {:?}", nr.chain_problem);
    // damaged documents never panic
    for cut in (0..signed.len()).step_by(97) {
        if let Some(prefix) = signed.get(..cut) {
            let _ = check(prefix, None, now);
        }
    }
}

/// Certificates that do not parse, and hostile DER, are errors.
#[test]
fn hostile_certificates() {
    assert!(Certificate::from_der(&[]).is_err());
    assert!(Certificate::from_der(&[0x30, 0x84, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    assert!(Certificate::from_der(&[0x30, 0x80]).is_err());
    assert!(Certificate::from_der(&[0x30; 100]).is_err());
}

/// A chain of plain X.509 certificates (no dnQualifier, path lengths or AuthorityKeyIdentifier):
/// the content verifies, the chain is reported as not keeping ST 430-2.
#[test]
fn non_conforming_chains_are_a_chain_problem() {
    let Some(s) = Signed::non_conforming("plain") else { return };
    let signed = s.sign(&DOC.replace("SIGNATURE", crate::testing::TEMPLATE_SIGNATURE));
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let now = DateTime::from_timestamp(secs, 0).unwrap();
    let v = check(&signed, None, now).unwrap();
    let p = v.chain_problem.unwrap();
    assert!(p.contains("CS.signer") && p.contains("ST 430-2"), "{p}");
    // tampering is still an error, not a chain problem
    assert!(check(&signed.replace("Próba", "Proba"), None, now).is_err());
}
