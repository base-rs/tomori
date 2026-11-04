// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The pairing code, shared by the server and the devices that pair with it. See `design/security.md`.
//!
//! The device commits to a random value before the server sends its own, and reveals it only then, so neither side, nor anyone relaying between them, can choose a value after seeing the other's. Both derive the code from the certificate fingerprint as each saw it and the two values: a relay holding another certificate shows the two ends different codes, except by a one-in-a-million chance.

use sha2::{Digest, Sha256};

use crate::Fingerprint;

/// A random value of one side of a pairing: 256 bits, written as 64 hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Nonce([u8; 32]);

impl Nonce {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("the operating system's random source is available");
        Self(bytes)
    }

    /// Read a nonce written as 64 hexadecimal digits.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.as_bytes();
        if text.len() != 64 {
            return None;
        }
        let digit = |byte: u8| char::from(byte).to_digit(16);
        let mut bytes = [0_u8; 32];
        for (index, pair) in text.as_chunks::<2>().0.iter().enumerate() {
            // Each digit is below 16, so the pair fits a byte.
            bytes[index] = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
        }
        Some(Self(bytes))
    }

    pub fn to_hex(self) -> String {
        crate::hex::encode(&self.0)
    }

    /// The commitment a device sends before it reveals this nonce: its SHA-256, as hexadecimal.
    pub fn commitment(self) -> String {
        crate::hex::encode(&Sha256::digest(self.0))
    }
}

/// Written without its value, which is a secret until revealed.
impl std::fmt::Debug for Nonce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Nonce([redacted])")
    }
}

/// The 6-digit pairing code for a certificate's `fingerprint` and the device's and server's nonces, such as `"042917"`.
pub fn code(fingerprint: Fingerprint, device: Nonce, server: Nonce) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tomori pairing 1");
    digest.update(fingerprint.to_string().as_bytes());
    digest.update(device.0);
    digest.update(server.0);
    let digest = digest.finalize();
    let value = u64::from_be_bytes(digest[..8].try_into().expect("eight bytes"));
    format!("{:06}", value % 1_000_000)
}

/// A code written for reading aloud and comparing: `"042 917"`.
pub fn display(code: &str) -> String {
    match code.len() {
        6 => format!("{} {}", &code[..3], &code[3..]),
        _ => code.to_owned(),
    }
}

/// A code as someone typed it, with spaces removed: six digits, or `None`.
pub fn typed(text: &str) -> Option<String> {
    let digits: String = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_round_trip_and_commit() {
        let nonce = Nonce::generate();
        assert_ne!(nonce, Nonce::generate());
        assert_eq!(Nonce::parse(&nonce.to_hex()), Some(nonce));
        assert_eq!(nonce.commitment().len(), 64);
        assert_ne!(nonce.commitment(), nonce.to_hex());
        // A pair whose second digit is not hexadecimal is refused as well as one whose first is.
        for text in [
            "",
            "abc",
            &"g".repeat(64),
            &"0g".repeat(32),
            &"0".repeat(63),
        ] {
            assert_eq!(Nonce::parse(text), None, "{text}");
        }
        assert!(!format!("{nonce:?}").contains(&nonce.to_hex()));
    }

    #[test]
    fn both_ends_derive_the_same_code_only_from_the_same_certificate() {
        let ours = Fingerprint::of(b"ours");
        let device = Nonce::generate();
        let server = Nonce::generate();
        let code = code(ours, device, server);
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
        assert_eq!(super::code(ours, device, server), code);
        // A relay with another certificate cannot steer the codes to match: whatever server value it chooses for the device, having seen only the commitment, the device's code depends on a value it does not know. Here, a thousand choices all differ from the real server's code but by chance.
        let theirs = Fingerprint::of(b"theirs");
        let matches = (0..1000)
            .filter(|_| super::code(theirs, device, Nonce::generate()) == code)
            .count();
        assert!(matches <= 1, "{matches}");
    }

    #[test]
    fn codes_are_shown_in_two_groups_and_typed_with_or_without_spaces() {
        assert_eq!(display("042917"), "042 917");
        assert_eq!(display("42"), "42");
        assert_eq!(typed(" 042 917 "), Some("042917".to_owned()));
        assert_eq!(typed("042917"), Some("042917".to_owned()));
        assert_eq!(typed("04291"), None);
        assert_eq!(typed("04291x"), None);
        assert_eq!(typed("decline"), None);
    }
}
