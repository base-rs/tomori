// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whether participants arriving in an environment can read its Tomori wall and see its agent: the rule every
//! environment's tests check of the homes it gives them, in its own coordinates, with every anchor facing along its +Z.
//! The Tomori wall's home stands within comfortable reading distance of the eyes at the arrival, [`READING`], as the
//! agent's panels are put only on a wall that near (see `design/projection.md`), faces them and stands clear of the
//! scenery's solids; and the agent's home does the same, with nothing in the way of seeing it. See
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

/// Whether the Tomori wall's home, `wall`, the middle of its row facing along its +Z, can be read from `arrival`:
/// within [`READING`] of the eyes there, facing them within [`FACING`], its middle and both ends of its row, within
/// [`WALL_REACH`], clear of `solids`, and nothing of them between the eyes and its middle.
pub fn readable(arrival: &Spot, wall: &Spot, solids: &[Solid]) -> Result<(), String> {
    check(
        "The Tomori wall",
        arrival,
        wall,
        &[[0.0; 3], [-WALL_REACH, 0.0, 0.0], [WALL_REACH, 0.0, 0.0]],
        solids,
    )
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
        // A turned arrival carries its eyes with it.
        let turned = Spot::facing([2.0, 0.5, 0.0], [0.0, 0.5, 0.0]);
        assert_eq!(turned.eyes(), [2.0, 2.1, 0.0]);
        assert!((turned.place([0.0, 0.0, 1.0])[0] - 1.0).abs() < 1e-6);
    }
}
