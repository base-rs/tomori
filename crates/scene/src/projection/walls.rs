// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Each wall's own faces, kept across frames while what they are drawn from is the same: a flat wall's board, a curved
//! wall's slot board, which each of its slots shows, and its label, a flat wall's of a group drawn edge by edge, and the
//! outline of the slot a panel carried would take. They are handed to the renderer as the very canvases drawn before,
//! which it knows by their pointers, so that a wall moved, the head turning, or the walls changing their order by
//! distance from it draws none of them again, and the renderer builds none of their meshes again. A wall's are drawn
//! again only when its layout, its label or its group changes, and forgotten once it is no longer shown. See
//! `design/rendering.md`.
use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use tomori_protocol::{Pose, Wall, WallKind};

use super::{Cells, arc_label, boards, ghost, slot_board, wall_border, wall_face};
use crate::{canvas::Canvas, frame_timing::drawn};

/// A canvas drawn from `K`, kept while it is drawn from the same.
type Kept<K, T = Arc<Canvas>> = Option<(K, T)>;

/// What a flat wall's board of a group is drawn from: the wall, wherever it stands, and each other wall of its group,
/// with where it stands from the wall, since the boards are drawn in the wall's own frame.
#[derive(Clone, Debug)]
struct Group {
    wall: Wall,
    members: Vec<(Wall, Mat4)>,
}

/// How near two walls of a group must stand from each other as they stood, in metres and as a rotation's components,
/// to be drawn alike: their poses as a group carried moves them together, each computed afresh, differ by rounding.
const SAME: f32 = 1e-4;

/// One wall's faces, each kept by what it is drawn from.
#[derive(Clone, Debug, Default)]
struct Faces {
    /// A flat wall's board, by the wall as laid out, wherever it stands.
    board: Kept<Wall>,
    /// A curved wall's slot board, by its frame's colour and its slots' cells.
    slot: Kept<([f32; 4], Cells)>,
    /// A curved wall's label, by its kind and name.
    label: Kept<(WallKind, Option<String>)>,
    /// A flat wall's of a group, its board and its label each with where it stands from the wall's frame.
    group: Kept<Group, Vec<(Arc<Canvas>, Mat4)>>,
    /// The outline of a slot of its, by the slot's cells.
    ghost: Kept<Cells>,
}

/// The faces of the walls shown, by each wall's identity.
#[derive(Clone, Debug, Default)]
pub(super) struct WallFaces {
    walls: HashMap<String, Faces>,
}

/// What `kept` holds if `same` finds it drawn from what it would be drawn from now, compared in place; or else what is
/// `draw`n now from `key()`, and kept.
fn kept<K, T: Clone>(
    kept: &mut Kept<K, T>,
    same: impl FnOnce(&mut K) -> bool,
    key: impl FnOnce() -> K,
    draw: impl FnOnce(&K) -> T,
) -> T {
    if let Some((from, drawn)) = kept
        && same(from)
    {
        return drawn.clone();
    }
    let key = key();
    let drawn = draw(&key);
    *kept = Some((key, drawn.clone()));
    drawn
}

/// Whether `kept` is laid out as `wall` is, wherever each stands and whatever its version: `kept` takes `wall`'s pose
/// and version, so that the rest is compared in place.
fn laid_out(kept: &mut Wall, wall: &Wall) -> bool {
    kept.pose = wall.pose;
    kept.version = wall.version;
    *kept == *wall
}

/// Where `pose` stands from `from`.
fn relative(from: &Pose, pose: &Pose) -> Mat4 {
    let model = |pose: &Pose| {
        Mat4::from_rotation_translation(
            Quat::from_array(pose.orientation),
            Vec3::from_array(pose.position),
        )
    };
    model(from).inverse() * model(pose)
}

impl WallFaces {
    /// Forget the faces of every wall but those of `shown`, by identity.
    pub(super) fn retain(&mut self, shown: &[&Wall]) {
        self.walls
            .retain(|id, _| shown.iter().any(|wall| wall.id == *id));
    }

    fn of(&mut self, wall: &Wall) -> &mut Faces {
        if !self.walls.contains_key(&wall.id) {
            self.walls.insert(wall.id.clone(), Faces::default());
        }
        self.walls.get_mut(&wall.id).expect("just kept")
    }

    /// A flat wall's own board, as [`wall_face`] draws it.
    pub(super) fn board(&mut self, wall: &Wall) -> Arc<Canvas> {
        kept(
            &mut self.of(wall).board,
            |kept| laid_out(kept, wall),
            || wall.clone(),
            |wall| drawn(wall_face(wall)),
        )
    }

    /// The board each of a curved wall's slots shows, as [`slot_board`] draws it.
    pub(super) fn slot(&mut self, wall: &Wall) -> Arc<Canvas> {
        let key = (wall_border(wall), Cells::of(wall));
        kept(
            &mut self.of(wall).slot,
            |kept| *kept == key,
            || key,
            |(border, cells)| drawn(slot_board(*border, *cells)),
        )
    }

    /// A curved wall's label, as [`arc_label`] draws it.
    pub(super) fn label(&mut self, wall: &Wall) -> Arc<Canvas> {
        kept(
            &mut self.of(wall).label,
            |(kind, name)| *kind == wall.kind && *name == wall.name,
            || (wall.kind, wall.name.clone()),
            |_| drawn(arc_label(wall)),
        )
    }

    /// A flat wall's board and label as one of its group among `walls`, as [`boards::sheets`] draws them, each with
    /// where it stands from the wall's frame: kept as the group is carried, its walls standing as they stood from each
    /// other.
    pub(super) fn group(&mut self, wall: &Wall, walls: &[&Wall]) -> Vec<(Arc<Canvas>, Mat4)> {
        let members = boards::members(wall, walls);
        kept(
            &mut self.of(wall).group,
            |kept| {
                laid_out(&mut kept.wall, wall)
                    && kept.members.len() == members.len()
                    && kept
                        .members
                        .iter_mut()
                        .zip(&members)
                        .all(|((kept, at), other)| {
                            laid_out(kept, other)
                                && at.abs_diff_eq(relative(&wall.pose, &other.pose), SAME)
                        })
            },
            || Group {
                wall: wall.clone(),
                members: members
                    .iter()
                    .map(|other| ((*other).clone(), relative(&wall.pose, &other.pose)))
                    .collect(),
            },
            |_| {
                let sheets = boards::sheets(wall, Mat4::IDENTITY, walls);
                crate::frame_timing::faces(sheets.len());
                sheets
                    .into_iter()
                    .map(|sheet| {
                        let canvas = sheet.canvas.expect("a board has a face");
                        (canvas, sheet.model.expect("a board stands"))
                    })
                    .collect()
            },
        )
    }

    /// The outline of a slot of `wall`'s, as [`ghost`] draws it.
    pub(super) fn ghost(&mut self, wall: &Wall) -> Arc<Canvas> {
        let cells = Cells::of(wall);
        kept(
            &mut self.of(wall).ghost,
            |kept| *kept == cells,
            || cells,
            |cells| drawn(ghost(*cells)),
        )
    }
}

#[cfg(test)]
mod tests;
