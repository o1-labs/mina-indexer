//! Keep the exact bytes of Mina strings in a Rust [String].
//!
//! Mina strings are bytes, not text. The protocol checks only the length of a
//! token symbol or a zkApp URI, so a valid block can contain any byte. The
//! daemon writes these bytes to the block JSON with no escape, so a precomputed
//! block is not always valid UTF-8. Mainnet block 565083 sets the token symbol
//! to the single byte `0xff`, and `sok_digest` holds raw bytes in many blocks.
//!
//! [String::from_utf8_lossy] accepts such a block, but it changes the data: the
//! byte `0xff` becomes U+FFFD, which is the 3 bytes `ef bf bd`. A hash of the
//! command then has the wrong input.
//!
//! [decode] and [encode] do not lose data. [decode] writes each byte that is
//! not valid UTF-8 as one private use character, U+F700 + byte. [encode]
//! reverses this:
//!
//! ```text
//! block bytes   ff            61           ef 9c 80 (U+F700)
//! decode        U+F7FF        a            U+F7EF U+F79C U+F780
//! encode        ff            61           ef 9c 80
//! ```
//!
//! A block can also contain real characters from this range. [decode] writes
//! the bytes of those characters in the same way, as the last column shows, so
//! `encode(decode(x)) == x` for all inputs.

use std::borrow::Cow;

/// First character of the private use range that holds raw bytes
const RAW_BYTE_BASE: u32 = 0xF700;

fn raw_byte_char(byte: u8) -> char {
    char::from_u32(RAW_BYTE_BASE + byte as u32).expect("private use character")
}

fn is_raw_byte_char(c: char) -> bool {
    (RAW_BYTE_BASE..RAW_BYTE_BASE + 256).contains(&(c as u32))
}

/// Decodes `bytes` to text with no data loss. See [encode].
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        if !text.chars().any(is_raw_byte_char) {
            return Cow::Borrowed(text);
        }
    }

    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            if is_raw_byte_char(c) {
                let mut buf = [0; 4];
                text.extend(c.encode_utf8(&mut buf).bytes().map(raw_byte_char));
            } else {
                text.push(c);
            }
        }
        text.extend(chunk.invalid().iter().copied().map(raw_byte_char));
    }

    Cow::Owned(text)
}

/// The original bytes of text from [decode]
pub fn encode(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        if is_raw_byte_char(c) {
            bytes.push((c as u32 - RAW_BYTE_BASE) as u8);
        } else {
            let mut buf = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck_macros::quickcheck;

    #[test]
    fn valid_text_is_not_copied() {
        assert!(matches!(decode("MINA é".as_bytes()), Cow::Borrowed("MINA é")));
    }

    #[test]
    fn invalid_byte_is_one_character() {
        let text = decode(&[0xff]);

        assert_eq!(text.chars().count(), 1);
        assert_eq!(encode(&text), [0xff]);
    }

    #[test]
    fn real_character_in_the_range_is_kept() {
        let bytes = "\u{f700}".as_bytes();

        assert_eq!(encode(&decode(bytes)), bytes);
    }

    #[quickcheck]
    fn all_bytes_round_trip(bytes: Vec<u8>) -> bool {
        encode(&decode(&bytes)) == bytes
    }
}
