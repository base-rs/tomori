// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the agent stands for whoever it is talking with, and the way it goes there: the conversation spot, on the
//! straight line from where the agent is to them, [`CONVERSATION`] from them; whether it already stands near enough to
//! need no move; the spot that serves a huddle, the same way; the spot beside an object, off the asker's line of sight
//! to it; and the route, straight, through one waypoint, or streamed. Where the environment gives its walkable ground,
//! [`Ground`], each spot stands on the level of whoever it serves, as high above the floor beneath it as their eyes
//! stand above theirs, and every way there keeps above the ground, never through a floor nor up from beneath one. It
//! holds no I/O and no clock: the hub decides when the agent follows and asks this where. See
//! `design/environments.md`.
use tomori_environment::sightlines::Solid;
use tomori_protocol::{Pose, Wall, entourage};

use crate::stage::geometry::{boxes, rotate};

pub(crate) mod ground;
pub(crate) use ground::Ground;

/// How far from someone's eyes, horizontally, the agent stops as it comes to them, in metres: comfortably within
/// [`NEAR`], so that it never stands at the edge of moving again. The maintainer chose it; 2.5 m is the alternative to
/// try.
pub(crate) const CONVERSATION: f32 = 2.0;

/// How far round them, in degrees, each try swings from the straight line where the spot on it is not clear, either
/// way, until it has gone half a turn.
pub(crate) const SWING: f32 = 10.0;

/// How far from the asker's eyes, horizontally, the agent stands beside an object, in metres, tried in this order: a
/// comfortable conversational distance, between a person's 1.2 m social bubble and the 1.8 m at which a group forms.
pub(crate) const DISTANCES: [f32; 3] = [1.5, 1.8, 1.2];

/// How far it stands beside an object when no spot at [`DISTANCES`] is clear, in metres: as far as the summons stood
/// it.
pub(crate) const FALLBACK: f32 = entourage::SUMMONED;

/// The bearings it is tried at beside an object, in degrees clockwise from the asker's line of sight to it, as seen
/// from above: to their left first, since a panel floating beside them goes to their right first, then their right,
/// then further out, then nearer in.
pub(crate) const BEARINGS: [f32; 6] = [-45.0, 45.0, -70.0, 70.0, -25.0, 25.0];

/// How far its reach stays off the asker's line of sight to an object it stands beside, as seen from their eyes, in
/// degrees, so that it never stands in the way of what they look at.
pub(crate) const SIGHT: f32 = 12.0;

/// How far above the speaker's eyes its middle stands, in metres, as a face across from them.
pub(crate) const RISE: f32 = 0.1;

/// The lowest and highest its middle stands above the floor, in metres.
pub(crate) const HEIGHTS: (f32, f32) = (1.1, 2.4);

/// How far above someone's eyes its middle stands at most where the environment declares a follow height, in metres:
/// as far as a follow height of 5.1 m leaves above eyes 1.6 m up on the space's floor, so that it never hovers out of
/// talking distance above anyone on a lower level.
pub(crate) const HIGHEST_ABOVE: f32 = 3.5;

/// How far above an object its middle stands beside it where the environment declares a follow height, in metres.
pub(crate) const ABOVE_OBJECT: f32 = 0.5;

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

/// How near the speaker's eyes the agent already stands to need no move, in metres: within it, it never moves on its
/// own, only turning to them, whichever way they face and whatever it stands in front of. The maintainer named three or
/// four metres; 4 m is the alternative to try.
pub(crate) const NEAR: f32 = 3.0;

/// How far from an object the asker is taken to look at it from, beside it, in metres, where they stand further
/// from it.
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

/// How far apart, in metres, the eyes of a huddle stand at most up and down, so that someone on a deck above and
/// someone beneath it are no huddle, however near across.
pub(crate) const HUDDLE_RISE: f32 = 1.5;

/// How much higher, each time, a waypoint over the ground is tried, in metres, where the way there passes below it,
/// from the higher of where it goes from and to, and how many times.
pub(crate) const LIFT: f32 = 0.5;
pub(crate) const LIFTS: usize = 12;

/// Someone the agent is talking with, as they began speaking, or as they last stood still: where their head was.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Speaker {
    pub head: Pose,
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
    /// How high above the floor the environment has its middle hover, if it says, in place of [`RISE`] above the eyes.
    pub hover: Option<f32>,
    /// The environment's walkable ground, where it gives it: the floors it stands above and keeps clear of.
    pub ground: Option<Ground>,
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
            hover: None,
            ground: None,
        }
    }

    /// What the agent keeps clear of, standing its middle `hover` above the floor, if the environment says, as it
    /// comes to someone.
    pub(crate) fn hovering(self, hover: Option<f32>) -> Self {
        Self { hover, ..self }
    }

    /// What the agent keeps clear of, standing on `ground`, the environment's walkable ground, where it gives it.
    pub(crate) fn on(self, ground: Option<Ground>) -> Self {
        Self { ground, ..self }
    }

    /// The floor someone whose eyes are at `eyes` stands on, on the ground, where it has one there.
    fn underfoot(&self, eyes: [f32; 3]) -> Option<f32> {
        self.ground.as_ref()?.underfoot(eyes)
    }

    /// Where the agent's middle at `x` and `z` stands for someone whose eyes are at `eyes`, on the floor `floor`: the
    /// eyes it stands for there, as high above the floor beneath it as theirs stand above theirs, and that floor. With no
    /// ground, theirs and the space's floor; on the ground, over a floor there on their level, that floor; over no
    /// floor near no other level, as over a meadow beside a path, theirs, at their eyes over their floor; over or beside
    /// only floors on other levels, as beyond a deck's edge, none where `level`, and otherwise as over none; and,
    /// standing over no floor themselves, a little beneath their eyes.
    fn standing(
        &self,
        x: f32,
        z: f32,
        eyes: [f32; 3],
        floor: Option<f32>,
        level: bool,
    ) -> Option<(f32, f32)> {
        match (&self.ground, floor) {
            (None, _) => Some((eyes[1], entourage::FLOOR)),
            (Some(ground), Some(floor)) => match ground.level_at(x, z, floor) {
                Some(there) => Some((there + eyes[1] - floor, there)),
                None if level && ground.off_level(x, z, floor) => None,
                None => Some((eyes[1], floor)),
            },
            (Some(_), None) => Some((eyes[1], eyes[1] - tomori_environment::sightlines::EYES)),
        }
    }

    /// Whether the agent standing at `pose` keeps clear of the ground, where there is any: clear of its floors, and
    /// nothing of it between `eyes` and its middle.
    fn grounded(&self, pose: &Pose, eyes: [f32; 3]) -> bool {
        self.ground.as_ref().is_none_or(|ground| {
            ground.clear_of(pose.position) && !ground.crosses(eyes, pose.position)
        })
    }

    /// Whether the agent standing at `agent` stands on the level of someone whose floor is at `floor`, or beneath none:
    /// clear of the ground, and its middle no lower than [`ground::GROUND`] above their floor, so that it never
    /// stands below them, nor stuck in a floor.
    fn on_level(&self, agent: &Pose, floor: Option<f32>) -> bool {
        self.ground.as_ref().is_none_or(|ground| {
            ground.clear_of(agent.position)
                && floor.is_none_or(|floor| agent.position[1] >= floor + ground::GROUND)
        })
    }

    /// The heights the agent's middle is tried at for eyes at `eyes` over the floor `floor`, in order: by [`RISE`] and
    /// [`HEIGHTS`] above `floor`; or, where the environment says, at its hover above the space's floor, but no more
    /// than [`HIGHEST_ABOVE`] above the eyes nor less than [`RISE`], so that it stands above whoever it comes to, never
    /// before their face below their eyes nor out of talking distance above them; then, where nothing there is clear,
    /// as beneath a ceiling, [`RISE`] above the eyes.
    fn heights_on(&self, eyes: f32, floor: f32) -> Vec<f32> {
        match self.hover {
            Some(hover) => {
                let face = eyes + RISE;
                let hovering = (entourage::FLOOR + hover)
                    .min(eyes + HIGHEST_ABOVE)
                    .max(face);
                if hovering > face {
                    vec![hovering, face]
                } else {
                    vec![face]
                }
            }
            None => vec![height_over(eyes, floor)],
        }
    }

    /// The height the agent's middle stands at beside an object at `object` for eyes at `eyes`: by [`RISE`] and
    /// [`HEIGHTS`]; or, where the environment declares a follow height, [`ABOVE_OBJECT`] above the object, but never
    /// less than [`RISE`] above the eyes, so that it stands by what it is beside rather than high over it.
    fn beside_height(&self, eyes: f32, object: f32, floor: f32) -> f32 {
        match self.hover {
            Some(_) => (object + ABOVE_OBJECT).max(eyes + RISE),
            None => height_over(eyes, floor),
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
    /// Streaming there as a flock, which needs no way clear of the scenery, through the waypoint, where it has one,
    /// that keeps it above the ground.
    Stream(Option<[f32; 3]>),
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

/// The height the agent's middle stands at for eyes at `eyes` over the floor `floor`, by [`RISE`] and [`HEIGHTS`]
/// above it, by default.
fn height_over(eyes: f32, floor: f32) -> f32 {
    (eyes + RISE).clamp(floor + HEIGHTS.0, floor + HEIGHTS.1)
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

/// The box, upright and level, about the agent standing at `pose`, its reach and [`MARGIN`] on every side, that a
/// panel floating beside someone keeps clear of, so that the panel never stands in the swarm. See
/// `design/projection.md`.
pub(crate) fn body(pose: &Pose) -> Solid {
    let corners = corners(pose);
    let least = std::array::from_fn(|axis| {
        corners
            .iter()
            .map(|corner| corner[axis])
            .fold(f32::INFINITY, f32::min)
    });
    let greatest = std::array::from_fn(|axis| {
        corners
            .iter()
            .map(|corner| corner[axis])
            .fold(f32::NEG_INFINITY, f32::max)
    });
    Solid::new(least, greatest)
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
        && scene.grounded(pose, eyes)
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

/// The turns, in radians, about someone that a spot swings round from the straight line to them where it is not
/// clear: none first, then [`SWING`] more each time, one way and then the other, to half a turn.
fn swings() -> impl Iterator<Item = f32> {
    let steps = (180.0 / SWING).round() as i32;
    std::iter::once(0.0).chain((1..=steps).flat_map(|step| {
        let turn = (step as f32 * SWING).to_radians();
        [turn, -turn]
    }))
}

/// Where the agent, at `from`, stands for `speaker`, clear of `scene`: on the straight line from their eyes towards
/// it, [`CONVERSATION`] from them, at the first of [`Scene::heights_on`] at which it is clear, facing them, whichever way
/// they face or what they attend to, even behind them; where that is not clear, at the same distance, swung round them
/// by the least of [`swings`] at which it is; else none, for it to stay where it is and only turn. On the ground, it
/// stands over a floor on their level, as high above it as their eyes stand above theirs, or over no floor, as high
/// as over theirs, but never over only another level, swinging round them where the line leaves their level, as over
/// a deck's edge; and only where nowhere about them is, over whatever is there, clear of it, at their eyes' height
/// over their floor. Coming to someone, it only ever comes straight towards them. See `design/environments.md`.
pub(crate) fn spot(from: [f32; 3], speaker: &Speaker, scene: &Scene) -> Option<Pose> {
    let eyes = speaker.head.position;
    // Standing where their eyes are, about the vertical, there is no line to them: the way their head faces.
    let back = sub(from, eyes);
    let line = if level(back) > 1e-3 {
        bearing(back)
    } else {
        heading(&speaker.head)
    };
    let panels = scene.panels_near(eyes);
    let floor = scene.underfoot(eyes);
    [true, false].into_iter().find_map(|on_level| {
        (0..2).find_map(|choice| {
            swings()
                .filter_map(|turn| {
                    let towards = line + turn;
                    let (x, z) = (
                        eyes[0] + CONVERSATION * towards.sin(),
                        eyes[2] - CONVERSATION * towards.cos(),
                    );
                    let (over, beneath) = scene.standing(x, z, eyes, floor, on_level)?;
                    let y = *scene.heights_on(over, beneath).get(choice)?;
                    Some(Pose::upright_facing([x, y, z], [eyes[0], y, eyes[2]]))
                })
                .filter_map(|pose| crate::stage::valid(&pose))
                .find(|pose| clear(pose, eyes, scene, &panels))
        })
    })
}

/// Where the agent stands beside an object at `object`, for the asker, whose head is at `head`, looking at it from
/// where they stand, clear of `scene`: at the first of [`DISTANCES`] and [`BEARINGS`] about the line from their eyes to
/// it at which it stands clear and out of their way, at [`Scene::beside_height`], facing them; else the same bearings at
/// [`FALLBACK`]; else none. See `design/environments.md`.
pub(crate) fn beside(head: &Pose, object: [f32; 3], scene: &Scene) -> Option<Pose> {
    let eyes = head.position;
    let line = {
        let to_object = sub(object, eyes);
        if level(to_object) > 1e-3 {
            bearing(to_object)
        } else {
            heading(head)
        }
    };
    let panels = scene.panels_near(eyes);
    let floor = scene.underfoot(eyes).unwrap_or(entourage::FLOOR);
    let y = scene.beside_height(eyes[1], object[1], floor);
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
        .find(|pose| clear(pose, eyes, scene, &panels) && !in_the_way(pose, eyes, object))
}

/// Where the agent, at `from`, stands for a huddle, `speakers`, who spoke to it lately and stand together, the one it
/// answers first: on the straight line from the middle of their heads towards it, about [`CONVERSATION`] from the
/// nearest of them, facing that middle, at the first of [`Scene::heights_on`] at which it is clear for their eyes on
/// average, clear of `scene` and seen by each, whichever way they face; where that is not clear, swung round them by
/// the least of [`swings`] at which it is; else where it stands for the first alone, by [`spot`]; else none. On the
/// ground, it stands over a floor on their level, as [`spot`] does, their floor the average of theirs. See
/// `design/environments.md`.
pub(crate) fn huddle(from: [f32; 3], speakers: &[Speaker], scene: &Scene) -> Option<Pose> {
    let count = speakers.len() as f32;
    let middle: [f32; 3] = std::array::from_fn(|axis| {
        speakers
            .iter()
            .map(|speaker| speaker.head.position[axis])
            .sum::<f32>()
            / count
    });
    let back = sub(from, middle);
    let line = if level(back) > 1e-3 {
        bearing(back)
    } else {
        heading(&speakers[0].head)
    };
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
    let floor = speakers
        .iter()
        .map(|speaker| scene.underfoot(speaker.head.position))
        .sum::<Option<f32>>()
        .map(|sum| sum / count);
    [true, false]
        .into_iter()
        .find_map(|on_level| {
            (0..2).find_map(|choice| {
                swings()
                    .filter_map(|turn| {
                        let towards = line + turn;
                        let (dx, dz) = (towards.sin(), -towards.cos());
                        // Out from the middle of the group until it stands far enough from the nearest of them.
                        let [x, _, z] = (0..=60)
                            .map(|step| step as f32 * 0.1)
                            .map(|out| [middle[0] + dx * out, middle[1], middle[2] + dz * out])
                            .find(|at| nearest(*at) >= CONVERSATION)?;
                        let (over, beneath) = scene.standing(x, z, middle, floor, on_level)?;
                        let y = *scene.heights_on(over, beneath).get(choice)?;
                        Some(Pose::upright_facing([x, y, z], [middle[0], y, middle[2]]))
                    })
                    .filter_map(|pose| crate::stage::valid(&pose))
                    .find(|pose| {
                        speakers
                            .iter()
                            .all(|speaker| clear(pose, speaker.head.position, scene, &panels))
                    })
            })
        })
        .or_else(|| spot(from, &speakers[0], scene))
}

/// Whether the agent standing at `agent` already serves `speaker`, so that it never moves on its own, only turning to
/// them: within [`NEAR`] of their eyes; or, where `scene` hovers it at the environment's follow height, well above
/// their eyes, within [`NEAR`] of them across the floor. Distance is the only rule that moves it: where it stands off
/// the way they face, or in their way to what they attend to, it stays, and moves only when asked; once it moves, it
/// comes straight towards them, by [`spot`], to [`CONVERSATION`], within this. On the ground, it serves them only on
/// their level, clear of the ground, never below their floor nor stuck in one, however near.
pub(crate) fn near_enough(agent: &Pose, speaker: &Speaker, scene: &Scene) -> bool {
    let apart = sub(agent.position, speaker.head.position);
    if !scene.on_level(agent, scene.underfoot(speaker.head.position)) {
        false
    } else if scene.hover.is_some() {
        level(apart) <= NEAR
    } else {
        distance(agent.position, speaker.head.position) <= NEAR
    }
}

/// Whether the agent standing at `agent` already serves `huddle`, those who spoke to it lately standing together, the
/// one it answers first: within [`HUDDLE`] of the middle of their heads, horizontally, so that one of them moving about
/// within the group does not move it; on the ground, only on their level, as [`near_enough`] says, above the lowest of
/// their floors.
pub(crate) fn serves(agent: &Pose, huddle: &[Speaker], scene: &Scene) -> bool {
    let lowest = huddle
        .iter()
        .filter_map(|speaker| scene.underfoot(speaker.head.position))
        .reduce(f32::min);
    if !scene.on_level(agent, lowest) {
        return false;
    }
    let count = huddle.len().max(1) as f32;
    let middle: [f32; 3] = std::array::from_fn(|axis| {
        huddle
            .iter()
            .map(|speaker| speaker.head.position[axis])
            .sum::<f32>()
            / count
    });
    level(sub(agent.position, middle)) <= HUDDLE
}

/// The bearing, horizontally, from `from` to `to`, in radians clockwise from −Z as seen from above.
pub(crate) fn towards(from: [f32; 3], to: [f32; 3]) -> f32 {
    bearing(sub(to, from))
}

/// The bearing, horizontally, of the way `head` faces.
pub(crate) fn heading(head: &Pose) -> f32 {
    let (x, z) = level_ahead(head);
    x.atan2(-z)
}

/// The bearing, horizontally, of the way the agent standing at `agent` faces, along its +Z.
pub(crate) fn faces(agent: &Pose) -> f32 {
    bearing(rotate(agent.orientation, [0.0, 0.0, 1.0]))
}

/// How far apart the bearings `a` and `b` are, in radians, either way round.
pub(crate) fn apart(a: f32, b: f32) -> f32 {
    wrapped(a - b).abs()
}

/// Whether someone whose eyes are at `eyes` sees the agent standing at `agent`, nothing of the scene between.
pub(crate) fn sees(agent: &Pose, eyes: [f32; 3], scene: &Scene) -> bool {
    !scene.blocked(eyes, agent.position)
}

/// The way the agent goes from `from` to `to` among `scene`: streamed, beyond [`STREAM`]; straight, where the way
/// passes through no solid nor wall, within [`HEADS`] of nobody's head and keeps above the ground; else through the
/// first clear waypoint, raised [`OVER`] above the tallest head along the way or [`ASIDE`] to either side, or, where
/// the way passes below the ground, as up onto a deck or down from one, over it, by [`over_ground`]; else streamed.
/// Streaming needs no way clear of the scenery, but keeps above the ground as well, by [`over_ground`]'s waypoint.
pub(crate) fn route(from: [f32; 3], to: [f32; 3], scene: &Scene) -> Route {
    let along = sub(to, from);
    let streamed = || Route::Stream(over_ground(from, to, scene.ground.as_ref()));
    if (0..3).map(|axis| along[axis].powi(2)).sum::<f32>().sqrt() > STREAM {
        return streamed();
    }
    let open = |a: [f32; 3], b: [f32; 3]| {
        !scene.blocked(a, b)
            && scene
                .heads
                .iter()
                .all(|head| near_segment(*head, a, b) >= HEADS)
            && scene.ground.as_ref().is_none_or(|ground| ground.open(a, b))
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
    let raised = scene
        .ground
        .iter()
        .flat_map(|_| raised(from, to))
        .collect::<Vec<_>>();
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
    .chain(raised)
    .find(|via| open(from, *via) && open(*via, to))
    .map_or_else(streamed, Route::Via)
}

/// The waypoints over the ground tried between `from` and `to`, in order: above where it sets off, so that it rises
/// first, then above where it goes, so that it travels then descends, then above the middle of the way; at the higher
/// of the two, then [`LIFT`] higher each time, [`LIFTS`] times.
fn raised(from: [f32; 3], to: [f32; 3]) -> impl Iterator<Item = [f32; 3]> {
    let top = from[1].max(to[1]);
    (0..=LIFTS).flat_map(move |lift| {
        let y = top + lift as f32 * LIFT;
        [0.0, 1.0, 0.5].map(|share: f32| {
            [
                from[0] + (to[0] - from[0]) * share,
                y,
                from[2] + (to[2] - from[2]) * share,
            ]
        })
    })
}

/// The waypoint by which the agent's way from `from` to `to` keeps above `ground`, where the straight way does not,
/// as when it passes below a deck or through a raised level's edge: the first of [`raised`] by which both halves keep
/// clear of it, or else the highest, over the middle of the way; none where the straight way keeps clear, or there is
/// no ground.
pub(crate) fn over_ground(
    from: [f32; 3],
    to: [f32; 3],
    ground: Option<&Ground>,
) -> Option<[f32; 3]> {
    let ground = ground?;
    if ground.open(from, to) {
        return None;
    }
    raised(from, to)
        .find(|via| ground.open(from, *via) && ground.open(*via, to))
        .or_else(|| raised(from, to).last())
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

/// Whether two heads stand within [`HUDDLE`] of each other, across, and [`HUDDLE_RISE`], up and down.
pub(crate) fn together(a: [f32; 3], b: [f32; 3]) -> bool {
    level(sub(a, b)) <= HUDDLE && (a[1] - b[1]).abs() <= HUDDLE_RISE
}

/// How far apart `a` and `b` are.
pub(crate) fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let offset = sub(a, b);
    (0..3).map(|axis| offset[axis].powi(2)).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests;
