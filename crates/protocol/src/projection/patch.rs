// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Patches to a projection: what changed in a space's projection from one of its revisions to a later one, so that a
//! client holding the projection at the first is sent only the change rather than all of it again. Whole walls and
//! panels are replaced, added and removed by their identity, and a panel that only moved is moved, rather than fields
//! of them changed, so that applying a patch is simple and its result is exactly the projection the server sends
//! whole. A live panel's data, as a chart's points, which only its environment's server side writes, changes by data
//! ops of its own, which a client that says it applies them takes in place of the panel whole. See
//! `design/projection.md`.
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Panel, Place, Projection, Wall};
use crate::chart::Note;

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
    /// another slot, left floating, or only stands elsewhere in the list of panels. Its `version`, sent as `revision`,
    /// is the panel's now, which a move changes; only a client whose `Hello` sets `live_panels` is sent one that changes
    /// it, and an older client the panel whole instead.
    Place {
        panel: String,
        place: Place,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pinned: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        first: bool,
        #[serde(rename = "revision", default, skip_serializing_if = "is_zero")]
        version: u64,
    },
    /// The panel `panel` is no longer shown.
    RemovePanel { panel: String },
    /// The chart panel `panel`'s live data, at its data seq `seq`, from the data it had at `from`: `values` appended to
    /// each of its series, in order, the same number to each, and as many of its oldest points dropped, so that it keeps
    /// its points; each series' `breaks` whole, where given, or else its breaks moved along with its points, those
    /// dropped with them; and its `status`, `badge` and summary, its `body`, as they now are. Its version is unchanged.
    /// A client applies it only to the panel's data at `from`; one at `seq` or later already has it, and drops it, and
    /// one at another seq cannot apply it, and asks for the whole projection. Sent only to a client whose `Hello` sets
    /// `live_panels`.
    ChartAppend {
        panel: String,
        seq: u64,
        from: u64,
        values: Vec<Vec<Option<f32>>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        breaks: Option<Vec<Vec<u32>>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<Note>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        badge: Option<Note>,
        #[serde(default)]
        body: String,
    },
    /// The chart panel `panel`'s live data, whole, at its data seq `seq`: each of its series' `values` and `breaks`, in
    /// order, and its `status`, `badge` and summary, its `body`. Its version is unchanged. A client applies it where it
    /// holds the panel's data at an earlier seq, and drops it otherwise. Sent only to a client whose `Hello` sets
    /// `live_panels`.
    ChartReplace {
        panel: String,
        seq: u64,
        values: Vec<Vec<Option<f32>>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        breaks: Vec<Vec<u32>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<Note>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        badge: Option<Note>,
        #[serde(default)]
        body: String,
    },
    #[serde(other)]
    Unknown,
}

/// Why a [`ProjectionPatch`] could not be applied. The projection is left as it was, at its revision, so that the
/// client keeps showing it and asks for the whole projection.
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
    /// A data op that does not fit the panel it names: one with no chart, or other series, or an append from another
    /// seq than the panel's data has.
    Data(String),
}

fn is_zero(value: &u64) -> bool {
    *value == 0
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
            Self::Data(id) => write!(formatter, "the patch's data for {id:?} does not fit it"),
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
    ///
    /// This is the patch for a client whose `Hello` does not set `live_panels`: a panel whose version or live data
    /// changed is sent whole, as before data ops, so that what it holds is exactly what is sent whole.
    pub fn between(before: &Projection, after: &Projection) -> Self {
        Self::diff(before, after, false)
    }

    /// The patch that makes `before` into `after`, as [`ProjectionPatch::between`] makes it, for a client whose `Hello`
    /// sets `live_panels`: a panel whose structure is unchanged, but that moved or whose version changed, is a
    /// [`PatchOp::Place`] carrying its version, and one whose live data changed a [`PatchOp::ChartAppend`], where its
    /// series only moved along and gained points, or a [`PatchOp::ChartReplace`], rather than the panel whole.
    pub fn between_live(before: &Projection, after: &Projection) -> Self {
        Self::diff(before, after, true)
    }

    fn diff(before: &Projection, after: &Projection, live: bool) -> Self {
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
            let place = || PatchOp::Place {
                panel: panel.id.clone(),
                place: panel.place.clone(),
                pinned: panel.pinned,
                after: after.clone(),
                first,
                version: panel.version,
            };
            let moved = |was: &Panel| {
                was.place != panel.place || was.pinned != panel.pinned || position.is_some()
            };
            match previous.get(panel.id.as_str()) {
                Some(was) if !live && same_content(was, panel) => {
                    if moved(was) {
                        ops.push(place());
                    }
                }
                // Its live data sent apart, only where its seq moved on, so that a client never takes it for data it
                // already has; data changed at the same seq, as no server sends, goes whole.
                Some(was)
                    if live
                        && was.same_structure(panel)
                        && (was.data_seq < panel.data_seq && panel.chart.is_some()
                            || was.data_seq == panel.data_seq && was.same_data(panel)) =>
                {
                    if moved(was) || was.version != panel.version {
                        ops.push(place());
                    }
                    if was.data_seq != panel.data_seq {
                        ops.push(data(was, panel));
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
    /// projection of another space or environment, at another revision than `from`, or with an op that names a wall or
    /// panel it does not have or that this side does not know: the client keeps showing what it had and asks for the
    /// whole projection. The ops are applied to copies of the lists they change, which replace them only once every op
    /// applied, so that a patch refused partway never leaves the projection half patched.
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
        // Copied only where an op changes them, as a patch of panels alone leaves the walls.
        let mut walls: Option<Vec<Wall>> = None;
        let mut panels: Option<Vec<Panel>> = None;
        for op in &self.ops {
            match op {
                PatchOp::Wall { wall, after, first } => put(
                    walls.get_or_insert_with(|| projection.walls.clone()),
                    |shown| shown.id == wall.id,
                    |shown| &shown.id,
                    wall.clone(),
                    after.as_deref(),
                    *first,
                )?,
                PatchOp::RemoveWall { wall } => remove(
                    walls.get_or_insert_with(|| projection.walls.clone()),
                    |shown| shown.id == *wall,
                    wall,
                )?,
                PatchOp::Panel {
                    panel,
                    after,
                    first,
                } => put(
                    panels.get_or_insert_with(|| projection.panels.clone()),
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
                    version,
                } => {
                    let list = panels.get_or_insert_with(|| projection.panels.clone());
                    let Some(shown) = list.iter().find(|shown| shown.id == *panel) else {
                        return Err(PatchError::Missing(panel.clone()));
                    };
                    let mut moved = shown.clone();
                    moved.place = place.clone();
                    moved.pinned = *pinned;
                    // An older server's says none, and leaves the panel's as it was.
                    if *version != 0 {
                        moved.version = *version;
                    }
                    put(
                        list,
                        |shown| shown.id == *panel,
                        |shown| &shown.id,
                        moved,
                        after.as_deref(),
                        *first,
                    )?;
                }
                PatchOp::RemovePanel { panel } => remove(
                    panels.get_or_insert_with(|| projection.panels.clone()),
                    |shown| shown.id == *panel,
                    panel,
                )?,
                PatchOp::ChartAppend {
                    panel,
                    seq,
                    from,
                    values,
                    breaks,
                    status,
                    badge,
                    body,
                } => {
                    let list = panels.get_or_insert_with(|| projection.panels.clone());
                    let shown = list
                        .iter_mut()
                        .find(|shown| shown.id == *panel)
                        .ok_or_else(|| PatchError::Missing(panel.clone()))?;
                    if shown.data_seq >= *seq {
                        continue;
                    }
                    if shown.data_seq != *from {
                        return Err(PatchError::Data(panel.clone()));
                    }
                    append(shown, values, breaks.as_deref())
                        .ok_or_else(|| PatchError::Data(panel.clone()))?;
                    live(shown, *seq, status, badge, body);
                }
                PatchOp::ChartReplace {
                    panel,
                    seq,
                    values,
                    breaks,
                    status,
                    badge,
                    body,
                } => {
                    let list = panels.get_or_insert_with(|| projection.panels.clone());
                    let shown = list
                        .iter_mut()
                        .find(|shown| shown.id == *panel)
                        .ok_or_else(|| PatchError::Missing(panel.clone()))?;
                    if shown.data_seq >= *seq {
                        continue;
                    }
                    replace(shown, values, breaks)
                        .ok_or_else(|| PatchError::Data(panel.clone()))?;
                    live(shown, *seq, status, badge, body);
                }
                PatchOp::Unknown => return Err(PatchError::Unknown),
            }
        }
        if let Some(walls) = walls {
            projection.walls = walls;
        }
        if let Some(panels) = panels {
            projection.panels = panels;
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
        version,
        data_seq,
        chart,
        rows,
        columns,
        diagram,
        size,
        context,
    } = a;
    *id == b.id
        && *title == b.title
        && *body == b.body
        && *meta == b.meta
        && *code == b.code
        && *slug == b.slug
        && *image == b.image
        && *version == b.version
        && *data_seq == b.data_seq
        && *chart == b.chart
        && *rows == b.rows
        && *columns == b.columns
        && *diagram == b.diagram
        && *size == b.size
        && *context == b.context
}

/// The data op that makes `was`'s live data `now`'s, two chart panels of the same structure: an append, where every
/// series only moved along by the same number of points, fewer than half of them, and gained as many, or else the data
/// whole.
fn data(was: &Panel, now: &Panel) -> PatchOp {
    let (Some(old), Some(new)) = (&was.chart, &now.chart) else {
        unreachable!("only chart panels have live data")
    };
    let status = new.status.clone();
    let badge = new.badge.clone();
    let body = now.body.clone();
    let points = new.series.first().map_or(0, |series| series.values.len());
    let shifted = (1..points.div_ceil(2)).find(|&along| {
        old.series.iter().zip(&new.series).all(|(old, new)| {
            old.values.len() == points
                && new.values.len() == points
                && old.values[along..] == new.values[..points - along]
        })
    });
    match shifted {
        Some(along) => {
            let moved: Vec<Vec<u32>> = old
                .series
                .iter()
                .map(|series| moved_breaks(&series.breaks, along))
                .collect();
            let breaks: Vec<Vec<u32>> = new
                .series
                .iter()
                .map(|series| series.breaks.clone())
                .collect();
            PatchOp::ChartAppend {
                panel: now.id.clone(),
                seq: now.data_seq,
                from: was.data_seq,
                values: new
                    .series
                    .iter()
                    .map(|series| series.values[points - along..].to_vec())
                    .collect(),
                breaks: (breaks != moved).then_some(breaks),
                status,
                badge,
                body,
            }
        }
        None => {
            let breaks: Vec<Vec<u32>> = new
                .series
                .iter()
                .map(|series| series.breaks.clone())
                .collect();
            PatchOp::ChartReplace {
                panel: now.id.clone(),
                seq: now.data_seq,
                values: new
                    .series
                    .iter()
                    .map(|series| series.values.clone())
                    .collect(),
                breaks: if breaks.iter().all(Vec::is_empty) {
                    Vec::new()
                } else {
                    breaks
                },
                status,
                badge,
                body,
            }
        }
    }
}

/// A series' `breaks` once its points move along by `along`: those after a point dropped are dropped with it.
fn moved_breaks(breaks: &[u32], along: usize) -> Vec<u32> {
    let along = u32::try_from(along).unwrap_or(u32::MAX);
    breaks
        .iter()
        .filter(|at| **at >= along)
        .map(|at| at - along)
        .collect()
}

/// Append `values` to each series of `panel`'s chart, as many to each, dropping as many of its oldest points, and give
/// each series `breaks` where given, or else move its breaks along; `None` where it has no chart, other series, or values
/// it cannot take.
fn append(
    panel: &mut Panel,
    values: &[Vec<Option<f32>>],
    breaks: Option<&[Vec<u32>]>,
) -> Option<()> {
    let chart = panel.chart.as_mut()?;
    let along = values.first()?.len();
    if values.len() != chart.series.len()
        || along == 0
        || breaks.is_some_and(|breaks| breaks.len() != values.len())
        || values.iter().any(|appended| appended.len() != along)
        || chart
            .series
            .iter()
            .any(|series| series.values.len() < along)
    {
        return None;
    }
    for (index, (series, appended)) in chart.series.iter_mut().zip(values).enumerate() {
        series.values.drain(..along);
        series.values.extend_from_slice(appended);
        series.breaks = match breaks {
            Some(breaks) => breaks[index].clone(),
            None => moved_breaks(&series.breaks, along),
        };
    }
    Some(())
}

/// Give each series of `panel`'s chart its `values` and `breaks` whole, no breaks where `breaks` is empty; `None` where
/// it has no chart, or other series.
fn replace(panel: &mut Panel, values: &[Vec<Option<f32>>], breaks: &[Vec<u32>]) -> Option<()> {
    let chart = panel.chart.as_mut()?;
    if values.len() != chart.series.len() || !(breaks.is_empty() || breaks.len() == values.len()) {
        return None;
    }
    for (index, (series, values)) in chart.series.iter_mut().zip(values).enumerate() {
        series.values = values.clone();
        series.breaks = breaks.get(index).cloned().unwrap_or_default();
    }
    Some(())
}

/// Give `panel` the rest of its live data, at its data seq `seq`.
fn live(panel: &mut Panel, seq: u64, status: &Option<Note>, badge: &Option<Note>, body: &str) {
    let chart = panel
        .chart
        .as_mut()
        .expect("a chart panel's data was applied");
    chart.status = status.clone();
    chart.badge = badge.clone();
    panel.body = body.to_owned();
    panel.data_seq = seq;
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
