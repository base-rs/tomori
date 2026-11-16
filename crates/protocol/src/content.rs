// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The content store's side of the protocol: the kinds of content a server serves by digest, from a fixed allow-list,
//! where a client asks for it, and the environments' assets a server offers a client. A client reads content with
//! `GET /content/DIGEST` beside its connection, presenting its credential and the session ticket that
//! [`crate::ServerMessage::Content`] gave it, and checks what arrives against the digest before using it. See
//! `design/content.md`.
//!
//! ```
//! use tomori_protocol::ContentKind;
//!
//! assert_eq!(ContentKind::Model.content_type(), "model/gltf-binary");
//! assert_eq!(ContentKind::from_content_type("audio/ogg"), Some(ContentKind::Sound));
//! assert_eq!(ContentKind::from_content_type("text/html"), None);
//! assert!(ContentKind::Image.recognises(b"\x89PNG\r\n\x1a\n...."));
//! assert!(!ContentKind::Image.recognises(b"<svg>"));
//! ```
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The path content is read from, followed by its digest: 64 lower-case hexadecimal digits of its SHA-256.
pub const CONTENT_PATH: &str = "/content/";

/// The request field that carries a connection's session ticket on a content request.
pub const SESSION_FIELD: &str = "tomori-session";

/// The most assets a server offers one client, and that a client reads from one offer.
pub const ASSET_LIMIT: usize = 256;

/// The longest licence notice an offered asset carries, in bytes.
pub const LICENCE_BYTES: usize = 512;

/// What a piece of content is, from the fixed list a server serves and a client decodes: nothing else is ever stored,
/// served or read, and none of these is anything a browser runs. Each has its `Content-Type`, which decides how a
/// client decodes it, the most bytes it may be, and the signature its bytes must start with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContentKind {
    /// A glTF 2.0 binary model, read as `tomori-assets` reads every `.glb`, within its limits.
    Model,
    /// A PNG image, such as a texture.
    Image,
    /// An Ogg Vorbis sound, such as one of an ambience's.
    Sound,
}

impl ContentKind {
    /// Every kind, in the order they were added.
    pub const EVERY: [Self; 3] = [Self::Model, Self::Image, Self::Sound];

    /// Its `Content-Type`, which a server serves it with and a client checks.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Model => "model/gltf-binary",
            Self::Image => "image/png",
            Self::Sound => "audio/ogg",
        }
    }

    /// The kind whose `Content-Type` is exactly `content_type`, if one is.
    pub fn from_content_type(content_type: &str) -> Option<Self> {
        Self::EVERY
            .into_iter()
            .find(|kind| kind.content_type() == content_type)
    }

    /// The most bytes one piece of it may be: a model as `tomori-assets` reads one by default, an image as large as an
    /// image panel's, and a sound as large as an ambience plays.
    pub fn most_bytes(self) -> u64 {
        match self {
            Self::Model => 16 << 20,
            Self::Image => 8 << 20,
            Self::Sound => 1 << 20,
        }
    }

    /// Whether `bytes` start as this kind's must: `glTF`, PNG's signature or `OggS`. Only a check, never what decides
    /// the kind, which is declared.
    pub fn recognises(self, bytes: &[u8]) -> bool {
        let signature: &[u8] = match self {
            Self::Model => b"glTF",
            Self::Image => b"\x89PNG\r\n\x1a\n",
            Self::Sound => b"OggS",
        };
        bytes.starts_with(signature)
    }
}

impl Serialize for ContentKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.content_type())
    }
}

impl<'de> Deserialize<'de> for ContentKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let content_type = String::deserialize(deserializer)?;
        Self::from_content_type(&content_type).ok_or_else(|| {
            serde::de::Error::custom(format!("{content_type:?} is not a content type read"))
        })
    }
}

/// One of an environment's assets that the server serves by digest: the environment and version it belongs to, its
/// name within the environment, as `tomori-assets`' `AssetRef` names it, its digest, length and kind, and the licence it
/// is shared under, if it says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferedAsset {
    pub environment: String,
    pub version: u32,
    pub name: String,
    /// The SHA-256 of its bytes, as 64 lower-case hexadecimal digits.
    pub digest: String,
    pub length: u64,
    #[serde(rename = "type")]
    pub kind: ContentKind,
    /// Who made it and under what licence, such as "by Hinoki Maple Williams, CC BY 4.0", at most [`LICENCE_BYTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
}

/// The assets of an offer that can be read, at most [`ASSET_LIMIT`]: one of a kind this build does not read, or
/// otherwise unreadable, is skipped, as a newer server may offer kinds an older client does not know.
pub(crate) fn readable_assets<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<OfferedAsset>, D::Error> {
    let entries = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            OfferedAsset::deserialize(entry)
                .inspect_err(|error| {
                    log::debug!("Skipped an offered asset that could not be read: {error}.")
                })
                .ok()
        })
        .take(ASSET_LIMIT)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_has_its_type_its_bound_and_its_signature() {
        for kind in ContentKind::EVERY {
            assert_eq!(
                ContentKind::from_content_type(kind.content_type()),
                Some(kind)
            );
            assert!(kind.most_bytes() <= 16 << 20);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("{:?}", kind.content_type()));
            assert_eq!(serde_json::from_str::<ContentKind>(&json).unwrap(), kind);
        }
        assert!(ContentKind::Model.recognises(b"glTF\x02\0\0\0"));
        assert!(ContentKind::Sound.recognises(b"OggS\0"));
        assert!(!ContentKind::Sound.recognises(b"RIFF"));
        // Nothing a browser renders as a document, nor a type with parameters, is a kind.
        assert!(serde_json::from_str::<ContentKind>("7").is_err());
        for refused in ["text/html", "image/svg+xml", "image/png; charset=utf-8", ""] {
            assert_eq!(ContentKind::from_content_type(refused), None);
            assert!(serde_json::from_str::<ContentKind>(&format!("{refused:?}")).is_err());
        }
    }

    #[test]
    fn an_offer_skips_what_it_cannot_read_and_keeps_at_most_its_limit() {
        log::set_max_level(log::LevelFilter::Trace);
        let asset = OfferedAsset {
            environment: "garden".to_owned(),
            version: 1,
            name: "lantern.glb".to_owned(),
            digest: "0".repeat(64),
            length: 12,
            kind: ContentKind::Model,
            licence: None,
        };
        let json = serde_json::to_value(&asset).unwrap();
        assert_eq!(json["type"], "model/gltf-binary");
        assert!(json.get("licence").is_none());
        let mut unknown = json.clone();
        unknown["type"] = "video/mp4".into();
        // Nor one whose type is not even text, read from a value rather than from text as the test above reads one.
        let mut untyped = json.clone();
        untyped["type"] = 7.into();
        let entries = serde_json::json!([json, unknown, untyped, 7]);
        let read = readable_assets(entries).unwrap();
        assert_eq!(read, std::slice::from_ref(&asset));
        let many = serde_json::Value::Array(vec![json; ASSET_LIMIT + 3]);
        assert_eq!(readable_assets(many).unwrap().len(), ASSET_LIMIT);
        assert!(readable_assets(serde_json::json!("not a list")).is_err());
    }
}
