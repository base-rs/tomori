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
    /// The objects the side described, every one it publishes now within [`OBJECT_LIMIT`]: each identity, its
    /// description, cleaned, and its reference, if it has one. Pointing, `inspect` and `act` look an object up among
    /// all of them; only the agent's ambient list, [`crate::ambient`], is cut shorter.
    pub objects: Objects,
    /// What the agent's ambient list is ranked by, and the list as last ranked. See [`crate::ambient`].
    pub ambient: crate::ambient::Ambient,
    /// How many more changes to the panels it publishes it may make, once it has made one.
    pub publishes: Option<Publishes>,
    /// The newest update of each panel it published that came too soon to be made, made as soon as it may be, so that
    /// the latest always arrives: one for each panel, at most the [`tomori_environment::PANEL_LIMIT`] an environment
    /// may show, since those waiting count towards them.
    pub pending: Vec<crate::stage::Publishing>,
    /// The panels an update of which, waiting, the stage refused once it could be made, the latest last, at most
    /// [`REFUSED`], each said once until it is taken or retracted. See [`Channel::refuse`].
    pub refused: std::collections::VecDeque<String>,
    /// The cluster anchors the side placed, and its changes of them waiting, once it has placed one.
    pub places: Option<Places>,
    /// The actions the side declared, each description and label cleaned. See `design/environment-actions.md`.
    pub actions: Vec<tomori_environment::Action>,
    /// How many more states and events it may send, once it has sent one.
    pub updates: Option<Updates>,
    /// The zones the side declared, and who is within each, once it has declared them.
    pub zones: crate::zones::Declared,
}

/// An object a side described, as the server keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Described {
    /// The identity clients name it by.
    pub id: String,
    /// What the agent is told of it, cleaned and bounded.
    pub description: String,
    /// The reference the agent may inspect it by, as the side gave it, if it is one [`tomori_environment::valid_reference`] takes.
    pub reference: Option<String>,
    /// Where it stands, in the space's coordinates, where the side said and it is within bounds once carried there.
    pub position: Option<[f32; 3]>,
    /// How far it reaches from its middle, in metres, within [`tomori_environment::EXTENT_LIMITS`], as the side said,
    /// or else [`tomori_environment::DEFAULT_EXTENT`].
    pub extent: f32,
    /// The names of the actions it offers, each one [`tomori_environment::valid_action_name`] takes, at most
    /// [`tomori_environment::OFFER_LIMIT`], none twice, and none without a reference.
    pub offers: Vec<String>,
}

/// The most panels whose refused updates a side's channel remembers having said, the earliest forgotten first: twice
/// as many as it may show.
pub const REFUSED: usize = 2 * tomori_environment::PANEL_LIMIT;

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
            objects: Objects::default(),
            ambient: crate::ambient::Ambient::default(),
            publishes: None,
            pending: Vec::new(),
            refused: std::collections::VecDeque::new(),
            places: None,
            actions: Vec::new(),
            updates: None,
            zones: crate::zones::Declared::default(),
        }
    }

    /// The actions the side declared that `object` offers, in the order it offers them.
    pub fn offered(&self, object: &Described) -> Vec<&tomori_environment::Action> {
        object
            .offers
            .iter()
            .filter_map(|name| self.actions.iter().find(|action| action.name() == name))
            .collect()
    }

    /// The cluster anchors the side placed, in the space's coordinates.
    pub fn anchors(&self) -> &[tomori_protocol::cluster::Anchor] {
        self.places
            .as_ref()
            .map_or(&[], |places| places.anchors.as_slice())
    }

    /// The object `id`, if the side described it.
    pub fn describe(&self, id: &str) -> Option<&Described> {
        self.objects.get(id)
    }

    /// The object the side describes now with `reference`, if any.
    pub fn referenced(&self, reference: &str) -> Option<&Described> {
        self.objects.referenced(reference)
    }
}

/// The objects a side describes, as the server keeps them, in the order it described them, each found by its identity
/// and its reference without a search, since there may be [`OBJECT_LIMIT`] of them.
///
/// At most [`OBJECT_LIMIT`], 1,024, are kept, each at most an identity of [`IDENTITY_LIMIT`], 128 bytes, a description
/// of [`DESCRIPTION_LIMIT`] characters, 800 bytes at most, a reference of 64 bytes and
/// [`tomori_environment::OFFER_LIMIT`] names of 32, with an index of the identity and reference: about 2 KB each, and 2
/// MB for a binding at the worst.
#[derive(Debug, Default)]
pub struct Objects {
    listed: Vec<Described>,
    ids: std::collections::HashMap<String, usize>,
    references: std::collections::HashMap<String, usize>,
}

impl Objects {
    /// The object `id`, if there is one.
    pub fn get(&self, id: &str) -> Option<&Described> {
        self.ids.get(id).map(|index| &self.listed[*index])
    }

    /// Where the object `id` is among them, if there is one.
    pub fn index(&self, id: &str) -> Option<usize> {
        self.ids.get(id).copied()
    }

    /// Where the object with `reference` is among them, if there is one.
    pub fn reference_index(&self, reference: &str) -> Option<usize> {
        self.references.get(reference).copied()
    }

    /// The object with `reference`, if there is one.
    pub fn referenced(&self, reference: &str) -> Option<&Described> {
        self.references
            .get(reference)
            .map(|index| &self.listed[*index])
    }
}

impl std::ops::Deref for Objects {
    type Target = [Described];

    fn deref(&self) -> &[Described] {
        &self.listed
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
/// [`tomori_environment::valid_reference`] takes it and no object before has it, so that a reference names one object,
/// and each position where `carry` takes it into the space's coordinates, within bounds.
pub fn objects(
    described: Vec<tomori_environment::Object>,
    carry: impl Fn([f32; 3]) -> Option<[f32; 3]>,
) -> Objects {
    let mut kept = Objects::default();
    for object in described {
        let id = object.id();
        if id.is_empty() || id.len() > IDENTITY_LIMIT || kept.ids.contains_key(id) {
            continue;
        }
        if kept.listed.len() == OBJECT_LIMIT {
            log::debug!(
                "An environment described more objects than the {OBJECT_LIMIT} the server keeps."
            );
            break;
        }
        let reference = object
            .reference()
            .filter(|reference| tomori_environment::valid_reference(reference))
            .filter(|reference| !kept.references.contains_key(*reference))
            .map(str::to_owned);
        if reference.is_none() && object.reference().is_some() {
            log::debug!(
                "An environment described an object with a reference the server does not take."
            );
        }
        let position = object.position().and_then(&carry);
        if position.is_none() && object.position().is_some() {
            log::debug!("An environment described an object at a position out of bounds.");
        }
        let mut offers: Vec<String> = Vec::new();
        if reference.is_some() {
            for name in object.offers() {
                if offers.len() == tomori_environment::OFFER_LIMIT {
                    break;
                }
                if tomori_environment::valid_action_name(name) && !offers.contains(name) {
                    offers.push(name.clone());
                }
            }
        }
        let index = kept.listed.len();
        kept.ids.insert(id.to_owned(), index);
        if let Some(reference) = &reference {
            kept.references.insert(reference.clone(), index);
        }
        kept.listed.push(Described {
            id: id.to_owned(),
            description: crate::space::one_line(object.description(), DESCRIPTION_LIMIT),
            reference,
            position,
            extent: object.extent().filter(|extent| extent.is_finite()).map_or(
                tomori_environment::DEFAULT_EXTENT,
                |extent| {
                    extent.clamp(
                        tomori_environment::EXTENT_LIMITS.0,
                        tomori_environment::EXTENT_LIMITS.1,
                    )
                },
            ),
            offers,
        });
    }
    kept
}

/// The actions a side declares, as the server keeps them, or why they are refused: a declaration
/// [`tomori_environment::check_actions`] refuses, and otherwise each description and label cleaned onto one line and
/// cut, as everything else told to the agent of an environment is. See `design/environment-actions.md`.
pub fn actions(
    declared: Vec<tomori_environment::Action>,
) -> Result<Vec<tomori_environment::Action>, String> {
    tomori_environment::check_actions(&declared).map_err(|error| error.message().to_owned())?;
    Ok(declared
        .iter()
        .map(|action| action.cleaned(crate::space::one_line))
        .collect())
}

/// How many actions the space's agent may take at once in one binding, and how many more each second after: 10 at
/// once, then 30 a minute, so that the agent answering a few requests together is never refused, and one looping on
/// an action is held to the rate. See `design/environment-actions.md`.
pub const ACT_BURST: f32 = 10.0;
pub const ACTS_PER_SECOND: f32 = 0.5;

/// The actions the agent may still take in a binding: a bucket of [`ACT_BURST`], refilled at [`ACTS_PER_SECOND`].
#[derive(Debug)]
pub struct Acts {
    tokens: f32,
    at: Instant,
}

impl Acts {
    pub fn new(now: Instant) -> Self {
        Self {
            tokens: ACT_BURST,
            at: now,
        }
    }

    /// Whether an action taken at `now` is within the rate, taking it from the bucket if so.
    pub fn take(&mut self, now: Instant) -> bool {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60));
        self.at = self.at.max(now);
        self.tokens = (self.tokens + elapsed.as_secs_f32() * ACTS_PER_SECOND).min(ACT_BURST);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }

    /// How many actions are left in the bucket, as of the last taken.
    #[cfg(test)]
    pub fn left(&self) -> f32 {
        self.tokens
    }

    /// Give back an action taken from the bucket that was not handed on, as one its side could not take.
    pub fn give_back(&mut self) {
        self.tokens = (self.tokens + 1.0).min(ACT_BURST);
    }
}

/// The states and events a side may still send in its space: a bucket of [`tomori_environment::UPDATE_BURST`], refilled
/// at [`tomori_environment::UPDATES_PER_SECOND`], so that a side saying where what moves is 20 times a second is never
/// refused, and one announcing as fast as it can is held to the rate.
#[derive(Debug)]
pub struct Updates {
    tokens: f32,
    at: Instant,
}

impl Updates {
    pub fn new(now: Instant) -> Self {
        Self {
            tokens: tomori_environment::UPDATE_BURST as f32,
            at: now,
        }
    }

    /// Whether an update sent at `now` is taken, taking it from the bucket if so, or why not.
    pub fn take(&mut self, now: Instant) -> Result<(), &'static str> {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60));
        self.at = self.at.max(now);
        self.tokens = (self.tokens
            + elapsed.as_secs_f32() * tomori_environment::UPDATES_PER_SECOND as f32)
            .min(tomori_environment::UPDATE_BURST as f32);
        if self.tokens < 1.0 {
            return Err("The environment is updating faster than Tomori sends its updates.");
        }
        self.tokens -= 1.0;
        Ok(())
    }
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
/// a page, [`PUBLISH_PAGE`], half the panels it may show, [`tomori_environment::PANEL_LIMIT`], 64 at once, and as many
/// each second after, so that a side updating every one each second has each one's latest made within two, and one
/// putting up all 128 has a page up at once and the rest at the hub's next sweep, within a second. A page, rather than
/// every panel at once, so that no one message to a space's clients carries all 128 panels whole: a patch carries at
/// most a page and what changed since, and a client that applies no patches is sent the projection whole within its
/// frame however many changed. Each change sends the panel again to everyone in the space, in a projection patch to a
/// client that applies them, coalesced for a connection that has not taken the last. Only changes the stage takes
/// count: the same again, one to a panel dismissed, one refused and a retraction of nothing cost nothing.
pub const PUBLISH_BURST: f32 = PUBLISH_PAGE as f32;
pub const PUBLISHES_PER_SECOND: f32 = PUBLISH_BURST;

/// The most panels a side puts up, or updates, at once: half as many as it may show, 64, so that a page of them, every
/// series of each chart at every point, is one patch of at most about 1 MiB. See [`PUBLISH_BURST`].
pub const PUBLISH_PAGE: usize = tomori_environment::PANEL_LIMIT / 2;

/// How many bytes of panels a side may publish in a space at once, and how many more each second after: 16 KiB at once
/// for each panel of a page, 1 MiB, so that it puts up a page together even where each is a chart of every series at
/// every point, about 13 KiB, and 4 KiB each second for each panel it may show, 512 KiB, as much as every one of its
/// panels a live chart of three series over 181 points, under 4 KiB, changing each second.
pub const PUBLISH_BYTES_BURST: f32 = PUBLISH_BURST * 16.0 * 1024.0;
pub const PUBLISH_BYTES_PER_SECOND: f32 = tomori_environment::PANEL_LIMIT as f32 * 4.0 * 1024.0;

/// How many times a side may publish or retract in a space at once, and how many more each second after, whether or
/// not they change anything, so that a side sending nothing new as fast as it can never keeps the hub busy: 640 and
/// 320, five times and two and a half times the panels it may show, so that a side publishing every one each second,
/// as on a timer, is never refused for it.
pub const MESSAGE_BURST: f32 = tomori_environment::PANEL_LIMIT as f32 * 5.0;
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

/// How many times a side may place or remove anchors in a space at once, and how many more each second after, whether
/// or not each changes anything, so that a side calling as fast as it can never keeps the hub busy: eight times the
/// changes it may make.
pub const PLACE_BURST: f32 = tomori_environment::ANCHOR_BURST as f32 * 8.0;
pub const PLACES_PER_SECOND: f32 = tomori_environment::ANCHOR_CHANGES_PER_SECOND as f32 * 8.0;

/// A change a side asked of its anchors that waits for its rate: placing the anchor, or removing the one of its name.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Place(tomori_protocol::cluster::Anchor),
    Remove(String),
}

impl Change {
    /// The name of the anchor it changes.
    pub fn name(&self) -> &str {
        match self {
            Self::Place(anchor) => &anchor.name,
            Self::Remove(name) => name,
        }
    }
}

/// The anchors a side placed in a space, in the space's coordinates, and what it may still change of them: a bucket of
/// changes, [`tomori_environment::ANCHOR_BURST`] at once and [`tomori_environment::ANCHOR_CHANGES_PER_SECOND`] after,
/// one of calls, and when it last changed each anchor, so that each changes at most once every
/// [`tomori_environment::ANCHOR_INTERVAL`]; and the changes waiting for them, the latest of each anchor's. Every change
/// lays a cluster out again and moves what stands there, so a flapping alert must not shake workers about. See
/// `design/environments.md`.
#[derive(Debug)]
pub struct Places {
    /// The anchors placed, in the order they were first placed.
    pub anchors: Vec<tomori_protocol::cluster::Anchor>,
    /// The changes waiting, in the order they were first asked for.
    pub waiting: Vec<Change>,
    changes: f32,
    calls: f32,
    at: Instant,
    last: Vec<(String, Instant)>,
}

impl Places {
    pub fn new(now: Instant) -> Self {
        Self {
            anchors: Vec::new(),
            waiting: Vec::new(),
            changes: tomori_environment::ANCHOR_BURST as f32,
            calls: PLACE_BURST,
            at: now,
            last: Vec::new(),
        }
    }

    /// Refill the buckets for the time since they were last, up to `now`.
    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60))
            .as_secs_f32();
        self.at = self.at.max(now);
        let burst = tomori_environment::ANCHOR_BURST as f32;
        let rate = tomori_environment::ANCHOR_CHANGES_PER_SECOND as f32;
        self.changes = (self.changes + elapsed * rate).min(burst);
        self.calls = (self.calls + elapsed * PLACES_PER_SECOND).min(PLACE_BURST);
        self.last.retain(|(_, at)| {
            now.saturating_duration_since(*at) < tomori_environment::ANCHOR_INTERVAL
        });
    }

    /// Whether a side may place or remove an anchor at `now` at all, taking it from the calls' bucket if so, or why not.
    pub fn call(&mut self, now: Instant) -> Result<(), &'static str> {
        self.refill(now);
        if self.calls < 1.0 {
            return Err("The environment is placing anchors faster than Tomori takes them.");
        }
        self.calls -= 1.0;
        Ok(())
    }

    /// Whether the anchor `name` may change at `now`: not changed within [`tomori_environment::ANCHOR_INTERVAL`], with
    /// a change left in the space's bucket.
    pub fn ready(&mut self, now: Instant, name: &str) -> bool {
        self.refill(now);
        self.changes >= 1.0 && !self.last.iter().any(|(changed, _)| changed == name)
    }

    /// Take a change of the anchor `name` made at `now`, as [`Self::ready`] allowed.
    pub fn charge(&mut self, now: Instant, name: &str) {
        self.changes -= 1.0;
        self.last.push((name.to_owned(), now));
    }

    /// Let `change` wait, in place of any change of its anchor waiting before it; a removal of an anchor not placed
    /// cancels its placement waiting, and waits for nothing.
    pub fn wait(&mut self, change: Change) {
        let placed = self
            .anchors
            .iter()
            .any(|anchor| anchor.name == change.name());
        match self
            .waiting
            .iter()
            .position(|waiting| waiting.name() == change.name())
        {
            Some(at) if matches!(change, Change::Remove(_)) && !placed => {
                self.waiting.remove(at);
            }
            Some(at) => self.waiting[at] = change,
            None if matches!(change, Change::Remove(_)) && !placed => {}
            None => self.waiting.push(change),
        }
    }

    /// Whether an anchor of `name` is placed, or waits to be.
    pub fn has(&self, name: &str) -> bool {
        self.anchors.iter().any(|anchor| anchor.name == name)
            || self
                .waiting
                .iter()
                .any(|waiting| matches!(waiting, Change::Place(anchor) if anchor.name == name))
    }

    /// How many anchors are placed, or wait to be, but not to be removed.
    pub fn count(&self) -> usize {
        let removing = |name: &str| {
            self.waiting
                .iter()
                .any(|waiting| matches!(waiting, Change::Remove(removed) if removed == name))
        };
        self.anchors
            .iter()
            .filter(|anchor| !removing(&anchor.name))
            .count()
            + self
                .waiting
                .iter()
                .filter(|waiting| {
                    matches!(waiting, Change::Place(anchor)
                        if !self.anchors.iter().any(|placed| placed.name == anchor.name))
                })
                .count()
    }

    /// Make `change` to the anchors placed: placing one, in place of one of its name, or removing it. Returns whether it
    /// changed them.
    pub fn make(&mut self, change: Change) -> bool {
        match change {
            Change::Place(anchor) => match self
                .anchors
                .iter_mut()
                .find(|placed| placed.name == anchor.name)
            {
                Some(placed) if *placed == anchor => false,
                Some(placed) => {
                    *placed = anchor;
                    true
                }
                None => {
                    self.anchors.push(anchor);
                    true
                }
            },
            Change::Remove(name) => {
                let before = self.anchors.len();
                self.anchors.retain(|anchor| anchor.name != name);
                self.anchors.len() != before
            }
        }
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
        // The space's changes: a burst, a page, then as many a second; retractions too.
        let mut publishes = Publishes::new(start);
        for index in 0..PUBLISH_BURST as usize {
            let panel = format!("p{index}");
            assert!(publishes.ready(start, Some(&panel), 10));
            publishes.charge(start, Some(&panel), 10);
        }
        assert!(!publishes.ready(start, Some("late"), 10));
        assert!(!publishes.ready(start, None, 0));
        assert!(!publishes.ready(at(15), None, 0));
        assert!(publishes.ready(at(16), None, 0));
        publishes.charge(at(16), None, 0);
        // A clock that goes back refills nothing.
        assert!(!publishes.ready(start, Some("earlier"), 10));
        // Bytes: a burst, then 512 KiB a second; a panel larger than what is left waits until the bucket is full.
        let mut publishes = Publishes::new(start);
        let chart = 384 * 1024;
        for panel in ["a", "b"] {
            assert!(publishes.ready(start, Some(panel), chart));
            publishes.charge(start, Some(panel), chart);
        }
        assert!(!publishes.ready(start, Some("c"), chart));
        assert!(!publishes.ready(at(200), Some("c"), chart));
        assert!(publishes.ready(at(260), Some("c"), chart));
        let mut publishes = Publishes::new(start);
        let huge = 2400 * 1024;
        assert!(publishes.ready(start, Some("a"), huge));
        publishes.charge(start, Some("a"), huge);
        assert!(!publishes.ready(at(2_600), Some("b"), 1));
        assert!(publishes.ready(at(2_700), Some("b"), 1));
        // Messages, whatever they change: a burst, then half as many a second.
        let mut publishes = Publishes::new(start);
        for _ in 0..MESSAGE_BURST as usize {
            assert_eq!(publishes.message(start), Ok(()));
        }
        assert!(publishes.message(start).is_err());
        assert!(publishes.message(at(3)).is_err());
        assert_eq!(publishes.message(at(4)), Ok(()));
    }

    /// What a space's panels may change grows with how many a side may show, so that a side showing as many as it may
    /// is held no harder for each than one showing a few: these are the numbers `design/projection.md` states.
    #[test]
    fn a_space_s_rates_are_those_the_panels_a_side_may_show_need() {
        assert_eq!(tomori_environment::PANEL_LIMIT, 128);
        assert_eq!((PUBLISH_BURST, PUBLISHES_PER_SECOND), (64.0, 64.0));
        assert_eq!(
            (PUBLISH_BYTES_BURST, PUBLISH_BYTES_PER_SECOND),
            (1024.0 * 1024.0, 512.0 * 1024.0)
        );
        assert_eq!((MESSAGE_BURST, MESSAGES_PER_SECOND), (640.0, 320.0));
        // A page of panels is put up at once, each a chart of every series at every point, 16 KiB; one more waits.
        let start = Instant::now();
        let mut publishes = Publishes::new(start);
        for index in 0..PUBLISH_PAGE {
            let panel = format!("p{index}");
            assert!(publishes.ready(start, Some(&panel), 16 * 1024));
            publishes.charge(start, Some(&panel), 16 * 1024);
        }
        assert!(!publishes.ready(start, Some("more"), 1));
        // The rest, live charts of 4 KiB each, at the hub's next sweep, a second later.
        let later = start + Duration::from_secs(1);
        for index in PUBLISH_PAGE..tomori_environment::PANEL_LIMIT {
            let panel = format!("p{index}");
            assert!(publishes.ready(later, Some(&panel), 4 * 1024));
            publishes.charge(later, Some(&panel), 4 * 1024);
        }
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
            Object::new("lamp-1", "a lamp [lit]\nby the \"gate\"")
                .with_reference("lamp/1")
                .at([1.0, 0.5, -2.0]),
            Object::new("lamp-1", "again"),
            Object::new("lamp-2", "z".repeat(DESCRIPTION_LIMIT * 2)).at([f32::NAN, 0.0, 0.0]),
        ];
        described
            .extend((0..OBJECT_LIMIT).map(|index| Object::new(format!("more-{index}"), "more")));
        // Carried a metre along x, and only where finite.
        let kept = objects(described, |[x, y, z]| {
            [x, y, z]
                .iter()
                .all(|value| value.is_finite())
                .then_some([x + 1.0, y, z])
        });
        assert_eq!(kept.len(), OBJECT_LIMIT);
        assert_eq!(
            kept[0],
            Described {
                id: "lamp-1".to_owned(),
                description: "a lamp (lit) by the 'gate'".to_owned(),
                reference: Some("lamp/1".to_owned()),
                position: Some([2.0, 0.5, -2.0]),
                extent: tomori_environment::DEFAULT_EXTENT,
                offers: Vec::new(),
            }
        );
        // Out of bounds, it is described without a position.
        assert_eq!(kept[1].position, None);
        // How far each reaches, held to its bounds, and as much as an object reaches without one, or one not finite.
        let reaching = objects(
            [Some(0.6), Some(9.0), Some(0.0), Some(f32::NAN), None]
                .into_iter()
                .enumerate()
                .map(|(index, extent)| {
                    let object = Object::new(format!("o-{index}"), "a thing");
                    match extent {
                        Some(extent) => object.with_extent(extent),
                        None => object,
                    }
                })
                .collect(),
            Some,
        );
        let extents: Vec<f32> = reaching.iter().map(|object| object.extent).collect();
        assert_eq!(
            extents,
            [
                0.6,
                tomori_environment::EXTENT_LIMITS.1,
                tomori_environment::EXTENT_LIMITS.0,
                tomori_environment::DEFAULT_EXTENT,
                tomori_environment::DEFAULT_EXTENT,
            ]
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
    fn an_object_offers_actions_by_valid_names_only_with_a_reference() {
        use tomori_environment::{Action, OFFER_LIMIT};
        let names: Vec<String> = ["light", "Bad", "light"]
            .into_iter()
            .map(str::to_owned)
            .chain((0..OFFER_LIMIT).map(|index| format!("a{index}")))
            .collect();
        let kept = objects(
            vec![
                Object::new("lamp-1", "a lamp")
                    .with_reference("lamp/1")
                    .offering(names),
                Object::new("lamp-2", "a lamp").offering(["light"]),
            ],
            Some,
        );
        // Valid, once each, at most eight; none without a reference, since the agent names it by one.
        let expected: Vec<String> = std::iter::once("light".to_owned())
            .chain((0..OFFER_LIMIT - 1).map(|index| format!("a{index}")))
            .collect();
        assert_eq!(kept[0].offers, expected);
        assert!(kept[1].offers.is_empty());
        // Offered are only those declared, in the object's order.
        let mut channel = Channel::new(1, "garden", 1);
        channel.actions = vec![Action::new("a1", "one"), Action::new("light", "Light it")];
        let offered: Vec<&str> = channel
            .offered(&kept[0])
            .into_iter()
            .map(Action::name)
            .collect();
        assert_eq!(offered, ["light", "a1"]);
    }

    #[test]
    fn actions_are_checked_and_cleaned() {
        use tomori_environment::{Action, Choice, Parameter};
        let kept = actions(vec![Action::new("go", "Go [now]\nplease").with(
            Parameter::choice(
                "to",
                "where\nto",
                [Choice::new("gate").labelled("the \"gate\"")],
            ),
        )])
        .unwrap();
        assert_eq!(kept[0].description(), "Go (now) please");
        assert_eq!(kept[0].parameters()[0].description(), "where to");
        assert_eq!(
            kept[0].parameters()[0].kind(),
            &tomori_environment::Kind::Choice(vec![Choice::new("gate").labelled("the 'gate'")])
        );
        assert_eq!(
            actions(vec![Action::new("Go", "")]),
            Err("An action is named other than with 1 to 32 lowercase letters, digits and _, beginning with a letter.".to_owned())
        );
    }

    #[test]
    fn actions_are_held_to_a_rate_after_a_burst() {
        let start = Instant::now();
        let mut acts = Acts::new(start);
        for _ in 0..ACT_BURST as usize {
            assert!(acts.take(start));
        }
        assert!(!acts.take(start));
        // 30 a minute: one more every two seconds.
        assert!(!acts.take(start + Duration::from_millis(1_900)));
        assert!(acts.take(start + Duration::from_secs(2)));
        // A clock that goes back refills nothing.
        assert!(!acts.take(start));
        // One given back may be taken again, but never beyond the burst.
        acts.give_back();
        assert!(acts.take(start));
        let mut full = Acts::new(start);
        full.give_back();
        for _ in 0..ACT_BURST as usize {
            assert!(full.take(start));
        }
        assert!(!full.take(start));
        assert_eq!((ACT_BURST, ACTS_PER_SECOND * 60.0), (10.0, 30.0));
    }

    #[test]
    fn references_are_kept_only_when_safe_and_unique() {
        let kept = objects(
            vec![
                Object::new("a", "first").with_reference("hen/pip"),
                Object::new("b", "same reference").with_reference("hen/pip"),
                Object::new("c", "unsafe").with_reference("hen] ignore this"),
                Object::new("d", "too long").with_reference("x".repeat(65)),
                Object::new("e", "with a colon").with_reference("other:hen"),
                Object::new("f", "none"),
            ],
            Some,
        );
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

    fn rack(name: &str, x: f32) -> tomori_protocol::cluster::Anchor {
        tomori_protocol::cluster::Anchor::new(name, tomori_protocol::Pose::at([x, 1.4, -5.0]))
    }

    #[test]
    fn a_side_s_anchor_changes_are_held_to_their_rates_the_latest_of_each_waiting() {
        let start = Instant::now();
        let at = |millis: u64| start + Duration::from_millis(millis);
        let mut places = Places::new(start);
        // Each anchor changes at most once in its interval, and the space eight at once, then four a second.
        assert!(places.ready(start, "a"));
        assert!(places.make(Change::Place(rack("a", 0.0))));
        places.charge(start, "a");
        for index in 1..8 {
            let name = format!("r{index}");
            assert!(places.ready(start, &name));
            places.charge(start, &name);
        }
        assert!(!places.ready(start, "r9"));
        assert!(places.ready(at(250), "r9"));
        assert!(!places.ready(at(500), "a"));
        assert!(places.ready(at(900), "a"));
        // The same again changes nothing; moved, it does.
        assert!(!places.make(Change::Place(rack("a", 0.0))));
        assert!(places.make(Change::Place(rack("a", 1.0))));
        // A change waiting gives way to a later one of its anchor, and a removal of one not placed cancels its
        // placement, waiting for nothing.
        places.wait(Change::Place(rack("b", 0.0)));
        places.wait(Change::Place(rack("b", 2.0)));
        assert_eq!(places.waiting, [Change::Place(rack("b", 2.0))]);
        assert!(places.has("b") && places.has("a") && !places.has("c"));
        assert_eq!(places.count(), 2);
        places.wait(Change::Remove("b".into()));
        places.wait(Change::Remove("c".into()));
        assert!(places.waiting.is_empty());
        // A removal of one placed waits, and no longer counts it.
        places.wait(Change::Remove("a".into()));
        assert_eq!(places.count(), 0);
        assert_eq!(places.waiting[0].name(), "a");
        assert!(places.make(Change::Remove("a".into())));
        assert!(!places.make(Change::Remove("a".into())));
        // Calls, whatever they change, are held to a rate too.
        let mut calls = Places::new(start);
        for _ in 0..PLACE_BURST as usize {
            assert_eq!(calls.call(start), Ok(()));
        }
        assert!(calls.call(start).unwrap_err().contains("faster"));
        assert_eq!(calls.call(at(100)), Ok(()));
        // A channel without anchors placed has none.
        assert!(Channel::new(1, "yard", 1).anchors().is_empty());
    }

    #[test]
    fn a_side_s_updates_are_held_to_thirty_a_second_after_a_burst_of_sixty() {
        let now = Instant::now();
        let mut updates = Updates::new(now);
        for _ in 0..tomori_environment::UPDATE_BURST {
            assert_eq!(updates.take(now), Ok(()));
        }
        assert!(updates.take(now).unwrap_err().contains("faster"));
        // A second later, thirty more, and no more.
        let later = now + Duration::from_secs(1);
        for _ in 0..tomori_environment::UPDATES_PER_SECOND {
            assert_eq!(updates.take(later), Ok(()));
        }
        assert!(updates.take(later).is_err());
        // Twenty a second, as a side saying where what moves is does, is never refused.
        let mut steady = Updates::new(now);
        for tick in 0..2_000u64 {
            assert_eq!(steady.take(now + Duration::from_millis(tick * 50)), Ok(()));
        }
    }
}
