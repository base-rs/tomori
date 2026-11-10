// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whether a device may ask to pair: only during a short window that an owner or the operator opens, which closes once a device pairs, when its time is up, or when someone closes it. Pairing is closed otherwise, also on a server with no owner yet, whose first device pairs in a window the operator opens. See `design/security.md`.
use std::time::{Duration, Instant, SystemTime};

use tomori_protocol::ServerMessage;

use super::{Deferred, Hub, HubState, PairingEvent, PairingSetup};
use crate::access::Grant;

/// How long pairing stays open once an owner or the operator opens it: long enough to pick up the device and choose its pairing object, and short enough that an opening nobody uses soon closes. A pairing begun within it may still be confirmed for as long as it waits, [`super::PAIRING_TIMEOUT`]. See `design/security.md`.
pub const PAIRING_OPEN_FOR: Duration = Duration::from_secs(120);

/// How often the console and the log tell of the requests to pair refused while pairing is closed, at most, so that a device asking again and again cannot fill them.
pub const REFUSALS_TOLD_EVERY: Duration = Duration::from_secs(60);

/// Who opened pairing, and where, as the console, the log, the status page and every space are told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opener {
    /// Who: an owner by the name the server knows them by, or "The operator".
    pub who: String,
    /// Where, as words that follow "opened pairing": "from a paired device", "on the status page", "on the server's console".
    pub place: &'static str,
}

impl Opener {
    /// An owner, by the name the server knows them by, from a device they paired.
    pub fn owner(name: &str) -> Self {
        Self {
            who: name.to_owned(),
            place: "from a paired device",
        }
    }

    /// An owner on the status page, by the name of their token, if it has one.
    pub fn status_page(name: Option<&str>) -> Self {
        Self {
            who: name.unwrap_or("An owner").to_owned(),
            place: "on the status page",
        }
    }

    /// The operator, typing on the server's console.
    pub fn console() -> Self {
        Self {
            who: "The operator".to_owned(),
            place: "on the server's console",
        }
    }

    /// The operator, starting the server with `--open-pairing`.
    pub fn startup() -> Self {
        Self {
            who: "The operator".to_owned(),
            place: "at startup, with --open-pairing",
        }
    }
}

/// An open window: until when, and who opened it.
pub(super) struct Window {
    until: Instant,
    by: Opener,
}

/// Requests to pair refused while pairing was closed, not yet told: told at most every [`REFUSALS_TOLD_EVERY`].
#[derive(Default)]
pub(super) struct Refusals {
    told: Option<Instant>,
    untold: u32,
}

/// Why pairing closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Closing {
    /// Someone closed it, as the words that follow "Pairing is closed:" say.
    Closed(String),
    /// Its time was up.
    Expired,
    /// A device paired, which uses the window.
    Paired,
}

/// Whether a device may ask to pair, for the status page. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingState {
    /// Closed: a device asking to pair is refused. Opening it would open it for `opens_for`.
    Closed { opens_for: Duration },
    /// Open for `remaining` more, as `by` opened it.
    Open { remaining: Duration, by: Opener },
}

/// A duration as a sentence says it: "2 minutes", "1 minute", "90 seconds".
pub fn spoken(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let plural =
        |count: u64, unit: &str| format!("{count} {unit}{}", if count == 1 { "" } else { "s" });
    if seconds >= 60 && seconds.is_multiple_of(60) {
        plural(seconds / 60, "minute")
    } else {
        plural(seconds, "second")
    }
}

impl HubState {
    /// The seconds the window has left, at least one while it is open, as an owner's client is told; `None` while it is closed.
    fn closes_in(&self, now: Instant) -> Option<u32> {
        self.window.as_ref().map(|window| {
            let left = window.until.saturating_duration_since(now);
            u32::try_from(left.as_secs() + u64::from(left.subsec_nanos() > 0))
                .unwrap_or(u32::MAX)
                .max(1)
        })
    }

    /// Tell every owner's connection that reads it whether pairing is open now.
    fn tell_owners_of_window(&self, now: Instant) {
        let closes_in = self.closes_in(now);
        for record in self.connections.values().filter(|record| {
            record.pairing_window
                && record.participant.is_some()
                && record.access.grant == Grant::Owner
        }) {
            record
                .outbox
                .message(ServerMessage::PairingWindow { closes_in });
        }
    }

    /// Close the window, if one is open, for `closing`, telling owners, the console and the log. A device pairing after its window's time was up, as the sweep closed it, finds none.
    fn close_window(&mut self, closing: Closing, now: Instant, deferred: &mut Vec<Deferred>) {
        if self.window.take().is_some() {
            self.tell_owners_of_window(now);
            let event = PairingEvent::Closed { closing };
            log::info!("{event}");
            deferred.push(Deferred::PairingPrompt(event));
        }
    }

    /// Count a request to pair refused while pairing is closed, and tell of those not yet told if it is time.
    fn refused_while_closed(&mut self, now: Instant, deferred: &mut Vec<Deferred>) {
        self.refusals.untold += 1;
        self.tell_refusals(now, deferred);
    }

    /// Tell of the refusals not yet told, unless they were told less than [`REFUSALS_TOLD_EVERY`] ago.
    fn tell_refusals(&mut self, now: Instant, deferred: &mut Vec<Deferred>) {
        let due = self
            .refusals
            .told
            .is_none_or(|told| now.saturating_duration_since(told) >= REFUSALS_TOLD_EVERY);
        if self.refusals.untold == 0 || !due {
            return;
        }
        let event = PairingEvent::Refused {
            count: std::mem::take(&mut self.refusals.untold),
        };
        self.refusals.told = Some(now);
        log::info!("{event}");
        deferred.push(Deferred::PairingPrompt(event));
    }
}

impl Hub {
    /// How long pairing stays open once opened; see [`PAIRING_OPEN_FOR`].
    pub(crate) fn set_pairing_open_for(&self, length: Duration) {
        *self.pairing_open_for.write().expect("never left poisoned") = length;
    }

    fn pairing_setup(&self) -> Option<PairingSetup> {
        self.pairing.read().expect("never left poisoned").clone()
    }

    /// Whether a device may ask to pair at `now`: only while a window is open, whether or not the server has an owner yet. A device that asks otherwise is refused at once with [`tomori_protocol::PAIRING_CLOSED`]. See `design/security.md`.
    pub(super) fn pairing_open(&self, state: &HubState, now: Instant) -> bool {
        state
            .window
            .as_ref()
            .is_some_and(|window| now < window.until)
    }

    /// Refuse a request to pair while pairing is closed, telling the console and the log at most every [`REFUSALS_TOLD_EVERY`].
    pub(super) fn refuse_closed(
        &self,
        state: &mut HubState,
        now: Instant,
        deferred: &mut Vec<Deferred>,
    ) {
        state.refused_while_closed(now, deferred);
    }

    /// Whether a device may ask to pair, for the status page; `None` on a server that does not pair, as in `--auth open` mode.
    pub fn pairing_state(&self) -> Option<PairingState> {
        self.pairing_setup()?;
        let state = self.lock();
        let now = Instant::now();
        Some(match &state.window {
            Some(window) => PairingState::Open {
                remaining: window.until.saturating_duration_since(now),
                by: window.by.clone(),
            },
            None => PairingState::Closed {
                opens_for: *self.pairing_open_for.read().expect("never left poisoned"),
            },
        })
    }

    /// Open pairing for [`PAIRING_OPEN_FOR`], as `by` asked, or open it afresh for that long if it is open: tell owners, the console and the log, and put a notice in every open space, so that a window nobody expected is seen. Returns how long it is open for.
    pub(crate) fn open_pairing(&self, by: Opener) -> Result<Duration, &'static str> {
        let mut deferred = Vec::new();
        let opened = {
            let mut state = self.lock();
            self.open_window(&mut state, by, &mut deferred)
        };
        self.run(deferred);
        opened
    }

    /// [`Self::open_pairing`], under the lock.
    pub(super) fn open_window(
        &self,
        state: &mut HubState,
        by: Opener,
        deferred: &mut Vec<Deferred>,
    ) -> Result<Duration, &'static str> {
        if self.pairing_setup().is_none() {
            return Err("This server does not pair devices.");
        }
        let length = *self.pairing_open_for.read().expect("never left poisoned");
        let now = Instant::now();
        state.window = Some(Window {
            until: now + length,
            by: by.clone(),
        });
        state.tell_owners_of_window(now);
        // Every space is told, so that a window opened by someone else, or by nobody here, is noticed.
        let body = format!(
            "{} opened pairing {} for {}, so that a new device can ask to pair with this server. If nobody expected this, an owner can close it in People & security or on the status page.",
            by.who,
            by.place,
            spoken(length)
        );
        let environments = self.environments();
        let mut told = Vec::new();
        for record in state
            .spaces
            .iter_mut()
            .filter(|record| !record.model.is_closed())
        {
            let walls = environments.walls(&record.model.environment);
            let content = crate::stage::Content {
                title: "Pairing".to_owned(),
                body: body.clone(),
                meta: vec![("source".to_owned(), "Tomori".to_owned())],
                code: None,
                slug: None,
                image: None,
            };
            // Opening it again replaces the notice rather than adding another. See `design/projection.md`.
            let panel = record.stage.project_notice(
                &walls,
                "pairing-window",
                content,
                crate::cards::unix_now(),
            );
            log::info!("Space {} projected {panel}.", record.model.id);
            told.push(record.model.id.clone());
        }
        for space in &told {
            state.touch_projection(space);
        }
        if !told.is_empty() {
            deferred.push(Deferred::Keep);
        }
        let event = PairingEvent::Opened {
            by,
            length,
            closes: SystemTime::now() + length,
        };
        log::info!("{event}");
        deferred.push(Deferred::PairingPrompt(event));
        Ok(length)
    }

    /// Close pairing, withdrawing a pairing waiting, and tell owners, the console and the log `how` it closed, such as "Ada closed it.". Returns whether it was open.
    pub(crate) fn close_pairing(&self, how: String) -> bool {
        let mut deferred = Vec::new();
        let closed = {
            let mut state = self.lock();
            self.close_window(&mut state, how, &mut deferred)
        };
        self.run(deferred);
        closed
    }

    /// [`Self::close_pairing`], under the lock.
    pub(super) fn close_window(
        &self,
        state: &mut HubState,
        how: String,
        deferred: &mut Vec<Deferred>,
    ) -> bool {
        if state.window.is_none() {
            return false;
        }
        if state.end_pairing(None, "Pairing was closed, so the pairing was withdrawn.") {
            log::info!("A pairing was withdrawn, since pairing was closed.");
        }
        state.close_window(Closing::Closed(how), Instant::now(), deferred);
        true
    }

    /// A device paired: close the window it used, which a pairing begun within it holds open until it ends.
    pub(super) fn paired_closes_pairing(&self) {
        let mut deferred = Vec::new();
        self.lock()
            .close_window(Closing::Paired, Instant::now(), &mut deferred);
        self.run(deferred);
    }

    /// Close a window whose time is up, once no pairing begun within it waits, and tell of refusals not yet told. Called with the other timeouts.
    pub(super) fn expire_window(
        &self,
        state: &mut HubState,
        now: Instant,
        deferred: &mut Vec<Deferred>,
    ) {
        let over = state
            .window
            .as_ref()
            .is_some_and(|window| now >= window.until);
        if over && state.pairing.is_none() {
            state.close_window(Closing::Expired, now, deferred);
        }
        state.tell_refusals(now, deferred);
    }

    /// Tell a connection that has just said Hello whether pairing is open, if it is an owner's that reads it, on a server that pairs.
    pub(super) fn tell_window_on_hello(&self, state: &HubState, connection: super::ConnectionId) {
        if self.pairing_setup().is_none() {
            return;
        }
        if let Some(record) = state.connections.get(&connection)
            && record.pairing_window
            && record.access.grant == Grant::Owner
        {
            record.outbox.message(ServerMessage::PairingWindow {
                closes_in: state.closes_in(Instant::now()),
            });
        }
    }
}

/// What the console says of why pairing closed, after "Pairing is closed".
impl std::fmt::Display for Closing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed(how) => write!(formatter, ": {how}"),
            Self::Expired => formatter.write_str(": its time is up."),
            Self::Paired => formatter.write_str(": a device paired."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_said_as_a_sentence_says_them() {
        assert_eq!(spoken(Duration::from_secs(120)), "2 minutes");
        assert_eq!(spoken(Duration::from_secs(60)), "1 minute");
        assert_eq!(spoken(Duration::from_secs(90)), "90 seconds");
        assert_eq!(spoken(Duration::from_secs(1)), "1 second");
        assert_eq!(spoken(Duration::ZERO), "0 seconds");
    }

    #[test]
    fn refusals_while_closed_are_told_at_most_once_a_minute() {
        let mut state = HubState::default();
        let mut deferred = Vec::new();
        let now = Instant::now();
        let told = |deferred: &mut Vec<Deferred>| -> Vec<u32> {
            deferred
                .drain(..)
                .map(|event| {
                    let Deferred::PairingPrompt(PairingEvent::Refused { count }) = event else {
                        unreachable!("only refusals are told");
                    };
                    count
                })
                .collect()
        };
        state.refused_while_closed(now, &mut deferred);
        assert_eq!(told(&mut deferred), [1]);
        state.refused_while_closed(now + Duration::from_secs(1), &mut deferred);
        state.refused_while_closed(now + Duration::from_secs(2), &mut deferred);
        assert_eq!(told(&mut deferred), [0; 0]);
        // Those not yet told are told once a minute has passed, as the sweep finds.
        state.tell_refusals(now + Duration::from_secs(30), &mut deferred);
        assert_eq!(told(&mut deferred), [0; 0]);
        state.tell_refusals(now + REFUSALS_TOLD_EVERY, &mut deferred);
        assert_eq!(told(&mut deferred), [2]);
        // With none untold, nothing is told.
        state.tell_refusals(now + REFUSALS_TOLD_EVERY * 3, &mut deferred);
        assert_eq!(told(&mut deferred), [0; 0]);
        assert_eq!(
            PairingEvent::Refused { count: 2 }.to_string(),
            "Refused 2 requests to pair, since pairing is closed."
        );
        assert_eq!(state.closes_in(now), None);
        // Closing with no window open, as a device pairing after the sweep closed its window finds, tells nothing.
        state.close_window(Closing::Paired, now, &mut deferred);
        assert_eq!(told(&mut deferred), [0; 0]);
    }

    #[test]
    fn the_console_says_why_pairing_closed() {
        let said = |closing: Closing| PairingEvent::Closed { closing }.to_string();
        assert_eq!(
            said(Closing::Closed("Ada closed it.".into())),
            "Pairing is closed: Ada closed it."
        );
        assert_eq!(said(Closing::Expired), "Pairing is closed: its time is up.");
        assert_eq!(said(Closing::Paired), "Pairing is closed: a device paired.");
    }

    #[test]
    fn openers_say_who_and_where() {
        assert_eq!(Opener::owner("Ada").who, "Ada");
        assert_eq!(Opener::status_page(None).who, "An owner");
        assert_eq!(Opener::status_page(Some("Ada")).place, "on the status page");
        assert_eq!(Opener::console().place, "on the server's console");
        assert_eq!(Opener::startup().place, "at startup, with --open-pairing");
    }
}
