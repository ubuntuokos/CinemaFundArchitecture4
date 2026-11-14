//! Test helpers shared with other crates' tests (feature `test-util`): an XML-signed document
//! maker built on the openssl and xmlsec1 command lines (oracles only, skipped when missing). The
//! certificate chain keeps ST 430-2 (dnQualifier thumbprints, roles, key usage, path lengths,
//! AuthorityKeyIdentifier), or, for tests of non-conforming chains, only plain X.509.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A signing chain (root, intermediate, signer) with its keys in a temporary directory.
pub struct Signed {
    pub dir: PathBuf,
}

fn tool(name: &str, arg: &str) -> bool {
    let ok = Command::new(name).arg(arg).output().is_ok_and(|o| o.status.success());
    if !ok {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "{name} not found");
        eprintln!("SKIPPED: {name} not found");
    }
    ok
}

fn run(dir: &Path, prog: &str, args: &[&str]) {
    let out = Command::new(prog).current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "{prog} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Extensions of ST 430-2 conforming certificates (5.2): CAs sign certificates only and have a
/// path length; leaves sign and encipher; everyone names its issuer's key.
const EXT: &str = "[v3root]\nbasicConstraints=critical,CA:TRUE,pathlen:1\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\n\
[v3ca]\nbasicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\n\
[v3leaf]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\n";

/// Plain X.509 extensions without what ST 430-2 adds (no path length, no AuthorityKeyIdentifier).
const PLAIN_EXT: &str = "[v3root]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n\
[v3ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n\
[v3leaf]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n";

/// The template: a KDM-shaped Signature that xmlsec1 fills in (ST 430-3 algorithms).
pub const TEMPLATE_SIGNATURE: &str = r##"<dsig:Signature>
    <dsig:SignedInfo>
      <dsig:CanonicalizationMethod Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315#WithComments"/>
      <dsig:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/>
      <dsig:Reference URI="#ID_AuthenticatedPublic"><dsig:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><dsig:DigestValue/></dsig:Reference>
      <dsig:Reference URI="#ID_AuthenticatedPrivate"><dsig:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><dsig:DigestValue/></dsig:Reference>
    </dsig:SignedInfo>
    <dsig:SignatureValue/>
    <dsig:KeyInfo><dsig:X509Data><dsig:X509Certificate/></dsig:X509Data></dsig:KeyInfo>
  </dsig:Signature>"##;

/// The serial number of the signer's certificate.
pub const SIGNER_SERIAL: &str = "12345678901234567890";

impl Signed {
    /// A root (`.root`), an intermediate (`.inter`) and a signer (`CS.signer`, serial
    /// [`SIGNER_SERIAL`]) keeping ST 430-2, or `None` without openssl and xmlsec1.
    pub fn new(name: &str) -> Option<Signed> {
        Signed::make(name, true)
    }

    /// The same chain as plain X.509: no dnQualifier, path lengths or AuthorityKeyIdentifier.
    pub fn non_conforming(name: &str) -> Option<Signed> {
        Signed::make(name, false)
    }

    fn make(name: &str, st430: bool) -> Option<Signed> {
        if !tool("openssl", "version") || !tool("xmlsec1", "--version") {
            return None;
        }
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cfa4-xmldsig-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ext.cnf"), if st430 { EXT } else { PLAIN_EXT }).unwrap();
        for (k, cn) in [("root", ".root"), ("inter", ".inter"), ("leaf", "CS.signer")] {
            run(&dir, "openssl", &["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", &format!("{k}.key")]);
            // the subject, with the public key thumbprint as dnQualifier (ST 430-2 5.3.1, 5.4)
            let mut dn = format!("[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nO=test\nOU=cfa4\nCN={cn}\n");
            if st430 {
                run(&dir, "openssl", &["rsa", "-in", &format!("{k}.key"), "-RSAPublicKey_out", "-outform", "DER", "-out", &format!("{k}.rsa")]);
                use sha1::Digest as _;
                let tp = base64::engine::general_purpose::STANDARD.encode(sha1::Sha1::digest(std::fs::read(dir.join(format!("{k}.rsa"))).unwrap()));
                dn.push_str(&format!("dnQualifier={tp}\n"));
            }
            std::fs::write(dir.join(format!("{k}.cnf")), dn).unwrap();
        }
        let ext = |e: &'static str| ["-extfile", "ext.cnf", "-extensions", e];
        let mut root =
            vec!["req", "-x509", "-new", "-key", "root.key", "-sha256", "-days", "3650", "-config", "root.cnf", "-set_serial", "1", "-out", "root.pem"];
        root.extend(["-extensions", "v3root"]);
        // req reads the extensions from its own config: append them there
        let mut root_cnf = std::fs::read_to_string(dir.join("root.cnf")).unwrap();
        root_cnf.push_str(if st430 { EXT } else { PLAIN_EXT });
        std::fs::write(dir.join("root.cnf"), root_cnf).unwrap();
        run(&dir, "openssl", &root);
        for (k, ca, serial, days, e) in [("inter", "root", "2", "3000", "v3ca"), ("leaf", "inter", SIGNER_SERIAL, "2000", "v3leaf")] {
            run(&dir, "openssl", &["req", "-new", "-key", &format!("{k}.key"), "-config", &format!("{k}.cnf"), "-out", &format!("{k}.csr")]);
            let (csr, pem, cakey) = (format!("{k}.csr"), format!("{ca}.pem"), format!("{ca}.key"));
            let out = format!("{k}.pem");
            let mut args = vec!["x509", "-req", "-in", &csr, "-CA", &pem, "-CAkey", &cakey, "-set_serial", serial, "-sha256", "-days", days, "-out", &out];
            args.extend(ext(e));
            run(&dir, "openssl", &args);
        }
        Some(Signed { dir })
    }

    /// The signer certificate's thumbprint, as a KDM's encrypted key blocks name it.
    pub fn signer_thumbprint(&self) -> [u8; 20] {
        run(&self.dir, "openssl", &["x509", "-in", "leaf.pem", "-outform", "DER", "-out", "leaf.der"]);
        crate::x509::Certificate::from_der(&std::fs::read(self.dir.join("leaf.der")).unwrap()).unwrap().thumbprint()
    }

    /// `xml` (with a Signature template) signed by the signer, the chain in KeyInfo.
    pub fn sign(&self, xml: &str) -> String {
        std::fs::write(self.dir.join("in.xml"), xml).unwrap();
        let mut args = vec![
            "--sign",
            "--privkey-pem",
            "leaf.key,leaf.pem,inter.pem,root.pem",
            "--id-attr:Id",
            "AuthenticatedPublic",
            "--id-attr:Id",
            "AuthenticatedPrivate",
        ];
        // The template's KeyInfo names no key (a KDM carries only the X509Data chain). From
        // xmlsec1 1.3 on the key search is strict and finds nothing without a KeyName
        // (KEY-NOT-FOUND); `--lax-key-search` restores the 1.2 behaviour (any key that fits the
        // signature method). xmlsec1 1.2 doesn't know the option, so it is passed only from 1.3.
        if lax_key_search() {
            args.push("--lax-key-search");
        }
        args.extend(["--output", "out.xml", "in.xml"]);
        run(&self.dir, "xmlsec1", &args);
        std::fs::read_to_string(self.dir.join("out.xml")).unwrap()
    }
}

/// Whether the installed xmlsec1 has strict key search (1.3 and later), which needs
/// `--lax-key-search` to sign with a key that the template's KeyInfo doesn't name.
fn lax_key_search() -> bool {
    let out = Command::new("xmlsec1").arg("--version").output().unwrap();
    // "xmlsec1 1.3.9 (openssl)"
    let version = String::from_utf8_lossy(&out.stdout);
    xmlsec1_is_strict(&version)
}

/// `xmlsec1 --version` output names version 1.3 or later.
fn xmlsec1_is_strict(version: &str) -> bool {
    let Some(number) = version.split_whitespace().find(|w| w.starts_with(|c: char| c.is_ascii_digit())) else {
        return false;
    };
    let mut parts = number.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (major, minor) >= (1, 3)
}

impl Drop for Signed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A POSIX ustar archive (ST 430-9 5) of `files` (path, contents), as a bundle tool writes it:
/// read-only regular files owned by root.
pub fn ustar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (path, data) in files {
        let mut h = [0u8; 512];
        let (prefix, name) = if path.len() > 100 { path.rsplit_once('/').unwrap() } else { ("", *path) };
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000444\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[265..269].copy_from_slice(b"root");
        h[297..301].copy_from_slice(b"root");
        h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// A ST 430-9 CATALOG listing `files` as (FilePath, CPLId).
pub fn catalog(files: &[(&str, &str)]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Catalog xmlns=\"http://www.smpte-ra.org/schemas/429-10/2008/KDMB\" xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\">\n<Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><AnnotationText>Test bundle</AnnotationText><Creator>cfa4 tests</Creator><KDMFileList>",
    );
    for (path, cpl) in files {
        s.push_str(&format!(
            "<KDMFile><CPLId>urn:uuid:{cpl}</CPLId><FilePath>{path}</FilePath><Recipient><X509IssuerSerial><ds:X509IssuerName>CN=.ca,O=test</ds:X509IssuerName><ds:X509SerialNumber>7</ds:X509SerialNumber></X509IssuerSerial><X509SubjectName>CN=SM.server,O=test</X509SubjectName></Recipient><ContentKeysNotValidBefore>2020-01-01T00:00:00Z</ContentKeysNotValidBefore><ContentKeysNotValidAfter>2099-01-01T00:00:00Z</ContentKeysNotValidAfter></KDMFile>"
        ));
    }
    s.push_str("</KDMFileList></Catalog>\n");
    s
}

#[cfg(test)]
mod version_tests {
    use super::xmlsec1_is_strict;

    #[test]
    fn strict_key_search_from_xmlsec1_1_3() {
        assert!(!xmlsec1_is_strict("xmlsec1 1.2.39 (openssl)\n"));
        assert!(xmlsec1_is_strict("xmlsec1 1.3.9 (openssl)\n"));
        assert!(xmlsec1_is_strict("xmlsec1 1.3.0 (openssl)"));
        assert!(xmlsec1_is_strict("xmlsec1 2.0.0 (openssl)"));
        assert!(!xmlsec1_is_strict(""));
    }
}
