//! DCP-10 (plan F14): the XML signatures of the composition playlists and packing lists.
//!
//! `DCP.SIG.001`, per signed document: the enveloped `ds:Signature` (W3C XML Signature as ST 430-3
//! profiles it) verifies with `cfa4_dcp::xmldsig`, covers the whole document (`URI=""`), and the
//! certificate the `Signer` element names is the one that signed. `FAIL` when the signature does
//! not verify, covers only part of the document, the `Signer` and the `Signature` do not come
//! together, or the document cannot be read again.
//!
//! An unsigned document is a `FAIL` where the profile requires `DCP.SIG.001` (`security`). Where
//! the rule is optional, it is `NOT_APPLICABLE` when no document of the package is signed and a
//! `WARNING` for each unsigned one when others are.
//!
//! `DCP.SIG.002`, per verified signature: the certificate chain in `KeyInfo` keeps ST 430-2 6.2
//! (every certificate, every link, validity now, up to a self-signed root), and the signer's
//! certificate has the content signer role (`CS` in its common name, ST 430-2 5.3.4). Whether
//! the root is trusted is not decided here.

use std::path::Path;

use cfa4_dcp::xmldsig::{self, DSIG_NS};
use cfa4_dcp::{Composition, PackingList, Uuid};
use chrono::{DateTime, Utc};

use crate::integrity::Files;
use crate::rules::{SIG_CERTIFICATES, SIG_SIGNATURE};
use crate::{Finding, Location, Status, ValidationControl};

/// One CPL or PKL as the readers saw it.
struct Doc<'a> {
    kind: &'static str,
    path: &'a str,
    file: &'a Path,
    signer: bool,
    signature: bool,
}

/// What the signature check of one document found.
enum Outcome {
    Unsigned,
    /// SIG.001 failed; SIG.002 cannot be checked.
    Broken(String),
    /// Verified; the chain problem, if any, and the signer's name.
    Verified {
        chain: Option<String>,
        signer: String,
        role: bool,
    },
}

fn signer_serial(root: roxmltree::Node<'_, '_>) -> Option<String> {
    let signer = root.children().find(|c| c.is_element() && c.tag_name().name() == "Signer")?;
    signer
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "X509SerialNumber" && n.tag_name().namespace() == Some(DSIG_NS))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

fn check_one(d: &Doc<'_>, limits: cfa4_dcp::Limits, now: DateTime<Utc>) -> Outcome {
    match (d.signer, d.signature) {
        (false, false) => return Outcome::Unsigned,
        (true, false) => return Outcome::Broken(format!("{} '{}' has a Signer but no Signature", d.kind, d.path)),
        (false, true) => return Outcome::Broken(format!("{} '{}' has a Signature but no Signer", d.kind, d.path)),
        (true, true) => {}
    }
    let len = std::fs::metadata(d.file).map(|m| m.len()).unwrap_or(u64::MAX);
    if len > limits.max_bytes {
        return Outcome::Broken(format!("{} '{}' is {len} bytes, over the limit of {}; its signature is not read", d.kind, d.path, limits.max_bytes));
    }
    let text = match std::fs::read(d.file).map(String::from_utf8) {
        Ok(Ok(t)) => t,
        Ok(Err(_)) => return Outcome::Broken(format!("{} '{}' is not UTF-8", d.kind, d.path)),
        Err(e) => return Outcome::Broken(format!("{} '{}' cannot be read again: {e}", d.kind, d.path)),
    };
    let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: limits.max_nodes, ..roxmltree::ParsingOptions::default() };
    let doc = match roxmltree::Document::parse_with_options(text.trim_start_matches('\u{feff}'), opts) {
        Ok(doc) => doc,
        Err(e) => return Outcome::Broken(format!("{} '{}' cannot be parsed again: {e}", d.kind, d.path)),
    };
    let root = doc.root_element();
    let sigs: Vec<_> = root.children().filter(|c| c.is_element() && c.tag_name().name() == "Signature" && c.tag_name().namespace() == Some(DSIG_NS)).collect();
    let [sig] = sigs[..] else {
        return Outcome::Broken(format!("{} '{}' has {} Signature elements; it needs one", d.kind, d.path, sigs.len()));
    };
    let serial = signer_serial(root);
    let v = match xmldsig::verify(&doc, sig, serial.as_deref(), now) {
        Ok(v) => v,
        Err(e) => return Outcome::Broken(format!("{} '{}': the signature does not verify: {e}", d.kind, d.path)),
    };
    if !v.references.iter().any(String::is_empty) {
        return Outcome::Broken(format!("{} '{}': the signature covers only {:?}, not the whole document (URI=\"\")", d.kind, d.path, v.references));
    }
    let leaf = v.chain.first();
    let signer = leaf.map(xmldsig::name).unwrap_or_default();
    // ST 430-2: the role is the first word of the common name, e.g. "CS.example.com" or "CS SM"
    let role = signer.split(['.', ' ']).next().is_some_and(|r| r == "CS");
    Outcome::Verified { chain: v.chain_problem, signer, role }
}

/// `DCP.SIG.001` and `DCP.SIG.002`; `None` when cancelled. `required` says whether the profile
/// requires `DCP.SIG.001`.
#[allow(clippy::type_complexity)]
pub(crate) fn check(
    pkls: &[(String, PackingList)],
    cpls: &[(String, Composition)],
    files: &Files,
    limits: cfa4_dcp::Limits,
    required: bool,
    control: &ValidationControl,
) -> Option<((Vec<Finding>, String), (Vec<Finding>, String))> {
    // the system clock, as the report's timestamps read it
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let now = i64::try_from(secs).ok().and_then(|s| DateTime::from_timestamp(s, 0)).unwrap_or_default();
    let mut docs = Vec::new();
    let mut add = |kind: &'static str, id: Option<Uuid>, signer: bool, signature: bool| {
        if let Some((id, (path, file))) = id.and_then(|id| Some((id, files.get(&id)?))) {
            docs.push((id, Doc { kind, path, file, signer, signature }));
        }
    };
    for (_, c) in cpls {
        add("composition playlist", c.id, c.has_signer, c.has_signature);
    }
    for (_, p) in pkls {
        add("packing list", p.id, p.has_signer, p.has_signature);
    }
    let (mut sig, mut cert) = (Vec::new(), Vec::new());
    let mut outcomes = Vec::new();
    for (id, d) in &docs {
        if control.is_cancelled() {
            return None;
        }
        outcomes.push((*id, d, check_one(d, limits, now)));
    }
    let signed = outcomes.iter().filter(|(_, _, o)| !matches!(o, Outcome::Unsigned)).count();
    let at = |id: Uuid, d: &Doc<'_>| Location { file: Some(d.path.to_string()), asset: Some(id), ..Location::default() };
    let (mut verified, mut chains) = (0usize, 0usize);
    for (id, d, o) in &outcomes {
        let loc = at(*id, d);
        match o {
            Outcome::Unsigned if required => sig.push(Finding::new(SIG_SIGNATURE, Status::Fail, format!("{} '{}' is not signed", d.kind, d.path)).at(loc)),
            Outcome::Unsigned if signed > 0 => {
                sig.push(
                    Finding::new(SIG_SIGNATURE, Status::Warning, format!("{} '{}' is not signed, while others of the package are", d.kind, d.path)).at(loc),
                );
            }
            Outcome::Unsigned => {}
            Outcome::Broken(m) => {
                sig.push(Finding::new(SIG_SIGNATURE, Status::Fail, m.clone()).at(loc.clone()));
                cert.push(Finding::new(SIG_CERTIFICATES, Status::Unchecked, format!("{} '{}': the signature did not verify", d.kind, d.path)).at(loc));
            }
            Outcome::Verified { chain, signer, role } => {
                verified += 1;
                match chain {
                    Some(p) => cert.push(Finding::new(SIG_CERTIFICATES, Status::Fail, format!("{} '{}': {p}", d.kind, d.path)).at(loc.clone())),
                    None => chains += 1,
                }
                if !role {
                    cert.push(
                        Finding::new(
                            SIG_CERTIFICATES,
                            Status::Fail,
                            format!("{} '{}' is signed by {signer:?}, which has not the content signer role (CS, ST 430-2)", d.kind, d.path),
                        )
                        .at(loc),
                    );
                }
            }
        }
    }
    if signed == 0 && !required {
        let why = if docs.is_empty() { "no composition playlist or packing list was read" } else { "no composition playlist or packing list is signed" };
        sig.push(Finding::new(SIG_SIGNATURE, Status::NotApplicable, why));
        cert.push(Finding::new(SIG_CERTIFICATES, Status::NotApplicable, why));
    } else if verified == 0 && cert.is_empty() {
        cert.push(Finding::new(SIG_CERTIFICATES, Status::Unchecked, "no signature verified"));
    }
    Some(((sig, format!("{verified} signatures verify")), (cert, format!("{chains} certificate chains keep ST 430-2"))))
}
