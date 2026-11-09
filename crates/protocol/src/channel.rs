// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environment state channel: what an environment's server side shares with the clients in a space that present
//! the environment, and what their participants ask of it, each in an envelope naming the environment and its version
//! around a payload that only the environment's own crate reads. It is the first, small step of state synchronization
//! (#76): a kept state for whoever joins, events for whoever is there, and intents that the server side validates.
//! See `design/environments.md`.
use serde::{Deserialize, Serialize};

/// The largest state a server side keeps for a space, as its payload's JSON text, in bytes. A larger one is refused,
/// and the side is told why. `tomori-environment`'s `STATE_LIMIT` is the same, as the server's tests check.
pub const STATE_LIMIT: usize = 64 * 1024;

/// The largest event or intent, as its payload's JSON text, in bytes. A larger intent is dropped, and a larger event
/// refused, and the side told why. `tomori-environment`'s `EVENT_LIMIT` is the same, as the server's tests check.
pub const MESSAGE_LIMIT: usize = 4 * 1024;

/// An environment's own payload: any JSON value, which the channel carries without reading, and the environment's own
/// crate encodes and decodes. Its size is the length of its JSON text, which [`STATE_LIMIT`] and [`MESSAGE_LIMIT`]
/// bound.
///
/// ```
/// use tomori_protocol::Payload;
///
/// let payload = Payload::parse(r#"{"lantern": 3}"#).unwrap();
/// assert_eq!(payload.text(), r#"{"lantern":3}"#);
/// assert_eq!(payload.len(), 13);
/// assert!(Payload::parse("not json").is_err());
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Payload(serde_json::Value);

impl Payload {
    /// The payload `value`.
    pub fn new(value: serde_json::Value) -> Self {
        Self(value)
    }

    /// The payload whose JSON text is `text`, or why it is not JSON.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text).map(Self)
    }

    /// Its value, for the environment's crate to read.
    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }

    /// Its JSON text, as the channel sends it.
    pub fn text(&self) -> String {
        self.0.to_string()
    }

    /// The length of its JSON text, in bytes.
    pub fn len(&self) -> usize {
        self.text().len()
    }

    /// Whether its JSON text is empty, which it never is: even `null` has a length.
    pub fn is_empty(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_is_carried_as_its_value() {
        let payload = Payload::new(serde_json::json!({"lantern": 3, "away": true}));
        assert_eq!(payload.value()["lantern"], 3);
        assert!(!payload.is_empty());
        assert_eq!(payload.len(), payload.text().len());
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json, serde_json::json!({"lantern": 3, "away": true}));
        assert_eq!(serde_json::from_value::<Payload>(json).unwrap(), payload);
        assert_eq!(Payload::parse("null").unwrap().len(), 4);
    }
}
