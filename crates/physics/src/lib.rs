// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Rigid-body physics for Tomori's environments: loose objects that participants take with either hand, carry, throw
//! and stack, simulated once, by the environment's server side, and drawn alike by every client. See
//! `design/physics.md`.
//!
//! **Authority follows the hand.** Whoever holds a body controls it: their own client draws it from their hand every
//! frame, and every other client from the hand their presence already brings, by the offset it was taken at, so holding
//! costs nothing on the wire. Let go, the server simulates it, with Rapier, and sends the placements of what moves 20
//! times a second, [`message::Update::moving`], and once where each comes to rest, [`message::Update::rested`], snapping
//! the body to exactly that, so that the server, what it keeps and every client agree to the bit. A body at rest costs
//! nothing until something wakes it.
//!
//! **Halves.** What both halves share is here without the simulation: placements and their encoding
//! ([`placement`]), what an environment declares ([`body`]), what the side shares, announces and is asked
//! ([`message`]), and the client's mirror of it, which interpolates and foresees ([`mirror`]). The simulation itself,
//! [`world`], is behind the `simulation` feature, on by default, which only a server needs: a client depends on this
//! crate without its default features, and never simulates.
//!
//! Nothing here knows of stones or sticks: an environment's own crates declare those.
pub mod body;
pub mod message;
pub mod mirror;
pub mod placement;
#[cfg(feature = "simulation")]
pub mod world;

pub use body::{Body, Kind, Settings, Static, Water};
pub use message::{Held, Seen, Side};
pub use placement::{Encoded, Placement, Precision};

/// The most bodies a world holds, so that a snapshot of every one of them moving fits one 4 KiB event.
pub const BODY_LIMIT: usize = 64;
/// The most points of each body's hull, and of a static hull.
pub const HULL_POINTS: usize = 32;
/// The most triangles a world's static colliders hold together, a hull or a box counting as twelve.
pub const STATIC_TRIANGLES: usize = 16_384;
/// How many times a second, at most, the world says where what moves is.
pub const SNAPSHOT_RATE: u32 = 20;
/// How many bodies each participant may hold at once: one in each hand.
pub const HELD_LIMIT: usize = 2;
/// How fast a body may be let go, in metres a second, and how fast it may spin, in radians a second: a release beyond
/// either is cut to it.
pub const RELEASE_SPEED: f32 = 8.0;
pub const RELEASE_SPIN: f32 = 20.0;
/// How far from the holder's head, as the server last saw it, a body may be let go: one let go farther away is let go
/// where the server saw their hand hold it instead.
pub const REACH: f32 = 1.5;
/// How far from a hand a body may be taken: within reach of the hand, or pulled to it along its ray from this far.
pub const PULL: f32 = 3.0;
/// How far from the hand a held body may be held, at most.
pub const HOLD: f32 = 0.5;
/// How long a body takes to fly to the hand that pulls it, in seconds.
pub const PULL_TIME: f32 = 0.25;
/// How long a participant's presence may say nothing before what they hold is let go, in milliseconds.
pub const SILENT: u64 = 3_000;
/// How far behind the clock a client draws what moves, in milliseconds, so that it interpolates between snapshots.
pub const DELAY: u64 = 100;
/// How long the holder's foresight of what they let go blends into the server's placements, in milliseconds.
pub const BLEND: u64 = 150;

/// Why the world refused what it was given or asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// More than [`BODY_LIMIT`] bodies.
    Bodies,
    /// Two bodies with this number.
    Duplicate(u32),
    /// The body's hull has fewer than 4 or more than [`HULL_POINTS`] points.
    Hull(u32),
    /// The body is not finite, or not made of anything, or its home is not a placement.
    Body(u32),
    /// A static collider that is not finite, or has too few points, or no triangles.
    Static,
    /// The static colliders hold more than [`STATIC_TRIANGLES`] triangles.
    Triangles,
    /// The world has no body with this number.
    Unknown(u32),
    /// The body has moved on from the epoch asked about: someone took or let go of it since.
    Epoch(u32),
    /// The body is held by someone else, or not by whoever lets go of it.
    Held(u32),
    /// The participant already holds what they may, or something in that hand.
    Hands,
    /// The body is out of the participant's reach, or the server has not seen where they are.
    Reach(u32),
    /// The body is only grabbable, and nobody holds it to let go.
    Kinematic(u32),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bodies => write!(formatter, "more than {BODY_LIMIT} bodies"),
            Self::Duplicate(id) => write!(formatter, "two bodies numbered {id}"),
            Self::Hull(id) => write!(
                formatter,
                "body {id}'s hull has fewer than 4 or more than {HULL_POINTS} points"
            ),
            Self::Body(id) => write!(formatter, "body {id} is not one the world can simulate"),
            Self::Static => write!(formatter, "a static collider the world cannot take"),
            Self::Triangles => write!(
                formatter,
                "more than {STATIC_TRIANGLES} triangles of static colliders"
            ),
            Self::Unknown(id) => write!(formatter, "no body numbered {id}"),
            Self::Epoch(id) => write!(formatter, "body {id} has moved on"),
            Self::Held(id) => write!(formatter, "body {id} is held by someone else"),
            Self::Hands => write!(formatter, "both hands are full"),
            Self::Reach(id) => write!(formatter, "body {id} is out of reach"),
            Self::Kinematic(id) => write!(formatter, "body {id} is held by nobody"),
        }
    }
}

impl std::error::Error for Refused {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_of_every_body_moving_fits_one_event() {
        let moving = message::Update {
            moving: (0..BODY_LIMIT as u32)
                .map(|id| message::Snapshot::new(id, &Placement::IDENTITY, Precision::Moving))
                .map(|mut snapshot| {
                    // As large as each can be written: a body 30 m away, turned every way.
                    snapshot.0[1..].copy_from_slice(&[
                        -30_000, -30_000, -30_000, -1_000, -1_000, -1_000, 1_000,
                    ]);
                    snapshot
                })
                .collect(),
            ..Default::default()
        };
        let event = serde_json::json!({"clock": u64::MAX, "physics": moving});
        assert!(serde_json::to_string(&event).unwrap().len() < 4 * 1024);
    }

    #[test]
    fn every_refusal_says_why() {
        for (refused, says) in [
            (Refused::Bodies, "more than 64 bodies"),
            (Refused::Duplicate(2), "two bodies numbered 2"),
            (Refused::Hull(2), "fewer than 4 or more than 32 points"),
            (Refused::Body(2), "not one the world can simulate"),
            (Refused::Static, "static collider"),
            (Refused::Triangles, "more than 16384 triangles"),
            (Refused::Unknown(2), "no body numbered 2"),
            (Refused::Epoch(2), "moved on"),
            (Refused::Held(2), "held by someone else"),
            (Refused::Hands, "both hands are full"),
            (Refused::Reach(2), "out of reach"),
            (Refused::Kinematic(2), "held by nobody"),
        ] {
            assert!(refused.to_string().contains(says), "{refused:?}");
        }
        let error: &dyn std::error::Error = &Refused::Bodies;
        assert!(error.source().is_none());
    }
}
