// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where everything in the farmyard is, in the yard's own coordinates, metres, with the gate on the +Z side, the coop
//! towards −Z, and the ground at 0: the fence and its gate, the coop and its ramp, the water trough, the feed, the hay,
//! the notice board and the workbench in the front left corner; and where participants arrive, [`ARRIVAL`], and where
//! the agent's home is, [`AGENT_HOME`], at the heart of the workbench's curve. The scenery draws them here, the
//! participants land where [`walkable`] says, and the flock keeps clear of them, so that all three agree; Tomori
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
/// How tall the fence's posts are.
pub const FENCE_HEIGHT: f32 = 1.0;

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
/// The lantern hanging from the coop's eave, beside its door.
pub const LANTERN: Vec3 = Vec3::new(-2.15, 1.75, -7.65);
/// The lantern on the gate's post.
pub const GATE_LANTERN: Vec3 = Vec3::new(1.0, 1.25, 1.6);

/// The water trough.
pub const TROUGH: Footprint = Footprint::new([3.0, -8.5], [4.4, -8.05]);
pub const TROUGH_HEIGHT: f32 = 0.38;
/// The feeder, a round tray the grain is scattered about, and how far the grain is scattered.
pub const FEEDER: Vec2 = Vec2::new(0.8, -6.8);
pub const FEEDER_RADIUS: f32 = 0.28;
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
/// The hay bales, stacked in the back corner.
pub const HAY: Footprint = Footprint::new([3.0, -10.7], [4.6, -9.5]);

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
        solid(feeder, 0.35),
        solid(HAY, 1.2),
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

/// Where participants may land in the yard, inside the fence and clear of what stands in it, by a margin: the rectangles
/// a teleport profile's surfaces are made of, at the ground's height.
pub const WALKABLE: [Footprint; 6] = [
    // In front of everything, from the ramp's foot, short of the notice board, to short of the workbench.
    Footprint::new([-4.6, -5.9], [4.35, -2.0]),
    // From there to the gate, beside the workbench, the agent's home and the ring about it included.
    Footprint::new([-2.65, -2.0], [4.35, 1.2]),
    // Within the workbench's curve, before its left half.
    Footprint::new([-3.7, -2.0], [-2.65, -0.6]),
    // Between the coop and the trough and hay, past the feeder, to the back fence, and beside the coop's window.
    Footprint::new([-1.4, -10.6], [0.3, -5.9]),
    Footprint::new([1.3, -10.6], [2.6, -5.9]),
    // Beside the trough, towards the fence.
    Footprint::new([2.6, -7.7], [4.6, -5.9]),
];

/// [`WALKABLE`]'s rectangles, as `(centre, size)` on the ground, for a teleport profile.
pub fn walkable() -> Vec<(Vec3, Vec2)> {
    WALKABLE
        .iter()
        .map(|footprint| {
            let centre = footprint.centre();
            (Vec3::new(centre.x, 0.0, centre.y), footprint.size())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_lands_on_anything_and_everything_is_inside_the_fence() {
        let inside = FENCE.grown(-0.3);
        for footprint in WALKABLE {
            assert!(inside.contains(footprint.low) && inside.contains(footprint.high));
            for obstacle in obstacles() {
                assert!(
                    !footprint.overlaps(&obstacle.grown(0.2)),
                    "{footprint:?} {obstacle:?}"
                );
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
        assert!(WALKABLE[0].contains(Vec2::new(ARRIVAL.x, ARRIVAL.z)));
        let window = Vec2::new(COOP.high.x + 0.6, (WINDOW[0][0] + WINDOW[1][0]) * 0.5);
        assert!(WALKABLE.iter().any(|footprint| footprint.contains(window)));
        assert!(
            WALKABLE
                .iter()
                .any(|footprint| footprint.contains(WORKBENCH))
        );
        // The door is in the coop's front wall, and the ramp leads from it to the ground in front.
        assert_eq!(DOOR.z, COOP.high.y);
        assert!(COOP.contains(Vec2::new(DOOR.x, DOOR.z)));
        assert!(ramp().high.y > COOP.high.y);
        let (centre, size) = walkable()[0];
        assert!(
            centre.distance(Vec3::new(-0.125, 0.0, -3.95)) < 1e-5
                && size.distance(Vec2::new(8.95, 3.9)) < 1e-5
        );
        // No two of the rectangles overlap.
        for (index, footprint) in WALKABLE.iter().enumerate() {
            for other in &WALKABLE[index + 1..] {
                assert!(!footprint.overlaps(other), "{footprint:?} {other:?}");
            }
        }
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
        assert!(
            WALKABLE
                .iter()
                .any(|footprint| footprint.contains(Vec2::new(before.x, before.z)))
        );
        let reader = Spot::facing(before.to_array(), NOTICEBOARD.to_array());
        let wall = Spot::facing(NOTICEBOARD.to_array(), NOTICEBOARD_FACING.to_array());
        // Its row is narrower than the Tomori wall's, whose ends `readable` checks too, so its own ends stand clear.
        assert_eq!(readable(&reader, &wall, &solids), Ok(()));
        let eyes = Spot::facing(ARRIVAL.to_array(), ARRIVAL_FACING.to_array()).eyes();
        assert!(Vec3::from_array(eyes).distance(NOTICEBOARD) > READING);
    }
}
