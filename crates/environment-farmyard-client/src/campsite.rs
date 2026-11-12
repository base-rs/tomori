// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The campsite's marshmallow sticks as a client shows them: loose bodies ([`Loose`]), as the beach's stones are, taken
//! with either grip by their handle, pointing at them or reaching for them, held along the hand through the fist, drawn
//! from this participant's own hand every frame and from the others' hands their presence brings, and let go, thrown as
//! fast as the hand moved, foreseen until the server's snapshots take over; each marshmallow as toasted as the server
//! side says, which toasts it from where the holder's hand holds it, and, for one this participant holds, as they
//! foresee it toasting by the same rule from where they draw it, easing to what the server side says; and each stick on
//! its way back fading out where it was, its marshmallow as it was, and in on its log, fresh. See
//! `tomori-environment-farmyard`'s `campsite`.
use std::collections::BTreeMap;
use std::time::Instant;

use glam::{Mat4, Vec3};
use tomori_physics::Placement;
use tomori_physics::message::{Intent, State, Update};
use tomori_physics::mirror::Fade;
use tomori_scene::client_side::{Grab, Hand, Picked, Side, Sound};
use tomori_scene::pointing::{Ray, Shape};

pub use crate::loose::{Loose, Shapes};
use tomori_environment_farmyard::campsite::{self, Event, LOGS, Pose, STICKS, Shared};
use tomori_environment_farmyard::random::{self, mix};

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

/// A stick as it is drawn this frame: which, where, the shade of its marshmallow and whether it burns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub index: usize,
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

    /// Anywhere along it, as the other hand holds its handle.
    fn touch(_body: u32, placement: &Placement, hand: Vec3) -> f32 {
        Pose::of(placement).from_stick(hand)
    }

    /// By its handle, along the hand, however it was taken: flying there from where it was, as a stone pulled along the
    /// ray does, so that a stick taken anywhere along its handle settles into the fist. One passed from the other hand is
    /// held where the hand took it instead (see [`Loose`]).
    fn held(_body: u32, _at: &Placement, _hand: &Placement, _within: bool) -> (Placement, bool) {
        (campsite::grip(), true)
    }

    /// A tap of wood, softer than a stone's clack, or a splash into the pool.
    fn sound(_body: u32, water: bool, speed: f32) -> Option<Sound> {
        let speed = if water { speed } else { speed * 0.4 };
        (speed >= crate::stones::QUIET).then(|| crate::stones::sound(water, speed))
    }
}

/// How near the ray passes a log to point at it, beyond its sides, and how near a hand comes to them to take it.
const LOG_POINTED: f32 = 0.04;
pub const LOG_REACH: f32 = 0.08;

/// The woodpile's logs, as loose bodies are: called by their place on the woodpile, met along their length, taken where
/// the hand closes on them, or pulled into the palm along the ray, and heard, lightly, as they land.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pile;

impl Shapes for Pile {
    const PREFIX: &'static str = "log-";
    const NAME: &'static str = "a log";
    const REACH: f32 = LOG_REACH;

    fn number(body: u32) -> u32 {
        campsite::log_index(body).map_or(body, |index| index as u32)
    }

    fn body(number: u32) -> Option<u32> {
        ((number as usize) < LOGS).then(|| campsite::log_body(number as usize))
    }

    fn met(origin: Vec3, direction: Vec3, _body: u32, placement: &Placement) -> Option<f32> {
        campsite::log_met(placement, origin, direction, LOG_POINTED)
    }

    fn reach(_body: u32, placement: &Placement, hand: Vec3) -> f32 {
        campsite::from_log(placement, hand)
    }

    fn touch(_body: u32, placement: &Placement, hand: Vec3) -> f32 {
        campsite::from_log(placement, hand)
    }

    /// Within reach, held where the hand closed on it; otherwise pulled into the palm, across it, turned as it was.
    fn held(_body: u32, at: &Placement, hand: &Placement, within: bool) -> (Placement, bool) {
        if within {
            (hand.inverse().then(at), false)
        } else {
            (
                Placement::new(
                    Vec3::new(0.0, 0.0, -campsite::LOG_RADIUS),
                    hand.rotation.inverse() * at.rotation,
                ),
                true,
            )
        }
    }

    /// A knock of wood, softer than a stone's clack, or a splash into the pool.
    fn sound(_body: u32, water: bool, speed: f32) -> Option<Sound> {
        let speed = if water { speed } else { speed * 0.6 };
        (speed >= crate::stones::QUIET).then(|| crate::stones::sound(water, speed))
    }
}

/// `state`, but only the bodies `keep` keeps.
fn only(state: &State, keep: fn(u32) -> bool) -> State {
    State {
        bodies: state
            .bodies
            .iter()
            .filter(|body| keep(body.body))
            .cloned()
            .collect(),
    }
}

/// `update`, but only of the bodies `keep` keeps.
fn only_of(update: &Update, keep: fn(u32) -> bool) -> Update {
    Update {
        moving: update
            .moving
            .iter()
            .filter(|snapshot| keep(snapshot.body()))
            .cloned()
            .collect(),
        rested: update
            .rested
            .iter()
            .filter(|snapshot| keep(snapshot.body()))
            .cloned()
            .collect(),
        holding: update
            .holding
            .iter()
            .filter(|holding| keep(holding.body))
            .cloned()
            .collect(),
        landed: update
            .landed
            .iter()
            .filter(|landed| keep(landed.body))
            .cloned()
            .collect(),
        returning: update
            .returning
            .iter()
            .filter(|returning| keep(returning.body))
            .cloned()
            .collect(),
    }
}

/// Whether body `body` is a stick's, or a log's.
fn is_stick(body: u32) -> bool {
    campsite::index(body).is_some()
}

fn is_log(body: u32) -> bool {
    campsite::log_index(body).is_some()
}

/// A log as it is drawn this frame: where, and its shade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownLog {
    pub transform: Mat4,
    pub shade: usize,
}

/// How soon, in seconds, the fire drawn eases to the size the server side says, about a fifth of the way each twentieth
/// of a second, so that it never jumps as it is said twice a second.
const SIZE_EASE: f32 = 0.25;

/// How long each chance of a crackle lasts, on the farmyard's clock, in milliseconds, and the most chance of one in
/// each, at the fire's biggest: none at its default, whose crackle is the ambience's, and about three a second at its
/// biggest.
const CRACKLE_SLOT: u64 = 100;
const CRACKLE_CHANCE: f32 = 0.3;

/// The sticks as this client shows them: their bodies, how toasted the server side says each is, this participant's
/// foresight of those they hold, how toasted each on its way back was as it began, which it is drawn as fading out,
/// the server side having said its marshmallow fresh, how long each marshmallow burns on, as this client draws it, and
/// when they were last drawn.
///
/// And the woodpile's logs, as the server side says how far each has burnt, how far each on its way back had as it
/// began, and the fire, as big as the server side says, and as big as it is drawn, eased there, and when it last
/// crackled, on the farmyard's clock.
#[derive(Clone, Debug, Default)]
pub struct Sticks {
    loose: Loose<Rack>,
    toast: Vec<f32>,
    foreseen: BTreeMap<u32, f32>,
    leaving: BTreeMap<u32, f32>,
    flames: BTreeMap<u32, f32>,
    last: Option<Instant>,
    logs: Loose<Pile>,
    burnt: Vec<f32>,
    burning_out: BTreeMap<u32, f32>,
    size: f32,
    drawn_size: Option<f32>,
    crackled: Option<u64>,
}

impl Sticks {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sticks and logs as the server side shares them, in its state: every one anew, with how big the fire is.
    pub fn share(&mut self, shared: &Shared) {
        self.loose.share(&only(&shared.sticks, is_stick));
        self.logs.share(&only(&shared.sticks, is_log));
        self.toast = shared.toast.iter().copied().take(STICKS).collect();
        self.toast.resize(STICKS, 0.0);
        self.burnt = shared
            .burnt
            .iter()
            .copied()
            .take(LOGS)
            .map(campsite::unit)
            .collect();
        self.burnt.resize(LOGS, 0.0);
        self.size = campsite::unit(shared.size);
        self.keep_burning();
    }

    /// Keep the logs burning from being taken.
    fn keep_burning(&mut self) {
        let burning = (0..LOGS)
            .filter(|index| self.burnt[*index] > 0.0)
            .map(campsite::log_body);
        self.logs.keep(burning);
    }

    /// What changed, as the server side announced it, arriving at `now` on the farmyard's clock; and the sounds of what
    /// landed.
    pub fn changed(&mut self, event: &Event, now: u64) -> Vec<(Sound, [f32; 3])> {
        for returning in &event.sticks.returning {
            if let Some(index) = campsite::index(returning.body) {
                self.leaving.insert(returning.body, self.toast(index));
            } else if let Some(index) = campsite::log_index(returning.body) {
                self.burning_out.insert(
                    returning.body,
                    self.burnt.get(index).copied().unwrap_or(0.0),
                );
            }
        }
        for toasted in &event.toast {
            if let Some(index) = campsite::index(toasted.body)
                && let Some(toast) = self.toast.get_mut(index)
                && toasted.toast.is_finite()
            {
                *toast = toasted.toast.clamp(0.0, 1.0);
            }
        }
        for burning in &event.burnt {
            if let Some(index) = campsite::log_index(burning.body)
                && let Some(burnt) = self.burnt.get_mut(index)
            {
                *burnt = campsite::unit(burning.burnt);
            }
        }
        if let Some(size) = event.size {
            self.size = campsite::unit(size);
        }
        self.keep_burning();
        let mut sounds = self
            .loose
            .changed(event.clock, &only_of(&event.sticks, is_stick), now);
        sounds.extend(
            self.logs
                .changed(event.clock, &only_of(&event.sticks, is_log), now),
        );
        sounds
    }

    /// How toasted stick `index` is, as the server side last said.
    pub fn toast(&self, index: usize) -> f32 {
        self.toast.get(index).copied().unwrap_or_default()
    }

    /// The loose bodies the sticks are.
    pub fn loose(&self) -> &Loose<Rack> {
        &self.loose
    }

    /// How far log `index` has burnt, as the server side last said.
    pub fn burnt(&self, index: usize) -> f32 {
        self.burnt.get(index).copied().unwrap_or_default()
    }

    /// How big the fire is, as the server side last said, from 0, its default, to 1, its biggest.
    pub fn size(&self) -> f32 {
        self.size
    }

    /// How big the fire is drawn, eased towards what the server side says as the sticks were last drawn.
    pub fn drawn_size(&self) -> f32 {
        self.drawn_size.unwrap_or(self.size)
    }

    /// What `ray` meets first of the campsite: a free stick or log it passes near, which the grip takes, or the
    /// bonfire, each with how far along the ray; nothing until the server side has said what there is.
    pub fn pick(&self, ray: &Ray) -> Option<Picked> {
        self.nearest(ray, self.loose.pick(ray), self.logs.pick(ray))
    }

    /// What the ray of the hand `side` meets first of the campsite, as [`Sticks::pick`] does, or the stick or log this
    /// participant holds in their other hand, which its grip passes to it.
    pub fn pick_by(&self, side: Side, ray: &Ray) -> Option<Picked> {
        self.nearest(
            ray,
            self.loose.pick_by(side, ray),
            self.logs.pick_by(side, ray),
        )
    }

    /// What `ray` meets first of `stick` and `log`, what it meets of those, and the bonfire, as big as it is drawn;
    /// nothing until the server side has said what there is.
    fn nearest(&self, ray: &Ray, stick: Option<Picked>, log: Option<Picked>) -> Option<Picked> {
        if self.toast.is_empty() {
            return None;
        }
        let size = self.drawn_size();
        let bonfire = Shape::Sphere {
            center: campsite::heart_at(size),
            radius: 0.35 * campsite::reach(size),
        }
        .intersect(ray)
        .map(|distance| Picked::new("bonfire", "the bonfire", distance));
        stick
            .into_iter()
            .chain(log)
            .chain(bonfire)
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// The free stick whose handle a hand at `hand` is within [`REACH`] of, or else the free log it is within
    /// [`LOG_REACH`] of, the nearest.
    pub fn within_reach(&self, hand: Vec3) -> Option<String> {
        self.loose
            .within_reach(hand)
            .or_else(|| self.logs.within_reach(hand))
    }

    /// The stick or log the hand `side`, at `hand`, takes, as [`Loose::within_reach_of`] says: a free stick by its
    /// handle, or a free log, or the one this participant holds in their other hand anywhere along it.
    pub fn within_reach_of(&self, side: Side, hand: Vec3) -> Option<String> {
        self.loose
            .within_reach_of(side, hand)
            .or_else(|| self.logs.within_reach_of(side, hand))
    }

    /// The hands that took a stick or a log from the other hand since this was last asked.
    pub fn passed(&mut self) -> Vec<Side> {
        let mut passed = self.loose.passed();
        passed.extend(self.logs.passed());
        passed
    }

    /// A frame at `now`, the farmyard's clock at `clock`: the grips on sticks and logs this frame, by this participant,
    /// `participant`, if they take part; what to ask the server side.
    pub fn update<'a>(
        &mut self,
        grabs: impl Iterator<Item = &'a Grab> + Clone,
        participant: Option<u32>,
        now: Instant,
        clock: u64,
    ) -> Vec<Intent> {
        let mut intents = self.loose.update(grabs.clone(), participant, now, clock);
        intents.extend(self.logs.update(grabs, participant, now, clock));
        intents
    }

    /// Every stick as it is drawn at `now`, the farmyard's clock at `clock`: those this participant holds, in their
    /// hand, their marshmallows as they foresee them toasting; one another holds, in theirs, from `right` and `left`,
    /// their hands by their numbers; and the rest where the server side says they are; each as toasted as the server
    /// side says, but one fading out on its way back, as toasted as it was, and burning only drawn whole.
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
        let size = self.size;
        let drawn = self.drawn_size.get_or_insert(size);
        *drawn += (size - *drawn) * (seconds / SIZE_EASE).min(1.0);
        let shown_size = *drawn;
        let placed = self.loose.placed(right, left, now, clock);
        self.foreseen.retain(|body, _| {
            placed
                .iter()
                .any(|(held, _, _, mine)| held == body && *mine)
        });
        self.leaving.retain(|body, _| {
            placed
                .iter()
                .any(|(leaving, _, fade, _)| leaving == body && matches!(fade, Fade::Out(_)))
        });
        placed
            .into_iter()
            .map(|(body, placement, fade, mine)| {
                let said = self
                    .leaving
                    .get(&body)
                    .copied()
                    .unwrap_or_else(|| self.toast(Rack::number(body) as usize));
                let pose = Pose::of(&placement);
                let marshmallow = pose.marshmallow();
                let toast = if mine {
                    let foreseen = self.foreseen.entry(body).or_insert(said);
                    *foreseen = foresee_by(*foreseen, said, marshmallow, seconds, size);
                    *foreseen
                } else {
                    said
                };
                // Smaller about its middle as it fades, its model running from its butt.
                let drawn = campsite::drawn(&placement);
                let transform = match fade {
                    Fade::Whole => drawn,
                    _ => {
                        placement.matrix()
                            * Mat4::from_scale(Vec3::splat(fade.scale()))
                            * placement.matrix().inverse()
                            * drawn
                    }
                };
                let index = Rack::number(body) as usize;
                let flame = self.flames.entry(body).or_default();
                *flame = match fade {
                    Fade::Whole => {
                        campsite::burns_at(*flame, toast, marshmallow, seconds, shown_size)
                    }
                    _ => 0.0,
                };
                Shown {
                    index,
                    transform,
                    marshmallow,
                    shade: campsite::shade(toast),
                    burning: *flame > 0.0,
                }
            })
            .collect()
    }

    /// Every log as it is drawn at `now`, the farmyard's clock at `clock`: those this participant holds, in their hand;
    /// one another holds, in theirs, from `right` and `left`, their hands by their numbers; and the rest where the
    /// server side says they are, each charred, glowing and shrunk as far as it has burnt, but one fading out on its
    /// way back, as burnt as it was, and fresh as it fades in.
    pub fn logs_shown(
        &mut self,
        right: &[(u32, Hand)],
        left: &[(u32, Hand)],
        now: Instant,
        clock: u64,
    ) -> Vec<ShownLog> {
        let placed = self.logs.placed(right, left, now, clock);
        self.burning_out.retain(|body, _| {
            placed
                .iter()
                .any(|(leaving, _, fade, _)| leaving == body && matches!(fade, Fade::Out(_)))
        });
        placed
            .into_iter()
            .map(|(body, placement, fade, _)| {
                let burnt = self
                    .burning_out
                    .get(&body)
                    .copied()
                    .unwrap_or_else(|| self.burnt(Pile::number(body) as usize));
                let scale = campsite::charred(burnt) * fade.scale();
                ShownLog {
                    transform: placement.matrix() * Mat4::from_scale(Vec3::splat(scale)),
                    shade: campsite::log_shade(burnt),
                }
            })
            .collect()
    }

    /// The crackles the fire makes after the farmyard's clock was last asked and up to `clock`, more and louder the
    /// bigger it is drawn, each at its heart, the same for everyone; none as the clock jumps, as on joining.
    pub fn crackles(&mut self, clock: u64) -> Vec<(Sound, [f32; 3])> {
        let from = self.crackled.replace(clock);
        let Some(from) = from.filter(|from| *from < clock && clock - from < 1_000) else {
            return Vec::new();
        };
        let size = self.drawn_size();
        let heart = campsite::heart_at(size);
        (from / CRACKLE_SLOT + 1..=clock / CRACKLE_SLOT)
            .filter_map(|slot| {
                let draw = mix(slot);
                let chance = (draw >> 40) as f32 / (1u64 << 24) as f32;
                (chance < CRACKLE_CHANCE * size).then(|| {
                    let level = 0.35 + 0.65 * size * ((draw >> 8) & 0xff) as f32 / 255.0;
                    let variant = (draw % CRACKLES as u64) as usize;
                    let offset = Vec3::new(
                        ((draw >> 16) & 0xff) as f32 / 255.0 - 0.5,
                        0.0,
                        ((draw >> 24) & 0xff) as f32 / 255.0 - 0.5,
                    ) * 0.3
                        * campsite::reach(size);
                    (crackle(variant, level), (heart + offset).to_array())
                })
            })
            .collect()
    }
}

/// How many crackles there are, each its own.
const CRACKLES: usize = 4;

/// Crackle `variant` at `level`: a few milliseconds of wood splitting in the heat, a sharp burst of noise, darkened,
/// gone in a twentieth of a second.
fn crackle(variant: usize, level: f32) -> Sound {
    static RENDERED: std::sync::OnceLock<Vec<Vec<f32>>> = std::sync::OnceLock::new();
    let rendered = RENDERED.get_or_init(|| {
        (0..CRACKLES)
            .map(|variant| {
                let mut noise = random::noise(0x2545_f491_u32.wrapping_add(variant as u32 * 7919));
                let length = crate::stones::RATE as usize / 20;
                let decay = 0.9985 - variant as f32 * 0.0003;
                let mut low = 0.0f32;
                let mut gain = 1.0f32;
                (0..length)
                    .map(|_| {
                        low += (noise() - low) * 0.45;
                        gain *= decay;
                        low * gain * 0.6
                    })
                    .collect()
            })
            .collect()
    });
    let level = campsite::unit(level);
    Sound::new(
        rendered[variant % CRACKLES]
            .iter()
            .map(|sample| sample * level)
            .collect::<Vec<f32>>(),
        crate::stones::RATE,
    )
}

/// The toasting this participant foresees of a marshmallow they hold, `foreseen`, at `at`, `seconds` on, as the server
/// side said `said`, the fire at its default size: as [`foresee_by`].
pub fn foresee(foreseen: f32, said: f32, at: Vec3, seconds: f32) -> f32 {
    foresee_by(foreseen, said, at, seconds, 0.0)
}

/// The toasting this participant foresees of a marshmallow they hold, `foreseen`, at `at`, `seconds` on, as the server
/// side said `said`, the fire as big as it said, `size`: toasted by the same rule the side toasts it by, never more
/// than [`AHEAD`] beyond what the side said, which only rises, so that it never runs far from what everyone else sees,
/// and eased up to it, should it fall more than [`BEHIND`] short, so that it never jumps.
pub fn foresee_by(foreseen: f32, said: f32, at: Vec3, seconds: f32, size: f32) -> f32 {
    let toasted = campsite::toasted_at(foreseen, at, seconds, size).min((said + AHEAD).min(1.0));
    let least = (said - BEHIND).max(0.0);
    if toasted < least {
        toasted + (least - toasted) * (seconds / EASE).min(1.0)
    } else {
        toasted
    }
}

#[cfg(test)]
mod tests;
