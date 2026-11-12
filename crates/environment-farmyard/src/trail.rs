// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The walking trail around the outside of the fence: out through the gate, along the front, down the yard's right side
//! along the river's bank, round behind the coop to a lookout over the waterfall's pool, with a log seat, along the edge
//! of the bush on the left and back to the gate; with five exercise stations beside it, each a post with a plaque that
//! says what to do there, standing and safe with a headset on. It is a track of packed earth, a little darker than the
//! yard's, raised a little above the meadow, with raised edges and a few stones along them, low-poly as the yard is.
//!
//! Participants walk it as they walk the yard: its track, from crest to crest, and the clearings beside it
//! ([`clearings`]) are drawn as ground ([`ground`]), walkable where they are drawn, within a centimetre of the yard's
//! earth, joined to it through the gate ([`GATEWAY`], [`way_out`]), a pair of teleport pads ([`pads`]) takes them
//! between the yard and the lookout, and pointing at the
//! waterfall rock from the lookout, or at the lookout's seat from the rock, takes them there ([`targets`]). Its
//! stations, viewpoints and the rock are named anchors ([`places`]), and its posts, its seat and the rock are solids
//! ([`solids`]). It stays within [`REACH`] of the backdrop's middle, so that the
//! whole backdrop stands within the headset's far plane from it, and clear of the river's banks, the pool's rim, the
//! pines, the bush and the fence. See the crate's `context/trail.md`.
use std::f32::consts::TAU;

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_assets::Model;

use crate::backdrop::{self, facing};
use crate::layout;
use crate::props::{palette, scatter};
use crate::shapes::Shapes;

/// How wide the trail's track is, from the crest of one edge to the other, all of it walkable, and half that.
pub const WIDTH: f32 = 1.2;
pub const HALF: f32 = WIDTH / 2.0;
/// How high its track is drawn, above the yard's earth's level and so 18 mm above the meadow, which lies 10 mm below
/// it, clear of the meadow so that nothing fights it, and its edges' crests above the track.
pub const LIFT: f32 = 0.008;
pub const EDGE: f32 = 0.035;
/// How far from its middle the foot of the track's edge is, where its slope meets the meadow.
pub const FOOT: f32 = HALF + 0.16;
/// The height participants stand at on its track: the top of the track as drawn, so that their feet and the ring where a
/// teleport lands are on it, not beneath it; [`LIFT`] above the yard's earth, less than the centimetre a backstep goes
/// up or down, so that nothing stops at the gate.
pub const HEIGHT: f32 = LIFT;
/// The height of its clearings, drawn a little below the track where they meet it, and stood on as drawn.
pub const CLEARING_HEIGHT: f32 = LIFT - 0.006;
/// The height of the yard's earth, which the way out begins on inside the gate.
pub const EARTH: f32 = 0.0;
/// How far from the backdrop's middle every part of the trail stays, so that everything of the backdrop stands within the
/// headset's far plane, 2 m short of it, from anywhere on it, as from the yard.
pub const REACH: f32 = 11.5;

/// The points the trail's loop is smoothed through, in order: from the gate, along the front to the right, down the
/// yard's right side along the river's bank, round behind the coop to the lookout over the pool, along the bush's edge
/// on the left, and along the front back to the gate.
pub const WAYPOINTS: [Vec2; 17] = [
    Vec2::new(0.0, 3.5),
    Vec2::new(3.4, 3.6),
    Vec2::new(6.4, 2.7),
    Vec2::new(8.6, 0.2),
    Vec2::new(9.4, -3.4),
    Vec2::new(9.1, -7.4),
    Vec2::new(7.6, -10.5),
    Vec2::new(5.4, -12.7),
    Vec2::new(2.4, -13.4),
    Vec2::new(-1.2, -13.5),
    Vec2::new(-4.8, -12.9),
    Vec2::new(-8.0, -10.0),
    Vec2::new(-10.0, -5.8),
    Vec2::new(-10.1, -1.4),
    Vec2::new(-8.6, 1.9),
    Vec2::new(-5.6, 3.3),
    Vec2::new(-2.6, 3.7),
];

/// How many points of the trail's middle lie between each two waypoints.
pub const STEPS: usize = 4;

/// The way out of the yard: from the yard's earth inside the gate, between its posts, to the loop's first point.
pub const GATEWAY: [Vec2; 2] = [Vec2::new(0.0, 0.9), WAYPOINTS[0]];

/// The middle of the yard, which the loop goes round.
const YARD: Vec2 = Vec2::new(0.0, -4.7);

/// How far `point` is from the trail's middle, across the ground.
pub fn from_middle(point: Vec2) -> f32 {
    let line = line();
    (0..line.len())
        .map(|index| {
            let (from, to) = (line[index], line[(index + 1) % line.len()]);
            let along = to - from;
            let t = ((point - from).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            point.distance(from + along * t)
        })
        .fold(f32::INFINITY, f32::min)
}

/// The trail's middle, a closed loop smoothed through [`WAYPOINTS`], [`STEPS`] points to each, the first at the gate;
/// the last joins the first.
pub fn centreline() -> Vec<Vec2> {
    line().to_vec()
}

/// [`centreline`], made once.
fn line() -> &'static [Vec2] {
    static LINE: std::sync::OnceLock<Vec<Vec2>> = std::sync::OnceLock::new();
    LINE.get_or_init(smoothed)
}

fn smoothed() -> Vec<Vec2> {
    let count = WAYPOINTS.len();
    let at = |index: usize| WAYPOINTS[index % count];
    (0..count)
        .flat_map(|reach| {
            let [p0, p1, p2, p3] = [count - 1, 0, 1, 2].map(|offset| at(reach + offset));
            (0..STEPS).map(move |step| {
                let t = step as f32 / STEPS as f32;
                let (t2, t3) = (t * t, t * t * t);
                0.5 * (2.0 * p1
                    + (p2 - p0) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
            })
        })
        .collect()
}

/// The point of the trail's middle `index` along it, wrapping round, the way along it there and the way out of the loop,
/// away from the yard, square to it.
pub fn frame(index: usize) -> (Vec2, Vec2, Vec2) {
    let line = line();
    let count = line.len();
    let point = line[index % count];
    let along = (line[(index + 1) % count] - line[(index + count - 1) % count]).normalize();
    let across = along.perp();
    (point, along, across * across.dot(point - YARD).signum())
}

/// How far a clearing beside the trail stands from its middle, how wide it is, and how far its post stands from the
/// clearing's middle, along the trail and away from it.
const CLEARING_OFFSET: f32 = 1.0;
pub const CLEARING: f32 = 0.85;

/// How near, across the ground, a participant comes to a station's anchor for the agent to be told they are at it, as
/// its anchor's `notify_within`: its clearing and the trail beside it, but not the arrival, the gate or a pad.
pub const STATION_RADIUS: f32 = 1.8;

/// An exercise station beside the trail: its anchor's name and its label, the exercise; the exercise on its plaque, in
/// a few short lines; where along the trail it stands, as a point of [`centreline`], and on which side of it, outwards
/// or towards the fence; and how far along from there its post stands, beside its clearing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Station {
    pub name: &'static str,
    pub label: &'static str,
    pub plaque: [&'static str; 3],
    pub at: usize,
    pub side: f32,
    pub along: f32,
}

/// The stations, in order along the trail from the gate, at least 7 m apart: each exercise works standing, with a
/// headset on, counted rather than timed, without leaning on anything, since the posts and rails are not there to lean
/// on. Each clearing is on the side of the trail with room for it within [`REACH`]: outwards at the front, by the river
/// and at the lookout, and towards the fence at the back right corner and by the bush, where the trail runs furthest
/// out.
pub const STATIONS: [Station; 5] = [
    Station {
        name: "station-1",
        label: "10 squats",
        plaque: ["Station 1", "10 squats", ""],
        at: 5,
        side: 1.0,
        along: 0.6,
    },
    Station {
        name: "station-2",
        label: "10 shoulder rolls and 10 neck tilts, 5 each side",
        plaque: ["Station 2", "10 shoulder rolls", "and 10 neck tilts"],
        at: 16,
        side: 1.0,
        along: 0.6,
    },
    Station {
        name: "station-3",
        label: "20 calf raises",
        plaque: ["Station 3", "20 calf raises", ""],
        at: 24,
        side: -1.0,
        along: 0.6,
    },
    Station {
        name: "station-4",
        label: "10 lunges each side",
        plaque: ["Station 4", "10 lunges", "each side"],
        at: 34,
        side: 1.0,
        along: 0.6,
    },
    Station {
        name: "station-5",
        label: "30 arm circles and 3 deep breaths, 15 circles each way",
        plaque: ["Station 5", "30 arm circles and", "3 deep breaths"],
        at: 49,
        side: -1.0,
        along: 0.6,
    },
];

/// How tall a station's post stands, and its plaque: how wide and tall, and how high its middle.
pub const POST_HEIGHT: f32 = 1.45;
const POST: f32 = 0.11;
pub const PLAQUE: Vec2 = Vec2::new(0.52, 0.2);
pub const PLAQUE_HEIGHT: f32 = 1.18;

impl Station {
    /// The middle of its clearing, where a participant stands to read its plaque and exercise.
    pub fn clearing(&self) -> Vec2 {
        let (point, _, outward) = frame(self.at);
        point + outward * self.side * CLEARING_OFFSET
    }

    /// Where its post stands, at the edge of its clearing, a little along the trail.
    pub fn post(&self) -> Vec2 {
        let (_, along, outward) = frame(self.at);
        self.clearing() + outward * self.side * 0.55 + along * self.along
    }

    /// The way its plaque faces: towards its clearing.
    pub fn faces(&self) -> Vec2 {
        (self.clearing() - self.post()).normalize()
    }
}

/// A place along the trail that is a named anchor: its name, kind and label, where its point is, and what it faces,
/// so that whatever gathers there faces whoever stands on the trail; and how near someone comes for the agent to be
/// told, if it is told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub name: &'static str,
    pub kind: &'static str,
    pub label: &'static str,
    pub at: Vec3,
    pub towards: Vec3,
    pub notify_within: Option<f32>,
}

/// The room each of the trail's anchors gives what gathers there, as width, height and depth, so that one to eighteen
/// workers placed there stand in slots of their own, clear of its posts and seat and the plants and pines beside it.
pub const ROOM: [f32; 3] = [5.0, 3.0, 2.0];

/// How high a station's anchor stands above the trail, a little above the head of one exercising, and how far beside
/// its clearing, along the trail, opposite its post; and how high a viewpoint's stands.
const STATION_ANCHOR: (f32, f32) = (2.0, 1.4);
const VIEWPOINT_HEIGHT: f32 = 1.8;

/// The viewpoints along the trail: the riverbank, on the yard's right side, and the lookout behind the coop over the
/// waterfall's pool, where the seat is; each as an anchor's name and label, a point of [`centreline`], and how far
/// beyond the trail its anchor stands: the lookout's a little nearer, so that its room keeps clear of the tree ferns
/// on the pool's bank and of the fourth station's post.
pub const VIEWPOINTS: [(&str, &str, usize, f32); 2] = [
    ("riverbank", "The riverbank", 18, 3.0),
    ("pool-lookout", "The pool lookout", 36, 2.8),
];

/// How far from the waterfall rock's middle, towards the lookout, its anchor stands, and how high above its top: behind
/// whoever stands on it facing the waterfall, so that whatever gathers there is beside them rather than before the fall,
/// and within 20 m of where participants arrive, as every anchor was before the upper terrace.
const ROCK_ANCHOR: (f32, f32) = (1.6, 1.8);

/// The trail's named anchors: its viewpoints, beyond the trail towards what they look at, facing it; the waterfall
/// rock's, beside the rock towards the lookout, facing the rock; the upper terrace's above it, behind whoever arrives
/// there ([`crate::terrace::place`]); and its stations,
/// each beside its clearing, along the trail, opposite its post, facing the clearing, telling the agent of whoever
/// comes within [`STATION_RADIUS`] of it, its clearing and the trail beside it;
/// the agent may go to any, and workers be placed at one, in its [`ROOM`].
pub fn places() -> Vec<Place> {
    let raised = |point: Vec2, height: f32| Vec3::new(point.x, EARTH + height, point.y);
    let high = VIEWPOINT_HEIGHT;
    let mut places: Vec<Place> = VIEWPOINTS
        .iter()
        .map(|(name, label, at, beyond)| {
            let (point, _, outward) = frame(*at);
            Place {
                name,
                kind: "viewpoint",
                label,
                at: raised(point + outward * *beyond, high),
                towards: raised(point, high),
                notify_within: None,
            }
        })
        .collect();
    let (high, beside) = STATION_ANCHOR;
    places.extend(STATIONS.iter().map(|station| {
        let (_, along, _) = frame(station.at);
        let clearing = station.clearing();
        Place {
            name: station.name,
            kind: "station",
            label: station.label,
            at: raised(clearing - along * station.along.signum() * beside, high),
            towards: raised(clearing, high),
            notify_within: Some(STATION_RADIUS),
        }
    }));
    let rock = layout::WATERFALL_ROCK;
    let (lookout, _, _) = frame(LOOKOUT);
    let (beside, high) = ROCK_ANCHOR;
    let back = (lookout - Vec2::new(rock.x, rock.z)).normalize() * beside;
    places.insert(
        2,
        Place {
            name: "waterfall-rock",
            kind: "viewpoint",
            label: "The waterfall rock",
            at: rock + Vec3::new(back.x, high, back.y),
            towards: rock + Vec3::Y * high,
            notify_within: None,
        },
    );
    places.insert(3, crate::terrace::place());
    places
}

/// A teleport pad of the trail: where it is, its name, which the label on its partner shows, and which way a
/// participant arriving on it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadPlace {
    pub centre: Vec3,
    pub name: &'static str,
    pub facing: Vec3,
}

/// The point of [`centreline`] at the lookout, which its seat and its pad's bay stand beside.
const LOOKOUT: usize = 36;

/// The bay beside the trail at the lookout, towards the fence, where its pad stands, out of the way of those walking.
pub fn bay() -> Vec2 {
    let (point, _, outward) = frame(LOOKOUT);
    point - outward * 1.15
}

/// The pair of teleport pads between the yard, beside the way to the gate, and the lookout over the pool, at the far
/// end of the trail; arriving at the lookout faces the pool, and back in the yard the coop.
pub fn pads() -> [PadPlace; 2] {
    let bay = bay();
    [
        PadPlace {
            centre: Vec3::new(3.0, EARTH, 0.3),
            name: "Farmyard",
            facing: Vec3::new(-1.0, 0.0, -1.6),
        },
        PadPlace {
            centre: Vec3::new(bay.x, CLEARING_HEIGHT, bay.y),
            name: "Pool lookout",
            facing: Vec3::new(layout::POOL.x - bay.x, 0.0, layout::POOL.z - bay.y),
        },
    ]
}

/// A teleport target of the trail's, an object participants point at to go to it rather than a pad they step onto:
/// the middle of its foot, how far from its upright axis and how high above its foot it is pointed at, where it takes
/// them, its name, which the label beside it shows, and which way they face there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetPlace {
    pub foot: Vec3,
    pub radius: f32,
    pub height: f32,
    pub arrival: Vec3,
    pub name: &'static str,
    pub facing: Vec3,
    /// How near the middle of its foot one must point from, if only from near it.
    pub within: Option<f32>,
}

/// The waterfall rock and the lookout's seat, each a teleport target: pointing at the rock, from the lookout or anywhere
/// within reach, takes one onto its top, facing the waterfall, and pointing at the seat from the rock takes one back
/// onto the trail before it, facing the pool. The rock's top is where one stands, but not a surface a teleport lands on,
/// so that it is reached only by pointing at it.
pub fn targets() -> [TargetPlace; 2] {
    let rock = layout::WATERFALL_ROCK;
    let (middle, along, length, radius) = seat();
    let (lookout, _, _) = frame(LOOKOUT);
    let before = lookout + along * 0.7;
    let arrival = Vec3::new(before.x, HEIGHT, before.y);
    let level = |towards: Vec3, from: Vec3| (towards - from) * Vec3::new(1.0, 0.0, 1.0);
    [
        TargetPlace {
            foot: Vec3::new(rock.x, layout::ROCK_FOOT, rock.z),
            radius: layout::ROCK_REACH,
            height: rock.y - layout::ROCK_FOOT,
            arrival: rock,
            name: "Waterfall rock",
            facing: level(layout::WATERFALL, rock),
            within: None,
        },
        TargetPlace {
            foot: middle,
            radius: length * 0.5 + 0.05,
            height: radius * 2.0 + 0.1,
            arrival,
            name: "Pool lookout",
            facing: level(layout::POOL, arrival),
            within: None,
        },
    ]
}

/// The clearings beside the trail, each its middle and radius, walkable as the trail is: one at each station, and the
/// bay at the lookout with its pad.
pub fn clearings() -> Vec<(Vec2, f32)> {
    let mut clearings: Vec<(Vec2, f32)> = STATIONS
        .iter()
        .map(|station| (station.clearing(), CLEARING))
        .collect();
    clearings.push((bay(), 0.75));
    clearings
}

/// The walkable outlines of the way out through the gate, each its corners in order on the ground and its height, as
/// drawn there: from the yard's earth to the fence at [`EARTH`], and from there to the loop on its track at [`HEIGHT`],
/// [`WIDTH`] wide, the trail's whole width. Declared by hand, unlike the rest of the trail, since the gate's posts
/// stand closer to it than the clearance that keeps participants from what stands on the ground: 0.12 m, rather than
/// `layout::CLEARANCE`.
pub fn way_out() -> Vec<(Vec<Vec2>, f32)> {
    let [inside, loop_] = GATEWAY;
    let across = |from: f32, to: f32| {
        vec![
            Vec2::new(inside.x - HALF, from),
            Vec2::new(inside.x + HALF, from),
            Vec2::new(loop_.x + HALF, to),
            Vec2::new(loop_.x - HALF, to),
        ]
    };
    let fence = layout::FENCE.high.y;
    vec![
        (across(inside.y, fence), EARTH),
        (across(fence, loop_.y), HEIGHT),
    ]
}

/// The log seat at the lookout, beyond the trail, facing the pool: its middle, the way it lies, along the trail, how
/// long it is and its radius.
pub fn seat() -> (Vec3, Vec2, f32, f32) {
    let (point, along, outward) = frame(LOOKOUT);
    let middle = point + outward * 1.15 + along * 0.7;
    (Vec3::new(middle.x, 0.0, middle.y), along, 1.5, 0.21)
}

/// The solids of the trail, which the agent, walls and panels keep clear of, as `(least, greatest)` corners: each
/// station's post with its plaque, the seat, and the waterfall rock.
pub fn solids() -> Vec<([f32; 3], [f32; 3])> {
    let mut solids: Vec<([f32; 3], [f32; 3])> = STATIONS
        .iter()
        .map(|station| {
            let post = station.post();
            let reach = PLAQUE.x * 0.5 + 0.05;
            (
                [post.x - reach, 0.0, post.y - reach],
                [post.x + reach, POST_HEIGHT + 0.06, post.y + reach],
            )
        })
        .collect();
    let (middle, along, length, radius) = seat();
    let ends = [
        middle + Vec3::new(along.x, 0.0, along.y) * length * 0.5,
        middle - Vec3::new(along.x, 0.0, along.y) * length * 0.5,
    ];
    let low = ends[0].min(ends[1]) - Vec3::new(radius, 0.0, radius);
    let high = ends[0].max(ends[1]) + Vec3::new(radius, 0.0, radius);
    solids.push(([low.x, 0.0, low.z], [high.x, radius * 2.0 + 0.05, high.z]));
    let (rock, reach) = (layout::WATERFALL_ROCK, layout::ROCK_REACH);
    solids.push((
        [rock.x - reach, layout::ROCK_FOOT, rock.z - reach],
        [rock.x + reach, rock.y, rock.z + reach],
    ));
    solids
}

/// How far from the trail's middle, on the ground, the backdrop's plants and pines are counted among [`beside`]: as far
/// as the agent's swarm reaches, coming to 2 m from someone at the far side of a clearing.
pub const BESIDE: f32 = 5.0;

/// The backdrop's plants and pines, and the tree ferns' trunks and crowns ([`crate::ferns`]), within [`BESIDE`] of the
/// trail or the waterfall rock, each as the box it stands within, as `(least, greatest)` corners: the agent, coming to
/// someone on the trail or the rock, keeps clear of them as it does of the trail's [`solids`], swinging round them, so
/// that it never stands in a tree fern or a pine, nor a panel floats in one.
pub fn beside() -> Vec<([f32; 3], [f32; 3])> {
    let line = line();
    let pines = crate::props::pines().into_iter().map(|(base, height)| {
        let reach = height * 0.32;
        (
            (base - Vec3::new(reach, 0.0, reach)).to_array(),
            (base + Vec3::new(reach, height, reach)).to_array(),
        )
    });
    backdrop::plants()
        .iter()
        .map(|plant| {
            let solid = plant.solid();
            (solid.low, solid.high)
        })
        .chain(pines)
        .chain(
            crate::ferns::solids()
                .into_iter()
                .map(|solid| (solid.low, solid.high)),
        )
        .filter(|(low, high)| {
            let (low, high) = (Vec2::new(low[0], low[2]), Vec2::new(high[0], high[2]));
            let rock = Vec2::new(layout::WATERFALL_ROCK.x, layout::WATERFALL_ROCK.z);
            line.iter()
                .chain([&rock])
                .any(|point| (low - *point).max(*point - high).max(Vec2::ZERO).length() < BESIDE)
        })
        .collect()
}

/// What a plaque says, and where its text stands: on its face, centred, `+X` along it and `+Y` up, facing `+Z` towards
/// whoever reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plaque {
    pub lines: [&'static str; 3],
    pub transform: Mat4,
}

/// Each station's plaque, its text just proud of its face.
pub fn plaques() -> Vec<Plaque> {
    STATIONS
        .iter()
        .map(|station| {
            let (post, faces) = (station.post(), station.faces());
            let face = post + faces * (POST * 0.5 + 0.05);
            let yaw = faces.x.atan2(faces.y);
            Plaque {
                lines: station.plaque,
                transform: Mat4::from_rotation_translation(
                    Quat::from_rotation_y(yaw),
                    Vec3::new(face.x, PLAQUE_HEIGHT, face.y),
                ),
            }
        })
        .collect()
}

/// The colours of the trail: its track, its edges and its clearings, and the stones along it.
pub mod colors {
    use glam::Vec3;

    pub const TRACK: Vec3 = Vec3::new(0.2, 0.135, 0.085);
    pub const EDGE: Vec3 = Vec3::new(0.14, 0.11, 0.065);
    pub const CLEARING: Vec3 = Vec3::new(0.22, 0.16, 0.105);
    pub const STONE: Vec3 = Vec3::new(0.2, 0.195, 0.18);
}

/// Whether the edge of the track at `point` is open, without a crest, where the way through the gate or a clearing
/// meets it.
fn opening(point: Vec2) -> bool {
    let [inside, loop_] = GATEWAY;
    let gateway =
        (point.x - inside.x).abs() < HALF + 0.12 && point.y > inside.y && point.y < loop_.y;
    gateway
        || clearings()
            .iter()
            .any(|(middle, radius)| point.distance(*middle) < radius + 0.05)
}

/// A band across the track at one of its points, from its outer foot over its crests to the other: `[x, z]` on the
/// ground and how high, from the outer side to the inner.
fn section(point: Vec2, outward: Vec2) -> [Vec3; 6] {
    let at = |across: f32, height: f32| {
        let ground = point + outward * across;
        Vec3::new(ground.x, height, ground.y)
    };
    let crest = |side: f32| {
        if opening(point + outward * side * HALF) {
            LIFT
        } else {
            LIFT + EDGE
        }
    };
    [
        at(FOOT, -0.004),
        at(HALF, crest(1.0)),
        at(HALF - 0.08, LIFT),
        at(-(HALF - 0.08), LIFT),
        at(-HALF, crest(-1.0)),
        at(-FOOT, -0.004),
    ]
}

/// The quads of the track along `points`, each with the way out of it, as bands across it, between each band and the
/// next: by the band they start at, and by their strip across it, from the outer foot of one edge, 0, over its slope,
/// its crest and the track's middle to the other's crest and its slope, 4.
fn strips(points: &[(Vec2, Vec2)]) -> Vec<(usize, usize, [Vec3; 4])> {
    let sections: Vec<[Vec3; 6]> = points
        .iter()
        .map(|(point, outward)| section(*point, *outward))
        .collect();
    sections
        .windows(2)
        .enumerate()
        .flat_map(|(index, pair)| {
            (0..5).map(move |strip| {
                (
                    index,
                    strip,
                    [
                        pair[0][strip],
                        pair[0][strip + 1],
                        pair[1][strip + 1],
                        pair[1][strip],
                    ],
                )
            })
        })
        .collect()
}

/// The track along `points`, each with the way out of it, as bands across it, quads between each band and the next:
/// its middle and, either side, its raised edges sloping down to the meadow; drawn as ground from one crest to the
/// other, [`WIDTH`] wide, but not beyond them, down to the meadow, which participants do not walk on.
fn track(shapes: &mut Shapes, points: &[(Vec2, Vec2)]) {
    for (index, strip, corners) in strips(points) {
        let color = if strip == 2 {
            colors::TRACK * (0.92 + scatter(index as u32 + 9100) * 0.16)
        } else {
            colors::EDGE
        };
        if (1..=3).contains(&strip) {
            shapes.ground(|shapes| facing(shapes, &corners, Vec3::Y, color));
        } else {
            facing(shapes, &corners, Vec3::Y, color);
        }
    }
}

/// A small stone, low-poly, at `at` on the ground, `size` across, `seed` turning and shaping it.
fn pebble(shapes: &mut Shapes, at: Vec3, size: f32, seed: u32) {
    let sides = 5;
    let turn = scatter(seed) * TAU;
    let corners: Vec<Vec3> = (0..sides)
        .map(|side| {
            let yaw = turn + side as f32 / sides as f32 * TAU;
            let reach = size * 0.5 * (0.75 + scatter(seed + side + 1) * 0.5);
            at + Vec3::new(yaw.sin() * reach, 0.0, yaw.cos() * reach)
        })
        .collect();
    let top = at + Vec3::Y * size * (0.35 + scatter(seed + 9) * 0.2);
    for side in 0..sides as usize {
        let corners = [corners[side], corners[(side + 1) % sides as usize], top];
        let outward = corners.iter().copied().sum::<Vec3>() / 3.0 - at;
        facing(
            shapes,
            &corners,
            outward,
            colors::STONE * (0.85 + scatter(seed + side as u32 + 20) * 0.3),
        );
    }
}

/// A station's post, its cap and its plaque's board, facing its clearing.
fn post(shapes: &mut Shapes, station: &Station) {
    let (at, faces) = (station.post(), station.faces());
    let yaw = faces.x.atan2(faces.y);
    let turn = Quat::from_rotation_y(yaw);
    let base = Vec3::new(at.x, 0.0, at.y);
    shapes.cuboid(
        base + Vec3::Y * POST_HEIGHT * 0.5,
        Vec3::new(POST * 0.5, POST_HEIGHT * 0.5, POST * 0.5),
        turn,
        palette::DARK_WOOD,
    );
    shapes.cuboid(
        base + Vec3::Y * (POST_HEIGHT + 0.025),
        Vec3::new(POST * 0.75, 0.025, POST * 0.75),
        turn,
        palette::WEATHERED,
    );
    // The board, on the post's face, framed in lighter wood, which the text stands just proud of.
    let face = base + Vec3::new(faces.x, 0.0, faces.y) * (POST * 0.5 + 0.02);
    shapes.cuboid(
        face + Vec3::Y * PLAQUE_HEIGHT,
        Vec3::new(PLAQUE.x * 0.5 + 0.025, PLAQUE.y * 0.5 + 0.025, 0.018),
        turn,
        palette::WOOD,
    );
    shapes.cuboid(
        face + Vec3::Y * PLAQUE_HEIGHT + Vec3::new(faces.x, 0.0, faces.y) * 0.01,
        Vec3::new(PLAQUE.x * 0.5, PLAQUE.y * 0.5, 0.015),
        turn,
        palette::DARK_WOOD * 0.7,
    );
}

/// The log seat: a log of eight sides lying on two chocks.
fn log(shapes: &mut Shapes) {
    let (middle, along, length, radius) = seat();
    let along = Vec3::new(along.x, 0.0, along.y);
    let across = Vec3::new(-along.z, 0.0, along.x);
    let centre = middle + Vec3::Y * (radius + 0.04);
    let ring = |end: f32| -> Vec<Vec3> {
        (0..8)
            .map(|side| {
                let angle = side as f32 / 8.0 * TAU + 0.2;
                centre
                    + along * length * 0.5 * end
                    + (across * angle.cos() + Vec3::Y * angle.sin()) * radius
            })
            .collect()
    };
    let (first, last) = (ring(-1.0), ring(1.0));
    for side in 0..8 {
        let next = (side + 1) % 8;
        let corners = [first[side], first[next], last[next], last[side]];
        let outward = corners.iter().copied().sum::<Vec3>() / 4.0 - centre;
        facing(
            shapes,
            &corners,
            outward,
            palette::BARK * (1.1 + scatter(side as u32 + 9300) * 0.4),
        );
    }
    facing(shapes, &first, -along, palette::WOOD);
    facing(shapes, &last, along, palette::WOOD);
    for end in [-0.32, 0.32] {
        let chock = middle + along * length * end;
        shapes.cuboid(
            chock + Vec3::Y * 0.04,
            Vec3::new(0.06, 0.04, radius * 0.9),
            Quat::from_rotation_arc(Vec3::X, along),
            palette::DARK_WOOD,
        );
    }
}

/// The trail, as a model of one node whose mesh draws it all: its track round the loop and out through the gate, its
/// clearings, the stones along its edges, the stations' posts and the seat.
pub fn model() -> Model {
    backdrop::built("trail", shapes())
}

/// What the trail draws as ground, which participants walk on, each triangle as [`model`] draws it: its track round
/// the loop and out through the gate, from crest to crest, and its clearings.
pub fn ground() -> Vec<[Vec3; 3]> {
    shapes().walkable().to_vec()
}

/// The ways the trail's track runs, each point with the way out of it: round the loop, and through the gate, from the
/// fence to the loop's inner edge.
fn ways() -> [Vec<(Vec2, Vec2)>; 2] {
    let count = centreline().len();
    let points: Vec<(Vec2, Vec2)> = (0..=count)
        .map(|index| {
            let (point, _, outward) = frame(index);
            (point, outward)
        })
        .collect();
    let [_, loop_] = GATEWAY;
    let gateway: Vec<(Vec2, Vec2)> = (0..=2)
        .map(|step| {
            let z =
                layout::FENCE.high.y + (loop_.y - HALF - layout::FENCE.high.y) * step as f32 / 2.0;
            (Vec2::new(loop_.x, z), Vec2::X)
        })
        .collect();
    [points, gateway]
}

/// What the trail draws on the ground, which what is thrown onto it rests on, each triangle as [`model`] draws it: its
/// track from one edge's outer foot to the other's, down to the meadow, and its clearings.
pub fn surface() -> Vec<[Vec3; 3]> {
    let mut surface = ground();
    for points in ways() {
        surface.extend(
            strips(&points)
                .into_iter()
                .filter(|(_, strip, _)| [0, 4].contains(strip))
                .flat_map(|(_, _, corners)| crate::shapes::fan(&corners).collect::<Vec<_>>()),
        );
    }
    surface
}

/// The trail's shapes.
fn shapes() -> Shapes {
    let mut shapes = Shapes::new();
    let count = centreline().len();
    for points in ways() {
        track(&mut shapes, &points);
    }
    // The clearings, a little below the track where they meet it.
    for (middle, radius) in clearings() {
        shapes.ground(|shapes| {
            shapes.disc(
                Vec3::new(middle.x, CLEARING_HEIGHT, middle.y),
                radius,
                9,
                colors::CLEARING,
            )
        });
    }
    // A few stones along its edges, now one side, now the other, but where a clearing or the gate opens it.
    for stone in 0..24u32 {
        let index = (stone as usize * count / 24 + (scatter(stone + 9400) * 2.0) as usize) % count;
        let (point, along, outward) = frame(index);
        let side = if scatter(stone + 9410) < 0.5 {
            1.0
        } else {
            -1.0
        };
        let at =
            point + outward * side * (HALF + 0.07) + along * (scatter(stone + 9420) - 0.5) * 0.6;
        if opening(point + outward * side * HALF) {
            continue;
        }
        pebble(
            &mut shapes,
            Vec3::new(at.x, -0.004, at.y),
            0.1 + scatter(stone + 9430) * 0.1,
            stone * 37 + 9500,
        );
    }
    for station in &STATIONS {
        post(&mut shapes, station);
    }
    log(&mut shapes);
    shapes
}

#[cfg(test)]
mod tests;
