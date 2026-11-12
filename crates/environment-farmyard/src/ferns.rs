// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The ferns close about the trail, near enough to be seen and walked among: tree ferns, ponga, the silver fern, with
//! the silver undersides of its fronds, mamaku, the black tree fern, tallest of them, and wheki, short and rough under a
//! skirt of dead fronds, along the bush's edge, the riverbank, about the pool and the campsite, and a few by the fence;
//! and ground ferns, crown fern, kiokio and hen-and-chickens fern, in drifts along the trail's edges, the banks above
//! the river and round the campsite, a few koru unfurling among them.
//!
//! Each kind is a mesh of its own, low-poly in the farmyard's style and palette, built in code ([`mesh`]): a frond is a
//! long tapering strip arching out and drooping, its edges cut into alternating leaflets, so that it reads as finely
//! divided, many of them about a slender trunk's top, rather than a palm's few broad leaves. Every point says how far
//! it sways in the breeze, its tips most, its trunk and foot not at all, which the client's shader moves by the time
//! alone, and the colour of its underside, which the shader shows where the back of a frond is seen. They are placed
//! deterministically ([`ferns`]), so that every client draws the same ferns, each instanced where it stands, and kept
//! off everywhere participants stand or walk, clear of the water, the backdrop's plants, the trail's posts and seats,
//! the campsite, the views the backdrop keeps open and the rooms of the anchors. The bush's own tree ferns are drawn the
//! same way ([`bush`]), so that none reads as a palm. A tree fern is a pair of solids
//! ([`Fern::solids`]), its trunk and its crown, which the agent, walls and panels keep clear of; a ground fern, under
//! knee to waist high and off every walkable surface, is decoration, which nothing need keep clear of. See the crate's
//! `context/ferns.md`.
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_environment::sightlines::Solid;

use crate::backdrop::{self, BANKS, RIM};
use crate::campsite;
use crate::layout::{self, FENCE, Footprint, POOL};
use crate::props::scatter;
use crate::trail;

/// The most triangles the ferns may draw from anywhere participants stand, every one of them, the bush's tree ferns
/// among them, each in its detail there: a budget of their own, as near foliage, beside the backdrop's.
pub const BUDGET: usize = 24_000;

/// The most fronds a tree fern's crown has, drawn near.
pub const MOST_FRONDS: usize = 18;

/// How far below the ground a tree fern's trunk reaches, so that none floats on a slope.
const SUNK: f32 = 0.4;

/// How high above where anyone stands a tree fern's crown is, at its lowest, wherever it reaches over them: above a
/// tall participant's head and the agent's swarm beside them.
pub const OVERHEAD: f32 = 2.2;

/// How far from the eyes a fern is drawn in full; beyond, it is drawn with fewer fronds and leaflets, and a koru not at
/// all.
pub const NEAR: f32 = 10.0;

/// How far, at most, a fern's points sway in the breeze at its own size: a tree fern's frond tips, and a ground fern's.
pub const TREE_SWAY: f32 = 0.14;
pub const GROUND_SWAY: f32 = 0.06;

/// The colours of the ferns, linear RGB, as the yard's palette is: each kind's fronds above and beneath, its stalks,
/// and the tree ferns' trunks.
pub mod colors {
    use glam::Vec3;

    /// Ponga's fronds, dark green above and silver beneath, its stalks and trunk brown.
    pub const PONGA: Vec3 = Vec3::new(0.065, 0.15, 0.055);
    pub const SILVER: Vec3 = Vec3::new(0.27, 0.32, 0.29);
    pub const PONGA_STALK: Vec3 = Vec3::new(0.11, 0.095, 0.065);
    pub const PONGA_TRUNK: Vec3 = Vec3::new(0.085, 0.065, 0.045);
    /// Mamaku's fronds, a brighter green and paler beneath, its stalks and trunk black.
    pub const MAMAKU: Vec3 = Vec3::new(0.08, 0.17, 0.06);
    pub const MAMAKU_UNDER: Vec3 = Vec3::new(0.11, 0.19, 0.085);
    pub const MAMAKU_STALK: Vec3 = Vec3::new(0.025, 0.022, 0.02);
    pub const MAMAKU_TRUNK: Vec3 = Vec3::new(0.035, 0.03, 0.026);
    /// Wheki's harsh fronds, and its rough brown trunk.
    pub const WHEKI: Vec3 = Vec3::new(0.055, 0.125, 0.045);
    pub const WHEKI_UNDER: Vec3 = Vec3::new(0.08, 0.14, 0.06);
    pub const WHEKI_TRUNK: Vec3 = Vec3::new(0.075, 0.052, 0.035);
    /// The dead fronds of a tree fern's skirt.
    pub const DEAD: Vec3 = Vec3::new(0.16, 0.11, 0.06);
    /// Crown fern's stiff fronds, paler beneath.
    pub const CROWN: Vec3 = Vec3::new(0.06, 0.14, 0.05);
    pub const CROWN_UNDER: Vec3 = Vec3::new(0.11, 0.16, 0.09);
    /// Kiokio's arching fronds, and its young ones, flushed red.
    pub const KIOKIO: Vec3 = Vec3::new(0.075, 0.15, 0.05);
    pub const KIOKIO_UNDER: Vec3 = Vec3::new(0.1, 0.17, 0.07);
    pub const KIOKIO_YOUNG: Vec3 = Vec3::new(0.15, 0.08, 0.045);
    /// Hen-and-chickens fern's soft, bright fronds.
    pub const HEN: Vec3 = Vec3::new(0.1, 0.2, 0.06);
    pub const HEN_UNDER: Vec3 = Vec3::new(0.13, 0.22, 0.085);
    /// A ground fern's stalks.
    pub const STALK: Vec3 = Vec3::new(0.07, 0.08, 0.035);
    /// A koru, a young frond still curled, pale green.
    pub const KORU: Vec3 = Vec3::new(0.15, 0.22, 0.08);
    pub const KORU_UNDER: Vec3 = Vec3::new(0.2, 0.24, 0.13);
}

/// The kinds of fern, each a mesh of its own, and the koru, a young frond still curled, which stands at the heart of a
/// few of the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Ponga, the silver fern: a slender brown trunk, a short skirt of dead fronds, and a wide crown of arching fronds,
    /// silver beneath.
    Ponga,
    /// Mamaku, the black tree fern, the tallest: a black trunk and black stalks, and a crown of long fronds, hanging,
    /// without a skirt.
    Mamaku,
    /// Wheki, the rough tree fern, the shortest: a rough brown trunk under a long skirt of dead fronds, and a crown of
    /// harsh fronds, held stiffly.
    Wheki,
    /// Crown fern, piupiu: a shuttlecock of stiff, upright fronds, their leaflets close as a comb's teeth, pale beneath.
    CrownFern,
    /// Kiokio: a few long fronds arching from the ground, the young ones flushed red.
    Kiokio,
    /// Hen-and-chickens fern: soft, lacy fronds, bright green, drooping from the ground.
    HenAndChickens,
    /// A koru: a young frond, curled.
    Koru,
}

impl Kind {
    /// The tree ferns.
    pub const TREES: [Kind; 3] = [Kind::Ponga, Kind::Mamaku, Kind::Wheki];
    /// The ground ferns.
    pub const GROUND: [Kind; 3] = [Kind::CrownFern, Kind::Kiokio, Kind::HenAndChickens];

    /// Whether it is a tree fern.
    pub fn is_tree(self) -> bool {
        Self::TREES.contains(&self)
    }

    /// The details it is drawn in: near and far, but a koru, which is drawn near alone.
    pub fn details(self) -> &'static [Detail] {
        if self == Kind::Koru {
            &[Detail::Near]
        } else {
            &[Detail::Near, Detail::Far]
        }
    }
}

/// How finely a fern is drawn: in full, within [`NEAR`] of the eyes, or, beyond, with fewer fronds and leaflets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Detail {
    Near,
    Far,
}

impl Detail {
    /// How finely a fern whose heart is at `heart` is drawn for eyes at `eyes`: in full within [`NEAR`] of it, and with
    /// less beyond.
    pub fn seen(heart: Vec3, eyes: Vec3) -> Self {
        if heart.distance(eyes) <= NEAR {
            Detail::Near
        } else {
            Detail::Far
        }
    }
}

/// Every mesh the ferns are drawn from, each kind in each of its details, in the order the scenery draws them, one draw
/// each with all its instances.
pub fn drawn() -> Vec<(Kind, Detail)> {
    Kind::TREES
        .iter()
        .chain(&Kind::GROUND)
        .chain(&[Kind::Koru])
        .flat_map(|kind| kind.details().iter().map(move |detail| (*kind, *detail)))
        .collect()
}

/// A point of a fern's mesh, a corner of one of its triangles, at its own size: where it is, the way its face faces on
/// its upper side, its colour there and beneath, and how far it sways in the breeze, at most, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub position: Vec3,
    pub normal: Vec3,
    pub color: Vec3,
    pub underside: Vec3,
    pub sway: f32,
}

/// `color` varied a little by `seed`, by at most 6% of itself, so that each face reads as a hand-made model's does.
fn varied(color: Vec3, seed: u32) -> Vec3 {
    color * (0.94 + scatter(seed.wrapping_mul(7) + 40_000) * 0.12)
}

/// A fern's mesh under construction: triangles of points, each face flat; and how far its trunk's faces reach from its
/// axis and how high, and its fronds', from how low to how high, as [`Extent`] says.
struct Builder {
    points: Vec<Point>,
    faces: u32,
    /// How far a point at a place sways, for fronds as long as `span`.
    sway: fn(Vec3, f32) -> f32,
    span: f32,
    /// Whether the faces being built are its trunk's.
    trunk: bool,
    extent: Extent,
}

impl Builder {
    fn new(sway: fn(Vec3, f32) -> f32, span: f32) -> Self {
        Self {
            points: Vec::new(),
            faces: 0,
            sway,
            span,
            trunk: false,
            extent: Extent {
                trunk: (0.0, 0.0),
                crown: (0.0, f32::INFINITY, f32::NEG_INFINITY),
            },
        }
    }

    /// A flat polygon through `corners`, in order, its upper side the one `up` points to, a corner that repeats the one
    /// before it dropped, as where a frond's blade narrows to its midrib or its tip; or nothing, with fewer than three
    /// corners left.
    fn polygon(&mut self, corners: &[Vec3], up: Vec3, (color, underside): (Vec3, Vec3)) {
        let mut kept: Vec<Vec3> = Vec::with_capacity(corners.len());
        for corner in corners {
            if kept.last().is_none_or(|last| last.distance(*corner) > 1e-5) {
                kept.push(*corner);
            }
        }
        if kept.len() < 3 {
            return;
        }
        let mut normal = (kept[1] - kept[0]).cross(kept[2] - kept[0]).normalize();
        if normal.dot(up) < 0.0 {
            kept.reverse();
            normal = -normal;
        }
        let (color, underside) = (varied(color, self.faces), varied(underside, self.faces));
        self.faces += 1;
        for corner in &kept {
            let out = Vec2::new(corner.x, corner.z).length();
            let extent = &mut self.extent;
            if self.trunk {
                extent.trunk = (extent.trunk.0.max(out), extent.trunk.1.max(corner.y));
            } else {
                let (reach, low, high) = extent.crown;
                extent.crown = (reach.max(out), low.min(corner.y), high.max(corner.y));
            }
        }
        for index in 1..kept.len() - 1 {
            for corner in [kept[0], kept[index], kept[index + 1]] {
                self.points.push(Point {
                    position: corner,
                    normal,
                    color,
                    underside,
                    sway: (self.sway)(corner, self.span),
                });
            }
        }
    }
}

/// A frond: from `base`, `yaw` round from +Z towards +X, rising at `pitch` and `length` long, arching over and its tip
/// falling `droop` below where it would have reached, its stalk bare for `stipe` of its length and its blade at
/// most `width` either side, cut into `segments`, each with a leaflet either side, alternately, `notch` of its width
/// deep, its upper side `colors.0`, its underside `colors.1` and its stalk `stalk`.
#[derive(Clone, Copy, Debug)]
struct Frond {
    base: Vec3,
    yaw: f32,
    pitch: f32,
    length: f32,
    droop: f32,
    width: f32,
    stipe: f32,
    segments: usize,
    notch: f32,
    colors: (Vec3, Vec3),
    stalk: Vec3,
}

impl Frond {
    fn outward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos())
    }

    /// The point of its middle `t` along it, from its base at 0 to its tip at 1.
    fn centre(&self, t: f32) -> Vec3 {
        let (reach, rise) = (
            self.length * self.pitch.cos(),
            self.length * self.pitch.sin(),
        );
        self.base
            + self.outward() * reach * t
            + Vec3::Y * (rise * (t * PI * 0.5).sin() - self.droop * t * t)
    }

    /// The way it runs at `t`.
    fn tangent(&self, t: f32) -> Vec3 {
        let (reach, rise) = (
            self.length * self.pitch.cos(),
            self.length * self.pitch.sin(),
        );
        let climb = rise * PI * 0.5 * (t * PI * 0.5).cos() - 2.0 * self.droop * t;
        (self.outward() * reach + Vec3::Y * climb).normalize()
    }

    /// How wide its blade is either side at `t`: nothing along its stalk, widest a third of the way along its blade, and
    /// tapering to its tip.
    fn half_width(&self, t: f32) -> f32 {
        if t <= self.stipe {
            return 0.0;
        }
        let along = ((t - self.stipe) / (1.0 - self.stipe)).min(1.0);
        self.width * (PI * along.powf(0.7)).sin().max(0.0)
    }
}

fn frond(builder: &mut Builder, frond: &Frond) {
    let outward = frond.outward();
    let side = Vec3::new(outward.z, 0.0, -outward.x);
    let up = |t: f32| frond.tangent(t).cross(side);
    // The stalk, a narrow strip.
    let stalk = (frond.width * 0.1).max(0.008);
    let (from, to) = (frond.centre(0.0), frond.centre(frond.stipe));
    builder.polygon(
        &[
            from + side * stalk,
            from - side * stalk,
            to - side * stalk * 0.7,
            to + side * stalk * 0.7,
        ],
        up(frond.stipe * 0.5),
        (frond.stalk, frond.stalk),
    );
    // The blade: in each segment, the midrib and the inner leaflets as one strip, and the leaflets' outer ends as teeth,
    // swept towards the tip, one either side, alternately, so that its edges are notched.
    let span = (1.0 - frond.stipe) / frond.segments as f32;
    let at = |t: f32, across: f32| frond.centre(t) + side * across;
    for segment in 0..frond.segments {
        let t0 = frond.stipe + span * segment as f32;
        let t1 = t0 + span;
        let inner = |t: f32| frond.half_width(t) * (1.0 - frond.notch);
        let shade = 1.0 - 0.1 * segment as f32 / frond.segments as f32;
        let colors = (frond.colors.0 * shade, frond.colors.1);
        let middle = t0 + span * 0.5;
        builder.polygon(
            &[
                at(t0, inner(t0)),
                at(t0, -inner(t0)),
                at(t1, -inner(t1)),
                at(t1, inner(t1)),
            ],
            up(middle),
            colors,
        );
        // Each leaflet's foot along part of the segment, its tip swept on towards the frond's, apart from the next, so
        // that the sky shows between them; the other side's half a leaflet on.
        for (sign, (from, to, tip)) in [(1.0, (0.0, 0.62, 0.9)), (-1.0, (0.38, 1.0, 1.25))] {
            let [from, to, tip] = [from, to, tip].map(|along: f32| (t0 + span * along).min(1.0));
            if frond.half_width(tip) < 0.004 || to <= from {
                continue;
            }
            builder.polygon(
                &[
                    at(from, inner(from) * sign),
                    at(to, inner(to) * sign),
                    at(tip, frond.half_width(tip) * sign),
                ],
                up(tip),
                colors,
            );
        }
    }
}

/// What a tree fern is: its trunk, from its foot to its crown, its foot's and top's radii and colour, how far it leans;
/// its fronds, how many, how long, how steeply they rise, the young and the old, how far they droop, how wide they are,
/// in how many segments and how deeply notched, their colours and their stalks'; and its skirt of dead fronds, how
/// many and how long.
struct Tree {
    crown: f32,
    radii: (f32, f32),
    trunk: Vec3,
    lean: Vec2,
    fronds: usize,
    length: f32,
    pitch: (f32, f32),
    droop: (f32, f32),
    width: f32,
    segments: usize,
    notch: f32,
    colors: (Vec3, Vec3),
    stalk: Vec3,
    skirt: usize,
    skirt_length: f32,
}

/// How far a tree fern's point at `position` sways, its fronds `span` long: more the further out they reach, and nothing
/// of its trunk and little of its skirt.
fn tree_sway(position: Vec3, span: f32) -> f32 {
    let out = Vec2::new(position.x, position.z).length();
    TREE_SWAY * ((out - 0.3).max(0.0) / span).min(1.0).powf(1.5)
}

/// How far a ground fern's point at `position` sways, its fronds `span` long: more the further out from its heart and
/// the higher it reaches.
fn ground_sway(position: Vec3, span: f32) -> f32 {
    let out = Vec2::new(position.x, position.z).length();
    GROUND_SWAY * (out / span).min(1.0).powf(1.2) * (0.5 + position.y.max(0.0)).min(1.0)
}

/// How far a koru sways: a little, more at its curl.
fn koru_sway(position: Vec3, _: f32) -> f32 {
    0.02 * (position.y / 0.2).clamp(0.0, 1.0)
}

fn tree(builder: &mut Builder, tree: &Tree, detail: Detail, seed: u32) {
    let sides = if detail == Detail::Near { 6 } else { 5 };
    builder.trunk = true;
    let top = Vec3::new(tree.lean.x, tree.crown, tree.lean.y);
    // The trunk, flaring to its matted roots, bending a little towards its lean, its faces alternately lighter and darker,
    // as fibres.
    let rings: Vec<Vec<Vec3>> = [
        (-SUNK, tree.radii.0, 0.0),
        (tree.crown * 0.5, (tree.radii.0 + tree.radii.1) * 0.5, 0.4),
        (tree.crown, tree.radii.1, 1.0),
    ]
    .iter()
    .filter(|(_, _, bend)| detail == Detail::Near || *bend != 0.4)
    .map(|(y, radius, bend)| {
        (0..sides)
            .map(|side| {
                let yaw = side as f32 / sides as f32 * TAU;
                let lean = tree.lean * *bend;
                Vec3::new(yaw.sin() * radius + lean.x, *y, yaw.cos() * radius + lean.y)
            })
            .collect()
    })
    .collect();
    for pair in rings.windows(2) {
        let axis =
            (pair[0].iter().sum::<Vec3>() + pair[1].iter().sum::<Vec3>()) / (2 * sides) as f32;
        for side in 0..sides {
            let next = (side + 1) % sides;
            let corners = [pair[0][side], pair[0][next], pair[1][next], pair[1][side]];
            let outward = corners.iter().sum::<Vec3>() / 4.0 - axis;
            let shade = if side % 2 == 0 { 0.85 } else { 1.1 };
            let color = tree.trunk * shade;
            builder.polygon(&corners, outward.with_y(0.0), (color, color));
        }
    }
    let cap = rings[rings.len() - 1].clone();
    builder.polygon(&cap, Vec3::Y, (tree.trunk, tree.trunk));
    builder.trunk = false;
    // The skirt: dead fronds hanging close about the trunk below the crown.
    let skirt = if detail == Detail::Near {
        tree.skirt
    } else {
        tree.skirt / 2
    };
    for index in 0..skirt {
        let seed = seed + 100 + index as u32 * 7;
        let yaw = (index as f32 + scatter(seed) * 0.6) / skirt as f32 * TAU;
        let out = Vec3::new(yaw.sin(), 0.0, yaw.cos()) * (tree.radii.1 + 0.02);
        frond(
            builder,
            &Frond {
                base: top + out - Vec3::Y * 0.05,
                yaw,
                pitch: -1.25 - scatter(seed + 1) * 0.2,
                length: tree.skirt_length * (0.8 + scatter(seed + 2) * 0.35),
                droop: 0.05,
                width: 0.09,
                stipe: 0.1,
                segments: 2,
                notch: 0.35,
                colors: (
                    colors::DEAD * (0.75 + scatter(seed + 3) * 0.4),
                    colors::DEAD * 0.8,
                ),
                stalk: colors::DEAD * 0.7,
            },
        );
    }
    // The crown: many long fronds, arching up and out from the trunk's top round its heart, drooping at their tips, the
    // lower, older ones darker, lower and drooping further, turned by the golden angle so that none hides another.
    let (fronds, segments) = match detail {
        Detail::Near => (tree.fronds, tree.segments),
        Detail::Far => (tree.fronds * 2 / 3, (tree.segments - 2).max(2)),
    };
    for index in 0..fronds {
        let seed = seed + 300 + index as u32 * 11;
        let old = index % 3 == 2;
        let yaw = index as f32 * 2.399_963 + (scatter(seed) - 0.5) * 0.3;
        let (pitch, droop, shade) = if old {
            (tree.pitch.0, tree.droop.1, 0.72)
        } else {
            (tree.pitch.1, tree.droop.0, 1.0)
        };
        frond(
            builder,
            &Frond {
                base: top + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * tree.radii.1 * 0.6,
                yaw,
                pitch: pitch + (scatter(seed + 1) - 0.5) * 0.2,
                length: tree.length * (0.88 + scatter(seed + 2) * 0.24),
                droop: droop * (0.9 + scatter(seed + 3) * 0.2),
                width: tree.width,
                stipe: 0.14,
                segments,
                notch: tree.notch,
                colors: (
                    tree.colors.0 * shade * (0.9 + scatter(seed + 4) * 0.2),
                    tree.colors.1,
                ),
                stalk: tree.stalk,
            },
        );
    }
}

/// What a ground fern is: how many fronds it has, how long, how steeply they rise, by how much they differ, how far they
/// droop, how wide they are, how much is stalk, in how many segments and how deeply notched, their colours above and
/// beneath, and how many of them are young, in their own colour.
struct Clump {
    fronds: usize,
    length: f32,
    pitch: f32,
    spread: f32,
    droop: f32,
    width: f32,
    stipe: f32,
    segments: usize,
    notch: f32,
    colors: (Vec3, Vec3),
    young: (usize, Vec3),
}

fn clump(builder: &mut Builder, clump: &Clump, detail: Detail, seed: u32) {
    let (fronds, segments) = match detail {
        Detail::Near => (clump.fronds, clump.segments),
        Detail::Far => (clump.fronds / 2 + 1, (clump.segments - 2).max(2)),
    };
    for index in 0..fronds {
        let seed = seed + index as u32 * 13;
        let yaw = index as f32 * 2.399_963 + (scatter(seed) - 0.5) * 0.4;
        let young = index < clump.young.0;
        let color = if young { clump.young.1 } else { clump.colors.0 };
        frond(
            builder,
            &Frond {
                base: Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 0.03,
                yaw,
                pitch: clump.pitch + (scatter(seed + 1) - 0.5) * clump.spread,
                length: clump.length * (0.8 + scatter(seed + 2) * 0.35),
                droop: clump.droop * (0.85 + scatter(seed + 3) * 0.3),
                width: clump.width,
                stipe: clump.stipe,
                segments,
                notch: clump.notch,
                colors: (color * (0.9 + scatter(seed + 4) * 0.2), clump.colors.1),
                stalk: colors::STALK,
            },
        );
    }
}

/// A koru, 0.2 m tall at its own size: a stalk rising and curling over into a tight spiral, a ribbon seen from either
/// side.
fn koru(builder: &mut Builder) {
    let (stalk, radius) = (0.11, 0.05);
    let centre = Vec3::new(radius, stalk, 0.0);
    let turns = 9;
    let point = |step: usize| {
        if step == 0 {
            return Vec3::ZERO;
        }
        let along = (step - 1) as f32 / (turns - 1) as f32;
        let angle = PI - along * 2.3 * PI;
        let reach = radius * (1.0 - 0.72 * along);
        centre + Vec3::new(angle.cos() * reach, angle.sin() * reach, 0.0)
    };
    let half = Vec3::Z * 0.014;
    for step in 0..turns {
        let (from, to) = (point(step), point(step + 1));
        let width = half * (1.0 - 0.5 * step as f32 / turns as f32);
        let up = (to - from).cross(Vec3::Z);
        builder.polygon(
            &[from + width, from - width, to - width, to + width],
            up,
            (colors::KORU, colors::KORU_UNDER),
        );
    }
}

/// What each kind is built as: a tree fern's trunk, skirt and crown, a ground fern's clump, or the koru.
enum Shape {
    Tree(Tree, u32),
    Clump(Clump, u32),
    Koru,
}

fn shape(kind: Kind) -> Shape {
    match kind {
        Kind::Ponga => Shape::Tree(
            Tree {
                crown: 3.6,
                radii: (0.2, 0.13),
                trunk: colors::PONGA_TRUNK,
                lean: Vec2::new(0.12, 0.05),
                fronds: 18,
                length: 2.35,
                pitch: (0.15, 0.5),
                droop: (0.95, 1.2),
                width: 0.4,
                segments: 6,
                notch: 0.55,
                colors: (colors::PONGA, colors::SILVER),
                stalk: colors::PONGA_STALK,
                skirt: 7,
                skirt_length: 0.95,
            },
            1000,
        ),
        Kind::Mamaku => Shape::Tree(
            Tree {
                crown: 4.6,
                radii: (0.24, 0.15),
                trunk: colors::MAMAKU_TRUNK,
                lean: Vec2::new(-0.15, 0.1),
                fronds: MOST_FRONDS,
                length: 2.9,
                pitch: (0.12, 0.45),
                droop: (1.25, 1.6),
                width: 0.46,
                segments: 6,
                notch: 0.5,
                colors: (colors::MAMAKU, colors::MAMAKU_UNDER),
                stalk: colors::MAMAKU_STALK,
                skirt: 0,
                skirt_length: 0.0,
            },
            2000,
        ),
        Kind::Wheki => Shape::Tree(
            Tree {
                crown: 2.6,
                radii: (0.19, 0.14),
                trunk: colors::WHEKI_TRUNK,
                lean: Vec2::new(0.06, -0.08),
                fronds: 14,
                length: 1.45,
                pitch: (0.25, 0.6),
                droop: (0.7, 0.85),
                width: 0.28,
                segments: 5,
                notch: 0.5,
                colors: (colors::WHEKI, colors::WHEKI_UNDER),
                stalk: colors::WHEKI_TRUNK,
                skirt: 11,
                skirt_length: 1.35,
            },
            3000,
        ),
        Kind::CrownFern => Shape::Clump(
            Clump {
                fronds: 9,
                length: 0.8,
                pitch: 1.1,
                spread: 0.35,
                droop: 0.2,
                width: 0.09,
                stipe: 0.12,
                segments: 3,
                notch: 0.6,
                colors: (colors::CROWN, colors::CROWN_UNDER),
                young: (0, colors::CROWN),
            },
            4000,
        ),
        Kind::Kiokio => Shape::Clump(
            Clump {
                fronds: 6,
                length: 0.95,
                pitch: 1.0,
                spread: 0.35,
                droop: 0.6,
                width: 0.14,
                stipe: 0.22,
                segments: 4,
                notch: 0.5,
                colors: (colors::KIOKIO, colors::KIOKIO_UNDER),
                young: (2, colors::KIOKIO_YOUNG),
            },
            5000,
        ),
        Kind::HenAndChickens => Shape::Clump(
            Clump {
                fronds: 6,
                length: 0.85,
                pitch: 0.85,
                spread: 0.3,
                droop: 0.45,
                width: 0.17,
                stipe: 0.2,
                segments: 4,
                notch: 0.65,
                colors: (colors::HEN, colors::HEN_UNDER),
                young: (0, colors::HEN),
            },
            6000,
        ),
        Kind::Koru => Shape::Koru,
    }
}

/// The mesh of `kind` drawn in `detail`, as triangles of points at its own size, standing on its origin.
pub fn mesh(kind: Kind, detail: Detail) -> Vec<Point> {
    built(kind, detail).points
}

/// The mesh of `kind` drawn in `detail`, and its extent.
fn built(kind: Kind, detail: Detail) -> Builder {
    let sway = match kind {
        Kind::Ponga | Kind::Mamaku | Kind::Wheki => tree_sway,
        Kind::Koru => koru_sway,
        _ => ground_sway,
    };
    let shape = shape(kind);
    let span = match &shape {
        Shape::Tree(tree, _) => tree.length,
        Shape::Clump(clump, _) => clump.length,
        Shape::Koru => 0.2,
    };
    let mut builder = Builder::new(sway, span);
    match shape {
        Shape::Tree(shape, seed) => tree(&mut builder, &shape, detail, seed),
        Shape::Clump(shape, seed) => clump(&mut builder, &shape, detail, seed),
        Shape::Koru => koru(&mut builder),
    }
    builder
}

impl Kind {
    /// The heart its fronds spring from, at its own size: a tree fern's trunk's top, and a ground fern's foot.
    pub fn heart(self) -> Vec3 {
        match shape(self) {
            Shape::Tree(tree, _) => Vec3::new(tree.lean.x, tree.crown, tree.lean.y),
            _ => Vec3::ZERO,
        }
    }
}

/// How a kind's mesh stands at its own size: how far its trunk reaches from its axis, and how high it stands, and how
/// far its fronds and skirt reach, from how low to how high; for a ground fern or a koru, its trunk is nothing and its
/// fronds all of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub trunk: (f32, f32),
    pub crown: (f32, f32, f32),
}

impl Kind {
    /// Its [`Extent`], drawn near.
    pub fn extent(self) -> Extent {
        static EXTENTS: OnceLock<Vec<(Kind, Extent)>> = OnceLock::new();
        let extents = EXTENTS.get_or_init(|| {
            Kind::TREES
                .iter()
                .chain(&Kind::GROUND)
                .chain(&[Kind::Koru])
                .map(|kind| (*kind, built(*kind, Detail::Near).extent))
                .collect()
        });
        extents
            .iter()
            .find(|(kind, _)| *kind == self)
            .map(|(_, extent)| *extent)
            .expect("every kind has an extent")
    }
}

/// A fern: its kind, where its foot stands, how it is turned about the vertical, how large it is, and where in the
/// breeze's sway it is, so that no two sway together.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fern {
    pub kind: Kind,
    pub at: Vec3,
    pub turn: f32,
    pub scale: f32,
    pub phase: f32,
}

impl Fern {
    /// Where its mesh is drawn.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(self.scale),
            Quat::from_rotation_y(self.turn),
            self.at,
        )
    }

    /// How far it reaches on the ground, and how tall it stands.
    pub fn size(&self) -> (f32, f32) {
        let extent = self.kind.extent();
        (extent.crown.0 * self.scale, extent.crown.2 * self.scale)
    }

    /// The boxes it stands within: a tree fern's trunk, with its skirt, from its foot to its crown, and its crown, from
    /// its fronds' lowest tips to its top; a ground fern's whole clump.
    pub fn solids(&self) -> Vec<Solid> {
        let extent = self.kind.extent();
        let scaled = |reach: f32, low: f32, high: f32| {
            let reach = reach * self.scale;
            Solid::new(
                (self.at + Vec3::new(-reach, low * self.scale, -reach)).to_array(),
                (self.at + Vec3::new(reach, high * self.scale, reach)).to_array(),
            )
        };
        let (reach, low, high) = extent.crown;
        let crown = scaled(reach, low.max(0.0), high);
        if self.kind.is_tree() {
            vec![scaled(extent.trunk.0, -SUNK, extent.trunk.1), crown]
        } else {
            vec![crown]
        }
    }

    /// Where its heart is, where it stands: a tree fern's trunk's top, and a ground fern's foot.
    pub fn heart(&self) -> Vec3 {
        self.transform().transform_point3(self.kind.heart())
    }

    /// How finely it is drawn for eyes at `eyes`: in full within [`NEAR`] of its heart, and with less beyond.
    pub fn detail(&self, eyes: Vec3) -> Detail {
        Detail::seen(self.heart(), eyes)
    }
}

/// The tree ferns' solids, their trunks and crowns, which the agent, walls and panels keep clear of; the ground ferns
/// are decoration, off every walkable surface, and have none.
pub fn solids() -> Vec<Solid> {
    ferns()
        .iter()
        .filter(|fern| fern.kind.is_tree())
        .flat_map(Fern::solids)
        .collect()
}

/// How far `point` is, on the ground, from anywhere participants stand or walk outside the yard: the trail's band and its
/// edges, the way out through the gate, the clearings beside it, the campsite's clearing, the beach's shingle and the
/// waterfall rock's top; and, with the yard, from the fence.
pub fn from_standing(point: Vec2) -> f32 {
    let band = trail::from_middle(point) - trail::HALF - 0.16;
    let [inside, outside] = trail::GATEWAY;
    let gateway = Vec2::new(
        (point.x - inside.x).abs() - trail::HALF,
        (inside.y - point.y).max(point.y - outside.y),
    )
    .max(Vec2::ZERO)
    .length();
    let clearings = trail::clearings()
        .into_iter()
        .map(|(middle, radius)| point.distance(middle) - radius)
        .fold(f32::INFINITY, f32::min);
    let campsite =
        point.distance(Vec2::new(campsite::FIRE.x, campsite::FIRE.z)) - campsite::CLEARING;
    let rock = layout::WATERFALL_ROCK;
    let rock = point.distance(Vec2::new(rock.x, rock.z)) - layout::ROCK_TOP;
    [
        band,
        gateway,
        clearings,
        campsite,
        crate::beach::from_beach(point),
        rock,
        FENCE.distance(point),
    ]
    .into_iter()
    .fold(f32::INFINITY, f32::min)
}

/// How high the ground is drawn at `point`, near the trail: the meadow inside the foothills' first ring, and the
/// foothills' triangles beyond.
pub fn foot(point: Vec2) -> f32 {
    backdrop::drawn_ground(point).unwrap_or(backdrop::MEADOW)
}

/// The sightlines the ferns keep open, besides the backdrop's ([`backdrop::views`]): from the trail's viewpoints, the
/// riverbank's to the river, and the lookout's to the waterfall, its foot and the pool, as the trail's own are seen.
pub fn sights() -> Vec<([f32; 3], [f32; 3])> {
    let mut sights = backdrop::views();
    for ((_, _, at, _), towards) in trail::VIEWPOINTS.iter().zip([
        vec![layout::RIVER[6], layout::RIVER[7]],
        vec![layout::WATERFALL, layout::WATERFALL_FOOT, POOL],
    ]) {
        let (point, _, _) = trail::frame(*at);
        let eyes = [
            point.x,
            trail::HEIGHT + tomori_environment::sightlines::EYES,
            point.y,
        ];
        sights.extend(towards.into_iter().map(|to| (eyes, to.to_array())));
    }
    sights
}

/// The clusters of workers at the trail's and the campsite's anchors, as the server lays them out: each anchor's point,
/// facing what it faces, in its room.
pub fn clusters() -> Vec<tomori_protocol::cluster::Cluster> {
    use tomori_protocol::cluster::{Anchor, Cluster};
    let room = |place: &trail::Place, [width, height, depth]: [f32; 3]| {
        let pose =
            tomori_protocol::Pose::at(place.at.to_array()).facing_towards(place.towards.to_array());
        Cluster::new(&Anchor::new(place.name, pose).with_extent(width, height, depth))
    };
    trail::places()
        .iter()
        .map(|place| room(place, trail::ROOM))
        .chain([room(&campsite::place(), campsite::ROOM)])
        .collect()
}

/// The box within which a solid may stand in the way of `cluster`'s workers, one to eighteen of them: about every place
/// each takes, clear by [`tomori_environment::sightlines::CLEAR`], and the eyes of whoever watches them; a solid beyond
/// it neither stands within their clearance nor between them and those eyes.
fn room(cluster: &tomori_protocol::cluster::Cluster) -> Solid {
    use tomori_environment::sightlines::{CLEAR, MEMBERS};
    let eyes = Vec3::from_array(cluster.audience());
    let (low, high) = (1..=MEMBERS)
        .flat_map(|count| cluster.slots(count))
        .map(|slot| cluster.reach(&slot))
        .fold((eyes, eyes), |(low, high), (least, most)| {
            (
                low.min(Vec3::from_array(least) - CLEAR),
                high.max(Vec3::from_array(most) + CLEAR),
            )
        });
    Solid::new(low.to_array(), high.to_array())
}

/// What a fern placed keeps apart from or clear of: what already stands there, the backdrop's plants, the pines, the
/// river's stones, and the solids of the trail and the campsite, each as its footprint's middle and reach on the
/// ground; the sightlines kept open; and the clusters of workers at the anchors, by their points.
struct Context {
    there: Vec<(Vec2, f32)>,
    sights: Vec<([f32; 3], [f32; 3])>,
    clusters: Vec<(Vec3, tomori_protocol::cluster::Cluster, Solid)>,
    /// Where anyone stands along the trail, and which of the ways round them the agent's spot beside them is taken by
    /// what stands there, and the tree ferns placed so far.
    standing: Vec<(Vec2, u64)>,
}

impl Context {
    fn new() -> Self {
        let flat = |at: Vec3| Vec2::new(at.x, at.z);
        let mut there: Vec<(Vec2, f32)> = backdrop::plants()
            .iter()
            .map(|plant| (flat(plant.at), plant.size().0))
            .collect();
        there.extend(
            crate::props::pines()
                .into_iter()
                .map(|(base, height)| (flat(base), height * 0.32)),
        );
        there.extend(
            backdrop::stones()
                .iter()
                .map(|stone| (flat(stone.at), stone.size.x * 0.5)),
        );
        there.extend(
            trail::solids()
                .into_iter()
                .chain(campsite::solids())
                .map(|(low, high)| {
                    let (low, high) = (Vec2::new(low[0], low[2]), Vec2::new(high[0], high[2]));
                    ((low + high) * 0.5, (high - low).length() * 0.5)
                }),
        );
        let kept = kept();
        let standing = along_the_trail()
            .into_iter()
            .map(|at| (at, taken(at, &kept)))
            .collect();
        Self {
            there,
            standing,
            sights: sights(),
            clusters: trail::places()
                .into_iter()
                .chain([campsite::place()])
                .map(|place| place.at)
                .zip(clusters())
                .map(|(at, cluster)| {
                    let room = room(&cluster);
                    (at, cluster, room)
                })
                .collect(),
        }
    }
}

/// What the agent keeps clear of beside the trail but the ferns: the yard's solids and the trail's, the backdrop's
/// plants and the pines.
fn kept() -> Vec<Solid> {
    let mut kept: Vec<Solid> = layout::solids()
        .into_iter()
        .chain(trail::solids())
        .map(|(low, high)| Solid::new(low, high))
        .collect();
    kept.extend(backdrop::plants().iter().map(backdrop::Plant::solid));
    kept.extend(crate::props::pines().into_iter().map(|(base, height)| {
        let reach = height * 0.32;
        Solid::new(
            (base - Vec3::new(reach, 0.0, reach)).to_array(),
            (base + Vec3::new(reach, height, reach)).to_array(),
        )
    }));
    kept
}

/// Where anyone may stand along the trail: across its band, every quarter of a step along it, and about the edge of each
/// clearing beside it.
pub fn along_the_trail() -> Vec<Vec2> {
    let mut standing = Vec::new();
    for index in 0..trail::centreline().len() {
        let (from, _, outward) = trail::frame(index);
        let (to, _, _) = trail::frame(index + 1);
        for step in 0..4 {
            let at = from.lerp(to, step as f32 / 4.0);
            standing.extend([-0.5, 0.0, 0.5].map(|across| at + outward * across));
        }
    }
    for (middle, radius) in trail::clearings() {
        standing.extend((0..8).map(|step| {
            let yaw = step as f32 / 8.0 * TAU;
            middle + Vec2::new(yaw.sin(), yaw.cos()) * (radius - 0.05)
        }));
    }
    standing
}

/// How many ways round someone the agent may come to them from, and how many of them must stay clear of the tree
/// ferns, unless fewer were before them.
pub const WAYS: u32 = 36;
const CLEAR_WAYS: u32 = 20;

/// Which of [`WAYS`] ways round someone standing at `at` on the trail the agent's spot beside them meets one of
/// `solids`, as bits: its middle 2 m from their eyes, 0.1 m above them, facing them, its box its swarm's reach and
/// 0.1 m more on every side, as the server's `follow` stands it, seen from their eyes.
pub fn taken(at: Vec2, solids: &[Solid]) -> u64 {
    use tomori_environment::sightlines::EYES;
    use tomori_protocol::entourage::SWARM;
    let eyes = Vec3::new(at.x, trail::HEIGHT + EYES, at.y);
    let half = Vec3::from_array(SWARM.map(|reach| reach / 2.0 + 0.1));
    let near: Vec<&Solid> = solids
        .iter()
        .filter(|solid| {
            Footprint::new([solid.low[0], solid.low[2]], [solid.high[0], solid.high[2]])
                .distance(at)
                < 3.5
        })
        .collect();
    let mut taken = 0;
    if near.is_empty() {
        return taken;
    }
    for way in 0..WAYS {
        let yaw = way as f32 / WAYS as f32 * TAU;
        let spot = at + Vec2::new(yaw.sin(), -yaw.cos()) * 2.0;
        let middle = Vec3::new(spot.x, eyes.y + 0.1, spot.y);
        let ahead = ((eyes - middle) * Vec3::new(1.0, 0.0, 1.0)).normalize();
        let across = Vec3::new(ahead.z, 0.0, -ahead.x);
        let corners: [Vec3; 8] = std::array::from_fn(|index| {
            let side = |bit: usize| if index & bit == 0 { -1.0 } else { 1.0 };
            middle
                + across * half.x * side(1)
                + Vec3::Y * half.y * side(2)
                + ahead * half.z * side(4)
        });
        // Its box's twelve edges, its four diagonals and the sight of it from their eyes, all within the box about them,
        // which a solid that meets any of them overlaps.
        let edges = [
            (0, 1),
            (0, 2),
            (0, 4),
            (1, 3),
            (1, 5),
            (2, 3),
            (2, 6),
            (3, 7),
            (4, 5),
            (4, 6),
            (5, 7),
            (6, 7),
        ];
        let lines: [(Vec3, Vec3); 17] = std::array::from_fn(|index| match index {
            0..12 => (corners[edges[index].0], corners[edges[index].1]),
            12..16 => (corners[index - 12], corners[7 - (index - 12)]),
            _ => (eyes, middle),
        });
        let (low, high) = corners.iter().fold((eyes, eyes), |(low, high), corner| {
            (low.min(*corner), high.max(*corner))
        });
        let meets = |solid: &&Solid| {
            let overlaps =
                (0..3).all(|axis| solid.low[axis] <= high[axis] && low[axis] <= solid.high[axis]);
            overlaps
                && (solid.contains(middle.to_array())
                    || lines
                        .iter()
                        .any(|(from, to)| solid.blocks(from.to_array(), to.to_array())))
        };
        if near.iter().any(meets) {
            taken |= 1 << way;
        }
    }
    taken
}

/// How far `point` is along the waterfall's way from the backdrop's middle, and how far across it.
fn by_waterfall(point: Vec2) -> (f32, f32) {
    let offset = point - layout::BACKDROP_CENTRE;
    let azimuth = layout::WATERFALL_AZIMUTH;
    let outward = Vec2::new(azimuth.sin(), -azimuth.cos());
    (offset.dot(outward), offset.perp_dot(outward).abs())
}

/// Whether a fern of `kind`, `scale` large, may stand at `point`, given what is already there and the ferns placed
/// before it: beyond the fence, a tree fern's crown over it by no more than half a metre and a ground fern not at all;
/// off everywhere participants stand or walk, a tree fern's crown too, and 0.4 m back, so that the agent finds room
/// beside them, but where it hangs [`OVERHEAD`]; a ground fern within
/// 5 m of where they do, to be seen; clear of the water, the ground fern on the banks' tops and the tree fern back from
/// them, of the waterfall's cliff and the knoll behind it, and of the notice board; apart from what is there and from
/// the ferns placed before it; on ground no steeper than it stands on; out of the views kept open; and, a tree fern, out
/// of the anchors' rooms, leaving the agent room to come alongside anyone on the trail from most ways round them.
fn fits(kind: Kind, scale: f32, point: Vec2, context: &Context, placed: &[Fern]) -> Option<Fern> {
    judged(kind, scale, point, context, placed).ok()
}

/// As [`fits`], saying why a fern may not stand there.
fn judged(
    kind: Kind,
    scale: f32,
    point: Vec2,
    context: &Context,
    placed: &[Fern],
) -> Result<Fern, &'static str> {
    let extent = kind.extent();
    let tree = kind.is_tree();
    let reach = extent.crown.0 * scale;
    let trunk = extent.trunk.0 * scale;
    let footing = if tree { trunk } else { reach };
    let fence = FENCE.distance(point);
    let fenced = if tree {
        fence < (reach - 0.5).max(trunk + 0.4)
    } else {
        fence < reach + 0.1
    };
    let standing = from_standing(point);
    if fenced || standing < footing + 0.15 || (!tree && standing > 5.0) {
        return Err("fence or standing");
    }
    let river = layout::nearest_river(point).1;
    let pool = point.distance(Vec2::new(POOL.x, POOL.z));
    let (bank, rim) = if tree {
        (BANKS[1] + 0.25, RIM[1] + 0.3)
    } else {
        (BANKS[1] + 0.3 + reach * 0.4, RIM[1] + 0.4 + reach * 0.4)
    };
    let (along, across) = by_waterfall(point);
    let cliff =
        across < 6.0 + reach && along > layout::CLIFF[0] - 0.5 && along < layout::CLIFF[1] + 6.0;
    let board = layout::noticeboard();
    let by_board = board.distance(point) < reach + 1.0;
    if river < bank || pool < rim || cliff || by_board {
        return Err("water, cliff or board");
    }
    let near = if tree { 0.4 } else { 0.5 };
    if context
        .there
        .iter()
        .any(|(other, theirs)| point.distance(*other) < (reach + theirs) * near + 0.1)
    {
        return Err("what is there");
    }
    let crowded = placed.iter().any(|fern| {
        let apart = point.distance(Vec2::new(fern.at.x, fern.at.z));
        let theirs = fern.size().0;
        match (tree, fern.kind.is_tree()) {
            (true, true) => apart < (reach + theirs) * 0.5,
            (false, false) => apart < (reach + theirs) * 0.62,
            (true, false) => apart < trunk + theirs * 0.7,
            (false, true) => apart < fern.kind.extent().trunk.0 * fern.scale + reach * 0.7,
        }
    });
    if crowded {
        return Err("crowded");
    }
    // Its foot a little into the ground, so that it never floats.
    let ground = foot(point);
    let at = Vec3::new(point.x, ground - 0.02, point.y);
    if tree && standing < reach + 0.4 && at.y + extent.crown.1 * scale < OVERHEAD {
        return Err("overhead");
    }
    let steep = [Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y]
        .iter()
        .any(|way| (foot(point + *way * 0.4) - ground).abs() > if tree { 0.35 } else { 0.25 });
    if steep {
        return Err("steep");
    }
    let fern = Fern {
        kind,
        at,
        turn: 0.0,
        scale,
        phase: 0.0,
    };
    let solids = fern.solids();
    if context
        .sights
        .iter()
        .any(|(from, to)| solids.iter().any(|solid| solid.blocks(*from, *to)))
    {
        return Err("sights");
    }
    // Out of the anchors' rooms, for those it reaches into.
    let overlaps = |solid: &Solid, room: &Solid| {
        (0..3).all(|axis| solid.low[axis] <= room.high[axis] && room.low[axis] <= solid.high[axis])
    };
    let crowds = |(_, cluster, room): &(Vec3, tomori_protocol::cluster::Cluster, Solid)| {
        solids.iter().any(|solid| overlaps(solid, room))
            && tomori_environment::sightlines::fits(cluster, trail::HEIGHT, &solids).is_err()
    };
    if tree && context.clusters.iter().any(crowds) {
        return Err("rooms");
    }
    // Clear of every anchor's point by 0.4 m on the ground, or a metre below it, as the trail's posts and seat are.
    let by_anchor = context.clusters.iter().any(|(at, ..)| {
        let flat = Vec2::new(at.x, at.z);
        solids.iter().any(|solid| {
            Footprint::new([solid.low[0], solid.low[2]], [solid.high[0], solid.high[2]])
                .distance(flat)
                < 0.45
                && at.y < solid.high[1] + 1.0
        })
    });
    if tree && by_anchor {
        return Err("anchor");
    }
    let clear = |taken: u64| WAYS - taken.count_ones();
    if tree
        && context
            .standing
            .iter()
            .filter(|(at, _)| at.distance(point) < reach + 3.5)
            .any(|(at, before)| {
                clear(before | taken(*at, &solids)) < CLEAR_WAYS.min(clear(*before))
            })
    {
        return Err("agent");
    }
    Ok(fern)
}

/// How many tree ferns and ground ferns there are, at most, and how many tries are made for each.
pub const TREE_FERNS: usize = 30;
pub const GROUND_FERNS: usize = 100;
const TRIES: u32 = 60;

/// The way along the ground at `angle`, a yaw from +Z towards +X.
fn way(angle: f32) -> Vec2 {
    Vec2::new(angle.sin(), angle.cos())
}

/// How much `point` is where ferns grow, from 0 to 1: shady places, along the bush's edge, by the river, about the pool
/// and the campsite, and by the fence on the yard's sides, most; the open front, beside the gate, least, and no tree
/// ferns there but at its corners.
fn shady(point: Vec2, tree: bool) -> f32 {
    let fire = Vec2::new(campsite::FIRE.x, campsite::FIRE.z);
    if point.distance(fire) < 5.0 || point.distance(Vec2::new(POOL.x, POOL.z)) < RIM[3] + 3.0 {
        1.0
    } else if point.x < -7.0 || point.y < -11.5 {
        0.95
    } else if point.x > 7.0 && point.y < 1.5 {
        0.85
    } else if point.y > 2.0 {
        match (point.x.abs() > 5.5, tree) {
            (true, _) => 0.35,
            (false, true) => 0.0,
            (false, false) => 0.1,
        }
    } else {
        0.6
    }
}

/// Where a fern might stand, the `index`th tried: beside the trail, either side of it; about the campsite; on the
/// river's banks, mostly the yard's side; and, a tree fern, on the pool's; and a ground fern, a third of the time,
/// beside one already placed, so that they grow in drifts.
fn candidate(tree: bool, index: u32, placed: &[Fern]) -> Vec2 {
    let seed = 31_000 + index * 5 + u32::from(tree) * 70_001;
    let (first, second, third, fourth) = (
        scatter(seed),
        scatter(seed + 1),
        scatter(seed + 2),
        scatter(seed + 3),
    );
    let ground: Vec<&Fern> = placed.iter().filter(|fern| !fern.kind.is_tree()).collect();
    if !tree && fourth < 0.35 && !ground.is_empty() {
        let beside = ground[(first * ground.len() as f32) as usize % ground.len()];
        return Vec2::new(beside.at.x, beside.at.z) + way(second * TAU) * (0.55 + third * 0.6);
    }
    let fire = Vec2::new(campsite::FIRE.x, campsite::FIRE.z);
    match (fourth * 10.0) as u32 {
        0..=5 => {
            let line = trail::centreline();
            let along = first * line.len() as f32;
            let (point, _, outward) = trail::frame(along as usize);
            let (next, _, _) = trail::frame(along as usize + 1);
            let point = point.lerp(next, along.fract());
            let side = if second < 0.6 { 1.0 } else { -1.0 };
            let distance = trail::HALF
                + if tree {
                    1.0 + third * 3.8
                } else {
                    0.35 + third * 2.4
                };
            point + outward * side * distance
        }
        6..=7 => {
            let reach = if tree { 3.5 } else { 1.8 };
            fire + way(first * TAU) * (campsite::CLEARING + 0.3 + second * reach)
        }
        8 if tree => Vec2::new(POOL.x, POOL.z) + way(first * TAU) * (RIM[1] + 0.3 + second * 4.0),
        _ => {
            // Along the river from the pool's lip to beside the yard, on its banks' tops, mostly the yard's side.
            let line = layout::river_line(layout::RIVER_STEPS);
            let reach = (line.len() as f32 * 0.6) as usize;
            let index = 1 + (first * reach as f32) as usize;
            let (point, next) = (line[index], line[index + 1]);
            let across = Vec2::new(next.x - point.x, next.z - point.z)
                .normalize()
                .perp();
            let towards = Vec2::new(point.x, point.z) - layout::BACKDROP_CENTRE;
            let side = across.dot(towards).signum() * if second < 0.75 { -1.0 } else { 1.0 };
            Vec2::new(point.x, point.z) + across * side * (BANKS[1] + 0.3 + third * 2.0)
        }
    }
}

/// Which kinds a fern at `point` may be, the first the likeliest: tree ferns mamaku by the water, wheki by the fence
/// and ponga everywhere else, each a little of the time anywhere; ground ferns hen-and-chickens in the shade about the
/// pool, the river and the campsite, kiokio on the banks and the trail's edges, and crown fern elsewhere.
fn kinds(tree: bool, point: Vec2, seed: u32) -> [Kind; 3] {
    let chance = scatter(seed);
    let water = layout::nearest_river(point)
        .1
        .min(point.distance(Vec2::new(POOL.x, POOL.z)) - layout::POOL_RADIUS);
    if tree {
        let first = if FENCE.distance(point) < 3.0 {
            Kind::Wheki
        } else if water < 7.0 && chance < 0.6 {
            Kind::Mamaku
        } else if chance < 0.2 {
            Kind::Wheki
        } else {
            Kind::Ponga
        };
        match first {
            Kind::Mamaku => [Kind::Mamaku, Kind::Ponga, Kind::Wheki],
            Kind::Ponga => [Kind::Ponga, Kind::Mamaku, Kind::Wheki],
            _ => [Kind::Wheki, Kind::Ponga, Kind::Mamaku],
        }
    } else {
        let fire = Vec2::new(campsite::FIRE.x, campsite::FIRE.z);
        let first = if (point.distance(fire) < 4.5 || water < 5.0) && chance < 0.55 {
            Kind::HenAndChickens
        } else if (water < 6.0 || trail::from_middle(point) < 2.0) && chance < 0.6 {
            Kind::Kiokio
        } else if chance < 0.3 {
            Kind::HenAndChickens
        } else {
            Kind::CrownFern
        };
        match first {
            Kind::HenAndChickens => [Kind::HenAndChickens, Kind::Kiokio, Kind::CrownFern],
            Kind::Kiokio => [Kind::Kiokio, Kind::CrownFern, Kind::HenAndChickens],
            _ => [Kind::CrownFern, Kind::HenAndChickens, Kind::Kiokio],
        }
    }
}

/// Each kind of fern: how many there are at most, so that each is seen among the others, and how large each grows,
/// from the least to the most.
pub const KINDS: [(Kind, usize, [f32; 2]); 6] = [
    (Kind::Ponga, 13, [0.8, 1.2]),
    (Kind::Mamaku, 8, [0.85, 1.2]),
    (Kind::Wheki, 10, [0.8, 1.15]),
    (Kind::CrownFern, 45, [0.75, 1.35]),
    (Kind::Kiokio, 30, [0.7, 1.15]),
    (Kind::HenAndChickens, 35, [0.75, 1.3]),
];

/// How large a koru is at the heart of a tree fern and of a ground fern, beside the fern's own size.
pub const KORU: (f32, f32) = (2.5, 0.8);

/// The ferns, the same every time: tree ferns, then ground ferns, each tried where `candidate` says, kept where it is
/// shady enough and `fits`, the kind that grows there or the next that fits, of those not yet at their most
/// ([`KINDS`]), turned, sized and set swaying from its
/// place; and koru unfurling at the heart of every other tree fern and a few ground ferns.
pub fn ferns() -> Vec<Fern> {
    static FERNS: OnceLock<Vec<Fern>> = OnceLock::new();
    FERNS.get_or_init(placed).clone()
}

fn placed() -> Vec<Fern> {
    let mut context = Context::new();
    let mut placed: Vec<Fern> = Vec::new();
    for (tree, count) in [(true, TREE_FERNS), (false, GROUND_FERNS)] {
        let mut found = 0;
        let mut index = 0;
        while found < count && index < count as u32 * TRIES {
            let point = candidate(tree, index, &placed);
            let seed = 47_000 + index * 9 + u32::from(tree) * 50_003;
            index += 1;
            if scatter(seed) > shady(point, tree) {
                continue;
            }
            let fern = kinds(tree, point, seed + 1).into_iter().find_map(|kind| {
                let (_, most, [least, largest]) = KINDS
                    .into_iter()
                    .find(|(of, ..)| *of == kind)
                    .expect("every kind placed is among the kinds");
                if placed.iter().filter(|fern| fern.kind == kind).count() >= most {
                    return None;
                }
                let scale = least + (largest - least) * scatter(seed + 2);
                fits(kind, scale, point, &context, &placed)
            });
            if let Some(fern) = fern {
                if fern.kind.is_tree() {
                    let solids = fern.solids();
                    for (at, before) in &mut context.standing {
                        *before |= taken(*at, &solids);
                    }
                }
                placed.push(Fern {
                    turn: scatter(seed + 3) * TAU,
                    phase: scatter(seed + 4) * TAU,
                    ..fern
                });
                found += 1;
            }
        }
    }
    let koru: Vec<Fern> = placed
        .iter()
        .enumerate()
        .flat_map(|(index, fern)| {
            // Two at the heart of every other tree fern, among its fronds' bases, and one at a few ground ferns'.
            let count = match fern.kind.is_tree() {
                true if index % 2 == 0 => 2,
                false if scatter(index as u32 + 52_000) < 0.12 => 1,
                _ => 0,
            };
            (0..count).map(move |koru| {
                let turn = fern.turn + 1.3 + koru as f32 * 2.4;
                let beside = Vec3::new(turn.sin(), 0.0, turn.cos()) * 0.1 * koru as f32;
                Fern {
                    kind: Kind::Koru,
                    at: fern
                        .transform()
                        .transform_point3(fern.kind.heart() + beside),
                    turn,
                    scale: fern.scale * if fern.kind.is_tree() { KORU.0 } else { KORU.1 },
                    phase: fern.phase,
                }
            })
        })
        .collect();
    placed.extend(koru);
    placed
}

/// The bush's tree ferns ([`backdrop::Kind::Fern`]), placed with the backdrop's plants, drawn as ponga, or, by the water
/// half the time, mamaku, each reaching as far as the plant does, so that they read as the ferns near the trail do,
/// rather than as palms, and sway with them.
pub fn bush() -> Vec<Fern> {
    let (reach, _) = backdrop::Kind::Fern.size();
    backdrop::plants()
        .iter()
        .enumerate()
        .filter(|(_, plant)| plant.kind == backdrop::Kind::Fern)
        .map(|(index, plant)| {
            let at = Vec2::new(plant.at.x, plant.at.z);
            let wet = layout::nearest_river(at)
                .1
                .min(at.distance(Vec2::new(POOL.x, POOL.z)) - layout::POOL_RADIUS)
                < 8.0;
            let kind = if wet && scatter(index as u32 + 56_000) < 0.5 {
                Kind::Mamaku
            } else {
                Kind::Ponga
            };
            Fern {
                kind,
                at: plant.at,
                turn: plant.turn,
                scale: plant.scale * reach / kind.extent().crown.0,
                phase: scatter(index as u32 + 57_000) * TAU,
            }
        })
        .collect()
}

/// Every fern drawn: those near the trail, with their koru, the bush's and the upper terrace's
/// ([`crate::terrace::ferns`]).
pub fn all() -> Vec<Fern> {
    ferns()
        .into_iter()
        .chain(bush())
        .chain(crate::terrace::ferns())
        .collect()
}

/// How many triangles the ferns draw for eyes at `eyes`, [`all`] of them, each in the detail it is drawn in there; or,
/// with none, as though every one were near.
pub fn triangles(eyes: Option<Vec3>) -> usize {
    static TRIANGLES: OnceLock<Vec<((Kind, Detail), usize)>> = OnceLock::new();
    let meshes = TRIANGLES.get_or_init(|| {
        drawn()
            .into_iter()
            .map(|(kind, detail)| ((kind, detail), mesh(kind, detail).len() / 3))
            .collect()
    });
    all()
        .iter()
        .filter_map(|fern| {
            let detail = eyes.map_or(Detail::Near, |eyes| fern.detail(eyes));
            meshes
                .iter()
                .find(|(drawn, _)| *drawn == (fern.kind, detail))
                .map(|(_, triangles)| *triangles)
        })
        .sum()
}

#[cfg(test)]
mod tests;
