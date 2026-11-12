// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where everything in the farmyard is, in the yard's own coordinates, metres, with the gate on the +Z side, the coop
//! towards −Z, and the ground at 0: the fence and its gate, the coop and its ramp, the water trough, the feed, the hay,
//! the notice board and the workbench in the front left corner; and where participants arrive, [`ARRIVAL`], and where
//! the agent's home is, [`AGENT_HOME`], at the heart of the workbench's curve. The scenery draws them here, the
//! participants land on the ground it draws clear of them, as `crate::walkable` derives, and the flock keeps clear of
//! them, so that all three agree; Tomori
//! carries them all into a space's coordinates, where the arrival is home, at the origin facing −Z (see
//! `design/environments.md`).
use glam::{Vec2, Vec3};

/// An axis-aligned box on the ground, `[x, z]` from its least to its greatest corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub low: Vec2,
    pub high: Vec2,
}

impl Footprint {
    pub const fn new(low: [f32; 2], high: [f32; 2]) -> Self {
        Self {
            low: Vec2::new(low[0], low[1]),
            high: Vec2::new(high[0], high[1]),
        }
    }

    pub fn centre(&self) -> Vec2 {
        (self.low + self.high) * 0.5
    }

    pub fn size(&self) -> Vec2 {
        self.high - self.low
    }

    /// Whether `point` is inside it.
    pub fn contains(&self, point: Vec2) -> bool {
        point.cmpge(self.low).all() && point.cmple(self.high).all()
    }

    /// It, grown by `margin` on every side.
    pub fn grown(&self, margin: f32) -> Self {
        Self {
            low: self.low - Vec2::splat(margin),
            high: self.high + Vec2::splat(margin),
        }
    }

    /// Whether it and `other` overlap.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.low.cmplt(other.high).all() && other.low.cmplt(self.high).all()
    }
}

/// The fence around the yard: the posts stand on this rectangle.
pub const FENCE: Footprint = Footprint::new([-5.0, -11.0], [5.0, 1.6]);
/// The gate, the gap in the fence in front of home, from one post to the other along X.
pub const GATE: [f32; 2] = [-0.8, 0.8];
/// How tall the fence's posts are, and how thick, square.
pub const FENCE_HEIGHT: f32 = 1.0;
pub const FENCE_POST: f32 = 0.12;
/// How thick the gate's two posts are, square, and how tall, taller than the fence's.
pub const GATE_POST: f32 = 0.16;
pub const GATE_HEIGHT: f32 = FENCE_HEIGHT * 1.35;

/// The coop: a hen house raised on legs, its door facing +Z, towards home.
pub const COOP: Footprint = Footprint::new([-3.8, -9.4], [-1.8, -7.8]);
/// The height of the coop's floor, which its ramp climbs to.
pub const COOP_FLOOR: f32 = 0.6;
/// The top of the coop's walls, and of its roof's ridge.
pub const COOP_WALLS: f32 = 1.6;
pub const COOP_RIDGE: f32 = 2.2;
/// The middle of the coop's door, at the sill, in its front wall.
pub const DOOR: Vec3 = Vec3::new(-2.8, COOP_FLOOR, -7.8);
/// The door's width and height.
pub const DOOR_SIZE: Vec2 = Vec2::new(0.36, 0.4);
/// Where the ramp meets the ground, in front of the door.
pub const RAMP_FOOT: Vec3 = Vec3::new(-2.8, 0.0, -6.2);
/// How wide the ramp is.
pub const RAMP_WIDTH: f32 = 0.42;
/// The window in the coop's side towards the yard, `+X`: its least and greatest corners on that wall, `[z, y]`.
pub const WINDOW: [[f32; 2]; 2] = [[-9.0, 0.85], [-8.2, 1.35]];
/// The lantern hanging beside the coop's door, the middle of its glass: from a bracket on the front wall, under the
/// roof's overhang and clear of it.
pub const LANTERN: Vec3 = Vec3::new(-2.18, 1.45, -7.67);
/// The coop's two nest boxes, against its back wall, inside: each box's inside, from its back board to its front lip
/// and between its sides, on the ground, room enough for a hen to walk in, stop and turn round.
pub const NESTS: [Footprint; 2] = [
    Footprint::new([-3.46, -9.315], [-2.98, -8.795]),
    Footprint::new([-2.62, -9.315], [-2.14, -8.795]),
];
/// How thick a nest box's boards are, the top of its bottom, which a hen in it stands on, the top of its low front lip,
/// and the top of its sides and back.
pub const NEST_BOARD: f32 = 0.02;
pub const NEST_FLOOR: f32 = COOP_FLOOR + 0.03;
pub const NEST_LIP: f32 = NEST_FLOOR + 0.05;
pub const NEST_WALLS: f32 = COOP_FLOOR + 0.3;
/// The flat stones of the path from the gate towards the coop: how many, and how far each one's edge is bevelled in from
/// its outline on the earth to its top.
pub const STONES: u32 = 9;
pub const STONE_BEVEL: f32 = 0.02;
/// The highest a chicken stands while it is on the ground: on a stone of the path, but not up the coop's ramp.
pub const ON_GROUND: f32 = 0.03;
/// The lantern on the gate's post.
pub const GATE_LANTERN: Vec3 = Vec3::new(1.0, 1.25, 1.6);

/// The water trough.
pub const TROUGH: Footprint = Footprint::new([3.0, -8.5], [4.4, -8.05]);
pub const TROUGH_HEIGHT: f32 = 0.38;
/// The feeder, a round tray the grain is scattered about, and how far the grain is scattered.
pub const FEEDER: Vec2 = Vec2::new(0.8, -6.8);
pub const FEEDER_RADIUS: f32 = 0.28;
pub const FEEDER_HEIGHT: f32 = 0.35;
pub const GRAIN_RADIUS: f32 = 0.9;
/// Where the farmyard's own projection wall stands, the middle of its face: on the notice board against the right of
/// the fence, short of the trough, a little below eye height, facing into the yard, along −X, for what the agent
/// projects for whoever asks from that side of the yard, and what participants put on it. It is part of the place,
/// locked until a space's owner unlocks it. See `design/projection.md`.
pub const NOTICEBOARD: Vec3 = Vec3::new(4.565, 1.45, -4.8);
/// The way the notice board faces: into the yard, square to the fence.
pub const NOTICEBOARD_FACING: Vec3 = Vec3::new(0.0, 1.45, -4.8);
/// The notice board's grid of slots, as `[columns, rows]`: two standard slots side by side, about 1.3 by 0.5 m.
pub const NOTICEBOARD_GRID: [u32; 2] = [2, 1];
/// The notice board's back, against which the wall stands: its least and greatest corners, `[x, y, z]`, wider and
/// taller than the wall, so that the wood frames it.
pub const NOTICEBOARD_BACK: [[f32; 3]; 2] = [[4.585, 1.08, -5.56], [4.63, 1.82, -4.04]];
/// The notice board's two posts, either side of its back, `[x, z]` their middles, how thick they are and how tall.
pub const NOTICEBOARD_POSTS: [[f32; 2]; 2] = [[4.63, -5.62], [4.63, -3.98]];
pub const NOTICEBOARD_POST: f32 = 0.09;
pub const NOTICEBOARD_HEIGHT: f32 = 2.05;
/// The hay bales, stacked in the back corner, and how high they are stacked.
pub const HAY: Footprint = Footprint::new([3.0, -10.7], [4.6, -9.5]);
pub const HAY_HEIGHT: f32 = 1.2;

/// Where participants arrive in the yard, home: inside the gate, on the earth, facing [`ARRIVAL_FACING`], so that they
/// see at once the coop and the yard, the agent at its workbench to their left. Travel from there is free.
pub const ARRIVAL: Vec3 = Vec3::new(0.6, 0.0, -2.4);
/// What participants face as they arrive: the coop's front corner, beside its ramp.
pub const ARRIVAL_FACING: Vec3 = Vec3::new(-1.6, 0.0, -7.0);

/// The workbench in the yard's front left corner, where the fence by the gate meets its left side, on the coop's side of
/// the yard: a wooden bench curving through 100° about the middle of its curve, this point on the ground, its concave
/// side facing out of the corner towards where participants arrive ([`workbench_facing`]), with a tool rack behind it.
/// The agent's home floats above this point, at the heart of the curve, and its workers stand in the ring about it,
/// those behind it hovering over the bench. Placed so that the rack's back stands 8 cm inside the fence on the left,
/// where the curve passes square to it, and at the gate's end. See the crate's `context/workbench.md`.
pub const WORKBENCH: Vec2 = Vec2::new(-2.5, -0.84);
/// How far the workbench curves to either side of its middle, in radians: 50°.
pub const WORKBENCH_HALF_SPAN: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// The bench top's inner and outer radius about [`WORKBENCH`], 65 cm deep, and how high it stands.
pub const WORKBENCH_TOP: [f32; 2] = [1.65, 2.3];
pub const WORKBENCH_HEIGHT: f32 = 0.9;
/// How high whatever lies on the bench top reaches, and how far the vice at its front stands proud of it: below the
/// lowest the workers hovering above the bench reach, with their clearance.
pub const WORKBENCH_CLUTTER: f32 = 0.97;
pub const VICE: f32 = 0.1;
/// The tool rack behind the bench: the inner and outer radius its boards and posts stand within, about
/// [`WORKBENCH`], how tall it stands, and how many flat boards it is made of, each a solid of its own, so that the
/// solids follow its curve closely enough for the workers before it to stand clear.
pub const RACK: [f32; 2] = [2.34, 2.42];
pub const RACK_HEIGHT: f32 = 1.9;
pub const RACK_BOARDS: usize = 12;
/// How many pieces the bench's solids and footprints are made of, along its curve.
pub const WORKBENCH_PIECES: usize = 5;

/// The agent's home: above the middle of the workbench's curve, at the heart of its workshop, about 3.5 m from the eyes
/// where participants arrive, facing them and out of the corner. It appears here as a space opens, and is here
/// whenever it is at home; its workers stand in the ring about it ([`HOME_SWEEP`]), those behind it over the bench.
pub const AGENT_HOME: Vec3 = Vec3::new(WORKBENCH.x, 1.5, WORKBENCH.y);

/// How much of the ring about the agent's home its workers stand on, in degrees, about the way the home faces, out of
/// the corner towards the arrival, which Tomori's own server gives the farmyard's `home` cluster anchor: all but the place straight behind the agent, which
/// the swarm would hide from there, so that six stand in each row at full size, two of them behind the agent, over the
/// bench either side of its middle, and ten compact, four of them behind it.
pub const HOME_SWEEP: f32 = 300.0;

/// The way the workbench faces, the middle of its curve out of the corner: towards where participants arrive, as a
/// yaw about the vertical, 0 along +Z and a quarter turn along +X, as every anchor faces, so that the agent's home above
/// it faces the same way, and the ring's places behind the agent, as seen from the arrival, stand over the bench.
pub fn workbench_facing() -> f32 {
    let towards = Vec2::new(ARRIVAL.x, ARRIVAL.z) - WORKBENCH;
    towards.x.atan2(towards.y)
}

/// The point on the ground `radius` from the workbench's middle, `turn` radians along its curve from straight behind
/// the middle, positive towards the gate.
pub fn on_workbench(radius: f32, turn: f32) -> Vec2 {
    let yaw = workbench_facing() + std::f32::consts::PI + turn;
    WORKBENCH + Vec2::new(yaw.sin(), yaw.cos()) * radius
}

/// The turn along the workbench's curve, from its middle, where piece `piece` of `pieces` begins.
pub fn workbench_turn(piece: usize, pieces: usize) -> f32 {
    WORKBENCH_HALF_SPAN * (2.0 * piece as f32 / pieces as f32 - 1.0)
}

/// The footprints of `pieces` flat pieces of the band from `inner` to `outer` about the workbench's middle, along the
/// whole of its curve: each the box about its corners, which the flat pieces the scenery draws stand within.
pub fn workbench_band(inner: f32, outer: f32, pieces: usize) -> Vec<Footprint> {
    (0..pieces)
        .map(|piece| {
            let ends = [
                workbench_turn(piece, pieces),
                workbench_turn(piece + 1, pieces),
            ];
            let corners = ends
                .into_iter()
                .flat_map(|turn| [on_workbench(inner, turn), on_workbench(outer, turn)]);
            let (low, high) = corners.fold(
                (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
                |(low, high), corner| (low.min(corner), high.max(corner)),
            );
            Footprint { low, high }
        })
        .collect()
}

/// The workbench's footprints on the ground, the bench, its vice and the rack behind it, which nobody lands on and the
/// chickens walk round: [`WORKBENCH_PIECES`] pieces along its curve.
pub fn workbench() -> Vec<Footprint> {
    workbench_band(WORKBENCH_TOP[0] - VICE, RACK[1], WORKBENCH_PIECES)
}

/// The lantern hung on the rack's post at the gate's end of the workbench, which lights it warmly.
pub fn workbench_lantern() -> Vec3 {
    let at = on_workbench(RACK[0] - 0.1, WORKBENCH_HALF_SPAN - 0.06);
    Vec3::new(at.x, 1.62, at.y)
}

/// The notice board's footprint on the ground, its posts and its back, which nobody lands on and nothing walks through.
pub fn noticeboard() -> Footprint {
    let half = NOTICEBOARD_POST * 0.5;
    let [first, last] = NOTICEBOARD_POSTS;
    Footprint::new(
        [NOTICEBOARD_BACK[0][0].min(first[0] - half), first[1] - half],
        [first[0] + half, last[1] + half],
    )
}

/// The solids of the yard that nothing stands inside nor is seen through, as `(least, greatest)` corners: the coop
/// with its roof's overhang, its ramp, the trough, the feeder, the hay and the notice board, each as high as it stands;
/// then the workbench, its top and what lies on it in [`WORKBENCH_PIECES`] pieces, as high as [`WORKBENCH_CLUTTER`], and
/// its rack in [`RACK_BOARDS`], one for each of its boards, as tall as its posts.
pub fn solids() -> Vec<([f32; 3], [f32; 3])> {
    let solid = |footprint: Footprint, height: f32| {
        (
            [footprint.low.x, 0.0, footprint.low.y],
            [footprint.high.x, height, footprint.high.y],
        )
    };
    let feeder = obstacles()[3];
    let mut solids = vec![
        solid(COOP.grown(0.18), COOP_RIDGE + 0.1),
        solid(ramp(), COOP_FLOOR),
        solid(TROUGH, TROUGH_HEIGHT),
        solid(feeder, FEEDER_HEIGHT),
        solid(HAY, HAY_HEIGHT),
        solid(noticeboard(), NOTICEBOARD_HEIGHT),
    ];
    solids.extend(
        workbench_band(WORKBENCH_TOP[0] - VICE, WORKBENCH_TOP[1], WORKBENCH_PIECES)
            .into_iter()
            .map(|piece| solid(piece, WORKBENCH_CLUTTER)),
    );
    solids.extend(
        workbench_band(RACK[0], RACK[1], RACK_BOARDS)
            .into_iter()
            .map(|board| solid(board, RACK_HEIGHT)),
    );
    solids
}

/// What stands in the yard, which nobody lands on and nothing walks through: the coop and its ramp, the trough, the
/// feeder, the hay and the notice board, then the workbench's pieces ([`workbench`]).
pub fn obstacles() -> Vec<Footprint> {
    let mut obstacles = vec![
        COOP,
        ramp(),
        TROUGH,
        Footprint::new(
            [FEEDER.x - FEEDER_RADIUS, FEEDER.y - FEEDER_RADIUS],
            [FEEDER.x + FEEDER_RADIUS, FEEDER.y + FEEDER_RADIUS],
        ),
        HAY,
        noticeboard(),
    ];
    obstacles.extend(workbench());
    obstacles
}

/// The ramp's footprint, from the door to its foot.
pub fn ramp() -> Footprint {
    let half = RAMP_WIDTH * 0.5;
    Footprint::new([DOOR.x - half, DOOR.z], [RAMP_FOOT.x + half, RAMP_FOOT.z])
}

/// The height of the ramp's board where it is `z` along the yard, rising straight from the ground at its foot to the
/// coop's floor at the door's sill: the ground's before its foot and the floor's beyond its sill.
pub fn ramp_height(z: f32) -> f32 {
    let up = (z - RAMP_FOOT.z) / (DOOR.z - RAMP_FOOT.z);
    COOP_FLOOR * up.clamp(0.0, 1.0)
}

/// What a chicken's feet stand on at `point`, on the ground: up the ramp, its board; in the coop, its floor, or a nest
/// box's bottom or front lip; on a stone of the path, its top, sloping down its bevel; and elsewhere, the ground.
pub fn underfoot(point: Vec2) -> f32 {
    if let Some(nest) = nest_height(point) {
        return nest;
    }
    if ramp().contains(point) {
        return ramp_height(point.y);
    }
    if COOP.contains(point) {
        return COOP_FLOOR;
    }
    stones()
        .iter()
        .map(|stone| stone.height_at(point))
        .fold(0.0, f32::max)
}

/// The top of a nest box's front lip or bottom at `point`, if it is over one: the lip along the front of its inside, and
/// the bottom beneath the boards and the inside.
pub fn nest_height(point: Vec2) -> Option<f32> {
    NESTS.iter().find_map(|inside| {
        let lip = Footprint::new(
            [inside.low.x - NEST_BOARD, inside.high.y],
            [inside.high.x + NEST_BOARD, inside.high.y + NEST_BOARD],
        );
        if lip.contains(point) {
            Some(NEST_LIP)
        } else {
            inside
                .grown(NEST_BOARD)
                .contains(point)
                .then_some(NEST_FLOOR)
        }
    })
}

/// A flat stone of the path, set in the earth: its outline on the ground, `[x, z]`, anticlockwise seen from above, and
/// how high its top stands, its edge bevelled [`STONE_BEVEL`] in from its outline.
#[derive(Clone, Debug, PartialEq)]
pub struct Stone {
    pub outline: Vec<Vec2>,
    pub height: f32,
}

impl Stone {
    /// The middle of its outline.
    pub fn middle(&self) -> Vec2 {
        self.outline.iter().sum::<Vec2>() / self.outline.len() as f32
    }

    /// Its top's outline, [`STONE_BEVEL`] in from its outline towards its middle.
    pub fn top(&self) -> Vec<Vec2> {
        let middle = self.middle();
        self.outline
            .iter()
            .map(|corner| *corner - (*corner - middle).normalize() * STONE_BEVEL)
            .collect()
    }

    /// How high it stands at `point`: its height on its top, less down its bevel, and nothing beyond its outline.
    pub fn height_at(&self, point: Vec2) -> f32 {
        // How far in from its nearest edge `point` is, which is negative outside, as the outline is convex.
        let inside = self
            .outline
            .iter()
            .zip(self.outline.iter().cycle().skip(1))
            .map(|(from, to)| {
                let edge = (*to - *from).normalize();
                // Anticlockwise seen from above, `[x, z]`, the inside is to the right of each edge.
                -edge.perp_dot(point - *from)
            })
            .fold(f32::INFINITY, f32::min);
        let bevel = STONE_BEVEL * (std::f32::consts::PI / self.outline.len() as f32).cos();
        self.height * (inside / bevel).clamp(0.0, 1.0)
    }
}

/// The path's stones, from inside the gate towards the coop and the feeder, each of six or seven sides, a little
/// irregular, turned and set off the path's line a little, standing 8 to 20 mm proud of the earth, the same every time.
pub fn stones() -> Vec<Stone> {
    use crate::props::scatter;
    (0..STONES)
        .map(|stone| {
            let seed = stone * 11 + 700;
            let along = stone as f32;
            let middle = Vec2::new(
                -0.2 - along * 0.25 + (scatter(seed) - 0.5) * 0.06,
                0.8 - along * 0.8 + (scatter(seed + 1) - 0.5) * 0.06,
            );
            let sides = 6 + u32::from(scatter(seed + 2) < 0.5);
            let radius = 0.3 + scatter(seed + 3) * 0.05;
            let turn = scatter(seed + 4) * std::f32::consts::TAU;
            let outline = (0..sides)
                .map(|side| {
                    let angle = turn
                        + (side as f32 + (scatter(seed + 5 + side) - 0.5) * 0.2) / sides as f32
                            * std::f32::consts::TAU;
                    let reach = radius * (0.9 + scatter(seed + 12 + side) * 0.1);
                    middle + Vec2::new(angle.cos(), -angle.sin()) * reach
                })
                .collect();
            Stone {
                outline,
                height: 0.008 + scatter(seed + 20) * 0.012,
            }
        })
        .collect()
}

/// How far the earth participants land on in the yard stops short of each of what stands in it, [`obstacles`], clear
/// of the coop's roof, which overhangs its walls by 0.18 m, and of what stands beside the trail; and how far short of
/// the fence's line, its posts' middles. Where participants land is derived from them, as `crate::walkable` says.
pub const CLEARANCE: f32 = 0.25;
pub const FENCE_CLEARANCE: f32 = 0.4;

/// The middle of the land beyond the fence, about which the backdrop's foothills ring the yard: the yard's middle.
pub const BACKDROP_CENTRE: Vec2 = Vec2::new(0.0, -4.0);
/// The way the waterfall lies from [`BACKDROP_CENTRE`], as a yaw from −Z towards +X: behind the coop, a little to
/// its right as participants arrive, so that they see it over the back fence as they come in.
pub const WATERFALL_AZIMUTH: f32 = -7.5 * std::f32::consts::PI / 180.0;
/// How far from [`BACKDROP_CENTRE`] the waterfall's cliff rises, from its foot to its lip, two of the foothills' rings,
/// and how high its top stands.
pub const CLIFF: [f32; 2] = [22.0, 23.5];
pub const CLIFF_TOP: f32 = 6.5;
/// Where the waterfall's water leaves the lip of its cliff, and where it meets its pool.
pub const WATERFALL_LIP: Vec3 = Vec3::new(-3.067, CLIFF_TOP, -27.299);
pub const WATERFALL_FOOT: Vec3 = Vec3::new(-2.872, POOL.y, -25.812);
/// The middle of the waterfall's fall, where a sound of it would come from.
pub const WATERFALL: Vec3 = Vec3::new(-2.9695, (CLIFF_TOP + POOL.y) * 0.5, -26.5555);
/// The middle of the pool at the waterfall's foot, at its water's level, and how wide it is to its shore: sunk below the
/// land all round it, and below the trail, in a hollow at the cliff's foot.
pub const POOL: Vec3 = Vec3::new(-2.545, -0.3, -23.333);
pub const POOL_RADIUS: f32 = 4.0;
/// The river, from the pool downstream, its middle at its water's level: out over the pool's lip towards the yard, then
/// round its right side, 11 m beyond the fence, and away into the foothills to the front right, behind a spur; running
/// downhill all the way in its channel, below the trail and the land either side.
pub const RIVER: [Vec3; 13] = [
    POOL,
    // Where it leaves the pool, beyond the pool's water, a little below it, and down a short run to the next.
    Vec3::new(0.112, -0.32, -19.337),
    Vec3::new(2.6, -0.42, -19.6),
    Vec3::new(6.6, -0.47, -18.8),
    Vec3::new(10.4, -0.51, -16.6),
    Vec3::new(13.6, -0.55, -12.8),
    Vec3::new(15.4, -0.58, -7.8),
    Vec3::new(16.0, -0.61, -2.8),
    Vec3::new(15.3, -0.64, 2.2),
    Vec3::new(15.9, -0.67, 7.0),
    Vec3::new(18.4, -0.7, 11.6),
    Vec3::new(23.5, -0.73, 12.5),
    Vec3::new(28.5, -0.76, 10.0),
];
/// The waterfall rock: a boulder in the pool in front of the waterfall, a little to its right as one faces it from the
/// yard, close enough to feel its spray and clear of its fall and its mist: the middle of its flat top, where one
/// stands, half a metre above the water; how far its top reaches from there, and how far its sides at their widest,
/// at the water; and how far below the water its foot stands, on the pool's basin.
pub const WATERFALL_ROCK: Vec3 = Vec3::new(-1.045, POOL.y + 0.5, -23.43);
pub const ROCK_TOP: f32 = 0.6;
pub const ROCK_REACH: f32 = 0.9;
pub const ROCK_FOOT: f32 = POOL.y - 0.3;

/// The upper terrace, a private place at the top of the falls, reached only by pointing at the waterfall from its rock:
/// how high its floor is drawn, 7.2 m above the pool, on the mountain behind the waterfall, beside the stream that runs
/// from the upper pool to the waterfall's lip. See [`crate::terrace`].
pub const TERRACE: f32 = 6.9;
/// How far from [`BACKDROP_CENTRE`] the upper cliff behind the terrace rises, from its foot to its lip, two of the
/// foothills' rings, and how high its top stands.
pub const UPPER_CLIFF: [f32; 2] = [29.5, 30.5];
pub const UPPER_TOP: f32 = 9.5;
/// The upper pool at the upper cliff's foot, on the waterfall's way, its middle at its water's level, which is the
/// water's as it leaves the waterfall's lip, and how wide it is to its shore: its overflow runs straight to the lip and
/// over it, so that the two falls are one stream.
pub const UPPER_POOL: Vec3 = Vec3::new(-3.681, CLIFF_TOP + 0.08, -31.959);
pub const UPPER_POOL_RADIUS: f32 = 1.2;
/// Where the upper falls' water leaves the lip of the upper cliff, where it meets the upper pool, and the middle of its
/// fall, where a sound of it comes from.
pub const UPPER_FALLS_LIP: Vec3 = Vec3::new(-3.981, UPPER_TOP, -34.239);
pub const UPPER_FALLS_FOOT: Vec3 = Vec3::new(-3.8244, UPPER_POOL.y, -33.0493);
pub const UPPER_FALLS: Vec3 = Vec3::new(-3.90265, (UPPER_TOP + UPPER_POOL.y) * 0.5, -33.6442);

/// The river's far bank seen from `from`, by its point `point`: 2 m beyond its middle, away from `from`, 0.3 m above its
/// water, on the far shore rising out of it, which is what is seen of a river in its channel from afar.
pub fn far_bank(point: Vec3, from: Vec2) -> Vec3 {
    let away = (Vec2::new(point.x, point.z) - from).normalize() * 2.0;
    Vec3::new(point.x + away.x, point.y + 0.3, point.z + away.y)
}

/// How far the river's water reaches either side of its middle, to its shingle.
pub const RIVER_HALF_WIDTH: f32 = 1.2;
/// The patches of native bush beyond the fence, `(middle, radius)` on the ground: in the gully by the waterfall, behind
/// the hay, on the foothills to the left, behind the workbench, and across the river to the right, where tree ferns,
/// cabbage trees, flax, podocarps and the bush's canopy gather most.
pub const BUSH: [(Vec2, f32); 5] = [
    (Vec2::new(-10.5, -25.5), 6.0),
    (Vec2::new(7.5, -27.5), 6.0),
    (Vec2::new(-21.0, -9.0), 7.0),
    (Vec2::new(-16.0, 10.5), 7.0),
    (Vec2::new(23.0, -8.0), 6.0),
];

/// Where the backdrop's sounds come from, each a name and a place: the `waterfall` at the middle of its fall, its
/// `pool`, the `upper-falls` at the middle of the falls above the terrace, the `river` at each of its bends and every
/// `bush` patch's middle, on the ground. Ambient sound attaches here.
pub fn sounds() -> Vec<(&'static str, Vec3)> {
    let mut sounds = vec![
        ("waterfall", WATERFALL),
        ("pool", POOL),
        ("upper-falls", UPPER_FALLS),
    ];
    sounds.extend(RIVER[1..].iter().map(|point| ("river", *point)));
    sounds.extend(BUSH.iter().map(|(middle, _)| {
        let ground = crate::backdrop::ground(*middle);
        ("bush", Vec3::new(middle.x, ground, middle.y))
    }));
    sounds
}

/// How many steps the river's middle is drawn in between each two of [`RIVER`]'s points.
pub const RIVER_STEPS: usize = 3;

/// The river's middle, smoothed through [`RIVER`]'s points, `steps` to each reach, at its water's level: where its water
/// is drawn, and what its banks are measured from.
pub fn river_line(steps: usize) -> Vec<Vec3> {
    let mut line = Vec::new();
    for reach in 0..RIVER.len() - 1 {
        let at = |index: isize| RIVER[index.clamp(0, RIVER.len() as isize - 1) as usize];
        let [p0, p1, p2, p3] = [-1, 0, 1, 2].map(|offset| at(reach as isize + offset));
        for step in 0..steps {
            let t = step as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            line.push(
                0.5 * (2.0 * p1
                    + (p2 - p0) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3),
            );
        }
    }
    line.push(RIVER[RIVER.len() - 1]);
    line
}

/// The river's middle as it is drawn, [`RIVER_STEPS`] to each reach, made once.
fn drawn_river() -> &'static [Vec3] {
    static LINE: std::sync::OnceLock<Vec<Vec3>> = std::sync::OnceLock::new();
    LINE.get_or_init(|| river_line(RIVER_STEPS))
}

/// The nearest point to `point` on the river's middle as it is drawn ([`river_line`]), at its water's level, and how
/// far it is on the ground.
pub fn nearest_river(point: Vec2) -> (Vec3, f32) {
    drawn_river()
        .windows(2)
        .map(|pair| {
            let (from, to) = (pair[0], pair[1]);
            let along = Vec2::new(to.x - from.x, to.z - from.z);
            let offset = point - Vec2::new(from.x, from.z);
            let t = (offset.dot(along) / along.length_squared()).clamp(0.0, 1.0);
            let nearest = from.lerp(to, t);
            (nearest, point.distance(Vec2::new(nearest.x, nearest.z)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("the river has reaches")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walkable ground's triangles within the fence, the yard's, seen from above.
    fn yard_ground() -> Vec<[Vec2; 3]> {
        crate::walkable::navmesh()
            .triangles()
            .iter()
            .map(|triangle| triangle.map(|corner| Vec2::new(corner.x, corner.z)))
            .filter(|triangle| {
                triangle
                    .iter()
                    .all(|corner| FENCE.grown(1e-3).contains(*corner))
            })
            .collect()
    }

    /// Whether `point`, seen from above, is on the walkable ground.
    fn on(point: Vec2) -> bool {
        crate::walkable::height_at(point).is_some()
    }

    #[test]
    fn nobody_lands_on_anything_and_everything_is_inside_the_fence() {
        let inside = FENCE.grown(-0.3);
        // Beyond the fence's clearance, the yard's ground reaches only between the gate's posts, within the way out.
        let [gateway, _] = crate::trail::GATEWAY;
        let way_out = Footprint::new(
            [gateway.x - crate::trail::HALF, gateway.y],
            [gateway.x + crate::trail::HALF, FENCE.high.y],
        )
        .grown(1e-3);
        let ground = yard_ground();
        let count = ground.len();
        assert!(count > 50, "{count}");
        for triangle in &ground {
            // Its corners, the middles of its edges and its own middle are clear of everything that stands in the yard.
            let middle = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
            let points = [0, 1, 2].map(|corner| triangle[corner]).into_iter().chain(
                [0, 1, 2].map(|corner| (triangle[corner] + triangle[(corner + 1) % 3]) * 0.5),
            );
            for point in points.chain([middle]) {
                assert!(inside.contains(point) || way_out.contains(point), "{point}");
                for obstacle in obstacles() {
                    assert!(!obstacle.grown(0.2).contains(point), "{point} {obstacle:?}");
                }
            }
        }
        // Everything stands well inside the fence, but the workbench, whose rack stands just inside it.
        let obstacles = obstacles();
        let (yard, bench) = obstacles.split_at(6);
        for obstacle in yard {
            assert!(inside.contains(obstacle.low) && inside.contains(obstacle.high));
        }
        let just = FENCE.grown(-0.05);
        for piece in bench {
            assert!(
                just.contains(piece.low) && just.contains(piece.high),
                "{piece:?}"
            );
        }
        // Where participants arrive is walkable, and so is the spot to peek in at the coop's window from, and the ground
        // beneath the agent's home.
        assert!(on(Vec2::new(ARRIVAL.x, ARRIVAL.z)));
        assert!(on(Vec2::new(
            COOP.high.x + 0.6,
            (WINDOW[0][0] + WINDOW[1][0]) * 0.5
        )));
        assert!(on(WORKBENCH));
        // The door is in the coop's front wall, and the ramp leads from it to the ground in front.
        assert_eq!(DOOR.z, COOP.high.y);
        assert!(COOP.contains(Vec2::new(DOOR.x, DOOR.z)));
        assert!(ramp().high.y > COOP.high.y);
        // The yard's ground is on its earth, and none of its triangles overlaps another: every 10 cm over the yard, a
        // point well within one is within no other.
        let mesh = crate::walkable::navmesh();
        assert!(
            mesh.triangles()
                .iter()
                .flatten()
                .filter(|corner| FENCE.grown(-0.01).contains(Vec2::new(corner.x, corner.z)))
                .all(|corner| corner.y == 0.0)
        );
        for column in 0..=(FENCE.size().x * 10.0).round() as u32 {
            for row in 0..=(FENCE.size().y * 10.0).round() as u32 {
                let point = FENCE.low + Vec2::new(column as f32, row as f32) * 0.1 + 0.003;
                let within = ground
                    .iter()
                    .filter(|[a, b, c]| {
                        let doubled = (*b - *a).perp_dot(*c - *a);
                        [(*a, *b), (*b, *c), (*c, *a)].iter().all(|(from, to)| {
                            (*to - *from).perp_dot(point - *from) * doubled.signum()
                                > 1e-3 * from.distance(*to)
                        })
                    })
                    .count();
                assert!(within <= 1, "{point}: {within}");
            }
        }
    }

    #[test]
    fn all_of_the_yard_s_open_ground_is_walkable_and_nothing_that_stands_in_it() {
        // Every 10 cm over the yard, a point more than a centimetre inside the fence's clearance and beyond every
        // obstacle's is walkable, and one more than a centimetre within an obstacle's clearance is not, so that no
        // change to the layout leaves a patch of open ground out, as the strip beside the feeder once was, from the
        // ramp's foot to the back fence, nor lets anyone land on the coop, its ramp, the trough, the feeder, the hay, the
        // notice board or the workbench.
        const { assert!(CLEARANCE >= 0.2 && CLEARANCE <= 0.3) };
        let kept: Vec<Footprint> = obstacles()
            .iter()
            .map(|obstacle| obstacle.grown(CLEARANCE))
            .collect();
        let yard = FENCE.grown(-FENCE_CLEARANCE);
        let (mut open, mut closed) = (0, 0);
        for column in 0..=(FENCE.size().x * 10.0).round() as u32 {
            for row in 0..=(FENCE.size().y * 10.0).round() as u32 {
                let point = FENCE.low + Vec2::new(column as f32, row as f32) * 0.1;
                if kept
                    .iter()
                    .any(|footprint| footprint.grown(-0.01).contains(point))
                {
                    assert!(!on(point), "{point}");
                    closed += 1;
                } else if yard.grown(-0.01).contains(point)
                    && kept
                        .iter()
                        .all(|footprint| !footprint.grown(0.01).contains(point))
                {
                    assert!(on(point), "{point}");
                    open += 1;
                }
            }
        }
        assert!(open > 7000 && closed > 1000, "{open} {closed}");
        // Beside the feeder, where the ground had been left out, from the ramp's foot to the back fence, and between the
        // trough and the hay.
        for z in [-6.0, -7.5, -8.5, -10.0] {
            assert!(on(Vec2::new(FEEDER.x, z)), "{z}");
        }
        assert!(on(Vec2::new(3.7, (TROUGH.low.y + HAY.high.y) * 0.5)));
        // And beyond the fence's clearance, nowhere but between the gate's posts, where the way out goes.
        for triangle in yard_ground() {
            for corner in triangle {
                assert!(
                    yard.grown(1e-4).contains(corner)
                        || (corner.x.abs() <= 0.4 + 1e-4 && corner.y <= FENCE.high.y + 1e-3),
                    "{corner}"
                );
            }
        }
    }

    #[test]
    fn the_path_s_stones_stand_a_little_proud_of_the_earth_apart_and_bevelled() {
        let stones = stones();
        assert_eq!(stones.len(), STONES as usize);
        assert_eq!(stones, super::stones());
        let mut heights = Vec::new();
        for (index, stone) in stones.iter().enumerate() {
            // Six or seven sides, convex, anticlockwise seen from above, inside the fence and clear of what stands in
            // the yard.
            let corners = stone.outline.len();
            assert!((6..=7).contains(&corners));
            for corner in 0..corners {
                let [a, b, c] = [0, 1, 2].map(|step| stone.outline[(corner + step) % corners]);
                assert!((b - a).perp_dot(c - b) < 0.0, "{index}");
                assert!(FENCE.grown(-0.3).contains(a));
                assert!(obstacles().iter().all(|obstacle| !obstacle.contains(a)));
            }
            // A centimetre or two proud, its top bevelled in from its edge, and standing on the ground beyond it.
            assert!((0.008..=0.02).contains(&stone.height), "{}", stone.height);
            heights.push(stone.height);
            let middle = stone.middle();
            assert_eq!(stone.height_at(middle), stone.height);
            assert_eq!(underfoot(middle), stone.height);
            let edge = stone.outline[0];
            assert!(stone.height_at(edge) < 1e-4);
            let partway = stone.height_at(edge.lerp(middle, 0.03));
            assert!(partway > 0.0 && partway < stone.height);
            let beyond = edge + (edge - middle).normalize() * 0.05;
            assert_eq!(stone.height_at(beyond), 0.0);
            for (corner, top) in stone.outline.iter().zip(stone.top()) {
                let inset = corner.distance(middle) - top.distance(middle);
                assert!((inset - STONE_BEVEL).abs() < 1e-5);
            }
            // None reaches another.
            for other in &stones[index + 1..] {
                let reach = |stone: &Stone| {
                    let middle = stone.middle();
                    stone
                        .outline
                        .iter()
                        .map(|corner| corner.distance(middle))
                        .fold(0.0, f32::max)
                };
                assert!(middle.distance(other.middle()) > reach(stone) + reach(other) + 0.05);
            }
        }
        // Not all alike.
        heights.sort_by(f32::total_cmp);
        assert!(
            heights[heights.len() - 1] - heights[0] > 0.004,
            "{heights:?}"
        );
        // Elsewhere is the ground, and the ramp rises from it to the coop's floor.
        assert_eq!(underfoot(Vec2::new(2.0, -3.0)), 0.0);
        assert_eq!(ramp_height(RAMP_FOOT.z + 1.0), 0.0);
        assert_eq!(ramp_height(DOOR.z - 1.0), COOP_FLOOR);
        assert!((ramp_height((RAMP_FOOT.z + DOOR.z) * 0.5) - COOP_FLOOR * 0.5).abs() < 1e-6);
        assert_eq!(underfoot(COOP.centre()), COOP_FLOOR);
    }

    #[test]
    fn the_workbench_curves_in_the_front_left_corner_facing_the_arrival_its_rack_just_inside_the_fence()
     {
        // In the corner where the fence by the gate meets its left side: nearer it than any other corner of the yard.
        let corner = Vec2::new(FENCE.low.x, FENCE.high.y);
        let corners = [
            corner,
            Vec2::new(FENCE.high.x, FENCE.high.y),
            Vec2::new(FENCE.low.x, FENCE.low.y),
            Vec2::new(FENCE.high.x, FENCE.low.y),
        ];
        let middle = on_workbench((WORKBENCH_TOP[0] + WORKBENCH_TOP[1]) * 0.5, 0.0);
        let nearest = corners
            .into_iter()
            .min_by(|a, b| a.distance(middle).total_cmp(&b.distance(middle)))
            .unwrap();
        assert_eq!(nearest, corner);
        // Its concave side faces where participants arrive, the middle of its curve straight behind its middle as seen
        // from there, and the gate's end towards the gate.
        let towards = Vec2::new(ARRIVAL.x, ARRIVAL.z) - WORKBENCH;
        let behind = on_workbench(1.0, 0.0) - WORKBENCH;
        assert!((behind.normalize().dot(-towards.normalize()) - 1.0).abs() < 1e-5);
        assert!(
            on_workbench(2.0, WORKBENCH_HALF_SPAN).y > on_workbench(2.0, -WORKBENCH_HALF_SPAN).y
        );
        // The rack's back stands 8 cm inside the fence on the left, where the curve passes square to it, its flat boards
        // a little less near, and at the gate's end; nothing of it beyond.
        let band = workbench();
        let low = band
            .iter()
            .map(|piece| piece.low.x)
            .fold(f32::INFINITY, f32::min);
        let high = band
            .iter()
            .map(|piece| piece.high.y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((0.0..0.01).contains(&(low - (FENCE.low.x + 0.08))), "{low}");
        assert!((high - (FENCE.high.y - 0.08)).abs() < 1e-2, "{high}");
        // A third of a turn and a little more, at a workbench's height, 65 cm deep, the rack behind its back edge, taller
        // than the bench and below the second row of workers; clear of everything else in the yard, the gate's posts
        // and the arrival among them.
        assert!((WORKBENCH_HALF_SPAN.to_degrees() - 50.0).abs() < 1e-4);
        assert!((WORKBENCH_TOP[1] - WORKBENCH_TOP[0] - 0.65).abs() < 1e-5);
        const { assert!(WORKBENCH_TOP[1] < RACK[0] && WORKBENCH_HEIGHT < WORKBENCH_CLUTTER) };
        const { assert!(RACK_HEIGHT > WORKBENCH_HEIGHT + 0.8) };
        for piece in &band {
            for other in &obstacles()[..6] {
                assert!(!piece.grown(1.0).overlaps(other), "{piece:?} {other:?}");
            }
            assert!(!piece.grown(0.5).contains(Vec2::new(ARRIVAL.x, ARRIVAL.z)));
            assert!(piece.high.x < GATE[0] - 1.5);
        }
        // Its solids are its pieces and its rack's boards, after the yard's; and the lantern on the rack's post hangs
        // just before the rack, above the bench, at its gate's end.
        let solids = solids();
        assert_eq!(solids.len(), 6 + WORKBENCH_PIECES + RACK_BOARDS);
        for (low, high) in &solids[6..6 + WORKBENCH_PIECES] {
            assert!(low[1] == 0.0 && high[1] == WORKBENCH_CLUTTER);
        }
        for (low, high) in &solids[6 + WORKBENCH_PIECES..] {
            assert!(low[1] == 0.0 && high[1] == RACK_HEIGHT);
        }
        let lantern = workbench_lantern();
        let lantern_at = Vec2::new(lantern.x, lantern.z);
        assert!(lantern.y > WORKBENCH_HEIGHT + 0.5 && lantern.y < RACK_HEIGHT);
        assert!((lantern_at.distance(WORKBENCH) - (RACK[0] - 0.1)).abs() < 1e-4);
        assert!(lantern_at.distance(on_workbench(RACK[0], WORKBENCH_HALF_SPAN)) < 0.25);
    }

    #[test]
    fn the_tomori_wall_beneath_the_agent_reads_and_the_agent_is_seen_from_where_participants_arrive()
     {
        use tomori_environment::sightlines::{READING, Solid, Spot, readable, seen};
        use tomori_protocol::{Pose, entourage};
        let arrival = Spot::facing(ARRIVAL.to_array(), ARRIVAL_FACING.to_array());
        let eyes = arrival.eyes();
        let solids: Vec<Solid> = solids()
            .into_iter()
            .map(|(low, high)| Solid::new(low, high))
            .collect();
        // The agent faces the eyes where participants arrive, as the server registers it, and the Tomori wall stands
        // beneath it, as the server stands it, on the ground where they arrive.
        let agent = Pose::at(AGENT_HOME.to_array()).facing_towards(eyes);
        let wall = entourage::wall_beneath(&agent, ARRIVAL.y);
        assert_eq!(
            readable(&arrival, &Spot::new(wall.position, wall.yaw()), &solids),
            Ok(())
        );
        assert_eq!(
            seen(&arrival, &Spot::new(agent.position, agent.yaw()), &solids),
            Ok(())
        );
        // Within reading distance, though further than the default's 2.3 m, below the swarm with the gap, and above
        // the ground.
        let far = Vec3::from_array(eyes).distance(Vec3::from_array(wall.position));
        assert!(far > 3.5 && far < READING, "{far}");
        assert!((AGENT_HOME.y - wall.position[1] - entourage::BENEATH).abs() < 1e-5);
        assert!(wall.position[1] - entourage::ROW_HALF >= ARRIVAL.y + entourage::CLEARANCE);
        // Inside the fence, and the agent at the workbench, above the middle of its curve, facing out of the corner
        // as the bench does, the bench and its rack behind it, and the wall beneath it clear of the bench.
        let inside = FENCE.grown(-0.3);
        let near = Vec2::new(AGENT_HOME.x, AGENT_HOME.z);
        assert!(inside.contains(near));
        assert_eq!(near, WORKBENCH);
        assert!((agent.yaw() - workbench_facing()).abs() < 1e-4);
        assert!(!COOP.grown(2.5).contains(near) && !ramp().grown(2.5).contains(near));
        for piece in workbench() {
            assert!(!piece.contains(near));
            let gap = (near - piece.centre()).length();
            assert!(gap > WORKBENCH_TOP[0] - VICE, "{gap}");
        }
        let distance = Vec3::from_array(eyes).distance(AGENT_HOME);
        assert!(distance > 3.4 && distance < 3.6, "{distance}");
    }

    #[test]
    fn one_to_eighteen_workers_stand_about_the_agent_s_home_clear_of_the_yard_and_seen_from_the_arrival()
     {
        use tomori_environment::sightlines::{Solid, Spot, fits};
        use tomori_protocol::Pose;
        use tomori_protocol::cluster::{Anchor, Cluster, Formation, HOME, Step};
        let arrival = Spot::facing(ARRIVAL.to_array(), ARRIVAL_FACING.to_array());
        let solids: Vec<Solid> = solids()
            .into_iter()
            .map(|(low, high)| Solid::new(low, high))
            .collect();
        // The farmyard's home anchor stands its workers in a ring about the agent's home, all but straight behind it,
        // gathered about the eyes at the arrival, as Tomori's own server registers it: without room, which an older
        // client would lay the arc's steps out in, so that it keeps today's arc; the ring's rows fit the yard, which
        // `fits` checks.
        let agent = Pose::at(AGENT_HOME.to_array()).facing_towards(arrival.eyes());
        let anchor = Anchor::new(HOME, agent)
            .with_formation(Formation::Ring)
            .with_sweep(HOME_SWEEP);
        let home = Cluster::new(&anchor).with_audience(arrival.eyes());
        assert_eq!(home.arrange(18).step(), Step::Ring);
        assert!(!home.arrange(12).compact() && home.arrange(13).compact());
        assert_eq!(fits(&home, ARRIVAL.y, &solids), Ok(()));
        // Its signs read and are pointed at from the yard: none of eighteen above 2.5 m, and the notice board seen past
        // them from the arrival.
        for slot in home.slots(18) {
            assert!(
                slot.position[1] <= tomori_protocol::cluster::RING_HIGHEST,
                "{slot:?}"
            );
        }
        let reach: Vec<Solid> = home
            .slots(18)
            .iter()
            .map(|slot| {
                let (low, high) = home.reach(slot);
                Solid::new(low, high)
            })
            .collect();
        assert!(
            !reach
                .iter()
                .any(|worker| worker.blocks(arrival.eyes(), NOTICEBOARD.to_array()))
        );
        // Today's arc, which it stood in before and an older client still stands it in, fits too.
        let arc = Cluster::new(&Anchor::new(HOME, agent)).with_audience(arrival.eyes());
        assert_eq!(arc.arrange(18).step(), Step::Home);
        assert_eq!(fits(&arc, ARRIVAL.y, &solids), Ok(()));
    }

    #[test]
    fn the_workers_ring_the_agent_at_its_workbench_those_behind_it_over_the_bench_clear_of_the_rack_and_fence()
     {
        use tomori_environment::sightlines::{CLEAR, Spot};
        use tomori_protocol::Pose;
        use tomori_protocol::cluster::{Anchor, Cluster, Formation, HOME, RING, WORKER};
        let eyes = Spot::facing(ARRIVAL.to_array(), ARRIVAL_FACING.to_array()).eyes();
        let agent = Pose::at(AGENT_HOME.to_array()).facing_towards(eyes);
        let home = Cluster::new(
            &Anchor::new(HOME, agent)
                .with_formation(Formation::Ring)
                .with_sweep(HOME_SWEEP),
        )
        .with_audience(eyes);
        let middle = Vec2::new(AGENT_HOME.x, AGENT_HOME.z);
        let towards = on_workbench(1.0, 0.0) - middle;
        // Its sweep is centred on the way the home faces, out of the corner, so that its gap is over the bench's middle.
        assert!((Vec2::from(home.faces()) + towards).length() < 1e-4);
        // How many of `count` stand behind the agent, as seen from the arrival, and how many of those over the bench,
        // along its curve.
        let stand = |count: usize| {
            let slots = home.slots(count);
            let (mut behind, mut over) = (0, 0);
            for (index, slot) in slots.iter().enumerate() {
                let at = Vec2::new(slot.position[0], slot.position[2]);
                assert!((at.distance(middle) - RING).abs() < 1e-4);
                assert!(!slot.more, "{count}: {index}");
                // None reaches another: a footprint's reach apart about the vertical, or a row apart.
                let footprint = WORKER.scaled(slot.scale);
                for other in &slots[index + 1..] {
                    let apart = at.distance(Vec2::new(other.position[0], other.position[2]));
                    let above = (slot.position[1] - other.position[1]).abs();
                    assert!(
                        apart >= footprint.width.hypot(footprint.depth) - 1e-3
                            || above >= footprint.height - 1e-4,
                        "{count}: {index} {apart} {above}"
                    );
                }
                // Each clear of the fence and of the lantern, the box it takes, with its clearance.
                let (low, high) = home.reach(slot);
                let reach = Footprint::new([low[0], low[2]], [high[0], high[2]]).grown(CLEAR);
                assert!(
                    FENCE.grown(-0.1).contains(reach.low) && FENCE.grown(-0.1).contains(reach.high)
                );
                let lantern = workbench_lantern();
                let hung =
                    Footprint::new([lantern.x, lantern.z], [lantern.x, lantern.z]).grown(0.1);
                let level = low[1] - CLEAR < lantern.y + 0.15 && lantern.y - 0.15 < high[1] + CLEAR;
                assert!(
                    !(level && reach.overlaps(&hung)),
                    "{count}: {index} {slot:?}"
                );
                // Those behind the agent stand above what lies on the bench; the rest in the yard about the agent, off
                // the bench; and none straight behind it, where the swarm would hide them from the arrival.
                let offset = at - middle;
                let turn = towards.perp_dot(offset).atan2(towards.dot(offset)).abs();
                assert!(turn > 0.3, "{count}: {index} {turn}");
                if offset.dot(towards) > 0.0 {
                    assert!(low[1] - CLEAR >= WORKBENCH_CLUTTER, "{count}: {index}");
                    behind += 1;
                    if turn <= WORKBENCH_HALF_SPAN + 2f32.to_radians() {
                        over += 1;
                    }
                } else {
                    assert!(workbench().iter().all(|piece| !piece.contains(at)));
                }
            }
            (behind, over)
        };
        // At full size, twelve, in two rows of six: of each, two behind the agent, over the bench either side of its
        // middle.
        assert!(!home.arrange(12).compact());
        assert_eq!(stand(12), (4, 4));
        // Compact, eighteen, ten to a row: of each, four behind the agent, two over the bench and two by its ends.
        assert!(home.arrange(18).compact());
        assert_eq!(stand(18), (8, 4));
    }

    #[test]
    fn the_notice_board_stands_against_the_fence_its_wall_on_its_back_read_from_the_earth_in_front()
    {
        use tomori_environment::sightlines::{READING, Solid, Spot, readable};
        let solids: Vec<Solid> = solids()
            .into_iter()
            .map(|(low, high)| Solid::new(low, high))
            .collect();
        let board = noticeboard();
        // Inside the fence, against its right side, square to it, and clear of everything else that stands in the yard.
        assert!(FENCE.grown(-0.3).contains(board.low) && FENCE.grown(-0.3).contains(board.high));
        assert!(FENCE.high.x - board.high.x < 0.35);
        assert_eq!(NOTICEBOARD_FACING.z, NOTICEBOARD.z);
        for obstacle in obstacles().iter().filter(|obstacle| **obstacle != board) {
            assert!(!board.grown(0.3).overlaps(obstacle), "{obstacle:?}");
        }
        // Its wall, two slots wide and about 1.3 by 0.5 m, stands just in front of its back, which frames it, below the
        // top of its posts, and the wall's middle is on neither side of the back's.
        let back = NOTICEBOARD_BACK;
        assert_eq!(NOTICEBOARD_GRID, [2, 1]);
        let gap = back[0][0] - NOTICEBOARD.x;
        assert!(gap > 0.0 && gap < 0.03, "{gap}");
        let (width, height) = (1.32, 0.5);
        assert!(back[1][2] - back[0][2] > width + 0.15 && back[1][1] - back[0][1] > height + 0.15);
        assert!((NOTICEBOARD.z - (back[0][2] + back[1][2]) / 2.0).abs() < 1e-3);
        assert!((NOTICEBOARD.y - (back[0][1] + back[1][1]) / 2.0).abs() < 1e-3);
        assert!(back[1][1] < NOTICEBOARD_HEIGHT);
        // Read from the earth in front of it, within reach, as the agent's panels go there for whoever asks from that
        // side of the yard; and the arrival's sightlines to the Tomori wall and the agent stay clear of it.
        let before = Vec3::new(NOTICEBOARD.x - 2.0, 0.0, NOTICEBOARD.z + 0.4);
        assert!(on(Vec2::new(before.x, before.z)));
        let reader = Spot::facing(before.to_array(), NOTICEBOARD.to_array());
        let wall = Spot::facing(NOTICEBOARD.to_array(), NOTICEBOARD_FACING.to_array());
        // Its row is narrower than the Tomori wall's, whose ends `readable` checks too, so its own ends stand clear.
        assert_eq!(readable(&reader, &wall, &solids), Ok(()));
        let eyes = Spot::facing(ARRIVAL.to_array(), ARRIVAL_FACING.to_array()).eyes();
        assert!(Vec3::from_array(eyes).distance(NOTICEBOARD) > READING);
    }
}
