use std::panic::catch_unwind;

use sha2::{Digest, Sha512};

use crate::mxf_digest::{MxfDigest, SequenceDigest, mxf_digest, packet_digest};
use crate::{Error, base58, vtfp};

/// A KLV packet with a short-form or long-form BER length.
fn klv(key: [u8; 16], value: &[u8], long: bool) -> Vec<u8> {
    let mut out = key.to_vec();
    if long || value.len() >= 0x80 {
        out.push(0x84);
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    } else {
        out.push(value.len() as u8);
    }
    out.extend_from_slice(value);
    out
}

const HEADER_PARTITION: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x02, 0x04, 0x00];
const FILL: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x02, 0x03, 0x01, 0x02, 0x10, 0x01, 0x00, 0x00, 0x00];
const ESSENCE: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x16, 0x01, 0x01, 0x01];

/// Three packets: a header partition pack, a long-form fill item and a 100 kB essence element
/// (more than one read step).
fn packets() -> Vec<Vec<u8>> {
    let essence: Vec<u8> = (0..100_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    vec![klv(HEADER_PARTITION, &[0u8; 88], false), klv(FILL, &[0u8; 300], true), klv(ESSENCE, &essence, true)]
}

/// The memo's algorithm written out directly: SHA-512 over the SHA-512 of each packet.
fn reference(packets: &[Vec<u8>]) -> [u8; 64] {
    let mut seq = Sha512::new();
    for p in packets {
        seq.update(Sha512::digest(p));
    }
    seq.finalize().into()
}

#[test]
fn digest_of_packets() {
    let p = packets();
    let file = p.concat();
    let (d, n) = mxf_digest(&file[..]).unwrap();
    assert_eq!(n, 3);
    assert_eq!(d.0, reference(&p));
    // The incremental API gives the same value.
    let mut seq = SequenceDigest::new();
    for packet in &p {
        seq.push(&packet_digest(packet));
    }
    assert_eq!(seq.finish(), d);
}

#[test]
fn run_in_is_skipped() {
    let file = packets().concat();
    let (plain, _) = mxf_digest(&file[..]).unwrap();
    let mut with_run_in = vec![0x55u8; 4_000];
    with_run_in.extend_from_slice(&file);
    assert_eq!(mxf_digest(&with_run_in[..]).unwrap().0, plain);
}

#[test]
fn not_mxf_and_truncation() {
    assert!(matches!(mxf_digest(&b"not an MXF file"[..]), Err(Error::NotMxf)));
    assert!(matches!(mxf_digest(&vec![0u8; 70_000][..]), Err(Error::NotMxf)));
    let file = packets().concat();
    for cut in [file.len() - 1, file.len() - 60_000, 16 + 1 + 88 + 10, 16 + 1 + 88 + 17] {
        assert!(matches!(mxf_digest(&file[..cut]), Err(Error::Truncated(_))), "cut {cut}");
    }
    // Reserved BER forms: 0x80 (indefinite) and more than 8 length bytes.
    for bad in [0x80u8, 0x89] {
        let mut f = klv(HEADER_PARTITION, &[0u8; 4], false);
        f.extend_from_slice(&FILL);
        f.push(bad);
        f.extend_from_slice(&[0u8; 16]);
        assert!(matches!(mxf_digest(&f[..]), Err(Error::BadLength(_))), "{bad:#x}");
    }
}

#[test]
fn hostile_input_never_panics() {
    let file = packets().concat();
    let mut seed = 0x9E37_79B9u32;
    for i in 0..400usize {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let mut f = file[..file.len().min(4_000)].to_vec();
        let at = seed as usize % f.len();
        f[at] ^= 1 << (i % 8);
        let cut = (seed as usize / 7) % (f.len() + 1);
        f.truncate(cut);
        let r = catch_unwind(|| mxf_digest(&f[..]));
        assert!(r.is_ok(), "panic at flip {at}, cut {cut}");
    }
    // A length field claiming 2^63 bytes reads to the end and reports truncation, without
    // allocating for it.
    let mut f = klv(HEADER_PARTITION, &[0u8; 4], false);
    f.extend_from_slice(&FILL);
    f.extend_from_slice(&[0x88, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(matches!(mxf_digest(&f[..]), Err(Error::Truncated(_))));
}

#[test]
fn base58_c4_vector() {
    // SMPTE ST 2114 / C4: the C4 ID of the empty string is "c4" + the C4 Base58 form of its SHA-512.
    let empty: [u8; 64] = Sha512::digest(b"").into();
    assert_eq!(base58::encode(&empty), "59dsjfscH38cYeXXYogktxf4Cd9ibshE3BHUo6a58hBXmRQdZrAkZzsWcbWtDg5oQstpDuni4Hirj75GEmTc1sFT");
    assert_eq!(base58::encode(&[0u8; 64]), "1".repeat(88));
    assert_eq!(base58::encode(&[0xFFu8; 64]).len(), 88);
    for v in [empty, [0u8; 64], [0xFFu8; 64]] {
        assert_eq!(base58::decode(&base58::encode(&v)), Some(v));
    }
    // Larger than 2^512, a non-alphabet character, wrong length.
    assert_eq!(base58::decode(&"z".repeat(88)), None);
    assert_eq!(base58::decode(&format!("0{}", "1".repeat(87))), None);
    assert_eq!(base58::decode(&"1".repeat(87)), None);
}

#[test]
fn mxf_digest_urn() {
    let urn = "urn:smpte:mxf-digest:64pMA4dgr8iLqAEkiMpfJv2JHLubLY9wpDUeAcr3pto3gKGsszyCqr9ofBk668EJrVNagTW7WujyYZV9YEUqCRGE";
    let d = MxfDigest::from_urn(urn).unwrap();
    assert_eq!(d.to_urn(), urn);
    assert!(MxfDigest::from_urn("urn:smpte:mxf-digest:64pMA4").is_err());
    assert!(MxfDigest::from_urn(&urn.replace("mxf-digest", "imf-vtfp")).is_err());
}

const CPL: &str = r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016" xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">
 <SegmentList><Segment><SequenceList>
  <cc:MainImageSequence><TrackId>urn:uuid:11111111-2222-3333-4444-555555555555</TrackId><ResourceList>
   <Resource><IntrinsicDuration>100</IntrinsicDuration><TrackFileId>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</TrackFileId></Resource>
  </ResourceList></cc:MainImageSequence>
  <cc:MainAudioSequence><TrackId>urn:uuid:66666666-7777-8888-9999-000000000000</TrackId><ResourceList>
   <Resource><IntrinsicDuration>100</IntrinsicDuration><EntryPoint>10</EntryPoint><TrackFileId>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-ffffffffffff</TrackFileId></Resource>
  </ResourceList></cc:MainAudioSequence>
 </SequenceList></Segment></SegmentList>
</CompositionPlaylist>"#;

#[test]
fn vtfp_defaults_and_encoding() {
    let tracks = vtfp::virtual_tracks(CPL).unwrap();
    assert_eq!(tracks.iter().map(|t| t.kind.as_str()).collect::<Vec<_>>(), ["MainImageSequence", "MainAudioSequence"]);
    // Defaults: EntryPoint 0, SourceDuration = IntrinsicDuration - EntryPoint, RepeatCount 1.
    let mut sha = sha1::Sha1::new();
    sha1::Digest::update(&mut sha, [0xAA, 0xAA, 0xAA, 0xAA, 0xBB, 0xBB, 0xCC, 0xCC, 0xDD, 0xDD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    for v in [10u64, 90, 1] {
        sha1::Digest::update(&mut sha, v.to_be_bytes());
    }
    let want: [u8; 20] = sha1::Digest::finalize(sha).into();
    assert_eq!(vtfp::fingerprint(CPL, "66666666-7777-8888-9999-000000000000").unwrap().0, want);
    assert!(vtfp::fingerprint(CPL, "urn:uuid:99999999-7777-8888-9999-000000000000").is_err());
    assert!(vtfp::fingerprint(CPL, "not-a-uuid").is_err());
}

#[test]
fn vtfp_hostile_input() {
    assert!(matches!(vtfp::virtual_tracks("<!DOCTYPE x [<!ENTITY a \"b\">]><CompositionPlaylist/>"), Err(Error::Xml(_))));
    assert!(matches!(vtfp::virtual_tracks("<Other/>"), Err(Error::Cpl(_))));
    let id = "11111111-2222-3333-4444-555555555555";
    for bad in ["<EntryPoint>-1</EntryPoint>", "<EntryPoint>200</EntryPoint>", "<RepeatCount>x</RepeatCount>", "<LeftEye/>"] {
        let xml = CPL.replacen("<IntrinsicDuration>100</IntrinsicDuration>", &format!("<IntrinsicDuration>100</IntrinsicDuration>{bad}"), 1);
        assert!(matches!(vtfp::fingerprint(&xml, id), Err(Error::Cpl(_))), "{bad}");
    }
    let bytes = CPL.as_bytes();
    for cut in (0..bytes.len()).step_by(7) {
        let Ok(text) = std::str::from_utf8(&bytes[..cut]) else { continue };
        let text = text.to_string();
        assert!(catch_unwind(|| (vtfp::virtual_tracks(&text).is_ok(), vtfp::fingerprint(&text, id).is_ok())).is_ok(), "cut {cut}");
    }
}
