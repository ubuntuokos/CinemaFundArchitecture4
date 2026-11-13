//! Key Delivery Messages (SMPTE ST 430-1): the content keys of an encrypted composition, each
//! RSA-OAEP encrypted (ST 430-3: SHA-1 digest and MGF1) for the recipient's certificate.
//!
//! [`open_kdm`] decrypts every `EncryptedKey` with the recipient's RSA private key and checks the
//! informative validity rules of ST 430-1 7.2.3: the Structure ID, the composition named in the RSA
//! block against the one in the public part, and the key validity window against the current time.
//! The KDM's XML signature is not verified (#933 follow-up): the content keys only open with the
//! recipient's own private key, which the user supplies.
//!
//! Nothing here stores a key: the caller gets them in memory and decides what to do with them.

use base64::Engine as _;
use chrono::{DateTime, Utc};
use roxmltree::Node;
use rsa::Oaep;
use rsa::PrivateKey;
use sha1::Sha1;

use crate::{Limits, Uuid};

/// ST 430-1 Table 6 Structure ID.
pub const STRUCTURE_ID: [u8; 16] = [0xf1, 0xdc, 0x12, 0x44, 0x60, 0x16, 0x9a, 0x0e, 0x85, 0xbc, 0x30, 0x06, 0x42, 0xf8, 0x66, 0xab];
/// Length of the plaintext of one `EncryptedKey` (ST 430-1 Table 6).
pub const CIPHER_DATA_LEN: usize = 138;
/// Largest KDM read (a KDM with hundreds of keys stays far below).
const MAX_KDM: u64 = 4 << 20;

/// Errors that stop reading a KDM.
#[derive(Debug, thiserror::Error)]
pub enum KdmError {
    #[error("{0}")]
    Document(#[from] crate::Error),
    #[error("not a KDM: {0}")]
    NotKdm(String),
    #[error("the private key could not be read (PEM PKCS#1 or PKCS#8 RSA key expected): {0}")]
    PrivateKey(String),
    #[error("the KDM was not made for this private key (an EncryptedKey does not decrypt)")]
    WrongRecipient,
    #[error("damaged KDM: {0}")]
    Invalid(String),
    #[error("the KDM is not valid yet: its keys work from {0}")]
    NotYetValid(String),
    #[error("the KDM has expired: its keys worked until {0}")]
    Expired(String),
}

/// One content key of a KDM. `Debug` never prints the key.
#[derive(Clone, PartialEq, Eq)]
pub struct KdmKey {
    pub key_id: Uuid,
    /// Four-character key type: `MDIK` (image), `MDAK` (sound), `MDSK` (subtitle), `MDMK` (MIC
    /// key of the content key with the same KeyId), `FMIK` / `FMAK` (forensic marking).
    pub key_type: String,
    pub cpl_id: Uuid,
    pub not_before: String,
    pub not_after: String,
    pub key: [u8; 16],
}

impl std::fmt::Debug for KdmKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KdmKey")
            .field("key_id", &self.key_id)
            .field("key_type", &self.key_type)
            .field("cpl_id", &self.cpl_id)
            .field("not_before", &self.not_before)
            .field("not_after", &self.not_after)
            .finish_non_exhaustive()
    }
}

/// A decrypted KDM.
#[derive(Clone, Debug)]
pub struct Kdm {
    pub message_id: Option<Uuid>,
    pub annotation: Option<String>,
    pub content_title: Option<String>,
    pub cpl_id: Option<Uuid>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub keys: Vec<KdmKey>,
}

/// Reads an RSA private key from PEM text (`RSA PRIVATE KEY` PKCS#1 or `PRIVATE KEY` PKCS#8).
pub fn private_key_from_pem(pem: &str) -> Result<PrivateKey, KdmError> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    if pem.contains("ENCRYPTED PRIVATE KEY") {
        return Err(KdmError::PrivateKey("the key is password-protected; export it without a password".into()));
    }
    PrivateKey::from_pkcs1_pem(pem).or_else(|e1| PrivateKey::from_pkcs8_pem(pem).map_err(|e8| KdmError::PrivateKey(format!("PKCS#1: {e1}; PKCS#8: {e8}"))))
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn descendants<'a, 'i>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.descendants().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn text(n: Option<Node>) -> Option<String> {
    n.and_then(|n| n.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn time(s: &str) -> Result<DateTime<Utc>, KdmError> {
    DateTime::parse_from_rfc3339(s.trim()).map(|t| t.with_timezone(&Utc)).map_err(|_| KdmError::Invalid(format!("date-time {s:?}")))
}

/// Decrypts the content keys of KDM `xml` with the recipient's `key`, checking them at `now`.
pub fn open_kdm(xml: &str, key: &PrivateKey, now: DateTime<Utc>) -> Result<Kdm, KdmError> {
    let doc = crate::xml::parse(xml, Limits { max_bytes: MAX_KDM, max_nodes: 200_000 })?;
    let root = doc.root_element();
    if root.tag_name().name() != "DCinemaSecurityMessage" {
        return Err(KdmError::NotKdm(format!("root element {}", root.tag_name().name())));
    }
    let public = child(root, "AuthenticatedPublic").ok_or_else(|| KdmError::NotKdm("no AuthenticatedPublic".into()))?;
    let ext = descendants(public, "KDMRequiredExtensions").next().ok_or_else(|| KdmError::NotKdm("no KDMRequiredExtensions".into()))?;
    let private = child(root, "AuthenticatedPrivate").ok_or_else(|| KdmError::NotKdm("no AuthenticatedPrivate".into()))?;
    let mut kdm = Kdm {
        message_id: text(child(public, "MessageId")).and_then(|t| Uuid::parse_lenient(&t)),
        annotation: text(child(public, "AnnotationText")),
        content_title: text(child(ext, "ContentTitleText")),
        cpl_id: text(child(ext, "CompositionPlaylistId")).and_then(|t| Uuid::parse_lenient(&t)),
        not_before: text(child(ext, "ContentKeysNotValidBefore")),
        not_after: text(child(ext, "ContentKeysNotValidAfter")),
        keys: Vec::new(),
    };
    for ek in descendants(private, "EncryptedKey") {
        let b64: String = text(descendants(ek, "CipherValue").next()).ok_or_else(|| KdmError::Invalid("EncryptedKey without a CipherValue".into()))?;
        let b64: String = b64.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let ct = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()).map_err(|e| KdmError::Invalid(format!("CipherValue: {e}")))?;
        let pt = key.decrypt(Oaep::<Sha1>::new(), &ct).map_err(|_| KdmError::WrongRecipient)?;
        kdm.keys.push(cipher_data(&pt)?);
    }
    if kdm.keys.is_empty() {
        return Err(KdmError::NotKdm("no EncryptedKey".into()));
    }
    for k in &kdm.keys {
        if kdm.cpl_id.is_some_and(|c| c != k.cpl_id) {
            return Err(KdmError::Invalid("an EncryptedKey names another composition than the KDM".into()));
        }
        if now < time(&k.not_before)? {
            return Err(KdmError::NotYetValid(k.not_before.clone()));
        }
        if now > time(&k.not_after)? {
            return Err(KdmError::Expired(k.not_after.clone()));
        }
    }
    // the RSA blocks are authoritative (ST 430-1 7.2.3)
    kdm.cpl_id = kdm.keys.first().map(|k| k.cpl_id);
    Ok(kdm)
}

/// The fields of one decrypted `CipherData` block (ST 430-1 Table 6).
pub fn cipher_data(pt: &[u8]) -> Result<KdmKey, KdmError> {
    if pt.len() != CIPHER_DATA_LEN {
        return Err(KdmError::Invalid(format!("EncryptedKey plaintext of {} bytes, not {CIPHER_DATA_LEN}", pt.len())));
    }
    let at = |a: usize, n: usize| pt.get(a..a + n).unwrap_or_default();
    if at(0, 16) != STRUCTURE_ID {
        return Err(KdmError::Invalid("wrong Structure ID in an EncryptedKey".into()));
    }
    let uuid = |a: usize| Uuid(at(a, 16).try_into().unwrap_or([0; 16]));
    let ascii = |a: usize, n: usize| -> Result<String, KdmError> {
        std::str::from_utf8(at(a, n)).map(str::to_string).map_err(|_| KdmError::Invalid("EncryptedKey text field is not ASCII".into()))
    };
    let key_type = ascii(52, 4)?;
    Ok(KdmKey {
        key_id: uuid(56),
        key_type,
        cpl_id: uuid(36),
        not_before: ascii(72, 25)?,
        not_after: ascii(97, 25)?,
        key: at(122, 16).try_into().unwrap_or([0; 16]),
    })
}

#[cfg(test)]
mod tests;
