use super::*;

#[test]
fn rejects_what_is_not_musepack() {
    assert_eq!(probe(b""), Err(Error::NotMusepack));
    assert_eq!(probe(b"RIFF0000WAVE"), Err(Error::NotMusepack));
    assert_eq!(probe(b"MP+\x06"), Err(Error::Version(6)));
    assert!(probe(b"MPCK").is_err());
}

/// Fuzz finding (cfa4-fuzz `musepack`, 2026-11-21): a 64-byte SV7 file announcing ~755 million
/// samples made `decode_file` reserve 3 GB before decoding anything.
#[test]
fn a_huge_announced_length_reserves_little() {
    let data: &[u8] = b"MP+\xf7\x0e\x00\x0a\x00\xb7\xbb3\xde\xde\xff#\xff\xff\xff\xff\xff\xff\xffID6\x00\x00MP+\xf7\x0e\x00\x00\x00\x00\xdf\xde\xde\xde\xff#\xff\xff\xff\xff\xff\xffID6\x00\x00\xe0M\x10\xff+\xf7\x1a\x00\xfe\xff\xff";
    assert_eq!(data.len(), 64);
    let (info, planes) = decode_file(data).expect("decodes as far as it can");
    {
        assert!(info.samples > MAX_RESERVE, "the sample is meant to announce a huge length");
        for p in &planes {
            assert!(p.capacity() as u64 <= MAX_RESERVE.max(p.len() as u64), "reserved {} samples", p.capacity());
        }
    }
}
