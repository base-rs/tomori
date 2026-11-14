// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How a person looks: a small swarm of glowing cubes in their colour, shaped as a person where the agent's is a
//! shapeless cloud. A rounded head, its core the brightest, with two soft, round eyes on its face that warm to gold and
//! glow while it faces this participant, and a little orbit of tiny cubes about it that quickens and brightens while
//! their voice plays; shoulders and a torso below it, turning with the head's yaw, damped, and never with its pitch or
//! roll, dissolving below the chest into cubes that fall, shrink and fade, with no legs; and an arm from each shoulder
//! to each hand by two bones, its elbow bending out and down, the hand a small, bright cluster. Each cube is brighter
//! towards the middle of what it is part of, as seen, and dimmer and smaller at its outline, and shimmers a little, so
//! that a person glows as a volume of light rather than standing as a solid; every cube drifts a little about where it
//! rests, and fewer are drawn farther away. See `design/people.md`.
use std::f32::consts::TAU;
use std::sync::LazyLock;

use glam::{Mat3, Quat, Vec3};

use super::Present;
use crate::delegation::Cube;

/// A person's colour until people have colours of their own, in linear light: a warm coral, which glows against the
/// evening and is far from the agent's violet. Every colour of a person's swarm is made from theirs (`Cues::colour`).
pub const COLOUR: [f32; 3] = [1.0, 0.36, 0.24];
/// How bright a cube at the middle of what it is part of, as seen, is, whitening a little; how bright one at its outline
/// is; and how much smaller.
pub const CORE: f32 = 1.2;
pub const RIM: f32 = 0.5;
const RIM_SIZE: f32 = 0.6;
/// How bright the lowest of the torso is, of its chest.
pub const BELOW: f32 = 0.45;
/// How much each cube's brightness shimmers, either way, and how fast, in radians a second, each at one of these.
pub const SHIMMER: f32 = 0.15;
const SHIMMERS: [f32; 3] = [2.3, 3.4, 5.1];
/// How far towards white the eyes are, and how bright, while cool: soft, a little paler than the face about them.
const EYE_PALE: f32 = 0.55;
const EYE_DIM: f32 = 0.8;
/// The eyes' warm glow while the head faces this participant.
pub const EYE_GLOW: [f32; 3] = [1.0, 0.8, 0.22];
/// How far from where each eye looks out of the head, in degrees, the head's shell leaves a gap for it.
const EYE_GAP: f32 = 15.0;
/// How far about the middle of each eye the cubes around it are, and its halo, in metres.
const EYE_RING: f32 = 0.0125;
const EYE_HALO: f32 = 0.024;
/// How much less the eye away from this participant glows than the one towards them, at most.
const GLOW_SIDE: f32 = 0.15;
/// How far towards white the hands are.
const HAND_PALE: f32 = 0.45;
/// How far towards white the orbit is, and its brightness, and the same while their voice plays.
const ORBIT_PALE: f32 = 0.35;
const ORBIT_DIM: f32 = 0.75;
pub const ORBIT_SPEAKING_PALE: f32 = 0.7;

/// The middle of the head from the eyes, in the head's own coordinates, which face along −Z, and its radii: about 18
/// by 20 by 19 cm, a little larger than a head, so that it reads from across a space.
pub const MIDDLE: Vec3 = Vec3::new(0.0, 0.0, 0.05);
pub const HEAD: Vec3 = Vec3::new(0.092, 0.1, 0.095);
/// Where the neck turns, from the eyes, in the head's coordinates, below and behind them: the body hangs from it.
pub const NECK: Vec3 = Vec3::new(0.0, -0.09, 0.1);
/// The right shoulder from the neck, in the body's coordinates, which turn with the head's yaw alone; the left is its
/// mirror.
pub const SHOULDER: Vec3 = Vec3::new(0.18, -0.16, 0.02);
/// How long the upper arm and the forearm are, to the middle of the hand, and how far both stretch at most to reach a
/// hand farther away.
pub const UPPER: f32 = 0.3;
pub const LOWER: f32 = 0.32;
pub const STRETCH: f32 = 1.2;
/// Where the right hand rests while it is not tracked, from the neck, in the body's coordinates: hanging at the side.
pub const REST: Vec3 = Vec3::new(0.22, -0.66, -0.06);
/// Which way the right elbow bends, in the body's coordinates: out, down and a little back.
pub const POLE: Vec3 = Vec3::new(1.0, -0.7, 0.45);
/// How long the body takes to turn most of the way the head has, in seconds, and how far, in degrees, the head turns
/// from it at most.
pub const FOLLOW: f32 = 0.35;
pub const TWIST: f32 = 50.0;
/// How long a hand takes to come up from rest as it is tracked, and to fall back as it is lost.
const REACH_RISE: f32 = 0.25;
const REACH_FALL: f32 = 0.4;
/// How long the orbit takes to quicken as their voice plays, and to calm again once it stops, so that the breaths
/// between words do not flicker.
pub const SPEAK_RISE: f32 = 0.15;
pub const SPEAK_FALL: f32 = 0.5;
/// How fast the orbit turns, in radians a second, and while their voice plays; and how far from the head's middle.
pub const ORBIT_TURN: f32 = 0.6;
pub const ORBIT_SPEAKING_TURN: f32 = 3.2;
const ORBIT_RADIUS: f32 = 0.19;
/// How the orbit is tilted about the head, its front above the eyes and its back below them, and how far above the
/// head's middle it is.
const ORBIT_TILT: (f32, f32) = (0.3, 0.2);
const ORBIT_ABOVE: f32 = 0.02;
/// Beyond how far from the eyes, in metres, a person is drawn with fewer cubes, and as a head alone, each changing
/// only once they are as much again nearer or farther, so that someone standing at the edge does not flicker.
pub const FEWER: f32 = 5.0;
pub const HEAD_ONLY: f32 = 15.0;
const DETAIL_HOLD: f32 = 0.5;
/// How far the cubes below the chest fall, in metres, each between these, before they begin again; how much of the way
/// down they have grown in from nothing, so that none appears at once; and how far out they spread as they fall, of
/// where they began.
pub const FALL: (f32, f32) = (0.3, 0.6);
const BORN: f32 = 0.12;
const SPREAD: f32 = 0.25;
/// Below the chest, from how far down the torso it is the body begins to dissolve: its cubes fewer, smaller, dimmer
/// and looser lower down.
const DISSOLVE: f32 = 0.3;
/// How fast the cubes drift about where they rest, in radians a second, each at one of these.
const BEATS: [f32; 3] = [1.3, 1.9, 2.7];
/// The torso from the neck down, in the body's coordinates: at each height, how wide it is, and how deep, each a half.
const TORSO: [(f32, f32, f32); 5] = [
    (-0.10, 0.10, 0.07),
    (-0.16, 0.19, 0.08),
    (-0.30, 0.17, 0.095),
    (-0.45, 0.15, 0.09),
    (-0.62, 0.13, 0.08),
];
/// How far behind the neck the torso's middle is.
const TORSO_BACK: f32 = 0.02;

/// How much of a person is drawn, by how far away they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    /// The head alone, its eyes and a little of its orbit.
    Head,
    /// The head's core, two thirds of its shell, its eyes and their halo, two thirds of its orbit, and a quarter of the
    /// rest, each larger.
    Fewer,
    Full,
}

impl Detail {
    /// The detail `distance` metres away, having been drawn at `was`.
    pub fn at(distance: f32, was: Self) -> Self {
        let hold = |kept: bool| if kept { DETAIL_HOLD } else { -DETAIL_HOLD };
        if distance < FEWER + hold(was == Self::Full) {
            Self::Full
        } else if distance < HEAD_ONLY + hold(was >= Self::Fewer) {
            Self::Fewer
        } else {
            Self::Head
        }
    }
}

/// What a cube of the swarm is part of: each of the arms, and the eyes, left then right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Part {
    Core,
    Shell,
    Eye(usize),
    Ring(usize),
    Halo(usize),
    Orbit,
    Neck,
    Torso,
    Drift,
    Upper(usize),
    Lower(usize),
    Hand(usize),
}

impl Part {
    /// How much larger its cubes are drawn at `detail`, to fill in for those left out.
    fn grow(self, detail: Detail) -> f32 {
        match (detail, self) {
            (Detail::Full, _) | (_, Part::Core | Part::Eye(_) | Part::Ring(_) | Part::Halo(_)) => {
                1.0
            }
            (Detail::Fewer, Part::Shell) => 1.15,
            (Detail::Head, Part::Shell) => 1.45,
            (Detail::Head, _) => 1.6,
            (Detail::Fewer, _) => 1.5,
        }
    }
}

/// One cube of the swarm as it rests: in its part's own coordinates, how large, how bright, which way it is turned,
/// which way is out of what it is part of, which way and how far it drifts and when, when it shimmers, at which detail
/// it is drawn first, and how much of the eyes' glow it catches. A cube below the chest falls `drop` once each `fall`
/// seconds, `offset` of the way on, turning as `spin` says.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mote {
    pub part: Part,
    pub at: Vec3,
    pub size: Vec3,
    tone: f32,
    turn: Quat,
    out: Vec3,
    wobble: Vec3,
    phase: [f32; 2],
    beat: usize,
    glint: [f32; 2],
    shimmer: usize,
    pub from: Detail,
    warm: f32,
    fall: f32,
    drop: f32,
    offset: f32,
    spin: Vec3,
}

/// Numbers that look random, the same every time, so that every client draws everyone alike.
struct Dice(u32);

impl Dice {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }

    fn within(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    /// Between `low` and `high`, nearer `low` more often: many small cubes and a few larger.
    fn mostly(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next().powi(2)
    }

    fn direction(&mut self) -> Vec3 {
        let y = self.within(-1.0, 1.0);
        let angle = self.within(0.0, TAU);
        let across = (1.0 - y * y).sqrt();
        Vec3::new(across * angle.cos(), y, across * angle.sin())
    }

    /// A turn of up to `most` radians about any axis.
    fn tilt(&mut self, most: f32) -> Quat {
        let axis = self.direction();
        Quat::from_scaled_axis(axis * self.within(0.0, most))
    }
}

/// The left or right side's sign along the body's X.
fn sign(side: usize) -> f32 {
    [-1.0, 1.0][side]
}

/// `vector` on `side`, the right's mirrored for the left.
fn sided(vector: Vec3, side: usize) -> Vec3 {
    Vec3::new(vector.x * sign(side), vector.y, vector.z)
}

/// Where the eye on `side` looks out of the head, from its middle.
fn eye_direction(side: usize) -> Vec3 {
    Vec3::new(0.42 * sign(side), 0.05, -1.0).normalize()
}

/// How wide and deep the torso is `y` below the neck, as halves.
fn torso(y: f32) -> (f32, f32) {
    let (mut width, mut depth) = (TORSO[0].1, TORSO[0].2);
    for pair in TORSO.windows(2) {
        let ((above, above_width, above_depth), (below, below_width, below_depth)) =
            (pair[0], pair[1]);
        if y <= above {
            let t = ((above - y) / (above - below)).min(1.0);
            width = above_width + (below_width - above_width) * t;
            depth = above_depth + (below_depth - above_depth) * t;
        }
    }
    (width, depth)
}

/// The detail each `index`th of a part's cubes is drawn from, one in `every` from `coarse` and the rest from `fine`.
fn every(index: usize, one_in: usize, coarse: Detail, fine: Detail) -> Detail {
    if index.is_multiple_of(one_in) {
        coarse
    } else {
        fine
    }
}

/// A cube of `part` resting `at`, `size` large, drawn from `from`, out of its part along `out`, drifting as far as
/// `wobble`, how bright, how it is turned, which way it drifts and when, and when it shimmers, as `dice` says.
fn mote(
    dice: &mut Dice,
    part: Part,
    at: Vec3,
    size: Vec3,
    from: Detail,
    out: Vec3,
    wobble: f32,
) -> Mote {
    let phase = dice.within(0.0, TAU);
    let glint = dice.within(0.0, TAU);
    Mote {
        part,
        at,
        size,
        tone: dice.within(0.88, 1.08),
        turn: dice.tilt(0.4),
        out,
        wobble: dice.direction() * wobble,
        phase: [phase.sin(), phase.cos()],
        beat: (dice.next() * BEATS.len() as f32) as usize % BEATS.len(),
        glint: [glint.sin(), glint.cos()],
        shimmer: (dice.next() * SHIMMERS.len() as f32) as usize % SHIMMERS.len(),
        from,
        warm: 0.0,
        fall: 1.0,
        drop: 0.0,
        offset: 0.0,
        spin: Vec3::ZERO,
    }
}

/// How far `down` the torso, from the neck's 0 to the waist's 1, it has dissolved, from nothing above the chest to
/// all of it at the waist.
fn dissolved(down: f32) -> f32 {
    let t = ((down - DISSOLVE) / (1.0 - DISSOLVE)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The swarm's cubes as they rest, the middle of the head's core first.
fn layout() -> Vec<Mote> {
    let mut dice = Dice(0x7f4a_7c15);
    let mut motes = Vec::new();
    // The head's core, a few cubes filling it, its brightest, glowing between the shell's, the first still at its middle.
    let core = 10;
    for index in 0..core {
        let (at, size, wobble) = if index == 0 {
            (Vec3::ZERO, 0.036, 0.0)
        } else {
            (
                dice.direction() * dice.within(0.25, 0.65) * HEAD,
                dice.within(0.022, 0.034),
                0.002,
            )
        };
        let from = every(index, 2, Detail::Head, Detail::Fewer);
        let mote = mote(
            &mut dice,
            Part::Core,
            at,
            Vec3::splat(size),
            from,
            Vec3::ZERO,
            wobble,
        );
        motes.push(if index == 0 {
            Mote {
                turn: Quat::IDENTITY,
                ..mote
            }
        } else {
            mote
        });
    }
    // Its shell, a rounded cluster of cubes of many sizes, with a small gap where each eye is, so that nothing of it
    // covers them.
    let shell = 60;
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    let mut kept = 0;
    for index in 0..shell {
        let y = 1.0 - 2.0 * (index as f32 + 0.5) / shell as f32;
        let across = (1.0 - y * y).sqrt();
        let angle = golden * index as f32;
        let direction = Vec3::new(across * angle.cos(), y, across * angle.sin());
        let nearest = (0..2)
            .map(|side| direction.dot(eye_direction(side)))
            .fold(-1.0f32, f32::max);
        if nearest > EYE_GAP.to_radians().cos() {
            continue;
        }
        let at = direction * HEAD * dice.within(0.88, 1.0);
        let size = Vec3::splat(dice.mostly(0.014, 0.034));
        let from = match kept % 3 {
            0 => Detail::Head,
            1 => Detail::Fewer,
            _ => Detail::Full,
        };
        kept += 1;
        // Those within 15° beyond the gap about an eye catch a little of its glow, the nearer the more.
        let warm = 0.35 * (1.0 - (nearest.clamp(-1.0, 1.0).acos().to_degrees() - EYE_GAP) / 15.0);
        motes.push(Mote {
            warm: warm.clamp(0.0, 0.35),
            ..mote(&mut dice, Part::Shell, at, size, from, direction, 0.003)
        });
    }
    // The eyes, each a small round cluster on its face: a cube turned on its corner, with smaller ones about it, and a
    // halo that shows as they glow.
    let across = |side: usize| {
        let ahead = eye_direction(side);
        let right = Vec3::Y.cross(ahead).normalize();
        (ahead, right, ahead.cross(right))
    };
    for side in 0..2 {
        let (ahead, ..) = across(side);
        motes.push(Mote {
            tone: 1.0,
            turn: Quat::from_rotation_arc(Vec3::NEG_Z, ahead)
                * Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
            ..mote(
                &mut dice,
                Part::Eye(side),
                ahead * HEAD * 1.04,
                Vec3::new(0.018, 0.018, 0.01),
                Detail::Head,
                ahead,
                0.0,
            )
        });
    }
    for (part, count, radius, low, high, from) in [
        (
            Part::Ring as fn(usize) -> Part,
            6,
            EYE_RING,
            0.007,
            0.01,
            Detail::Fewer,
        ),
        (Part::Halo, 5, EYE_HALO, 0.005, 0.007, Detail::Fewer),
    ] {
        for side in 0..2 {
            let (ahead, right, up) = across(side);
            for index in 0..count {
                let angle = (index as f32 + dice.within(-0.2, 0.2)) / count as f32 * TAU;
                let at = ahead * HEAD
                    + (right * angle.cos() + up * angle.sin()) * radius * dice.within(0.85, 1.1);
                let size = Vec3::splat(dice.within(low, high));
                motes.push(mote(&mut dice, part(side), at, size, from, ahead, 0.0015));
            }
        }
    }
    // The orbit, a tilted ring of tiny cubes.
    let orbit = 18;
    for index in 0..orbit {
        let angle = index as f32 / orbit as f32 * TAU + dice.within(-0.12, 0.12);
        let at =
            Vec3::new(angle.cos(), dice.within(-0.06, 0.06), angle.sin()) * dice.within(0.95, 1.05);
        let size = Vec3::splat(dice.within(0.006, 0.011));
        let from = match index % 3 {
            0 => Detail::Head,
            1 => Detail::Fewer,
            _ => Detail::Full,
        };
        motes.push(mote(
            &mut dice,
            Part::Orbit,
            at,
            size,
            from,
            Vec3::ZERO,
            0.004,
        ));
    }
    // The neck, each cube along it, around it and how far out.
    let neck = 5;
    for index in 0..neck {
        let around = dice.within(0.0, TAU);
        let out = dice.within(0.4, 1.0);
        let at = Vec3::new(
            around.cos() * out,
            (index as f32 + 0.5) / neck as f32,
            around.sin() * out,
        );
        let from = every(index, 2, Detail::Fewer, Detail::Full);
        let size = Vec3::splat(dice.mostly(0.012, 0.024));
        let direction = Vec3::new(around.cos(), 0.0, around.sin());
        motes.push(mote(
            &mut dice,
            Part::Neck,
            at,
            size,
            from,
            direction,
            0.003,
        ));
    }
    // The torso, through its depth rather than on its surface, densest at the shoulders, dissolving below the chest into
    // fewer, smaller, dimmer and looser cubes.
    let torso_cubes = 106;
    let (top, bottom) = (TORSO[0].0, TORSO[TORSO.len() - 1].0);
    for index in 0..torso_cubes {
        let down = dice.next().powf(2.5);
        let y = top + (bottom - top) * down;
        let (width, depth) = torso(y);
        let around = dice.within(0.0, TAU);
        let loose = dissolved(down);
        let out = dice.within(0.7, 1.0 + 0.15 * loose);
        let at = Vec3::new(
            width * around.cos() * out,
            y,
            depth * around.sin() * out + TORSO_BACK,
        );
        let size = Vec3::splat(dice.mostly(0.016, 0.036) * (1.0 - 0.5 * loose));
        let from = every(index, 4, Detail::Fewer, Detail::Full);
        let direction = Vec3::new(around.cos(), 0.0, around.sin());
        let mote = mote(
            &mut dice,
            Part::Torso,
            at,
            size,
            from,
            direction,
            0.004 + 0.016 * loose,
        );
        motes.push(Mote {
            tone: mote.tone * (1.0 - (1.0 - BELOW) * loose),
            ..mote
        });
    }
    // Below the chest, cubes falling from it, more of them lower, each shrinking and fading as it falls, turning, and
    // beginning again.
    let drift = 28;
    for index in 0..drift {
        let down = DISSOLVE + (1.0 - DISSOLVE) * dice.next().sqrt();
        let y = top + (bottom - top) * down;
        let (width, depth) = torso(y);
        let around = dice.within(0.0, TAU);
        let out = dice.within(0.2, 1.05);
        let at = Vec3::new(
            width * around.cos() * out,
            y,
            depth * around.sin() * out + TORSO_BACK,
        );
        let size = Vec3::splat(dice.mostly(0.008, 0.02));
        let from = every(index, 4, Detail::Fewer, Detail::Full);
        let mote = mote(&mut dice, Part::Drift, at, size, from, Vec3::ZERO, 0.015);
        motes.push(Mote {
            tone: mote.tone * (1.0 - (1.0 - BELOW) * dissolved(down)),
            fall: dice.within(3.0, 6.0),
            drop: dice.within(FALL.0, FALL.1),
            offset: dice.next(),
            spin: dice.direction() * dice.within(0.5, 1.5),
            ..mote
        });
    }
    // The arms, each cube along its bone, around it and how far out; and the hands' clusters.
    for side in 0..2 {
        for (part, count, low, high) in [
            (Part::Upper(side), 12, 0.015, 0.03),
            (Part::Lower(side), 11, 0.013, 0.027),
        ] {
            for index in 0..count {
                let around = dice.within(0.0, TAU);
                let out = dice.within(0.55, 1.0);
                let at = Vec3::new(
                    around.cos() * out,
                    (index as f32 + dice.next()) / count as f32,
                    around.sin() * out,
                );
                let size = Vec3::splat(dice.mostly(low, high));
                let from = every(index, 4, Detail::Fewer, Detail::Full);
                let direction = Vec3::new(around.cos(), 0.0, around.sin());
                motes.push(mote(&mut dice, part, at, size, from, direction, 0.004));
            }
        }
        for index in 0..8 {
            let at = dice.direction() * dice.within(0.2, 1.0) * Vec3::new(0.04, 0.045, 0.035);
            let size = Vec3::splat(dice.mostly(0.01, 0.02));
            let from = every(index, 4, Detail::Fewer, Detail::Full);
            motes.push(mote(
                &mut dice,
                Part::Hand(side),
                at,
                size,
                from,
                Vec3::ZERO,
                0.003,
            ));
        }
    }
    motes
}

/// The swarm's cubes as they rest, made once.
pub(crate) static MOTES: LazyLock<Vec<Mote>> = LazyLock::new(layout);

/// How many cubes a person's swarm has at most, drawn in full.
pub const MOST: usize = 310;

/// How many of the swarm's cubes are drawn at `detail`.
pub fn cubes(detail: Detail) -> usize {
    MOTES.iter().filter(|mote| mote.from <= detail).count()
}

/// How a head turns about the vertical, from its rotation's twist about Y, within half a turn either way: nodding and
/// tilting it leave it be.
pub fn yaw(orientation: Quat) -> f32 {
    wrapped(2.0 * orientation.y.atan2(orientation.w))
}

/// `angle` wrapped to within half a turn either way.
fn wrapped(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI
}

/// Where the hand on `side` is in `present`: a tracked hand's palm, or else its wrist, or else its controller's grip.
fn hand(present: &Present, side: usize) -> Option<Vec3> {
    present.joints[side]
        .and_then(|joints| joints[0].or(joints[1]))
        .map(|joint| joint.position)
        .or_else(|| present.controllers[side].map(|controller| controller.position))
}

/// An arm as drawn: where its hand was last tracked, in the body's coordinates from the neck, and how far it has come
/// up from rest towards it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Arm {
    reach: f32,
    hand: Vec3,
}

/// Someone's body as drawn, kept from frame to frame: which way it faces about the vertical, its arms, how much of it is
/// drawn, how much their voice plays and how far their orbit has turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub yaw: f32,
    arms: [Arm; 2],
    pub detail: Detail,
    pub speaking: f32,
    pub orbit: f32,
}

/// Where the neck is, of the head `present` has.
fn neck(present: &Present) -> Vec3 {
    present.head.position + present.head.orientation * NECK
}

impl Body {
    /// Someone first drawn as `present`, facing as their head does, their hands up where they are, seen from `eyes`.
    pub fn new(present: &Present, eyes: Option<Vec3>) -> Self {
        let mut body = Self {
            yaw: yaw(present.head.orientation),
            arms: [0, 1].map(|side| Arm {
                reach: 0.0,
                hand: sided(REST, side),
            }),
            detail: Detail::Full,
            speaking: 0.0,
            orbit: 0.0,
        };
        body.follow(present, 0.0, eyes, false);
        for (side, arm) in body.arms.iter_mut().enumerate() {
            arm.reach = if hand(present, side).is_some() {
                1.0
            } else {
                0.0
            };
        }
        body
    }

    /// Follow `present`, `dt` seconds on, seen from `eyes`, their voice playing or not: the body turning after the
    /// head's yaw, never more than [`TWIST`] from it, each hand up as it is tracked and down to rest as it is lost, as
    /// much drawn as their distance says, and the orbit turning, quicker while their voice plays.
    pub fn follow(&mut self, present: &Present, dt: f32, eyes: Option<Vec3>, speaking: bool) {
        let head = yaw(present.head.orientation);
        let turned = wrapped(head - self.yaw) * (1.0 - (-dt / FOLLOW).exp());
        let twist = TWIST.to_radians();
        let behind = wrapped(head - self.yaw - turned).clamp(-twist, twist);
        self.yaw = wrapped(head - behind);
        let facing = Quat::from_rotation_y(self.yaw);
        let neck = neck(present);
        for (side, arm) in self.arms.iter_mut().enumerate() {
            match hand(present, side) {
                Some(hand) => {
                    arm.hand = facing.inverse() * (hand - neck);
                    arm.reach = (arm.reach + dt / REACH_RISE).min(1.0);
                }
                None => arm.reach = (arm.reach - dt / REACH_FALL).max(0.0),
            }
        }
        if let Some(eyes) = eyes {
            self.detail = Detail::at(eyes.distance(present.head.position), self.detail);
        }
        self.speaking = if speaking {
            (self.speaking + dt / SPEAK_RISE).min(1.0)
        } else {
            (self.speaking - dt / SPEAK_FALL).max(0.0)
        };
        let turn = ORBIT_TURN + (ORBIT_SPEAKING_TURN - ORBIT_TURN) * self.speaking;
        self.orbit = (self.orbit + turn * dt).rem_euclid(TAU);
    }

    /// The arm on `side` of someone drawn as `present`, facing `facing`: its shoulder, elbow and where its hand is.
    pub fn arm(&self, present: &Present, facing: Quat, side: usize) -> [Vec3; 3] {
        let neck = neck(present);
        let arm = &self.arms[side];
        let up = arm.reach * arm.reach * (3.0 - 2.0 * arm.reach);
        let shoulder = neck + facing * sided(SHOULDER, side);
        let hand = neck + facing * sided(REST, side).lerp(arm.hand, up);
        let pole = facing * sided(POLE, side);
        let (elbow, hand) = elbow(shoulder, hand, pole, UPPER, LOWER);
        [shoulder, elbow, hand]
    }
}

/// The elbow of an arm from `shoulder` reaching for `hand`, its upper arm `upper` long and its forearm `lower`, bending
/// towards `pole`, and where its hand reaches: `hand` itself, but beyond the arm's reach stretched by [`STRETCH`], or
/// closer to the shoulder than it folds, where it reaches as far as it can along the way.
pub fn elbow(shoulder: Vec3, hand: Vec3, pole: Vec3, upper: f32, lower: f32) -> (Vec3, Vec3) {
    let to = hand - shoulder;
    let distance = to.length();
    // A hand at the shoulder folds the arm along its pole.
    let along = to
        .try_normalize()
        .or_else(|| (-pole).try_normalize())
        .unwrap_or(Vec3::NEG_Y);
    let stretch = (distance / (upper + lower)).clamp(1.0, STRETCH);
    let (upper, lower) = (upper * stretch, lower * stretch);
    let reach = distance.clamp((upper - lower).abs() + 1e-4, upper + lower);
    let out = (upper * upper - lower * lower + reach * reach) / (2.0 * reach);
    let bent = (upper * upper - out * out).max(0.0).sqrt();
    // Along the pole, the arm bends any way across it.
    let side = Some(pole - along * pole.dot(along))
        .filter(|side| side.length_squared() > 1e-6 * pole.length_squared())
        .map_or_else(|| along.any_orthonormal_vector(), Vec3::normalize);
    (
        shoulder + along * out + side * bent,
        shoulder + along * reach,
    )
}

/// What a person's swarm shows besides their presence and body: their colour, how far it is drawn in, how much the eyes
/// glow and where this participant's eyes are, from which each cube is seen and towards which the nearer eye glows a
/// little more, and the time, in seconds, its cubes drift, fall and shimmer by.
#[derive(Clone, Copy, Debug)]
pub struct Cues {
    pub colour: [f32; 3],
    pub fade: f32,
    pub glow: f32,
    pub eyes: Option<Vec3>,
    pub time: f32,
}

/// `color` as bright as `bright`, no brighter than white in any channel, so that the brightest whiten a little.
fn toned(color: [f32; 3], bright: f32) -> [f32; 3] {
    color.map(|value| (value * bright).min(1.0))
}

fn mix(from: [f32; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    Vec3::from_array(from)
        .lerp(Vec3::from_array(to), t)
        .to_array()
}

/// `colour` `t` of the way to white.
fn pale(colour: [f32; 3], t: f32) -> [f32; 3] {
    mix(colour, [1.0; 3], t)
}

/// The eyes' colour while cool, of someone whose colour is `colour`: soft, paler than their face, and no brighter.
pub fn eyes(colour: [f32; 3]) -> [f32; 3] {
    toned(pale(colour, EYE_PALE), EYE_DIM)
}

/// How bright, and how large, a cube is whose part's outside faces `facing` towards the eyes, as a cosine: [`CORE`] at
/// the middle of what it is part of, as seen, and [`RIM`] at its outline and behind it, [`RIM_SIZE`] as large.
fn seen(facing: f32) -> (f32, f32) {
    let towards = facing.max(0.0);
    (
        RIM + (CORE - RIM) * towards,
        RIM_SIZE + (1.0 - RIM_SIZE) * towards,
    )
}

/// A limb from `start` to `end`, as a frame whose −Y runs along it, turned as little as it can be from `rest`, and how
/// long it is.
fn limb(start: Vec3, end: Vec3, rest: Quat) -> (Vec3, Mat3, Quat, f32) {
    let to = end - start;
    let length = to.length();
    let along = to.try_normalize().unwrap_or(rest * Vec3::NEG_Y);
    let turn = Quat::from_rotation_arc(rest * Vec3::NEG_Y, along) * rest;
    (start, Mat3::from_quat(turn), turn, length)
}

/// Draw the swarm of someone as `present`, with `body`, as `cues` say, onto `cubes`: the middle of the head's core
/// first, about the head's middle, then the rest as much as its detail says, each cube shrunk where it rests as it
/// fades.
pub fn look(cubes: &mut Vec<Cube>, present: &Present, body: &Body, cues: &Cues) {
    let head = present.head;
    let turn = head.orientation;
    let turned = Mat3::from_quat(turn);
    let middle = head.position + turned * MIDDLE;
    let facing = Quat::from_rotation_y(body.yaw);
    let faced = Mat3::from_quat(facing);
    let neck = neck(present);
    // Which way this participant's eyes are, from the head and from the body, or ahead of the head without them.
    let ahead = turn * Vec3::NEG_Z;
    let seen_from = |from: Vec3| {
        cues.eyes
            .and_then(|eyes| (eyes - from).try_normalize())
            .unwrap_or(ahead)
    };
    let (to_head, to_body) = (seen_from(middle), seen_from(neck));
    // Which eye is towards this participant, the left's −1 and the right's 1, or between.
    let towards = cues
        .eyes
        .and_then(|eyes| (eyes - head.position).try_normalize())
        .map_or(0.0, |towards| {
            ((turn * Vec3::X).dot(towards) / super::GAZE_LEFT.to_radians().sin()).clamp(-1.0, 1.0)
        });
    let arms = [0, 1].map(|side| {
        let [shoulder, elbow, hand] = body.arm(present, facing, side);
        let rest =
            facing * Quat::from_rotation_arc(Vec3::NEG_Y, sided(REST - SHOULDER, side).normalize());
        let upper = limb(shoulder, elbow, rest);
        let lower = limb(elbow, hand, upper.2);
        (upper, lower, hand)
    });
    let throat = limb(
        middle + turned * Vec3::new(0.0, -0.075, 0.0),
        neck + faced * Vec3::new(0.0, -0.1, TORSO_BACK),
        facing,
    );
    let wave = |beat: f32| {
        let (sin, cos) = (beat * cues.time).sin_cos();
        [sin, cos]
    };
    let (waves, glints) = (BEATS.map(wave), SHIMMERS.map(wave));
    let (orbit_sin, orbit_cos) = body.orbit.sin_cos();
    let ring = turned * Mat3::from_euler(glam::EulerRot::XYZ, ORBIT_TILT.0, 0.0, ORBIT_TILT.1);
    let orbited = middle + turned * Vec3::new(0.0, ORBIT_ABOVE, 0.0);
    let colour = cues.colour;
    let cool = eyes(colour);
    let hands = pale(colour, HAND_PALE);
    let orbit = mix(
        toned(pale(colour, ORBIT_PALE), ORBIT_DIM),
        pale(colour, ORBIT_SPEAKING_PALE),
        body.speaking,
    );
    let detail = body.detail;
    for mote in MOTES.iter().filter(|mote| mote.from <= detail) {
        let wave = waves[mote.beat];
        let swing = wave[0] * mote.phase[1] + wave[1] * mote.phase[0];
        let wobble = mote.wobble * swing;
        let glint = glints[mote.shimmer];
        let bright =
            mote.tone * (1.0 + SHIMMER * (glint[0] * mote.glint[1] + glint[1] * mote.glint[0]));
        let grow = mote.part.grow(detail) * cues.fade;
        let mut size = mote.size * grow;
        // How bright and large a cube of the body is, as it is seen, its outside along `out` in the scene.
        let mut lit = |out: Vec3, towards: Vec3| {
            let (light, large) = seen(out.dot(towards));
            size *= large;
            toned(colour, bright * light)
        };
        let (position, orientation, color) = match mote.part {
            Part::Core => (
                middle + turned * (mote.at + wobble),
                turn * mote.turn,
                toned(colour, CORE * bright),
            ),
            Part::Shell => {
                let lit = lit(turned * mote.out, to_head);
                (
                    middle + turned * (mote.at + wobble),
                    turn * mote.turn,
                    mix(lit, EYE_GLOW, cues.glow * mote.warm),
                )
            }
            Part::Eye(side) | Part::Ring(side) => {
                let glow = cues.glow * (1.0 - GLOW_SIDE * (1.0 - sign(side) * towards) / 2.0);
                let (grown, dimmer) = match mote.part {
                    Part::Eye(_) => (0.3, 1.0),
                    _ => (0.25, 0.85),
                };
                size *= 1.0 + grown * cues.glow;
                // Steady, so that the cue they give never flickers.
                (
                    middle + turned * (mote.at * (1.0 + 0.15 * cues.glow) + wobble),
                    turn * mote.turn,
                    toned(mix(cool, EYE_GLOW, glow), dimmer),
                )
            }
            // Nothing while the eyes are cool, which the renderer leaves out.
            Part::Halo(_) => {
                size *= cues.glow;
                (
                    middle + turned * (mote.at + wobble),
                    turn * mote.turn,
                    toned(EYE_GLOW, 0.7 * bright),
                )
            }
            Part::Orbit => {
                size *= 1.0 + 0.35 * body.speaking;
                let at = Vec3::new(
                    mote.at.x * orbit_cos - mote.at.z * orbit_sin,
                    mote.at.y,
                    mote.at.x * orbit_sin + mote.at.z * orbit_cos,
                );
                (
                    orbited + ring * (at * ORBIT_RADIUS) + turned * wobble,
                    turn * mote.turn,
                    toned(orbit, bright),
                )
            }
            Part::Neck => {
                let (start, frame, turn, length) = throat;
                let at = Vec3::new(mote.at.x * 0.035, -mote.at.y * length, mote.at.z * 0.035);
                let lit = lit(frame * mote.out, to_body);
                (start + frame * (at + wobble), turn * mote.turn, lit)
            }
            Part::Torso => {
                let lit = lit(faced * mote.out, to_body);
                (neck + faced * (mote.at + wobble), facing * mote.turn, lit)
            }
            Part::Drift => {
                let fallen = (cues.time / mote.fall + mote.offset).fract();
                size *= (fallen / BORN).min(1.0) * (1.0 - 0.7 * fallen);
                let spread = Vec3::new(mote.at.x, 0.0, mote.at.z - TORSO_BACK) * SPREAD * fallen;
                let at = mote.at + spread + Vec3::new(0.0, -mote.drop * fallen, 0.0) + wobble;
                (
                    neck + faced * at,
                    facing * Quat::from_scaled_axis(mote.spin * cues.time),
                    toned(colour, bright * (1.0 - 0.6 * fallen)),
                )
            }
            Part::Upper(side) | Part::Lower(side) => {
                let ((start, frame, turn, length), radius) = match mote.part {
                    Part::Upper(_) => (arms[side].0, 0.04 - 0.006 * mote.at.y),
                    _ => (arms[side].1, 0.033 - 0.006 * mote.at.y),
                };
                let at = Vec3::new(mote.at.x * radius, -mote.at.y * length, mote.at.z * radius);
                let lit = lit(frame * mote.out, to_body);
                (start + frame * (at + wobble), turn * mote.turn, lit)
            }
            Part::Hand(side) => {
                let (_, frame, turn, _) = arms[side].1;
                (
                    arms[side].2 + frame * (mote.at + wobble),
                    turn * mote.turn,
                    toned(hands, bright),
                )
            }
        };
        cubes.push(Cube {
            position: position.to_array(),
            scale: size.to_array(),
            color,
            orientation: orientation.to_array(),
        });
    }
}

#[cfg(test)]
mod tests;
