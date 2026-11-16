// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Someone's hands and controllers as this participant sees them, in their colour, at the end of each arm of their
//! swarm: a tracked hand as its joints and bones, small cubes on the joints and a slender rod along each bone, a little
//! brighter and paler than the rest of them, so that pointing, a thumb up or a wave reads; a controller that no hand
//! tracks as a small stylised one at its grip, a rounded body with a ring of rods above it, never the runtime's model
//! of this participant's own hardware. Where both are tracked, as with simultaneous hands and controllers, the hand is
//! drawn. Switching between the two cross-fades, and a hand that loses tracking dissolves where it was last drawn rather
//! than freezing. Farther away a tracked hand is one cluster at its palm, and farther still neither is drawn. See
//! `design/people.md`.
use glam::{Mat3, Quat, Vec3};

use super::swarm::Detail;
use super::{Joints, Present};
use crate::client_side::Hand;
use crate::delegation::Cube;

/// How long switching between a controller and a tracked hand takes, each fading out as the other fades in, in
/// seconds; and how long a hand that loses tracking, with no controller to take over, takes to dissolve.
pub const CROSS: f32 = 0.2;
pub const LOST: f32 = 0.15;

/// The palm's joint and the wrist's, in OpenXR's order, and the knuckles the hand's frame is found from: the middle
/// finger's, and the index's and the little finger's either side.
const PALM: usize = 0;
const WRIST: usize = 1;
const MIDDLE: usize = 12;
const INDEX: usize = 7;
const LITTLE: usize = 22;
/// The fingertips, drawn a little smaller and brighter.
const TIPS: [usize; 5] = [5, 10, 15, 20, 25];

/// How many cubes a tracked hand is drawn with in full: one on each joint but the palm's, a rod along each bone, the
/// palm a flat block, and its gleam behind it as seen.
pub const HAND: usize = tomori_protocol::JOINT_COUNT - 1 + tomori_protocol::BONES.len() + 2;
/// How many a tracked hand is drawn with beyond [`super::swarm::FEWER`]: one cluster at its palm.
pub const CLUSTER: usize = CLUSTERED.len();
/// How many a controller is drawn with in full: its rounded body of three boxes, its face, a button and its ring.
pub const CONTROLLER: usize = BODY.len() + 2 + RING;
/// How many cubes someone's hands and controllers are drawn with at most, both cross-fading at once on each side.
pub const MOST: usize = 2 * (HAND + CONTROLLER);

/// How far towards white a hand's joints are, and how bright, a little brighter than the swarm about them; its bones,
/// and its palm.
const JOINT_PALE: f32 = 0.5;
const JOINT_BRIGHT: f32 = 1.15;
const BONE_PALE: f32 = 0.3;
const BONE_BRIGHT: f32 = 1.05;
const PALM_PALE: f32 = 0.3;
const PALM_BRIGHT: f32 = 0.95;
/// How bright the palm's gleam is, of the person's colour, how large, of the palm, and how far behind it, as seen.
const GLEAM: f32 = 0.55;
const GLEAM_SIZE: f32 = 1.5;
const GLEAM_BEHIND: f32 = 0.025;
/// How large a joint's cube is, of its radius, and within what, in metres; a fingertip's, of a joint's.
const JOINT_SIZE: f32 = 1.7;
const JOINT_SIZES: (f32, f32) = (0.008, 0.024);
const TIP_SIZE: f32 = 0.8;
/// How thick a bone's rod is, of its joints' lesser radius, and how much of the bone it runs along, leaving a little
/// air at each joint, as the swarm's rods do.
const BONE_THICK: f32 = 1.1;
const BONE_ALONG: f32 = 0.78;
/// How thick the palm's block is, in metres, and how much of the knuckles' width and the wrist's length it is.
const PALM_THICK: f32 = 0.022;
const PALM_WIDE: f32 = 1.05;
const PALM_LONG: f32 = 0.85;
/// The palm's width and length where its knuckles are not seen.
const PALM_REST: (f32, f32) = (0.075, 0.08);

/// The cluster a far hand is drawn as, in the hand's frame (X across the knuckles, Y out of the back of the hand, −Z
/// towards the fingers) about the palm: where each cube is and how large.
const CLUSTERED: [(Vec3, Vec3); 5] = [
    (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.07, 0.03, 0.075)),
    (Vec3::new(0.025, 0.0, -0.055), Vec3::new(0.03, 0.025, 0.05)),
    (Vec3::new(-0.02, 0.0, -0.06), Vec3::new(0.03, 0.025, 0.055)),
    (Vec3::new(0.045, 0.0, 0.0), Vec3::new(0.025, 0.025, 0.045)),
    (Vec3::new(0.0, 0.01, 0.045), Vec3::new(0.05, 0.03, 0.03)),
];

/// A controller's rounded body, in its grip's frame (−Z along the tube the fingers close round, from the little finger
/// to the thumb, +Y above it): three boxes about one middle, each a little longer or wider than the others, so that
/// together their corners are cut, as a few pixels draw a round.
const BODY: [(Vec3, Vec3); 3] = [
    (Vec3::new(0.0, 0.0, 0.005), Vec3::new(0.034, 0.036, 0.1)),
    (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.026, 0.044, 0.092)),
    (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.04, 0.03, 0.088)),
];
/// Its face, above the thumb, a little wider than the body, and a button on it.
const FACE: (Vec3, Vec3) = (
    Vec3::new(0.0, 0.006, -0.058),
    Vec3::new(0.046, 0.044, 0.026),
);
const BUTTON: (Vec3, Vec3) = (Vec3::new(0.0, 0.026, -0.06), Vec3::new(0.012, 0.01, 0.012));
/// Its tracking ring, of rods round a circle above its face: how many, how far round, where its middle is, how far it
/// tilts back about X, and how thick each rod is.
const RING: usize = 8;
const RING_RADIUS: f32 = 0.048;
const RING_MIDDLE: Vec3 = Vec3::new(0.0, 0.03, -0.07);
const RING_TILT: f32 = 0.5;
const RING_THICK: f32 = 0.009;
/// How bright the body is, of the person's colour; how far towards white its face, button and ring are, and how bright.
const BODY_BRIGHT: f32 = 0.85;
const FACE_PALE: f32 = 0.2;
const BUTTON_PALE: f32 = 0.45;
const RING_PALE: f32 = 0.25;
const RING_BRIGHT: f32 = 1.1;

/// One side's hand and controller as drawn: how much of each is shown, and where each was last tracked, so that one
/// that is lost dissolves where it was.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Side {
    hand: f32,
    controller: f32,
    joints: Option<Joints>,
    grip: Option<Hand>,
}

/// Someone's hands and controllers as drawn, kept from frame to frame, left then right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hands {
    sides: [Side; 2],
}

/// `joints`, if a hand can be drawn of them: at least one bone of it seen.
fn drawable(joints: &Joints) -> bool {
    tomori_protocol::BONES
        .iter()
        .any(|&(start, end)| joints[start].is_some() && joints[end].is_some())
}

/// What `present` tracks on `side`: its hand, where one is drawable, or else its controller.
fn tracked(present: &Present, side: usize) -> (Option<Joints>, Option<Hand>) {
    let joints = present.joints[side].filter(drawable);
    (joints, present.controllers[side])
}

impl Hands {
    /// Someone first drawn as `present`, each hand and controller shown at once as it is tracked.
    pub fn new(present: &Present) -> Self {
        let sides = [0, 1].map(|side| {
            let (joints, grip) = tracked(present, side);
            Side {
                hand: if joints.is_some() { 1.0 } else { 0.0 },
                controller: if joints.is_none() && grip.is_some() {
                    1.0
                } else {
                    0.0
                },
                joints,
                grip,
            }
        });
        Self { sides }
    }

    /// Follow `present`, `dt` seconds on: a tracked hand shows, over [`CROSS`], and its controller, if any, fades as it
    /// does, since the hand is drawn where both are tracked; a controller alone shows, over [`CROSS`]; a hand lost
    /// dissolves where it was last drawn, over [`CROSS`] as its controller takes over, or [`LOST`] without one.
    pub fn follow(&mut self, present: &Present, dt: f32) {
        for (index, side) in self.sides.iter_mut().enumerate() {
            let (joints, grip) = tracked(present, index);
            if joints.is_some() {
                side.joints = joints;
            }
            if grip.is_some() {
                side.grip = grip;
            }
            let (hand, controller) = (joints.is_some(), joints.is_none() && grip.is_some());
            side.hand = if hand {
                (side.hand + dt / CROSS).min(1.0)
            } else {
                let fall = if controller { CROSS } else { LOST };
                (side.hand - dt / fall).max(0.0)
            };
            side.controller = if controller {
                (side.controller + dt / CROSS).min(1.0)
            } else {
                (side.controller - dt / CROSS).max(0.0)
            };
            if side.hand <= 0.0 {
                side.joints = None;
            }
            if side.controller <= 0.0 && grip.is_none() {
                side.grip = None;
            }
        }
    }

    /// How much the tracked hand on `side` is shown, and its controller, each from nothing to whole.
    pub fn shown(&self, side: usize) -> (f32, f32) {
        (self.sides[side].hand, self.sides[side].controller)
    }

    /// How much of the swarm's own cluster at the end of each arm is drawn, left then right: whatever the hand and the
    /// controller drawn there leave, so that a hand neither tracks, hanging at the side, is the swarm's own.
    pub fn clusters(&self) -> [f32; 2] {
        self.sides
            .map(|side| 1.0 - side.hand.max(side.controller).clamp(0.0, 1.0))
    }
}

/// `colour` `t` of the way to white, as bright as `bright`, no brighter than white in any channel.
fn tinted(colour: [f32; 3], t: f32, bright: f32) -> [f32; 3] {
    colour.map(|value| ((value + (1.0 - value) * t) * bright).min(1.0))
}

/// A cube at `position`, `scale` large, turned `orientation`, `color`.
fn cube(position: Vec3, scale: Vec3, orientation: Quat, color: [f32; 3]) -> Cube {
    Cube {
        position: position.to_array(),
        scale: scale.to_array(),
        color,
        orientation: orientation.to_array(),
    }
}

/// The frame of a hand of `joints` on `side`: as a hand held out flat, palm down and fingers ahead, X to its right
/// across its knuckles, Y out of its back and −Z from the wrist towards its fingers, as a rotation, and where its palm
/// is; turned as such a hand wherever the joints it is found from are not seen.
fn frame(joints: &Joints, side: usize) -> (Quat, Vec3) {
    let at = |index: usize| joints[index].map(|joint| joint.position);
    let palm = at(PALM)
        .or_else(|| at(WRIST))
        .or_else(|| joints.iter().flatten().next().map(|joint| joint.position))
        .unwrap_or(Vec3::ZERO);
    let ahead = at(WRIST)
        .zip(at(MIDDLE).or(at(MIDDLE - 1)))
        .and_then(|(wrist, knuckle)| (knuckle - wrist).try_normalize());
    let across = at(LITTLE)
        .zip(at(INDEX))
        .and_then(|(little, index)| (index - little).try_normalize());
    let turn = match (ahead, across) {
        (Some(ahead), Some(across)) => {
            // Palm down, the index is left of the little finger on the right hand and right of it on the left, so
            // that across is the frame's −X on the right hand and its +X on the left.
            let x = if side == 1 { -across } else { across };
            let z = -ahead;
            let y = z.cross(x).try_normalize();
            y.map(|y| {
                let x = y.cross(z).normalize();
                Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize()
            })
        }
        (Some(ahead), None) => Some(Quat::from_rotation_arc(Vec3::NEG_Z, ahead)),
        _ => None,
    };
    (turn.unwrap_or(Quat::IDENTITY), palm)
}

/// Draw `present`'s hands and controllers onto `cubes`, as `hands` shows them, at `detail`, in `colour`, `fade` of the
/// way in, seen from `eyes`: in full a tracked hand's joints and bones, and a controller's body and ring; beyond
/// [`super::swarm::FEWER`] a tracked hand as one cluster at its palm and a controller as its body alone; and as a
/// head alone, neither.
pub fn draw(
    cubes: &mut Vec<Cube>,
    hands: &Hands,
    detail: Detail,
    colour: [f32; 3],
    fade: f32,
    eyes: Option<Vec3>,
) {
    if detail == Detail::Head {
        return;
    }
    for (index, side) in hands.sides.iter().enumerate() {
        if let Some(joints) = side.joints.as_ref().filter(|_| side.hand > 0.0) {
            hand(cubes, joints, index, detail, colour, side.hand * fade, eyes);
        }
        if let Some(grip) = side.grip.filter(|_| side.controller > 0.0) {
            controller(cubes, &grip, detail, colour, side.controller * fade);
        }
    }
}

/// Draw a tracked hand of `joints` on `side`, `shown` of the way in, each cube shrinking where it rests as it fades.
fn hand(
    cubes: &mut Vec<Cube>,
    joints: &Joints,
    side: usize,
    detail: Detail,
    colour: [f32; 3],
    shown: f32,
    eyes: Option<Vec3>,
) {
    let (turn, palm) = frame(joints, side);
    let turned = Mat3::from_quat(turn);
    let palm_colour = tinted(colour, PALM_PALE, PALM_BRIGHT);
    if detail < Detail::Full {
        for (at, size) in CLUSTERED {
            cubes.push(cube(palm + turned * at, size * shown, turn, palm_colour));
        }
        return;
    }
    let joint_colour = tinted(colour, JOINT_PALE, JOINT_BRIGHT);
    let tip_colour = tinted(colour, JOINT_PALE + 0.15, JOINT_BRIGHT);
    let bone_colour = tinted(colour, BONE_PALE, BONE_BRIGHT);
    // The palm, a flat block as wide as the knuckles and most of the way from the wrist to them, and its gleam behind it
    // as seen, standing in for a bloom.
    let at = |index: usize| joints[index].map(|joint| joint.position);
    let wide = at(INDEX)
        .zip(at(LITTLE))
        .map_or(PALM_REST.0, |(index, little)| {
            index.distance(little) * PALM_WIDE
        });
    let long = at(WRIST)
        .zip(at(MIDDLE).or(at(MIDDLE - 1)))
        .map_or(PALM_REST.1, |(wrist, knuckle)| {
            wrist.distance(knuckle) * PALM_LONG
        });
    let block = Vec3::new(wide, PALM_THICK, long);
    cubes.push(cube(palm, block * shown, turn, palm_colour));
    let away = eyes
        .and_then(|eyes| (palm - eyes).try_normalize())
        .unwrap_or(turned * Vec3::NEG_Y);
    cubes.push(cube(
        palm + away * GLEAM_BEHIND,
        block * GLEAM_SIZE * shown,
        turn,
        tinted(colour, 0.0, GLEAM),
    ));
    // A cube on each joint seen, turned with the hand, a fingertip's a little smaller and paler.
    for (index, joint) in joints.iter().enumerate().skip(WRIST) {
        let Some(joint) = joint else { continue };
        let tip = TIPS.contains(&index);
        let size = (joint.radius * JOINT_SIZE).clamp(JOINT_SIZES.0, JOINT_SIZES.1)
            * if tip { TIP_SIZE } else { 1.0 };
        let color = if tip { tip_colour } else { joint_colour };
        cubes.push(cube(joint.position, Vec3::splat(size * shown), turn, color));
    }
    // A slender rod along each bone both of whose joints are seen, most of the way between them.
    for (start, end) in tomori_protocol::BONES {
        let (Some(start), Some(end)) = (joints[start], joints[end]) else {
            continue;
        };
        let to = end.position - start.position;
        let Some(along) = to.try_normalize() else {
            continue;
        };
        let thick = start.radius.min(end.radius) * BONE_THICK;
        let turn = Quat::from_rotation_arc(turned * Vec3::NEG_Z, along) * turn;
        cubes.push(cube(
            (start.position + end.position) * 0.5,
            Vec3::new(thick, thick, to.length() * BONE_ALONG) * shown,
            turn,
            bone_colour,
        ));
    }
}

/// Draw a controller at `grip`, `shown` of the way in: in full its rounded body, its face and button and its ring, and
/// farther away its body alone.
fn controller(cubes: &mut Vec<Cube>, grip: &Hand, detail: Detail, colour: [f32; 3], shown: f32) {
    let turn = grip.orientation;
    let turned = Mat3::from_quat(turn);
    let place = |at: Vec3| grip.position + turned * at;
    let body = tinted(colour, 0.0, BODY_BRIGHT);
    for (at, size) in BODY {
        cubes.push(cube(place(at), size * shown, turn, body));
    }
    if detail < Detail::Full {
        return;
    }
    cubes.push(cube(
        place(FACE.0),
        FACE.1 * shown,
        turn,
        tinted(colour, FACE_PALE, 1.0),
    ));
    cubes.push(cube(
        place(BUTTON.0),
        BUTTON.1 * shown,
        turn,
        tinted(colour, BUTTON_PALE, 1.0),
    ));
    // The ring, its rods along a circle tilted back about X, each as long as most of its share of the way round.
    let tilt = Quat::from_rotation_x(RING_TILT);
    let ring = tinted(colour, RING_PALE, RING_BRIGHT);
    let length = std::f32::consts::TAU * RING_RADIUS / RING as f32 * 0.8;
    for index in 0..RING {
        let angle = (index as f32 + 0.5) / RING as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let at = RING_MIDDLE + tilt * (Vec3::new(cos, sin, 0.0) * RING_RADIUS);
        // Along the circle at that angle.
        let rod = turn * tilt * Quat::from_rotation_z(angle);
        cubes.push(cube(
            place(at),
            Vec3::new(RING_THICK, length, RING_THICK) * shown,
            rod,
            ring,
        ));
    }
}

/// A way of holding a hand, as tests and previews pose the others' hands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Flat, the fingers together and the thumb out.
    Open,
    /// The index finger straight, the others curled and the thumb tucked over them.
    Point,
    /// A fist with the thumb straight out of it, which reads as a thumb up turned so that the thumb is above.
    Thumb,
}

/// Each finger's knuckle across the hand, index to little, in the frame of a right hand held palm down (X to its right,
/// Y out of its back, −Z towards its fingers, from the wrist), and how long its bones are from it, in metres.
const FINGERS: [(f32, f32, [f32; 3]); 4] = [
    (-0.025, -0.085, [0.045, 0.026, 0.022]),
    (-0.005, -0.088, [0.05, 0.029, 0.024]),
    (0.014, -0.084, [0.046, 0.027, 0.023]),
    (0.031, -0.075, [0.035, 0.02, 0.02]),
];
/// How far each bone of a curled finger turns from the one before, towards the palm, in radians.
const CURL: [f32; 3] = [1.45, 1.6, 1.0];
/// A right thumb's joints, metacarpal to tip, held out and tucked over the curled fingers.
const THUMB_OUT: [Vec3; 4] = [
    Vec3::new(-0.022, -0.01, -0.022),
    Vec3::new(-0.045, -0.015, -0.045),
    Vec3::new(-0.062, -0.015, -0.068),
    Vec3::new(-0.074, -0.015, -0.088),
];
const THUMB_TUCKED: [Vec3; 4] = [
    Vec3::new(-0.022, -0.015, -0.022),
    Vec3::new(-0.035, -0.035, -0.048),
    Vec3::new(-0.018, -0.048, -0.066),
    Vec3::new(-0.002, -0.05, -0.076),
];

/// A tracked hand on `side` held as `gesture`, its wrist at `wrist` turned `turn` from palm down with its fingers along
/// −Z, in the protocol's form, every joint seen, with a radius alike to a hand's.
pub fn posed(gesture: Gesture, side: usize, wrist: Vec3, turn: Quat) -> tomori_protocol::Hand {
    let mirror = if side == 0 { -1.0 } else { 1.0 };
    let mut joints = [None; tomori_protocol::JOINT_COUNT];
    let mut set = |index: usize, at: Vec3, radius: f32| {
        let at = wrist + turn * Vec3::new(at.x * mirror, at.y, at.z);
        joints[index] = Some(tomori_protocol::Joint {
            position: at.to_array(),
            radius,
        });
    };
    set(PALM, Vec3::new(0.0, 0.0, -0.045), 0.025);
    set(WRIST, Vec3::ZERO, 0.02);
    let thumb = if gesture == Gesture::Point {
        THUMB_TUCKED
    } else {
        THUMB_OUT
    };
    for (index, at) in thumb.into_iter().enumerate() {
        set(2 + index, at, 0.013 - 0.0015 * index as f32);
    }
    for (finger, (across, knuckle, bones)) in FINGERS.into_iter().enumerate() {
        let first = 6 + finger * 5;
        let curled = match gesture {
            Gesture::Open => false,
            Gesture::Point => finger > 0,
            Gesture::Thumb => true,
        };
        set(first, Vec3::new(across * 0.6, 0.0, -0.025), 0.011);
        let mut at = Vec3::new(across, 0.0, knuckle);
        set(first + 1, at, 0.011);
        let mut bend = 0.0;
        for (bone, length) in bones.into_iter().enumerate() {
            if curled {
                bend += CURL[bone];
            }
            at += Vec3::new(0.0, -bend.sin(), -bend.cos()) * length;
            set(first + 2 + bone, at, 0.009 - 0.001 * bone as f32);
        }
    }
    tomori_protocol::Hand { joints }
}

#[cfg(test)]
mod tests;
