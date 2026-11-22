//! The C4 Base58 encoding of a 512-bit value (SMPTE ST 2114:2017 5.1): the value as a big-endian
//! integer in base 58, most significant digit first, padded on the left with `'1'` (zero) to 88
//! digits. 58^88 > 2^512, so 88 digits always hold a SHA-512 value.

/// The C4 Base58 alphabet: digits and letters without `0`, `O`, `I` and `l`.
const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Number of digits of an encoded 512-bit value.
pub const DIGITS: usize = 88;

/// Encode a 64-byte big-endian value as 88 Base58 digits.
pub fn encode(value: &[u8; 64]) -> String {
    // Repeated division of the value by 58; the remainders are the digits, least significant first.
    let mut num = *value;
    let mut out = [ALPHABET[0]; DIGITS];
    for slot in out.iter_mut().rev() {
        let mut rem = 0u32;
        for byte in num.iter_mut() {
            let cur = (rem << 8) | u32::from(*byte);
            *byte = (cur / 58) as u8;
            rem = cur % 58;
        }
        *slot = ALPHABET[rem as usize];
    }
    // All bytes are ASCII from ALPHABET.
    out.iter().map(|&b| char::from(b)).collect()
}

/// Decode exactly 88 Base58 digits into a 64-byte big-endian value; `None` if a character is not a
/// Base58 digit, the length is wrong or the value does not fit in 512 bits.
pub fn decode(text: &str) -> Option<[u8; 64]> {
    if text.len() != DIGITS {
        return None;
    }
    let mut num = [0u8; 64];
    for ch in text.bytes() {
        let digit = ALPHABET.iter().position(|&a| a == ch)? as u32;
        // num = num * 58 + digit
        let mut carry = digit;
        for byte in num.iter_mut().rev() {
            let cur = u32::from(*byte) * 58 + carry;
            *byte = (cur & 0xFF) as u8;
            carry = cur >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(num)
}
