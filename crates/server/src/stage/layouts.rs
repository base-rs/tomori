// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Layouts the agent composes: several walls built in one step as one snapped group, each after the first attached to
//! an earlier one, the whole placed where someone can read it, or refused whole. See `design/projection.md`.
use tomori_protocol::{Align, Attach, Edge, Pose, Wall, WallKind, group, wall as layout};

use super::{Layout, Stage, Walls};

/// The most walls one layout has: six, as many as an environment took added before frames were negotiated, so that a
/// layout stays one group that a participant takes in at once, however many more walls are added.
pub const LAYOUT_WALLS: usize = 6;

/// One wall of a layout, as the agent asks for it: its own name for it, how it is laid out, the width in metres it is to
/// be instead of its scale, whether it said its slots' size at all, and, for every wall but the first, the earlier wall it
/// is attached to, by the agent's name, across which edge and aligned how.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Planned {
    pub id: String,
    pub layout: Layout,
    pub width: Option<f32>,
    pub sized: bool,
    pub attach: Option<(String, Edge, Align)>,
    /// Its context's settings, each fixing a parameter, or time, for every panel of its environment on it that does not
    /// fix it itself. See `design/panel-context.md`.
    pub context: Vec<(String, super::context::Setting)>,
}

/// How far each of `wall`'s slots may be from the eyes of whoever it is for, at `eyes`, and still be read: within
/// [`crate::presence::READING`] for a standard slot's text, and as much nearer or further as its text is smaller or
/// larger, so that a wall's slots are read at least as well as a flat wall's from 4 m.
fn readable(wall: &Wall, eyes: [f32; 3]) -> bool {
    let reach = crate::presence::READING * wall.scale();
    (0..wall.slots()).all(|slot| {
        wall.slot_pose(slot).is_some_and(|pose| {
            let [x, y, z] = pose.position;
            (x - eyes[0]).hypot(y - eyes[1]).hypot(z - eyes[2]) <= reach + 1e-4
        })
    })
}

/// `planned`'s layout, laid out to be `width` metres wide, if it says so, at its shape and grid.
fn widened(planned: &Planned) -> Result<Layout, &'static str> {
    let Some(width) = planned.width else {
        return Ok(planned.layout);
    };
    if !(width.is_finite() && width > 0.0) {
        return Err("A width is a number of metres.");
    }
    let unit = Layout {
        scale: 1.0,
        ..planned.layout
    }
    .apply(Wall::new("", WallKind::Added, Pose::default()));
    Ok(Layout {
        scale: (width / unit.size().0 * 100.0).round() / 100.0,
        ..planned.layout
    })
}

/// `layout`, its slots sized so that it reaches as far along `edge` of `target` as the target's slots do, within
/// [`layout::SCALES`]: a wall attached without a size of its own fills the edge it is attached to.
fn filling(layout: Layout, target: &Wall, edge: Edge) -> Layout {
    let unit = Layout {
        scale: 1.0,
        ..layout
    }
    .apply(Wall::new("", WallKind::Added, Pose::default()));
    let length = |extent: group::Extent| match edge {
        Edge::Left | Edge::Right => extent.top - extent.bottom,
        _ => extent.right - extent.left,
    };
    let scale = length(target.extent()) / length(unit.extent());
    Layout {
        scale: ((scale * 100.0).round() / 100.0).clamp(layout::SCALES.0, layout::SCALES.1),
        ..layout
    }
}

impl Stage {
    /// Build `planned` as one group of walls in `walls`' environment, for whoever stands with their eyes at `eyes`: the
    /// first wall, then each attached to the earlier one it names, a wall without a size of its own filling the edge it
    /// is attached to; then the whole moved so that its middle stands at the first of `middles` the layout's extent, on
    /// its first wall's surface, gives where every slot can be read from `eyes`, as [`readable`] says. All of it or
    /// nothing: refused whole, saying why, as beyond the walls an environment takes or the slots it shows, or where no
    /// middle given leaves it readable. Returns each wall's identity, in order.
    pub(crate) fn compose(
        &mut self,
        walls: &Walls<'_>,
        planned: &[Planned],
        middles: &dyn Fn(group::Extent, Option<f32>) -> Vec<Pose>,
        eyes: [f32; 3],
    ) -> Result<Vec<String>, String> {
        let Some(first) = planned.first() else {
            return Err("A layout has at least one wall.".to_owned());
        };
        if planned.len() > LAYOUT_WALLS {
            return Err(format!(
                "A layout has at most {LAYOUT_WALLS} walls, as one group can be taken in."
            ));
        }
        if first.attach.is_some() {
            return Err("A layout's first wall is attached to nothing: the rest are attached to it, or to each other.".to_owned());
        }
        for (index, wall) in planned.iter().enumerate() {
            if planned[..index].iter().any(|earlier| earlier.id == wall.id) {
                return Err(format!("Two walls of the layout are called {:?}.", wall.id));
            }
            if index > 0 && wall.attach.is_none() {
                return Err(format!(
                    "Say which earlier wall {:?} is attached to, and across which edge.",
                    wall.id
                ));
            }
            if let Some((to, ..)) = &wall.attach
                && !planned[..index].iter().any(|earlier| earlier.id == *to)
            {
                return Err(format!(
                    "{:?} is attached to {to:?}, which is not an earlier wall of the layout.",
                    wall.id
                ));
            }
            if wall.layout.radius != first.layout.radius {
                return Err(
                    "A layout's walls are all flat, or all curved at the same radius.".to_owned(),
                );
            }
        }
        // Built once alone, to measure it, and then where it is put, with what the space has.
        let build = |base: &Stage, at: &Pose| -> Result<(Stage, Vec<String>), String> {
            let mut trial = base.clone();
            let mut built: Vec<String> = Vec::new();
            for wall in planned {
                let mut layout = widened(wall)?;
                let pose = match &wall.attach {
                    None => *at,
                    Some((to, edge, align)) => {
                        let index = planned
                            .iter()
                            .position(|earlier| earlier.id == *to)
                            .expect("checked above");
                        let target = trial
                            .shown_wall(walls, &built[index])
                            .expect("a wall built is shown");
                        if !wall.sized {
                            layout = filling(layout, &target, *edge);
                        }
                        let member = layout.apply(Wall::new("", WallKind::Added, *at));
                        group::attached_pose(&target, &member, *edge, *align)
                            .ok_or("Tomori does not know that edge or alignment.")?
                    }
                };
                let id = trial.add_wall(walls, &pose, Some(layout))?;
                if let Some((to, edge, align)) = &wall.attach {
                    let index = planned
                        .iter()
                        .position(|earlier| earlier.id == *to)
                        .expect("checked above");
                    let attach = Attach::new(built[index].clone(), *edge, *align);
                    trial.attach_wall(walls, &id, Some(attach), true, None)?;
                }
                built.push(id);
            }
            // Nor may it overlap a wall already standing.
            let shown = trial.walls(walls);
            let (new, old): (Vec<&Wall>, Vec<&Wall>) =
                shown.iter().partition(|wall| built.contains(&wall.id));
            if new
                .iter()
                .any(|wall| old.iter().any(|other| group::overlaps(wall, other)))
            {
                return Err("It would overlap a wall already here: stand it elsewhere, or move that wall first.".to_owned());
            }
            Ok((trial, built))
        };
        let shown = |stage: &Stage, built: &[String]| -> Vec<Wall> {
            built
                .iter()
                .map(|id| stage.shown_wall(walls, id).expect("a wall built is shown"))
                .collect()
        };
        let provisional = Pose::upright_facing(eyes, [eyes[0], eyes[1], eyes[2] + 1.0]);
        let (measured, built) = build(&Stage::default(), &provisional)?;
        let laid = shown(&measured, &built);
        let radius = laid[0].radius();
        let extent = laid
            .iter()
            .map(|wall| {
                let (u, y, _, _) = group::on_surface(&laid[0].pose, radius, &wall.pose);
                wall.extent().shifted(u, y)
            })
            .reduce(|a, b| {
                group::Extent::new(
                    a.left.min(b.left),
                    a.right.max(b.right),
                    a.bottom.min(b.bottom),
                    a.top.max(b.top),
                )
            })
            .expect("a layout has a wall");
        let (across, up) = (
            (extent.left + extent.right) / 2.0,
            (extent.bottom + extent.top) / 2.0,
        );
        let mut refusal = "Laid out where it could stand, some of its slots would be too far to read: use fewer walls, fewer slots or larger ones.".to_owned();
        for middle in middles(extent, radius) {
            // Its first wall stands so that the layout's middle is at `middle`, along its surface.
            let first = group::along(&middle, radius, -across, -up);
            match build(self, &first) {
                Ok((trial, built))
                    if shown(&trial, &built)
                        .iter()
                        .all(|wall| readable(wall, eyes)) =>
                {
                    *self = trial;
                    return Ok(built);
                }
                Ok(_) => {}
                Err(error) => refusal = error,
            }
        }
        Err(refusal)
    }
}

#[cfg(test)]
mod tests;
