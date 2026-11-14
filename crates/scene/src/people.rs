// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The others in a space as this participant sees them: each a head, a stylised block with a visor, facing where they
//! look, and their rays while drawn in full on their own headset, built of the cubes Tomori draws everything with. Each
//! is drawn from their presence between its updates, one update behind, by the same tracks an environment's client side
//! is given their hands from (`relayed`), with no extrapolation; faded out where they were and in where they are when
//! they jump, faded out once silent, faded where they last were as they leave, and fainter close to the eyes. Someone
//! whose head faces this participant's lights their visor, so that eye contact reads in conversation. See
//! `design/people.md`.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};

use crate::client_side::Hand;
use crate::delegation::Cube;
use crate::relayed::{Between, Relayed};

/// How far someone's head moves between two updates, in metres, for it to be a jump, as a teleport is, faded out where
/// they were and in where they are rather than drawn gliding between: as far as a sign moves to be cut.
pub const JUMP: f32 = 1.5;
/// How long a jump fades them out where they were, and then in where they are.
pub const CUT: Duration = Duration::from_millis(100);
/// How long someone's presence says nothing before they fade out, and how long that takes.
pub const SILENT: Duration = Duration::from_secs(2);
pub const SILENT_FADE: Duration = Duration::from_millis(500);
/// How long someone who left the space fades out where they last were.
pub const LEAVING: Duration = Duration::from_millis(300);
/// Within how far of the eyes, in metres, someone's head is drawn fainter, and within how far not at all, so that two
/// people arriving at one place are never inside each other's heads.
pub const NEAR: f32 = 0.6;
pub const NEAREST: f32 = 0.3;
/// Within how many degrees of where someone's head faces this participant's head must be for their visor to light, and
/// beyond how many for it to go out again, so that a glance at the edge does not flicker; and within how far, in metres.
pub const GAZE: f32 = 10.0;
pub const GAZE_LEFT: f32 = 15.0;
pub const GAZE_REACH: f32 = 8.0;
/// How long the visor takes to light fully, and to go out.
pub const GLOW_RISE: f32 = 0.2;
pub const GLOW_FALL: f32 = 0.4;
/// How much less the half of a lit visor away from this participant glows than the half towards them, at most.
const GLOW_SIDE: f32 = 0.15;

/// The head's shell, its lighter top, its side pods and antenna, its visor, dark, with the sheen along its top edge, and
/// the antenna's tip, and how the visor, its sheen and the tip glow while the head faces this participant: neutral until
/// people have colours of their own.
pub const SHELL: [f32; 3] = [0.6, 0.61, 0.66];
const TOP: [f32; 3] = [0.8, 0.8, 0.84];
const POD: [f32; 3] = [0.4, 0.41, 0.47];
const VISOR: [f32; 3] = [0.035, 0.04, 0.055];
const SHEEN: [f32; 3] = [0.2, 0.22, 0.28];
const TIP: [f32; 3] = [0.5, 0.52, 0.6];
pub const VISOR_GLOW: [f32; 3] = [0.95, 0.76, 0.46];
const SHEEN_GLOW: [f32; 3] = [1.0, 0.93, 0.78];
/// How much of the visor's glow the side pods catch.
const POD_GLOW: f32 = 0.3;
/// The middle of the head from its eyes, in its own coordinates, which faces along −Z, about which it fades.
const MIDDLE: Vec3 = Vec3::new(0.0, 0.0, 0.05);

/// The head's parts, each from its middle, as large as it is, and how it is coloured: about 18 by 14 by 16 cm, a
/// rounded block of three overlapping, with a lighter top, a pod on either side, a visor across its front in two halves,
/// with a sheen along its top, and a little antenna, off to one side, that catches the visor's glow.
const PARTS: [(Vec3, Vec3, Part); 11] = [
    (Vec3::ZERO, Vec3::new(0.18, 0.12, 0.14), Part::Shell),
    (Vec3::ZERO, Vec3::new(0.16, 0.14, 0.14), Part::Shell),
    (Vec3::ZERO, Vec3::new(0.16, 0.12, 0.16), Part::Shell),
    (
        Vec3::new(0.0, 0.07, 0.01),
        Vec3::new(0.14, 0.008, 0.12),
        Part::Top,
    ),
    (
        Vec3::new(0.093, -0.005, 0.005),
        Vec3::new(0.016, 0.05, 0.06),
        Part::Pod,
    ),
    (
        Vec3::new(-0.093, -0.005, 0.005),
        Vec3::new(0.016, 0.05, 0.06),
        Part::Pod,
    ),
    (
        Vec3::new(-0.042, 0.004, -0.082),
        Vec3::new(0.084, 0.058, 0.012),
        Part::Visor(-1.0),
    ),
    (
        Vec3::new(0.042, 0.004, -0.082),
        Vec3::new(0.084, 0.058, 0.012),
        Part::Visor(1.0),
    ),
    (
        Vec3::new(0.0, 0.03, -0.0885),
        Vec3::new(0.15, 0.004, 0.002),
        Part::Sheen,
    ),
    (
        Vec3::new(0.045, 0.088, 0.04),
        Vec3::new(0.006, 0.03, 0.006),
        Part::Pod,
    ),
    (
        Vec3::new(0.045, 0.108, 0.04),
        Vec3::new(0.018, 0.018, 0.018),
        Part::Tip,
    ),
];

/// How many cubes one person is drawn with at most: their head and two rays, each with the dot where it ends.
pub const CUBES: usize = PARTS.len() + 4;

/// What each part of the head is: a half of the visor on its left, −1, or right, 1.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Shell,
    Top,
    Pod,
    Visor(f32),
    Sheen,
    Tip,
}

impl Part {
    /// Its colour, `glow` of the way to glowing, towards this participant on the head's right, `towards` 1, or its left,
    /// −1, or between.
    fn color(self, glow: f32, towards: f32) -> [f32; 3] {
        let mix = |from: [f32; 3], to: [f32; 3], t: f32| {
            Vec3::from_array(from)
                .lerp(Vec3::from_array(to), t)
                .to_array()
        };
        match self {
            Part::Shell => SHELL,
            Part::Top => TOP,
            Part::Pod => mix(POD, VISOR_GLOW, glow * POD_GLOW),
            Part::Visor(side) => mix(
                VISOR,
                VISOR_GLOW,
                glow * (1.0 - GLOW_SIDE * (1.0 - side * towards) / 2.0),
            ),
            Part::Sheen => mix(SHEEN, SHEEN_GLOW, glow),
            Part::Tip => mix(TIP, SHEEN_GLOW, glow),
        }
    }
}

/// A ray's colour, as this participant's own is drawn, meeting something or not, and a little fainter; and how thick
/// it is, and how large the dot where it ends on what it meets.
const RAY: [f32; 3] = [0.55, 0.6, 0.75];
const RAY_MEETS: [f32; 3] = [0.45, 0.95, 1.0];
const RAY_FAINTER: f32 = 0.6;
const RAY_WIDTH: f32 = 0.003;
const DOT: f32 = 0.012;

/// What someone else's presence last said, as the conversation keeps it, in the space's scene coordinates, and how many
/// of their reports have arrived, so that a report alike to the last is still heard.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub head: tomori_protocol::Pose,
    /// Their controllers' poses, left then right, while tracked.
    pub controllers: [Option<tomori_protocol::Pose>; 2],
    /// Their tracked hands, left then right.
    pub hands: [Option<tomori_protocol::Hand>; 2],
    /// Their rays drawn in full on their headset, left then right.
    pub pointing: tomori_protocol::Pointing,
    pub reports: u64,
}

/// A tracked hand's joint as it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    pub position: Vec3,
    pub radius: f32,
}

/// A tracked hand's joints as they are drawn, each absent where its tracker cannot see it.
pub type Joints = [Option<Joint>; tomori_protocol::JOINT_COUNT];

/// One of someone's rays, drawn in full on their headset: from where, which way, how far, and whether it ends on
/// something it meets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    /// A unit vector.
    pub direction: Vec3,
    pub length: f32,
    pub meets: bool,
}

impl Ray {
    /// Where it ends.
    pub fn end(&self) -> Vec3 {
        self.origin + self.direction * self.length
    }
}

/// Someone's whole presence as it is drawn between its updates, in the space's scene coordinates: their head, their
/// controllers, their tracked hands and their rays, each left then right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Present {
    pub head: Hand,
    pub controllers: [Option<Hand>; 2],
    pub joints: [Option<Joints>; 2],
    pub pointing: [Option<Ray>; 2],
}

/// `pose` as it is drawn, if it is finite and within reach and has a rotation, made unit.
fn placed(pose: &tomori_protocol::Pose) -> Option<Hand> {
    let position = Vec3::from_array(pose.position);
    let orientation = Quat::from_array(pose.orientation);
    (position.is_finite()
        && position.abs().max_element() < 10_000.0
        && orientation.is_finite()
        && orientation.length() > 0.5)
        .then(|| Hand::new(position, orientation.normalize()))
}

impl Present {
    /// What `heard` says, if its head is a pose: whatever else of it is not is left out, so that nothing another client
    /// sends makes the drawing fail.
    pub fn of(heard: &Heard) -> Option<Self> {
        let joints = |hand: &Option<tomori_protocol::Hand>| {
            (*hand).filter(tomori_protocol::Hand::valid).map(|hand| {
                hand.joints.map(|joint| {
                    joint.map(|joint| Joint {
                        position: Vec3::from_array(joint.position),
                        radius: joint.radius,
                    })
                })
            })
        };
        let ray = |ray: &Option<tomori_protocol::Ray>| {
            (*ray).filter(tomori_protocol::Ray::valid).and_then(|ray| {
                placed(&ray.aim).map(|aim| Ray {
                    origin: aim.position,
                    direction: aim.orientation * Vec3::NEG_Z,
                    length: ray.length,
                    meets: ray.meets,
                })
            })
        };
        Some(Self {
            head: placed(&heard.head)?,
            controllers: heard.controllers.map(|pose| pose.as_ref().and_then(placed)),
            joints: [joints(&heard.hands[0]), joints(&heard.hands[1])],
            pointing: [ray(&heard.pointing[0]), ray(&heard.pointing[1])],
        })
    }
}

/// `was` and `will` a share `t` of the way between, where both are; or else `will`, as it is now.
fn either<T: Copy>(was: Option<T>, will: Option<T>, between: impl Fn(T, T) -> T) -> Option<T> {
    match (was, will) {
        (Some(was), Some(will)) => Some(between(was, will)),
        _ => will,
    }
}

impl Between for Present {
    fn between(&self, will: &Self, t: f32) -> Self {
        let joints = |was: Joints, will: Joints| {
            std::array::from_fn(|index| {
                either(was[index], will[index], |was, will| Joint {
                    position: was.position.lerp(will.position, t),
                    radius: was.radius + (will.radius - was.radius) * t,
                })
            })
        };
        let ray = |was: Ray, will: Ray| Ray {
            origin: was.origin.lerp(will.origin, t),
            direction: was
                .direction
                .lerp(will.direction, t)
                .try_normalize()
                .unwrap_or(will.direction),
            length: was.length + (will.length - was.length) * t,
            meets: will.meets,
        };
        Self {
            head: self.head.between(&will.head, t),
            controllers: std::array::from_fn(|index| {
                either(
                    self.controllers[index],
                    will.controllers[index],
                    |was, will| was.between(&will, t),
                )
            }),
            joints: std::array::from_fn(|index| {
                either(self.joints[index], will.joints[index], joints)
            }),
            pointing: std::array::from_fn(|index| {
                either(self.pointing[index], will.pointing[index], ray)
            }),
        }
    }

    fn cuts(&self, was: &Self) -> bool {
        self.head.position.distance(was.head.position) > JUMP
    }
}

/// One person as drawn this frame, in the space's scene coordinates: their head's cubes, unless theirs is the head a
/// watcher sees from, and their rays'.
#[derive(Clone, Debug, Default)]
pub struct Figure {
    pub source: u32,
    pub cubes: Vec<Cube>,
}

/// Someone in the space as drawn: when their latest report arrived, how many had, when their track last cut and what was
/// drawn before it, what was drawn last frame, how much, and whether their head faces this participant's and how much
/// their visor glows.
#[derive(Clone, Debug)]
struct Known {
    reports: u64,
    heard: Instant,
    cut: Option<Instant>,
    before: Option<(Instant, Present)>,
    drawn: Present,
    shown: f32,
    gazing: bool,
    glow: f32,
}

/// Someone who left the space, fading where they last were.
#[derive(Clone, Debug)]
struct Leaving {
    source: u32,
    since: Instant,
    drawn: Present,
    shown: f32,
    glow: f32,
}

/// The others in the space, as drawn each frame.
#[derive(Clone, Debug, Default)]
pub struct People {
    relayed: Relayed<Present>,
    forgotten: u64,
    known: BTreeMap<u32, Known>,
    leaving: Vec<Leaving>,
    figures: Vec<Figure>,
    drawn: Vec<(u32, Present)>,
}

impl People {
    /// The others at `now`, `dt` seconds after the last frame, from `heard`, each as their presence last said, by their
    /// numbers, once the conversation has forgotten them `forgotten` times, seen from `eyes`, where this participant's
    /// eyes are, and leaving out the head of `followed`, the participant a watcher sees from.
    pub fn update<'a>(
        &mut self,
        now: Instant,
        dt: f32,
        forgotten: u64,
        heard: impl IntoIterator<Item = (u32, &'a Heard)>,
        eyes: Option<Vec3>,
        followed: Option<u32>,
    ) {
        if self.forgotten != forgotten {
            // Everyone may now be someone else, drawn anew where they first are, and nobody leaves.
            self.forgotten = forgotten;
            self.known.clear();
            self.leaving.clear();
        }
        self.relayed.since(forgotten);
        let heard: Vec<(u32, u64, Present)> = heard
            .into_iter()
            .filter_map(|(source, heard)| Some((source, heard.reports, Present::of(heard)?)))
            .collect();
        let drawn = self.relayed.update(
            now,
            heard
                .iter()
                .map(|(source, _, present)| (*source, *present))
                .collect(),
        );
        // Whoever is no longer here leaves, fading where they last were.
        let leaving = &mut self.leaving;
        self.known.retain(|source, known| {
            let here = drawn.iter().any(|(drawn, _)| drawn == source);
            if !here {
                leaving.push(Leaving {
                    source: *source,
                    since: now,
                    drawn: known.drawn,
                    shown: known.shown,
                    glow: known.glow,
                });
            }
            here
        });
        self.leaving
            .retain(|leaving| now.saturating_duration_since(leaving.since) < LEAVING);
        self.drawn.clear();
        let mut figures = std::mem::take(&mut self.figures);
        let mut used = 0;
        for ((source, present), (_, reports, _)) in drawn.into_iter().zip(&heard) {
            let cut = self.relayed.cut(source);
            let known = self.known.entry(source).or_insert(Known {
                reports: *reports,
                heard: now,
                cut,
                before: None,
                drawn: present,
                shown: 1.0,
                gazing: false,
                glow: 0.0,
            });
            if known.reports != *reports {
                known.reports = *reports;
                known.heard = now;
            }
            // A jump fades them out where they were drawn, then in where they are.
            if known.cut != cut {
                known.cut = cut;
                known.before = Some((now, known.drawn));
            }
            let (shown, mut fade) = match known.before {
                Some((since, before)) => {
                    let t = now.saturating_duration_since(since).as_secs_f32() / CUT.as_secs_f32();
                    if t < 1.0 {
                        (before, 1.0 - t)
                    } else if t < 2.0 {
                        (present, t - 1.0)
                    } else {
                        known.before = None;
                        (present, 1.0)
                    }
                }
                None => (present, 1.0),
            };
            // Silent, they fade out, and in again as they report.
            let quiet = now.saturating_duration_since(known.heard);
            fade *= 1.0
                - (quiet.saturating_sub(SILENT).as_secs_f32() / SILENT_FADE.as_secs_f32()).min(1.0);
            known.gazing = gazes(&shown.head, eyes, known.gazing);
            known.glow = if known.gazing {
                (known.glow + dt / GLOW_RISE).min(1.0)
            } else {
                (known.glow - dt / GLOW_FALL).max(0.0)
            };
            known.drawn = shown;
            known.shown = fade;
            self.drawn.push((source, shown));
            used += figure(
                &mut figures,
                used,
                source,
                &shown,
                fade,
                known.glow,
                eyes,
                followed != Some(source),
            );
        }
        for leaving in &self.leaving {
            let t =
                now.saturating_duration_since(leaving.since).as_secs_f32() / LEAVING.as_secs_f32();
            used += figure(
                &mut figures,
                used,
                leaving.source,
                &leaving.drawn,
                leaving.shown * (1.0 - t),
                leaving.glow,
                eyes,
                followed != Some(leaving.source),
            );
        }
        figures.truncate(used);
        self.figures = figures;
    }

    /// Each person as drawn this frame.
    pub fn figures(&self) -> &[Figure] {
        &self.figures
    }

    /// Each person's presence as drawn this frame, by their number, of which an environment's client side is given
    /// their hands: what was drawn before a jump while they fade out there, and nothing of who has left.
    pub fn drawn(&self) -> &[(u32, Present)] {
        &self.drawn
    }
}

/// Whether `head` faces `eyes`, this participant's, having faced them last frame or not, `gazing`: within [`GAZE`] of
/// where it faces to begin, and [`GAZE_LEFT`] to go on, and within [`GAZE_REACH`].
fn gazes(head: &Hand, eyes: Option<Vec3>, gazing: bool) -> bool {
    let Some(towards) = eyes
        .filter(|eyes| eyes.distance(head.position) <= GAZE_REACH)
        .and_then(|eyes| (eyes - head.position).try_normalize())
    else {
        return false;
    };
    let facing = head.orientation * Vec3::NEG_Z;
    let within = if gazing { GAZE_LEFT } else { GAZE };
    facing.dot(towards) >= within.to_radians().cos()
}

/// Draw `present`, of `source`, into `figures` at `index`, `fade` of the way in, its visor `glow` of the way lit, seen
/// from `eyes`, with its head if `head`: saying how many figures it drew, none where nothing of it shows.
#[allow(clippy::too_many_arguments)]
fn figure(
    figures: &mut Vec<Figure>,
    index: usize,
    source: u32,
    present: &Present,
    fade: f32,
    glow: f32,
    eyes: Option<Vec3>,
    head: bool,
) -> usize {
    let middle = present.head.position + present.head.orientation * MIDDLE;
    // Fainter close to the eyes, and not drawn at all closer still.
    let near = eyes.map_or(1.0, |eyes| {
        ((eyes.distance(middle) - NEAREST) / (NEAR - NEAREST)).clamp(0.0, 1.0)
    });
    let fade = fade.clamp(0.0, 1.0) * near;
    if fade <= 0.01 {
        return 0;
    }
    if figures.len() <= index {
        figures.push(Figure::default());
    }
    let figure = &mut figures[index];
    figure.source = source;
    figure.cubes.clear();
    if head {
        // Shrunk about its middle as it fades, its visor a little lighter on the side towards the eyes.
        let turn = present.head.orientation;
        let towards = eyes
            .and_then(|eyes| (eyes - present.head.position).try_normalize())
            .map_or(0.0, |towards| {
                ((turn * Vec3::X).dot(towards) / GAZE_LEFT.to_radians().sin()).clamp(-1.0, 1.0)
            });
        figure
            .cubes
            .extend(PARTS.iter().map(|(offset, size, part)| Cube {
                position: (middle + turn * (*offset * fade)).to_array(),
                scale: (*size * fade).to_array(),
                color: part.color(glow, towards),
                orientation: turn.to_array(),
            }));
    }
    for ray in present.pointing.iter().flatten() {
        let color = if ray.meets { RAY_MEETS } else { RAY };
        let end = ray.end();
        figure.cubes.push(Cube {
            position: ((ray.origin + end) * 0.5).to_array(),
            scale: [RAY_WIDTH * fade, RAY_WIDTH * fade, ray.length],
            color: color.map(|value| value * RAY_FAINTER),
            orientation: Quat::from_rotation_arc(Vec3::Z, ray.direction).to_array(),
        });
        if ray.meets {
            figure.cubes.push(Cube {
                position: end.to_array(),
                scale: [DOT * fade; 3],
                color: color.map(|value| (value + (1.0 - value) * 0.4) * RAY_FAINTER),
                orientation: Quat::IDENTITY.to_array(),
            });
        }
    }
    usize::from(!figure.cubes.is_empty())
}

#[cfg(test)]
mod tests;
