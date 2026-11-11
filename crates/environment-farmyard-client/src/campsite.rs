// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The campsite's marshmallow sticks as a client shows them: where each is, as the server side last said, the stick
//! this participant holds, drawn from their own hand every frame and toasted over the fire as they hold it, and those
//! the others hold, drawn from the hands their presence brings. Whoever holds a stick controls it; let go, it rests
//! where the server side decides, which this client foresees with the same rule, so that nothing jumps while the
//! server agrees. See `tomori-environment-farmyard`'s `context/campsite.md`.
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};
use tomori_scene::client_side::{Actions, Grab, Hand, Phase, Picked};
use tomori_scene::pointing::{Ray, Shape};

use tomori_environment_farmyard::campsite::{
    self, Event, Intent, Lying, Pose, STICKS, Shared, Stick, TOAST_RATE,
};

/// How near the ray passes a stick to point at it, and how much nearer for each metre away, as a thin thing is hard to
/// point at exactly from afar; and how near a hand comes to a stick's handle to take it.
const POINTED: f32 = 0.04;
const POINTED_PER_METRE: f32 = 0.012;
pub const REACH: f32 = 0.1;
/// How often, at most, the holder says how toasted their marshmallow is, and by how much it must have changed: within
/// the intents' rate, with room for taking and letting go.
const TOAST_EVERY: Duration = Duration::from_millis(500);
const TOAST_STEP: f32 = 0.005;
/// How long a grab waits for the server side to agree before it is given up, as when it was dropped, and a letting go
/// before it is asked again, or given up where the server side never had the stick held, as when the grab was dropped.
const UNAGREED: Duration = Duration::from_millis(1500);

/// The stick this participant holds: which, the epoch their grab makes it, how toasted it is as they hold it, where
/// their hand holds it, when they took it and last said how toasted it is, and whether the server side has agreed.
#[derive(Clone, Debug)]
struct Holding {
    stick: usize,
    epoch: u32,
    toast: f32,
    pose: Pose,
    since: Instant,
    last: Instant,
    said: (Instant, f32),
    agreed: bool,
}

/// A stick this participant let go, foreseen where the server side will rest it, until the server side says it has:
/// with what it asked, and when, so that a letting go the intents' rate dropped is asked again.
#[derive(Clone, Debug)]
struct Letting {
    epoch: u32,
    lying: Option<Lying>,
    toast: f32,
    asked: Intent,
    sent: Instant,
}

/// A stick as it is drawn this frame: where, the shade of its marshmallow and whether it burns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub transform: Mat4,
    pub marshmallow: Vec3,
    pub shade: usize,
    pub burning: bool,
}

/// The sticks as this client shows them.
#[derive(Clone, Debug, Default)]
pub struct Sticks {
    shared: Vec<Stick>,
    holding: Option<Holding>,
    letting: Vec<Option<Letting>>,
    /// Where each stick another holds was last drawn, should their hand go unreported.
    drawn: Vec<Option<Pose>>,
    /// This participant's number, as the last frame said.
    me: Option<u32>,
}

impl Sticks {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sticks as the server side shares them, in its state: every one anew.
    pub fn share(&mut self, shared: Shared) {
        self.shared = shared.sticks.into_iter().take(STICKS).collect();
        self.letting.resize(self.shared.len(), None);
        self.drawn.resize(self.shared.len(), None);
        for index in 0..self.shared.len() {
            self.agree(index);
        }
        if self
            .holding
            .as_ref()
            .is_some_and(|holding| holding.stick >= self.shared.len())
        {
            self.holding = None;
        }
    }

    /// A stick that changed, as the server side announces it.
    pub fn changed(&mut self, event: Event) {
        let Some(stick) = self.shared.get_mut(event.stick) else {
            return;
        };
        *stick = event.now;
        self.agree(event.stick);
    }

    /// What the server side now says of stick `index`, against what this participant did with it: their grab agreed,
    /// or given up as another holds it or it moved on; and their letting go, once it rests.
    fn agree(&mut self, index: usize) {
        let stick = &self.shared[index];
        if let Some(holding) = self
            .holding
            .as_mut()
            .filter(|holding| holding.stick == index)
        {
            let mine = stick.held.is_some() && stick.held == self.me;
            if mine && stick.epoch == holding.epoch {
                holding.agreed = true;
            } else if holding.agreed || stick.held.is_some() || stick.epoch >= holding.epoch {
                // Another took it first, or it has moved on without this participant, as when they left it too long:
                // they hold nothing.
                self.holding = None;
            }
        }
        // Rested, or another holds it, as when this participant let go of a grab another won before hearing so.
        let other = stick.held.is_some() && stick.held != self.me;
        if self.letting[index]
            .as_ref()
            .is_some_and(|letting| other || stick.epoch >= letting.epoch)
        {
            self.letting[index] = None;
        }
    }

    /// Whether stick `index` may be taken: nobody holds it, as far as this client knows.
    fn free(&self, index: usize) -> bool {
        self.shared[index].held.is_none()
            && self
                .holding
                .as_ref()
                .is_none_or(|holding| holding.stick != index)
            && self.letting[index].is_none()
    }

    /// Where stick `index` rests, as the server side says or this client foresees, and how toasted it is.
    fn resting(&self, index: usize) -> (Pose, f32) {
        match &self.letting[index] {
            Some(letting) => (
                match &letting.lying {
                    Some(lying) => Pose::lying(lying),
                    None => Pose::racked(index),
                },
                letting.toast,
            ),
            None => (self.shared[index].resting(index), self.shared[index].toast),
        }
    }

    /// What `ray` meets first of the campsite: a free stick it passes near, which the grip takes, or the bonfire, each
    /// with how far along the ray.
    pub fn pick(&self, ray: &Ray) -> Option<Picked> {
        // Nothing until the server side has said what there is.
        if self.shared.is_empty() {
            return None;
        }
        let sticks = (0..self.shared.len())
            .filter(|index| self.free(*index))
            .filter_map(|index| {
                let (pose, _) = self.resting(index);
                let distance = pose.middle().distance(ray.origin);
                pose.met(
                    ray.origin,
                    ray.direction,
                    POINTED + POINTED_PER_METRE * distance,
                )
                .map(|along| {
                    Picked::new(format!("stick-{index}"), "a marshmallow stick", along)
                        .handles(Actions::GRAB)
                })
            });
        let bonfire = Shape::Sphere {
            center: campsite::heart(),
            radius: 0.35,
        }
        .intersect(ray)
        .map(|distance| Picked::new("bonfire", "the bonfire", distance));
        sticks
            .chain(bonfire)
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// The free stick whose handle a hand at `hand` is within [`REACH`] of, the nearest.
    pub fn within_reach(&self, hand: Vec3) -> Option<String> {
        (0..self.shared.len())
            .filter(|index| self.free(*index))
            .map(|index| (index, self.resting(index).0.from_handle(hand)))
            .filter(|(_, distance)| *distance <= REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| format!("stick-{index}"))
    }

    /// A frame at `now`: the grip on a stick, if any, by this participant, `participant`, if they take part; what to ask
    /// the server side.
    pub fn update(
        &mut self,
        grab: Option<&Grab>,
        participant: Option<u32>,
        now: Instant,
    ) -> Vec<Intent> {
        let mut intents = Vec::new();
        self.me = participant;
        if self.holding.as_ref().is_some_and(|holding| {
            !holding.agreed && now.saturating_duration_since(holding.since) >= UNAGREED
        }) {
            self.holding = None;
        }
        // A letting go the server side has not taken for a while is asked again while it still says this participant
        // holds the stick, and otherwise, as when it never had their grab, given up.
        for index in 0..self.letting.len() {
            let Some(letting) = self.letting[index]
                .as_mut()
                .filter(|letting| now.saturating_duration_since(letting.sent) >= UNAGREED)
            else {
                continue;
            };
            let stick = &self.shared[index];
            if stick.held.is_some()
                && stick.held == participant
                && stick.epoch.wrapping_add(1) == letting.epoch
            {
                letting.sent = now;
                intents.push(letting.asked.clone());
            } else {
                self.letting[index] = None;
            }
        }
        let Some(grab) = grab else {
            return intents;
        };
        let index = grab
            .object
            .strip_prefix("stick-")
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| *index < self.shared.len());
        let pose = Pose::held(grab.hand.position, grab.hand.orientation);
        match (grab.phase, index) {
            (Phase::Began, Some(index)) if participant.is_some() && self.free(index) => {
                let stick = &self.shared[index];
                intents.push(Intent::Grab {
                    grab: index,
                    epoch: stick.epoch,
                });
                self.holding = Some(Holding {
                    stick: index,
                    epoch: stick.epoch.wrapping_add(1),
                    toast: stick.toast,
                    pose,
                    since: now,
                    last: now,
                    said: (now, stick.toast),
                    agreed: false,
                });
            }
            (Phase::Changed, Some(index)) => {
                let Some(holding) = self
                    .holding
                    .as_mut()
                    .filter(|holding| holding.stick == index)
                else {
                    return intents;
                };
                let seconds = now
                    .saturating_duration_since(holding.last)
                    .as_secs_f32()
                    .min(0.1);
                holding.last = now;
                holding.pose = pose;
                holding.toast = (holding.toast
                    + TOAST_RATE * campsite::heat(pose.marshmallow()) * seconds)
                    .min(1.0);
                let (when, level) = holding.said;
                if now.saturating_duration_since(when) >= TOAST_EVERY
                    && holding.toast - level >= TOAST_STEP
                {
                    holding.said = (now, holding.toast);
                    intents.push(Intent::Toast {
                        toast: index,
                        epoch: holding.epoch,
                        level: holding.toast,
                    });
                }
            }
            (Phase::Ended, Some(index)) => {
                let Some(holding) = self.holding.take_if(|holding| holding.stick == index) else {
                    return intents;
                };
                let (at, yaw) = (pose.middle(), pose.yaw());
                let lying = campsite::rest(at.to_array(), yaw);
                let asked = Intent::Release {
                    release: index,
                    epoch: holding.epoch,
                    level: holding.toast,
                    at: at.to_array(),
                    yaw,
                };
                intents.push(asked.clone());
                self.letting[index] = Some(Letting {
                    epoch: holding.epoch.wrapping_add(1),
                    lying,
                    toast: if lying.is_some() { holding.toast } else { 0.0 },
                    asked,
                    sent: now,
                });
            }
            _ => {}
        }
        intents
    }

    /// Every stick as it is drawn this frame: the one this participant holds, in their hand; one another holds, in
    /// theirs, from `hands`, by their numbers, or where it was last drawn; and the rest where they rest.
    pub fn shown(&mut self, hands: &[(u32, Hand)]) -> Vec<Shown> {
        (0..self.shared.len())
            .map(|index| {
                let (pose, toast) = match (&self.holding, self.shared[index].held) {
                    (Some(holding), _) if holding.stick == index => (holding.pose, holding.toast),
                    (_, Some(holder)) if self.letting[index].is_none() => {
                        let held = hands
                            .iter()
                            .find(|(number, _)| *number == holder)
                            .map(|(_, hand)| Pose::held(hand.position, hand.orientation));
                        let pose = held
                            .or(self.drawn[index])
                            .unwrap_or_else(|| self.resting(index).0);
                        self.drawn[index] = Some(pose);
                        (pose, self.shared[index].toast)
                    }
                    _ => self.resting(index),
                };
                Shown {
                    transform: pose.transform(),
                    marshmallow: pose.marshmallow(),
                    shade: campsite::shade(toast),
                    burning: campsite::burning(toast, pose.marshmallow()),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
