// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The beach's stones as the farmyard's server side keeps them: loose bodies ([`Loose`]) of a `tomori-physics` world
//! of them, settled as the side starts, taken and let go as any loose body is; and what the agent is told of the beach.
//! See `crate::beach`.
use std::time::Instant;

use glam::Vec3;
use tomori_physics::message::State;
use tomori_physics::world::World;

pub use super::loose::{LOG_BURST, LOG_RATE, Logged, Loose, Noun, seen};
use crate::beach;

/// How long the side lets the stones settle as it starts, at most, in simulated seconds.
pub const SETTLE: f32 = 10.0;

/// What the stones are called in the server's log.
pub const NOUN: Noun = Noun {
    one: |body| format!("stone {body}"),
    many: "stones",
};

/// The beach's world, as the side keeps it: its stones, loose bodies.
pub struct Shingle {
    loose: Loose,
}

impl std::ops::Deref for Shingle {
    type Target = Loose;

    fn deref(&self) -> &Loose {
        &self.loose
    }
}

impl std::ops::DerefMut for Shingle {
    fn deref_mut(&mut self) -> &mut Loose {
        &mut self.loose
    }
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
            loose: Loose::new(world, NOUN, now),
        }
    }

    /// What the space's agent is told of the beach: one object, its identity, reference, description and where it is.
    pub fn describe(&self) -> (String, String, String, Vec3) {
        let middle = beach::at((beach::NEAR + beach::FAR) * 0.5, 0.0);
        let world = self.world();
        let held = world
            .bodies()
            .filter(|body| world.held(*body).is_some())
            .count();
        let stones = format!(
            "the shingle beach by the river, where {} river stones lie to pick up, throw and stack, and {} big \
             building stones, flagstones and blocks, to build with",
            beach::RIVER,
            beach::BUILDING
        );
        let description = match held {
            0 => stones,
            held => format!("{stones}, {held} in someone's hand"),
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
    let building = shared
        .bodies
        .iter()
        .filter(|body| beach::building(body.body as usize))
        .count();
    serde_json::json!({
        "kind": "shingle beach",
        "stones": shared.bodies.len(),
        "building": building,
        "held": held,
        "moving": moving,
    })
}

#[cfg(test)]
mod tests;
