// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A list panel's rows drawn as rows, for a list its environment published with its rows ([`Panel::rows`]): the row it
//! highlights on a bar across the face, rather than after the body's pointer, each row's tone as a strip at its start
//! and a mark in place of its bullet, a tick, an exclamation mark or a cross, so that it reads without telling colours
//! apart, and its value as a thin bar under its text, in its tone; the choosable row this participant points at on a
//! fainter bar, outlined, and the one they just chose on a brighter one. Its text is the body every client draws, so
//! that a row wraps, scrolls and is pointed at as a list item is. See `design/panel-actions.md`.
use tomori_protocol::{ListRow, Panel, chart::Tone};

use super::HIGHLIGHT;
use crate::shapes::Shape;
use crate::transcript::Line;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Rgba, Style};

/// The pointer the server writes before the highlighted row's text, which a client that draws the row's bar leaves out.
const POINTER: &str = "- **▸** ";

/// The highlighted row's solid bar and the accent at its right end; the outline of the row pointed at, with no fill, so
/// that it is never taken for the highlight; and the brighter, thicker outline of the row just chosen, which leaves its
/// text as legible as it was. Each outline's width, in font pixels.
const HIGHLIGHTED: Rgba = [HIGHLIGHT[0], HIGHLIGHT[1], HIGHLIGHT[2], 0.24];
const HIGHLIGHTED_EDGE: Rgba = [HIGHLIGHT[0], HIGHLIGHT[1], HIGHLIGHT[2], 1.0];
const HOVERED: Rgba = [HIGHLIGHT[0], HIGHLIGHT[1], HIGHLIGHT[2], 0.7];
const HOVERED_WIDTH: f32 = 1.0;
const CHOSEN: Rgba = [0.85, 0.97, 1.0, 1.0];
const CHOSEN_WIDTH: f32 = 2.0;
/// How wide the highlight's accent is, in font pixels.
const ACCENT_WIDTH: f32 = 2.0;
/// How far a row's bar reaches into the face's padding at its left, and past its text at its right, short of the scroll
/// bar, in font pixels, and how rounded its corners are.
const BAR_LEFT: f32 = 4.0;
const BAR_RIGHT: f32 = 1.0;
const BAR_RADIUS: f32 = 2.0;
/// Where a row's tone strip stands, in the padding at its left, and how wide it is, in font pixels.
const STRIP_X: f32 = -3.0;
const STRIP_WIDTH: f32 = 2.0;
/// Where a value's bar starts, under the row's text after its mark, how far short of the text's right edge it ends, how
/// far up from the bottom of the row's last line it stands and how thick it is, in font pixels, and its track's colour
/// and a neutral row's fill.
const VALUE_X: f32 = (2 * CELL_WIDTH) as f32;
const VALUE_END: f32 = 2.0;
const VALUE_Y: f32 = 1.25;
const VALUE_WIDTH: f32 = 2.5;
const VALUE_TRACK: Rgba = [0.6, 0.66, 0.75, 0.08];
const VALUE_NEUTRAL: Rgba = [0.5, 0.72, 0.98, 0.95];

/// Which rows of one list panel this participant marks, by their place in its rows: the choosable row they point at,
/// which A or the trigger would choose, and the one they just chose. Only this participant sees them; what everyone
/// sees, the highlight, is the environment's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marked {
    pub hovered: Option<usize>,
    pub chosen: Option<usize>,
}

/// What this participant marks of the space's lists, by each panel's identity and the row's id, as
/// [`super::Projector::mark_rows`] takes them: the choosable row they point at, which A or the trigger would choose,
/// and the row they just chose, for as long as the label on the ray says so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowMarks {
    pub hovered: Option<(String, String)>,
    pub chosen: Option<(String, String)>,
}

impl RowMarks {
    /// The rows it marks of `panel`, by their place among its rows now: none of a panel it does not name, nor a row
    /// the panel no longer has, so that a row keeps its mark wherever a reordered list puts it.
    pub fn of(&self, panel: &Panel) -> Marked {
        let find = |mark: &Option<(String, String)>| {
            mark.as_ref()
                .filter(|(shown, _)| *shown == panel.id)
                .and_then(|(_, row)| panel.rows.iter().position(|listed| listed.id == *row))
        };
        Marked {
            hovered: find(&self.hovered),
            chosen: find(&self.chosen),
        }
    }
}

/// The body of `panel`, a list, as its face lays it out `columns` wide: the body the server wrote, the highlighted row's
/// pointer left out, as its bar shows it, and each row in a tone with its mark in place of its bullet, bold, in its
/// tone's colour.
pub(super) fn lines(panel: &Panel, columns: usize) -> Vec<Line> {
    let body = panel
        .body
        .split('\n')
        .enumerate()
        .map(|(index, line)| match line.strip_prefix(POINTER) {
            Some(text) if panel.rows.get(index).is_some_and(|row| row.highlighted) => {
                std::borrow::Cow::Owned(format!("- {text}"))
            }
            _ => std::borrow::Cow::Borrowed(line),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = crate::transcript::markdown_lines(&body, columns);
    retone(&mut lines, &panel.rows);
    lines
}

/// Mark each row of `rows` laid out as `lines` in its tone, in place of its bullet, or of the mark it had, on its first
/// line: a neutral row with its bullet, as the body's list draws it.
pub(super) fn retone(lines: &mut [Line], rows: &[ListRow]) {
    let mut last = None;
    for line in lines {
        let first = line.item.is_some() && line.item != last;
        last = line.item;
        let Some(row) = line.item.and_then(|item| rows.get(item)) else {
            continue;
        };
        if let (true, Some(bullet)) = (
            first,
            line.cells
                .iter_mut()
                .find(|(ch, _)| matches!(ch, '•' | '✓' | '!' | '✕')),
        ) {
            *bullet = match mark(row.tone) {
                Some((mark, color)) => (mark, Style::bold(color)),
                None => ('•', BULLET.with(|bullet| *bullet)),
            };
        }
    }
}

thread_local! {
    /// The style a list's bullet is drawn in, as the body's list draws it.
    static BULLET: Style = crate::transcript::markdown_lines("- x", 8)[0].cells[0].1;
}

/// Whether `after` differs from `before`, a list laid out, only in its rows' tones and values, and its version, so that
/// its lines are kept, only their marks changed, and its faces drawn again.
pub(super) fn restyled(before: &Panel, after: &Panel) -> bool {
    let same = |a: &ListRow, b: &ListRow| {
        (&a.id, a.choosable, a.highlighted) == (&b.id, b.choosable, b.highlighted)
    };
    if after.rows.is_empty()
        || before.rows.len() != after.rows.len()
        || !before.rows.iter().zip(&after.rows).all(|(a, b)| same(a, b))
    {
        return false;
    }
    let mut restyled = before.clone();
    restyled.rows.clone_from(&after.rows);
    restyled.version = after.version;
    before.rows != after.rows && restyled == *after
}

/// The mark in place of the bullet of a row in `tone`, and its colour: none for a neutral row, which keeps its bullet.
fn mark(tone: Tone) -> Option<(char, Rgba)> {
    let mark = match tone {
        Tone::Good => '✓',
        Tone::Warn => '!',
        Tone::Bad => '✕',
        _ => return None,
    };
    Some((mark, crate::chart::tone(tone)))
}

/// The colour of a row's strip and value in `tone`: none for a neutral row's strip.
fn strip(tone: Tone) -> Option<Rgba> {
    mark(tone).map(|(_, color)| color)
}

/// The shapes that draw `rows` as rows, behind the text of the `count` lines of `lines` a face `columns` wide shows
/// scrolled `scroll` lines down, in font pixels from its text's top-left corner: each row's bar, highlighted, pointed
/// at or chosen as `marked` says, its tone's strip, and its value's bar under its last line, where that shows. None for
/// a panel without rows.
pub(super) fn shapes(
    rows: &[ListRow],
    lines: &[Line],
    (scroll, count): (usize, usize),
    columns: usize,
    marked: Marked,
) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let end = lines.len().min(scroll + count);
    let width = (columns * CELL_WIDTH) as f32;
    let mut at = scroll.min(end);
    while at < end {
        let start = at;
        let item = lines[at].item;
        while at < end && lines[at].item == item {
            at += 1;
        }
        let Some((index, row)) = item.and_then(|item| rows.get(item).map(|row| (item, row))) else {
            continue;
        };
        let top = ((3 + start - scroll) * CELL_HEIGHT) as f32;
        let height = ((at - start) * CELL_HEIGHT) as f32;
        let rect = |color: Rgba, outline: Option<f32>| Shape::Rect {
            x: -BAR_LEFT,
            y: top,
            width: width + BAR_LEFT + BAR_RIGHT,
            height,
            color,
            radius: if outline.is_some() { 0.0 } else { BAR_RADIUS },
            outline,
        };
        if row.highlighted {
            let edge = width + BAR_RIGHT - ACCENT_WIDTH / 2.0;
            shapes.push(rect(HIGHLIGHTED, None));
            shapes.push(Shape::Stroke {
                from: [edge, top],
                to: [edge, top + height],
                width: ACCENT_WIDTH,
                color: HIGHLIGHTED_EDGE,
            });
        }
        if marked.chosen == Some(index) {
            shapes.push(rect(CHOSEN, Some(CHOSEN_WIDTH)));
        } else if marked.hovered == Some(index) {
            shapes.push(rect(HOVERED, Some(HOVERED_WIDTH)));
        }
        if let Some(color) = strip(row.tone) {
            shapes.push(Shape::Stroke {
                from: [STRIP_X, top + 1.5],
                to: [STRIP_X, top + height - 1.5],
                width: STRIP_WIDTH,
                color,
            });
        }
        // Under its last line, where that shows.
        let ends = lines.get(at).is_none_or(|next| next.item != item);
        if let (Some(value), true) = (row.fraction(), ends) {
            let y = top + height - VALUE_Y;
            let right = width - VALUE_END;
            let fill = strip(row.tone).unwrap_or(VALUE_NEUTRAL);
            shapes.push(Shape::Stroke {
                from: [VALUE_X, y],
                to: [right, y],
                width: VALUE_WIDTH,
                color: VALUE_TRACK,
            });
            shapes.push(Shape::Stroke {
                from: [VALUE_X, y],
                to: [VALUE_X + (right - VALUE_X) * value, y],
                width: VALUE_WIDTH,
                color: fill,
            });
        }
    }
    shapes
}

/// Which rows of a list of `rows` laid out as `lines` show, `count` of them scrolled `scroll` lines down, as its
/// metadata says, such as `rows 4–9 of 30`: none where every line shows.
pub(super) fn position(
    rows: &[ListRow],
    lines: &[Line],
    (scroll, count): (usize, usize),
) -> Option<String> {
    if rows.is_empty() || lines.len() <= count {
        return None;
    }
    let last = lines.len().min(scroll + count).saturating_sub(1);
    let first = lines[scroll.min(last)].item?;
    let last = lines[last].item?;
    Some(format!("rows {}–{} of {}", first + 1, last + 1, rows.len()))
}

/// Where a list of `rows` laid out as `lines`, scrolled `scroll` lines down, is scrolled to, by the row at the top of
/// its face, by its id, and how many lines into it: none where it is scrolled to its top, which stays there as rows
/// come and go, so that a queue's newest at its top is seen.
pub(super) fn anchor(rows: &[ListRow], lines: &[Line], scroll: f32) -> Option<(String, f32)> {
    if scroll < 0.5 {
        return None;
    }
    let item = lines.get(scroll.round() as usize)?.item?;
    let row = rows.get(item)?;
    // The line it begins on, which is there, as `item` is on one.
    let first = lines
        .iter()
        .position(|line| line.item == Some(item))
        .unwrap_or_default();
    Some((row.id.clone(), scroll - first as f32))
}

/// How far to scroll a list of `rows` laid out as `lines` so that the row `anchor` names, by its id, stands at the top
/// of its face as far into it as it did: none where the list no longer has it.
pub(super) fn anchored(
    rows: &[ListRow],
    lines: &[Line],
    (id, into): &(String, f32),
) -> Option<f32> {
    let index = rows.iter().position(|row| row.id == *id)?;
    let first = lines.iter().position(|line| line.item == Some(index))?;
    Some(first as f32 + into)
}

#[cfg(test)]
mod tests;
