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
    /// How many more changes to the panels it publishes it may make, once it has made one.
    pub publishes: Option<Publishes>,
    /// The newest update of each panel it published that came too soon to be made, made as soon as it may be, so that
    /// the latest always arrives: one for each panel, at most the [`tomori_environment::PANEL_LIMIT`] an environment
    /// may show, since those waiting count towards them.
    pub pending: Vec<crate::stage::Publishing>,
    /// The panels an update of which, waiting, the stage refused once it could be made, the latest last, at most
    /// [`REFUSED`], each said once until it is taken or retracted. See [`Channel::refuse`].
    pub refused: std::collections::VecDeque<String>,
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

/// The most panels whose refused updates a side's channel remembers having said, the earliest forgotten first.
pub const REFUSED: usize = 64;

impl Channel {
    /// Note that an update of the panel `id` was refused once it could be made. Returns whether to say so: not where it
    /// was said already and the panel has been neither taken nor retracted since.
    pub fn refuse(&mut self, id: &str) -> bool {
        if self.refused.iter().any(|refused| refused == id) {
            return false;
        }
        self.refused.push_back(id.to_owned());
        if self.refused.len() > REFUSED {
            self.refused.pop_front();
        }
        true
    }

    /// Forget that updates of the panel `id` were refused, as one is taken or it is retracted, so that a later refusal
    /// is said again.
    pub fn accept(&mut self, id: &str) {
        self.refused.retain(|refused| refused != id);
    }

    pub fn new(binding: u64, environment: &str, version: u32) -> Self {
        Self {
            binding,
            environment: environment.to_owned(),
            version,
            state: None,
            objects: Vec::new(),
            publishes: None,
            pending: Vec::new(),
            refused: std::collections::VecDeque::new(),
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

/// How many changes a side may make to the panels it publishes in a space at once, and how many more each second after:
/// as many at once as the panels it may show, [`tomori_environment::PANEL_LIMIT`], 32, so that it puts up, or updates,
/// every one together, and half as many each second after, 16, so that a side updating every one each second has each
/// one's latest made within two. Each change sends the panel again to everyone in the space, in a projection patch to a
/// client that applies them, coalesced for a connection that has not taken the last. Only changes the stage takes
/// count: the same again, one to a panel dismissed, one refused and a retraction of nothing cost nothing.
pub const PUBLISH_BURST: f32 = tomori_environment::PANEL_LIMIT as f32;
pub const PUBLISHES_PER_SECOND: f32 = PUBLISH_BURST / 2.0;

/// How many bytes of panels a side may publish in a space at once, and how many more each second after: 16 KiB at once
/// for each panel it may show, 512 KiB, so that it puts up every one together even where each is a chart of every
/// series at every point, about 13 KiB, and a quarter of that each second, 128 KiB, 8 KiB for each change it may make,
/// as much as every one of its panels a live chart of three series over 181 points, under 4 KiB, changing each second.
pub const PUBLISH_BYTES_BURST: f32 = PUBLISH_BURST * 16.0 * 1024.0;
pub const PUBLISH_BYTES_PER_SECOND: f32 = PUBLISH_BYTES_BURST / 4.0;

/// How many times a side may publish or retract in a space at once, and how many more each second after, whether or
/// not they change anything, so that a side sending nothing new as fast as it can never keeps the hub busy: 160 and 80,
/// five times and two and a half times the panels it may show, so that a side publishing every one each second, as on
/// a timer, is never refused for it.
pub const MESSAGE_BURST: f32 = PUBLISH_BURST * 5.0;
pub const MESSAGES_PER_SECOND: f32 = MESSAGE_BURST / 2.0;

/// How soon after the last a side may update the same panel: once a second, as fast as a live chart needs, with a tenth
/// of that to spare, so that a side publishing on a timer of a second never has a tick held back.
pub const PANEL_INTERVAL: Duration = Duration::from_millis(900);

/// What a side may still publish in a space: buckets of changes, bytes and messages, refilled each second, and when it
/// last changed each panel, so that a side publishing as fast as it can never floods the space's clients.
#[derive(Debug)]
pub struct Publishes {
    changes: f32,
    bytes: f32,
    messages: f32,
    at: Instant,
    /// When each panel was last changed, by the side's id, those older than [`PANEL_INTERVAL`] forgotten.
    last: Vec<(String, Instant)>,
}

impl Publishes {
    pub fn new(now: Instant) -> Self {
        Self {
            changes: PUBLISH_BURST,
            bytes: PUBLISH_BYTES_BURST,
            messages: MESSAGE_BURST,
            at: now,
            last: Vec::new(),
        }
    }

    /// Buckets with no changes left, as a side's that has just made a burst of them.
    #[cfg(test)]
    pub fn spent(now: Instant) -> Self {
        Self {
            changes: 0.0,
            ..Self::new(now)
        }
    }

    /// Refill the buckets for the time since they were last, up to `now`.
    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60))
            .as_secs_f32();
        self.at = self.at.max(now);
        self.changes = (self.changes + elapsed * PUBLISHES_PER_SECOND).min(PUBLISH_BURST);
        self.bytes = (self.bytes + elapsed * PUBLISH_BYTES_PER_SECOND).min(PUBLISH_BYTES_BURST);
        self.messages = (self.messages + elapsed * MESSAGES_PER_SECOND).min(MESSAGE_BURST);
        self.last
            .retain(|(_, at)| now.saturating_duration_since(*at) < PANEL_INTERVAL);
    }

    /// Whether a side may publish or retract at `now` at all, taking it from the messages' bucket if so, or why not.
    pub fn message(&mut self, now: Instant) -> Result<(), &'static str> {
        self.refill(now);
        if self.messages < 1.0 {
            return Err("The environment is publishing faster than Tomori takes its panels.");
        }
        self.messages -= 1.0;
        Ok(())
    }

    /// Whether a change of `bytes` bytes to the panel `panel`, or, without one, a retraction, may be made at `now`: the
    /// panel not changed within [`PANEL_INTERVAL`], and changes and bytes left. A change larger than the bytes left may be
    /// made once the bucket is full, leaving it owing, so that a large panel is still published, as rarely as its size
    /// allows.
    pub fn ready(&mut self, now: Instant, panel: Option<&str>, bytes: usize) -> bool {
        self.refill(now);
        let recent = self.last.iter().any(|(id, _)| Some(id.as_str()) == panel);
        let full = self.bytes >= PUBLISH_BYTES_BURST;
        !recent && self.changes >= 1.0 && (self.bytes >= bytes as f32 || full)
    }

    /// Take a change of `bytes` bytes to `panel`, or a retraction, made at `now`, as [`Self::ready`] allowed.
    pub fn charge(&mut self, now: Instant, panel: Option<&str>, bytes: usize) {
        self.changes -= 1.0;
        self.bytes -= bytes as f32;
        if let Some(panel) = panel {
            self.last.push((panel.to_owned(), now));
        }
    }

    /// Forget when `panel` was last changed, as it is retracted, so that it may be published again at once.
    pub fn forget(&mut self, panel: &str) {
        self.last.retain(|(id, _)| id != panel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_environment::Object;

    #[test]
    fn a_refused_update_is_said_once_until_its_panel_is_taken_and_so_many_are_remembered() {
        let mut channel = Channel::new(1, "yard", 1);
        assert!(channel.refuse("rota"));
        assert!(!channel.refuse("rota"));
        // Taken, or retracted, it is forgotten, and said again.
        channel.accept("rota");
        assert!(channel.refuse("rota"));
        // At most so many, the earliest forgotten first.
        for index in 0..REFUSED {
            assert!(channel.refuse(&format!("p{index}")));
        }
        assert_eq!(channel.refused.len(), REFUSED);
        assert!(channel.refuse("rota"));
        assert!(!channel.refuse(&format!("p{}", REFUSED - 1)));
    }

    #[test]
    fn publishing_is_held_to_a_rate_for_each_panel_and_for_the_space() {
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);
        // Each panel about once a second, a tenth to spare; a retraction forgets it, so that it may come back at once.
        let mut publishes = Publishes::new(start);
        assert!(publishes.ready(start, Some("a"), 100));
        publishes.charge(start, Some("a"), 100);
        assert!(!publishes.ready(at(899), Some("a"), 100));
        assert!(publishes.ready(at(899), Some("b"), 100));
        assert!(publishes.ready(at(900), Some("a"), 100));
        publishes.charge(at(900), Some("a"), 100);
        publishes.forget("a");
        assert!(publishes.ready(at(901), Some("a"), 100));
        // The space's changes: a burst, as many as the panels a side may show, then half as many a second; retractions
        // too.
        let mut publishes = Publishes::new(start);
        for index in 0..PUBLISH_BURST as usize {
            let panel = format!("p{index}");
            assert!(publishes.ready(start, Some(&panel), 10));
            publishes.charge(start, Some(&panel), 10);
        }
        assert!(!publishes.ready(start, Some("late"), 10));
        assert!(!publishes.ready(start, None, 0));
        assert!(!publishes.ready(at(62), None, 0));
        assert!(publishes.ready(at(63), None, 0));
        publishes.charge(at(63), None, 0);
        // A clock that goes back refills nothing.
        assert!(!publishes.ready(start, Some("earlier"), 10));
        // Bytes: a burst, then 128 KiB a second; a panel larger than what is left waits until the bucket is full.
        let mut publishes = Publishes::new(start);
        let chart = 192 * 1024;
        for panel in ["a", "b"] {
            assert!(publishes.ready(start, Some(panel), chart));
            publishes.charge(start, Some(panel), chart);
        }
        assert!(!publishes.ready(start, Some("c"), chart));
        assert!(publishes.ready(at(500), Some("c"), chart));
        let mut publishes = Publishes::new(start);
        let huge = 2400 * 1024;
        assert!(publishes.ready(start, Some("a"), huge));
        publishes.charge(start, Some("a"), huge);
        assert!(!publishes.ready(at(10_000), Some("b"), 1));
        assert!(publishes.ready(at(60_000), Some("b"), 1));
        // Messages, whatever they change: a burst, then half as many a second.
        let mut publishes = Publishes::new(start);
        for _ in 0..MESSAGE_BURST as usize {
            assert_eq!(publishes.message(start), Ok(()));
        }
        assert!(publishes.message(start).is_err());
        assert!(publishes.message(at(12)).is_err());
        assert_eq!(publishes.message(at(13)), Ok(()));
    }

    /// What a space's panels may change grows with how many a side may show, so that a side showing as many as it may
    /// is held no harder for each than one showing a few: these are the numbers `design/projection.md` states.
    #[test]
    fn a_space_s_rates_are_those_the_panels_a_side_may_show_need() {
        assert_eq!(tomori_environment::PANEL_LIMIT, 32);
        assert_eq!((PUBLISH_BURST, PUBLISHES_PER_SECOND), (32.0, 16.0));
        assert_eq!(
            (PUBLISH_BYTES_BURST, PUBLISH_BYTES_PER_SECOND),
            (512.0 * 1024.0, 128.0 * 1024.0)
        );
        assert_eq!((MESSAGE_BURST, MESSAGES_PER_SECOND), (160.0, 80.0));
        // Every panel a side may show is put up at once, each a chart of every series at every point, 16 KiB; one more
        // would wait, were it not refused first.
        let start = Instant::now();
        let mut publishes = Publishes::new(start);
        for index in 0..tomori_environment::PANEL_LIMIT {
            let panel = format!("p{index}");
            assert!(publishes.ready(start, Some(&panel), 16 * 1024));
            publishes.charge(start, Some(&panel), 16 * 1024);
        }
        assert!(!publishes.ready(start, Some("more"), 1));
        // A side publishing every one each second, whether or not each changed, is never refused for how often it
        // calls, for as long as it likes.
        let mut publishes = Publishes::new(start);
        for second in 0..600 {
            for _ in 0..tomori_environment::PANEL_LIMIT {
                assert_eq!(
                    publishes.message(start + Duration::from_secs(second)),
                    Ok(())
                );
            }
        }
    }

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
