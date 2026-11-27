//! Encrypted track files (SMPTE ST 429-6): the Cryptographic Context of a track, the Encrypted
//! Triplet Variable Length Pack that replaces each essence triplet, its AES-128-CBC decryption with
//! the Check Value test, the optional HMAC-SHA1 message integrity code, and the in-memory key ring
//! the content keys of a KDM (ST 430-1, read by `cfa4-dcp`) are put into.
//!
//! Keys live only in process memory: nothing here writes them anywhere, and [`clear_keys`]
//! overwrites them before dropping.

use crate::klv::ber_length;
use crate::meta::Set;
use crate::{Error, Result, Ul};
use aes::Aes128;
use aes::cipher::{BlockCipherDecrypt, KeyInit};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// Bytes 9..16 of the Encrypted Triplet Key (ST 429-6 Table 10: 06 0E 2B 34 02 04 01 01 0D 01 03 01 02 7E 01 00).
pub const ENCRYPTED_TRIPLET_ITEM: [u8; 8] = [0x0D, 0x01, 0x03, 0x01, 0x02, 0x7E, 0x01, 0x00];
/// The Check Value (ST 429-6 Table 11): "CHUK" four times.
pub const CHECK_VALUE: [u8; 16] = *b"CHUKCHUKCHUKCHUK";
/// Cryptographic Context set key item (ST 429-6 Table 15: 0D 01 04 01 02 02 00 00).
const CONTEXT_SET_ITEM: [u8; 6] = [0x0D, 0x01, 0x04, 0x01, 0x02, 0x02];
/// Cipher Algorithm AES-CBC with a 128-bit key (Table 7: 06 0E 2B 34 04 01 01 07 02 09 02 01 01 00 00 00).
const AES_128_CBC_ITEM: [u8; 5] = [0x02, 0x09, 0x02, 0x01, 0x01];
/// MIC Algorithm HMAC-SHA1 with a 128-bit key (Table 8: ... 02 09 02 02 01 00 00 00).
const HMAC_SHA1_ITEM: [u8; 5] = [0x02, 0x09, 0x02, 0x02, 0x01];
/// Largest encrypted triplet value decrypted in one piece (a 4K J2K frame is far below this).
const MAX_TRIPLET: usize = 512 << 20;

/// Whether `k` is the Encrypted Triplet Key.
pub fn is_encrypted_triplet(k: &Ul) -> bool {
    k.is_smpte() && k.0[4] == 0x02 && k.0[5] == 0x04 && k.0[8..16] == ENCRYPTED_TRIPLET_ITEM
}

/// The Cryptographic Context of an encrypted track (ST 429-6 clause 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encryption {
    /// Context ID: what the triplets' Cryptographic Context Link names.
    pub context_id: [u8; 16],
    /// Cryptographic Key ID: the KeyId of the content key in a KDM.
    pub key_id: [u8; 16],
    /// Essence Container Label of the plaintext essence.
    pub source_container: Option<Ul>,
    /// AES-128-CBC (the only cipher defined); `false` for the "no cipher" value.
    pub aes: bool,
    /// HMAC-SHA1 integrity codes are present.
    pub mic: bool,
}

impl Encryption {
    /// The context of a Cryptographic Context set.
    pub(crate) fn from_set(s: &Set) -> Option<Encryption> {
        if !s.key.item_starts_with(&CONTEXT_SET_ITEM) {
            return None;
        }
        let uuid = |item: &[u8]| -> Option<[u8; 16]> { s.dynamic_item(item)?.get(..16)?.try_into().ok() };
        let ul = |item: &[u8]| -> Option<Ul> { Some(Ul(s.dynamic_item(item)?.get(..16)?.try_into().ok()?)) };
        let key_id = uuid(&[0x02, 0x09, 0x03, 0x01, 0x02])?;
        let context_id = uuid(&[0x01, 0x01, 0x15, 0x11]).unwrap_or([0; 16]);
        let cipher = ul(&[0x02, 0x09, 0x03, 0x01, 0x01]);
        let mic = ul(&[0x02, 0x09, 0x03, 0x02, 0x01]);
        Some(Encryption {
            context_id,
            key_id,
            source_container: ul(&[0x06, 0x01, 0x01, 0x02, 0x02]),
            aes: cipher.is_some_and(|c| c.item_starts_with(&AES_128_CBC_ITEM)),
            mic: mic.is_some_and(|c| c.item_starts_with(&HMAC_SHA1_ITEM)),
        })
    }
}

/// A content key: the AES cipher key and, when the KDM carries one (key type MDMK), the HMAC key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ContentKey {
    pub cipher: [u8; 16],
    pub mic: Option<[u8; 16]>,
}

impl std::fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentKey(..)")
    }
}

static KEYS: Mutex<Option<HashMap<[u8; 16], ContentKey>>> = Mutex::new(None);

/// Puts the content key `key` for Cryptographic Key ID `id` into the key ring (memory only).
pub fn add_key(id: [u8; 16], key: ContentKey) {
    let mut g = KEYS.lock().unwrap_or_else(PoisonError::into_inner);
    let ring = g.get_or_insert_with(HashMap::new);
    match ring.get_mut(&id) {
        // a MIC key arriving separately (MDMK) joins the cipher key already held
        Some(have) if key.cipher == [0; 16] => have.mic = key.mic.or(have.mic),
        Some(have) => *have = ContentKey { cipher: key.cipher, mic: key.mic.or(have.mic) },
        None => {
            ring.insert(id, key);
        }
    }
}

/// The content key of Cryptographic Key ID `id`, if loaded.
pub fn key(id: &[u8; 16]) -> Option<ContentKey> {
    KEYS.lock().unwrap_or_else(PoisonError::into_inner).as_ref()?.get(id).copied().filter(|k| k.cipher != [0; 16])
}

/// The Key IDs in the key ring.
pub fn key_ids() -> Vec<[u8; 16]> {
    let g = KEYS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut v: Vec<[u8; 16]> = g.as_ref().map(|r| r.keys().copied().collect()).unwrap_or_default();
    v.sort_unstable();
    v
}

/// Overwrites and forgets every key.
pub fn clear_keys() {
    let mut g = KEYS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(ring) = g.as_mut() {
        for k in ring.values_mut() {
            *k = ContentKey { cipher: [0; 16], mic: None };
        }
        ring.clear();
    }
}

/// A UUID as text (8-4-4-4-12 lower-case hex).
pub fn uuid_text(u: &[u8; 16]) -> String {
    let h: String = u.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// The items of an Encrypted Triplet value (ST 429-6 Table 9).
#[derive(Clone, Debug)]
pub struct Triplet<'a> {
    pub context_link: [u8; 16],
    pub plaintext_offset: u64,
    pub source_key: Ul,
    pub source_length: u64,
    /// Encrypted Source Value: IV, Check Value, plaintext header, cipher blocks.
    pub esv: &'a [u8],
    pub track_file_id: Option<[u8; 16]>,
    pub sequence: Option<u64>,
    pub mic: Option<[u8; 20]>,
    /// The bytes the MIC covers: from the first byte of the IV to the MIC item's length field.
    pub mic_input: &'a [u8],
}

struct Items<'a> {
    v: &'a [u8],
    at: usize,
}

impl<'a> Items<'a> {
    fn next(&mut self, what: &str) -> Result<&'a [u8]> {
        let rest = self.v.get(self.at..).unwrap_or_default();
        let (len, n) = ber_length(rest).ok_or_else(|| Error::Invalid(format!("encrypted triplet: bad {what} length")))?;
        let start = self.at + n;
        let end = usize::try_from(len).ok().and_then(|l| start.checked_add(l)).filter(|&e| e <= self.v.len());
        let end = end.ok_or_else(|| Error::Invalid(format!("encrypted triplet: {what} runs past the triplet")))?;
        self.at = end;
        Ok(&self.v[start..end])
    }
    fn fixed<const N: usize>(&mut self, what: &str) -> Result<[u8; N]> {
        self.next(what)?.try_into().map_err(|_| Error::Invalid(format!("encrypted triplet: {what} is not {N} bytes")))
    }
    fn optional<const N: usize>(&mut self, what: &str) -> Result<Option<[u8; N]>> {
        if self.at >= self.v.len() {
            return Ok(None);
        }
        let b = self.next(what)?;
        if b.is_empty() {
            return Ok(None);
        }
        b.try_into().map(Some).map_err(|_| Error::Invalid(format!("encrypted triplet: {what} is not {N} bytes")))
    }
}

/// The Source Key and Source Length of an Encrypted Triplet from its first bytes (at least 70).
pub(crate) fn source_of(head: &[u8]) -> Option<(Ul, u64)> {
    let mut it = Items { v: head, at: 0 };
    it.fixed::<16>("context link").ok()?;
    it.fixed::<8>("plaintext offset").ok()?;
    let key = Ul(it.fixed::<16>("source key").ok()?);
    let length = u64::from_be_bytes(it.fixed::<8>("source length").ok()?);
    Some((key, length))
}

/// The MIC key that earlier editions of ST 429-6 derive from the content key (ST 429-6:2023
/// Annex B, NIST FIPS 186-2 Appendix 3.1 with G(t, c) of Appendix 3.3 and no "mod q"): XKEY is
/// the 128-bit key widened to b = 160 bits, x0 = G(XKEY) is discarded, XKEY = (1 + XKEY + x0) mod
/// 2^160, and the MIC key is the first 16 bytes of x1 = G(XKEY). G is the SHA-1 compression of
/// XKEY followed by zero bits, without SHA-1 padding.
pub fn legacy_mic_key(cipher: &[u8; 16]) -> [u8; 16] {
    const H: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let g = |xkey: &[u8; 20]| -> [u8; 20] {
        let mut block = [0u8; 64];
        block[..20].copy_from_slice(xkey);
        let mut h = H;
        sha1::block_api::compress(&mut h, &[block]);
        let mut out = [0u8; 20];
        for (o, w) in out.chunks_exact_mut(4).zip(h) {
            o.copy_from_slice(&w.to_be_bytes());
        }
        out
    };
    let mut xkey = [0u8; 20];
    xkey[..16].copy_from_slice(cipher);
    let x0 = g(&xkey);
    // XKEY = (1 + XKEY + x0) mod 2^160, big-endian
    let mut carry = 1u16;
    for (k, x) in xkey.iter_mut().zip(x0).rev() {
        let sum = u16::from(*k) + u16::from(x) + carry;
        *k = sum as u8;
        carry = sum >> 8;
    }
    let x1 = g(&xkey);
    std::array::from_fn(|i| x1.get(i).copied().unwrap_or_default())
}

/// Whether the MIC item of Encrypted Triplet value `v` is the HMAC-SHA1 of its integrity pack
/// under `mic_key` (false without a MIC item or for a value that does not parse).
pub fn mic_matches(v: &[u8], mic_key: &[u8; 16]) -> bool {
    let Ok(t) = parse(v) else { return false };
    let Some(mic) = t.mic else { return false };
    let Ok(mut m) = Hmac::<Sha1>::new_from_slice(mic_key) else { return false };
    m.update(t.mic_input);
    m.verify_slice(&mic).is_ok()
}

/// Parses an Encrypted Triplet value.
pub fn parse(v: &[u8]) -> Result<Triplet<'_>> {
    let mut it = Items { v, at: 0 };
    let context_link = it.fixed::<16>("context link")?;
    let plaintext_offset = u64::from_be_bytes(it.fixed::<8>("plaintext offset")?);
    let source_key = Ul(it.fixed::<16>("source key")?);
    let source_length = u64::from_be_bytes(it.fixed::<8>("source length")?);
    let esv = it.next("encrypted source value")?;
    let mic_from = it.at - esv.len();
    let track_file_id = it.optional::<16>("track file id")?;
    let sequence = it.optional::<8>("sequence number")?.map(u64::from_be_bytes);
    // the MIC covers its own length field, not its value
    let mic = it.optional::<20>("MIC")?;
    let mic_to = it.at - mic.map_or(0, |m| m.len());
    Ok(Triplet {
        context_link,
        plaintext_offset,
        source_key,
        source_length,
        esv,
        track_file_id,
        sequence,
        mic,
        mic_input: v.get(mic_from..mic_to).unwrap_or_default(),
    })
}

/// The plaintext source value of an Encrypted Triplet value `v` (ST 429-6 11.3.3 and 11.3.4).
/// With a MIC key and a MIC item the HMAC-SHA1 is checked first.
pub fn decrypt(v: &[u8], key: &ContentKey) -> Result<Vec<u8>> {
    if v.len() > MAX_TRIPLET {
        return Err(Error::Unsupported(format!("encrypted triplet of {} bytes", v.len())));
    }
    let t = parse(v)?;
    if let (Some(mic_key), Some(mic)) = (key.mic, t.mic) {
        let mut m = Hmac::<Sha1>::new_from_slice(&mic_key).map_err(|_| Error::Invalid("MIC key".into()))?;
        m.update(t.mic_input);
        if m.verify_slice(&mic).is_err() {
            return Err(Error::Invalid("encrypted triplet fails its integrity check (MIC): damaged file or wrong MIC key".into()));
        }
    }
    let len = usize::try_from(t.source_length).map_err(|_| Error::Invalid("encrypted triplet source length".into()))?;
    let po = usize::try_from(t.plaintext_offset).map_err(|_| Error::Invalid("encrypted triplet plaintext offset".into()))?;
    if po > len {
        return Err(Error::Invalid("encrypted triplet plaintext offset past the source length".into()));
    }
    if po == len {
        // nothing encrypted: IV and Check Value, then the plaintext
        return t.esv.get(32..32 + len).map(<[u8]>::to_vec).ok_or_else(|| Error::Invalid("encrypted triplet shorter than its source".into()));
    }
    let cipher = Aes128::new_from_slice(&key.cipher).map_err(|_| Error::Invalid("content key".into()))?;
    let block = |i: usize| -> Result<[u8; 16]> {
        t.esv.get(i..i + 16).and_then(|b| b.try_into().ok()).ok_or_else(|| Error::Invalid("encrypted triplet shorter than its source".into()))
    };
    let iv = block(0)?;
    let check = block(16)?;
    let mut prev = check;
    let mut c = aes::Block::from(check);
    cipher.decrypt_block(&mut c);
    let plain_check: [u8; 16] = std::array::from_fn(|i| c[i] ^ iv[i]);
    if plain_check != CHECK_VALUE {
        return Err(Error::Invalid("wrong content key for this encrypted track (Check Value mismatch)".into()));
    }
    let header = t.esv.get(32..32 + po).ok_or_else(|| Error::Invalid("encrypted triplet shorter than its plaintext offset".into()))?;
    let body = t.esv.get(32 + po..).unwrap_or_default();
    if body.len() % 16 != 0 || body.len() < len - po {
        return Err(Error::Invalid("encrypted triplet cipher text is not whole AES blocks of the source".into()));
    }
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(header);
    for chunk in body.chunks_exact(16) {
        let ct: [u8; 16] = chunk.try_into().unwrap_or([0; 16]);
        let mut b = aes::Block::from(ct);
        cipher.decrypt_block(&mut b);
        out.extend((0..16).map(|i| b[i] ^ prev[i]));
        prev = ct;
        if out.len() >= len {
            break;
        }
    }
    out.truncate(len);
    Ok(out)
}

#[cfg(test)]
mod tests;
