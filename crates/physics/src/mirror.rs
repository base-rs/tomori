// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The client's half: every body as the side shares and announces it, drawn where it rests, between the snapshots of
//! what moves, turning as it moves, [`DELAY`] behind the environment's clock, or, for a body this participant just let
//! go, from their own foresight of it, under gravity from where they let it go, until the server's snapshots arrive, and
//! then from those carried to the clock, so that the holder sees no hitch; and one someone else let go, alike, from
//! where it was drawn in their hand, as fast as it moved there. What arrives that moves where a body is drawn is blended
//! in over [`BLEND`], so that nothing jumps. A body on its way back fades out where it was and in where it returns
//! to, [`Fade`], each over [`FADE`], and is never drawn between. A held body is placed by its holder's hand, which only
//! the client knows each frame: see [`Mirror::held`].
use std::collections::{BTreeMap, VecDeque};

use glam::{Quat, Vec3};

use crate::message::{State, Update};
use crate::{
    BLEND, DELAY, FADE, Held, Placement, Precision, RELEASE_SPEED, RELEASE_SPIN, SNAPSHOT_RATE,
    Settings,
};

/// How many snapshots of each moving body are kept: two seconds of them.
const TRACK: usize = 40;
/// How far before its first snapshot or beyond its latest a body is carried on, at most, in milliseconds, and how long
/// the holder's foresight lasts without a snapshot, at most.
const AHEAD: u64 = DELAY;
const FORESIGHT: u64 = 500;
/// How long back where a body was drawn in someone else's hand is followed, in milliseconds, to say how fast it moved
/// as they let it go.
const FOLLOWED: u64 = 80;

/// How a body moves, in metres and radians a second.
type Motion = (Vec3, Vec3);

/// A body this participant let go: where, how fast and how it spun, on the environment's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Foresight {
    from: u64,
    at: Placement,
    velocity: Vec3,
    spin: Vec3,
}

impl Foresight {
    /// How long since it was let go at `clock`, for at most [`FORESIGHT`], in seconds.
    fn seconds(&self, clock: u64) -> f32 {
        clock.saturating_sub(self.from).min(FORESIGHT) as f32 / 1000.0
    }

    /// Where it is at `clock`, under gravity from where it was let go.
    fn at(&self, clock: u64) -> Placement {
        let seconds = self.seconds(clock);
        let gravity = Settings::default().gravity;
        Placement::new(
            self.at.position + self.velocity * seconds + gravity * (0.5 * seconds * seconds),
            spun(self.at.rotation, self.spin, seconds),
        )
    }

    /// How it moves at `clock`.
    fn motion(&self, clock: u64) -> Motion {
        (
            self.velocity + Settings::default().gravity * self.seconds(clock),
            self.spin,
        )
    }
}

/// `rotation`, turned on by `spin`, radians a second about its axis, for `seconds`, or back, for fewer than none.
fn spun(rotation: Quat, spin: Vec3, seconds: f32) -> Quat {
    (Quat::from_scaled_axis(spin * seconds) * rotation).normalize()
}

/// How a body moved from `from` to `to` in `milliseconds`.
fn moved(from: &Placement, to: &Placement, milliseconds: u64) -> Motion {
    let seconds = milliseconds as f32 / 1000.0;
    let turn = to.rotation * from.rotation.inverse();
    // The shorter way round.
    let turn = if turn.w < 0.0 { -turn } else { turn };
    (
        (to.position - from.position) / seconds,
        turn.to_scaled_axis() / seconds,
    )
}

/// `from` to `to`, `t` of the way.
fn between(from: &Placement, to: &Placement, t: f32) -> Placement {
    Placement::new(
        from.position.lerp(to.position, t),
        from.rotation.slerp(to.rotation, t),
    )
}

/// `t`, from 0 to 1, eased in and out.
fn eased(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// How much of a body is drawn: all of it; or, on its way back, fading out where it was, or in where it returns to, by
/// how large it is drawn, from 1 to 0 and from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fade {
    Whole,
    Out(f32),
    In(f32),
}

impl Fade {
    /// How large it is drawn, from 0 to 1, about its middle.
    pub fn scale(self) -> f32 {
        match self {
            Self::Whole => 1.0,
            Self::Out(scale) | Self::In(scale) => scale,
        }
    }
}

/// A body on its way back, since `began` on the environment's clock, from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Back {
    began: u64,
    from: Placement,
    to: Placement,
}

impl Back {
    /// How much of it is drawn at `clock`: whole until it begins, fading out over [`FADE`], in over [`FADE`] more,
    /// eased, and whole again once it is back.
    fn fade(&self, clock: u64) -> Fade {
        let since = clock.saturating_sub(self.began);
        if since < FADE {
            Fade::Out(1.0 - eased(since as f32 / FADE as f32))
        } else if since < 2 * FADE {
            Fade::In(eased((since - FADE) as f32 / FADE as f32))
        } else {
            Fade::Whole
        }
    }
}

/// How far a body was drawn from where its snapshots put it, at `at` on the environment's clock, as something arrived
/// that moved where they put it: by `position` and turned by `rotation`, which it is drawn off from them by, less and
/// less, eased, over [`BLEND`], so that it never jumps.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Correction {
    at: u64,
    position: Vec3,
    rotation: Quat,
}

impl Correction {
    /// `placement`, drawn off by what is left of it at `clock`.
    fn applied(&self, placement: Placement, clock: u64) -> Placement {
        let left = 1.0 - eased((clock.saturating_sub(self.at) as f32 / BLEND as f32).min(1.0));
        Placement::new(
            placement.position + self.position * left,
            (Quat::IDENTITY.slerp(self.rotation, left) * placement.rotation).normalize(),
        )
    }
}

/// One body as this client knows it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Mirrored {
    epoch: u32,
    held: Option<Held>,
    /// Where it rests, or last rested.
    rest: Placement,
    /// The snapshots of it moving, by the environment's clock, ending, once it rests, where it rests.
    track: VecDeque<(u64, Placement)>,
    /// The clock it came to rest at, once it has since it last moved.
    rested: Option<u64>,
    /// This participant's foresight of it, until the server's first snapshot of it arrives.
    foresight: Option<Foresight>,
    /// Once that has, until it rests: drawn carried to the clock rather than behind it, from how it moves as the
    /// snapshots say, or, until there are two, as it was foreseen to.
    ahead: Option<Motion>,
    /// How far it is drawn from where its snapshots put it, as the latest of them arrived.
    correction: Option<Correction>,
    /// Its way back, while it is on it.
    back: Option<Back>,
    /// Where it was drawn in someone else's hand, by the clock, over the last [`FOLLOWED`], while they hold it.
    followed: VecDeque<(u64, Placement)>,
    /// Whether they passed it to their other hand since it was last drawn; and, once it is, how far it is drawn from
    /// where that hand puts it, as it was passed.
    passing: bool,
    passed: Option<Correction>,
}

impl Mirrored {
    /// Where its snapshots put it at `clock`: between the two either side; or, before the first or beyond the last,
    /// carried on from it as it last moved between them, or else by `motion`, for at most [`AHEAD`], unless it rests.
    fn tracked(&self, clock: u64, motion: Option<Motion>) -> Option<Placement> {
        let (Some(first), Some(last)) = (self.track.front(), self.track.back()) else {
            return None;
        };
        if clock > first.0 && clock < last.0 {
            let after = self
                .track
                .iter()
                .position(|(at, _)| *at > clock)
                .expect("before the last");
            let (from, to) = (self.track[after - 1], self.track[after]);
            let t = (clock - from.0) as f32 / (to.0 - from.0) as f32;
            return Some(between(&from.1, &to.1, t));
        }
        let (at, before) = if clock <= first.0 {
            (first, None)
        } else {
            (last, self.track.iter().rev().nth(1))
        };
        let motion = before
            .filter(|(then, _)| at.0 > *then)
            .map(|(then, was)| moved(was, &at.1, at.0 - then))
            .or(motion)
            .filter(|_| self.rested.is_none());
        Some(motion.map_or(at.1, |(velocity, spin)| {
            let since = (clock as i64 - at.0 as i64).clamp(-(AHEAD as i64), AHEAD as i64);
            let seconds = since as f32 / 1000.0;
            Placement::new(
                at.1.position + velocity * seconds,
                spun(at.1.rotation, spin, seconds),
            )
        }))
    }

    /// How much of it is drawn at `clock`, [`DELAY`] behind it, as what moves is.
    fn fade(&self, clock: u64) -> Fade {
        self.back
            .map_or(Fade::Whole, |back| back.fade(clock.saturating_sub(DELAY)))
    }

    /// Where it is drawn at `clock`, but for any correction: as foreseen; carried to the clock; or between its
    /// snapshots, [`DELAY`] behind it, and then where it rests.
    fn drawn(&self, clock: u64) -> Placement {
        if let Some(foresight) = &self.foresight {
            return foresight.at(clock);
        }
        if let Some(motion) = self.ahead {
            return self.tracked(clock, Some(motion)).unwrap_or(self.rest);
        }
        match self.rested {
            Some(rested) if clock >= rested + DELAY => self.rest,
            _ => self
                .tracked(clock.saturating_sub(DELAY), None)
                .unwrap_or(self.rest),
        }
    }

    /// Where it is drawn at `clock`, held or not: on its way back, where it was as it fades out, and where it returns to
    /// as it fades in, nowhere between; or else as [`Mirrored::drawn`] says, corrected.
    fn placement(&self, clock: u64) -> Placement {
        if let Some(back) = &self.back {
            return match self.fade(clock) {
                Fade::Out(_) => back.from,
                _ => back.to,
            };
        }
        let drawn = self.drawn(clock);
        self.correction
            .map_or(drawn, |correction| correction.applied(drawn, clock))
    }

    /// Having been drawn at `before` at `clock`, if it was, just before something arrived, drawn off from where it now
    /// is by as much, less and less, so that it never jumps.
    fn correct(&mut self, before: Option<Placement>, clock: u64) {
        let after = self.drawn(clock);
        self.correction = before
            .filter(|before| *before != after)
            .map(|before| Correction {
                at: clock,
                position: before.position - after.position,
                rotation: (before.rotation * after.rotation.inverse()).normalize(),
            });
    }

    /// Where it was last drawn in someone else's hand, and how fast and spinning it moved there, over the last
    /// [`FOLLOWED`], within the bounds a letting go is held to, as a foresight from then.
    fn followed(&self) -> Option<Foresight> {
        let (&(from, first), &(to, last)) = self.followed.front().zip(self.followed.back())?;
        let (velocity, spin) = if to > from {
            moved(&first, &last, to - from)
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        };
        Some(Foresight {
            from: to,
            at: last,
            velocity: velocity.clamp_length_max(RELEASE_SPEED),
            spin: spin.clamp_length_max(RELEASE_SPIN),
        })
    }

    /// Where it would be drawn at `clock` in someone else's hand, moving on as it moved there, if it was drawn there.
    fn foreseen(&self, clock: u64) -> Option<Placement> {
        self.followed().map(|followed| {
            let seconds = clock.saturating_sub(followed.from) as f32 / 1000.0;
            Placement::new(
                followed.at.position + followed.velocity * seconds,
                spun(followed.at.rotation, followed.spin, seconds),
            )
        })
    }

    /// Let go by someone else: foreseen from where it was last drawn in their hand, as fast and spinning as it moved
    /// there, as its holder foresees it, if it was drawn there.
    fn freed(&mut self) -> bool {
        let Some(foresight) = self.followed() else {
            return false;
        };
        self.followed.clear();
        self.passed = None;
        self.track.clear();
        self.rested = None;
        self.ahead = None;
        self.correction = None;
        self.foresight = Some(foresight);
        true
    }

    /// Its foresight, once the server's first snapshot of it arrives at `clock`, giving way to them.
    fn answered(&mut self, clock: u64) {
        if let Some(foresight) = self.foresight.take() {
            self.ahead = Some(foresight.motion(clock));
        }
    }
}

/// Every body as this client knows it, by its number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mirror {
    bodies: BTreeMap<u32, Mirrored>,
}

impl Mirror {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bodies as the side shares them, in its state, every one anew.
    pub fn share(&mut self, state: &State) {
        self.bodies = state
            .bodies
            .iter()
            .map(|shared| {
                let rest = shared.at.placement(Precision::Rest);
                (
                    shared.body,
                    Mirrored {
                        epoch: shared.epoch,
                        held: shared.held,
                        rest,
                        track: if shared.moving {
                            VecDeque::from([(0, rest)])
                        } else {
                            VecDeque::new()
                        },
                        rested: (!shared.moving && shared.held.is_none()).then_some(0),
                        ..Default::default()
                    },
                )
            })
            .collect();
    }

    /// What changed, as the side announced it at `sent` on the environment's clock, arriving at `now` on it, as this
    /// client reckons it. Each body it moves is drawn off from where it now is by as far as that moved it, less and
    /// less, over [`BLEND`], so that it never jumps; one beginning to move from rest moves from where it rested, a
    /// snapshot's interval before. One someone else let go is drawn on from where it was drawn in their hand, as
    /// [`Mirror::drawn_held`] said, as fast as it moved there, foreseen until the server's snapshots take over and then
    /// carried to the clock on them, as for its holder. A body on its way back, from then, forgets how it moved, and its
    /// snapshots begin anew where it returns to.
    pub fn apply(&mut self, sent: u64, update: &Update, now: u64) {
        // What someone else let go that was never drawn in their hand is drawn from the first snapshot, not drawn off.
        let mut freed = Vec::new();
        for holding in &update.holding {
            if let Some(body) = self.bodies.get_mut(&holding.body) {
                if body.held.is_some() && holding.held.is_none() && !body.freed() {
                    freed.push(holding.body);
                }
                if body.held.is_none() {
                    // Taken anew, followed anew.
                    body.followed.clear();
                    body.passing = false;
                    body.passed = None;
                } else if holding.held.is_some() && holding.held != body.held {
                    body.passing = true;
                }
                body.epoch = holding.epoch;
                body.held = holding.held;
                if holding.held.is_some() {
                    body.track.clear();
                    body.rested = None;
                    body.foresight = None;
                    body.ahead = None;
                    body.correction = None;
                    body.back = None;
                }
            }
        }
        for snapshot in &update.moving {
            if let Some(body) = self.bodies.get_mut(&snapshot.body()) {
                let before = (!freed.contains(&snapshot.body())).then(|| body.placement(now));
                if body.track.back().is_some_and(|(at, _)| *at > sent) {
                    body.track.clear();
                } else if let Some(rested) = body.rested {
                    body.track.clear();
                    let from = rested.max(sent.saturating_sub(1000 / SNAPSHOT_RATE as u64));
                    if from < sent {
                        body.track.push_back((from, body.rest));
                    }
                }
                body.rested = None;
                body.track
                    .push_back((sent, snapshot.placement(Precision::Moving)));
                while body.track.len() > TRACK {
                    body.track.pop_front();
                }
                body.answered(now);
                body.correct(before, now);
            }
        }
        for snapshot in &update.rested {
            if let Some(body) = self.bodies.get_mut(&snapshot.body()) {
                let before = (!freed.contains(&snapshot.body())).then(|| body.placement(now));
                body.rest = snapshot.placement(Precision::Rest);
                body.track.push_back((sent, body.rest));
                body.rested = Some(sent);
                body.answered(now);
                body.correct(before, now);
            }
        }
        for returning in &update.returning {
            if let Some(body) = self.bodies.get_mut(&returning.body) {
                let to = returning.to.placement(Precision::Rest);
                body.back = Some(Back {
                    began: sent,
                    from: returning.from.placement(Precision::Rest),
                    to,
                });
                body.rest = to;
                body.track.clear();
                body.rested = None;
                body.foresight = None;
                body.ahead = None;
                body.correction = None;
            }
        }
    }

    /// Foresee body `body`, which this participant let go at `at` on the environment's clock `clock`, moving at
    /// `velocity` and spinning at `spin`, until the server's snapshots take over.
    pub fn let_go(&mut self, body: u32, at: Placement, velocity: Vec3, spin: Vec3, clock: u64) {
        if let Some(mirrored) = self.bodies.get_mut(&body) {
            // In their hand until now, it neither rests nor moves as its snapshots said.
            mirrored.held = None;
            mirrored.followed.clear();
            mirrored.passed = None;
            mirrored.track.clear();
            mirrored.rested = None;
            mirrored.ahead = None;
            mirrored.correction = None;
            mirrored.foresight = Some(Foresight {
                from: clock,
                at,
                velocity,
                spin,
            });
        }
    }

    /// Body `body`, which someone else holds, put at `at` at `clock` on the environment's clock by their hand; and where
    /// it is drawn: there, or, just passed to their other hand, which puts it elsewhere, on from where it was drawn as it
    /// moved, drawn off from where that hand puts it by as far, less and less, over [`BLEND`], so that it never jumps.
    /// So that, once they let it go, it is drawn on from where it was drawn, as fast as it moved over the last
    /// [`FOLLOWED`].
    pub fn drawn_held(&mut self, body: u32, at: Placement, clock: u64) -> Placement {
        let Some(mirrored) = self
            .bodies
            .get_mut(&body)
            .filter(|body| body.held.is_some())
        else {
            return at;
        };
        if std::mem::take(&mut mirrored.passing) {
            mirrored.passed = mirrored.foreseen(clock).map(|was| Correction {
                at: clock,
                position: was.position - at.position,
                rotation: (was.rotation * at.rotation.inverse()).normalize(),
            });
        }
        if mirrored
            .passed
            .is_some_and(|passed| clock >= passed.at + BLEND)
        {
            mirrored.passed = None;
        }
        let at = mirrored
            .passed
            .map_or(at, |passed| passed.applied(at, clock));
        let followed = &mut mirrored.followed;
        if followed
            .back()
            .is_some_and(|(then, _)| *then > clock || clock - then > FOLLOWED)
        {
            followed.clear();
        }
        followed.push_back((clock, at));
        while followed.len() > 2 && clock - followed[1].0 >= FOLLOWED {
            followed.pop_front();
        }
        at
    }

    /// Whether body `body` is known, and its epoch.
    pub fn epoch(&self, body: u32) -> Option<u32> {
        self.bodies.get(&body).map(|body| body.epoch)
    }

    /// Who holds body `body`, if anyone does, as the side last said.
    pub fn held(&self, body: u32) -> Option<Held> {
        self.bodies.get(&body).and_then(|body| body.held)
    }

    /// Whether body `body` is on its way back, which nobody may take until it is there, as it was last drawn.
    pub fn returning(&self, body: u32) -> bool {
        self.bodies
            .get(&body)
            .is_some_and(|body| body.back.is_some())
    }

    /// How much of body `body` is drawn at `clock` on the environment's clock: whole, unless it is on its way back.
    pub fn fade(&self, body: u32, clock: u64) -> Fade {
        self.bodies
            .get(&body)
            .map_or(Fade::Whole, |body| body.fade(clock))
    }

    /// Where body `body` is drawn at `clock` on the environment's clock, unless someone holds it: where it rests,
    /// between its snapshots, as this participant foresees it, carried to the clock once the server's snapshots have
    /// taken over from that, or on its way back, where it was or where it returns to. A foresight nothing answers for
    /// [`FORESIGHT`] is given up; one answered is drawn behind the clock again once it rests; and a body is back once
    /// it is whole.
    pub fn placement(&mut self, body: u32, clock: u64) -> Option<Placement> {
        let mirrored = self
            .bodies
            .get_mut(&body)
            .filter(|body| body.held.is_none())?;
        if mirrored
            .foresight
            .is_some_and(|foresight| clock >= foresight.from + FORESIGHT)
        {
            mirrored.foresight = None;
        }
        if let Some(rested) = mirrored.rested.filter(|rested| clock >= *rested)
            && mirrored.ahead.take().is_some()
        {
            mirrored.rested = Some(rested.min(clock.saturating_sub(DELAY)));
        }
        if mirrored
            .correction
            .is_some_and(|correction| clock >= correction.at + BLEND)
        {
            mirrored.correction = None;
        }
        if mirrored.fade(clock) == Fade::Whole {
            mirrored.back = None;
        }
        Some(mirrored.placement(clock))
    }

    /// The numbers of the bodies it knows, in order.
    pub fn bodies(&self) -> impl Iterator<Item = u32> + '_ {
        self.bodies.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Holding, Shared, Snapshot};
    use crate::{Encoded, Side};

    fn state() -> State {
        State {
            bodies: (0..3)
                .map(|body| Shared {
                    body,
                    epoch: body,
                    at: Placement::at(Vec3::new(body as f32, 0.0, 0.0)).encoded(Precision::Rest),
                    moving: body == 2,
                    held: (body == 1).then_some(Held {
                        by: 7,
                        hand: Side::Left,
                        offset: Encoded::default(),
                    }),
                })
                .collect(),
        }
    }

    fn moving(body: u32, x: f32) -> Update {
        turned(body, x, 0.0)
    }

    /// Body `body` moving, at `x` a metre up, turned `angle` about the vertical.
    fn turned(body: u32, x: f32, angle: f32) -> Update {
        Update {
            moving: vec![Snapshot::new(
                body,
                &Placement::new(Vec3::new(x, 1.0, 0.0), Quat::from_rotation_y(angle)),
                Precision::Moving,
            )],
            ..Default::default()
        }
    }

    fn rested(body: u32, at: &Placement) -> Update {
        Update {
            rested: vec![Snapshot::new(body, at, Precision::Rest)],
            ..Default::default()
        }
    }

    /// How far `at` is from `x`, and how far it is turned from `angle` about the vertical.
    fn off(at: Placement, x: f32, angle: f32) -> (f32, f32) {
        (
            (at.position.x - x).abs(),
            at.rotation.angle_between(Quat::from_rotation_y(angle)),
        )
    }

    /// How far `a` is from `b`, and how far it is turned from it.
    fn apart(a: Placement, b: Placement) -> (f32, f32) {
        (
            a.position.distance(b.position),
            a.rotation.angle_between(b.rotation),
        )
    }

    #[test]
    fn a_shared_body_rests_where_it_was_said_and_a_held_one_is_its_holder_s_to_place() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        assert_eq!(mirror.bodies().collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(mirror.epoch(1), Some(1));
        assert_eq!(mirror.epoch(9), None);
        assert_eq!(mirror.placement(0, 5_000), Some(Placement::at(Vec3::ZERO)));
        assert_eq!(mirror.held(1).map(|held| held.by), Some(7));
        assert_eq!(mirror.placement(1, 5_000), None);
        assert_eq!(mirror.placement(9, 5_000), None);
        // One moving as it was shared is where it was until its snapshots come.
        assert_eq!(
            mirror.placement(2, 5_000),
            Some(Placement::at(Vec3::new(2.0, 0.0, 0.0)))
        );
    }

    #[test]
    fn what_moves_is_drawn_between_its_snapshots_behind_the_clock_and_then_where_it_rests() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        mirror.apply(1_000, &turned(0, 1.0, 0.0), 1_010);
        mirror.apply(1_050, &turned(0, 2.0, 0.2), 1_060);
        // Beginning to move from rest, it moves from where it rested, a snapshot's interval before the first; between
        // them, it is between, turned between, a delay behind.
        assert_eq!(mirror.placement(0, 1_000), Some(Placement::at(Vec3::ZERO)));
        let at = mirror.placement(0, 1_075).unwrap();
        assert!(
            (at.position - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-3,
            "{at:?}"
        );
        let (x, angle) = off(mirror.placement(0, 1_125).unwrap(), 1.5, 0.1);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        // Beyond the last, carried on as it moved and turned, for at most the delay.
        let (x, angle) = off(mirror.placement(0, 1_175).unwrap(), 2.5, 0.3);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        let carried = mirror.placement(0, 1_200).unwrap();
        let (x, angle) = off(carried, 3.0, 0.4);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        // A snapshot that puts it elsewhere than it was carried to moves it there without a jump, over the blend.
        mirror.apply(1_100, &turned(0, 2.5, 0.1), 1_200);
        let (x, angle) = apart(mirror.placement(0, 1_200).unwrap(), carried);
        assert!(x < 1e-4 && angle < 1e-4, "{x} {angle}");
        assert!(mirror.bodies[&0].correction.is_some());
        let (x, angle) = off(mirror.placement(0, 1_350).unwrap(), 3.5, -0.1);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        assert_eq!(mirror.bodies[&0].correction, None);
        // Come to rest, it is drawn there once the delay has passed, its snapshots ending there.
        let rest = Placement::new(Vec3::new(2.7, 0.5, 0.0), Quat::from_rotation_y(0.3))
            .snapped(Precision::Rest);
        mirror.apply(1_150, &rested(0, &rest), 1_360);
        let x = mirror.placement(0, 1_400).unwrap().position.x;
        assert!(x > 3.0 && x < 3.5, "{x}");
        assert_eq!(mirror.placement(0, 1_550), Some(rest));
        assert_eq!(mirror.placement(0, 1_551), Some(rest));
        // Moving again, it begins anew from where it rested; and a snapshot older than the last begins anew too, as
        // does one said as it came to rest.
        mirror.apply(2_000, &moving(0, 5.0), 2_010);
        assert_eq!(mirror.placement(0, 2_050), Some(rest));
        assert_eq!(mirror.placement(0, 2_100).unwrap().position.x, 5.0);
        mirror.apply(1_900, &moving(0, 6.0), 2_110);
        assert_eq!(mirror.placement(0, 2_300).unwrap().position.x, 6.0);
        mirror.apply(2_500, &rested(0, &rest), 2_510);
        mirror.apply(2_500, &moving(0, 7.0), 2_520);
        assert_eq!(mirror.bodies[&0].track.len(), 1);
        assert_eq!(mirror.placement(0, 2_800).unwrap().position.x, 7.0);
        // Only so many snapshots are kept.
        for step in 0..100 {
            mirror.apply(3_000 + step * 50, &moving(0, step as f32), 3_000);
        }
        assert_eq!(mirror.bodies[&0].track.len(), TRACK);
        // Snapshots and rests of bodies it does not know change nothing.
        let before = mirror.clone();
        mirror.apply(9_000, &moving(9, 1.0), 9_000);
        mirror.apply(
            9_000,
            &Update {
                rested: vec![Snapshot::new(9, &rest, Precision::Rest)],
                holding: vec![Holding {
                    body: 9,
                    epoch: 1,
                    held: None,
                }],
                ..Default::default()
            },
            9_000,
        );
        assert_eq!(mirror, before);
    }

    #[test]
    fn taken_a_body_forgets_how_it_moved_and_let_go_it_is_drawn_again() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        mirror.apply(1_000, &moving(0, 1.0), 1_000);
        let held = Held {
            by: 3,
            hand: Side::Right,
            offset: Encoded::default(),
        };
        mirror.apply(
            1_050,
            &Update {
                holding: vec![Holding {
                    body: 0,
                    epoch: 5,
                    held: Some(held),
                }],
                ..Default::default()
            },
            1_050,
        );
        assert_eq!((mirror.epoch(0), mirror.held(0)), (Some(5), Some(held)));
        assert_eq!(mirror.placement(0, 1_100), None);
        mirror.apply(
            1_100,
            &Update {
                holding: vec![Holding {
                    body: 0,
                    epoch: 6,
                    held: None,
                }],
                ..Default::default()
            },
            1_100,
        );
        assert_eq!(mirror.placement(0, 1_200), Some(Placement::at(Vec3::ZERO)));
        // Let go by someone else, drawn from its snapshots, from the first, not from where it rested before they took
        // it, which it was not drawn at.
        mirror.apply(
            1_300,
            &Update {
                holding: vec![Holding {
                    body: 0,
                    epoch: 7,
                    held: Some(held),
                }],
                ..Default::default()
            },
            1_300,
        );
        mirror.apply(
            1_400,
            &Update {
                holding: vec![Holding {
                    body: 0,
                    epoch: 8,
                    held: None,
                }],
                ..moving(0, 1.0)
            },
            1_410,
        );
        assert_eq!(mirror.bodies[&0].correction, None);
        assert_eq!(
            mirror.placement(0, 1_420),
            Some(Placement::at(Vec3::new(1.0, 1.0, 0.0)))
        );
    }

    #[test]
    fn what_someone_else_lets_go_is_drawn_on_from_their_hand_as_fast_as_it_moved_there() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        // Body 1, held by participant 7, drawn in their hand each frame, moving along x at 2 m/s and turning about the
        // vertical at a radian a second; followed for the last 80 ms only, anew after a gap, and passed to their other
        // hand on the way, still followed. A body nobody holds is not followed.
        let hand = |clock: u64| {
            let seconds = (clock as f32 - 1_000.0) / 1_000.0;
            Placement::new(
                Vec3::new(2.0 * seconds, 1.0, 0.0),
                Quat::from_rotation_y(seconds),
            )
        };
        mirror.drawn_held(1, Placement::at(Vec3::new(9.0, 0.0, 0.0)), 500);
        for clock in (800..=996).step_by(14) {
            mirror.drawn_held(1, hand(clock), clock);
            if clock == 912 {
                mirror.apply(
                    clock,
                    &Update {
                        holding: vec![Holding {
                            body: 1,
                            epoch: 2,
                            held: Some(Held {
                                by: 7,
                                hand: Side::Right,
                                offset: Encoded::default(),
                            }),
                        }],
                        ..Default::default()
                    },
                    clock,
                );
            }
        }
        mirror.drawn_held(0, hand(996), 996);
        assert!(mirror.bodies[&0].followed.is_empty());
        let followed = &mirror.bodies[&1].followed;
        assert_eq!((followed[0].0, followed.len()), (912, 7));
        // Let go, with the side's first snapshot, as it had it a little behind their hand: drawn on from where it was
        // drawn in their hand, as fast as it moved there, under gravity, and then on the snapshots carried to the clock,
        // each frame farther along, until it is where they say.
        let snapshot = |sent: u64| Snapshot::new(1, &hand(sent), Precision::Moving);
        mirror.apply(
            1_000,
            &Update {
                holding: vec![Holding {
                    body: 1,
                    epoch: 3,
                    held: None,
                }],
                moving: vec![snapshot(1_000)],
                ..Default::default()
            },
            1_010,
        );
        let foreseen = Placement::new(
            Vec3::new(0.02, 1.0 - 0.5 * 9.81 * 0.014 * 0.014, 0.0),
            Quat::from_rotation_y(0.01),
        );
        let (x, angle) = apart(mirror.placement(1, 1_010).unwrap(), foreseen);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        assert!(mirror.bodies[&1].followed.is_empty());
        let mut was = mirror.placement(1, 1_010).unwrap();
        let mut next = 1_050;
        for clock in (1_024..1_500).step_by(14) {
            if clock >= next + 10 && next <= 1_400 {
                mirror.apply(
                    next,
                    &Update {
                        moving: vec![snapshot(next)],
                        ..Default::default()
                    },
                    clock,
                );
                next += 50;
            }
            let at = mirror.placement(1, clock).unwrap();
            let step = at.position.x - was.position.x;
            assert!(step > 0.01 && step < 0.05, "{clock}: {step}");
            was = at;
        }
        let (x, angle) = apart(mirror.placement(1, 1_490).unwrap(), hand(1_490));
        assert!(x < 1e-2 && angle < 1e-2, "{x} {angle}");
        // Taken again, drawn in their hand only once and let go: falling from there, unmoving as it begins to.
        let holding = |epoch, held| Update {
            holding: vec![Holding {
                body: 1,
                epoch,
                held,
            }],
            ..Default::default()
        };
        let held = state().bodies[1].held;
        mirror.apply(2_000, &holding(4, held), 2_000);
        let at = Placement::at(Vec3::new(5.0, 1.0, 0.0));
        mirror.drawn_held(1, at, 2_000);
        mirror.apply(2_000, &holding(5, None), 2_000);
        let fallen = mirror.placement(1, 2_100).unwrap().position;
        assert_eq!((fallen.x, fallen.z), (5.0, 0.0));
        assert!(
            (fallen.y - (1.0 - 0.5 * 9.81 * 0.01)).abs() < 1e-4,
            "{fallen}"
        );
    }

    #[test]
    fn what_someone_else_passes_to_their_other_hand_is_eased_there_from_where_it_was() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        // Body 1, held by participant 7 in their left hand, moving along x at a metre a second, drawn where it is put.
        let moving = |clock: u64| Placement::at(Vec3::new(clock as f32 / 1000.0, 1.0, 0.0));
        for clock in (1_000..=1_098).step_by(14) {
            assert_eq!(mirror.drawn_held(1, moving(clock), clock), moving(clock));
        }
        // Passed to their right hand, which puts it 5 cm higher and turned: drawn on from where it was as it moved, then
        // eased into that hand, each frame moving on, rising no more than a centimetre, until it is where it is put.
        mirror.apply(
            1_100,
            &Update {
                holding: vec![Holding {
                    body: 1,
                    epoch: 2,
                    held: Some(Held {
                        by: 7,
                        hand: Side::Right,
                        offset: Encoded::default(),
                    }),
                }],
                ..Default::default()
            },
            1_100,
        );
        let other = |clock: u64| {
            Placement::new(
                moving(clock).position + Vec3::new(0.0, 0.05, 0.0),
                Quat::from_rotation_y(0.2),
            )
        };
        let mut was = mirror.drawn_held(1, other(1_112), 1_112);
        let (x, angle) = apart(was, moving(1_112));
        assert!(x < 1e-4 && angle < 1e-4, "{x} {angle}");
        for clock in (1_126..1_300).step_by(14) {
            let at = mirror.drawn_held(1, other(clock), clock);
            assert!(at.position.x - was.position.x > 0.013, "{clock}");
            assert!((at.position.y - was.position.y).abs() < 0.01, "{clock}");
            was = at;
        }
        assert_eq!(mirror.drawn_held(1, other(1_300), 1_300), other(1_300));
        assert_eq!(mirror.bodies[&1].passed, None);
        // A body nobody holds is drawn where it is put.
        assert_eq!(mirror.drawn_held(0, other(1_300), 1_300), other(1_300));
    }

    #[test]
    fn what_this_participant_lets_go_is_foreseen_then_carried_to_the_clock_on_the_server_s_placements()
     {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        let at = Placement::at(Vec3::new(0.0, 1.0, 0.0));
        mirror.let_go(
            0,
            at,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 4.0, 0.0),
            1_000,
        );
        mirror.let_go(9, at, Vec3::ZERO, Vec3::ZERO, 1_000);
        // Foreseen under gravity, spinning, before anything arrives.
        let foreseen = mirror.placement(0, 1_100).unwrap();
        assert!((foreseen.position.x - 0.2).abs() < 1e-4);
        assert!((foreseen.position.y - (1.0 - 0.5 * 9.81 * 0.01)).abs() < 1e-4);
        let (_, angle) = off(foreseen, 0.2, 0.4);
        assert!(angle < 1e-4, "{angle}");
        // The server's first snapshot arrives, behind where it was foreseen: drawn on from where it was, without a
        // jump, carried to the clock as it was foreseen to move, until there are two, before the first as after it.
        let before = mirror.placement(0, 1_120).unwrap();
        mirror.apply(1_050, &moving(0, 0.1), 1_120);
        let (x, angle) = apart(mirror.placement(0, 1_120).unwrap(), before);
        assert!(x < 1e-4 && angle < 1e-4, "{x} {angle}");
        assert_eq!(mirror.bodies[&0].foresight, None);
        let (x, angle) = off(mirror.placement(0, 1_300).unwrap(), 0.3, 0.4);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        let motion = mirror.bodies[&0].ahead;
        let (x, angle) = off(mirror.bodies[&0].tracked(1_000, motion).unwrap(), 0.0, -0.2);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        // A second arrives: carried on as the two say, still without a jump.
        let before = mirror.placement(0, 1_310).unwrap();
        mirror.apply(1_100, &turned(0, 0.2, 0.1), 1_310);
        let (x, angle) = apart(mirror.placement(0, 1_310).unwrap(), before);
        assert!(x < 1e-4 && angle < 1e-4, "{x} {angle}");
        let carried = mirror.placement(0, 1_500).unwrap();
        let (x, angle) = off(carried, 0.4, 0.3);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        assert_eq!(carried.position.y, 1.0);
        // It rests: drawn on from where it was to where it rests, and then, the foresight forgotten, behind the clock,
        // where it rests.
        let rest = Placement::at(Vec3::new(0.5, 0.0, 0.0)).snapped(Precision::Rest);
        mirror.apply(1_400, &rested(0, &rest), 1_510);
        let (x, angle) = apart(mirror.placement(0, 1_510).unwrap(), carried);
        assert!(x < 1e-3 && angle < 1e-3, "{x} {angle}");
        assert_eq!(mirror.bodies[&0].ahead, None);
        assert_ne!(mirror.placement(0, 1_600), Some(rest));
        assert_eq!(mirror.placement(0, 1_660), Some(rest));
        assert_eq!(mirror.placement(0, 1_670), Some(rest));
        assert_eq!(mirror.bodies[&0].foresight, None);
    }

    #[test]
    fn a_body_on_its_way_back_fades_out_where_it_was_and_in_where_it_returns_to_in_one_jump() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        let (from, to) = (
            Placement::at(Vec3::new(0.0, 0.0, 0.0)),
            Placement::new(Vec3::new(5.0, 0.5, 0.0), Quat::from_rotation_y(1.0))
                .snapped(Precision::Rest),
        );
        let back = |body| Update {
            returning: vec![crate::message::Returning {
                body,
                from: from.encoded(Precision::Rest),
                to: to.encoded(Precision::Rest),
            }],
            ..Default::default()
        };
        // Foreseen as this participant let it go, then lost, and taken back at 1 s.
        mirror.let_go(0, from, Vec3::new(8.0, 0.0, 0.0), Vec3::ZERO, 900);
        mirror.apply(1_000, &back(0), 1_010);
        mirror.apply(1_000, &back(9), 1_010);
        assert!(mirror.returning(0) && !mirror.returning(9));
        assert_eq!(mirror.fade(9, 1_500), Fade::Whole);
        assert_eq!(mirror.bodies[&0].foresight, None);
        // A delay behind, as what moves is: whole where it was, until it begins.
        assert_eq!(mirror.placement(0, 1_050), Some(from));
        assert_eq!(mirror.fade(0, 1_050), Fade::Out(1.0));
        assert_eq!(mirror.fade(0, 1_350), Fade::Out(0.5));
        assert_eq!(mirror.fade(0, 1_850), Fade::In(0.5));
        // Each frame where it was or where it returns to, jumping once, as it is drawn smallest, and never between; and
        // never drawn larger or smaller than a little more than a frame's fading.
        let mut jumps = 0;
        let (mut was, mut scale) = (from, 1.0);
        for clock in (1_000..2_300).step_by(5) {
            let at = mirror.placement(0, clock).unwrap();
            let fade = mirror.fade(0, clock);
            assert!(at == from || at == to, "{clock}: {at:?}");
            if at != was {
                jumps += 1;
                assert!(fade.scale() < 0.01, "{clock}: {fade:?}");
            }
            assert!((fade.scale() - scale).abs() < 0.02, "{clock}: {fade:?}");
            (was, scale) = (at, fade.scale());
        }
        assert_eq!(jumps, 1);
        // Back, it is whole where it returns to, no longer on its way, and moving from there begins anew there.
        assert_eq!(mirror.fade(0, 2_300), Fade::Whole);
        assert!(!mirror.returning(0));
        mirror.apply(2_300, &moving(0, 5.0), 2_310);
        let x = mirror.placement(0, 2_350).unwrap().position.x;
        assert!((x - 5.0).abs() < 1e-3, "{x}");
        // Taken on its way back, as the side refuses, but should it say so, it is in the hand whole.
        mirror.apply(3_000, &back(0), 3_000);
        mirror.apply(
            3_100,
            &Update {
                holding: vec![Holding {
                    body: 0,
                    epoch: 1,
                    held: Some(Held {
                        by: 7,
                        hand: Side::Left,
                        offset: Encoded::default(),
                    }),
                }],
                ..Default::default()
            },
            3_100,
        );
        assert!(!mirror.returning(0));
        assert_eq!(mirror.fade(0, 3_200), Fade::Whole);
    }

    #[test]
    fn a_foresight_nothing_answers_is_given_up() {
        let mut mirror = Mirror::new();
        mirror.share(&state());
        let at = Placement::at(Vec3::new(0.0, 1.0, 0.0));
        mirror.let_go(0, at, Vec3::ZERO, Vec3::ZERO, 1_000);
        assert!(mirror.placement(0, 1_400).unwrap().position.y < 1.0);
        assert_eq!(mirror.placement(0, 1_600), Some(Placement::at(Vec3::ZERO)));
        // A rest arriving first, as for a body let go where it lay, is blended into too.
        mirror.let_go(2, at, Vec3::ZERO, Vec3::ZERO, 2_000);
        let rest = Placement::at(Vec3::new(2.0, 0.0, 0.0)).snapped(Precision::Rest);
        mirror.apply(
            2_000,
            &Update {
                rested: vec![Snapshot::new(2, &rest, Precision::Rest)],
                ..Default::default()
            },
            2_010,
        );
        assert_eq!(mirror.placement(2, 2_200), Some(rest));
        assert_eq!(spun(Quat::IDENTITY, Vec3::ZERO, 1.0), Quat::IDENTITY);
    }
}
