//! Enveloped XML Signature checking (W3C XML Signature Syntax and Processing) as D-Cinema
//! messages use it (ST 430-3 via ST 430-1): same-document references (`URI="#id"`) with SHA-256
//! or SHA-1 digests, inclusive Canonical XML 1.0 with or without comments, RSA PKCS#1 v1.5
//! signatures, and the signer's certificate chain carried in `KeyInfo`.
//!
//! The signed content is checked first: the references' digests and the signature value under the
//! signer's key. Only then the chain, which is a separate verdict: every certificate keeps the
//! rules of ST 430-2 6.2 (steps 2 to 18: version 3, extensions, key usage, OrganizationName,
//! roles, sha256WithRSA, 2048-bit keys, dnQualifier thumbprints, issuer names and validity
//! nesting), is valid at the given time, is signed by the next one, and the last one is a
//! self-signed root. Whether that root is trusted (step 19) is the caller's decision.

use base64::Engine as _;
use chrono::{DateTime, Utc};
use roxmltree::{Document, Node};

use crate::c14n::canonicalize;
use crate::x509::{Certificate, SigAlg, verify_rsa};

/// The XML Signature namespace.
pub const DSIG_NS: &str = "http://www.w3.org/2000/09/xmldsig#";
const C14N: &str = "http://www.w3.org/TR/2001/REC-xml-c14n-20010315";
const C14N_COMMENTS: &str = "http://www.w3.org/TR/2001/REC-xml-c14n-20010315#WithComments";
const ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
const RSA_SHA1: &str = "http://www.w3.org/2000/09/xmldsig#rsa-sha1";
const SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
const SHA1: &str = "http://www.w3.org/2000/09/xmldsig#sha1";
/// Longest certificate chain followed.
const MAX_CHAIN: usize = 16;

/// A signature whose signed content checked out.
#[derive(Clone, Debug)]
pub struct Verified {
    /// The signer's chain, signer first, root last (as far as it was followed).
    pub chain: Vec<Certificate>,
    /// The `Id`s of the elements the signature covers.
    pub references: Vec<String>,
    /// Why the certificate chain does not check out, when it does not. The content is intact
    /// either way: this is about who signed it.
    pub chain_problem: Option<String>,
}

fn ds_child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(DSIG_NS))
}

fn ds_children<'a, 'i>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(DSIG_NS))
}

fn algorithm<'a>(n: Option<Node<'a, '_>>, what: &str) -> Result<&'a str, String> {
    n.and_then(|n| n.attribute("Algorithm")).ok_or_else(|| format!("{what} has no Algorithm"))
}

fn b64(n: Option<Node<'_, '_>>, what: &str) -> Result<Vec<u8>, String> {
    let t: String = n.and_then(|n| n.text()).unwrap_or_default().chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if t.is_empty() {
        return Err(format!("{what} is empty"));
    }
    base64::engine::general_purpose::STANDARD.decode(t.as_bytes()).map_err(|e| format!("{what}: {e}"))
}

fn digest(alg: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    use sha1::Digest as _;
    match alg {
        SHA256 => Ok(rsa::sha2::Sha256::digest(data).to_vec()),
        SHA1 => Ok(sha1::Sha1::digest(data).to_vec()),
        other => Err(format!("unsupported digest method {other}")),
    }
}

fn c14n_comments(alg: &str) -> Result<bool, String> {
    match alg {
        C14N => Ok(false),
        C14N_COMMENTS => Ok(true),
        other => Err(format!("unsupported canonicalization {other}")),
    }
}

/// Checks the `Signature` element `sig` of `doc` at time `now`. `signer_serial` (decimal), when
/// given, picks the signer's certificate; otherwise it is the one that signs no other. An error
/// means the signed content could not be verified (changed, malformed, or no signer key); a chain
/// that does not check out is `Verified::chain_problem`.
pub fn verify(doc: &Document<'_>, sig: Node<'_, '_>, signer_serial: Option<&str>, now: DateTime<Utc>) -> Result<Verified, String> {
    let si = ds_child(sig, "SignedInfo").ok_or("Signature without SignedInfo")?;
    let canon = c14n_comments(algorithm(ds_child(si, "CanonicalizationMethod"), "CanonicalizationMethod")?)?;
    let alg = match algorithm(ds_child(si, "SignatureMethod"), "SignatureMethod")? {
        RSA_SHA256 => SigAlg::RsaSha256,
        RSA_SHA1 => SigAlg::RsaSha1,
        other => return Err(format!("unsupported signature method {other}")),
    };
    // the references
    let mut references = Vec::new();
    for r in ds_children(si, "Reference") {
        let uri = r.attribute("URI").ok_or("Reference without URI")?;
        let id = uri.strip_prefix('#').ok_or_else(|| format!("only same-document references are supported, not {uri:?}"))?;
        let mut targets = doc.descendants().filter(|n| n.is_element() && ["Id", "ID", "id"].iter().any(|a| n.attribute(*a) == Some(id)));
        let target = targets.next().ok_or_else(|| format!("no element with Id {id:?}"))?;
        if targets.next().is_some() {
            return Err(format!("several elements with Id {id:?}"));
        }
        // a same-document reference is canonicalized without comments unless a transform says
        let (mut comments, mut skip) = (false, None);
        if let Some(ts) = ds_child(r, "Transforms") {
            for t in ds_children(ts, "Transform") {
                match t.attribute("Algorithm").unwrap_or_default() {
                    ENVELOPED => skip = Some(sig.id()),
                    a => comments = c14n_comments(a)?,
                }
            }
        }
        let data = canonicalize(target, comments, skip)?;
        let want = b64(ds_child(r, "DigestValue"), "DigestValue")?;
        if digest(algorithm(ds_child(r, "DigestMethod"), "DigestMethod")?, &data)? != want {
            return Err(format!("the digest of {id:?} does not match: the signed content was changed"));
        }
        references.push(id.to_string());
    }
    if references.is_empty() {
        return Err("the signature has no references".into());
    }
    // the certificates
    let ki = ds_child(sig, "KeyInfo").ok_or("Signature without KeyInfo")?;
    let mut certs = Vec::new();
    for x in ds_children(ki, "X509Data") {
        for c in ds_children(x, "X509Certificate") {
            certs.push(Certificate::from_der(&b64(Some(c), "X509Certificate")?)?);
        }
    }
    let leaf = match signer_serial {
        Some(s) => certs.iter().position(|c| c.serial_decimal() == s.trim()).ok_or("the signer's certificate is not in KeyInfo")?,
        None => certs.iter().position(|c| !certs.iter().any(|o| o.issuer == c.subject && o.subject != c.subject)).ok_or("no signer certificate in KeyInfo")?,
    };
    let signer = certs.get(leaf).ok_or("no signer certificate")?;
    let value = b64(ds_child(sig, "SignatureValue"), "SignatureValue")?;
    let signed = canonicalize(si, canon, None)?;
    verify_rsa(&signer.public_key()?, alg, &signed, &value).map_err(|_| "the signature value does not verify with the signer's certificate".to_string())?;
    let mut chain = Vec::new();
    let chain_problem = build_chain(&certs, leaf, now, &mut chain).err();
    Ok(Verified { chain, references, chain_problem })
}

/// The chain from `certs[leaf]` up to a self-signed root into `chain`, every certificate and
/// link checked (ST 430-2 6.2).
fn build_chain(certs: &[Certificate], leaf: usize, now: DateTime<Utc>, chain: &mut Vec<Certificate>) -> Result<(), String> {
    let mut cur = certs.get(leaf).ok_or("no signer certificate")?.clone();
    loop {
        if let Some(p) = cur.st430_2_problem() {
            let n = name(&cur);
            chain.push(cur);
            return Err(format!("certificate {n} {p} (ST 430-2)"));
        }
        if now < cur.not_before || now > cur.not_after {
            return Err(format!("certificate {} is not valid at {now} (valid {} to {})", name(&cur), cur.not_before, cur.not_after));
        }
        if !chain.is_empty() && !cur.ca {
            return Err(format!("certificate {} signs another but is not a CA", name(&cur)));
        }
        if cur.path_len.is_some_and(|p| (chain.len().saturating_sub(1)) > p as usize) {
            return Err(format!("certificate {} is beyond its path length limit", name(&cur)));
        }
        let root = cur.issuer == cur.subject;
        let issuer = if root {
            cur.clone()
        } else {
            certs.iter().find(|c| c.subject == cur.issuer).cloned().ok_or_else(|| format!("the issuer of {} is not in KeyInfo", name(&cur)))?
        };
        cur.signed_by(&issuer).map_err(|e| format!("certificate {}: {e}", name(&cur)))?;
        if cur.not_before < issuer.not_before || cur.not_after > issuer.not_after {
            return Err(format!("certificate {} is valid outside its issuer's validity (ST 430-2)", name(&cur)));
        }
        chain.push(cur);
        if root {
            return Ok(());
        }
        if chain.len() >= MAX_CHAIN {
            return Err("certificate chain too long".into());
        }
        cur = issuer;
    }
}

/// A certificate's common name (or its serial number).
pub fn name(c: &Certificate) -> String {
    c.subject_attributes.iter().find(|(oid, _)| oid == crate::x509::OID_CN).map(|(_, v)| v.clone()).unwrap_or_else(|| format!("#{}", c.serial_decimal()))
}

#[cfg(test)]
mod tests;
