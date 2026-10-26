use super::*;

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(decode(&[]).is_err());
    assert!(decode(&[0xFF, 0x4F, 0xFF, 0x51, 0, 2]).is_err());
    assert!(decode(b"not an image").is_err());
}
