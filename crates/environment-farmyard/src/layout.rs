// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where everything in the farmyard is, in the yard's own coordinates, metres, with the gate on the +Z side, the coop
//! towards −Z, and the ground at 0: the fence and its gate, the coop and its ramp, the water trough, the feed and the
//! hay; and where participants arrive, [`ARRIVAL`], and where the agent's home is, [`AGENT_HOME`], with the Tomori
//! wall's home beneath it, as Tomori stands it by default. The scenery draws them here, the participants land where [`walkable`] says, and the flock keeps
//! clear of them, so that all three agree; Tomori carries them all into a space's coordinates, where the arrival is
//! home, at the origin facing −Z (see `design/environments.md`).
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
/// see at once the coop, the agent beside it and the Tomori wall beneath it. Travel from there is free.
pub const ARRIVAL: Vec3 = Vec3::new(0.6, 0.0, -2.4);
/// What participants face as they arrive: the coop's front corner, beside its ramp.
pub const ARRIVAL_FACING: Vec3 = Vec3::new(-1.6, 0.0, -7.0);
/// The agent's home: beside the coop's front corner, clear of the coop and its ramp, at the height of its eaves, about
/// 3.6 m from the eyes where participants arrive, facing them. It appears here as a space opens, and stays here until a
/// participant summons it; its workers stand about it, and the Tomori wall beneath it, as Tomori stands the wall
/// beneath the agent's home by default, the middle of its row 0.88 m below the swarm's middle, about 3.7 m from those
/// eyes, so that it reads from the arrival.
pub const AGENT_HOME: Vec3 = Vec3::new(-1.0, 1.5, -5.6);

/// The room the agent's workers take about its home, which Tomori's own server gives the farmyard's `home` cluster
/// anchor, standing in a ring there: across, up and along, in metres, a box about the agent's home. The ring, 1.4 m
/// about the swarm's middle, and the workers' signs, turned whichever way, need 4 m across and along; three rows, from a
/// little below the swarm's middle, 4.4 m up, so that eighteen workers stand on the ring in slots of their own, clear of
/// the coop's corner, 2.1 m from home. See `design/farmyard.md`.
pub const HOME_ROOM: [f32; 3] = [4.0, 4.4, 4.0];

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
/// with its roof's overhang, its ramp, the trough, the feeder, the hay and the notice board, each as high as it stands.
pub fn solids() -> [([f32; 3], [f32; 3]); 6] {
    let solid = |footprint: Footprint, height: f32| {
        (
            [footprint.low.x, 0.0, footprint.low.y],
            [footprint.high.x, height, footprint.high.y],
        )
    };
    [
        solid(COOP.grown(0.18), COOP_RIDGE + 0.1),
        solid(ramp(), COOP_FLOOR),
        solid(TROUGH, TROUGH_HEIGHT),
        solid(obstacles()[3], 0.35),
        solid(HAY, 1.2),
        solid(noticeboard(), NOTICEBOARD_HEIGHT),
    ]
}

/// What stands in the yard, which nobody lands on and nothing walks through: the coop and its ramp, the trough, the
/// feeder, the hay and the notice board.
pub fn obstacles() -> [Footprint; 6] {
    [
        COOP,
        ramp(),
        TROUGH,
        Footprint::new(
            [FEEDER.x - FEEDER_RADIUS, FEEDER.y - FEEDER_RADIUS],
            [FEEDER.x + FEEDER_RADIUS, FEEDER.y + FEEDER_RADIUS],
        ),
        HAY,
        noticeboard(),
    ]
}

/// The ramp's footprint, from the door to its foot.
pub fn ramp() -> Footprint {
    let half = RAMP_WIDTH * 0.5;
    Footprint::new([DOOR.x - half, DOOR.z], [RAMP_FOOT.x + half, RAMP_FOOT.z])
}

/// Where participants may land in the yard, inside the fence and clear of what stands in it, by a margin: the rectangles
/// a teleport profile's surfaces are made of, at the ground's height.
pub const WALKABLE: [Footprint; 4] = [
    // In front of everything, from the gate to the ramp's foot, short of the notice board.
    Footprint::new([-4.6, -5.9], [4.35, 1.2]),
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
        for obstacle in obstacles() {
            assert!(inside.contains(obstacle.low) && inside.contains(obstacle.high));
        }
        // Where participants arrive is walkable, and so is the spot to peek in at the coop's window from.
        assert!(WALKABLE[0].contains(Vec2::new(ARRIVAL.x, ARRIVAL.z)));
        let window = Vec2::new(COOP.high.x + 0.6, (WINDOW[0][0] + WINDOW[1][0]) * 0.5);
        assert!(WALKABLE.iter().any(|footprint| footprint.contains(window)));
        // The door is in the coop's front wall, and the ramp leads from it to the ground in front.
        assert_eq!(DOOR.z, COOP.high.y);
        assert!(COOP.contains(Vec2::new(DOOR.x, DOOR.z)));
        assert!(ramp().high.y > COOP.high.y);
        let (centre, size) = walkable()[0];
        assert!(
            centre.distance(Vec3::new(-0.125, 0.0, -2.35)) < 1e-5
                && size.distance(Vec2::new(8.95, 7.1)) < 1e-5
        );
        assert!(!WALKABLE[0].overlaps(&WALKABLE[1].grown(-0.1)));
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
        // Inside the fence, and the agent by the coop, but not inside it nor over its ramp.
        let inside = FENCE.grown(-0.3);
        assert!(inside.contains(Vec2::new(AGENT_HOME.x, AGENT_HOME.z)));
        let near = Vec2::new(AGENT_HOME.x, AGENT_HOME.z);
        assert!(!COOP.grown(0.6).contains(near) && !ramp().grown(0.6).contains(near));
        assert!(COOP.grown(2.5).contains(near), "by the coop");
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
        // The farmyard's home anchor stands its workers in a ring about the agent's home, in its room, gathered about
        // the eyes at the arrival, as Tomori's own server registers it.
        let agent = Pose::at(AGENT_HOME.to_array()).facing_towards(arrival.eyes());
        let [width, height, depth] = HOME_ROOM;
        let anchor = Anchor::new(HOME, agent)
            .with_formation(Formation::Ring)
            .with_extent(width, height, depth);
        let home = Cluster::new(&anchor).with_audience(arrival.eyes());
        assert_eq!(home.arrange(18).step(), Step::Ring);
        assert!(!home.arrange(18).compact());
        assert_eq!(fits(&home, ARRIVAL.y, &solids), Ok(()));
        // Today's arc, which it stood in before, fits too, should a build stand it there.
        let arc = Cluster::new(&Anchor::new(HOME, agent)).with_audience(arrival.eyes());
        assert_eq!(arc.arrange(18).step(), Step::Home);
        assert_eq!(fits(&arc, ARRIVAL.y, &solids), Ok(()));
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
        for obstacle in &obstacles()[..5] {
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
