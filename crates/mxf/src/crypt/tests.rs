//! ST 429-6 tests: triplets made here (AES-128-CBC, HMAC-SHA1) are checked against the openssl
//! command line (an oracle only, skipped when it is missing), and whole track files written by our
//! D-Cinema writer are encrypted triplet by triplet and read back through `open`.

use super::*;
use crate::write::{FrameInfo, MxfWriter, PackageIds, Pattern, PictureDesc, SoundDesc, Umid, WriterConfig};
use crate::{Codec, Rational, TrackKind, open};
use std::io::Cursor;
use std::process::Command;

/// The content and MIC keys, made once per run (no fixed key in the source).
static KEYS: std::sync::LazyLock<([u8; 16], [u8; 16])> = std::sync::LazyLock::new(|| (crate::testing::test_key("content"), crate::testing::test_key("mic")));
fn content_key() -> [u8; 16] {
    KEYS.0
}
fn mic_key() -> [u8; 16] {
    KEYS.1
}
const KEY_ID: [u8; 16] = [0x5A; 16];
const CONTEXT: [u8; 16] = [0xC0; 16];

fn triplet(source: [u8; 16], plain: &[u8], po: usize, seq: u64, mic_key: Option<[u8; 16]>) -> Vec<u8> {
    crate::testing::encrypted_triplet(&content_key(), &CONTEXT, source, plain, po, seq, mic_key)
}

fn encrypt_file(f: &[u8], po: usize, mic: bool) -> Vec<u8> {
    crate::testing::encrypt_track_file(f, &content_key(), &KEY_ID, &CONTEXT, po, mic.then_some(mic_key()))
}

fn ring(mic: bool) {
    add_key(KEY_ID, ContentKey { cipher: content_key(), mic: mic.then_some(mic_key()) });
}

#[test]
fn triplets_round_trip_with_every_plaintext_offset() {
    let plain: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let key = ContentKey { cipher: content_key(), mic: Some(mic_key()) };
    for po in [0, 1, 15, 16, 17, 200, 999, 1000] {
        for len in [po, po + 1, po + 16, 1000.max(po)] {
            let p = &plain[..len.min(1000)];
            if po > p.len() {
                continue;
            }
            let v = triplet([0x06; 16], p, po, 3, Some(mic_key()));
            let t = parse(&v).unwrap();
            assert_eq!((t.plaintext_offset, t.source_length, t.sequence), (po as u64, p.len() as u64, Some(3)));
            assert_eq!(decrypt(&v, &key).unwrap(), p, "offset {po} length {}", p.len());
            // without the MIC key the MIC is not checked
            assert_eq!(decrypt(&v, &ContentKey { cipher: content_key(), mic: None }).unwrap(), p);
        }
    }
}

#[test]
fn wrong_keys_and_damage_are_errors_not_garbage() {
    let plain = vec![0x42u8; 333];
    let v = triplet([0x06; 16], &plain, 20, 1, Some(mic_key()));
    let e = decrypt(&v, &ContentKey { cipher: crate::testing::test_key("wrong"), mic: None }).unwrap_err().to_string();
    assert!(e.contains("Check Value"), "{e}");
    let e = decrypt(&v, &ContentKey { cipher: content_key(), mic: Some(crate::testing::test_key("wrong mic")) }).unwrap_err().to_string();
    assert!(e.contains("integrity"), "{e}");
    // a flipped cipher text bit fails the MIC
    let mut bad = v.clone();
    let n = bad.len();
    bad[n - 60] ^= 1;
    assert!(decrypt(&bad, &ContentKey { cipher: content_key(), mic: Some(mic_key()) }).is_err());
    // truncation and hostile lengths never panic
    for cut in 0..v.len() {
        let _ = decrypt(&v[..cut], &ContentKey { cipher: content_key(), mic: Some(mic_key()) });
    }
    let mut lie = v.clone();
    lie[16 + 4 + 4 + 8 + 4 + 16 + 4..][..8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(decrypt(&lie, &ContentKey { cipher: content_key(), mic: None }).is_err());
}

fn openssl() -> Option<&'static str> {
    let ok = Command::new("openssl").arg("version").output().is_ok_and(|o| o.status.success());
    if !ok {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "openssl not found");
        eprintln!("SKIPPED: openssl not found");
    }
    ok.then_some("openssl")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The cipher blocks and the MIC agree with OpenSSL's AES-128-CBC and HMAC-SHA1.
#[test]
fn openssl_oracle() {
    let Some(ossl) = openssl() else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-crypt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain: Vec<u8> = (0..777u32).map(|i| (i * 13 % 256) as u8).collect();
    let po = 33;
    let v = triplet([0x06; 16], &plain, po, 9, Some(mic_key()));
    let t = parse(&v).unwrap();
    // OpenSSL encrypts Check Value + rest of the source (PKCS#7 padding = RFC 2898 for 16-byte blocks)
    let input = dir.join("in.bin");
    let mut body = CHECK_VALUE.to_vec();
    body.extend_from_slice(&plain[po..]);
    std::fs::write(&input, &body).unwrap();
    let iv = &t.esv[..16];
    let out = Command::new(ossl).args(["enc", "-aes-128-cbc", "-K", &hex(&content_key()), "-iv", &hex(iv), "-in"]).arg(&input).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let ct = out.stdout;
    assert_eq!(&t.esv[16..32], &ct[..16], "encrypted Check Value");
    assert_eq!(&t.esv[32..32 + po], &plain[..po], "plaintext header in the clear");
    assert_eq!(&t.esv[32 + po..], &ct[16..], "cipher blocks");
    // and decrypting OpenSSL's own cipher text gives the source back
    std::fs::write(&input, t.mic_input).unwrap();
    let out = Command::new(ossl).args(["dgst", "-sha1", "-mac", "HMAC", "-macopt"]).arg(format!("hexkey:{}", hex(&mic_key()))).arg(&input).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.trim_end().ends_with(&hex(&t.mic.unwrap())), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

fn j2k_like(i: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F];
    v.extend(std::iter::repeat_n(i as u8 ^ 0x33, 900 + 11 * i));
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

fn picture_file() -> Vec<u8> {
    let mut ids = PackageIds::from_seed("enc-pic", "Reel 1");
    ids.file = Umid::dcinema([0x11; 16]);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.picture = Some(PictureDesc::dcinema(3, 1998, 1080));
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..5 {
        w.push_picture(&j2k_like(i), FrameInfo::intra()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// The key ring is process-wide: the tests that use it run one at a time.
static RING: Mutex<()> = Mutex::new(());

#[test]
fn encrypted_picture_track_file() {
    let _g = RING.lock().unwrap_or_else(PoisonError::into_inner);
    clear_keys();
    let f = encrypt_file(&picture_file(), 16, true);
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let t = &m.tracks[0];
    assert_eq!((t.kind, t.codec, t.samples.len()), (TrackKind::Picture, Codec::Jpeg2000, 5));
    let e = t.encryption.unwrap();
    assert_eq!((e.key_id, e.context_id, e.aes, e.mic), (KEY_ID, CONTEXT, true, true));
    assert!(m.needs_key());
    assert_eq!(m.missing_keys(), vec![KEY_ID]);
    let err = m.read_sample(&f, 0, 0).unwrap_err();
    assert!(matches!(err, Error::Encrypted(_)), "{err}");
    assert!(err.to_string().contains("5a5a5a5a-5a5a"), "{err}");
    ring(true);
    assert!(!m.needs_key());
    assert!(m.missing_keys().is_empty());
    for i in 0..5 {
        assert_eq!(m.read_sample(&f, 0, i).unwrap(), j2k_like(i));
    }
    // a wrong key is refused by the Check Value
    add_key(KEY_ID, ContentKey { cipher: crate::testing::test_key("other"), mic: None });
    assert!(m.read_sample(&f, 0, 0).is_err());
    clear_keys();
    assert!(key_ids().is_empty());
    // truncated encrypted files never panic
    ring(false);
    for cut in (0..f.len()).step_by(89) {
        if let Ok(m) = open(&f[..cut]) {
            for i in 0..m.tracks.first().map_or(0, |t| t.samples.len()) {
                let _ = m.read_sample(&f[..cut], 0, i);
            }
        }
    }
    clear_keys();
}

#[test]
fn encrypted_sound_track_file() {
    let _g = RING.lock().unwrap_or_else(PoisonError::into_inner);
    clear_keys();
    let mut ids = PackageIds::from_seed("enc-snd", "Reel 1");
    ids.file = Umid::dcinema([0x22; 16]);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 6, bits: 24 }];
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let frames = 5 * 2000;
    let sample = |k: usize, c: usize| ((k as i32 * 3 + c as i32 * 100_000) % 8_000_000) as f32 / 8_388_608.0;
    let mut pcm = Vec::new();
    for k in 0..frames {
        for c in 0..6i32 {
            let s = (k * 3 + c * 100_000) % 8_000_000;
            pcm.extend_from_slice(&s.to_le_bytes()[..3]);
        }
    }
    w.push_sound(0, &pcm).unwrap();
    let f = encrypt_file(&w.finish().unwrap().into_inner(), 0, false);
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let t = &m.tracks[0];
    assert_eq!((t.kind, t.stored_sample_frames()), (TrackKind::Sound, frames as u64));
    assert!(matches!(m.read_pcm(&f, 0, 0, 10), Err(Error::Encrypted(_))));
    ring(false);
    // a read across chunk boundaries, starting mid-chunk
    let got = m.read_pcm(&f, 0, 1500, 3000).unwrap();
    for k in (0..3000).step_by(41) {
        for (c, ch) in got.iter().enumerate() {
            assert_eq!(ch[k], sample(1500 + k, c), "frame {k} channel {c}");
        }
    }
    clear_keys();
}

/// ST 429-6 Annex B: the MIC key earlier editions derive from the content key (FIPS 186-2).
#[test]
fn legacy_mic_key_known_answers() {
    let k: [u8; 16] = std::array::from_fn(|i| i as u8);
    assert_eq!(legacy_mic_key(&k), [0xbb, 0x12, 0xf6, 0xb8, 0x4b, 0x2e, 0x7b, 0x25, 0x8c, 0xd8, 0xc3, 0x5e, 0x8f, 0x87, 0xfc, 0x18]);
    // XKEY + x0 + 1 carries through all 160 bits
    assert_eq!(legacy_mic_key(&[0xff; 16]), [0x0d, 0x18, 0xd6, 0x21, 0x06, 0xb6, 0x08, 0x8e, 0x38, 0x4a, 0x9d, 0xf4, 0x74, 0x0a, 0xca, 0x77]);
}

/// A track file whose MIC key is the legacy one: without an MDMK in the KDM the derived key is
/// found from the first triplet, joins the key ring and catches damage in later triplets.
#[test]
fn legacy_mic_key_is_used_without_an_mdmk() {
    let _g = RING.lock().unwrap_or_else(PoisonError::into_inner);
    clear_keys();
    let legacy = legacy_mic_key(&content_key());
    let mut f = crate::testing::encrypt_track_file(&picture_file(), &content_key(), &KEY_ID, &CONTEXT, 16, Some(legacy));
    let m = open(&f).unwrap();
    ring(false);
    assert_eq!(m.read_sample(&f, 0, 0).unwrap(), j2k_like(0));
    assert_eq!(key(&KEY_ID).and_then(|k| k.mic), Some(legacy));
    for i in 0..5 {
        assert_eq!(m.read_sample(&f, 0, i).unwrap(), j2k_like(i));
    }
    // a flipped cipher text bit in a later triplet now fails its MIC instead of decoding garbage
    let s = m.tracks[0].samples[3];
    let at = (s.offset + u64::from(s.size) - 60) as usize;
    f[at] ^= 1;
    let e = m.read_sample(&f, 0, 3).unwrap_err().to_string();
    assert!(e.contains("integrity"), "{e}");
    // a modern file (random MIC key, none in the KDM) still decrypts, unchecked
    clear_keys();
    let f = encrypt_file(&picture_file(), 16, true);
    let m = open(&f).unwrap();
    ring(false);
    assert_eq!(m.read_sample(&f, 0, 2).unwrap(), j2k_like(2));
    assert_eq!(key(&KEY_ID).and_then(|k| k.mic), None);
    clear_keys();
}

#[test]
fn encrypted_timed_text_track_file() {
    let _g = RING.lock().unwrap_or_else(PoisonError::into_inner);
    clear_keys();
    let xml = b"<?xml version=\"1.0\"?><SubtitleReel xmlns=\"http://www.smpte-ra.org/schemas/428-7/2010/DCST\"/>".as_slice();
    let font = b"OTTO fake font bytes, longer than one AES block".as_slice();
    let png = b"\x89PNG fake".as_slice();
    let plain = crate::testing::build_timed_text(xml, &[("application/x-font-opentype", font), ("image/png", png)]);
    for mic in [false, true] {
        let f = encrypt_file(&plain, 0, mic);
        assert_ne!(f, plain);
        assert!(!f.windows(xml.len()).any(|w| w == xml), "the document is not stored in the clear");
        assert!(crate::is_timed_text(&f));
        let tt = crate::open_timed_text(&f).unwrap();
        assert!(tt.warnings.is_empty(), "{:?}", tt.warnings);
        assert_eq!(tt.encryption.map(|e| e.key_id), Some(KEY_ID));
        assert!(tt.document.plain.is_some() && tt.resources.iter().all(|r| r.data.is_some_and(|d| d.plain.is_some())));
        // without the key: the error names the key, never garbage
        assert!(tt.needs_key());
        assert!(matches!(tt.read_document(&f), Err(Error::Encrypted(_))));
        assert!(matches!(tt.read_resource(&f, 0), Err(Error::Encrypted(_))));
        ring(mic);
        assert!(!tt.needs_key());
        assert_eq!(tt.read_document(&f).unwrap(), xml);
        assert_eq!(tt.read_resource(&f, 0).unwrap(), font);
        assert_eq!(tt.read_resource(&f, 1).unwrap(), png);
        // a wrong key fails the Check Value
        add_key(KEY_ID, ContentKey { cipher: crate::testing::test_key("other"), mic: None });
        assert!(tt.read_document(&f).is_err());
        clear_keys();
        // truncated files never panic
        ring(mic);
        for cut in (0..f.len()).step_by(37) {
            if let Ok(tt) = crate::open_timed_text(&f[..cut]) {
                let _ = tt.read_document(&f[..cut]);
                let _ = tt.read_resource(&f[..cut], 0);
            }
        }
        clear_keys();
    }
}
