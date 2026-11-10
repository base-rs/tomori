// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whether participants arriving in an environment can read its status wall and see its agent: the rule every
//! environment's tests check of the homes it gives them, in its own coordinates, with every anchor facing along its +Z.
//! The status wall's home, where the space's own stands, beneath the agent's home or where the environment stands it,
//! stands within comfortable reading distance of the eyes at the arrival, [`READING`], as the agent's panels are put
//! only on a wall that near (see `design/projection.md`), faces them and stands clear of the scenery's solids; one of
//! the environment's own walls in the role is read by walking up to it, and seen from the arrival ([`noticed`]); and the
//! agent's home stands within reach of the eyes too, with nothing in the way of seeing it. See
//! `design/environments.md`.
//!
//! ```
//! use tomori_environment::sightlines::{Spot, readable, seen};
//!
//! // Tomori's default layout: home at the origin facing −Z, the agent 2.3 m ahead and the wall below it.
//! let arrival = Spot::facing([0.0; 3], [0.0, 0.0, -1.0]);
//! let agent = Spot::facing([0.0, 1.4, -2.3], [0.0, 1.4, 0.0]);
//! let wall = Spot::facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]);
//! assert_eq!(readable(&arrival, &wall, &[]), Ok(()));
//! assert_eq!(seen(&arrival, &agent, &[]), Ok(()));
//! ```

/// How high above the floor where participants arrive their eyes are taken to be, in metres.
pub const EYES: f32 = 1.6;

/// How far from the eyes at the arrival the Tomori wall's home and the agent's may be, in metres: as far as a wall's
/// text can be read, as the agent's panels are put only on a wall within it.
pub const READING: f32 = 4.0;

/// How far from facing the eyes at the arrival the Tomori wall and the agent may turn, in radians: 30°.
pub const FACING: f32 = std::f32::consts::FRAC_PI_6;

/// How far to either side of its middle the Tomori wall's row of slots reaches, in metres, which must stand clear too.
pub const WALL_REACH: f32 = 1.3;

/// How far in front of an environment's own wall in the status role someone reads it from, in metres, having walked up
/// to it.
pub const BEFORE: f32 = 2.0;

/// A place and the way it faces about the vertical, its +Z, as every anchor faces: `yaw` is 0 along +Z and a quarter
/// turn along +X.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub position: [f32; 3],
    pub yaw: f32,
}

impl Spot {
    /// At `position`, facing `target`, levelled.
    pub fn facing(position: [f32; 3], target: [f32; 3]) -> Self {
        let (x, z) = (target[0] - position[0], target[2] - position[2]);
        Self {
            position,
            yaw: if x == 0.0 && z == 0.0 {
                0.0
            } else {
                x.atan2(z)
            },
        }
    }

    /// At `position`, facing `yaw`.
    pub fn new(position: [f32; 3], yaw: f32) -> Self {
        Self { position, yaw }
    }

    /// The point `local`, in its own frame, its +Z ahead, in the frame it is given in.
    pub fn place(&self, local: [f32; 3]) -> [f32; 3] {
        let (sin, cos) = self.yaw.sin_cos();
        let [x, y, z] = local;
        let [px, py, pz] = self.position;
        [px + x * cos + z * sin, py + y, pz - x * sin + z * cos]
    }

    /// Where the eyes of someone standing here are.
    pub fn eyes(&self) -> [f32; 3] {
        self.place([0.0, EYES, 0.0])
    }
}

/// A box of the scenery's that nothing should stand inside nor see through, aligned with the environment's axes, from
/// its least to its greatest corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solid {
    pub low: [f32; 3],
    pub high: [f32; 3],
}

impl Solid {
    pub const fn new(low: [f32; 3], high: [f32; 3]) -> Self {
        Self { low, high }
    }

    /// Whether `point` is inside it.
    pub fn contains(&self, point: [f32; 3]) -> bool {
        (0..3).all(|axis| self.low[axis] <= point[axis] && point[axis] <= self.high[axis])
    }

    /// Whether the segment from `from` to `to` passes through it.
    pub fn blocks(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let (mut near, mut far) = (0.0f32, 1.0f32);
        for axis in 0..3 {
            let delta = to[axis] - from[axis];
            if delta.abs() < 1e-9 {
                if from[axis] < self.low[axis] || from[axis] > self.high[axis] {
                    return false;
                }
                continue;
            }
            let (a, b) = (
                (self.low[axis] - from[axis]) / delta,
                (self.high[axis] - from[axis]) / delta,
            );
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
        true
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3)
        .map(|axis| (a[axis] - b[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Whether `spot`, named `what`, stands within [`READING`] of the eyes at `arrival`, faces them within [`FACING`],
/// and stands clear of `solids` at each of `points`, in its own frame, with nothing in the way between the eyes and
/// its middle.
fn check(
    what: &str,
    arrival: &Spot,
    spot: &Spot,
    points: &[[f32; 3]],
    solids: &[Solid],
) -> Result<(), String> {
    let eyes = arrival.eyes();
    let away = distance(eyes, spot.position);
    if away.is_nan() || away > READING {
        return Err(format!(
            "{what} is {away:.1} m from the eyes where participants arrive, beyond {READING} m."
        ));
    }
    let towards = Spot::facing(spot.position, eyes).yaw;
    let off = (spot.yaw - towards)
        .sin()
        .atan2((spot.yaw - towards).cos())
        .abs();
    if off > FACING {
        return Err(format!(
            "{what} faces {:.0}° away from where participants arrive.",
            off.to_degrees()
        ));
    }
    for point in points.iter().map(|local| spot.place(*local)) {
        if let Some(solid) = solids.iter().find(|solid| solid.contains(point)) {
            return Err(format!("{what} stands inside {solid:?}."));
        }
    }
    if let Some(solid) = solids
        .iter()
        .find(|solid| solid.blocks(eyes, spot.position))
    {
        return Err(format!(
            "{solid:?} stands between where participants arrive and {what}."
        ));
    }
    Ok(())
}

/// Whether the status wall's home, where the space's own stands, `wall`, the middle of its row facing along its +Z,
/// can be read from `arrival`:
/// within [`READING`] of the eyes there, facing them within [`FACING`], its middle and both ends of its row, within
/// [`WALL_REACH`], clear of `solids`, and nothing of them between the eyes and its middle.
pub fn readable(arrival: &Spot, wall: &Spot, solids: &[Solid]) -> Result<(), String> {
    check(
        "The status wall",
        arrival,
        wall,
        &[[0.0; 3], [-WALL_REACH, 0.0, 0.0], [WALL_REACH, 0.0, 0.0]],
        solids,
    )
}

/// Whether one of an environment's own walls in the status role, `wall`, the middle of its slots facing along its +Z,
/// can be read by walking up to it, and is seen from `arrival`, though perhaps too far to read from there: read, as
/// [`readable`] reads the Tomori wall, by someone standing on the floor at `floor`, [`BEFORE`] in front of it, outside
/// `solids`; and, from the eyes at the arrival, nothing of `solids` between them and its middle, its face turned
/// towards them rather than its back.
pub fn noticed(arrival: &Spot, wall: &Spot, floor: f32, solids: &[Solid]) -> Result<(), String> {
    let mut before = wall.place([0.0, 0.0, BEFORE]);
    before[1] = floor;
    let reader = Spot::facing(before, wall.position);
    if let Some(solid) = solids
        .iter()
        .find(|solid| solid.contains(before) || solid.contains(reader.eyes()))
    {
        return Err(format!(
            "The ground before the status wall stands inside {solid:?}."
        ));
    }
    readable(&reader, wall, solids)?;
    let eyes = arrival.eyes();
    let towards = Spot::facing(wall.position, eyes).yaw;
    let off = (wall.yaw - towards)
        .sin()
        .atan2((wall.yaw - towards).cos())
        .abs();
    if off >= std::f32::consts::FRAC_PI_2 {
        return Err("The status wall turns its back on where participants arrive.".to_owned());
    }
    if let Some(solid) = solids
        .iter()
        .find(|solid| solid.blocks(eyes, wall.position))
    {
        return Err(format!(
            "{solid:?} stands between where participants arrive and the status wall."
        ));
    }
    Ok(())
}

/// How many workers a cluster anchor's room must hold, each in a slot of its own: one to this many.
pub const MEMBERS: usize = 18;

/// How far clear of the scenery's solids each member of a cluster stands, about the box it takes, in metres.
pub const CLEAR: f32 = 0.1;

/// Whether `solid` and the box from `low` to `high` overlap.
fn overlaps(solid: &Solid, low: [f32; 3], high: [f32; 3]) -> bool {
    (0..3).all(|axis| low[axis] < solid.high[axis] && solid.low[axis] < high[axis])
}

/// Whether one to [`MEMBERS`] workers fit `cluster`, an anchor's cluster in the environment's own coordinates, as
/// `tomori_protocol::cluster` lays it out, gathered about its audience: for each number, every one in a slot of its
/// own rather than gathered into the "+N more" sign, within the cluster's room, as the rule keeps it; the box each
/// takes, [`CLEAR`] larger on every side, clear of `solids` and above the floor at `floor`; and nothing of them between
/// its audience's eyes and it, so that it reads from there, as signs grow with distance. For the `home` anchor, give the
/// cluster the eyes at the arrival as its audience, [`Spot::eyes`]. As [`readable`] checks walls. See
/// `design/environments.md`.
///
/// ```
/// use tomori_environment::sightlines::{Solid, Spot, fits};
/// use tomori_protocol::Pose;
/// use tomori_protocol::cluster::{Anchor, Cluster};
///
/// let arrival = Spot::facing([0.0; 3], [0.0, 0.0, -1.0]);
/// let pose = Pose::at([0.0, 1.4, -2.3]).facing_towards(arrival.eyes());
/// // Today's arc about the default home fits an empty yard; not one with a shed beside the agent.
/// let home = Cluster::new(&Anchor::new("home", pose)).with_audience(arrival.eyes());
/// assert_eq!(fits(&home, 0.0, &[]), Ok(()));
/// let shed = Solid::new([1.0, 0.0, -2.5], [3.0, 2.5, -0.5]);
/// assert!(fits(&home, 0.0, &[shed]).is_err());
/// ```
pub fn fits(
    cluster: &tomori_protocol::cluster::Cluster,
    floor: f32,
    solids: &[Solid],
) -> Result<(), String> {
    let eyes = cluster.audience();
    for count in 1..=MEMBERS {
        let slots = cluster.slots(count);
        if slots.iter().any(|slot| slot.more) {
            return Err(format!(
                "Its room holds {} workers in slots of their own, fewer than {count}.",
                slots.iter().filter(|slot| !slot.more).count()
            ));
        }
        for (index, slot) in slots.iter().enumerate() {
            let (low, high) = cluster.reach(slot);
            let low = low.map(|value| value - CLEAR);
            let high = high.map(|value| value + CLEAR);
            if let Some(solid) = solids.iter().find(|solid| overlaps(solid, low, high)) {
                return Err(format!(
                    "Worker {} of {count} stands within {CLEAR} m of {solid:?}.",
                    index + 1
                ));
            }
            if low[1] < floor {
                return Err(format!(
                    "Worker {} of {count} reaches below the floor.",
                    index + 1
                ));
            }
            if let Some(solid) = solids
                .iter()
                .find(|solid| solid.blocks(eyes, slot.position))
            {
                return Err(format!(
                    "{solid:?} stands between whoever watches and worker {} of {count}.",
                    index + 1
                ));
            }
        }
    }
    Ok(())
}

/// Whether the agent's home, `agent`, the middle of the swarm facing along its +Z, can be seen from `arrival`: within
/// [`READING`] of the eyes there, facing them within [`FACING`], its middle clear of `solids`, and nothing of them
/// between the eyes and it.
pub fn seen(arrival: &Spot, agent: &Spot, solids: &[Solid]) -> Result<(), String> {
    check("The agent", arrival, agent, &[[0.0; 3]], solids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_too_far_turned_away_inside_or_hidden_is_refused() {
        let arrival = Spot::facing([0.0; 3], [0.0, 0.0, -1.0]);
        let wall = |position: [f32; 3]| Spot::facing(position, [0.0, position[1], 0.0]);
        assert_eq!(readable(&arrival, &wall([0.0, 1.2, -2.5]), &[]), Ok(()));
        // Too far.
        assert!(
            readable(&arrival, &wall([0.0, 1.2, -7.0]), &[])
                .unwrap_err()
                .contains("beyond 4 m")
        );
        // Turned away: facing along −Z, its back to the arrival.
        let turned = Spot::new([0.0, 1.2, -2.5], std::f32::consts::PI);
        assert!(
            readable(&arrival, &turned, &[])
                .unwrap_err()
                .contains("faces 180°")
        );
        // Inside a box, at its middle or an end of its row.
        let coop = Solid::new([-1.0, 0.0, -3.0], [1.0, 2.0, -2.0]);
        assert!(
            readable(&arrival, &wall([0.0, 1.2, -2.5]), &[coop])
                .unwrap_err()
                .contains("inside")
        );
        let shed = Solid::new([1.0, 0.0, -3.0], [2.0, 2.0, -2.0]);
        assert!(
            readable(&arrival, &wall([0.0, 1.2, -2.5]), &[shed])
                .unwrap_err()
                .contains("inside")
        );
        // Hidden behind a box, as the agent behind a coop.
        let agent = Spot::facing([0.0, 1.5, -3.5], [0.0, 1.5, 0.0]);
        assert_eq!(seen(&arrival, &agent, &[shed]), Ok(()));
        assert!(
            seen(&arrival, &agent, &[coop])
                .unwrap_err()
                .contains("between")
        );
        // Facing itself, it is not turned.
        assert_eq!(Spot::facing([1.0, 0.0, 1.0], [1.0, 5.0, 1.0]).yaw, 0.0);
        // A board of an environment's own, far along a fence and facing into the yard, is read by walking up to it, and
        // seen from the arrival; not with its back to it, nor with a shed in the way, nor with the ground before it
        // inside one.
        let board = Spot::facing([6.0, 1.45, -4.0], [0.0, 1.45, -4.0]);
        assert_eq!(noticed(&arrival, &board, 0.0, &[]), Ok(()));
        let back = Spot::facing([6.0, 1.45, -4.0], [9.0, 1.45, -4.0]);
        assert!(
            noticed(&arrival, &back, 0.0, &[])
                .unwrap_err()
                .contains("back")
        );
        let barn = Solid::new([2.0, 0.0, -3.0], [3.0, 3.0, -2.0]);
        assert!(
            noticed(&arrival, &board, 0.0, &[barn])
                .unwrap_err()
                .contains("between")
        );
        let trough = Solid::new([3.5, 0.0, -4.5], [4.5, 1.0, -3.5]);
        assert!(
            noticed(&arrival, &board, 0.0, &[trough])
                .unwrap_err()
                .contains("ground")
        );
        // A post between where someone stands before it and its middle hides it.
        let post = Solid::new([4.8, 1.0, -4.1], [5.2, 2.0, -3.9]);
        assert!(
            noticed(&arrival, &board, 0.0, &[post])
                .unwrap_err()
                .contains("status wall")
        );
        // A turned arrival carries its eyes with it.
        let turned = Spot::facing([2.0, 0.5, 0.0], [0.0, 0.5, 0.0]);
        assert_eq!(turned.eyes(), [2.0, 2.1, 0.0]);
        assert!((turned.place([0.0, 0.0, 1.0])[0] - 1.0).abs() < 1e-6);
    }
}

#[cfg(test)]
mod fit {
    use super::*;
    use tomori_protocol::Pose;
    use tomori_protocol::cluster::{Anchor, Cluster, HOME};

    fn home(arrival: &Spot, pose: Pose) -> Cluster {
        Cluster::new(&Anchor::new(HOME, pose)).with_audience(arrival.eyes())
    }

    #[test]
    fn one_to_eighteen_workers_fit_clear_of_solids_above_the_floor_seen_and_in_their_room() {
        let arrival = Spot::facing([0.0; 3], [0.0, 0.0, -1.0]);
        let pose = Pose::at([0.0, 1.4, -2.3]).facing_towards(arrival.eyes());
        assert_eq!(fits(&home(&arrival, pose), 0.0, &[]), Ok(()));
        // A box beside the agent, within 0.1 m of a worker's reach.
        let crate_ = Solid::new([-2.6, 0.0, -1.6], [-2.2, 1.2, -1.2]);
        assert!(
            fits(&home(&arrival, pose), 0.0, &[crate_])
                .unwrap_err()
                .contains("within 0.1 m")
        );
        // A post between the eyes and a worker.
        let post = Solid::new([1.0, 1.0, -0.8], [1.1, 1.5, -0.6]);
        assert!(
            fits(&home(&arrival, pose), 0.0, &[post])
                .unwrap_err()
                .contains("between")
        );
        // A home too low, its first row below the floor of a raised arrival.
        assert!(
            fits(&home(&arrival, pose), 1.2, &[])
                .unwrap_err()
                .contains("floor")
        );
        // A room too small for eighteen workers in slots of their own.
        let small = Cluster::new(&Anchor::new(HOME, pose).with_extent(3.0, 1.0, 1.0))
            .with_audience(arrival.eyes());
        assert!(fits(&small, 0.0, &[]).unwrap_err().contains("fewer than"));
        // A room large enough holds them, fitted.
        let yard = Pose::at([0.0, 1.6, -5.0]).facing_towards(arrival.eyes());
        let roomy = Cluster::new(&Anchor::new(HOME, yard).with_extent(10.0, 3.0, 4.0))
            .with_audience(arrival.eyes());
        assert_eq!(fits(&roomy, 0.0, &[]), Ok(()));
        // A named anchor's, gathered about its audience before it.
        let aisle = Cluster::new(&Anchor::new("aisle", yard).with_extent(2.4, 2.0, 12.0));
        assert_eq!(fits(&aisle, 0.0, &[]), Ok(()));
        assert_eq!(tomori_protocol::cluster::NAME_BYTES, crate::REFERENCE_BYTES);
    }
}
