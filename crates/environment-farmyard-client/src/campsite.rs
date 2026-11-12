// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The campsite's marshmallow sticks as a client shows them: loose bodies ([`Loose`]), as the beach's stones are, taken
//! with either grip by their handle, pointing at them or reaching for them, held along the hand through the fist, drawn
//! from this participant's own hand every frame and from the others' hands their presence brings, and let go, thrown as
//! fast as the hand moved, foreseen until the server's snapshots take over; each marshmallow as toasted as the server
//! side says, which toasts it from where the holder's hand holds it, and, for one this participant holds, as they
//! foresee it toasting by the same rule from where they draw it, easing to what the server side says. See
//! `tomori-environment-farmyard`'s `campsite`.
use std::collections::BTreeMap;
use std::time::Instant;

use glam::{Mat4, Vec3};
use tomori_physics::Placement;
use tomori_physics::message::Intent;
use tomori_scene::client_side::{Grab, Hand, Picked, Sound};
use tomori_scene::pointing::{Ray, Shape};

pub use crate::loose::{Loose, Shapes};
use tomori_environment_farmyard::campsite::{self, Event, Pose, STICKS, Shared};

/// How near the ray passes a stick to point at it, and how much nearer for each metre away, as a thin thing is hard to
/// point at exactly from afar; and how near a hand comes to a stick's handle to take it.
const POINTED: f32 = 0.04;
const POINTED_PER_METRE: f32 = 0.012;
pub const REACH: f32 = 0.1;
/// How far ahead of what the server side says the holder's foresight of their marshmallow's toasting may run, as the
/// server side says it every half second from the hand presence brings it a moment late, and how far behind; and how
/// soon, in seconds, it eases up to what the server side says, should it fall behind, about a fifth of the way each
/// twentieth of a second.
const AHEAD: f32 = campsite::TOAST_RATE * 0.75;
const BEHIND: f32 = 0.01;
const EASE: f32 = 0.25;
/// The longest frame the foresight toasts over, in seconds.
const FRAME_MOST: f32 = 0.1;

/// A stick as it is drawn this frame: where, the shade of its marshmallow and whether it burns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub transform: Mat4,
    pub marshmallow: Vec3,
    pub shade: usize,
    pub burning: bool,
}

/// The campsite's sticks, as loose bodies are: called by their place on the log, met along their length, taken by
/// their handle and held along the hand however they were taken, and heard, lightly, as they land.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rack;

impl Shapes for Rack {
    const PREFIX: &'static str = "stick-";
    const NAME: &'static str = "a marshmallow stick";
    const REACH: f32 = REACH;

    fn number(body: u32) -> u32 {
        campsite::index(body).map_or(body, |index| index as u32)
    }

    fn body(number: u32) -> Option<u32> {
        ((number as usize) < STICKS).then(|| campsite::body(number as usize))
    }

    fn met(origin: Vec3, direction: Vec3, _body: u32, placement: &Placement) -> Option<f32> {
        let pose = Pose::of(placement);
        let distance = pose.middle().distance(origin);
        pose.met(origin, direction, POINTED + POINTED_PER_METRE * distance)
    }

    fn reach(_body: u32, placement: &Placement, hand: Vec3) -> f32 {
        Pose::of(placement).from_handle(hand)
    }

    /// By its handle, along the hand, however it was taken: flying there from where it was, as a stone pulled along the
    /// ray does, so that a stick taken anywhere along its handle settles into the fist.
    fn held(_body: u32, _at: &Placement, _hand: &Placement, _within: bool) -> (Placement, bool) {
        (campsite::grip(), true)
    }

    /// A tap of wood, softer than a stone's clack, or a splash into the pool.
    fn sound(_body: u32, water: bool, speed: f32) -> Option<Sound> {
        let speed = if water { speed } else { speed * 0.4 };
        (speed >= crate::stones::QUIET).then(|| crate::stones::sound(water, speed))
    }
}

/// The sticks as this client shows them: their bodies, how toasted the server side says each is, this participant's
/// foresight of those they hold, and when they were last drawn.
#[derive(Clone, Debug, Default)]
pub struct Sticks {
    loose: Loose<Rack>,
    toast: Vec<f32>,
    foreseen: BTreeMap<u32, f32>,
    last: Option<Instant>,
}

impl Sticks {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sticks as the server side shares them, in its state: every one anew.
    pub fn share(&mut self, shared: &Shared) {
        self.loose.share(&shared.sticks);
        self.toast = shared.toast.iter().copied().take(STICKS).collect();
        self.toast.resize(STICKS, 0.0);
    }

    /// What changed, as the server side announced it, arriving at `now` on the farmyard's clock; and the sounds of what
    /// landed.
    pub fn changed(&mut self, event: &Event, now: u64) -> Vec<(Sound, [f32; 3])> {
        for toasted in &event.toast {
            if let Some(index) = campsite::index(toasted.body)
                && let Some(toast) = self.toast.get_mut(index)
                && toasted.toast.is_finite()
            {
                *toast = toasted.toast.clamp(0.0, 1.0);
            }
        }
        self.loose.changed(event.clock, &event.sticks, now)
    }

    /// How toasted stick `index` is, as the server side last said.
    pub fn toast(&self, index: usize) -> f32 {
        self.toast.get(index).copied().unwrap_or_default()
    }

    /// The loose bodies the sticks are.
    pub fn loose(&self) -> &Loose<Rack> {
        &self.loose
    }

    /// What `ray` meets first of the campsite: a free stick it passes near, which the grip takes, or the bonfire, each
    /// with how far along the ray; nothing until the server side has said what there is.
    pub fn pick(&self, ray: &Ray) -> Option<Picked> {
        if self.toast.is_empty() {
            return None;
        }
        let bonfire = Shape::Sphere {
            center: campsite::heart(),
            radius: 0.35,
        }
        .intersect(ray)
        .map(|distance| Picked::new("bonfire", "the bonfire", distance));
        self.loose
            .pick(ray)
            .into_iter()
            .chain(bonfire)
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// The free stick whose handle a hand at `hand` is within [`REACH`] of, the nearest.
    pub fn within_reach(&self, hand: Vec3) -> Option<String> {
        self.loose.within_reach(hand)
    }

    /// A frame at `now`, the farmyard's clock at `clock`: the grips on sticks this frame, by this participant,
    /// `participant`, if they take part; what to ask the server side.
    pub fn update<'a>(
        &mut self,
        grabs: impl Iterator<Item = &'a Grab>,
        participant: Option<u32>,
        now: Instant,
        clock: u64,
    ) -> Vec<Intent> {
        self.loose.update(grabs, participant, now, clock)
    }

    /// Every stick as it is drawn at `now`, the farmyard's clock at `clock`: those this participant holds, in their
    /// hand, their marshmallows as they foresee them toasting; one another holds, in theirs, from `right` and `left`,
    /// their hands by their numbers; and the rest where the server side says they are; each as toasted as the server
    /// side says.
    pub fn shown(
        &mut self,
        right: &[(u32, Hand)],
        left: &[(u32, Hand)],
        now: Instant,
        clock: u64,
    ) -> Vec<Shown> {
        let seconds = self
            .last
            .map_or(0.0, |last| {
                now.saturating_duration_since(last).as_secs_f32()
            })
            .min(FRAME_MOST);
        self.last = Some(now);
        let placed = self.loose.placed(right, left, now, clock);
        self.foreseen
            .retain(|body, _| placed.iter().any(|(held, _, mine)| held == body && *mine));
        placed
            .into_iter()
            .map(|(body, placement, mine)| {
                let said = self.toast(Rack::number(body) as usize);
                let pose = Pose::of(&placement);
                let marshmallow = pose.marshmallow();
                let toast = if mine {
                    let foreseen = self.foreseen.entry(body).or_insert(said);
                    *foreseen = foresee(*foreseen, said, marshmallow, seconds);
                    *foreseen
                } else {
                    said
                };
                Shown {
                    transform: campsite::drawn(&placement),
                    marshmallow,
                    shade: campsite::shade(toast),
                    burning: campsite::burning(toast, marshmallow),
                }
            })
            .collect()
    }
}

/// The toasting this participant foresees of a marshmallow they hold, `foreseen`, at `at`, `seconds` on, as the server
/// side said `said`: toasted by the same rule the side toasts it by, never more than [`AHEAD`] beyond what the side
/// said, which only rises, so that it never runs far from what everyone else sees, and eased up to it, should it fall
/// more than [`BEHIND`] short, so that it never jumps.
pub fn foresee(foreseen: f32, said: f32, at: Vec3, seconds: f32) -> f32 {
    let toasted = campsite::toasted(foreseen, at, seconds).min((said + AHEAD).min(1.0));
    let least = (said - BEHIND).max(0.0);
    if toasted < least {
        toasted + (least - toasted) * (seconds / EASE).min(1.0)
    } else {
        toasted
    }
}

#[cfg(test)]
mod tests;
