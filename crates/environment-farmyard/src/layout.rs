// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where everything in the farmyard is, in the yard's own coordinates, metres, with the gate on the +Z side, the coop
//! towards −Z, and the ground at 0: the fence and its gate, the coop and its ramp, the water trough, the feed and the
//! hay; and where participants arrive, [`ARRIVAL`], where the agent's home is, [`AGENT_HOME`], and the Tomori wall's,
//! [`TOMORI_WALL`]. The scenery draws them here, the participants land where [`walkable`] says, and the flock keeps
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
/// Where the farmyard's own projection wall stands: on the right of the yard, short of the trough, at eye height, facing
/// home, for what the agent projects for whoever asks from that side of the yard. See `design/projection.md`.
pub const NOTICEBOARD: Vec3 = Vec3::new(3.6, 1.5, -5.4);
/// The hay bales, stacked in the back corner.
pub const HAY: Footprint = Footprint::new([3.0, -10.7], [4.6, -9.5]);

/// Where participants arrive in the yard, home: inside the gate, on the earth, facing [`ARRIVAL_FACING`], so that they
/// see at once the coop, the agent beside it and the Tomori wall. Travel from there is free.
pub const ARRIVAL: Vec3 = Vec3::new(0.6, 0.0, -2.4);
/// What participants face as they arrive: the coop's front corner, beside its ramp.
pub const ARRIVAL_FACING: Vec3 = Vec3::new(-1.6, 0.0, -7.0);
/// The agent's home: beside the coop's front corner, clear of the coop and its ramp, at the height of its eaves, about
/// 3.6 m from the eyes where participants arrive, facing them. It appears here as a space opens, and stays here until a
/// participant summons it; its workers stand about it.
pub const AGENT_HOME: Vec3 = Vec3::new(-1.0, 1.5, -5.6);
/// The Tomori wall's home, the middle of its row: ahead and to the right of where participants arrive, 2.4 m from
/// them, below their eyes, facing them, so that it reads from the arrival and hides neither the agent nor the coop.
pub const TOMORI_WALL: Vec3 = Vec3::new(1.4, 0.9, -4.67);

/// The solids of the yard that nothing stands inside nor is seen through, as `(least, greatest)` corners: the coop
/// with its roof's overhang, its ramp, the trough, the feeder and the hay, each as high as it stands.
pub fn solids() -> [([f32; 3], [f32; 3]); 5] {
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
    ]
}

/// What stands in the yard, which nobody lands on and nothing walks through: the coop and its ramp, the trough, the
/// feeder and the hay.
pub fn obstacles() -> [Footprint; 5] {
    [
        COOP,
        ramp(),
        TROUGH,
        Footprint::new(
            [FEEDER.x - FEEDER_RADIUS, FEEDER.y - FEEDER_RADIUS],
            [FEEDER.x + FEEDER_RADIUS, FEEDER.y + FEEDER_RADIUS],
        ),
        HAY,
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
    // In front of everything, from the gate to the ramp's foot.
    Footprint::new([-4.6, -5.9], [4.6, 1.2]),
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
            centre.distance(Vec3::new(0.0, 0.0, -2.35)) < 1e-5
                && size.distance(Vec2::new(9.2, 7.1)) < 1e-5
        );
        assert!(!WALKABLE[0].overlaps(&WALKABLE[1].grown(-0.1)));
    }

    #[test]
    fn the_tomori_wall_reads_and_the_agent_is_seen_from_where_participants_arrive() {
        use tomori_environment::sightlines::{Solid, Spot, readable, seen};
        let arrival = Spot::facing(ARRIVAL.to_array(), ARRIVAL_FACING.to_array());
        let eyes = arrival.eyes();
        let solids: Vec<Solid> = solids()
            .into_iter()
            .map(|(low, high)| Solid::new(low, high))
            .collect();
        // Each faces the eyes where participants arrive, as the server registers them.
        let wall = Spot::facing(TOMORI_WALL.to_array(), eyes);
        let agent = Spot::facing(AGENT_HOME.to_array(), eyes);
        assert_eq!(readable(&arrival, &wall, &solids), Ok(()));
        assert_eq!(seen(&arrival, &agent, &solids), Ok(()));
        // Inside the fence, and the agent by the coop, but not inside it nor over its ramp.
        let inside = FENCE.grown(-0.3);
        for at in [AGENT_HOME, TOMORI_WALL] {
            assert!(inside.contains(Vec2::new(at.x, at.z)), "{at}");
        }
        let near = Vec2::new(AGENT_HOME.x, AGENT_HOME.z);
        assert!(!COOP.grown(0.6).contains(near) && !ramp().grown(0.6).contains(near));
        assert!(COOP.grown(2.5).contains(near), "by the coop");
    }
}
