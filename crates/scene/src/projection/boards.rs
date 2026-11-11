// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A flat wall of a group drawn edge by edge, so that the group reads as one board: its translucent board reaches, across
//! an edge joined to another wall of its group, only to the seam halfway across the gap between their slots, with no
//! border there, and elsewhere as far as its group's largest walls' margins, so that the group's outer edge is even; its
//! border is drawn only where no wall of its group meets it; and the group's label is shown once, on its top left wall.
//! The frame and the slots are quads, as every rule and frame is. See `design/projection.md`.
use glam::{Mat4, Vec3};
use tomori_protocol::{Edge, Wall, group};

use crate::{
    canvas::Canvas,
    controller_ui::Sheet,
    shapes::Shape,
    ui_text::{Row, Style},
};

use super::{LABEL_INDENT, PIXEL, SLOT, WALL_PADDING, wall_border, wall_label};

/// Where a wall's board reaches on its surface, from its pose, in metres, as [`group::Extent`] measures its slots.
type Board = group::Extent;

/// The other walls of `wall`'s group among `walls`.
pub(super) fn members<'a>(wall: &Wall, walls: &[&'a Wall]) -> Vec<&'a Wall> {
    walls
        .iter()
        .filter(|other| other.id != wall.id && other.group.is_some() && other.group == wall.group)
        .copied()
        .collect()
}

/// The largest scale of `wall`'s group, which the group's outer margins are drawn at.
fn group_scale(wall: &Wall, walls: &[&Wall]) -> f32 {
    members(wall, walls)
        .iter()
        .map(|other| other.scale())
        .fold(wall.scale(), f32::max)
}

/// The walls of `wall`'s group it is joined to across each edge, as the server says and as they stand: with how far
/// along the edge they meet.
fn joins<'a>(wall: &Wall, walls: &[&'a Wall]) -> Vec<(Edge, &'a Wall)> {
    members(wall, walls)
        .into_iter()
        .filter(|other| wall.joined.iter().any(|join| join.wall == other.id))
        .filter_map(|other| group::joined(wall, other).map(|(edge, ..)| (edge, other)))
        .collect()
}

/// Where `wall`'s board reaches, among `walls`: to the seam across each edge joined to another wall of its group, and
/// elsewhere its margins at its group's largest scale, a gutter at its sides and below, and its label's row above.
fn board(wall: &Wall, walls: &[&Wall]) -> Board {
    let scale = group_scale(wall, walls) / wall.scale();
    let joined = joins(wall, walls);
    let slots = wall.extent();
    let margin = |edge: Edge| {
        joined
            .iter()
            .filter(|(at, _)| *at == edge)
            .map(|(_, other)| group::seam(wall, other))
            .reduce(f32::max)
            .unwrap_or_else(|| match edge {
                Edge::Top => wall.margin(Edge::Top) * scale,
                _ => wall.margin(Edge::Left) * scale,
            })
    };
    Board::new(
        slots.left - margin(Edge::Left),
        slots.right + margin(Edge::Right),
        slots.bottom - margin(Edge::Bottom),
        slots.top + margin(Edge::Top),
    )
}

/// From where to where along `edge` of `board` it reaches: up a side, and across the top or bottom.
fn span(board: &Board, edge: Edge) -> (f32, f32) {
    match edge {
        Edge::Left | Edge::Right => (board.bottom, board.top),
        _ => (board.left, board.right),
    }
}

/// The shortest span of a border drawn, in metres, so that where two walls' edges meet within rounding none is.
const SLIVER: f32 = 0.001;

/// `whole`, less each of `covered`, as the spans left, in order, none shorter than [`SLIVER`].
fn uncovered((from, to): (f32, f32), covered: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut covered: Vec<(f32, f32)> = covered
        .iter()
        .map(|(low, high)| (low.max(from), high.min(to)))
        .filter(|(low, high)| high > low)
        .collect();
    covered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut left = Vec::new();
    let mut at = from;
    for (low, high) in covered {
        if low > at {
            left.push((at, low));
        }
        at = at.max(high);
    }
    if to > at {
        left.push((at, to));
    }
    left.retain(|(low, high)| high - low >= SLIVER);
    left
}

/// Whether `wall` shows its group's label: its group's top left wall, or, curved, as a curved wall's label is below
/// its slots, its bottom left, as they stand on the surface of its group's first wall.
pub(super) fn labelled(wall: &Wall, walls: &[&Wall]) -> bool {
    chosen(wall, walls, true)
}

/// Whether `wall` shows its group's one grab bar: its group's top left wall, whose slots reach highest, so that the bar,
/// above its top edge, stands above every wall of its group and over none of their slots, whatever their shapes. See
/// `design/projection.md`.
pub(super) fn bears_bar(wall: &Wall, walls: &[&Wall]) -> bool {
    chosen(wall, walls, false)
}

/// Whether `wall` is its group's top left wall among `walls`, or, for its `label` where the group is curved, its bottom
/// left, as they stand on the surface of its group's first wall: a wall alone is.
fn chosen(wall: &Wall, walls: &[&Wall], label: bool) -> bool {
    let mut group: Vec<&Wall> = members(wall, walls);
    if group.is_empty() {
        return true;
    }
    group.push(wall);
    let first = group
        .iter()
        .find(|other| Some(&other.id) == wall.group.as_ref())
        .copied()
        .unwrap_or(wall);
    let low = label && first.radius().is_some();
    let place = |other: &Wall| {
        let (u, y, _, _) = group::on_surface(&first.pose, first.radius(), &other.pose);
        let slots = other.extent().shifted(u, y);
        let height = if low { -slots.bottom } else { slots.top };
        // Highest first, by a centimetre, then leftmost.
        (-(height * 100.0).round(), slots.left)
    };
    group
        .iter()
        .min_by(|a, b| {
            let (a, b) = (place(a), place(b));
            a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
        })
        .is_some_and(|chosen| chosen.id == wall.id)
}

/// The edges of `wall` joined to another wall of its group among `walls`, as the server says and as they stand.
pub(super) fn joined_edges(wall: &Wall, walls: &[&Wall]) -> Vec<Edge> {
    joins(wall, walls)
        .into_iter()
        .map(|(edge, _)| edge)
        .collect()
}

/// The slots of the other walls of `wall`'s group among `walls`, as they stand on `wall`'s surface, from its pose.
pub(super) fn members_slots(wall: &Wall, walls: &[&Wall]) -> Vec<group::Extent> {
    members(wall, walls)
        .into_iter()
        .map(|other| {
            let (u, y, _, _) = group::on_surface(&wall.pose, wall.radius(), &other.pose);
            other.extent().shifted(u, y)
        })
        .collect()
}

/// `wall`'s board, standing at `frame`, as one of its group among `walls`, drawn as [`board`] says: a translucent board,
/// its border where no wall of its group meets it, its slots faintly marked, and its label, where it is its group's
/// [`labelled`] wall.
pub(super) fn sheets(wall: &Wall, frame: Mat4, walls: &[&Wall]) -> Vec<Sheet> {
    let scale = wall.scale();
    // In the wall's own font pixels, from its pose, down from its middle.
    let unit = PIXEL * scale;
    // Filled to their very edges, so that two boards meet at the seam without a line between them.
    let rect = |left: f32, top: f32, right: f32, bottom: f32, color| Shape::Fill {
        x: left / unit,
        y: -top / unit,
        width: (right - left) / unit,
        height: (top - bottom) / unit,
        color,
    };
    let own = board(wall, walls);
    let border = wall_border(wall);
    let thick = PIXEL * group_scale(wall, walls);
    let mut canvas = Canvas::new(0, PIXEL);
    canvas.padding = (0, 0);
    canvas.background = [0.0; 4];
    canvas.shapes.push(rect(
        own.left,
        own.top,
        own.right,
        own.bottom,
        [0.01, 0.02, 0.04, 0.5],
    ));
    // Its border along each edge, but where a wall of its group meets it.
    let joined = joins(wall, walls);
    for edge in Edge::ALL {
        let covered: Vec<(f32, f32)> = joined
            .iter()
            .filter(|(at, _)| *at == edge)
            .map(|(_, other)| {
                let (u, y, _, _) = group::on_surface(&wall.pose, wall.radius(), &other.pose);
                span(&board(other, walls).shifted(u, y), edge)
            })
            .collect();
        for (from, to) in uncovered(span(&own, edge), &covered) {
            canvas.shapes.push(match edge {
                Edge::Left => rect(own.left, to, own.left + thick, from, border),
                Edge::Right => rect(own.right - thick, to, own.right, from, border),
                Edge::Top => rect(from, own.top, to, own.top - thick, border),
                _ => rect(from, own.bottom + thick, to, own.bottom, border),
            });
        }
    }
    // Its slots, where they stand.
    canvas.shapes.extend(wall.slot_frames().iter().map(|slot| {
        let ([x, y, _], [width, height]) = (slot.position, slot.size);
        rect(
            x - width / 2.0,
            y + height / 2.0,
            x + width / 2.0,
            y - height / 2.0,
            SLOT,
        )
    }));
    let mut sheets = vec![Sheet {
        canvas: Some(std::sync::Arc::new(canvas)),
        model: Some(frame * Mat4::from_scale(Vec3::splat(scale))),
    }];
    if labelled(wall, walls) {
        // Indented as a wall alone indents it, from its board's top left, at its group's largest scale.
        let (text, color) = wall_label(wall);
        let pixel = thick;
        let mut label = Canvas::new(crate::ui_text::width(&text), pixel);
        label.padding = (0, 0);
        label.background = [0.0; 4];
        label.rows = vec![Row::text(
            &text,
            Style::bold(color),
            crate::ui_text::width(&text),
        )];
        let (width, height) = label.size();
        let left = own.left + (LABEL_INDENT + WALL_PADDING.0) as f32 * pixel;
        let top = own.top - WALL_PADDING.1 as f32 * pixel;
        let middle = Vec3::new(left + width / 2.0, top - height / 2.0, 0.0);
        sheets.push(Sheet {
            canvas: Some(std::sync::Arc::new(label)),
            model: Some(frame * Mat4::from_translation(middle)),
        });
    }
    sheets
}

#[cfg(test)]
mod tests;
