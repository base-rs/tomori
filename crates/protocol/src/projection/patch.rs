// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Patches to a projection: what changed in a space's projection from one of its revisions to a later one, so that a
//! client holding the projection at the first is sent only the change rather than all of it again. Whole walls and
//! panels are replaced, added and removed by their identity, and a panel that only moved is moved, rather than fields
//! of them changed, so that applying a patch is simple and its result is exactly the projection the server sends
//! whole. See `design/projection.md`.
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Panel, Place, Projection, Wall};

/// What changed in the projection of `space`, in `environment`, from its revision `from` to its revision `to`: its
/// `ops`, applied in order to the projection at `from`, make it the projection at `to`. Several changes made together,
/// as a burst of them before the server sends, are one patch. A patch may change nothing, as when the server sends the
/// projection again so that a client undoes what it showed at once, such as a tick the server refused.
///
/// Sent as [`crate::ServerMessage::ProjectionPatch`], only to a client whose `Hello` sets `projection_patches`. A client
/// applies it only to its projection of the same space and environment at `from`; one that has another revision, or
/// cannot apply an op, as for one it does not know, asks again with [`crate::ClientMessage::ResyncProjection`] and is
/// sent the whole [`Projection`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProjectionPatch {
    pub space: String,
    pub environment: String,
    pub from: u64,
    pub to: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<PatchOp>,
}

/// One change of a [`ProjectionPatch`]. An op that names where a wall or panel stands puts it right after the one
/// `after` names, or first with `first`, in its list, taking it from where it was; one that names neither leaves it
/// where it was, or, if it is new, puts it last. `Unknown` stands for an op a newer server sends, which a client cannot
/// apply, so it asks for the whole projection instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PatchOp {
    /// The wall `wall`, whole, in place of the wall of its identity, or added.
    Wall {
        wall: Wall,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        first: bool,
    },
    /// The wall `wall` is no longer shown.
    RemoveWall { wall: String },
    /// The panel `panel`, whole, in place of the panel of its identity, or added: its content changed, as when the
    /// agent updated it or a participant ticked an item.
    Panel {
        panel: Panel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        first: bool,
    },
    /// The panel `panel` moved to `place`, pinned there or not, and is otherwise as it was: it was put on a wall, into
    /// another slot, left floating, or only stands elsewhere in the list of panels.
    Place {
        panel: String,
        place: Place,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pinned: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        first: bool,
    },
    /// The panel `panel` is no longer shown.
    RemovePanel { panel: String },
    #[serde(other)]
    Unknown,
}

/// Why a [`ProjectionPatch`] could not be applied. The projection may then be partly patched; its revision is not
/// changed, so that the client asks for the whole projection.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatchError {
    /// It is for another space or environment.
    Elsewhere,
    /// It applies to another revision: the client missed a patch, or has one from another connection.
    Revision { have: u64, from: u64 },
    /// It names a wall or panel that is not there, such as one to remove, to change in place or to stand after.
    Missing(String),
    /// An op this side does not know.
    Unknown,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Elsewhere => write!(formatter, "the patch is for another space or environment"),
            Self::Revision { have, from } => {
                write!(formatter, "the patch is from revision {from}, not {have}")
            }
            Self::Missing(id) => write!(formatter, "the patch names {id:?}, which is not there"),
            Self::Unknown => write!(formatter, "the patch has an op this client does not know"),
        }
    }
}

impl std::error::Error for PatchError {}

impl ProjectionPatch {
    /// The patch to the projection of `space` in `environment` from revision `from` to `to`, of `ops`.
    pub fn new(
        space: impl Into<String>,
        environment: impl Into<String>,
        from: u64,
        to: u64,
        ops: Vec<PatchOp>,
    ) -> Self {
        Self {
            space: space.into(),
            environment: environment.into(),
            from,
            to,
            ops,
        }
    }

    /// The patch that makes `before` into `after`, two projections of one space and environment, from `before`'s
    /// revision to `after`'s: walls and panels gone are removed, those new or changed sent whole, those that only moved
    /// moved, and each that stands elsewhere in its list said where, as few as keep the rest in their order.
    pub fn between(before: &Projection, after: &Projection) -> Self {
        let mut ops = Vec::new();
        let walls = order(&before.walls, &after.walls, |wall| &wall.id);
        ops.extend(
            walls
                .removed
                .into_iter()
                .map(|wall| PatchOp::RemoveWall { wall }),
        );
        let panels = order(&before.panels, &after.panels, |panel| &panel.id);
        ops.extend(
            panels
                .removed
                .into_iter()
                .map(|panel| PatchOp::RemovePanel { panel }),
        );
        let previous: HashMap<&str, &Wall> = before
            .walls
            .iter()
            .map(|wall| (wall.id.as_str(), wall))
            .collect();
        for (index, wall) in after.walls.iter().enumerate() {
            let position = walls.positions[index].clone();
            let changed = previous.get(wall.id.as_str()) != Some(&wall);
            if changed || position.is_some() {
                let (after, first) = Position::split(position);
                ops.push(PatchOp::Wall {
                    wall: wall.clone(),
                    after,
                    first,
                });
            }
        }
        let previous: HashMap<&str, &Panel> = before
            .panels
            .iter()
            .map(|panel| (panel.id.as_str(), panel))
            .collect();
        for (index, panel) in after.panels.iter().enumerate() {
            let position = panels.positions[index].clone();
            let (after, first) = Position::split(position.clone());
            match previous.get(panel.id.as_str()) {
                Some(was) if same_content(was, panel) => {
                    if was.place != panel.place || was.pinned != panel.pinned || position.is_some()
                    {
                        ops.push(PatchOp::Place {
                            panel: panel.id.clone(),
                            place: panel.place.clone(),
                            pinned: panel.pinned,
                            after,
                            first,
                        });
                    }
                }
                _ => ops.push(PatchOp::Panel {
                    panel: panel.clone(),
                    after,
                    first,
                }),
            }
        }
        Self::new(
            after.space.clone(),
            after.environment.clone(),
            before.revision,
            after.revision,
            ops,
        )
    }

    /// How many walls and panels it sends whole: as many as are new or changed.
    pub fn whole(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| matches!(op, PatchOp::Wall { .. } | PatchOp::Panel { .. }))
            .count()
    }

    /// Apply it to `projection`, the client's own, which is then at its revision `to`. Refused, changing nothing, for a
    /// projection of another space or environment, or at another revision than `from`; refused partway, leaving
    /// `projection` at its revision, for an op that names a wall or panel it does not have, or one this side does not
    /// know. Either way, the client asks for the whole projection.
    pub fn apply(&self, projection: &mut Projection) -> Result<(), PatchError> {
        if projection.space != self.space || projection.environment != self.environment {
            return Err(PatchError::Elsewhere);
        }
        if projection.revision != self.from {
            return Err(PatchError::Revision {
                have: projection.revision,
                from: self.from,
            });
        }
        for op in &self.ops {
            match op {
                PatchOp::Wall { wall, after, first } => put(
                    &mut projection.walls,
                    |shown| shown.id == wall.id,
                    |shown| &shown.id,
                    wall.clone(),
                    after.as_deref(),
                    *first,
                )?,
                PatchOp::RemoveWall { wall } => {
                    remove(&mut projection.walls, |shown| shown.id == *wall, wall)?
                }
                PatchOp::Panel {
                    panel,
                    after,
                    first,
                } => put(
                    &mut projection.panels,
                    |shown| shown.id == panel.id,
                    |shown| &shown.id,
                    panel.clone(),
                    after.as_deref(),
                    *first,
                )?,
                PatchOp::Place {
                    panel,
                    place,
                    pinned,
                    after,
                    first,
                } => {
                    let Some(shown) = projection.panels.iter().find(|shown| shown.id == *panel)
                    else {
                        return Err(PatchError::Missing(panel.clone()));
                    };
                    let mut moved = shown.clone();
                    moved.place = place.clone();
                    moved.pinned = *pinned;
                    put(
                        &mut projection.panels,
                        |shown| shown.id == *panel,
                        |shown| &shown.id,
                        moved,
                        after.as_deref(),
                        *first,
                    )?;
                }
                PatchOp::RemovePanel { panel } => {
                    remove(&mut projection.panels, |shown| shown.id == *panel, panel)?
                }
                PatchOp::Unknown => return Err(PatchError::Unknown),
            }
        }
        projection.revision = self.to;
        Ok(())
    }
}

/// Whether `a` and `b` show the same, wherever they are: everything but where they are, whether they are pinned there,
/// and how long ago they were projected, which a client counts on from when it was sent.
fn same_content(a: &Panel, b: &Panel) -> bool {
    let Panel {
        id,
        title,
        body,
        meta,
        age: _,
        place: _,
        code,
        pinned: _,
        slug,
        image,
        revision,
    } = a;
    *id == b.id
        && *title == b.title
        && *body == b.body
        && *meta == b.meta
        && *code == b.code
        && *slug == b.slug
        && *image == b.image
        && *revision == b.revision
}

/// Where an op puts what it names: right after another, by its identity, or first.
#[derive(Clone, Debug, PartialEq)]
enum Position {
    After(String),
    First,
}

impl Position {
    /// As an op says it: `after`, and `first`.
    fn split(position: Option<Position>) -> (Option<String>, bool) {
        match position {
            None => (None, false),
            Some(Position::After(id)) => (Some(id), false),
            Some(Position::First) => (None, true),
        }
    }
}

/// How a list changed its order: the identities gone from it, and, for each of the new list, where an op must put it,
/// or `None` where it stays where it was.
struct Order {
    removed: Vec<String>,
    positions: Vec<Option<Position>>,
}

/// How `before` became `after`, in order, by `key`: every item new, and as few of those kept as need be, said where to
/// stand, so that those of the longest run kept in their order stay where they are.
fn order<T>(before: &[T], after: &[T], key: impl Fn(&T) -> &String) -> Order {
    let target: HashMap<&str, usize> = after
        .iter()
        .enumerate()
        .map(|(index, item)| (key(item).as_str(), index))
        .collect();
    let removed = before
        .iter()
        .filter(|item| !target.contains_key(key(item).as_str()))
        .map(|item| key(item).clone())
        .collect();
    // Where each item kept stands in `after`, in the order they stood in `before`.
    let kept: Vec<usize> = before
        .iter()
        .filter_map(|item| target.get(key(item).as_str()).copied())
        .collect();
    let staying: HashSet<usize> = longest_increasing(&kept).into_iter().collect();
    let positions = (0..after.len())
        .map(|index| {
            if staying.contains(&index) {
                None
            } else if index == 0 {
                Some(Position::First)
            } else {
                Some(Position::After(key(&after[index - 1]).clone()))
            }
        })
        .collect();
    Order { removed, positions }
}

/// The values of the longest strictly increasing subsequence of `values`, which are distinct.
fn longest_increasing(values: &[usize]) -> Vec<usize> {
    // `length[i]` is the longest run ending at `values[i]`, and `previous[i]` the index of the value before it there.
    let mut length = vec![1; values.len()];
    let mut previous = vec![None; values.len()];
    for i in 0..values.len() {
        for j in 0..i {
            if values[j] < values[i] && length[j] + 1 > length[i] {
                length[i] = length[j] + 1;
                previous[i] = Some(j);
            }
        }
    }
    let mut run = Vec::new();
    let mut at = (0..values.len()).max_by_key(|index| (length[*index], std::cmp::Reverse(*index)));
    while let Some(index) = at {
        run.push(values[index]);
        at = previous[index];
    }
    run.reverse();
    run
}

/// Put `item` into `list`: in place of the one `matches`, where `after` and `first` say neither, or else where they
/// say, taking it from where it was; a new one where they say neither goes last.
fn put<T>(
    list: &mut Vec<T>,
    matches: impl Fn(&T) -> bool,
    key: impl Fn(&T) -> &String,
    item: T,
    after: Option<&str>,
    first: bool,
) -> Result<(), PatchError> {
    let existing = list.iter().position(&matches);
    if after.is_none() && !first {
        match existing {
            Some(index) => list[index] = item,
            None => list.push(item),
        }
        return Ok(());
    }
    if let Some(index) = existing {
        list.remove(index);
    }
    let index = match after {
        Some(after) => {
            list.iter()
                .position(|shown| key(shown) == after)
                .ok_or_else(|| PatchError::Missing(after.to_owned()))?
                + 1
        }
        None => 0,
    };
    list.insert(index, item);
    Ok(())
}

/// Take the one that `matches`, named `id`, out of `list`.
fn remove<T>(list: &mut Vec<T>, matches: impl Fn(&T) -> bool, id: &str) -> Result<(), PatchError> {
    let index = list
        .iter()
        .position(matches)
        .ok_or_else(|| PatchError::Missing(id.to_owned()))?;
    list.remove(index);
    Ok(())
}

#[cfg(test)]
#[path = "patch/tests.rs"]
mod tests;
