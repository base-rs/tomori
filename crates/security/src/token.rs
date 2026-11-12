// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Every token the server issues starts with this, so that a token is recognizable, such as by secret scanners.
pub const TOKEN_PREFIX: &str = "tomori_";

/// The random part of a token: 256 bits.
const TOKEN_BYTES: usize = 32;

/// A new random bearer token. See `design/security.md`.
pub fn generate() -> String {
    format!("{TOKEN_PREFIX}{}", crate::hex::random(TOKEN_BYTES))
}

/// The hash under which a token is stored, so that the store never holds a token in the clear.
#[cfg(any(feature = "issue", test))]
pub(crate) fn hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    crate::hex::encode(&Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_prefixed_random_and_long() {
        let token = generate();
        assert!(token.starts_with(TOKEN_PREFIX));
        assert_eq!(token.len(), TOKEN_PREFIX.len() + TOKEN_BYTES * 2);
        assert_ne!(token, generate());
    }

    #[test]
    fn the_hash_is_sha256() {
        assert_eq!(
            hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
