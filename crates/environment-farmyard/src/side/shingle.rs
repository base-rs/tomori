// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The beach's stones as the farmyard's server side keeps them: a `tomori-physics` world of them, settled as the side
//! starts; participants' taking and letting go, each checked against where the side last saw them; whatever a
//! participant who has left, or whose presence has gone quiet, held, let go where their hand was; and what changed,
//! gathered and said at most [`tomori_physics::SNAPSHOT_RATE`] times a second. See `crate::beach`.
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use tomori_environment::Presence;
use tomori_physics::message::{Intent, State, Update};
use tomori_physics::world::World;
use tomori_physics::{Placement, SILENT, SNAPSHOT_RATE, Seen};

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

/// The beach's world, as the side keeps it.
pub struct Shingle {
    world: World,
    /// When it last stepped, and when it last said what changed.
    stepped: Instant,
    said: Option<Instant>,
    /// What changed that it has not yet said.
    pending: Update,
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
        let seen = seen(presence, participant);
        let done = match intent {
            Intent::Take(take) => self.world.take(participant, &take, seen.as_ref()),
            Intent::LetGo(let_go) => self.world.let_go(participant, &let_go, seen.as_ref()),
        };
        if let Err(refused) = done {
            log::debug!("A stone's intent was refused: {refused}.");
        }
        true
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
                self.world
                    .let_go_of(holder, seen(presence, holder).as_ref());
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
