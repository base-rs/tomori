// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The walkable ground of a space's environment, as the agent finds its way about it: the environment's navmesh, the
//! same `tomori-environment` derives for its clients, carried into the space's coordinates, and asked where its
//! floors are, which floor someone stands on, whether a point stands clear of it and whether a way passes through it.
//! See `design/environments.md`.
use std::sync::Arc;

use glam::{Vec2, Vec3};
use tomori_environment::navmesh::Navmesh;
use tomori_protocol::Pose;

/// How near a floor, above or below, the agent's middle comes at most, in metres, there or [`ABOUT`] from it: enough
/// that the swarm's lowest reach stays a quarter of a metre clear of the ground beneath it, and its highest as far below
/// a floor above it.
pub(crate) const GROUND: f32 = 0.5;

/// How far about the agent's middle, along each axis, the ground is looked for as it stands or goes, in metres: as far
/// as its swarm reaches towards and away from whoever it faces, with its margin.
pub(crate) const ABOUT: f32 = 0.3;

/// How far apart two floors are, at most, to be one level, in metres, as a slope or a step, rather than a deck above.
pub(crate) const LEVEL: f32 = 1.0;

/// How far beneath someone's eyes their floor is, at least, in metres: crouching, their eyes stand that high above it.
pub(crate) const CROUCH: f32 = 0.3;

/// How far about someone, horizontally, their floor is looked for where there is none beneath them, in metres, as
/// where they stand in their room just beyond the walkable ground, beside what they keep clear of.
pub(crate) const UNDERFOOT: f32 = 1.0;

/// How far apart, in metres, a way is sampled to keep it clear of the ground.
pub(crate) const SAMPLE: f32 = 0.2;

/// The offsets, horizontally, about a point that the ground is looked for at near it: eight ways round, half a metre
/// out and [`UNDERFOOT`] out.
fn rings() -> impl Iterator<Item = (f32, f32)> {
    [0.5, UNDERFOOT].into_iter().flat_map(|out| {
        (0..8).map(move |way| {
            let angle = way as f32 * std::f32::consts::FRAC_PI_4;
            (out * angle.sin(), out * angle.cos())
        })
    })
}

/// An environment's walkable ground in a space: its navmesh, in the environment's own coordinates, and the carriage
/// that carries them into the space's, where the environment says where participants arrive.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ground {
    mesh: Arc<Navmesh>,
    carriage: Option<Pose>,
}

impl Ground {
    /// `mesh`, in the environment's own coordinates, carried into a space's by `carriage`, where it has one.
    pub(crate) fn new(mesh: Arc<Navmesh>, carriage: Option<Pose>) -> Self {
        Self { mesh, carriage }
    }

    /// `point`, in the space's coordinates, in the environment's own.
    fn local(&self, point: [f32; 3]) -> Vec3 {
        match &self.carriage {
            None => Vec3::from(point),
            Some(carriage) => {
                Vec3::from(crate::environments::uncarried(carriage, &Pose::at(point)).position)
            }
        }
    }

    /// How far its heights are raised as it is carried into the space.
    fn rise(&self) -> f32 {
        self.carriage.map_or(0.0, |carriage| carriage.position[1])
    }

    /// The heights of its floors at `x` and `z`, in the space's coordinates, above and below, one for each triangle
    /// there.
    pub(crate) fn floors(&self, x: f32, z: f32) -> Vec<f32> {
        let local = self.local([x, 0.0, z]);
        let rise = self.rise();
        self.mesh
            .heights(Vec2::new(local.x, local.z))
            .map(|height| height + rise)
            .collect()
    }

    /// The floor someone whose eyes are at `eyes` stands on: the highest of the floors at least [`CROUCH`] beneath
    /// their eyes, under them; or, where there is none, or a floor within [`UNDERFOOT`] of them is more than [`LEVEL`]
    /// above it, as standing in their room just beyond a deck's edge, the highest of those; or none, as flying over
    /// nothing.
    pub(crate) fn underfoot(&self, eyes: [f32; 3]) -> Option<f32> {
        let beneath = |x: f32, z: f32| {
            self.floors(x, z)
                .into_iter()
                .filter(|floor| *floor <= eyes[1] - CROUCH)
                .reduce(f32::max)
        };
        let under = beneath(eyes[0], eyes[2]);
        let near = rings()
            .filter_map(|(dx, dz)| beneath(eyes[0] + dx, eyes[2] + dz))
            .reduce(f32::max);
        match (under, near) {
            (Some(under), Some(near)) if near > under + LEVEL => Some(near),
            (Some(under), _) => Some(under),
            (None, near) => near,
        }
    }

    /// The floor at `x` and `z` on the level of `level`, within [`LEVEL`] of it, the nearest where several are.
    pub(crate) fn level_at(&self, x: f32, z: f32, level: f32) -> Option<f32> {
        self.floors(x, z)
            .into_iter()
            .filter(|floor| (floor - level).abs() <= LEVEL)
            .min_by(|a, b| (a - level).abs().total_cmp(&(b - level).abs()))
    }

    /// Whether `x` and `z`, with no floor of their own on the level of `level`, are over or beside another level: at
    /// them, or within [`UNDERFOOT`] of them, there is a floor and none on that level, as over the yard beyond a deck's
    /// edge, or in the gap its rock cuts in the yard beside it; rather than over nothing near any other level, as over a
    /// meadow beside a path.
    pub(crate) fn off_level(&self, x: f32, z: f32, level: f32) -> bool {
        std::iter::once((0.0, 0.0)).chain(rings()).any(|(dx, dz)| {
            let floors = self.floors(x + dx, z + dz);
            !floors.is_empty() && floors.iter().all(|floor| (floor - level).abs() > LEVEL)
        })
    }

    /// Whether the agent's middle at `point` stands clear of the ground: no floor within [`GROUND`] above or below it,
    /// there or [`ABOUT`] from it along either axis, and, there or about it, not beneath a floor with no floor under
    /// it, as inside a raised level's rock or beside its side.
    pub(crate) fn clear_of(&self, point: [f32; 3]) -> bool {
        let [x, y, z] = point;
        [
            (0.0, 0.0),
            (ABOUT, 0.0),
            (-ABOUT, 0.0),
            (0.0, ABOUT),
            (0.0, -ABOUT),
        ]
        .into_iter()
        .all(|(dx, dz)| {
            let floors = self.floors(x + dx, z + dz);
            let buried =
                floors.iter().any(|floor| *floor > y) && !floors.iter().any(|floor| *floor <= y);
            !buried && floors.iter().all(|floor| (floor - y).abs() >= GROUND)
        })
    }

    /// Whether the segment from `from` to `to` passes through one of its floors, from above or below.
    pub(crate) fn crosses(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let start = self.local(from);
        let along = self.local(to) - start;
        along.length_squared() > 0.0 && self.mesh.meets(start, along, 1.0, true).is_some()
    }

    /// Whether the agent's middle, going straight from `from` to `to`, keeps clear of the ground: it passes through no
    /// floor, and, sampled every [`SAMPLE`], stands clear of it all the way, as [`Self::clear_of`] says; but where it
    /// sets off from somewhere not clear, as stuck in a floor, only from where it first stands clear.
    pub(crate) fn open(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let along: [f32; 3] = std::array::from_fn(|axis| to[axis] - from[axis]);
        let length = along.iter().map(|value| value * value).sum::<f32>().sqrt();
        let steps = (length / SAMPLE).ceil().max(1.0) as usize;
        let at = |step: usize| -> [f32; 3] {
            let share = step as f32 / steps as f32;
            std::array::from_fn(|axis| from[axis] + along[axis] * share)
        };
        match (0..=steps).find(|step| self.clear_of(at(*step))) {
            // Never clear of it, it has nowhere to escape to.
            None => false,
            Some(0) => !self.crosses(from, to) && (1..=steps).all(|step| self.clear_of(at(step))),
            Some(free) => {
                !self.crosses(at(free), to) && (free..=steps).all(|step| self.clear_of(at(step)))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
