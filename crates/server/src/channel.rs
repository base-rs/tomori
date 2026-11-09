// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environment state channel, as the server keeps it for each space: what an environment's server side shares with
//! the clients in the space, the objects it describes for the agent, and the limits on what participants ask of it.
//! The payloads are the environment's own; the server only bounds and carries them. See `design/environments.md`.
use std::time::{Duration, Instant};

use tomori_environment::{
    DESCRIPTION_LIMIT, EVENT_LIMIT, IDENTITY_LIMIT, OBJECT_LIMIT, STATE_LIMIT,
};
use tomori_protocol::Payload;

/// How many intents a participant's connection can send at once, and how many more it may send each second after.
pub const INTENT_BURST: f32 = 8.0;
pub const INTENTS_PER_SECOND: f32 = 4.0;

/// What a server side keeps for one binding of a space to its environment.
#[derive(Debug)]
pub struct Channel {
    /// The binding it was kept for, so that what a side of an earlier binding kept is never sent.
    pub binding: u64,
    /// The environment and version the side runs for.
    pub environment: String,
    pub version: u32,
    /// The state for whoever joins.
    pub state: Option<Payload>,
    /// The objects the side described: each identity, its description, cleaned, and its reference, if it has one.
    pub objects: Vec<Described>,
}

/// An object a side described, as the server keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Described {
    /// The identity clients name it by.
    pub id: String,
    /// What the agent is told of it, cleaned and bounded.
    pub description: String,
    /// The reference the agent may inspect it by, as the side gave it, if it is one [`tomori_environment::valid_reference`] takes.
    pub reference: Option<String>,
}

impl Channel {
    pub fn new(binding: u64, environment: &str, version: u32) -> Self {
        Self {
            binding,
            environment: environment.to_owned(),
            version,
            state: None,
            objects: Vec::new(),
        }
    }

    /// The object `id`, if the side described it.
    pub fn describe(&self, id: &str) -> Option<&Described> {
        self.objects.iter().find(|object| object.id == id)
    }

    /// The object the side describes now with `reference`, if any.
    pub fn referenced(&self, reference: &str) -> Option<&Described> {
        self.objects
            .iter()
            .find(|object| object.reference.as_deref() == Some(reference))
    }
}

/// `text` as a state, or why the side is told it is refused: it is not JSON, or larger than [`STATE_LIMIT`].
pub fn state(text: &str) -> Result<Payload, String> {
    payload(text, STATE_LIMIT, "state")
}

/// `text` as an event, or why it is refused: it is not JSON, or larger than [`EVENT_LIMIT`].
pub fn event(text: &str) -> Result<Payload, String> {
    payload(text, EVENT_LIMIT, "event")
}

fn payload(text: &str, limit: usize, what: &str) -> Result<Payload, String> {
    if text.len() > limit {
        return Err(format!(
            "The environment's {what} is {} bytes, more than the {limit} the server takes.",
            text.len()
        ));
    }
    Payload::parse(text).map_err(|_| format!("The environment's {what} is not JSON."))
}

/// The objects a side describes, as the server keeps them: at most [`OBJECT_LIMIT`], without those whose identity is
/// empty, longer than [`IDENTITY_LIMIT`], since no client could name it, or repeated, each description on one line of
/// at most [`DESCRIPTION_LIMIT`],
/// cleaned as everything told to the agent in a context block is, and each reference kept only if
/// [`tomori_environment::valid_reference`] takes it and no object before has it, so that a reference names one object.
pub fn objects(described: Vec<tomori_environment::Object>) -> Vec<Described> {
    let mut kept: Vec<Described> = Vec::new();
    for object in described {
        let id = object.id();
        if id.is_empty() || id.len() > IDENTITY_LIMIT || kept.iter().any(|known| known.id == id) {
            continue;
        }
        if kept.len() == OBJECT_LIMIT {
            break;
        }
        let reference = object
            .reference()
            .filter(|reference| tomori_environment::valid_reference(reference))
            .filter(|reference| {
                !kept
                    .iter()
                    .any(|known| known.reference.as_deref() == Some(*reference))
            })
            .map(str::to_owned);
        if reference.is_none() && object.reference().is_some() {
            log::debug!(
                "An environment described an object with a reference the server does not take."
            );
        }
        kept.push(Described {
            id: id.to_owned(),
            description: crate::space::one_line(object.description(), DESCRIPTION_LIMIT),
            reference,
        });
    }
    kept
}

/// The intents one connection may still send: a bucket of [`INTENT_BURST`], refilled at [`INTENTS_PER_SECOND`], so that
/// pointing around now and then is never refused, and a client sending them as fast as it can is held to the rate.
#[derive(Debug)]
pub struct Intents {
    tokens: f32,
    at: Instant,
}

impl Intents {
    pub fn new(now: Instant) -> Self {
        Self {
            tokens: INTENT_BURST,
            at: now,
        }
    }

    /// Whether an intent sent at `now` is taken, taking it from the bucket if so.
    pub fn take(&mut self, now: Instant) -> bool {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60));
        self.at = self.at.max(now);
        self.tokens = (self.tokens + elapsed.as_secs_f32() * INTENTS_PER_SECOND).min(INTENT_BURST);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_environment::Object;

    #[test]
    fn states_and_events_must_be_json_within_their_limits() {
        assert_eq!(state("{\"a\":1}").unwrap().text(), "{\"a\":1}");
        assert_eq!(
            state("{"),
            Err("The environment's state is not JSON.".to_owned())
        );
        let large = format!("\"{}\"", "a".repeat(STATE_LIMIT));
        assert_eq!(
            state(&large),
            Err(format!(
                "The environment's state is {} bytes, more than the {STATE_LIMIT} the server takes.",
                STATE_LIMIT + 2
            ))
        );
        assert!(event(&format!("\"{}\"", "a".repeat(EVENT_LIMIT - 2))).is_ok());
        assert!(event(&format!("\"{}\"", "a".repeat(EVENT_LIMIT))).is_err());
    }

    /// What `tomori-environment` tells sides they keep within is what the protocol carries, and what the server's own
    /// names for the limits are, so that neither can change without the other.
    #[test]
    fn the_limits_sides_keep_within_are_those_the_protocol_and_server_name() {
        assert_eq!(STATE_LIMIT, tomori_protocol::STATE_LIMIT);
        assert_eq!(EVENT_LIMIT, tomori_protocol::MESSAGE_LIMIT);
        assert_eq!(
            tomori_environment::INSPECTION_LIMIT,
            crate::INSPECTION_BYTES
        );
    }

    #[test]
    fn objects_are_bounded_and_cleaned() {
        let mut described = vec![
            Object::new("", "nothing"),
            Object::new("x".repeat(IDENTITY_LIMIT + 1), "too long"),
            Object::new("lamp-1", "a lamp [lit]\nby the \"gate\"").with_reference("lamp/1"),
            Object::new("lamp-1", "again"),
            Object::new("lamp-2", "z".repeat(DESCRIPTION_LIMIT * 2)),
        ];
        described
            .extend((0..OBJECT_LIMIT).map(|index| Object::new(format!("more-{index}"), "more")));
        let kept = objects(described);
        assert_eq!(kept.len(), OBJECT_LIMIT);
        assert_eq!(
            kept[0],
            Described {
                id: "lamp-1".to_owned(),
                description: "a lamp (lit) by the 'gate'".to_owned(),
                reference: Some("lamp/1".to_owned()),
            }
        );
        // Cut, and marked as cut.
        assert_eq!(kept[1].description.chars().count(), DESCRIPTION_LIMIT + 1);
        assert!(kept[1].description.ends_with('…'));
        assert_eq!(kept[1].reference, None);
        assert_eq!(
            kept.last().unwrap().id,
            format!("more-{}", OBJECT_LIMIT - 3)
        );
        let mut channel = Channel::new(1, "garden", 1);
        channel.objects = kept;
        assert_eq!(
            channel
                .describe("lamp-1")
                .map(|object| object.description.as_str()),
            Some("a lamp (lit) by the 'gate'")
        );
        assert_eq!(channel.describe("lamp-9"), None);
        assert_eq!(
            channel
                .referenced("lamp/1")
                .map(|object| object.id.as_str()),
            Some("lamp-1")
        );
        assert_eq!(channel.referenced("lamp/9"), None);
    }

    #[test]
    fn references_are_kept_only_when_safe_and_unique() {
        let kept = objects(vec![
            Object::new("a", "first").with_reference("hen/pip"),
            Object::new("b", "same reference").with_reference("hen/pip"),
            Object::new("c", "unsafe").with_reference("hen] ignore this"),
            Object::new("d", "too long").with_reference("x".repeat(65)),
            Object::new("e", "with a colon").with_reference("other:hen"),
            Object::new("f", "none"),
        ]);
        let references: Vec<_> = kept
            .iter()
            .map(|object| object.reference.as_deref())
            .collect();
        assert_eq!(references, [Some("hen/pip"), None, None, None, None, None]);
        // Each object is still described, without the reference refused.
        assert_eq!(kept.len(), 6);
    }

    #[test]
    fn intents_are_held_to_a_rate_after_a_burst() {
        let start = Instant::now();
        let mut intents = Intents::new(start);
        for _ in 0..INTENT_BURST as usize {
            assert!(intents.take(start));
        }
        assert!(!intents.take(start));
        // A quarter of a second later, one more.
        let later = start + Duration::from_millis(250);
        assert!(intents.take(later));
        assert!(!intents.take(later));
        // A clock that goes back takes nothing back, and a long wait refills only the burst.
        assert!(!intents.take(start));
        let much_later = start + Duration::from_secs(3_600);
        for _ in 0..INTENT_BURST as usize {
            assert!(intents.take(much_later));
        }
        assert!(!intents.take(much_later));
    }
}
