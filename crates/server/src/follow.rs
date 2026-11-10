// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the agent stands for whoever it is talking with, and the way it goes there: the conversation spot, beside the
//! speaker, off their line of sight to what they attend to; whether it already stands near enough to need no move; the
//! spot that serves a huddle; and the route, straight, through one waypoint, or streamed. It holds no I/O and no clock:
//! the hub decides when the agent follows and asks this where. See `design/environments.md`.
use tomori_environment::sightlines::Solid;
use tomori_protocol::{Pose, Wall, entourage};

use crate::stage::geometry::{boxes, rotate};

/// How far from the speaker's eyes, horizontally, the agent stands, in metres, tried in this order: a comfortable
/// conversational distance, between a person's 1.2 m social bubble and the 1.8 m at which a group forms.
pub(crate) const DISTANCES: [f32; 3] = [1.5, 1.8, 1.2];

/// How far it stands when no spot at [`DISTANCES`] is clear, in metres: as far as the summons stood it.
pub(crate) const FALLBACK: f32 = entourage::SUMMONED;

/// The bearings it is tried at, in degrees clockwise from the speaker's line of sight to what they attend to, as seen
/// from above: to their left first, since a panel floating beside them goes to their right first, then their right,
/// then further out, then nearer in.
pub(crate) const BEARINGS: [f32; 6] = [-45.0, 45.0, -70.0, 70.0, -25.0, 25.0];

/// How far its reach stays off the speaker's line of sight to what they attend to, as seen from their eyes, in
/// degrees, so that it never stands in the way of what they look at.
pub(crate) const SIGHT: f32 = 12.0;

/// How far above the speaker's eyes its middle stands, in metres, as a face across from them.
pub(crate) const RISE: f32 = 0.1;

/// The lowest and highest its middle stands above the floor, in metres.
pub(crate) const HEIGHTS: (f32, f32) = (1.1, 2.4);

/// How far about its middle the swarm reaches, across, up and down, and towards and away from whoever it faces, in
/// metres: half of `entourage::SWARM` each way, as the swarm's shapes reach while it talks.
pub(crate) const REACH: [f32; 3] = [
    entourage::SWARM[0] / 2.0,
    entourage::SWARM[1] / 2.0,
    entourage::SWARM[2] / 2.0,
];

/// How much larger than its reach, on every side, the box kept clear about it is, in metres.
pub(crate) const MARGIN: f32 = 0.1;

/// How near anyone's head its box, and its way there, come at most, in metres, so that it never ends in someone's face.
pub(crate) const HEADS: f32 = 0.8;

/// How far from the speaker's eyes a floating panel is taken to be in its way, in metres.
pub(crate) const PANELS: f32 = 3.0;

/// The size of a floating panel, in metres, as the server takes it.
const PANEL: [f32; 2] = crate::stage::BESIDE_SIZE;

/// How thick a wall stands about its slots, in metres, that the agent stays clear of.
const THICKNESS: f32 = 0.05;

/// How near the speaker's eyes the agent already stands to need no move, in metres.
pub(crate) const NEAR: f32 = 3.0;

/// How far off the way the speaker's head faces it may stand and need no move, in degrees.
pub(crate) const AHEAD: f32 = 50.0;

/// How far along the way their head faces the speaker is taken to attend to, in metres, with nothing else known.
pub(crate) const GAZE: f32 = 2.0;

/// How much higher than the tallest head along the way a raised waypoint takes its middle, in metres: enough that its
/// way over keeps [`HEADS`] from that head, its swarm's lowest reach more than half a metre above it.
pub(crate) const OVER: f32 = 1.0;

/// How far to either side a waypoint is offset where the way is blocked, in metres.
pub(crate) const ASIDE: f32 = 1.0;

/// Beyond how far, in metres, the agent streams there as a flock rather than gliding.
pub(crate) const STREAM: f32 = 5.0;

/// How far apart, in metres, the heads of a huddle stand at most.
pub(crate) const HUDDLE: f32 = 4.0;

/// Someone the agent is talking with, as they began speaking: where their head was, and what they attended to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Speaker {
    pub head: Pose,
    pub focus: [f32; 3],
}

/// What the agent keeps clear of where it stands and on its way there.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scene {
    /// The environment's solids and the walls shown, each slot a little thicker.
    pub solids: Vec<Solid>,
    /// The panels floating in the space.
    pub panels: Vec<Pose>,
    /// Every participant's head.
    pub heads: Vec<[f32; 3]>,
}

impl Scene {
    /// What the agent keeps clear of among the environment's `solids`, the `walls` shown, the `panels` floating and
    /// everyone's `heads`.
    pub(crate) fn new(
        solids: &[Solid],
        walls: &[Wall],
        panels: Vec<Pose>,
        heads: Vec<[f32; 3]>,
    ) -> Self {
        let mut blocking = solids.to_vec();
        blocking.extend(walls.iter().flat_map(|wall| {
            boxes(wall).map(|(low, high)| {
                Solid::new(
                    low.map(|value| value - THICKNESS),
                    high.map(|value| value + THICKNESS),
                )
            })
        }));
        Self {
            solids: blocking,
            panels,
            heads,
        }
    }

    /// Whether the segment from `from` to `to` passes through a solid or a wall.
    fn blocked(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        self.solids.iter().any(|solid| solid.blocks(from, to))
    }

    /// The boxes of the panels floating within [`PANELS`] of `eyes`.
    fn panels_near(&self, eyes: [f32; 3]) -> Vec<Solid> {
        self.panels
            .iter()
            .filter(|panel| level(sub(panel.position, eyes)) <= PANELS)
            .map(|panel| {
                let right = rotate(panel.orientation, [1.0, 0.0, 0.0]);
                let half: [f32; 3] = std::array::from_fn(|axis| {
                    let up = if axis == 1 { PANEL[1] / 2.0 } else { 0.0 };
                    (right[axis] * PANEL[0] / 2.0).abs() + up + 0.02
                });
                Solid::new(
                    std::array::from_fn(|axis| panel.position[axis] - half[axis]),
                    std::array::from_fn(|axis| panel.position[axis] + half[axis]),
                )
            })
            .collect()
    }
}

/// How the agent goes where it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Route {
    /// Gliding straight there.
    Straight,
    /// Gliding there through this waypoint.
    Via([f32; 3]),
    /// Streaming there as a flock, which needs no way clear.
    Stream,
}

/// The way `head` faces, level, as a unit `(x, z)`.
fn level_ahead(head: &Pose) -> (f32, f32) {
    crate::stage::level_ahead(head)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// How far `offset` reaches, horizontally.
fn level(offset: [f32; 3]) -> f32 {
    offset[0].hypot(offset[2])
}

/// The bearing of `offset`, horizontally, in radians clockwise from −Z as seen from above.
fn bearing(offset: [f32; 3]) -> f32 {
    offset[0].atan2(-offset[2])
}

/// `angle`, in radians, brought within half a turn of nothing.
fn wrapped(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// What someone whose head is at `head` attends to with nothing else known: [`GAZE`] along the way it faces.
pub(crate) fn gaze(head: &Pose) -> [f32; 3] {
    let ahead = rotate(head.orientation, [0.0, 0.0, -1.0]);
    std::array::from_fn(|axis| head.position[axis] + ahead[axis] * GAZE)
}

/// The height the agent's middle stands at for eyes at `eyes`, by [`RISE`] and [`HEIGHTS`].
fn height(eyes: f32) -> f32 {
    (eyes + RISE).clamp(entourage::FLOOR + HEIGHTS.0, entourage::FLOOR + HEIGHTS.1)
}

/// The corners of the box the agent standing at `pose` keeps clear, its reach and [`MARGIN`] on every side, facing as
/// it faces.
fn corners(pose: &Pose) -> [[f32; 3]; 8] {
    let across = rotate(pose.orientation, [1.0, 0.0, 0.0]);
    let ahead = rotate(pose.orientation, [0.0, 0.0, 1.0]);
    let half = REACH.map(|reach| reach + MARGIN);
    std::array::from_fn(|index| {
        let side = |bit: usize| if index & bit == 0 { -1.0 } else { 1.0 };
        std::array::from_fn(|axis| {
            let up = if axis == 1 { half[1] * side(2) } else { 0.0 };
            pose.position[axis]
                + across[axis] * half[0] * side(1)
                + up
                + ahead[axis] * half[2] * side(4)
        })
    })
}

/// How far `point` is from the box the agent standing at `pose` keeps clear.
fn from_box(pose: &Pose, point: [f32; 3]) -> f32 {
    let across = rotate(pose.orientation, [1.0, 0.0, 0.0]);
    let ahead = rotate(pose.orientation, [0.0, 0.0, 1.0]);
    let offset = sub(point, pose.position);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let local = [dot(offset, across), offset[1], dot(offset, ahead)];
    let half = REACH.map(|reach| reach + MARGIN);
    (0..3)
        .map(|axis| (local[axis].abs() - half[axis]).max(0.0).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Whether the agent standing at `pose` keeps clear of `scene` for someone whose eyes are at `eyes`: its box meets no
/// solid, wall or panel floating near them, no edge of it nor any diagonal passing through one, nor stands within
/// [`HEADS`] of anyone's head; and nothing stands between their eyes and its middle.
fn clear(pose: &Pose, eyes: [f32; 3], scene: &Scene, panels: &[Solid]) -> bool {
    let corners = corners(pose);
    // Its twelve edges, by the bit each changes, and its four diagonals.
    let mut lines: Vec<([f32; 3], [f32; 3])> = (0..8)
        .flat_map(|from: usize| {
            [1, 2, 4]
                .into_iter()
                .filter(move |bit| from & bit == 0)
                .map(move |bit| (from, from | bit))
        })
        .map(|(from, to)| (corners[from], corners[to]))
        .collect();
    lines.extend((0..4).map(|from| (corners[from], corners[7 - from])));
    let inside = |solid: &Solid| solid.contains(pose.position);
    let crosses = |solid: &Solid| lines.iter().any(|(from, to)| solid.blocks(*from, *to));
    !scene
        .solids
        .iter()
        .chain(panels)
        .any(|solid| inside(solid) || crosses(solid))
        && scene
            .heads
            .iter()
            .all(|head| from_box(pose, *head) >= HEADS)
        && !scene.blocked(eyes, pose.position)
}

/// Whether the agent standing at `pose` stands in the way of what someone whose eyes are at `eyes` attends to, at
/// `focus`: its reach, as seen from their eyes, comes within [`SIGHT`] of the line to it, about the vertical.
fn in_the_way(pose: &Pose, eyes: [f32; 3], focus: [f32; 3]) -> bool {
    let to_focus = sub(focus, eyes);
    let to_agent = sub(pose.position, eyes);
    if level(to_focus) < 1e-3 {
        return false;
    }
    let away = level(to_agent).max(1e-3);
    let half = (REACH[0]).atan2(away);
    wrapped(bearing(to_agent) - bearing(to_focus)).abs() - half < SIGHT.to_radians()
}

/// Where the agent stands for `speaker`, clear of `scene`: at the first of [`DISTANCES`] and [`BEARINGS`] about the
/// line from their eyes to what they attend to at which it stands clear and out of their way, at [`height`], facing
/// them; else the same bearings at [`FALLBACK`]; else none, for it to stay where it is and only turn. See
/// `design/environments.md`.
pub(crate) fn spot(speaker: &Speaker, scene: &Scene) -> Option<Pose> {
    let eyes = speaker.head.position;
    let line = {
        let to_focus = sub(speaker.focus, eyes);
        if level(to_focus) > 1e-3 {
            bearing(to_focus)
        } else {
            let (x, z) = level_ahead(&speaker.head);
            x.atan2(-z)
        }
    };
    let panels = scene.panels_near(eyes);
    let y = height(eyes[1]);
    DISTANCES
        .iter()
        .chain(std::iter::once(&FALLBACK))
        .flat_map(|distance| BEARINGS.iter().map(move |turn| (*distance, *turn)))
        .map(|(distance, turn)| {
            let towards = line + turn.to_radians();
            let middle = [
                eyes[0] + distance * towards.sin(),
                y,
                eyes[2] - distance * towards.cos(),
            ];
            Pose::upright_facing(middle, [eyes[0], y, eyes[2]])
        })
        .filter_map(|pose| crate::stage::valid(&pose))
        .find(|pose| clear(pose, eyes, scene, &panels) && !in_the_way(pose, eyes, speaker.focus))
}

/// Where the agent stands for a huddle, `speakers`, who spoke to it lately and stand together, the one it answers
/// first: on the side of the group most of them face, about [`DISTANCES`]' first from the nearest of them, facing the
/// middle of their heads, at [`height`] for their eyes on average, clear of `scene` and out of the way of what each of
/// them attends to, and seen by each; else where it stands for the first alone, by [`spot`]; else none. See
/// `design/environments.md`.
pub(crate) fn huddle(speakers: &[Speaker], scene: &Scene) -> Option<Pose> {
    let count = speakers.len() as f32;
    let middle: [f32; 3] = std::array::from_fn(|axis| {
        speakers
            .iter()
            .map(|speaker| speaker.head.position[axis])
            .sum::<f32>()
            / count
    });
    let (x, z) = speakers.iter().fold((0.0, 0.0), |(x, z), speaker| {
        let (dx, dz) = level_ahead(&speaker.head);
        (x + dx, z + dz)
    });
    let facing = if x.hypot(z) > 1e-3 {
        x.atan2(-z)
    } else {
        let (x, z) = level_ahead(&speakers[0].head);
        x.atan2(-z)
    };
    let y = height(middle[1]);
    let panels: Vec<Solid> = speakers
        .iter()
        .flat_map(|speaker| scene.panels_near(speaker.head.position))
        .collect();
    let nearest = |at: [f32; 3]| {
        speakers
            .iter()
            .map(|speaker| level(sub(at, speaker.head.position)))
            .fold(f32::INFINITY, f32::min)
    };
    [0.0f32, -25.0, 25.0, -50.0, 50.0, -75.0, 75.0]
        .into_iter()
        .filter_map(|turn| {
            let towards = facing + turn.to_radians();
            let (dx, dz) = (towards.sin(), -towards.cos());
            // Out from the middle of the group until it stands far enough from the nearest of them.
            (0..=60)
                .map(|step| step as f32 * 0.1)
                .map(|out| [middle[0] + dx * out, y, middle[2] + dz * out])
                .find(|at| nearest(*at) >= DISTANCES[0])
        })
        .map(|at| Pose::upright_facing(at, [middle[0], y, middle[2]]))
        .filter_map(|pose| crate::stage::valid(&pose))
        .find(|pose| {
            speakers.iter().all(|speaker| {
                let eyes = speaker.head.position;
                clear(pose, eyes, scene, &panels) && !in_the_way(pose, eyes, speaker.focus)
            })
        })
        .or_else(|| spot(&speakers[0], scene))
}

/// Whether the agent standing at `agent` already serves `speaker`, so that it only turns to them: within [`NEAR`] of
/// their eyes, within [`AHEAD`] of the way their head faces, seen clearly from their eyes, and out of the way of what
/// they attend to.
pub(crate) fn near_enough(agent: &Pose, speaker: &Speaker, scene: &Scene) -> bool {
    let eyes = speaker.head.position;
    let offset = sub(agent.position, eyes);
    let distance = (0..3).map(|axis| offset[axis].powi(2)).sum::<f32>().sqrt();
    let (x, z) = level_ahead(&speaker.head);
    let off = wrapped(bearing(offset) - x.atan2(-z)).abs();
    distance <= NEAR
        && off <= AHEAD.to_radians()
        && sees(agent, eyes, scene)
        && !in_the_way(agent, eyes, speaker.focus)
}

/// Whether someone whose eyes are at `eyes` sees the agent standing at `agent`, nothing of the scene between.
pub(crate) fn sees(agent: &Pose, eyes: [f32; 3], scene: &Scene) -> bool {
    !scene.blocked(eyes, agent.position)
}

/// The way the agent goes from `from` to `to` among `scene`: streamed, beyond [`STREAM`]; straight, where the way
/// passes through no solid nor wall and within [`HEADS`] of nobody's head; else through the first clear waypoint,
/// raised [`OVER`] above the tallest head along the way or [`ASIDE`] to either side; else streamed.
pub(crate) fn route(from: [f32; 3], to: [f32; 3], scene: &Scene) -> Route {
    let along = sub(to, from);
    if (0..3).map(|axis| along[axis].powi(2)).sum::<f32>().sqrt() > STREAM {
        return Route::Stream;
    }
    let open = |a: [f32; 3], b: [f32; 3]| {
        !scene.blocked(a, b)
            && scene
                .heads
                .iter()
                .all(|head| near_segment(*head, a, b) >= HEADS)
    };
    if open(from, to) {
        return Route::Straight;
    }
    let middle: [f32; 3] = std::array::from_fn(|axis| (from[axis] + to[axis]) / 2.0);
    let tallest = scene
        .heads
        .iter()
        .filter(|head| near_segment(**head, from, to) < HEADS)
        .map(|head| head[1])
        .fold(middle[1], f32::max);
    let length = level(along).max(1e-3);
    let (across_x, across_z) = (-along[2] / length, along[0] / length);
    [
        [middle[0], tallest + OVER, middle[2]],
        [
            middle[0] + across_x * ASIDE,
            middle[1],
            middle[2] + across_z * ASIDE,
        ],
        [
            middle[0] - across_x * ASIDE,
            middle[1],
            middle[2] - across_z * ASIDE,
        ],
    ]
    .into_iter()
    .find(|via| open(from, *via) && open(*via, to))
    .map_or(Route::Stream, Route::Via)
}

/// How near `point` comes to the segment from `from` to `to`.
fn near_segment(point: [f32; 3], from: [f32; 3], to: [f32; 3]) -> f32 {
    let along = sub(to, from);
    let offset = sub(point, from);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let length = dot(along, along);
    let t = if length > 1e-9 {
        (dot(offset, along) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let nearest: [f32; 3] = std::array::from_fn(|axis| from[axis] + along[axis] * t);
    let gap = sub(point, nearest);
    dot(gap, gap).sqrt()
}

/// Whether two heads stand within [`HUDDLE`] of each other.
pub(crate) fn together(a: [f32; 3], b: [f32; 3]) -> bool {
    level(sub(a, b)) <= HUDDLE
}

/// How far apart `a` and `b` are.
pub(crate) fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let offset = sub(a, b);
    (0..3).map(|axis| offset[axis].powi(2)).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests;
