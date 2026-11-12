// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Loose bodies as the farmyard's server side keeps them, whatever they are, the beach's stones or the campsite's
//! sticks: a `tomori-physics` world of them; participants' taking and letting go, each checked against where the side
//! last saw them; whatever a participant who has left, or whose presence has gone quiet, held, let go where their hand
//! was; what changed, gathered and said at most [`tomori_physics::SNAPSHOT_RATE`] times a second; and, in the server's
//! log, each taking, passing from one hand to the other and letting go, taken or refused and why, at most [`LOG_BURST`] at once and [`LOG_RATE`] a second.
//! See `design/physics.md`.
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use tomori_environment::Presence;
use tomori_physics::message::{Intent, State, Update};
use tomori_physics::world::World;
use tomori_physics::{Placement, Refused, SILENT, SNAPSHOT_RATE, Seen, Side};

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

/// How many lines of the takings and lettings go the side logs at once, and how many a second after, so that someone
/// grabbing as fast as they can fills no log: those beyond are counted, and the next line logged says how many.
pub const LOG_BURST: f32 = 30.0;
pub const LOG_RATE: f32 = 2.0;

/// What the bodies are called in the log: one, by its number, and all of them, as "the stones'".
#[derive(Clone, Copy, Debug)]
pub struct Noun {
    pub one: fn(u32) -> String,
    pub many: &'static str,
}

/// The lines the side may still log of its bodies: a bucket of [`LOG_BURST`], refilled at [`LOG_RATE`] a second, and
/// how many it did not log since the last it did.
#[derive(Debug)]
pub struct Logged {
    many: &'static str,
    tokens: f32,
    at: Instant,
    pub(crate) skipped: u64,
}

impl Logged {
    /// The lines of `many`, as "stones", at `now`.
    pub fn new(many: &'static str, now: Instant) -> Self {
        Self {
            many,
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
                "{line} ({skipped} more of the {}' takings and lettings go were not logged.)",
                self.many
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

/// A world of loose bodies, as the side keeps it.
pub struct Loose {
    world: World,
    noun: Noun,
    /// When it last stepped, and when it last said what changed.
    stepped: Instant,
    said: Option<Instant>,
    /// What changed that it has not yet said.
    pending: Update,
    /// The lines it may still log.
    pub(crate) logged: Logged,
    /// The bodies let go since they were last asked for, as participants let go of them.
    released: Vec<u32>,
}

impl Loose {
    /// The bodies of `world`, called as `noun` says, at `now`.
    pub fn new(world: World, noun: Noun, now: Instant) -> Self {
        Self {
            world,
            noun,
            stepped: now,
            said: None,
            pending: Update::default(),
            logged: Logged::new(noun.many, now),
            released: Vec::new(),
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

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Every body, as everyone in the space is told of them.
    pub fn shared(&self) -> State {
        self.world.shared()
    }

    /// Do what `participant` asks, `payload`, if it is a taking or letting go of a body, with where the side last saw
    /// everyone, `presence`; says whether it was. What the world refuses, as a body someone else took first, changes
    /// nothing.
    pub fn ask(&mut self, participant: u32, payload: &str, presence: &[Presence]) -> bool {
        let Ok(intent) = serde_json::from_str::<Intent>(payload) else {
            return false;
        };
        let line = self.answer(participant, intent, presence);
        self.log(line);
        true
    }

    /// The bodies participants let go of since this was last asked, in turn.
    pub fn released(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.released)
    }

    /// Do what `participant` asks, `intent`, with where the side last saw everyone, `presence`; says what it did, or
    /// what it refused and why, as the log says it.
    pub(crate) fn answer(
        &mut self,
        participant: u32,
        intent: Intent,
        presence: &[Presence],
    ) -> String {
        let seen = seen(presence, participant);
        let one = self.noun.one;
        match intent {
            Intent::Take(take) => {
                let again = self
                    .world
                    .held(take.body)
                    .is_some_and(|held| held.by == participant);
                let passing = self.world.passing(participant, &take);
                let side = hand(take.hand);
                let body = one(take.body);
                match self.world.take(participant, &take, seen.as_ref()) {
                    Ok(epoch) if passing => format!(
                        "Participant {participant} passed {body} to their {side} hand, at epoch {epoch}."
                    ),
                    Ok(epoch) if again => format!(
                        "Participant {participant} took {body} again, with their {side} hand, at epoch {epoch}, as \
                         their letting go of it never arrived."
                    ),
                    Ok(epoch) => format!(
                        "Participant {participant} took {body} with their {side} hand, at epoch {epoch}."
                    ),
                    Err(refused) => format!(
                        "Participant {participant}'s taking of {body} with their {side} hand, at epoch {}, was \
                         refused: {}.",
                        take.epoch,
                        self.why(participant, refused)
                    ),
                }
            }
            Intent::LetGo(let_go) => {
                let body = one(let_go.body);
                match self.world.let_go(participant, &let_go, seen.as_ref()) {
                    Ok(epoch) => {
                        self.released.push(let_go.body);
                        let at = self
                            .world
                            .placement(let_go.body)
                            .map_or(Vec3::NAN, |at| at.position);
                        let speed = Vec3::from(let_go.velocity).length();
                        format!(
                            "Participant {participant} let go of {body} at ({:.2}, {:.2}, {:.2}), moving at \
                             {speed:.1} m/s, at epoch {epoch}.",
                            at.x, at.y, at.z
                        )
                    }
                    Err(refused) => format!(
                        "Participant {participant}'s letting go of {body}, at epoch {}, was refused: {}.",
                        let_go.epoch,
                        self.why(participant, refused)
                    ),
                }
            }
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
                let bodies = self
                    .world
                    .let_go_of(holder, seen(presence, holder).as_ref());
                let names: Vec<String> = bodies.iter().map(|body| (self.noun.one)(*body)).collect();
                let why = if gone { "left the space" } else { "went quiet" };
                self.log(format!(
                    "Participant {holder} {why}, so what they held, {}, was let go where their hand was.",
                    names.join(" and ")
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
}

#[cfg(test)]
mod tests;
