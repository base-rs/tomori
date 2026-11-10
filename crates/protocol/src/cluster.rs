// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How whatever stands at one of an environment's cluster anchors gathers about it: the one rule the server and every
//! client lay a cluster out by, for workers, floating panels and the agent alike, each by the footprint it draws. An
//! anchor is a named place, a pose whose point a single member stands at, facing along its +Z towards whoever watches
//! it, and optionally an extent, the room its cluster may take, a box centred on the point. Its members stand in
//! **slots**, in order of arrival, which they keep while they stay, so that nothing moves when another arrives, leaves
//! or changes, but for the gaps [`Cluster::reslot`] closes where that moves nobody who would not move anyway; the
//! cluster's [`Arrangement`] is the first of its steps that holds every slot taken:
//!
//! 1. at `home` without an extent, today's arc about the eyes at the arrival ([`home_arc`]), which holds any number;
//!    at `home` with an extent, that arc while every slot taken fits the extent;
//! 2. one member at the point, then an **arc** through it, curving about the anchor's audience, alternately right and
//!    left of the point, a footprint apart, as many as fit across the extent's width, then rows above and below it, as
//!    its height allows;
//! 3. where the arc does not hold them, as in an extent deeper than it is wide, a **grid** in the extent's face,
//!    columns across and rows up and down, then further back along its depth;
//! 4. then the same, **compact**: footprints at [`COMPACT`] of their size, the arc, or else the grid;
//! 5. and beyond what the compact step that holds more holds, the members from its last slot on gather into one
//!    **"+N more"** sign there.
//!
//! At `home`, the point is the agent's own, kept for it while it is away, so that workers stand either side of it. The
//! steps depend only on the anchor, its audience and the highest slot taken, so every client and the server lay a
//! cluster out alike, and a cluster lays out again only as that slot crosses what a step holds. See
//! `design/environments.md`.
//!
//! ```
//! use tomori_protocol::Pose;
//! use tomori_protocol::cluster::{Anchor, Cluster, Extent, Step};
//!
//! // A home anchor of room 6 m wide, 2.5 m tall and 2 m deep, 4 m ahead of the arrival, facing it.
//! let pose = Pose::at([0.0, 1.4, -4.0]).facing_towards([0.0, 1.6, 0.0]);
//! let home = Anchor::new("home", pose).with_extent(6.0, 2.5, 2.0);
//! let cluster = Cluster::new(&home);
//! // Three workers stand on an arc through it, either side of the agent's point.
//! let three = cluster.arrange(3);
//! assert_eq!(three.step(), Step::Arc);
//! assert!(!three.compact());
//! // Thirty are compacted, and those beyond what fits gather into one sign.
//! let thirty = cluster.arrange(30);
//! assert!(thirty.compact() && thirty.more().is_some());
//! ```
use serde::{Deserialize, Serialize};

use crate::Pose;

/// The name of the anchor whose cluster is the home's workers' arrangement.
pub const HOME: &str = "home";

/// The most bytes in an anchor's name, as in an object's reference.
pub const NAME_BYTES: usize = 64;

/// The least and the most of each of an extent's width, height and depth, in metres.
pub const LEAST_EXTENT: f32 = 0.3;
pub const MOST_EXTENT: f32 = 12.0;

/// How large footprints are drawn, and stand apart, in a compact cluster, as a share of their full size.
pub const COMPACT: f32 = 0.7;

/// How far before a named anchor its audience stands, along its +Z, in metres: as far as the agent's default home
/// stands from home.
pub const AUDIENCE: f32 = 2.3;

/// Where the eyes are, at home, of participants arriving: 1.6 m above home, whom the home cluster gathers about.
pub const EYES: [f32; 3] = [0.0, 1.6, 0.0];

/// How far to either side of the point at `home` stays the agent's, in metres, from the swarm's middle: as far as its
/// agents gather about it at their widest.
pub const KEPT: f32 = 0.85;

/// The room a named anchor's cluster takes when it gives none: 1.5 m wide, 2 m tall and 1 m deep.
pub const ROOM: Extent = Extent {
    width: 1.5,
    height: 2.0,
    depth: 1.0,
};

/// The most slots an arrangement lays out: as many as a client keeps for workers.
pub const PLACES: usize = 192;

/// How far from whoever watches today's arc stands at which its columns stand at their own angles, 40° from the agent
/// and 18⅔° apart, and its rows 85 cm apart, in metres; nearer, its columns stand as far apart along the arc as they do
/// here, so that their signs keep clear of one another and of the swarm, and further, its columns stand at those
/// angles, its rows spread in proportion, as its signs are drawn larger there.
pub const ARC_RADIUS: f32 = 3.2;

/// How near whoever watches today's arc stands at least, horizontally, in metres: as near as the agent's default home
/// stands, so that its first columns stand in front of them.
pub const NEAREST_ARC: f32 = crate::entourage::SUMMONED;

/// How far below the agent's middle today's arc's first row stands, and how far apart its rows stand, in metres, at
/// [`ARC_RADIUS`] or nearer: the first row a little below the agent's middle, the second above it, so that the agent
/// stands within the band they make, then each row above the last.
pub const ROW_BELOW: f32 = 0.25;
pub const ROW_APART: f32 = 0.85;

/// The lowest any member of today's arc stands above the floor, in metres.
pub const LOWEST: f32 = 0.3;

/// How many columns today's arc has on each side of the agent.
pub const COLUMNS: usize = 8;

/// The room an anchor's cluster may take, a box centred on its point, in its own frame: across, its X; up; and along
/// the way it faces, its Z; in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extent {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
}

impl Extent {
    /// `width` across, `height` up and `depth` along the way it faces.
    pub const fn new(width: f32, height: f32, depth: f32) -> Self {
        Self {
            width,
            height,
            depth,
        }
    }

    /// Whether each of its sides is finite and from [`LEAST_EXTENT`] to [`MOST_EXTENT`].
    pub fn valid(&self) -> bool {
        [self.width, self.height, self.depth]
            .iter()
            .all(|side| (LEAST_EXTENT..=MOST_EXTENT).contains(side))
    }
}

/// A named place in an environment that workers, panels and the agent gather at: its name, its pose, whose point a
/// single member stands at, facing along its +Z towards whoever watches it, and the room its cluster may take, if it
/// says. In an environment's registration, its pose is in the environment's own coordinates; in the space's state,
/// carried into the space's. See `design/environments.md`.
///
/// ```
/// use tomori_protocol::Pose;
/// use tomori_protocol::cluster::{Anchor, Extent};
///
/// let aisle = Anchor::new("aisle-3", Pose::at([4.0, 1.4, -6.0])).with_extent(4.0, 2.5, 1.5);
/// assert_eq!(aisle.extent, Some(Extent::new(4.0, 2.5, 1.5)));
/// assert_eq!(aisle.checked(), Ok(()));
/// assert!(!aisle.is_home());
/// assert!(Anchor::new("rack a12", Pose::HOME).checked().is_err());
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Anchor {
    /// 1 to [`NAME_BYTES`] bytes of ASCII letters, digits, `-`, `_`, `.` and `/`, such as `rack/a12`.
    pub name: String,
    pub pose: Pose,
    /// The room its cluster may take; none for today's arc at `home`, or [`ROOM`] elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extent: Option<Extent>,
}

impl Anchor {
    /// The anchor `name` at `pose`, taking the room it is given by default.
    pub fn new(name: impl Into<String>, pose: Pose) -> Self {
        Self {
            name: name.into(),
            pose,
            extent: None,
        }
    }

    /// This anchor, whose cluster may take `width` across, `height` up and `depth` along the way it faces, in metres.
    pub fn with_extent(mut self, width: f32, height: f32, depth: f32) -> Self {
        self.extent = Some(Extent::new(width, height, depth));
        self
    }

    /// Whether it is the home's: the anchor whose cluster the workers stand in by default.
    pub fn is_home(&self) -> bool {
        self.name == HOME
    }

    /// Whether its name and its extent are ones the server takes, or why not. Its pose is bounded as every anchor's is.
    pub fn checked(&self) -> Result<(), &'static str> {
        if !valid_name(&self.name) {
            return Err(
                "its name is not 1 to 64 bytes of ASCII letters, digits, '-', '_', '.' and '/'",
            );
        }
        if self.extent.is_some_and(|extent| !extent.valid()) {
            return Err("its extent is not from 0.3 to 12 m on each side");
        }
        Ok(())
    }
}

/// Whether `name` is one an anchor may have: 1 to [`NAME_BYTES`] bytes of ASCII letters, digits, `-`, `_`, `.` and `/`,
/// the rule an object's reference keeps.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

/// The box a member of a cluster takes about where it stands, as it is drawn at full size: across and along the way it
/// faces, centred on it; and up, from `base` below it, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
    pub base: f32,
}

impl Footprint {
    /// `width` across, `height` up from `base` below where it stands, and `depth` along the way it faces.
    pub const fn new(width: f32, height: f32, depth: f32, base: f32) -> Self {
        Self {
            width,
            height,
            depth,
            base,
        }
    }

    /// This footprint, drawn at `scale` of its size.
    pub fn scaled(self, scale: f32) -> Self {
        Self::new(
            self.width * scale,
            self.height * scale,
            self.depth * scale,
            self.base * scale,
        )
    }
}

/// A worker, its flock and the sign above it, a little wider than its widest sign, as today's arc's rows stand apart.
pub const WORKER: Footprint = Footprint::new(1.05, ROW_APART, 0.3, 0.15);

/// A floating panel, 0.85 m by 0.6 m, about its middle.
pub const PANEL: Footprint = Footprint::new(0.85, 0.6, 0.1, 0.3);

/// The swarm, drawn in to 0.7 m across, as near whoever it stands for.
pub const AGENT: Footprint = Footprint::new(0.7, 0.52, 0.42, 0.26);

/// Which of a cluster's steps lays it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Today's arc about the eyes at the arrival, at `home`.
    Home,
    /// An arc through the point, curving about the anchor's audience.
    Arc,
    /// A grid in the extent's face, then further back along its depth.
    Grid,
}

/// Where one member of a cluster stands: in the space, or the frame the anchor is given in; how large it is drawn, as a
/// share of its full size; and whether it stands in the "+N more" sign rather than in a slot of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub position: [f32; 3],
    pub scale: f32,
    pub more: bool,
}

/// How a cluster's members stand: which step lays it out, at which size, its slots, and the slot from which members
/// gather into the "+N more" sign, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    step: Step,
    scale: f32,
    places: Vec<[f32; 3]>,
    more: Option<usize>,
}

impl Arrangement {
    /// The step that lays it out.
    pub fn step(&self) -> Step {
        self.step
    }

    /// Whether its footprints are drawn at [`COMPACT`] of their size.
    pub fn compact(&self) -> bool {
        self.scale < 1.0
    }

    /// The slot from which members gather into the "+N more" sign, standing where that slot does, if any do.
    pub fn more(&self) -> Option<usize> {
        self.more
    }

    /// How many slots it holds, each a member's own: none for today's arc, which holds any number.
    pub fn capacity(&self) -> Option<usize> {
        match self.step {
            Step::Home => None,
            _ => Some(self.more.unwrap_or(self.places.len())),
        }
    }
}

/// An anchor's cluster: its point and the way it faces, the room it may take, the eyes of whoever watches it, the
/// footprint of its members and whether its point is kept for the agent, as at `home`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cluster {
    point: [f32; 3],
    yaw: f32,
    extent: Option<Extent>,
    audience: [f32; 3],
    footprint: Footprint,
    kept: bool,
}

/// A row's offset from the point, in rows: the point's own, then alternately above and below it.
fn row(index: usize) -> f32 {
    let step = index.div_ceil(2) as f32;
    if index % 2 == 1 { step } else { -step }
}

/// The most rows or columns on each side a step tries.
const TRIES: usize = 32;

impl Cluster {
    /// The cluster of `anchor`, where its pose is, of workers: at `home`, about the eyes at the arrival, its point kept
    /// for the agent, and taking today's arc's room unless it gives an extent; elsewhere, about its audience,
    /// [`AUDIENCE`] before it, taking [`ROOM`] unless it gives an extent.
    pub fn new(anchor: &Anchor) -> Self {
        let pose = anchor.pose;
        let yaw = pose.yaw();
        let point = pose.position;
        if anchor.is_home() {
            Self::home(pose, anchor.extent)
        } else {
            let (sin, cos) = yaw.sin_cos();
            Self {
                point,
                yaw,
                extent: Some(anchor.extent.unwrap_or(ROOM)),
                audience: [
                    point[0] + sin * AUDIENCE,
                    point[1],
                    point[2] + cos * AUDIENCE,
                ],
                footprint: WORKER,
                kept: false,
            }
        }
    }

    /// The home's cluster of workers about `pose`, gathered about the eyes at the arrival, [`EYES`], its point kept for
    /// the agent, in `extent`, or else on today's arc.
    pub fn home(pose: Pose, extent: Option<Extent>) -> Self {
        Self {
            point: pose.position,
            yaw: pose.yaw(),
            extent,
            audience: EYES,
            footprint: WORKER,
            kept: true,
        }
    }

    /// This cluster, gathered about whoever's eyes are at `audience`.
    pub fn with_audience(mut self, audience: [f32; 3]) -> Self {
        self.audience = audience;
        self
    }

    /// This cluster, of members whose footprint is `footprint`.
    pub fn with_footprint(mut self, footprint: Footprint) -> Self {
        self.footprint = footprint;
        self
    }

    /// The eyes of whoever it gathers about.
    pub fn audience(&self) -> [f32; 3] {
        self.audience
    }

    /// The room it may take, if it is bounded.
    pub fn extent(&self) -> Option<Extent> {
        self.extent
    }

    /// The footprint of its members, at full size.
    pub fn footprint(&self) -> Footprint {
        self.footprint
    }

    /// How it stands with slots up to `span` taken, the highest one less: the first of its steps that holds them all,
    /// or compact, the members from its last slot on gathered into the "+N more" sign there.
    pub fn arrange(&self, span: usize) -> Arrangement {
        let span = span.max(1);
        let Some(extent) = self.extent else {
            return Arrangement {
                step: Step::Home,
                scale: 1.0,
                places: Vec::new(),
                more: None,
            };
        };
        if self.kept {
            let fits = (0..span).all(|slot| {
                let position = home_arc(self.point, self.facing(), self.audience, slot);
                self.within(&extent, position, 1.0)
            });
            if fits {
                return Arrangement {
                    step: Step::Home,
                    scale: 1.0,
                    places: Vec::new(),
                    more: None,
                };
            }
        }
        for (step, places) in [
            (Step::Arc, self.arc(&extent, 1.0)),
            (Step::Grid, self.grid(&extent, 1.0)),
        ] {
            if span <= places.len() {
                return Arrangement {
                    step,
                    scale: 1.0,
                    places,
                    more: None,
                };
            }
        }
        let (step, mut places) = self.compacted(&extent, span);
        if places.is_empty() {
            // Room for none: one stands where the first would, with the rest in its sign.
            places.push(self.first(COMPACT));
        }
        let more = (span > places.len()).then(|| places.len() - 1);
        Arrangement {
            step,
            scale: COMPACT,
            places,
            more,
        }
    }

    /// Where the member in `slot` stands, as `arrangement`, this cluster's, lays it out.
    pub fn slot(&self, arrangement: &Arrangement, slot: usize) -> Slot {
        if arrangement.step == Step::Home {
            return Slot {
                position: home_arc(self.point, self.facing(), self.audience, slot),
                scale: 1.0,
                more: false,
            };
        }
        let last = arrangement.places.len() - 1;
        let gathered = arrangement.more.unwrap_or(last);
        let more = arrangement.more.is_some_and(|more| slot >= more) || slot > last;
        Slot {
            position: arrangement.places[if more { gathered } else { slot }],
            scale: arrangement.scale,
            more,
        }
    }

    /// Where its first `count` members stand, in slots from 0, as it is laid out for them.
    pub fn slots(&self, count: usize) -> Vec<Slot> {
        let arrangement = self.arrange(count);
        (0..count)
            .map(|slot| self.slot(&arrangement, slot))
            .collect()
    }

    /// Close the gaps among `members`, the slot each of a cluster's members takes, where that moves nobody who would not
    /// move anyway, as the cluster stands as `standing` now, none taking a slot in `held`, kept by those still leaving;
    /// and whether any member's slot changed. A member gathered into the "+N more" sign takes a slot of its own as one
    /// frees before the sign, the last gathered first, alone moving, so that nobody is gathered beside an empty slot.
    /// Where as many members in the lowest free slots would stand otherwise than it stands, it passes a step, and every
    /// member moves anyway: those beyond the lowest slots take the gaps among them, in slot order, so that gaps never
    /// keep a cluster compact, or gathered, that its members would fit at full size. Where only the slots taken would
    /// pass a step, as one arrives far along, the last take the first gaps until it stands as it stood, they alone moving.
    /// Otherwise, a gap stays until the next to arrive fills it.
    ///
    /// ```
    /// use tomori_protocol::Pose;
    /// use tomori_protocol::cluster::{Anchor, Cluster};
    ///
    /// let home = Anchor::new("home", Pose::at([0.0, 1.4, -4.0]).facing_towards([0.0, 1.6, 0.0]));
    /// let cluster = Cluster::new(&home.with_extent(4.2, 1.6, 2.0));
    /// // Thirty came and all but the last went: it stands in a slot of its own, laid out for one.
    /// let mut members = [29];
    /// assert!(cluster.reslot(&mut members, &[], &cluster.arrange(30)));
    /// assert_eq!(members, [0]);
    /// ```
    pub fn reslot(&self, members: &mut [usize], held: &[usize], standing: &Arrangement) -> bool {
        let span = |members: &[usize]| members.iter().max().map_or(0, |slot| slot + 1);
        // The free slots before `end`, in order.
        let gaps = |members: &[usize], end: usize| -> Vec<usize> {
            (0..end)
                .filter(|slot| !held.contains(slot) && !members.contains(slot))
                .collect()
        };
        // The members from slot `from` on, in the order of their slots.
        let beyond = |members: &[usize], from: usize| -> Vec<usize> {
            let mut beyond: Vec<usize> = (0..members.len())
                .filter(|index| members[*index] >= from)
                .collect();
            beyond.sort_by_key(|index| members[*index]);
            beyond
        };
        let mut changed = false;
        if let Some(more) = self.arrange(span(members)).more() {
            let open = gaps(members, more);
            for (index, slot) in beyond(members, more).into_iter().rev().zip(open) {
                members[index] = slot;
                changed = true;
            }
        }
        // The lowest slots as many members take, and how the cluster would stand in them.
        let packed = (0..)
            .filter(|slot| !held.contains(slot))
            .take(members.len())
            .last()
            .map_or(0, |slot| slot + 1);
        let open = gaps(members, packed);
        let moving = beyond(members, packed);
        if self.arrange(packed) != *standing {
            for (index, slot) in moving.into_iter().zip(open) {
                members[index] = slot;
                changed = true;
            }
        } else {
            for (index, slot) in moving.into_iter().rev().zip(open) {
                if self.arrange(span(members)) == *standing {
                    break;
                }
                members[index] = slot;
                changed = true;
            }
        }
        changed
    }

    /// The box, aligned with the axes, that the member in `slot` takes, facing whoever it gathers about.
    pub fn reach(&self, slot: &Slot) -> ([f32; 3], [f32; 3]) {
        let footprint = self.footprint.scaled(slot.scale);
        let [x, y, z] = slot.position;
        let (dx, dz) = (self.audience[0] - x, self.audience[2] - z);
        let length = dx.hypot(dz);
        let (sin, cos) = if length > 1e-6 {
            (dx / length, dz / length)
        } else {
            (0.0, 1.0)
        };
        let across = footprint.width / 2.0 * cos.abs() + footprint.depth / 2.0 * sin.abs();
        let along = footprint.width / 2.0 * sin.abs() + footprint.depth / 2.0 * cos.abs();
        (
            [x - across, y - footprint.base, z - along],
            [x + across, y - footprint.base + footprint.height, z + along],
        )
    }

    /// The way it faces, its +Z.
    fn facing(&self) -> [f32; 3] {
        let (sin, cos) = self.yaw.sin_cos();
        [sin, 0.0, cos]
    }

    /// `local`, in its own frame, in the frame it is given in.
    fn place(&self, local: [f32; 3]) -> [f32; 3] {
        let (sin, cos) = self.yaw.sin_cos();
        let [x, y, z] = local;
        let [px, py, pz] = self.point;
        [px + x * cos + z * sin, py + y, pz - x * sin + z * cos]
    }

    /// `position`, in the frame it is given in, in its own frame.
    fn unplace(&self, position: [f32; 3]) -> [f32; 3] {
        let (sin, cos) = self.yaw.sin_cos();
        let (dx, dz) = (position[0] - self.point[0], position[2] - self.point[2]);
        [
            dx * cos - dz * sin,
            position[1] - self.point[1],
            dx * sin + dz * cos,
        ]
    }

    /// Whether a member standing at `position`, drawn at `scale`, facing whoever it gathers about, stands within
    /// `extent`.
    fn within(&self, extent: &Extent, position: [f32; 3], scale: f32) -> bool {
        let footprint = self.footprint.scaled(scale);
        let [x, y, z] = self.unplace(position);
        // Turned from the cluster's facing towards whoever it gathers about, it reaches further across and along.
        let (dx, dz) = (
            self.audience[0] - position[0],
            self.audience[2] - position[2],
        );
        let turn = if dx == 0.0 && dz == 0.0 {
            0.0
        } else {
            dx.atan2(dz) - self.yaw
        };
        let (sin, cos) = (turn.sin().abs(), turn.cos().abs());
        let across = footprint.width / 2.0 * cos + footprint.depth / 2.0 * sin;
        let along = footprint.width / 2.0 * sin + footprint.depth / 2.0 * cos;
        let fits =
            |low: f32, high: f32, side: f32| low >= -side / 2.0 - 1e-4 && high <= side / 2.0 + 1e-4;
        fits(x - across, x + across, extent.width)
            && fits(z - along, z + along, extent.depth)
            && fits(
                y - footprint.base,
                y - footprint.base + footprint.height,
                extent.height,
            )
    }

    /// How far from the point, across, the first member on each side stands, at `scale`: by it, or, where the point
    /// is kept for the agent, beside what it keeps.
    fn gap(&self, scale: f32) -> f32 {
        if self.kept {
            KEPT + self.footprint.width * scale / 2.0
        } else {
            self.footprint.width * scale
        }
    }

    /// Where the first member stands, at `scale`, whether or not it fits: at the point, or beside what it keeps.
    fn first(&self, scale: f32) -> [f32; 3] {
        if self.kept {
            self.place([self.gap(scale), 0.0, 0.0])
        } else {
            self.point
        }
    }

    /// The compact arc, if it holds `span` slots, or else the compact grid, if it does, or else whichever holds more,
    /// the arc of equals, and its slots.
    fn compacted(&self, extent: &Extent, span: usize) -> (Step, Vec<[f32; 3]>) {
        let arc = self.arc(extent, COMPACT);
        if span <= arc.len() {
            return (Step::Arc, arc);
        }
        let grid = self.grid(extent, COMPACT);
        if span <= grid.len() || grid.len() > arc.len() {
            (Step::Grid, grid)
        } else {
            (Step::Arc, arc)
        }
    }

    /// The rows within `extent` at `scale`, in order, each as its height above the point.
    fn rows(&self, extent: &Extent, scale: f32) -> Vec<f32> {
        let footprint = self.footprint.scaled(scale);
        (0..TRIES)
            .map(|index| row(index) * footprint.height)
            .filter(|lift| {
                lift - footprint.base >= -extent.height / 2.0 - 1e-4
                    && lift - footprint.base + footprint.height <= extent.height / 2.0 + 1e-4
            })
            .collect()
    }

    /// The slots of the arc through the point within `extent`, at `scale`: curving about the audience, at least
    /// [`NEAREST_ARC`] before it, alternately right and left of the point, a footprint apart along it, row by row.
    fn arc(&self, extent: &Extent, scale: f32) -> Vec<[f32; 3]> {
        let footprint = self.footprint.scaled(scale);
        let (ax, az) = (
            self.point[0] - self.audience[0],
            self.point[2] - self.audience[2],
        );
        let distance = ax.hypot(az);
        let ahead = if distance > 1e-4 {
            [ax / distance, az / distance]
        } else {
            let [x, _, z] = self.facing();
            [-x, -z]
        };
        let radius = distance.max(NEAREST_ARC);
        let centre = [
            self.point[0] - ahead[0] * radius,
            self.point[2] - ahead[1] * radius,
        ];
        let right = [-ahead[1], ahead[0]];
        let angle = |chord: f32| 2.0 * (chord / (2.0 * radius)).min(1.0).asin();
        let first = angle(self.gap(scale));
        let apart = angle(footprint.width);
        // Each column's angle from the point, and its side, in order.
        let mut columns: Vec<f32> = Vec::new();
        if !self.kept {
            columns.push(0.0);
        }
        for column in 0..TRIES {
            let turn = if self.kept {
                first + column as f32 * apart
            } else {
                (column + 1) as f32 * apart
            };
            if turn >= std::f32::consts::FRAC_PI_2 {
                break;
            }
            columns.extend([turn, -turn]);
        }
        let mut places = Vec::new();
        for lift in self.rows(extent, scale) {
            for turn in &columns {
                let (sin, cos) = turn.sin_cos();
                let position = [
                    centre[0] + (ahead[0] * cos + right[0] * sin) * radius,
                    self.point[1] + lift,
                    centre[1] + (ahead[1] * cos + right[1] * sin) * radius,
                ];
                if self.within(extent, position, scale) && places.len() < PLACES {
                    places.push(position);
                }
            }
        }
        places
    }

    /// The slots of the grid in `extent`'s face, at `scale`: columns across, alternately right and left of the point,
    /// a footprint apart, rows up and down, then further back along its depth, each layer a footprint's width behind
    /// the last and half a column aside, so that it shows between those before it.
    fn grid(&self, extent: &Extent, scale: f32) -> Vec<[f32; 3]> {
        let footprint = self.footprint.scaled(scale);
        let back = footprint.width.max(footprint.depth);
        let mut places = Vec::new();
        for layer in 0..TRIES {
            let depth = -(layer as f32) * back;
            if depth - footprint.depth / 2.0 < -extent.depth / 2.0 - 1e-4 {
                break;
            }
            let aside = if layer % 2 == 1 {
                footprint.width / 2.0
            } else {
                0.0
            };
            // Kept, beside what the point keeps; else from the point, or half a column aside of it.
            let (mut columns, from) = if self.kept {
                (Vec::new(), self.gap(scale) + aside)
            } else if aside > 0.0 {
                (Vec::new(), aside)
            } else {
                (vec![0.0], footprint.width)
            };
            for column in 0..TRIES {
                let across = from + column as f32 * footprint.width;
                columns.extend([across, -across]);
            }
            for lift in self.rows(extent, scale) {
                for across in &columns {
                    let position = self.place([*across, lift, depth]);
                    if self.within(extent, position, scale) && places.len() < PLACES {
                        places.push(position);
                    }
                }
            }
        }
        places
    }
}

/// Where today's arc stands `slot` about whoever's eyes are at `audience`, through `point`, whose +Z is `facing`: one
/// curved band about them, on which the point itself stands, in the middle of its gap, its radius their horizontal
/// distance, but at least [`NEAREST_ARC`]; its members along it to either side, alternately left and right, two rows to
/// each pair of columns, from 40° of the point's bearing from them, 18⅔° apart, or as far apart along the arc where it
/// stands nearer than [`ARC_RADIUS`], at about the point's height: the first row [`ROW_BELOW`] below it, the rest
/// [`ROW_APART`] above one another, spread in proportion where it stands further than [`ARC_RADIUS`], as signs are
/// drawn larger there. Nearer than [`ARC_RADIUS`], the band holds the first two rows of the columns that fit before
/// where the last stands at [`ARC_RADIUS`], five at 2.3 m, eighteen members; the rest stand on the arc [`ARC_RADIUS`]
/// from whoever watches, at their columns' own angles, above and behind the band, the columns that do not fit from two
/// rows up. Never nearer the floor than [`LOWEST`].
pub fn home_arc(point: [f32; 3], facing: [f32; 3], audience: [f32; 3], slot: usize) -> [f32; 3] {
    let (side, column, row) = lane(slot);
    let across = [point[0] - audience[0], point[2] - audience[2]];
    let length = (across[0] * across[0] + 0.0 * 0.0 + across[1] * across[1]).sqrt();
    // Towards the middle, or else, as from its very place, the way it faces them from: normalised as `glam` does, by
    // the reciprocal of the length, so that clients that place in `glam` agree to the bit.
    let normal = |[x, z]: [f32; 2]| {
        let reciprocal = 1.0 / (x * x + 0.0 * 0.0 + z * z).sqrt();
        (reciprocal.is_finite() && reciprocal > 0.0).then_some([x * reciprocal, z * reciprocal])
    };
    let ahead = normal(across)
        .or_else(|| normal([-facing[0], -facing[2]]))
        .unwrap_or([0.0, -1.0]);
    let right = [-ahead[1], ahead[0]];
    let radius = length.max(NEAREST_ARC);
    // Nearer than the arc's own radius, its columns stand as far apart along it as there, as many as fit before the
    // last column there; their rows above the first two, and the rest of the columns, two rows up, stand on the arc's
    // own radius, at their own angles, above and behind the band.
    let widen = (ARC_RADIUS / radius).max(1.0);
    let last = column_angle(COLUMNS - 1);
    let fit = (0..COLUMNS)
        .take_while(|column| column_angle(*column) * widen <= last + 1e-4)
        .count();
    let (radius, angle, level) = if fit < COLUMNS && (column >= fit || row >= 2) {
        let lift = if column >= fit { 2 } else { 0 };
        (radius.max(ARC_RADIUS), column_angle(column), row + lift)
    } else {
        (radius, column_angle(column) * widen, row)
    };
    let (sin, cos) = (angle.sin(), angle.cos());
    let direction = [
        ahead[0] * cos + right[0] * (side * sin),
        ahead[1] * cos + right[1] * (side * sin),
    ];
    // Further than the arc's own radius, its rows spread in proportion, as signs are drawn larger, so that no sign
    // reaches the member above it.
    let spread = (radius / ARC_RADIUS).max(1.0);
    let y = (point[1] + (level as f32 * ROW_APART - ROW_BELOW) * spread)
        .max(crate::entourage::FLOOR + LOWEST);
    [
        audience[0] + direction[0] * radius,
        y,
        audience[2] + direction[1] * radius,
    ]
}

/// The side of `slot`'s column on today's arc, -1 on the left and 1 on the right, its column, from the nearest the
/// middle, and its row: two comfortable front columns and two rows before wrapping around the audience. A slot's place
/// is permanent, whatever else stands.
pub fn lane(slot: usize) -> (f32, usize, usize) {
    let lane = slot % 4 + (slot % 32) / 8 * 4;
    let side = if lane.is_multiple_of(2) { -1.0 } else { 1.0 };
    (side, lane / 2, (slot / 32) * 2 + (slot % 8) / 4)
}

/// How far a column of today's arc stands from the point, about whoever watches, on an arc [`ARC_RADIUS`] from them or
/// further, in radians: sixteen columns spanning 280°, keeping the agent's cone clear.
pub fn column_angle(column: usize) -> f32 {
    (40.0 + column as f32 * (280.0 / 15.0)).to_radians()
}

#[cfg(test)]
mod tests;
