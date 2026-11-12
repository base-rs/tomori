// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The beach's stones as the farmyard's server side keeps them: a `tomori-physics` world of them, settled as the side
//! starts; participants' taking and letting go, each checked against where the side last saw them; whatever a
//! participant who has left, or whose presence has gone quiet, held, let go where their hand was; what changed,
//! gathered and said at most [`tomori_physics::SNAPSHOT_RATE`] times a second; and, in the server's log, each taking and
//! letting go, taken or refused and why, at most [`LOG_BURST`] at once and [`LOG_RATE`] a second. See `crate::beach`.
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use tomori_environment::Presence;
use tomori_physics::message::{Intent, State, Update};
use tomori_physics::world::World;
use tomori_physics::{Placement, Refused, SILENT, SNAPSHOT_RATE, Seen, Side};

use crate::beach;

/// How long the side lets the stones settle as it starts, at most, in simulated seconds.
pub const SETTLE: f32 = 10.0;

/// Where participant `participant` was, as the side was last told, if it was.
pub fn seen(presence: &[Presence], participant: u32) -> Option<Seen> {
    let presence = presence
        .iter()
        .find(|presence| presence.participant == participant)?;
    let placed = |pose: &tomori_protocol::Pose| {
        Placement::new(
            Vec3::from(pose.position),
            Quat::from_array(pose.orientation).normalize(),
        )
    };
    Some(Seen {
        head: Vec3::from(presence.head.position),
        hands: [
            presence.left.as_ref().map(placed),
            presence.right.as_ref().map(placed),
        ],
    })
}

/// How many lines of the stones' takings and lettings go the side logs at once, and how many a second after, so that
/// someone grabbing as fast as they can fills no log: those beyond are counted, and the next line logged says how many.
pub const LOG_BURST: f32 = 30.0;
pub const LOG_RATE: f32 = 2.0;

/// The lines the side may still log of the stones: a bucket of [`LOG_BURST`], refilled at [`LOG_RATE`] a second, and
/// how many it did not log since the last it did.
#[derive(Debug)]
pub struct Logged {
    tokens: f32,
    at: Instant,
    skipped: u64,
}

impl Logged {
    pub fn new(now: Instant) -> Self {
        Self {
            tokens: LOG_BURST,
            at: now,
            skipped: 0,
        }
    }

    /// `line`, to be logged at `now`: with how many before it were not, if it is to be, or `None`, counted.
    pub fn line(&mut self, line: String, now: Instant) -> Option<String> {
        let elapsed = now
            .saturating_duration_since(self.at)
            .min(Duration::from_secs(60));
        self.at = self.at.max(now);
        self.tokens = (self.tokens + elapsed.as_secs_f32() * LOG_RATE).min(LOG_BURST);
        if self.tokens < 1.0 {
            self.skipped += 1;
            return None;
        }
        self.tokens -= 1.0;
        Some(match std::mem::take(&mut self.skipped) {
            0 => line,
            skipped => format!(
                "{line} ({skipped} more of the stones' takings and lettings go were not logged.)"
            ),
        })
    }
}

/// The name of a hand, as the log says it.
fn hand(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// The beach's world, as the side keeps it.
pub struct Shingle {
    world: World,
    /// When it last stepped, and when it last said what changed.
    stepped: Instant,
    said: Option<Instant>,
    /// What changed that it has not yet said.
    pending: Update,
    /// The lines it may still log.
    logged: Logged,
}

impl Shingle {
    /// The beach's stones, settled where they lie, at `now`.
    pub fn new(now: Instant) -> Self {
        let mut world = World::new(
            beach::settings(),
            beach::colliders(),
            beach::waters(),
            beach::bodies(),
        )
        .expect("the beach is within tomori-physics's limits");
        world.settle(SETTLE);
        Self {
            world,
            stepped: now,
            said: None,
            pending: Update::default(),
            logged: Logged::new(now),
        }
    }

    /// `line`, logged, unless too many have been lately.
    fn log(&mut self, line: String) {
        if let Some(line) = self.logged.line(line, Instant::now()) {
            log::info!("{line}");
        }
    }

    /// Why the world refused what participant `by` asked of a body, as the log says it.
    fn why(&self, by: u32, refused: Refused) -> String {
        match refused {
            Refused::Epoch(body) => format!(
                "it has moved on to epoch {}",
                self.world.epoch(body).unwrap_or_default()
            ),
            Refused::Held(body) => match self.world.held(body) {
                Some(held) if held.by == by => {
                    format!("they hold it in their {} hand", hand(held.hand))
                }
                Some(held) => format!("participant {} holds it", held.by),
                None => "nobody holds it".to_owned(),
            },
            Refused::Hands => "they hold something in that hand, or two already".to_owned(),
            Refused::Reach(_) => {
                "it is out of their reach, or the server has not seen where they are".to_owned()
            }
            refused => refused.to_string(),
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// Every stone, as everyone in the space is told of them.
    pub fn shared(&self) -> State {
        self.world.shared()
    }

    /// Do what `participant` asks, `payload`, if it is about the stones, with where the side last saw everyone,
    /// `presence`; says whether it was. What the world refuses, as a stone someone else took first, changes nothing.
    pub fn ask(&mut self, participant: u32, payload: &str, presence: &[Presence]) -> bool {
        let Ok(intent) = serde_json::from_str::<Intent>(payload) else {
            return false;
        };
        let line = self.answer(participant, intent, presence);
        self.log(line);
        true
    }

    /// Do what `participant` asks, `intent`, with where the side last saw everyone, `presence`; says what it did, or
    /// what it refused and why, as the log says it.
    fn answer(&mut self, participant: u32, intent: Intent, presence: &[Presence]) -> String {
        let seen = seen(presence, participant);
        match intent {
            Intent::Take(take) => {
                let again = self
                    .world
                    .held(take.body)
                    .is_some_and(|held| held.by == participant);
                let side = hand(take.hand);
                match self.world.take(participant, &take, seen.as_ref()) {
                    Ok(epoch) if again => format!(
                        "Participant {participant} took stone {} again, with their {side} hand, at epoch {epoch}, \
                         as their letting go of it never arrived.",
                        take.body
                    ),
                    Ok(epoch) => format!(
                        "Participant {participant} took stone {} with their {side} hand, at epoch {epoch}.",
                        take.body
                    ),
                    Err(refused) => format!(
                        "Participant {participant}'s taking of stone {} with their {side} hand, at epoch {}, was \
                         refused: {}.",
                        take.body,
                        take.epoch,
                        self.why(participant, refused)
                    ),
                }
            }
            Intent::LetGo(let_go) => match self.world.let_go(participant, &let_go, seen.as_ref()) {
                Ok(epoch) => {
                    let at = self
                        .world
                        .placement(let_go.body)
                        .map_or(glam::Vec3::NAN, |at| at.position);
                    let speed = glam::Vec3::from(let_go.velocity).length();
                    format!(
                        "Participant {participant} let go of stone {} at ({:.2}, {:.2}, {:.2}), moving at \
                             {speed:.1} m/s, at epoch {epoch}.",
                        let_go.body, at.x, at.y, at.z
                    )
                }
                Err(refused) => format!(
                    "Participant {participant}'s letting go of stone {}, at epoch {}, was refused: {}.",
                    let_go.body,
                    let_go.epoch,
                    self.why(participant, refused)
                ),
            },
        }
    }

    /// Let go of what everyone who has gone holds, as `present` says, by their numbers, if the server has said, or
    /// whose presence has said nothing for [`SILENT`], at `now`: where their hand last held it, unmoving.
    pub fn sweep(&mut self, present: Option<&[u32]>, presence: &[Presence], now: Instant) {
        for holder in self.world.holders() {
            let gone = present.is_some_and(|present| !present.contains(&holder));
            let silent = presence
                .iter()
                .find(|presence| presence.participant == holder)
                .is_none_or(|presence| {
                    now.saturating_duration_since(presence.at) >= Duration::from_millis(SILENT)
                });
            if gone || silent {
                let stones = self
                    .world
                    .let_go_of(holder, seen(presence, holder).as_ref());
                let why = if gone { "left the space" } else { "went quiet" };
                self.log(format!(
                    "Participant {holder} {why}, so the stones they held, {stones:?}, were let go where their hand was."
                ));
            }
        }
    }

    /// Move on to `now`, and what to say of what changed: whatever has not been said, once at least a snapshot's
    /// interval has passed since it last said, so that it says at most [`SNAPSHOT_RATE`] a second.
    pub fn update(&mut self, now: Instant) -> Option<Update> {
        let elapsed = now.saturating_duration_since(self.stepped).as_secs_f32();
        self.stepped = now;
        self.pending.merge(self.world.step(elapsed));
        let interval = Duration::from_millis(1000 / SNAPSHOT_RATE as u64);
        let due = self
            .said
            .is_none_or(|said| now.saturating_duration_since(said) >= interval);
        if self.pending.is_empty() || !due {
            return None;
        }
        self.said = Some(now);
        Some(std::mem::take(&mut self.pending))
    }

    /// What the space's agent is told of the beach: one object, its identity, reference, description and where it is.
    pub fn describe(&self) -> (String, String, String, Vec3) {
        let middle = beach::at((beach::NEAR + beach::FAR) * 0.5, 0.0);
        let held = self
            .world
            .bodies()
            .filter(|body| self.world.held(*body).is_some())
            .count();
        let description = match held {
            0 => format!(
                "the shingle beach by the river, where {} river stones lie to pick up, throw and stack",
                beach::STONES
            ),
            held => format!(
                "the shingle beach by the river, where {} river stones lie to pick up, throw and stack, {held} in someone's hand",
                beach::STONES
            ),
        };
        (
            "beach".to_owned(),
            "beach/shingle".to_owned(),
            description,
            Vec3::new(middle.x, beach::height(middle).unwrap_or(0.0), middle.y),
        )
    }
}

/// What the agent is told of the beach when it inspects it, from what the side last shared of its stones.
pub fn inspect(shared: &State) -> serde_json::Value {
    let held = shared
        .bodies
        .iter()
        .filter(|body| body.held.is_some())
        .count();
    let moving = shared.bodies.iter().filter(|body| body.moving).count();
    serde_json::json!({
        "kind": "shingle beach",
        "stones": shared.bodies.len(),
        "held": held,
        "moving": moving,
    })
}

#[cfg(test)]
mod tests;
