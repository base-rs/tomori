// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Lowercase hexadecimal.
pub(crate) fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// `bytes.len()` random bytes, as hexadecimal, from the operating system's random source.
pub(crate) fn random(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer).expect("the operating system's random source is available");
    encode(&buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_lowercase_pairs() {
        assert_eq!(encode(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }

    #[test]
    fn random_values_differ() {
        let first = random(16);
        assert_eq!(first.len(), 32);
        assert_ne!(first, random(16));
    }
}
