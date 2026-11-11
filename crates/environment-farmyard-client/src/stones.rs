// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The beach's stones as a client shows them, from what the farmyard's server side shares: every stone where it rests,
//! or between the snapshots of those that move, `tomori-physics`'s mirror of them; the stones this participant holds,
//! one in each hand, drawn from their own hand every frame, taken within reach of it or pulled to it along the ray, and
//! let go, thrown as fast as the hand moved, foreseen until the server's snapshots take over; those the others hold,
//! drawn from the hands their presence brings; and the clacks and splashes of those that land. The client never
//! simulates. See `tomori-environment-farmyard`'s `beach`.
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use glam::{Mat4, Vec2, Vec3};
use tomori_environment_farmyard::beach;
use tomori_physics::message::{Intent, LetGo, State, Take, Update};
use tomori_physics::mirror::Mirror;
use tomori_physics::{PULL, PULL_TIME, Placement, Precision, RELEASE_SPEED, RELEASE_SPIN};
use tomori_scene::client_side::{Actions, Grab, Hand, Phase, Picked, Side, Sound};
use tomori_scene::pointing::Ray;

/// How near a hand's grip pose comes to a stone's hull to take it there, rather than pull it along the ray.
pub const REACH: f32 = 0.05;
/// How long a taking waits for the server side to agree before it is given up, as when it was dropped.
const UNAGREED: Duration = Duration::from_millis(1500);
/// How long a hand's motion is followed back to say how fast it moved as it let go.
const FOLLOWED: Duration = Duration::from_millis(80);

/// A stone as it is drawn this frame: which, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub stone: usize,
    pub transform: Mat4,
}

/// A stone this participant holds: which, the epoch their taking makes it, where it is in the hand, where it flew from
/// as it was pulled to the hand and since when, where the hand was over the last moments, when they took it, and whether
/// the server side has agreed.
#[derive(Clone, Debug)]
struct Holding {
    stone: u32,
    epoch: u32,
    offset: Placement,
    pulled: Option<(Placement, Instant)>,
    hand: VecDeque<(Instant, Placement)>,
    since: Instant,
    agreed: bool,
}

impl Holding {
    /// Where it is drawn at `now`, in the hand as it was last told, or on its way to it.
    fn placement(&self, now: Instant) -> Placement {
        let hand = self
            .hand
            .back()
            .map_or(Placement::IDENTITY, |(_, hand)| *hand);
        let held = hand.then(&self.offset);
        match self.pulled {
            Some((from, began)) => {
                let t = (now.saturating_duration_since(began).as_secs_f32() / PULL_TIME).min(1.0);
                let t = t * t * (3.0 - 2.0 * t);
                Placement::new(
                    from.position.lerp(held.position, t),
                    from.rotation.slerp(held.rotation, t),
                )
            }
            None => held,
        }
    }

    /// How fast the stone moved and spun as the hand last moved, over the last [`FOLLOWED`], each within the bounds a
    /// release is held to.
    fn motion(&self) -> (Vec3, Vec3) {
        let (now, last) = *self.hand.back().expect("a held stone has a hand");
        let (then, first) = *self
            .hand
            .iter()
            .find(|(at, _)| now.saturating_duration_since(*at) <= FOLLOWED)
            .expect("the latest is within it");
        let seconds = now.saturating_duration_since(then).as_secs_f32();
        if seconds <= 0.0 {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let (was, is) = (first.then(&self.offset), last.then(&self.offset));
        let velocity = (is.position - was.position) / seconds;
        let (axis, angle) = (is.rotation * was.rotation.inverse()).to_axis_angle();
        let spin = axis * (angle / seconds);
        (
            velocity.clamp_length_max(RELEASE_SPEED),
            spin.clamp_length_max(RELEASE_SPIN),
        )
    }
}

/// How long before the latest of the hand's moments `at` was.
fn now_minus(latest: Option<&(Instant, Placement)>, at: Instant) -> Duration {
    latest.map_or(Duration::ZERO, |(latest, _)| {
        latest.saturating_duration_since(at)
    })
}

/// A hand as a placement.
fn placed(hand: &Hand) -> Placement {
    Placement::new(hand.position, hand.orientation.normalize())
}

/// The stones as this client shows them.
#[derive(Clone, Debug, Default)]
pub struct Stones {
    mirror: Mirror,
    /// What this participant holds, left hand then right.
    holding: [Option<Holding>; 2],
    /// This participant's number, as the last frame said.
    me: Option<u32>,
    /// Where each stone was last drawn: what the ray and the hands meet, and where one another holds stays should their
    /// hand go unreported.
    drawn: BTreeMap<u32, Placement>,
}

impl Stones {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stones as the server side shares them, in its state: every one anew.
    pub fn share(&mut self, state: &State) {
        self.mirror.share(state);
        self.drawn = state
            .bodies
            .iter()
            .map(|body| (body.body, body.at.placement(Precision::Rest)))
            .collect();
        self.agree();
    }

    /// What changed, as the server side announced it at `sent` on the farmyard's clock, arriving at `now` on it; and
    /// the sounds of what landed, as they are heard.
    pub fn changed(&mut self, sent: u64, update: &Update, now: u64) -> Vec<(Sound, [f32; 3])> {
        self.mirror.apply(sent, update, now);
        self.agree();
        update
            .landed
            .iter()
            .filter_map(|landed| {
                let at = self.drawn.get(&landed.body)?.position;
                (landed.speed >= QUIET).then(|| (sound(landed.water, landed.speed), at.to_array()))
            })
            .collect()
    }

    /// What the server side now says of the stones this participant holds: their taking agreed, or given up as another
    /// holds it or it has moved on without them.
    fn agree(&mut self) {
        for holding in &mut self.holding {
            let Some(held) = holding.as_mut() else {
                continue;
            };
            let (epoch, by) = (
                self.mirror.epoch(held.stone),
                self.mirror.held(held.stone).map(|held| held.by),
            );
            if epoch == Some(held.epoch) && by.is_some() && by == self.me {
                held.agreed = true;
            } else if held.agreed || by.is_some() || epoch.is_none_or(|epoch| epoch >= held.epoch) {
                *holding = None;
            }
        }
    }

    /// Whether stone `stone` may be taken: nobody holds it, as far as this client knows.
    fn free(&self, stone: u32) -> bool {
        self.mirror.held(stone).is_none()
            && self
                .holding
                .iter()
                .flatten()
                .all(|holding| holding.stone != stone)
    }

    /// What `ray` meets first of the stones that may be taken, within [`PULL`], which the grip pulls to the hand.
    pub fn pick(&self, ray: &Ray) -> Option<Picked> {
        self.drawn
            .iter()
            .filter(|(stone, _)| self.free(**stone))
            .filter_map(|(stone, placement)| {
                beach::met(ray.origin, ray.direction, *stone as usize, placement)
                    .filter(|along| *along <= PULL)
                    .map(|along| {
                        Picked::new(format!("stone-{stone}"), "a river stone", along)
                            .handles(Actions::GRAB)
                    })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// The stone that may be taken whose hull a hand at `hand` is within [`REACH`] of, the nearest.
    pub fn within_reach(&self, hand: Vec3) -> Option<String> {
        self.drawn
            .iter()
            .filter(|(stone, _)| self.free(**stone))
            .map(|(stone, placement)| (*stone, reach(*stone, placement, hand)))
            .filter(|(_, distance)| *distance <= REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(stone, _)| format!("stone-{stone}"))
    }

    /// A frame at `now`, the farmyard's clock at `clock`: the grips on stones this frame, by this participant,
    /// `participant`, if they take part; what to ask the server side.
    pub fn update<'a>(
        &mut self,
        grabs: impl Iterator<Item = &'a Grab>,
        participant: Option<u32>,
        now: Instant,
        clock: u64,
    ) -> Vec<Intent> {
        self.me = participant;
        for holding in &mut self.holding {
            if holding.as_ref().is_some_and(|holding| {
                !holding.agreed && now.saturating_duration_since(holding.since) >= UNAGREED
            }) {
                *holding = None;
            }
        }
        let mut intents = Vec::new();
        for grab in grabs {
            let Some(stone) = grab
                .object
                .strip_prefix("stone-")
                .and_then(|stone| stone.parse::<u32>().ok())
                .filter(|stone| self.mirror.epoch(*stone).is_some())
            else {
                continue;
            };
            let (side, which) = match grab.side {
                Side::Left => (tomori_physics::Side::Left, 0),
                _ => (tomori_physics::Side::Right, 1),
            };
            let hand = placed(&grab.hand);
            match grab.phase {
                Phase::Began => {
                    if participant.is_none() || !self.free(stone) || self.holding[which].is_some() {
                        continue;
                    }
                    let at = self.drawn[&stone];
                    let epoch = self.mirror.epoch(stone).expect("known above");
                    // Within reach, held where the hand closed on it; otherwise pulled to the palm, turned as it was.
                    let (offset, pulled) = if reach(stone, &at, hand.position) <= REACH {
                        (hand.inverse().then(&at), None)
                    } else {
                        let (width, _) = beach::size(stone as usize);
                        let turned = hand.rotation.inverse() * at.rotation;
                        let palm =
                            Placement::new(Vec3::new(0.0, 0.0, -(width * 0.3 + 0.03)), turned);
                        (palm, Some((at, now)))
                    };
                    let offset = offset.snapped(Precision::Rest);
                    intents.push(Intent::Take(Take {
                        body: stone,
                        epoch,
                        hand: side,
                        offset: offset.encoded(Precision::Rest),
                    }));
                    self.holding[which] = Some(Holding {
                        stone,
                        epoch: epoch.wrapping_add(1),
                        offset,
                        pulled,
                        hand: VecDeque::from([(now, hand)]),
                        since: now,
                        agreed: false,
                    });
                }
                Phase::Changed => {
                    if let Some(holding) = self.holding[which]
                        .as_mut()
                        .filter(|holding| holding.stone == stone)
                    {
                        holding.hand.push_back((now, hand));
                        while holding.hand.len() > 2
                            && now_minus(holding.hand.back(), holding.hand[1].0) > FOLLOWED
                        {
                            holding.hand.pop_front();
                        }
                    }
                }
                _ => {
                    let Some(mut holding) =
                        self.holding[which].take_if(|holding| holding.stone == stone)
                    else {
                        continue;
                    };
                    holding.hand.push_back((now, hand));
                    let at = lifted(stone, holding.placement(now)).snapped(Precision::Rest);
                    let (velocity, spin) = holding.motion();
                    intents.push(Intent::LetGo(LetGo {
                        body: stone,
                        epoch: holding.epoch,
                        at: at.encoded(Precision::Rest),
                        velocity: velocity.to_array(),
                        spin: spin.to_array(),
                        clock,
                    }));
                    self.mirror.let_go(stone, at, velocity, spin, clock);
                }
            }
        }
        intents
    }

    /// Every stone as it is drawn at `now`, the farmyard's clock at `clock`: those this participant holds, in their
    /// hand; one another holds, in theirs, from `right` and `left`, their hands by their numbers, or where it was last
    /// drawn; and the rest where the server side says they are.
    pub fn shown(
        &mut self,
        right: &[(u32, Hand)],
        left: &[(u32, Hand)],
        now: Instant,
        clock: u64,
    ) -> Vec<Shown> {
        let stones: Vec<u32> = self.mirror.bodies().collect();
        stones
            .into_iter()
            .map(|stone| {
                let mine = self
                    .holding
                    .iter()
                    .flatten()
                    .find(|holding| holding.stone == stone);
                let placement = match (mine, self.mirror.held(stone)) {
                    (Some(holding), _) => lifted(stone, holding.placement(now)),
                    (None, Some(held)) => {
                        let hands = match held.hand {
                            tomori_physics::Side::Left => left,
                            tomori_physics::Side::Right => right,
                        };
                        hands
                            .iter()
                            .find(|(number, _)| *number == held.by)
                            .map(|(_, hand)| lifted(stone, held.placement(&placed(hand))))
                            .or_else(|| self.drawn.get(&stone).copied())
                            .unwrap_or_default()
                    }
                    (None, None) => self
                        .mirror
                        .placement(stone, clock)
                        .expect("a stone nobody holds has a placement"),
                };
                self.drawn.insert(stone, placement);
                Shown {
                    stone: stone as usize,
                    transform: placement.matrix(),
                }
            })
            .collect()
    }
}

/// How far a hand at `hand` is from the hull of stone `stone` placed at `placement`, roughly, as from a sphere a
/// little within it.
fn reach(stone: u32, placement: &Placement, hand: Vec3) -> f32 {
    let (width, height) = beach::size(stone as usize);
    (hand.distance(placement.position) - (width * 0.5 + height * 0.5) * 0.5).max(0.0)
}

/// `placement` of stone `stone`, lifted so that none of it is below the beach's shingle, where there is shingle: so
/// that a stone held is never drawn inside the beach, and is let go on it rather than in it.
fn lifted(stone: u32, placement: Placement) -> Placement {
    let lowest = beach::stone(stone as usize)
        .into_iter()
        .map(|point| placement.transform_point(point))
        .filter_map(|point| {
            beach::height(Vec2::new(point.x, point.z)).map(|shingle| shingle - point.y)
        })
        .fold(0.0, f32::max);
    Placement::new(placement.position + Vec3::Y * lowest, placement.rotation)
}

/// How slowly a stone lands, in metres a second, for nothing to be heard.
const QUIET: f32 = 0.4;

/// The sample rate of the stones' sounds.
pub const RATE: u32 = 48_000;

/// The sound of a stone landing at `speed`, a clack on what it struck, or a splash into water, louder the harder.
pub fn sound(water: bool, speed: f32) -> Sound {
    let level = (speed / 4.0).clamp(0.15, 1.0);
    let samples: Vec<f32> = if water { splash() } else { clack() }
        .iter()
        .map(|sample| sample * level)
        .collect();
    Sound::new(samples, RATE)
}

/// A fixed noise, the same every time, from `seed`.
fn noise(seed: u32) -> impl FnMut() -> f32 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// A clack: a few milliseconds of stone striking stone, two ringing tones in a burst of noise, gone in a tenth of a
/// second.
pub fn clack() -> &'static [f32] {
    static CLACK: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    CLACK.get_or_init(|| {
        let mut noise = noise(0x51a7_c1ac);
        (0..RATE as usize / 10)
            .map(|index| {
                let t = index as f32 / RATE as f32;
                let ring = (t * 2_300.0 * std::f32::consts::TAU).sin() * 0.6
                    + (t * 3_700.0 * std::f32::consts::TAU).sin() * 0.4;
                (ring * 0.6 + noise() * 0.4) * (-t * 70.0).exp() * 0.3
            })
            .collect()
    })
}

/// A splash: a burst of water, its noise softened as it falls away, over a third of a second.
pub fn splash() -> &'static [f32] {
    static SPLASH: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    SPLASH.get_or_init(|| {
        let mut noise = noise(0x5b1a_54ed);
        let mut soft = 0.0;
        (0..RATE as usize / 3)
            .map(|index| {
                let t = index as f32 / RATE as f32;
                // A low-pass that closes as it fades, so that the burst turns to a gurgle.
                let open = 0.5 * (-t * 6.0).exp() + 0.05;
                soft += (noise() - soft) * open;
                let attack = (t * 400.0).min(1.0);
                soft * attack * (-t * 9.0).exp() * 0.35
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
