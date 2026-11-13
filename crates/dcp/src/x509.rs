//! The parts of an X.509 v3 certificate (IETF RFC 5280) that a signature and chain check needs,
//! read with a small DER reader: the signed `tbsCertificate` bytes, the signature algorithm and
//! value, serial number, issuer and subject names (raw DER, and the attributes as text), validity,
//! the RSA public key and the extensions D-Cinema certificates must carry (ST 430-2 5.2), and the
//! per-certificate validation rules of ST 430-2 6.2.

use chrono::{DateTime, NaiveDate, Utc};

/// Largest certificate read.
const MAX_CERT: usize = 64 << 10;

const OID_SHA256_RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];
const OID_SHA1_RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x05];
const OID_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1D, 0x13];
const OID_KEY_USAGE: &[u8] = &[0x55, 0x1D, 0x0F];
const OID_AUTHORITY_KEY_ID: &[u8] = &[0x55, 0x1D, 0x23];
const OID_SUBJECT_KEY_ID: &[u8] = &[0x55, 0x1D, 0x0E];
/// Name attributes: commonName, organizationName, dnQualifier.
pub const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
pub const OID_O: &[u8] = &[0x55, 0x04, 0x0A];
pub const OID_DN_QUALIFIER: &[u8] = &[0x55, 0x04, 0x2E];
/// KeyUsage bits (RFC 5280 4.2.1.3) as `Certificate::key_usage` holds them, bit 0 first.
pub const KU_DIGITAL_SIGNATURE: u16 = 1 << 0;
pub const KU_KEY_ENCIPHERMENT: u16 = 1 << 2;
pub const KU_KEY_CERT_SIGN: u16 = 1 << 5;
pub const KU_CRL_SIGN: u16 = 1 << 6;

/// Signature algorithms of a certificate or an XML signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigAlg {
    RsaSha256,
    RsaSha1,
}

/// One certificate.
#[derive(Clone, Debug)]
pub struct Certificate {
    /// The whole certificate (DER).
    pub der: Vec<u8>,
    /// `tbsCertificate`, the signed part (DER).
    pub tbs: Vec<u8>,
    pub signature_algorithm: Option<SigAlg>,
    pub signature: Vec<u8>,
    /// Serial number, big-endian two's complement as stored.
    pub serial: Vec<u8>,
    /// Issuer and subject `Name`s (DER).
    pub issuer: Vec<u8>,
    pub subject: Vec<u8>,
    /// The subject's and the issuer's attributes as (OID bytes, value text), in order.
    pub subject_attributes: Vec<(Vec<u8>, String)>,
    pub issuer_attributes: Vec<(Vec<u8>, String)>,
    /// The X.509 version as stored (2 for v3).
    pub version: u8,
    /// The `signature` algorithm inside `tbsCertificate` (must equal `signature_algorithm`).
    pub inner_signature_algorithm: Option<SigAlg>,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// `SubjectPublicKeyInfo` (DER) and the `subjectPublicKey` bits (an RSAPublicKey, DER).
    pub spki: Vec<u8>,
    pub public_key_bits: Vec<u8>,
    /// Basic constraints: a CA, with its path length limit.
    pub ca: bool,
    pub path_len: Option<u32>,
    /// Which of the extensions ST 430-2 requires are there.
    pub has_basic_constraints: bool,
    pub has_authority_key_id: bool,
    /// KeyUsage bits (bit 0 = digitalSignature), when the extension is there.
    pub key_usage: Option<u16>,
    /// An extension this reader does not know is marked critical (its OID, dotted).
    pub unknown_critical: Option<String>,
}

/// A DER element: tag, contents and the whole element.
struct Tlv<'a> {
    tag: u8,
    value: &'a [u8],
    whole: &'a [u8],
}

struct Der<'a> {
    b: &'a [u8],
}

impl<'a> Der<'a> {
    fn next(&mut self) -> Result<Tlv<'a>, String> {
        let b = self.b;
        let tag = *b.first().ok_or("DER element missing")?;
        let first = *b.get(1).ok_or("DER length missing")?;
        let (len, head) = if first < 0x80 {
            (usize::from(first), 2usize)
        } else {
            let n = usize::from(first & 0x7F);
            if n == 0 || n > 4 {
                return Err("unsupported DER length".into());
            }
            let bytes = b.get(2..2 + n).ok_or("DER length cut short")?;
            (bytes.iter().fold(0usize, |a, &x| a << 8 | usize::from(x)), 2 + n)
        };
        let end = head.checked_add(len).ok_or("DER length overflows")?;
        let value = b.get(head..end).ok_or("DER element cut short")?;
        let whole = b.get(..end).ok_or("DER element cut short")?;
        self.b = b.get(end..).unwrap_or_default();
        Ok(Tlv { tag, value, whole })
    }
    fn expect(&mut self, tag: u8, what: &str) -> Result<Tlv<'a>, String> {
        let t = self.next()?;
        if t.tag != tag {
            return Err(format!("{what}: DER tag {:02x}, expected {tag:02x}", t.tag));
        }
        Ok(t)
    }
    fn peek(&self) -> Option<u8> {
        self.b.first().copied()
    }
}

fn alg_of(seq: &[u8]) -> Result<Option<SigAlg>, String> {
    let oid = Der { b: seq }.expect(0x06, "algorithm")?;
    Ok(match oid.value {
        o if o == OID_SHA256_RSA => Some(SigAlg::RsaSha256),
        o if o == OID_SHA1_RSA => Some(SigAlg::RsaSha1),
        _ => None,
    })
}

fn time_of(t: &Tlv) -> Result<DateTime<Utc>, String> {
    let s = std::str::from_utf8(t.value).map_err(|_| "certificate time is not text")?;
    let digits = s.strip_suffix('Z').ok_or("certificate time is not UTC")?;
    let full = match t.tag {
        // UTCTime YYMMDDHHMMSS: years 50-99 are 19xx (RFC 5280 4.1.2.5.1)
        0x17 if digits.len() == 12 => {
            let yy: u32 = digits.get(..2).and_then(|y| y.parse().ok()).ok_or("certificate time")?;
            format!("{}{digits}", if yy >= 50 { "19" } else { "20" })
        }
        0x18 if digits.len() == 14 => digits.to_string(),
        _ => return Err("unsupported certificate time".into()),
    };
    let num = |a: usize, n: usize| -> Result<u32, String> { full.get(a..a + n).and_then(|v| v.parse().ok()).ok_or_else(|| "certificate time".to_string()) };
    let date = NaiveDate::from_ymd_opt(num(0, 4)? as i32, num(4, 2)?, num(6, 2)?).ok_or("certificate date")?;
    let dt = date.and_hms_opt(num(8, 2)?, num(10, 2)?, num(12, 2)?).ok_or("certificate time of day")?;
    Ok(dt.and_utc())
}

fn attributes(name: &[u8]) -> Result<Vec<(Vec<u8>, String)>, String> {
    let mut out = Vec::new();
    let mut rdns = Der { b: name };
    while rdns.peek().is_some() {
        let set = rdns.expect(0x31, "name RDN")?;
        let mut atvs = Der { b: set.value };
        while atvs.peek().is_some() {
            let atv = atvs.expect(0x30, "name attribute")?;
            let mut d = Der { b: atv.value };
            let oid = d.expect(0x06, "attribute type")?;
            let v = d.next()?;
            out.push((oid.value.to_vec(), String::from_utf8_lossy(v.value).into_owned()));
        }
    }
    Ok(out)
}

impl Certificate {
    /// Parses a DER certificate.
    pub fn from_der(der: &[u8]) -> Result<Certificate, String> {
        if der.len() > MAX_CERT {
            return Err("certificate too large".into());
        }
        let mut top = Der { b: der };
        let cert = top.expect(0x30, "certificate")?;
        let mut c = Der { b: cert.value };
        let tbs = c.expect(0x30, "tbsCertificate")?;
        let alg = c.expect(0x30, "signatureAlgorithm")?;
        let sig = c.expect(0x03, "signatureValue")?;
        let mut t = Der { b: tbs.value };
        // version [0] EXPLICIT INTEGER, v1 (0) when left out
        let mut version = 0u8;
        if t.peek() == Some(0xA0) {
            let v = t.next()?;
            version = Der { b: v.value }.expect(0x02, "version")?.value.last().copied().unwrap_or(0);
        }
        let serial = t.expect(0x02, "serialNumber")?;
        let inner_alg = t.expect(0x30, "signature")?;
        let issuer = t.expect(0x30, "issuer")?;
        let validity = t.expect(0x30, "validity")?;
        let subject = t.expect(0x30, "subject")?;
        let spki = t.expect(0x30, "subjectPublicKeyInfo")?;
        let mut v = Der { b: validity.value };
        let not_before = time_of(&v.next()?)?;
        let not_after = time_of(&v.next()?)?;
        let mut s = Der { b: spki.value };
        s.expect(0x30, "public key algorithm")?;
        let bits = s.expect(0x03, "subjectPublicKey")?;
        let (mut ca, mut path_len) = (false, None);
        let (mut has_bc, mut has_aki, mut key_usage, mut unknown_critical) = (false, false, None, None);
        while let Some(tag) = t.peek() {
            let e = t.next()?;
            if tag != 0xA3 {
                continue;
            }
            let mut exts = Der { b: Der { b: e.value }.expect(0x30, "extensions")?.value };
            while exts.peek().is_some() {
                let ext = exts.expect(0x30, "extension")?;
                let mut x = Der { b: ext.value };
                let oid = x.expect(0x06, "extension id")?;
                let mut val = x.next()?;
                let mut critical = false;
                if val.tag == 0x01 {
                    critical = val.value.first().is_some_and(|&v| v != 0);
                    val = x.next()?;
                }
                if oid.value == OID_KEY_USAGE {
                    // BIT STRING: unused-bit count, then the bits, first bit the most significant
                    let bits = Der { b: val.value }.expect(0x03, "key usage")?;
                    let mut ku = 0u16;
                    for (i, &byte) in bits.value.iter().skip(1).take(2).enumerate() {
                        for b in 0..8 {
                            if byte & (0x80 >> b) != 0 {
                                ku |= 1 << (i * 8 + b);
                            }
                        }
                    }
                    key_usage = Some(ku);
                } else if oid.value == OID_AUTHORITY_KEY_ID {
                    has_aki = true;
                } else if oid.value == OID_BASIC_CONSTRAINTS {
                    has_bc = true;
                    let bc = Der { b: val.value }.expect(0x30, "basic constraints")?;
                    let mut b = Der { b: bc.value };
                    if b.peek() == Some(0x01) {
                        ca = b.next()?.value.first().is_some_and(|&v| v != 0);
                    }
                    if b.peek() == Some(0x02) {
                        let n = b.next()?;
                        path_len = Some(n.value.iter().fold(0u32, |a, &x| a.saturating_mul(256).saturating_add(u32::from(x))));
                    }
                } else if critical && oid.value != OID_SUBJECT_KEY_ID && unknown_critical.is_none() {
                    unknown_critical = Some(dotted(oid.value));
                }
            }
        }
        Ok(Certificate {
            der: cert.whole.to_vec(),
            tbs: tbs.whole.to_vec(),
            signature_algorithm: alg_of(alg.value)?,
            signature: sig.value.get(1..).unwrap_or_default().to_vec(),
            serial: serial.value.to_vec(),
            issuer: issuer.whole.to_vec(),
            subject: subject.whole.to_vec(),
            subject_attributes: attributes(subject.value)?,
            issuer_attributes: attributes(issuer.value)?,
            version,
            inner_signature_algorithm: alg_of(inner_alg.value)?,
            not_before,
            not_after,
            spki: spki.whole.to_vec(),
            public_key_bits: bits.value.get(1..).unwrap_or_default().to_vec(),
            ca,
            path_len,
            has_basic_constraints: has_bc,
            has_authority_key_id: has_aki,
            key_usage,
            unknown_critical,
        })
    }

    /// The first value of attribute `oid` in the subject (`issuer` false) or issuer name.
    pub fn attribute(&self, issuer: bool, oid: &[u8]) -> Option<&str> {
        let attrs = if issuer { &self.issuer_attributes } else { &self.subject_attributes };
        attrs.iter().find(|(o, _)| o == oid).map(|(_, v)| v.as_str())
    }

    fn count(&self, issuer: bool, oid: &[u8]) -> usize {
        let attrs = if issuer { &self.issuer_attributes } else { &self.subject_attributes };
        attrs.iter().filter(|(o, _)| o == oid).count()
    }

    /// The Certificate Thumbprint (ST 430-2 5.4): SHA-1 of the `tbsCertificate` contents (without
    /// its DER tag and length), as the KDM's encrypted key blocks name their signer (ST 430-1).
    pub fn thumbprint(&self) -> [u8; 20] {
        use sha1::Digest as _;
        let body = Der { b: &self.tbs }.next().map(|t| t.value).unwrap_or_default();
        sha1::Sha1::digest(body).into()
    }

    /// The Public Key Thumbprint (ST 430-2 5.4): SHA-1 of the `subjectPublicKey` bits, Base64.
    pub fn public_key_thumbprint(&self) -> String {
        use base64::Engine as _;
        use sha1::Digest as _;
        base64::engine::general_purpose::STANDARD.encode(sha1::Sha1::digest(&self.public_key_bits))
    }

    /// The RSA modulus length in bits and the public exponent (when it fits 64 bits).
    pub fn rsa_size(&self) -> Result<(usize, Option<u64>), String> {
        let seq = Der { b: &self.public_key_bits }.expect(0x30, "RSA public key")?;
        let mut d = Der { b: seq.value };
        let n = d.expect(0x02, "RSA modulus")?;
        let e = d.expect(0x02, "RSA exponent")?;
        let mag: Vec<u8> = n.value.iter().copied().skip_while(|&b| b == 0).collect();
        let bits = mag.first().map_or(0, |&top| mag.len().saturating_sub(1).saturating_mul(8).saturating_add(8 - top.leading_zeros() as usize));
        let emag: Vec<u8> = e.value.iter().copied().skip_while(|&b| b == 0).collect();
        let exp = (emag.len() <= 8).then(|| emag.iter().fold(0u64, |a, &x| a << 8 | u64::from(x)));
        Ok((bits, exp))
    }

    /// The first rule of ST 430-2 6.2 (steps 2 to 8, 10, 11 and 13) this certificate breaks, on
    /// its own; the chain steps (14 to 18) are the caller's.
    pub fn st430_2_problem(&self) -> Option<String> {
        if self.version != 2 {
            return Some("is not an X.509 version 3 certificate".into());
        }
        if let Some(oid) = &self.unknown_critical {
            return Some(format!("has an unknown critical extension {oid}"));
        }
        if !self.has_basic_constraints || !self.has_authority_key_id || self.key_usage.is_none() {
            return Some("lacks BasicConstraints, AuthorityKeyIdentifier or KeyUsage".into());
        }
        if self.ca && self.path_len.is_none() {
            return Some("is a CA without a path length limit".into());
        }
        if !self.ca && self.path_len.is_some_and(|p| p != 0) {
            return Some("is not a CA but has a path length limit".into());
        }
        let ku = self.key_usage.unwrap_or(0);
        if self.ca {
            if ku & KU_KEY_CERT_SIGN == 0 || ku & !(KU_KEY_CERT_SIGN | KU_CRL_SIGN) != 0 {
                return Some("is a CA whose KeyUsage is not keyCertSign (and cRLSign) only".into());
            }
        } else if ku & (KU_KEY_CERT_SIGN | KU_CRL_SIGN) != 0 || ku & (KU_DIGITAL_SIGNATURE | KU_KEY_ENCIPHERMENT) == 0 {
            return Some("is a leaf whose KeyUsage is not digitalSignature / keyEncipherment".into());
        }
        for issuer in [false, true] {
            for (oid, what) in [(OID_DN_QUALIFIER, "dnQualifier"), (OID_O, "OrganizationName"), (OID_CN, "CommonName")] {
                if self.count(issuer, oid) != 1 {
                    return Some(format!("does not have exactly one {what} in its {} name", if issuer { "issuer" } else { "subject" }));
                }
            }
        }
        if self.attribute(false, OID_O) != self.attribute(true, OID_O) {
            return Some("has a subject OrganizationName other than its issuer's".into());
        }
        let cn = self.attribute(false, OID_CN).unwrap_or_default();
        if !self.ca && cn.split_once('.').is_none_or(|(roles, _)| roles.trim().is_empty()) {
            return Some("is a leaf without a role in its CommonName".into());
        }
        if self.signature_algorithm != Some(SigAlg::RsaSha256) || self.inner_signature_algorithm != Some(SigAlg::RsaSha256) {
            return Some("is not signed with sha256WithRSAEncryption".into());
        }
        match self.rsa_size() {
            Ok((2048, Some(65537))) => {}
            Ok((bits, e)) => return Some(format!("has a {bits}-bit RSA key with exponent {e:?}, not 2048 bits and 65537")),
            Err(e) => return Some(e),
        }
        if self.attribute(false, OID_DN_QUALIFIER) != Some(self.public_key_thumbprint().as_str()) {
            return Some("has a dnQualifier that is not its public key thumbprint".into());
        }
        None
    }

    /// The RSA public key.
    pub fn public_key(&self) -> Result<rsa::PublicKey, String> {
        use rsa::pkcs8::DecodePublicKey;
        rsa::PublicKey::from_public_key_der(&self.spki).map_err(|e| format!("certificate public key: {e}"))
    }

    /// The serial number as a decimal string (as XML Signature's `X509SerialNumber` writes it).
    pub fn serial_decimal(&self) -> String {
        // repeated division of the big-endian magnitude by 10
        let mut digits = Vec::new();
        let mut n: Vec<u8> = self.serial.iter().copied().skip_while(|&b| b == 0).collect();
        while !n.is_empty() {
            let mut rem = 0u32;
            let mut q = Vec::with_capacity(n.len());
            for &b in &n {
                let cur = rem * 256 + u32::from(b);
                q.push((cur / 10) as u8);
                rem = cur % 10;
            }
            digits.push(b'0' + rem as u8);
            n = q.into_iter().skip_while(|&b| b == 0).collect();
        }
        if digits.is_empty() {
            digits.push(b'0');
        }
        digits.reverse();
        String::from_utf8(digits).unwrap_or_default()
    }

    /// Whether `issuer`'s key signs this certificate (PKCS#1 v1.5).
    pub fn signed_by(&self, issuer: &Certificate) -> Result<(), String> {
        let alg = self.signature_algorithm.ok_or("unsupported certificate signature algorithm")?;
        verify_rsa(&issuer.public_key()?, alg, &self.tbs, &self.signature)
    }
}

/// An OID in dotted form.
fn dotted(oid: &[u8]) -> String {
    let mut parts = Vec::new();
    if let Some(&first) = oid.first() {
        parts.push(u64::from(first / 40));
        parts.push(u64::from(first % 40));
    }
    let mut v = 0u64;
    for &b in oid.iter().skip(1) {
        v = v.saturating_mul(128).saturating_add(u64::from(b & 0x7F));
        if b & 0x80 == 0 {
            parts.push(v);
            v = 0;
        }
    }
    parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// DigestInfo prefixes of PKCS#1 v1.5 signatures (RFC 8017 9.2 note 1).
const SHA256_PREFIX: &[u8] = &[0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20];
const SHA1_PREFIX: &[u8] = &[0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14];

/// Verifies the RSA PKCS#1 v1.5 signature `sig` of `data`.
pub fn verify_rsa(key: &rsa::PublicKey, alg: SigAlg, data: &[u8], sig: &[u8]) -> Result<(), String> {
    use sha1::Digest as _;
    let mut hashed = Vec::new();
    match alg {
        SigAlg::RsaSha256 => {
            hashed.extend_from_slice(SHA256_PREFIX);
            hashed.extend_from_slice(&rsa::sha2::Sha256::digest(data));
        }
        SigAlg::RsaSha1 => {
            hashed.extend_from_slice(SHA1_PREFIX);
            hashed.extend_from_slice(&sha1::Sha1::digest(data));
        }
    }
    key.verify(rsa::Pkcs1v15Sign::new_unprefixed(), &hashed, sig).map_err(|_| "signature does not verify".to_string())
}
