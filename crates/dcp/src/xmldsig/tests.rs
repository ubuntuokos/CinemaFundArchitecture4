//! XML Signature tests: KDM-shaped documents signed by xmlsec1 with a three-certificate chain made
//! by the openssl command line (both oracles only, skipped when missing), checked as signed and
//! after tampering.

use super::*;
use std::process::Command;

pub(crate) struct Signed {
    pub dir: std::path::PathBuf,
}

fn tool(name: &str, arg: &str) -> bool {
    let ok = Command::new(name).arg(arg).output().is_ok_and(|o| o.status.success());
    if !ok {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "{name} not found");
        eprintln!("SKIPPED: {name} not found");
    }
    ok
}

fn run(dir: &std::path::Path, prog: &str, args: &[&str]) {
    let out = Command::new(prog).current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "{prog} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

const EXT: &str = "[v3ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n[v3leaf]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n";

/// The template: a KDM skeleton whose Signature xmlsec1 fills in.
pub(crate) const TEMPLATE_SIGNATURE: &str = r##"<dsig:Signature>
    <dsig:SignedInfo>
      <dsig:CanonicalizationMethod Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315#WithComments"/>
      <dsig:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/>
      <dsig:Reference URI="#ID_AuthenticatedPublic"><dsig:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><dsig:DigestValue/></dsig:Reference>
      <dsig:Reference URI="#ID_AuthenticatedPrivate"><dsig:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><dsig:DigestValue/></dsig:Reference>
    </dsig:SignedInfo>
    <dsig:SignatureValue/>
    <dsig:KeyInfo><dsig:X509Data><dsig:X509Certificate/></dsig:X509Data></dsig:KeyInfo>
  </dsig:Signature>"##;

impl Signed {
    /// A root, an intermediate and a signer certificate (serial 12345678901234567890) with their
    /// keys in a new directory, or `None` without openssl and xmlsec1.
    pub(crate) fn new(name: &str) -> Option<Signed> {
        if !tool("openssl", "version") || !tool("xmlsec1", "--version") {
            return None;
        }
        let dir = std::env::temp_dir().join(format!("cfa4-xmldsig-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ext.cnf"), EXT).unwrap();
        for k in ["root", "inter", "leaf"] {
            run(&dir, "openssl", &["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", &format!("{k}.key")]);
        }
        run(
            &dir,
            "openssl",
            &[
                "req",
                "-x509",
                "-new",
                "-key",
                "root.key",
                "-sha256",
                "-days",
                "3650",
                "-subj",
                "/O=test/CN=.root",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
                "-out",
                "root.pem",
            ],
        );
        run(&dir, "openssl", &["req", "-new", "-key", "inter.key", "-subj", "/O=test/CN=.inter", "-out", "inter.csr"]);
        run(
            &dir,
            "openssl",
            &[
                "x509",
                "-req",
                "-in",
                "inter.csr",
                "-CA",
                "root.pem",
                "-CAkey",
                "root.key",
                "-set_serial",
                "2",
                "-sha256",
                "-days",
                "3000",
                "-extfile",
                "ext.cnf",
                "-extensions",
                "v3ca",
                "-out",
                "inter.pem",
            ],
        );
        run(&dir, "openssl", &["req", "-new", "-key", "leaf.key", "-subj", "/O=test/CN=CS.signer", "-out", "leaf.csr"]);
        run(
            &dir,
            "openssl",
            &[
                "x509",
                "-req",
                "-in",
                "leaf.csr",
                "-CA",
                "inter.pem",
                "-CAkey",
                "inter.key",
                "-set_serial",
                "12345678901234567890",
                "-sha256",
                "-days",
                "2000",
                "-extfile",
                "ext.cnf",
                "-extensions",
                "v3leaf",
                "-out",
                "leaf.pem",
            ],
        );
        Some(Signed { dir })
    }

    /// `xml` (with a Signature template) signed by the signer, the chain in KeyInfo.
    pub(crate) fn sign(&self, xml: &str) -> String {
        std::fs::write(self.dir.join("in.xml"), xml).unwrap();
        run(
            &self.dir,
            "xmlsec1",
            &[
                "--sign",
                "--privkey-pem",
                "leaf.key,leaf.pem,inter.pem,root.pem",
                "--id-attr:Id",
                "AuthenticatedPublic",
                "--id-attr:Id",
                "AuthenticatedPrivate",
                "--output",
                "out.xml",
                "in.xml",
            ],
        );
        std::fs::read_to_string(self.dir.join("out.xml")).unwrap()
    }
}

impl Drop for Signed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

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
    let signed = s.sign(&DOC.replace("SIGNATURE", TEMPLATE_SIGNATURE));
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let now = DateTime::from_timestamp(secs, 0).unwrap();
    let v = check(&signed, Some("12345678901234567890"), now).unwrap();
    assert_eq!(v.references, ["ID_AuthenticatedPublic", "ID_AuthenticatedPrivate"]);
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
    let e = check(&signed, None, now - chrono::Duration::days(2)).unwrap_err();
    assert!(e.contains("not valid at"), "{e}");
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
    assert!(check(&no_root, None, now).unwrap_err().contains("issuer"), "missing root");
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
