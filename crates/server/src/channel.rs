// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environment state channel, as the server keeps it for each space: what an environment's server side shares with
//! the clients in the space, the objects it describes for the agent, and the limits on what participants ask of it.
//! The payloads are the environment's own; the server only bounds and carries them. See `design/environments.md`.
use std::time::{Duration, Instant};

use tomori_protocol::{MESSAGE_LIMIT, Payload, STATE_LIMIT};

/// The most objects a side describes for a space; those after are left out.
pub const OBJECTS: usize = 32;

/// The longest identity of an object, in bytes; a longer one is left out, since no client could name it.
pub const OBJECT_BYTES: usize = 128;

/// The longest description of an object told to the agent, in characters, before the `…` that marks one cut.
pub const DESCRIPTION_CHARS: usize = 200;

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
    /// The objects the side described: each identity and its description, cleaned.
    pub objects: Vec<(String, String)>,
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

    /// The description of the object `id`, if the side described it.
    pub fn describe(&self, id: &str) -> Option<&str> {
        self.objects
            .iter()
            .find(|(object, _)| object == id)
            .map(|(_, description)| description.as_str())
    }
}

/// `text` as a state, or why the side is told it is refused: it is not JSON, or larger than [`STATE_LIMIT`].
pub fn state(text: &str) -> Result<Payload, String> {
    payload(text, STATE_LIMIT, "state")
}

/// `text` as an event, or why it is refused: it is not JSON, or larger than [`MESSAGE_LIMIT`].
pub fn event(text: &str) -> Result<Payload, String> {
    payload(text, MESSAGE_LIMIT, "event")
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

/// The objects a side describes, as the server keeps them: at most [`OBJECTS`], without those whose identity is
/// empty, longer than [`OBJECT_BYTES`] or repeated, each description on one line of at most [`DESCRIPTION_CHARS`],
/// cleaned as everything told to the agent in a context block is.
pub fn objects(described: Vec<tomori_environment::Object>) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for object in described {
        let id = object.id();
        if id.is_empty() || id.len() > OBJECT_BYTES || kept.iter().any(|(known, _)| known == id) {
            continue;
        }
        if kept.len() == OBJECTS {
            break;
        }
        kept.push((
            id.to_owned(),
            crate::space::one_line(object.description(), DESCRIPTION_CHARS),
        ));
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
        assert!(event(&format!("\"{}\"", "a".repeat(MESSAGE_LIMIT - 2))).is_ok());
        assert!(event(&format!("\"{}\"", "a".repeat(MESSAGE_LIMIT))).is_err());
    }

    #[test]
    fn objects_are_bounded_and_cleaned() {
        let mut described = vec![
            Object::new("", "nothing"),
            Object::new("x".repeat(OBJECT_BYTES + 1), "too long"),
            Object::new("lamp-1", "a lamp [lit]\nby the \"gate\""),
            Object::new("lamp-1", "again"),
            Object::new("lamp-2", "z".repeat(DESCRIPTION_CHARS * 2)),
        ];
        described.extend((0..OBJECTS).map(|index| Object::new(format!("more-{index}"), "more")));
        let kept = objects(described);
        assert_eq!(kept.len(), OBJECTS);
        assert_eq!(
            kept[0],
            ("lamp-1".to_owned(), "a lamp (lit) by the 'gate'".to_owned())
        );
        // Cut, and marked as cut.
        assert_eq!(kept[1].1.chars().count(), DESCRIPTION_CHARS + 1);
        assert!(kept[1].1.ends_with('…'));
        assert_eq!(kept.last().unwrap().0, format!("more-{}", OBJECTS - 3));
        let mut channel = Channel::new(1, "farmyard", 1);
        channel.objects = kept;
        assert_eq!(
            channel.describe("lamp-1"),
            Some("a lamp (lit) by the 'gate'")
        );
        assert_eq!(channel.describe("lamp-9"), None);
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
