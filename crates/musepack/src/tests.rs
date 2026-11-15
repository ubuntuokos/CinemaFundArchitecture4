use super::*;

#[test]
fn rejects_what_is_not_musepack() {
    assert_eq!(probe(b""), Err(Error::NotMusepack));
    assert_eq!(probe(b"RIFF0000WAVE"), Err(Error::NotMusepack));
    assert_eq!(probe(b"MP+\x06"), Err(Error::Version(6)));
    assert!(probe(b"MPCK").is_err());
}
