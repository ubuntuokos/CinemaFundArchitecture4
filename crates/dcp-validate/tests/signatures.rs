//! DCP-10: `DCP.SIG.001` and `DCP.SIG.002` on small SMPTE packages whose CPL and PKL are signed
//! with xmlsec1 (`cfa4_dcp::testing::Signed`, ST 430-2 chains made with openssl; those tests skip
//! without the tools unless `CFA4_REQUIRE_ORACLES` is set), and on unsigned and broken ones.

use std::path::{Path, PathBuf};

use cfa4_dcp::testing::{SIGNER_SERIAL, Signed};
use cfa4_dcp_validate::rules::{SIG_CERTIFICATES, SIG_SIGNATURE};
use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest};

const DSIG: &str = "http://www.w3.org/2000/09/xmldsig#";

fn uuid(n: u32) -> String {
    format!("{n:08x}-1111-4222-8333-444444444444")
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-validate-sig-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The `Signer` and the `Signature` template of a CPL or PKL (ST 429-7 / ST 429-8 with ST 430-3):
/// one reference to the whole document, enveloped.
fn signature_block() -> String {
    format!(
        r#"<Signer><dsig:X509Data><dsig:X509IssuerSerial><dsig:X509IssuerName>O=test,OU=cfa4,CN=.inter</dsig:X509IssuerName><dsig:X509SerialNumber>{SIGNER_SERIAL}</dsig:X509SerialNumber></dsig:X509IssuerSerial></dsig:X509Data></Signer>
<dsig:Signature>
  <dsig:SignedInfo>
    <dsig:CanonicalizationMethod Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315#WithComments"/>
    <dsig:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/>
    <dsig:Reference URI=""><dsig:Transforms><dsig:Transform Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/></dsig:Transforms><dsig:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><dsig:DigestValue/></dsig:Reference>
  </dsig:SignedInfo>
  <dsig:SignatureValue/>
  <dsig:KeyInfo><dsig:X509Data><dsig:X509Certificate/></dsig:X509Data></dsig:KeyInfo>
</dsig:Signature>"#
    )
}

fn cpl(signed: bool, title: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL" xmlns:dsig="{DSIG}"><Id>urn:uuid:{}</Id><IssueDate>2026-11-15T00:00:00Z</IssueDate><ContentTitleText>{title}</ContentTitleText><ContentKind>test</ContentKind><ReelList><Reel><Id>urn:uuid:{}</Id><AssetList>
<MainMarkers><Id>urn:uuid:{}</Id><EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><MarkerList><Marker><Label>FFOC</Label><Offset>0</Offset></Marker><Marker><Label>LFOC</Label><Offset>0</Offset></Marker></MarkerList></MainMarkers>
</AssetList></Reel></ReelList>{}</CompositionPlaylist>
"#,
        uuid(2),
        uuid(4),
        uuid(6),
        if signed { signature_block() } else { String::new() }
    )
}

fn pkl(cpl: &str, signed: bool) -> String {
    let digest = cfa4_dcp::sha1::base64(&cfa4_dcp::sha1::sha1(cpl.as_bytes()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL" xmlns:dsig="{DSIG}"><Id>urn:uuid:{}</Id><IssueDate>2026-11-15T00:00:00Z</IssueDate><Issuer>t</Issuer><Creator>t</Creator><AssetList><Asset><Id>urn:uuid:{}</Id><Hash>{digest}</Hash><Size>{}</Size><Type>text/xml</Type></Asset></AssetList>{}</PackingList>
"#,
        uuid(1),
        uuid(2),
        cpl.len(),
        if signed { signature_block() } else { String::new() }
    )
}

fn write(root: &Path, cpl: &str, pkl: &str) {
    let am = format!(
        r#"<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Id>urn:uuid:{}</Id><Creator>t</Creator><VolumeCount>1</VolumeCount><IssueDate>2026-11-15T00:00:00Z</IssueDate><Issuer>t</Issuer><AssetList>
<Asset><Id>urn:uuid:{}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>pkl.xml</Path></Chunk></ChunkList></Asset>
<Asset><Id>urn:uuid:{}</Id><ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
</AssetList></AssetMap>"#,
        uuid(9),
        uuid(1),
        uuid(2)
    );
    std::fs::write(root.join("ASSETMAP.xml"), am).unwrap();
    std::fs::write(root.join("cpl.xml"), cpl).unwrap();
    std::fs::write(root.join("pkl.xml"), pkl).unwrap();
}

fn validate(root: &Path, profile: &str) -> ValidationReport {
    let req = ValidationRequest::new(root, Profile::builtin(profile).unwrap());
    NativeValidator.validate(&req, &ValidationControl::new()).unwrap()
}

fn findings(r: &ValidationReport, rule: &str) -> Vec<(Status, String)> {
    r.findings.iter().filter(|f| f.rule == rule).map(|f| (f.status, f.message.clone())).collect()
}

/// A package whose CPL and PKL `s` signed; `edit` changes the signed CPL before the PKL is made.
fn signed_package(name: &str, s: &Signed, edit: impl Fn(String) -> String) -> PathBuf {
    let d = dir(name);
    let c = edit(s.sign(&cpl(true, "Signed")));
    let p = s.sign(&pkl(&c, true));
    write(&d, &c, &p);
    d
}

#[test]
fn a_signed_package_verifies() {
    let Some(s) = Signed::new("dcp-sig-good") else { return };
    let d = signed_package("good", &s, |c| c);
    let r = validate(&d, "security");
    assert_eq!(findings(&r, SIG_SIGNATURE), [(Status::Pass, "2 signatures verify".to_string())]);
    assert_eq!(findings(&r, SIG_CERTIFICATES), [(Status::Pass, "2 certificate chains keep ST 430-2".to_string())]);
}

#[test]
fn a_changed_document_fails() {
    let Some(s) = Signed::new("dcp-sig-changed") else { return };
    let d = signed_package("changed", &s, |c| c.replace("<ContentTitleText>Signed<", "<ContentTitleText>Changed<"));
    let r = validate(&d, "security");
    let f = findings(&r, SIG_SIGNATURE);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].0, Status::Fail);
    assert!(f[0].1.contains("'cpl.xml': the signature does not verify: the digest of the document does not match"), "{f:?}");
    assert_eq!(findings(&r, SIG_CERTIFICATES)[0].0, Status::Unchecked);
}

#[test]
fn a_chain_outside_st_430_2_fails() {
    let Some(s) = Signed::non_conforming("dcp-sig-plain") else { return };
    let d = signed_package("plain", &s, |c| c);
    let r = validate(&d, "security");
    assert_eq!(findings(&r, SIG_SIGNATURE), [(Status::Pass, "2 signatures verify".to_string())]);
    let f = findings(&r, SIG_CERTIFICATES);
    assert!(f.len() == 2 && f.iter().all(|(st, m)| *st == Status::Fail && m.contains("(ST 430-2)")), "{f:?}");
}

#[test]
fn an_unsigned_document_beside_signed_ones_is_a_warning_where_optional() {
    let Some(s) = Signed::new("dcp-sig-mixed") else { return };
    let d = dir("mixed");
    let c = cpl(false, "Unsigned");
    let p = s.sign(&pkl(&c, true));
    write(&d, &c, &p);
    let r = validate(&d, "delivery-preflight");
    assert_eq!(findings(&r, SIG_SIGNATURE), [(Status::Warning, "composition playlist 'cpl.xml' is not signed, while others of the package are".to_string())]);
    assert_eq!(findings(&r, SIG_CERTIFICATES), [(Status::Pass, "1 certificate chains keep ST 430-2".to_string())]);
}

#[test]
fn unsigned_packages_fail_only_where_signatures_are_required() {
    let d = dir("unsigned");
    let c = cpl(false, "Unsigned");
    write(&d, &c, &pkl(&c, false));
    let r = validate(&d, "security");
    let f = findings(&r, SIG_SIGNATURE);
    assert_eq!(f.len(), 2, "{f:?}");
    assert!(f.iter().all(|(st, m)| *st == Status::Fail && m.ends_with("is not signed")), "{f:?}");
    let r = validate(&d, "delivery-preflight");
    assert_eq!(findings(&r, SIG_SIGNATURE), [(Status::NotApplicable, "no composition playlist or packing list is signed".to_string())]);
    assert_eq!(findings(&r, SIG_CERTIFICATES), [(Status::NotApplicable, "no composition playlist or packing list is signed".to_string())]);
    assert!(findings(&validate(&d, "full-integrity"), SIG_SIGNATURE).is_empty());
}

#[test]
fn a_signer_without_a_signature_and_a_signature_that_is_not_one_fail() {
    let d = dir("half");
    let c = cpl(false, "Half").replace("</ReelList>", "</ReelList><Signer><dsig:X509Data><dsig:X509IssuerSerial><dsig:X509IssuerName>x</dsig:X509IssuerName><dsig:X509SerialNumber>1</dsig:X509SerialNumber></dsig:X509IssuerSerial></dsig:X509Data></Signer>");
    // the template itself, never filled in: no digest, no signature value, no certificate
    let p = pkl(&c, true);
    write(&d, &c, &p);
    let r = validate(&d, "security");
    let f = findings(&r, SIG_SIGNATURE);
    assert!(f.iter().any(|(st, m)| *st == Status::Fail && m == "composition playlist 'cpl.xml' has a Signer but no Signature"), "{f:?}");
    assert!(f.iter().any(|(st, m)| *st == Status::Fail && m.starts_with("packing list 'pkl.xml': the signature does not verify: ")), "{f:?}");
    assert!(findings(&r, SIG_CERTIFICATES).iter().all(|(st, _)| *st == Status::Unchecked));
}
