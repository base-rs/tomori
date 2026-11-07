// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What names an asset: the environment it belongs to, at its protocol version, a name within it, and the SHA-256 of
//! its bytes, which is what the asset is. Two references with the same digest are the same asset, wherever its bytes
//! came from, so a library shares what it loaded across instances and environments, and bytes that do not match the
//! digest are never used.
use std::fmt;

use sha2::{Digest as _, Sha256};

/// The SHA-256 of an asset's bytes.
///
/// ```
/// use tomori_assets::Digest;
///
/// let digest = Digest::of(b"chicken");
/// assert_eq!(Digest::parse(&digest.to_string()), Some(digest));
/// assert_eq!(Digest::parse("not hex"), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The digest written as 64 hexadecimal digits, in either case, or `None` if `hex` is not one.
    pub fn parse(hex: &str) -> Option<Self> {
        if hex.len() != 64 || !hex.is_ascii() {
            return None;
        }
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
            let pair = std::str::from_utf8(pair).unwrap_or_default();
            *byte = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(bytes))
    }

    /// Its bytes.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Digest({self})")
    }
}

/// An asset of an environment: the environment's identifier and protocol version, the asset's name within it, which is
/// also its path beneath the environment's folder where assets are bundled, and its digest, which is what it is. A
/// reference is made in code, beside the asset, so that an asset changes only with the code that expects it: changing
/// the file without its digest fails to load it.
///
/// ```
/// use tomori_assets::{AssetRef, Digest};
///
/// let chicken = AssetRef::new("farmyard", 1, "chicken.glb", Digest::of(b"chicken"));
/// assert_eq!(chicken.environment(), "farmyard");
/// assert_eq!(chicken.version(), 1);
/// assert_eq!(chicken.name(), "chicken.glb");
/// assert_eq!(chicken.path(), "farmyard/chicken.glb");
/// assert_eq!(chicken.digest(), Digest::of(b"chicken"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AssetRef {
    environment: String,
    version: u32,
    name: String,
    digest: Digest,
}

impl AssetRef {
    /// The asset `name` of the environment `environment` at `version`, whose bytes have `digest`.
    pub fn new(
        environment: impl Into<String>,
        version: u32,
        name: impl Into<String>,
        digest: Digest,
    ) -> Self {
        Self {
            environment: environment.into(),
            version,
            name: name.into(),
            digest,
        }
    }

    /// The environment it belongs to.
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// The environment's protocol version it belongs to.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Its name within the environment.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The digest of its bytes.
    pub fn digest(&self) -> Digest {
        self.digest
    }

    /// Where it is bundled, relative to a folder of assets: `ENVIRONMENT/NAME`.
    pub fn path(&self) -> String {
        format!("{}/{}", self.environment, self.name)
    }

    /// Whether `bytes` are this asset.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        Digest::of(bytes) == self.digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_is_written_in_hexadecimal() {
        let digest = Digest::of(b"");
        assert_eq!(
            digest.to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(format!("{digest:?}"), format!("Digest({digest})"));
        assert_eq!(
            Digest::parse(&digest.to_string().to_uppercase()),
            Some(digest)
        );
        assert_eq!(digest.bytes()[0], 0xe3);
        assert_eq!(Digest::parse(&"g".repeat(64)), None);
        assert_eq!(Digest::parse(&"é".repeat(32)), None);
        assert_eq!(Digest::parse("00"), None);
    }

    #[test]
    fn a_reference_matches_only_its_bytes() {
        let chicken = AssetRef::new("farmyard", 1, "chicken.glb", Digest::of(b"chicken"));
        assert_eq!(
            (chicken.environment(), chicken.version(), chicken.name()),
            ("farmyard", 1, "chicken.glb")
        );
        assert!(chicken.matches(b"chicken"));
        assert!(!chicken.matches(b"duck"));
    }
}
