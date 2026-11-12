// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The campsite's marshmallow sticks as the farmyard's server side keeps them: loose bodies ([`Loose`]) of a
//! `tomori-physics` world of the campsite's own, taken and let go as any loose body is, as the beach's stones are; each
//! held stick's marshmallow toasted from where its holder's hand holds it, as the server last saw it, in the fire's heat;
//! a stick put back by its log, or come to rest where it does not stay, back on its log, fading there, with a fresh
//! marshmallow as it goes; and, once nobody has been in the space for an hour, every stick back on its log.
//! See `crate::campsite`.
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use tomori_environment::Presence;
use tomori_physics::message::Intent;
use tomori_physics::world::World;
use tomori_physics::{Held, Placement, Precision, Side};

use super::loose::{Loose, Noun};
use crate::campsite::{self, Event, Shared, Toasted};

/// How long the side lets the sticks settle on their log as it starts, at most, in simulated seconds.
pub const SETTLE: f32 = 5.0;

/// What the sticks are called in the server's log.
pub const NOUN: Noun = Noun {
    one: |body| match campsite::index(body) {
        Some(index) => format!("stick {index}"),
        None => format!("body {body}"),
    },
    many: "sticks",
};

/// How often, at most, the side says how toasted the sticks are, and by how much one's toasting must have changed: so
/// that a marshmallow held in the heat browns smoothly for everyone else, within the rate the server takes a side's
/// updates at.
pub const TOAST_EVERY: Duration = Duration::from_millis(500);
pub const TOAST_STEP: f32 = 0.005;
/// The longest step the toasting takes at once, in seconds, as when the side was busy.
const TOAST_STEP_MOST: f32 = 0.25;

/// Where the hand that holds `held` is, as the server last saw it in `presence`: its controller's grip pose; or, where
/// the server has not seen it, as on the desktop, which carries what it holds before the eyes, half a metre before the
/// head and a little below, facing as the head does. None where the hand is not a placement at all, or is farther than
/// [`tomori_physics::REACH`] from the head, as a stick is let go no farther: so that a client saying its hand is in the
/// fire from across the campsite toasts nothing.
pub fn hand(presence: &[Presence], held: &Held) -> Option<Placement> {
    let presence = presence
        .iter()
        .find(|presence| presence.participant == held.by)?;
    let placed = |pose: &tomori_protocol::Pose| {
        Placement::new(
            Vec3::from(pose.position),
            Quat::from_array(pose.orientation).normalize(),
        )
    };
    let controller = match held.hand {
        Side::Left => presence.left.as_ref(),
        Side::Right => presence.right.as_ref(),
    };
    let hand = match controller {
        Some(pose) => placed(pose),
        None => placed(&presence.head).then(&Placement::at(Vec3::new(0.0, -0.2, -0.5))),
    };
    let head = Vec3::from(presence.head.position);
    let near = head.distance(hand.position) <= tomori_physics::REACH;
    (hand.valid() && near).then_some(hand)
}

/// What the agent is told of one stick: how toasted it is, who holds it, whether it is back on its log, and where its
/// marshmallow is, unless someone holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Said {
    pub toast: f32,
    pub held_by: Option<u32>,
    pub racked: bool,
    pub floating: bool,
    pub at: Option<Vec3>,
}

/// The campsite as the agent is told of it, as the side last said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub sticks: Vec<Said>,
}

impl Report {
    /// What the space's agent is told of the campsite: the bonfire, the fireside log, and each stick, how toasted its
    /// marshmallow is and where it is, each with its identity, reference and where it stands, but a stick in someone's
    /// hand, which only their presence says.
    pub fn describe(&self) -> Vec<(String, String, String, Option<Vec3>)> {
        let mut objects = vec![(
            "bonfire".to_owned(),
            "campsite/bonfire".to_owned(),
            "the campsite's bonfire, crackling in its ring of stones beside the pool lookout's log seat".to_owned(),
            Some(campsite::heart()),
        )];
        let (seat, _, _, radius) = campsite::fireside();
        objects.push((
            "fireside-log".to_owned(),
            "campsite/fireside-log".to_owned(),
            "the fireside log, a seat between the campfire and the pool, facing the fire; pointing at it takes \
             someone to sit on it"
                .to_owned(),
            Some(seat + Vec3::Y * campsite::seat_top(radius)),
        ));
        objects.extend(self.sticks.iter().enumerate().map(|(index, stick)| {
            let place = match stick {
                Said {
                    held_by: Some(_), ..
                } => "held by someone",
                Said { racked: true, .. } => "leaning on the log by the fire",
                Said { floating: true, .. } => "floating in the pool",
                _ => "lying where it was put down",
            };
            (
                format!("stick-{index}"),
                format!("campsite/stick-{index}"),
                format!(
                    "a marshmallow on a stick, {}, {place}",
                    campsite::looks(stick.toast)
                ),
                stick.at,
            )
        }));
        objects
    }

    /// The details of what it describes as `reference`, for the space's agent: the bonfire, or a stick, how toasted
    /// it is, who holds it, by their number, and whether it is on its log.
    pub fn inspect(&self, reference: &str) -> Option<serde_json::Value> {
        if reference == "campsite/fireside-log" {
            let (seat, _, _, radius) = campsite::fireside();
            return Some(serde_json::json!({
                "kind": "log seat",
                "at": (seat + Vec3::Y * campsite::seat_top(radius)).to_array(),
                "faces": "the campfire",
            }));
        }
        if reference == "campsite/bonfire" {
            return Some(serde_json::json!({
                "kind": "bonfire",
                "at": campsite::heart().to_array(),
            }));
        }
        let index: usize = reference.strip_prefix("campsite/stick-")?.parse().ok()?;
        let stick = self.sticks.get(index)?;
        Some(serde_json::json!({
            "kind": "marshmallow stick",
            "toast": stick.toast,
            "looks": campsite::looks(stick.toast),
            "held_by": stick.held_by,
            "on_its_log": stick.racked,
            "floating": stick.floating,
        }))
    }
}

/// The campsite's sticks, as the side keeps them.
pub struct Sticks {
    loose: Loose,
    /// How toasted each is, what it last said, when it last toasted them and last said.
    toast: Vec<f32>,
    said: Vec<f32>,
    toasted: Instant,
    told: Option<Instant>,
    /// Since when the space has been empty, if it is.
    empty: Option<Instant>,
}

impl Sticks {
    /// The sticks, each leaning on its log, settled there, at `now`.
    pub fn new(now: Instant) -> Self {
        let mut world = World::new(
            campsite::settings(),
            campsite::colliders(),
            campsite::waters(),
            campsite::bodies(),
        )
        .expect("the campsite is within tomori-physics's limits");
        world.settle(SETTLE);
        Self {
            loose: Loose::new(world, NOUN, now),
            toast: vec![0.0; campsite::STICKS],
            said: vec![0.0; campsite::STICKS],
            toasted: now,
            told: None,
            empty: None,
        }
    }

    pub fn loose(&self) -> &Loose {
        &self.loose
    }

    pub fn world(&self) -> &World {
        self.loose.world()
    }

    /// How toasted stick `index` is.
    pub fn toast(&self, index: usize) -> f32 {
        self.toast[index]
    }

    /// Every stick, as everyone in the space is told of them.
    pub fn shared(&self) -> Shared {
        Shared {
            sticks: self.loose.shared(),
            toast: self.toast.clone(),
        }
    }

    /// Do what `participant` asks, `payload`, if it is a taking or letting go of a stick, with where the side last saw
    /// everyone, `presence`; says whether it was. A stick let go gently by its log is put back on it.
    pub fn ask(&mut self, participant: u32, payload: &str, presence: &[Presence]) -> bool {
        let (body, gently) = match serde_json::from_str::<Intent>(payload) {
            Ok(Intent::Take(take)) => (take.body, false),
            Ok(Intent::LetGo(let_go)) => (
                let_go.body,
                Vec3::from(let_go.velocity).length() <= campsite::GENTLY,
            ),
            Err(_) => return false,
        };
        if campsite::index(body).is_none() {
            return false;
        }
        self.loose.ask(participant, payload, presence);
        for body in self.loose.released() {
            let put_back = gently
                && self
                    .world()
                    .placement(body)
                    .is_some_and(|at| campsite::by_its_log(at.position));
            if put_back {
                // Nobody holds it, as it was just let go.
                let _ = self.loose.world_mut().send_home(body);
            }
        }
        true
    }

    /// Let go of what everyone who has gone holds, as [`Loose::sweep`] does; and once nobody has been in the space for
    /// [`campsite::TIDY`], as `present` says, if the server has said, every stick not on its log goes back on it.
    pub fn sweep(&mut self, present: Option<&[u32]>, presence: &[Presence], now: Instant) {
        self.loose.sweep(present, presence, now);
        let Some(present) = present else {
            return;
        };
        if !present.is_empty() {
            self.empty = None;
            return;
        }
        let since = *self.empty.get_or_insert(now);
        if now.saturating_duration_since(since) < campsite::TIDY {
            return;
        }
        for index in 0..campsite::STICKS {
            let body = campsite::body(index);
            let world = self.loose.world_mut();
            if !world.at_home(body, campsite::RACKED) && world.held(body).is_none() {
                let _ = world.send_home(body);
            }
        }
        // Tidied once, until someone comes and goes again.
        self.empty = Some(now);
    }

    /// Toast each stick held in the fire's heat from where its holder's hand holds it, as the server last saw it in
    /// `presence`, up to `now`.
    fn toast_held(&mut self, presence: &[Presence], now: Instant) {
        let seconds = now
            .saturating_duration_since(self.toasted)
            .as_secs_f32()
            .min(TOAST_STEP_MOST);
        self.toasted = now;
        for index in 0..campsite::STICKS {
            let Some(held) = self.world().held(campsite::body(index)) else {
                continue;
            };
            let Some(hand) = hand(presence, &held) else {
                continue;
            };
            let at = campsite::Pose::of(&held.placement(&hand)).marshmallow();
            self.toast[index] = campsite::toasted(self.toast[index], at, seconds);
        }
    }

    /// Move on to `now`, with where the server last saw everyone, `presence`, at the farmyard's `clock`; and what to say
    /// of what changed, if anything did: the sticks' bodies, at most [`tomori_physics::SNAPSHOT_RATE`] times a second,
    /// and how toasted those whose toasting changed are, at most every [`TOAST_EVERY`], but a fresh marshmallow at once:
    /// on a stick that begins its way back to its log, which clients draw as it was until it fades in there, or that
    /// comes to rest on it.
    pub fn update(&mut self, presence: &[Presence], now: Instant, clock: u64) -> Option<Event> {
        self.toast_held(presence, now);
        let sticks = self.loose.update(now).unwrap_or_default();
        let mut fresh = false;
        for returning in &sticks.returning {
            let index =
                campsite::index(returning.body).expect("the campsite's world holds only sticks");
            fresh |= self.toast[index] != 0.0;
            self.toast[index] = 0.0;
        }
        for rested in &sticks.rested {
            let body = rested.body();
            let index = campsite::index(body).expect("the campsite's world holds only sticks");
            let at = rested.placement(Precision::Rest);
            let world = self.loose.world_mut();
            if world.at_home(body, campsite::RACKED) {
                // Back on its log, with a fresh marshmallow.
                fresh |= self.toast[index] != 0.0;
                self.toast[index] = 0.0;
            } else if campsite::goes_back(&at) {
                let _ = world.send_home(body);
            }
        }
        let due = fresh
            || self
                .told
                .is_none_or(|told| now.saturating_duration_since(told) >= TOAST_EVERY);
        let toast: Vec<Toasted> = if due {
            (0..campsite::STICKS)
                .filter(|index| {
                    let (toast, said) = (self.toast[*index], self.said[*index]);
                    (toast - said).abs() >= TOAST_STEP || (toast == 0.0 && said != 0.0)
                })
                .map(|index| Toasted {
                    body: campsite::body(index),
                    toast: self.toast[index],
                })
                .collect()
        } else {
            Vec::new()
        };
        if !toast.is_empty() {
            self.told = Some(now);
            for toasted in &toast {
                let index = campsite::index(toasted.body).expect("a stick");
                self.said[index] = toasted.toast;
            }
        }
        if sticks.is_empty() && toast.is_empty() {
            return None;
        }
        Some(Event {
            clock,
            sticks,
            toast,
        })
    }

    /// The campsite as the agent is told of it now.
    pub fn report(&self) -> Report {
        let world = self.world();
        Report {
            sticks: (0..campsite::STICKS)
                .map(|index| {
                    let body = campsite::body(index);
                    let placement = world.placement(body);
                    Said {
                        toast: self.toast[index],
                        held_by: world.held(body).map(|held| held.by),
                        racked: placement.zip(world.home(body)).is_some_and(|(at, home)| {
                            at.position.distance(home.position) <= campsite::RACKED
                        }),
                        floating: placement.is_some_and(|at| campsite::in_pool(&at)),
                        at: placement.map(|at| campsite::Pose::of(&at).marshmallow()),
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
