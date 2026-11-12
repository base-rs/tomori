// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The client's half: every body as the side shares and announces it, drawn where it rests, between the snapshots of
//! what moves, [`DELAY`] behind the environment's clock, or, for a body this participant just let go, from their own
//! foresight of it, under gravity from where they let it go, blending into the server's placements over [`BLEND`] as
//! they arrive, so that the holder sees no hitch. A body on its way back fades out where it was and in where it returns
//! to, [`Fade`], each over [`FADE`], and is never drawn between. A held body is placed by its holder's hand, which only
//! the client knows each frame: see [`Mirror::held`].
use std::collections::{BTreeMap, VecDeque};

use glam::{Quat, Vec3};

use crate::message::{State, Update};
use crate::{BLEND, DELAY, FADE, Held, Placement, Precision, Settings};

/// How many snapshots of each moving body are kept: two seconds of them.
const TRACK: usize = 40;
/// How far ahead of its latest snapshot a body is carried on, at most, in milliseconds, and how long the holder's
/// foresight lasts without a snapshot, at most.
const AHEAD: u64 = DELAY;
const FORESIGHT: u64 = 500;

/// A body this participant let go: where, how fast and how it spun, on the environment's clock, and when the first of
/// the server's snapshots after it arrived, on the same clock.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Foresight {
    from: u64,
    at: Placement,
    velocity: Vec3,
    spin: Vec3,
    arrived: Option<u64>,
}

impl Foresight {
    /// Where it is at `clock`, under gravity from where it was let go, for at most [`FORESIGHT`].
    fn at(&self, clock: u64) -> Placement {
        let seconds = clock.saturating_sub(self.from).min(FORESIGHT) as f32 / 1000.0;
        let gravity = Settings::default().gravity;
        Placement::new(
            self.at.position + self.velocity * seconds + gravity * (0.5 * seconds * seconds),
            spun(self.at.rotation, self.spin, seconds),
        )
    }
}

/// `rotation`, turned on by `spin`, radians a second about its axis, for `seconds`.
fn spun(rotation: Quat, spin: Vec3, seconds: f32) -> Quat {
    let angle = spin.length() * seconds;
    if angle <= 0.0 || !angle.is_finite() {
        return rotation;
    }
    (Quat::from_axis_angle(spin.normalize(), angle) * rotation).normalize()
}

/// `from` to `to`, `t` of the way.
fn between(from: &Placement, to: &Placement, t: f32) -> Placement {
    Placement::new(
        from.position.lerp(to.position, t),
        from.rotation.slerp(to.rotation, t),
    )
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
        let eased = |t: f32| t * t * (3.0 - 2.0 * t);
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
    foresight: Option<Foresight>,
    /// Its way back, while it is on it.
    back: Option<Back>,
}

impl Mirrored {
    /// Where its snapshots put it at `clock`, between the two either side, or the first before them; carried on beyond
    /// the last by how it last moved, for at most [`AHEAD`], unless it rests there.
    fn tracked(&self, clock: u64) -> Option<Placement> {
        let (Some(first), Some(last)) = (self.track.front(), self.track.back()) else {
            return None;
        };
        if clock <= first.0 {
            return Some(first.1);
        }
        if clock >= last.0 {
            let before = self
                .track
                .iter()
                .rev()
                .nth(1)
                .filter(|_| self.rested.is_none());
            return Some(match before {
                Some((then, was)) if last.0 > *then => {
                    let ahead = (clock - last.0).min(AHEAD) as f32;
                    let rate = ahead / (last.0 - then) as f32;
                    Placement::new(
                        last.1.position + (last.1.position - was.position) * rate,
                        last.1.rotation,
                    )
                }
                _ => last.1,
            });
        }
        let after = self
            .track
            .iter()
            .position(|(at, _)| *at > clock)
            .expect("before the last");
        let (from, to) = (self.track[after - 1], self.track[after]);
        let t = (clock - from.0) as f32 / (to.0 - from.0) as f32;
        Some(between(&from.1, &to.1, t))
    }

    /// How much of it is drawn at `clock`, [`DELAY`] behind it, as what moves is.
    fn fade(&self, clock: u64) -> Fade {
        self.back
            .map_or(Fade::Whole, |back| back.fade(clock.saturating_sub(DELAY)))
    }

    /// Where it is drawn at `clock`, held or not: on its way back, where it was as it fades out, and where it returns to
    /// as it fades in, nowhere between.
    fn placement(&self, clock: u64) -> Placement {
        if let Some(back) = &self.back {
            return match self.fade(clock) {
                Fade::Out(_) => back.from,
                _ => back.to,
            };
        }
        if let Some(foresight) = &self.foresight {
            let foreseen = foresight.at(clock);
            let Some(arrived) = foresight.arrived else {
                return foreseen;
            };
            let server = self.tracked(clock).unwrap_or(self.rest);
            let t = (clock.saturating_sub(arrived) as f32 / BLEND as f32).min(1.0);
            return between(&foreseen, &server, t);
        }
        match self.rested {
            Some(rested) if clock >= rested + DELAY => self.rest,
            _ => self
                .tracked(clock.saturating_sub(DELAY))
                .unwrap_or(self.rest),
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
                        rested: None,
                        foresight: None,
                        back: None,
                    },
                )
            })
            .collect();
    }

    /// What changed, as the side announced it at `sent` on the environment's clock, arriving at `now` on it, as this
    /// client reckons it. A body on its way back, from then, forgets how it moved, and its snapshots begin anew where it
    /// returns to.
    pub fn apply(&mut self, sent: u64, update: &Update, now: u64) {
        for holding in &update.holding {
            if let Some(body) = self.bodies.get_mut(&holding.body) {
                body.epoch = holding.epoch;
                body.held = holding.held;
                if holding.held.is_some() {
                    body.track.clear();
                    body.rested = None;
                    body.foresight = None;
                    body.back = None;
                }
            }
        }
        for snapshot in &update.moving {
            if let Some(body) = self.bodies.get_mut(&snapshot.body()) {
                if body.track.back().is_some_and(|(at, _)| *at > sent) || body.rested.is_some() {
                    body.track.clear();
                }
                body.rested = None;
                body.track
                    .push_back((sent, snapshot.placement(Precision::Moving)));
                while body.track.len() > TRACK {
                    body.track.pop_front();
                }
                if let Some(foresight) = &mut body.foresight {
                    foresight.arrived.get_or_insert(now);
                }
            }
        }
        for snapshot in &update.rested {
            if let Some(body) = self.bodies.get_mut(&snapshot.body()) {
                body.rest = snapshot.placement(Precision::Rest);
                body.track.push_back((sent, body.rest));
                body.rested = Some(sent);
                if let Some(foresight) = &mut body.foresight {
                    foresight.arrived.get_or_insert(now);
                }
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
            }
        }
    }

    /// Foresee body `body`, which this participant let go at `at` on the environment's clock `clock`, moving at
    /// `velocity` and spinning at `spin`, until the server's snapshots take over.
    pub fn let_go(&mut self, body: u32, at: Placement, velocity: Vec3, spin: Vec3, clock: u64) {
        if let Some(mirrored) = self.bodies.get_mut(&body) {
            mirrored.held = None;
            mirrored.foresight = Some(Foresight {
                from: clock,
                at,
                velocity,
                spin,
                arrived: None,
            });
        }
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
    /// between its snapshots, as this participant foresees it, or on its way back, where it was or where it returns to.
    /// Once the server's snapshots have taken over and it rests, the foresight is forgotten, and once it is back, its
    /// way back.
    pub fn placement(&mut self, body: u32, clock: u64) -> Option<Placement> {
        let mirrored = self
            .bodies
            .get_mut(&body)
            .filter(|body| body.held.is_none())?;
        if mirrored.foresight.is_some_and(|foresight| {
            foresight
                .arrived
                .is_some_and(|arrived| clock >= arrived + BLEND)
                && mirrored.rested.is_some_and(|rested| clock >= rested)
        }) || mirrored.foresight.is_some_and(|foresight| {
            foresight.arrived.is_none() && clock >= foresight.from + FORESIGHT
        }) {
            mirrored.foresight = None;
            mirrored.rested = mirrored
                .rested
                .map(|rested| rested.min(clock.saturating_sub(DELAY)));
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
        Update {
            moving: vec![Snapshot::new(
                body,
                &Placement::at(Vec3::new(x, 1.0, 0.0)),
                Precision::Moving,
            )],
            ..Default::default()
        }
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
        mirror.apply(1_000, &moving(0, 1.0), 1_010);
        mirror.apply(1_050, &moving(0, 2.0), 1_060);
        // Before the first, the first; between them, between, a delay behind.
        assert_eq!(mirror.placement(0, 1_000).unwrap().position.x, 1.0);
        let x = mirror.placement(0, 1_125).unwrap().position.x;
        assert!((x - 1.5).abs() < 1e-3, "{x}");
        // Beyond the last, carried on as it moved, for at most the delay.
        let x = mirror.placement(0, 1_175).unwrap().position.x;
        assert!((x - 2.5).abs() < 1e-3, "{x}");
        assert!((mirror.placement(0, 9_000).unwrap().position.x - 4.0).abs() < 1e-3);
        // Come to rest, it is drawn there once the delay has passed, its snapshots ending there.
        let rest = Placement::new(Vec3::new(2.2, 0.5, 0.0), Quat::from_rotation_y(0.3))
            .snapped(Precision::Rest);
        mirror.apply(
            1_100,
            &Update {
                rested: vec![Snapshot::new(0, &rest, Precision::Rest)],
                ..Default::default()
            },
            1_110,
        );
        let x = mirror.placement(0, 1_175).unwrap().position.x;
        assert!(x > 2.0 && x < 2.2, "{x}");
        assert_eq!(mirror.placement(0, 1_200), Some(rest));
        assert_eq!(mirror.placement(0, 1_201), Some(rest));
        // Moving again, it begins anew; and a snapshot older than the last begins anew too.
        mirror.apply(2_000, &moving(0, 5.0), 2_010);
        assert_eq!(mirror.placement(0, 2_050).unwrap().position.x, 5.0);
        mirror.apply(1_900, &moving(0, 6.0), 2_020);
        assert_eq!(mirror.placement(0, 2_050).unwrap().position.x, 6.0);
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
    }

    #[test]
    fn what_this_participant_lets_go_is_foreseen_then_blended_into_the_server_s_placements() {
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
        assert!(foreseen.rotation.angle_between(Quat::IDENTITY) > 0.3);
        // The server's snapshots arrive: blended from the foresight into them, carried to the clock rather than drawn
        // behind it, over the blend.
        mirror.apply(1_050, &moving(0, 0.1), 1_120);
        mirror.apply(1_100, &moving(0, 0.2), 1_120);
        let half = mirror.placement(0, 1_195).unwrap();
        let server = 0.2 + 0.1 * 95.0 / 50.0;
        assert!(
            (half.position.x - (0.39 + server) / 2.0).abs() < 0.01,
            "{half:?}"
        );
        let blended = mirror.placement(0, 1_270).unwrap();
        assert!((blended.position.x - 0.4).abs() < 1e-3, "{blended:?}");
        // It rests: once the blend is done, the foresight is forgotten, and it is drawn where it rests.
        let rest = Placement::at(Vec3::new(0.5, 0.0, 0.0)).snapped(Precision::Rest);
        mirror.apply(
            1_300,
            &Update {
                rested: vec![Snapshot::new(0, &rest, Precision::Rest)],
                ..Default::default()
            },
            1_310,
        );
        assert_eq!(mirror.placement(0, 1_320), Some(rest));
        assert_eq!(mirror.bodies[&0].foresight, None);
        assert_eq!(mirror.placement(0, 1_330), Some(rest));
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
