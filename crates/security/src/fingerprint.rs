// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

/// The SHA-256 of a certificate in DER form, by which a client pins a server's self-signed certificate. Written as 64 lowercase hexadecimal digits, as in the server's Bonjour TXT record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub fn of(certificate: &[u8]) -> Self {
        Self(Sha256::digest(certificate).into())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&crate::hex::encode(&self.0))
    }
}

/// Text that is not 64 hexadecimal digits.
#[derive(Debug, PartialEq, Eq)]
pub struct FingerprintError;

impl fmt::Display for FingerprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a fingerprint is 64 hexadecimal digits")
    }
}

impl std::error::Error for FingerprintError {}

impl FromStr for Fingerprint {
    type Err = FingerprintError;

    fn from_str(text: &str) -> Result<Self, FingerprintError> {
        let text = text.as_bytes();
        if text.len() != 64 {
            return Err(FingerprintError);
        }
        let digit = |byte: u8| char::from(byte).to_digit(16).ok_or(FingerprintError);
        let mut bytes = [0_u8; 32];
        for (index, pair) in text.as_chunks::<2>().0.iter().enumerate() {
            // Each digit is below 16, so the pair fits a byte.
            bytes[index] = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
        }
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_text() {
        let fingerprint = Fingerprint::of(b"certificate");
        let text = fingerprint.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<Fingerprint>(), Ok(fingerprint));
        assert_eq!(text.to_uppercase().parse::<Fingerprint>(), Ok(fingerprint));
    }

    #[test]
    fn rejects_other_text() {
        for text in ["", "abc", &"g".repeat(64), &"0".repeat(63), &"0".repeat(65)] {
            assert_eq!(text.parse::<Fingerprint>(), Err(FingerprintError), "{text}");
        }
        assert_eq!(
            FingerprintError.to_string(),
            "a fingerprint is 64 hexadecimal digits"
        );
    }
}
