// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Loose bodies as a client shows them, whatever they are, the beach's stones or the campsite's sticks, from what the
//! farmyard's server side shares: every body where it rests, or between the snapshots of those that move,
//! `tomori-physics`'s mirror of them; those this participant holds, one in each hand, drawn from their own hand every
//! frame, taken within reach of it or pulled to it along the ray, passed from one hand to the other as the free hand
//! grips what the other holds, and let go, thrown as fast as the hand moved, foreseen until the server's snapshots take
//! over; those the others hold, drawn from the hands their presence brings; and the sounds of those that land; and
//! those on their way back, fading out where they were and in where they return to, which nobody takes until they are
//! there. What differs between them, their shapes, how a hand holds them and how they sound, is
//! their [`Shapes`]. The client never simulates. See `design/physics.md`.
use std::collections::{BTreeMap, VecDeque};
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use glam::Vec3;
use tomori_physics::message::{Intent, LetGo, State, Take, Update};
use tomori_physics::mirror::{Fade, Mirror};
use tomori_physics::{HOLD, PULL, PULL_TIME, Placement, Precision, RELEASE_SPEED, RELEASE_SPIN};
use tomori_scene::client_side::{Actions, Grab, Hand, Phase, Picked, Side, Sound};
use tomori_scene::pointing::Ray;

/// What one kind of loose body is to a client: what it is called, how the ray and a hand meet it, how a hand holds it,
/// how it is drawn clear of the ground, how much of a hand's speed it leaves with and how it sounds as it lands.
pub trait Shapes {
    /// What a body is called as the ray picks it, as `stone-3`, by its number in that name, and back.
    const PREFIX: &'static str;
    /// What the participant is told they point at.
    const NAME: &'static str;
    /// How near a hand comes to a body, as [`Shapes::reach`] measures it, to take it there, rather than pull it along the
    /// ray.
    const REACH: f32;

    /// The number body `body` is called by, and the body called by `number`, if there is one.
    fn number(body: u32) -> u32;
    fn body(number: u32) -> Option<u32>;
    /// How far along the ray from `origin` going `direction`, a unit vector, it meets body `body` placed at `placement`.
    fn met(origin: Vec3, direction: Vec3, body: u32, placement: &Placement) -> Option<f32>;
    /// How far a hand at `hand` is from where it takes body `body`, placed at `placement`.
    fn reach(body: u32, placement: &Placement, hand: Vec3) -> f32;
    /// How far a hand at `hand` is from any part of body `body`, placed at `placement`, as a hand taking it from the
    /// other hand may grip it anywhere.
    fn touch(body: u32, placement: &Placement, hand: Vec3) -> f32;
    /// Where body `body`, at `at`, is held in a hand at `hand` that takes it, `within` reach or pulling it along the
    /// ray: and whether it flies there from where it was.
    fn held(body: u32, at: &Placement, hand: &Placement, within: bool) -> (Placement, bool);
    /// `placement` of body `body`, as it is drawn held or let go: above the ground, should it be drawn below.
    fn lifted(_body: u32, placement: Placement) -> Placement {
        placement
    }
    /// How much of the hand's speed body `body` leaves it with.
    fn heft(_body: u32) -> f32 {
        1.0
    }
    /// The sound of body `body` landing at `speed`, into water or not, if it is heard.
    fn sound(body: u32, water: bool, speed: f32) -> Option<Sound>;
}

/// How near a hand comes to any part of a body this participant holds in their other hand, as [`Shapes::touch`]
/// measures it, to take it there, passing it from that hand: farther than a body lying loose is reached from, so that
/// passing from hand to hand is forgiving.
pub const PASS_REACH: f32 = 0.18;

/// How long a taking waits for the server side to agree before it is given up, as when it was dropped.
pub const UNAGREED: Duration = Duration::from_millis(1500);
/// How long a hand's motion is followed back to say how fast it moved as it let go.
const FOLLOWED: Duration = Duration::from_millis(80);
/// How long the server side may say this participant holds a body that no hand of theirs does before it is let go of
/// again, and how often after: as when their letting go was refused, being one intent too many, or their taking was
/// agreed only after this client gave it up.
pub const ASTRAY: Duration = Duration::from_secs(1);

/// A body this participant holds: which, the epoch their taking makes it, where it is in the hand, where it flew from
/// as it was pulled to the hand and since when, where the hand was over the last moments, when they took it, and whether
/// the server side has agreed.
#[derive(Clone, Debug)]
pub(crate) struct Holding {
    pub(crate) body: u32,
    pub(crate) epoch: u32,
    pub(crate) offset: Placement,
    pub(crate) pulled: Option<(Placement, Instant)>,
    pub(crate) hand: VecDeque<(Instant, Placement)>,
    pub(crate) since: Instant,
    pub(crate) agreed: bool,
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

    /// How fast the body moved and spun as the hand last moved, over the last [`FOLLOWED`], each within the bounds a
    /// release is held to.
    fn motion(&self) -> (Vec3, Vec3) {
        let (now, last) = *self.hand.back().expect("a held body has a hand");
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

/// The place of hand `side` among those this participant holds with, left then right.
fn which(side: Side) -> usize {
    match side {
        Side::Left => 0,
        _ => 1,
    }
}

/// A hand as a placement.
pub fn placed(hand: &Hand) -> Placement {
    Placement::new(hand.position, hand.orientation.normalize())
}

/// Loose bodies of one kind, `S`, as this client shows them.
#[derive(Clone, Debug)]
pub struct Loose<S> {
    pub(crate) mirror: Mirror,
    /// What this participant holds, left hand then right.
    pub(crate) holding: [Option<Holding>; 2],
    /// This participant's number, as the last frame said.
    me: Option<u32>,
    /// Where each body was last drawn: what the ray and the hands meet, and where one another holds stays should their
    /// hand go unreported.
    pub(crate) drawn: BTreeMap<u32, Placement>,
    /// The bodies this participant let go of, or that the server side says they hold though no hand of theirs does,
    /// until the server side's epoch for each moves on: since when, or since each was last let go of again, and at
    /// which epoch.
    astray: BTreeMap<u32, (Instant, u32)>,
    /// The hands that took a body from the other hand since this was last asked, which feel it.
    passed: Vec<Side>,
    shapes: PhantomData<S>,
}

impl<S> Default for Loose<S> {
    fn default() -> Self {
        Self {
            mirror: Mirror::new(),
            holding: [None, None],
            me: None,
            drawn: BTreeMap::new(),
            astray: BTreeMap::new(),
            passed: Vec::new(),
            shapes: PhantomData,
        }
    }
}

impl<S: Shapes> Loose<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bodies as the server side shares them, in its state: every one anew.
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
                S::sound(landed.body, landed.water, landed.speed)
                    .map(|sound| (sound, at.to_array()))
            })
            .collect()
    }

    /// The body this participant holds in hand `side`, if they do.
    pub fn held_in(&self, side: tomori_physics::Side) -> Option<u32> {
        self.holding[side.index()]
            .as_ref()
            .map(|holding| holding.body)
    }

    /// What the server side now says of the bodies this participant holds: their taking agreed, or given up as another
    /// holds it or it has moved on without them. Until then, what it says of an epoch before the one their taking makes,
    /// their own letting go of it before or their own holding of it whose letting go was lost, changes nothing.
    fn agree(&mut self) {
        for holding in &mut self.holding {
            let Some(held) = holding.as_mut() else {
                continue;
            };
            let (epoch, by) = (
                self.mirror.epoch(held.body),
                self.mirror.held(held.body).map(|held| held.by),
            );
            let mine = by.is_some() && by == self.me;
            if !held.agreed && self.mirror.returning(held.body) {
                // Taken as it began its way back, which the side refuses.
                *holding = None;
            } else if epoch == Some(held.epoch) && mine {
                held.agreed = true;
            } else if held.agreed
                || (by.is_some() && !mine)
                || epoch.is_none_or(|epoch| epoch >= held.epoch)
            {
                *holding = None;
            }
        }
    }

    /// Whether body `body` may be taken: nobody holds it, as far as this client knows, or the server side says this
    /// participant does though no hand of theirs does, as when their letting go of it was lost, which it takes again;
    /// and it is not on its way back.
    fn free(&self, body: u32) -> bool {
        !self.mirror.returning(body)
            && self
                .mirror
                .held(body)
                .is_none_or(|held| Some(held.by) == self.me)
            && self
                .holding
                .iter()
                .flatten()
                .all(|holding| holding.body != body)
    }

    /// The body this participant holds in the hand other than `side`, which that hand may take from it.
    fn in_other(&self, side: Side) -> Option<u32> {
        self.holding[1 - which(side)]
            .as_ref()
            .map(|holding| holding.body)
    }

    /// What `ray` meets first of the bodies that may be taken, within [`PULL`], which the grip pulls to the hand.
    pub fn pick(&self, ray: &Ray) -> Option<Picked> {
        self.picked(ray, None)
    }

    /// What the ray of the hand `side` meets first of the bodies that may be taken, or that this participant holds in
    /// the other hand, which its grip passes to it, within [`PULL`].
    pub fn pick_by(&self, side: Side, ray: &Ray) -> Option<Picked> {
        self.picked(ray, self.in_other(side))
    }

    fn picked(&self, ray: &Ray, other: Option<u32>) -> Option<Picked> {
        self.drawn
            .iter()
            .filter(|(body, _)| self.free(**body) || other == Some(**body))
            .filter_map(|(body, placement)| {
                S::met(ray.origin, ray.direction, *body, placement)
                    .filter(|along| *along <= PULL)
                    .map(|along| {
                        Picked::new(format!("{}{}", S::PREFIX, S::number(*body)), S::NAME, along)
                            .handles(Actions::GRAB)
                    })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// The body that may be taken that a hand at `hand` is within [`Shapes::REACH`] of, the nearest.
    pub fn within_reach(&self, hand: Vec3) -> Option<String> {
        self.reached(hand, None)
    }

    /// The nearest body that the hand `side`, at `hand`, is within [`Shapes::REACH`] of, or of any part of which, held in
    /// this participant's other hand, it is within [`PASS_REACH`], which its grip passes to it.
    pub fn within_reach_of(&self, side: Side, hand: Vec3) -> Option<String> {
        self.reached(hand, self.in_other(side))
    }

    fn reached(&self, hand: Vec3, other: Option<u32>) -> Option<String> {
        self.drawn
            .iter()
            .filter_map(|(body, placement)| {
                if other == Some(*body) {
                    Some((*body, S::touch(*body, placement, hand)))
                        .filter(|(_, distance)| *distance <= PASS_REACH)
                } else {
                    Some((*body, S::reach(*body, placement, hand)))
                        .filter(|(body, distance)| *distance <= S::REACH && self.free(*body))
                }
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(body, _)| format!("{}{}", S::PREFIX, S::number(body)))
    }

    /// The hands that took a body from the other hand since this was last asked, which feel it.
    pub fn passed(&mut self) -> Vec<Side> {
        std::mem::take(&mut self.passed)
    }

    /// A frame at `now`, the farmyard's clock at `clock`: the grips on bodies this frame, by this participant,
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
            let Some(body) = grab
                .object
                .strip_prefix(S::PREFIX)
                .and_then(|number| number.parse::<u32>().ok())
                .and_then(S::body)
                .filter(|body| self.mirror.epoch(*body).is_some())
            else {
                continue;
            };
            let (side, which) = match grab.side {
                Side::Left => (tomori_physics::Side::Left, 0),
                _ => (tomori_physics::Side::Right, 1),
            };
            let hand = placed(&grab.hand);
            match grab.phase {
                Phase::Began
                    if participant.is_some()
                        && self.holding[which].is_none()
                        && self.holding[1 - which]
                            .as_ref()
                            .is_some_and(|holding| holding.body == body) =>
                {
                    intents.push(self.pass(body, grab.side, side, &hand, now));
                }
                Phase::Began => {
                    if participant.is_none() || !self.free(body) || self.holding[which].is_some() {
                        continue;
                    }
                    let at = self.drawn[&body];
                    // At the epoch the server side last said; or, for one this participant let go of whose letting go
                    // it has not yet answered, the epoch that letting go makes, which the side takes even if the letting
                    // go was lost: so that a body taken again just after it was dropped is taken, not refused.
                    let said = self.mirror.epoch(body).expect("known above");
                    let epoch = match self.astray.remove(&body) {
                        Some((_, epoch)) if said <= epoch => epoch.wrapping_add(1),
                        _ => said,
                    };
                    let within = S::reach(body, &at, hand.position) <= S::REACH;
                    let (offset, pulled) = S::held(body, &at, &hand, within);
                    let offset = offset.snapped(Precision::Rest);
                    intents.push(Intent::Take(Take {
                        body,
                        epoch,
                        hand: side,
                        offset: offset.encoded(Precision::Rest),
                    }));
                    self.holding[which] = Some(Holding {
                        body,
                        epoch: epoch.wrapping_add(1),
                        offset,
                        pulled: pulled.then_some((at, now)),
                        hand: VecDeque::from([(now, hand)]),
                        since: now,
                        agreed: false,
                    });
                }
                Phase::Changed => {
                    if let Some(holding) = self.holding[which]
                        .as_mut()
                        .filter(|holding| holding.body == body)
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
                        self.holding[which].take_if(|holding| holding.body == body)
                    else {
                        continue;
                    };
                    holding.hand.push_back((now, hand));
                    let at = S::lifted(body, holding.placement(now)).snapped(Precision::Rest);
                    let (velocity, spin) = holding.motion();
                    let (velocity, spin) = (velocity * S::heft(body), spin * S::heft(body));
                    intents.push(Intent::LetGo(LetGo {
                        body,
                        epoch: holding.epoch,
                        at: at.encoded(Precision::Rest),
                        velocity: velocity.to_array(),
                        spin: spin.to_array(),
                        clock,
                    }));
                    self.mirror.let_go(body, at, velocity, spin, clock);
                    self.astray.insert(body, (now, holding.epoch));
                }
            }
        }
        intents.extend(self.again(now, clock));
        intents
    }

    /// Body `body`, held in this participant's other hand, passed to the hand `grab`, `side` to the server side, at
    /// `hand`, at `now`: in that hand at once, at the epoch the other hand's taking made, never let go on the way, held
    /// where the hand took it, within [`PASS_REACH`] of any part of it and no farther than [`HOLD`] from it, or else pulled
    /// to it as along the ray, its toasting, or anything else of it, kept; the other hand holding nothing, though its grip
    /// is still closed, until it lets go and grips again. The latest grip wins: two hands on one body is a two-handed
    /// carry, which is the two-handed proposal's third stage. What to ask the server side.
    fn pass(
        &mut self,
        body: u32,
        grab: Side,
        side: tomori_physics::Side,
        hand: &Placement,
        now: Instant,
    ) -> Intent {
        let which = which(grab);
        let from = self.holding[1 - which]
            .take()
            .expect("held in the other hand");
        // Every body the mirror knows was drawn as it was shared, and one held is drawn each frame where it is held.
        let at = self.drawn[&body];
        let kept = hand.inverse().then(&at).snapped(Precision::Rest);
        let (offset, pulled) =
            if S::touch(body, &at, hand.position) <= PASS_REACH && kept.position.length() <= HOLD {
                (kept, false)
            } else {
                let (offset, _) = S::held(body, &at, hand, false);
                (offset.snapped(Precision::Rest), true)
            };
        self.holding[which] = Some(Holding {
            body,
            epoch: from.epoch.wrapping_add(1),
            offset,
            pulled: pulled.then_some((at, now)),
            hand: VecDeque::from([(now, *hand)]),
            since: now,
            agreed: false,
        });
        self.passed.push(grab);
        Intent::Take(Take {
            body,
            epoch: from.epoch,
            hand: side,
            offset: offset.encoded(Precision::Rest),
        })
    }

    /// The bodies this participant let go of whose letting go the server side has not answered, and those it says they
    /// hold that no hand of theirs does, let go of again, unmoving, where they are drawn, once each has been astray for
    /// [`ASTRAY`], and each [`ASTRAY`] after, until the server side's epoch moves on: so that a body whose letting go
    /// never arrived is not left in their hand, for everyone else, for as long as they stay.
    fn again(&mut self, now: Instant, clock: u64) -> Vec<Intent> {
        let me = self.me;
        for body in self.mirror.bodies() {
            let held = self
                .holding
                .iter()
                .flatten()
                .any(|holding| holding.body == body);
            if me.is_some() && !held && self.mirror.held(body).map(|held| held.by) == me {
                let epoch = self.mirror.epoch(body).expect("known");
                self.astray.entry(body).or_insert((now, epoch));
            }
        }
        let mirror = &self.mirror;
        // Kept until the side's epoch moves on beyond it, as one it has not yet said this participant's taking made.
        self.astray.retain(|body, (_, epoch)| {
            mirror.epoch(*body).is_some_and(|said| said <= *epoch)
                && mirror.held(*body).is_none_or(|held| Some(held.by) == me)
        });
        let mut intents = Vec::new();
        for (body, (since, epoch)) in &mut self.astray {
            // Every body the mirror knows was drawn as it was shared.
            let at = self.drawn[body];
            if now.saturating_duration_since(*since) < ASTRAY {
                continue;
            }
            *since = now;
            intents.push(Intent::LetGo(LetGo {
                body: *body,
                epoch: *epoch,
                at: at.snapped(Precision::Rest).encoded(Precision::Rest),
                velocity: [0.0; 3],
                spin: [0.0; 3],
                clock,
            }));
        }
        intents
    }

    /// Every body as it is drawn at `now`, the farmyard's clock at `clock`, by its number, with how much of it is drawn
    /// and whether this participant holds it: those this participant holds, in their hand; one another holds, in theirs,
    /// from `right` and `left`, their hands by their numbers, or where it was last drawn; and the rest where the server
    /// side says they are, fading on their way back.
    pub fn placed(
        &mut self,
        right: &[(u32, Hand)],
        left: &[(u32, Hand)],
        now: Instant,
        clock: u64,
    ) -> Vec<(u32, Placement, Fade, bool)> {
        let bodies: Vec<u32> = self.mirror.bodies().collect();
        bodies
            .into_iter()
            .map(|body| {
                let mine = self
                    .holding
                    .iter()
                    .flatten()
                    .find(|holding| holding.body == body);
                let placement = match (mine, self.mirror.held(body)) {
                    (Some(holding), _) => S::lifted(body, holding.placement(now)),
                    (None, Some(held)) => {
                        let hands = match held.hand {
                            tomori_physics::Side::Left => left,
                            tomori_physics::Side::Right => right,
                        };
                        hands
                            .iter()
                            .find(|(number, _)| *number == held.by)
                            .map(|(_, hand)| S::lifted(body, held.placement(&placed(hand))))
                            .or_else(|| self.drawn.get(&body).copied())
                            .unwrap_or_default()
                    }
                    (None, None) => self
                        .mirror
                        .placement(body, clock)
                        .expect("a body nobody holds has a placement"),
                };
                self.drawn.insert(body, placement);
                (
                    body,
                    placement,
                    self.mirror.fade(body, clock),
                    mine.is_some(),
                )
            })
            .collect()
    }
}
