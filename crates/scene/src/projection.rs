// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What is projected in a space: its walls and the text and code panels on them or floating, as the server sends them; where
//! each stands; which panel the ray points at, and the label that names it; and pulling a panel off with the grip,
//! carrying it along the ray, pushed further or pulled closer and turned in snaps with the right stick, and letting it
//! go, where it floats, or, near a wall, snaps back into that wall's layout. See `design/projection.md`.
//!
//! Where panels are is the space's, shared by everyone in it, so a panel let go is sent to the server, and shown where
//! it was let go until the server's projection says where it is. The server lays walls out: this only draws each panel
//! in the slot the server gives it.
use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec2, Vec3};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tomori_protocol::{
    CODE_LINES, Code, Panel, Place, Pose, Projection, TOMORI_WALL, TokenKind, Wall, WallKind,
    chart::ChartTime, wall as layout,
};

use crate::{
    arrival::HOME,
    board::{Grip, model, pose},
    canvas::Canvas,
    chart::readout as chart_readout,
    controller_ui::Sheet,
    manipulation::Manipulation,
    pointing::Ray,
    recording::facing,
    shapes::Shape,
    sides::{Hands, Side},
    transcript::{BRIGHT, DIM, Line, TEXT},
    ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Style, cut},
};

/// A panel's face in a wall's slot, in cells: a title, a row of metadata, a rule, and its body.
pub const COLUMNS: usize = 34;
pub const BODY_ROWS: usize = 12;
#[cfg(test)]
const ROWS: usize = BODY_ROWS + 3;
/// A floating panel's face fits its text, within these bounds, in cells: never narrower or wider, so that it reads at a
/// comfortable width, nor shorter or taller.
pub const FIT_COLUMNS: (usize, usize) = (24, 48);
pub const FIT_ROWS: (usize, usize) = (3, 30);
/// The widest a floating panel's face grows to show a table, in cells: about a metre, 38° from 1.5 m, so that a row of
/// the table is still read by moving the eyes, not the head.
pub const TABLE_COLUMNS: usize = 60;
/// The size of a font pixel on a wall, in metres: a panel is about 60 by 36 cm, its text legible from about 3 m. Walls
/// are laid out in it, as `tomori_protocol::wall` lays them out for the server and every client alike.
pub const PIXEL: f32 = layout::PIXEL;
pub(crate) const PADDING: (usize, usize) = (6, 5);

/// A code panel's face, in cells: a title, a row of its language, length and age, a rule, its rows of code, each line
/// after its number, and a row saying what there is above and below them.
pub const CODE_COLUMNS: usize = 48;
/// A code panel's rows of code in a wall's slot.
pub const SLOT_CODE_ROWS: usize = 16;
#[cfg(test)]
const CODE_ROWS: usize = 3 + SLOT_CODE_ROWS + 1;
/// A code panel's rows of code floating, fitting its code: never fewer, nor more, so that at most it is about 75 cm
/// tall, as tall as a text panel floating at most, about 28° from 1.5 m.
pub const FIT_CODE_ROWS: (usize, usize) = (3, 40);
const CODE_PADDING: (usize, usize) = (6, 12);
/// The cells of a code panel's line number, and the space after it.
const GUTTER: usize = 3;
// Even a code panel the server sends short, as it does where a projection would not otherwise fit in a frame, fills a
// slot's rows of code, however its lines wrap.
const _: () = assert!(SLOT_CODE_ROWS * (CODE_COLUMNS - GUTTER) <= 720);
/// The size of a font pixel on a code panel, in metres, smaller than a text panel's so that the same panel holds its
/// [`CODE_COLUMNS`] and [`SLOT_CODE_ROWS`], legible from about 2 m.
pub const CODE_PIXEL: f32 = PIXEL * (COLUMNS * CELL_WIDTH + 2 * PADDING.0) as f32
    / (CODE_COLUMNS * CELL_WIDTH + 2 * CODE_PADDING.0) as f32;

/// A flat wall's padding, in font pixels, inside which its board is cells of text, as `tomori_protocol::wall` measures
/// it; its label is indented by a gutter.
const WALL_PADDING: (usize, usize) = (3, 3);
const LABEL_INDENT: usize = 24;
/// How far in front of its wall a panel stands, so that it is drawn over it.
const LIFT: f32 = layout::LIFT;
/// How far in front of a chart's face its readout is drawn, in the face's own units, so that it is drawn over it.
const READOUT_LIFT: f32 = 0.002;
/// The most charts a readout is linked to, besides the chart pointed at.
pub const LINKED: usize = 8;

/// How near a slot's plane a panel carried must come to dock there, as a share of how far the participant is from the
/// slot, and the least and most it is, in metres: see [`capture`].
pub const CAPTURE_SHARE: f32 = 0.15;
pub const CAPTURE_LEAST: f32 = 0.3;
pub const CAPTURE_MOST: f32 = 1.0;
/// How far beyond a slot's edges a panel carried may be and still dock there, so that the gaps between slots dock in
/// one, in metres.
const SNAP_MARGIN: f32 = 0.25;
/// How much further, as a share, a panel docking in a slot may go before it no longer does, so that a hand's tremor at
/// the edge of reach does not flicker it.
pub const CAPTURE_HOLD: f32 = 1.25;
/// How much nearer across the slots' planes another slot must be than the one a panel carried docks in to take its
/// place, in metres.
const SWITCH: f32 = 0.05;
/// How far behind a slot's plane a panel carried still docks in it, as one pushed a little into a wall, in metres;
/// further behind, at the wall's back, it docks in none.
pub const BEHIND: f32 = 0.05;
/// How quickly a panel carried glides to the slot it docks in, and back to the ray, as the time to go most of the way,
/// in seconds.
const PULL_TIME: f32 = 0.06;
/// How far in front of the slot it docks in a panel carried is drawn, over its outline, in metres.
const DOCK_LIFT: f32 = 2.0 * LIFT;

/// How long a panel let go is shown where it was let go, while the server's projection does not yet say so.
pub const PENDING: Duration = Duration::from_secs(3);

/// How far ahead of the participant's head, in metres, a wall they add stands, and how far below their eyes its middle
/// is.
pub const WALL_DISTANCE: f32 = 1.8;
const WALL_DROP: f32 = 0.1;

/// The label on the ray that names a panel pointed at.
pub(crate) const LABEL_COLUMNS: usize = 32;
pub(crate) const LABEL_PIXEL: f32 = 0.0017;

/// How large the hint beside a panel carried is drawn, as the angle a font pixel takes from the participant's eyes,
/// whatever its distance: an x-height of about 0.48°, comfortably above the 0.45° text reads at (see `design/text.md`).
pub const HINT_ANGLE: f32 = 0.0015;
/// The gap between a panel carried and its hint, as an angle from the eyes.
const HINT_GAP: f32 = 0.012;
/// How far below the eye line the hint may stand, as the tangent of the angle, before it goes above the panel instead.
const HINT_LOWEST: f32 = 0.84;
/// How far below the eyes the floor is taken to be, below which the hint never stands, in metres.
const HINT_FLOOR: f32 = 1.5;
/// The hint's columns of what each control does, after the controls' own and a gap of [`HINT_SPACE`].
const HINT_ACTIONS: usize = 19;
const HINT_SPACE: usize = 2;
/// A throw's colour, as it takes the panel away for everyone.
const THROW: [f32; 4] = [1.0, 0.62, 0.3, 1.0];

/// The fastest the right stick scrolls a panel, pushed all the way, in rows a second: about a slot's rows of code.
pub const SCROLL_SPEED: f32 = 15.0;
/// How far from the middle the right stick is pushed before it scrolls, as far as ends a push.
const SCROLL_DEAD: f32 = 0.2;

/// How many rows a second the right stick, pushed `forward` so far forward, scrolls a panel: up for forward, as it
/// moves the transcript's focus up, and down for back, faster the further it is pushed, from none at [`SCROLL_DEAD`]
/// from the middle to [`SCROLL_SPEED`], growing as the square, so that a little push reads slowly.
pub fn scroll_rate(forward: f32) -> f32 {
    if !forward.is_finite() {
        return 0.0;
    }
    let past = ((forward.abs() - SCROLL_DEAD) / (1.0 - SCROLL_DEAD)).clamp(0.0, 1.0);
    -forward.signum() * SCROLL_SPEED * past * past
}

/// The fewest cells a wrapped line of code keeps beside its hanging indent.
const CODE_ROOM: usize = 16;
/// The continuation marker, drawn in the gutter of a line of code's later rows.
const CONTINUED: char = '↪';
const CONTINUED_COLOR: [f32; 4] = [0.42, 0.47, 0.55, 0.8];

const FRAME: [f32; 4] = [0.32, 0.38, 0.46, 0.9];
const NEWEST: [f32; 4] = [0.3, 0.75, 0.95, 1.0];
pub(crate) const HIGHLIGHT: [f32; 4] = [0.35, 0.95, 1.0, 1.0];
const SLOT: [f32; 4] = [0.0, 0.0, 0.0, 0.3];
const GHOST: [f32; 4] = [0.35, 0.95, 1.0, 0.16];
/// The scroll bar's gap from the text and its width, and its thumb's least height, in font pixels, and its colours.
const SCROLL_BAR: (f32, f32, f32) = (2.0, 2.0, 6.0);
const SCROLL_TRACK: [f32; 4] = [0.6, 0.66, 0.75, 0.12];
const SCROLL_THUMB: [f32; 4] = [0.6, 0.7, 0.82, 0.75];

/// Where a participant puts a panel they let go.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Into this slot of the wall with this identity.
    Wall(String, u32),
    /// Floating free at this pose.
    Free(Pose),
    /// Floating free at this pose, stretched or shrunk with both hands to a size of its own.
    Sized(Pose, Resized),
}

/// How large a panel let go floats, as a participant stretched or shrank it with both hands, and its version as the
/// second hand took hold of it, so that the server refuses the change where the panel has changed since.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resized {
    /// As a proportion of its own size, within [`tomori_protocol::FLOAT_SIZES`].
    pub size: f32,
    pub version: u64,
}

/// How far the second hand reaches to take hold of something the first holds by touch rather than with its ray, in
/// metres: the hand within this of its face, and of its edges. See `design/projection.md`.
pub const TOUCH: f32 = 0.12;

/// How long the size of something held with both hands shows after it last changed.
pub const SIZE_SHOWN: Duration = Duration::from_millis(1500);

/// How long a panel let go of by one of two hands takes to settle onto the ray of the hand that still holds it, as a
/// time constant: quickly, but without a jump.
const SETTLE: f32 = 0.12;

/// How a panel's face is sized: to its wall's slot, of the cells a slot of its shape holds, or, floating, to fit its
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Slot(Cells),
    Text,
}

impl Fit {
    /// A standard slot's.
    pub const SLOT: Fit = Fit::Slot(Cells::STANDARD);
}

/// The cells a panel's face has in a wall's slot, at the size of the wall's text: a text panel's columns and rows of
/// body, and a code panel's columns and rows of code. A slot taller or wider than a standard one, of a wall's own
/// [`Wall::aspect`], holds more rows or more columns, so that its text wraps to its width and fills its height; and a
/// wall's [`Wall::content_scale`] below 1 draws its text smaller, so that each slot holds more and longer lines, and
/// above 1 larger, holding fewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cells {
    pub columns: usize,
    pub body: usize,
    pub code_columns: usize,
    pub code_rows: usize,
}

impl Cells {
    /// A standard slot's: 34 columns of 12 rows of body, and 48 columns of 16 rows of code.
    pub const STANDARD: Cells = Cells {
        columns: COLUMNS,
        body: BODY_ROWS,
        code_columns: CODE_COLUMNS,
        code_rows: SLOT_CODE_ROWS,
    };

    /// The fewest a slot holds, at the most a wall's content is scaled, so that a face always has room for a few words
    /// of its body and for code beside its line numbers.
    const LEAST: Cells = Cells {
        columns: 12,
        body: 2,
        code_columns: GUTTER + CODE_ROOM,
        code_rows: 3,
    };

    /// A slot of `wall`'s: as many cells as fit in it at its text's size, its [`Wall::text_scale`], and never fewer
    /// than a standard slot's where its content is drawn as large as its slots give it or smaller, nor than
    /// [`Cells::LEAST`] where larger.
    pub fn of(wall: &Wall) -> Cells {
        let (width, height) = slot_unit(wall);
        let (width, height) = (width / PIXEL, height / PIXEL);
        let columns =
            ((width - 2.0 * PADDING.0 as f32) / CELL_WIDTH as f32 + 1e-3).floor() as usize;
        let rows = ((height - 2.0 * PADDING.1 as f32) / CELL_HEIGHT as f32 + 1e-3).floor() as usize;
        // A code panel's cells are smaller, in its own font pixels.
        let code = PIXEL / CODE_PIXEL;
        let code_columns = ((width * code - 2.0 * CODE_PADDING.0 as f32) / CELL_WIDTH as f32 + 1e-3)
            .floor() as usize;
        let code_rows = ((height * code - 2.0 * CODE_PADDING.1 as f32) / CELL_HEIGHT as f32 + 1e-3)
            .floor() as usize;
        let least = if wall.content_scale() <= 1.0 {
            Cells::STANDARD
        } else {
            Cells::LEAST
        };
        Cells {
            columns: columns.max(least.columns),
            body: rows.saturating_sub(3).max(least.body),
            code_columns: code_columns.max(least.code_columns),
            code_rows: code_rows.saturating_sub(4).max(least.code_rows),
        }
    }
}

/// A slot of `wall`'s, in metres at its text's size, scale 1: as a slot's frame, scaled by its
/// [`Wall::text_scale`], measures it.
fn slot_unit(wall: &Wall) -> (f32, f32) {
    let (width, height) = wall.slot_size();
    let scale = wall.text_scale();
    (width / scale, height / scale)
}

/// How many columns of code a code panel's face has, sized as `fit` says.
fn code_columns(fit: Fit) -> usize {
    match fit {
        Fit::Slot(cells) => cells.code_columns,
        Fit::Text => CODE_COLUMNS,
    }
}

/// The size of a panel's face of `columns` and `body` rows, in metres.
fn sized(columns: usize, body: usize) -> (f32, f32) {
    let width = columns * CELL_WIDTH + 2 * PADDING.0;
    let height = (body + 3) * CELL_HEIGHT + 2 * PADDING.1;
    (width as f32 * PIXEL, height as f32 * PIXEL)
}

/// The size of a panel in a wall's slot, in metres.
pub fn panel_size() -> (f32, f32) {
    sized(COLUMNS, BODY_ROWS)
}

/// A panel's body, as the rows its face shows at `columns`, styled as the transcript styles a message. The one place a
/// panel's content becomes rows, so that every kind of content sizes alike.
fn body_lines(panel: &Panel, columns: usize) -> Vec<Line> {
    // A list its environment published with its rows, drawn as rows.
    if !panel.rows.is_empty() {
        return list::lines(panel, columns);
    }
    crate::transcript::markdown_lines(&panel.body, columns)
}

/// The `count` rows of `lines` a face shows scrolled `scroll` rows down. Scrolled into a table's body, the table's header
/// and the rule below it stay at the top, over the rows they cover, so that each column is still named, while the face
/// has room for two rows below them; as the table's last row passes under them, what follows it pushes them up and away,
/// so that every row scrolled moves what is shown by one.
fn shown(lines: &[Line], scroll: usize, count: usize) -> Vec<Line> {
    shown_at(lines, scroll, count)
        .map(|at| lines[at].clone())
        .collect()
}

/// Where each of the rows [`shown`] shows is among `lines`, in the order they are shown.
fn shown_at(lines: &[Line], scroll: usize, count: usize) -> impl Iterator<Item = usize> {
    let end = lines.len().min(scroll + count);
    let pinned = lines
        .get(scroll)
        .and_then(|line| line.table)
        .filter(|table| table.header > 0 && scroll > table.start && table.header + 2 <= count);
    let (header, body) = match pinned {
        Some(table) => {
            // How many of the header's rows show: all of them, or as many as there are of the table below the top.
            let header = table.header.min(table.end - scroll);
            (
                table.start + table.header - header..table.start + table.header,
                (scroll + header).min(end)..end,
            )
        }
        None => (0..0, scroll.min(end)..end),
    };
    header.chain(body)
}

/// The line of `panel`'s body, from 0, that the list item `point` is on begins on, where the ray meets a text panel drawn
/// with `model` and sized as `fit` and its body scrolled `scroll` rows down: found from the body row under the point, as its face lays the body out, so that what
/// the participant points at as they speak names the item, such as one of a checklist. `None` off its body, on a row of
/// no list item, and on a code panel.
pub fn item_at(panel: &Panel, model: Mat4, fit: Fit, scroll: usize, point: Vec3) -> Option<u32> {
    if panel.code.is_some() {
        return None;
    }
    let (columns, body_count, lines) = fitted(panel, fit);
    item_in(&lines, (columns, body_count), model, scroll, point)
}

/// As [`item_at`] says, of a text panel whose body is `lines` on a face of `columns` and `body_count` rows of body.
fn item_in(
    lines: &[Line],
    (columns, body_count): (usize, usize),
    model: Mat4,
    scroll: usize,
    point: Vec3,
) -> Option<u32> {
    let (_, height) = sized(columns, body_count);
    let local = model.inverse().transform_point3(point);
    let from_top = (height / 2.0 - local.y) / PIXEL - PADDING.1 as f32;
    if from_top.is_nan() || from_top < 0.0 {
        return None;
    }
    // Below the title, the metadata and the rule.
    let row = (from_top / CELL_HEIGHT as f32) as usize;
    let body = row.checked_sub(3).filter(|body| *body < body_count)?;
    let line = shown(lines, scroll, body_count).into_iter().nth(body)?;
    line.item.and_then(|line| u32::try_from(line).ok())
}

/// How many columns and body rows a text panel's face has when it is sized as `fit` says: a slot's, or, to fit its
/// text, as wide as its widest row, title and metadata, within [`FIT_COLUMNS`], or [`TABLE_COLUMNS`] for a body with a
/// table, its body wrapped at that width, as many
/// rows as that takes, within [`FIT_ROWS`]. A code panel's columns are always a slot's, and its rows of code are its
/// [`CodeLayout`]'s.
pub fn fit(panel: &Panel, fit: Fit) -> (usize, usize) {
    if let Some(diagram) = &panel.diagram {
        return volume::fit(panel, diagram, fit);
    }
    if panel.chart.is_some() {
        return chart::fit(fit);
    }
    if let Some(image) = &panel.image {
        return image::fit(panel, image, fit);
    }
    match fit {
        Fit::Slot(cells) => (cells.columns, cells.body),
        Fit::Text => {
            let (columns, rows, _) = fitted(panel, fit);
            (columns, rows)
        }
    }
}

/// As [`fit`] says, with the rows of `panel`'s body at the columns its face has, as [`body_lines`] lays them out.
fn fitted(panel: &Panel, fit: Fit) -> (usize, usize, Vec<Line>) {
    let sized = |(columns, rows): (usize, usize)| (columns, rows, body_lines(panel, columns));
    // A volume's outline is for clients that draw none: its face has no rows of body to scroll or point at.
    if let Some(diagram) = &panel.diagram {
        let (columns, rows) = volume::fit(panel, diagram, fit);
        return (columns, rows, Vec::new());
    }
    if panel.chart.is_some() {
        return sized(chart::fit(fit));
    }
    if let Some(image) = &panel.image {
        return sized(image::fit(panel, image, fit));
    }
    match fit {
        Fit::Slot(cells) => sized((cells.columns, cells.body)),
        Fit::Text => {
            // A table may make it wider, to show more of its columns side by side.
            let mut lines = body_lines(panel, FIT_COLUMNS.1);
            let most = if lines.iter().any(|line| line.table.is_some()) {
                lines = body_lines(panel, TABLE_COLUMNS);
                TABLE_COLUMNS
            } else {
                FIT_COLUMNS.1
            };
            let widest = lines
                .iter()
                .map(|line| line.cells.len())
                .chain([heading_width(panel)])
                .max()
                .unwrap_or_default();
            let columns = widest.clamp(FIT_COLUMNS.0, most);
            // Laid out at its widest, it is laid out already.
            if columns != most {
                lines = body_lines(panel, columns);
            }
            let rows = lines.len().clamp(FIT_ROWS.0, FIT_ROWS.1);
            (columns, rows, lines)
        }
    }
}

/// How many cells `panel`'s title and its metadata row take, the wider of them.
fn heading_width(panel: &Panel) -> usize {
    let title = cut(
        &[crate::ui_text::Span::new(
            &panel.title,
            Style::plain(BRIGHT),
        )],
        usize::MAX,
    )
    .len();
    let about = cut(
        &[crate::ui_text::Span::new(about(panel), Style::plain(DIM))],
        usize::MAX,
    )
    .len();
    title.max(about)
}

/// The size of `panel`, in metres, sized as `fit` says: a code panel floating is as tall as its code, as its
/// [`CodeLayout`] says.
pub fn fitted_size(panel: &Panel, fit: Fit) -> (f32, f32) {
    let layout = panel
        .code
        .as_ref()
        .map(|code| CodeLayout::new(panel, code, code_columns(fit)));
    size_of(panel, fit, layout.as_ref())
}

/// The size of `panel`, in metres, sized as `fit` says, its code laid out as `layout`.
fn size_of(panel: &Panel, fit: Fit, layout: Option<&CodeLayout>) -> (f32, f32) {
    match (layout, fit) {
        (Some(layout), Fit::Text) => {
            let width = layout.columns * CELL_WIDTH + 2 * CODE_PADDING.0;
            let height = layout.face_rows(fit) * CELL_HEIGHT + 2 * CODE_PADDING.1;
            (width as f32 * CODE_PIXEL, height as f32 * CODE_PIXEL)
        }
        _ => {
            let (columns, body) = self::fit(panel, fit);
            sized(columns, body)
        }
    }
}

/// The size of `wall`, in metres: a flat wall's board, or the span of a curved wall's slots.
pub fn wall_size(wall: &Wall) -> (f32, f32) {
    wall.size()
}

/// What a panel is called, as the recording indicator and status line name it: its title, or else the start of its
/// body.
pub fn name(panel: &Panel) -> String {
    if !panel.title.is_empty() {
        return panel.title.clone();
    }
    let start = panel
        .body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    crate::ui_text::clip(start.trim().trim_start_matches('#').trim(), 60)
}

/// How many lines a code panel's block has, such as `40 lines`.
fn length(code: &Code) -> String {
    lines(code.lines as usize)
}

/// `count` lines, such as `1 line` or `40 lines`.
fn lines(count: usize) -> String {
    match count {
        1 => "1 line".to_owned(),
        count => format!("{count} lines"),
    }
}

/// How many of its block's lines a code panel holds: those of its body, at most [`CODE_LINES`].
fn held(panel: &Panel) -> usize {
    panel.body.split('\n').count().min(CODE_LINES)
}

/// How long ago, briefly, with what a panel's metadata says, such as `source: CI · 5m`, or, for a code panel, its
/// language and length, such as `rust · 40 lines · 5m`, and whether it is pinned.
fn about(panel: &Panel) -> String {
    let mut parts: Vec<String> = panel
        .meta
        .iter()
        .map(|meta| format!("{}: {}", meta.key, meta.value))
        .collect();
    if let Some(code) = &panel.code {
        parts.extend((!code.language.is_empty()).then(|| code.language.clone()));
        parts.push(length(code));
    }
    if let Some(image) = &panel.image {
        parts.push(format!("{} × {}", image.width, image.height));
    }
    parts.push(crate::board::age(panel.age));
    if panel.code.is_some() && panel.pinned {
        parts.push("pinned".to_owned());
    }
    parts.join(" · ")
}

/// The colour each kind of code is drawn in, one calm palette on a panel's dark face for every client, whichever
/// highlighter classified it on the server.
pub fn palette(kind: TokenKind) -> crate::ui_text::Rgba {
    match kind {
        TokenKind::Keyword => [0.78, 0.62, 0.96, 1.0],
        TokenKind::String => [0.66, 0.86, 0.58, 1.0],
        TokenKind::Comment => [0.5, 0.56, 0.64, 1.0],
        TokenKind::Number => [0.96, 0.7, 0.48, 1.0],
        TokenKind::Function => [0.5, 0.74, 0.98, 1.0],
        TokenKind::Type => [0.95, 0.84, 0.52, 1.0],
        TokenKind::Variable => [0.84, 0.88, 0.94, 1.0],
        TokenKind::Operator => [0.56, 0.86, 0.9, 1.0],
        TokenKind::Punctuation => [0.64, 0.68, 0.75, 1.0],
        TokenKind::Constant => [0.96, 0.62, 0.56, 1.0],
        TokenKind::Attribute => [0.9, 0.64, 0.82, 1.0],
        // Plain, and a kind this client does not know.
        _ => TEXT,
    }
}

/// What a panel's face shows, scrolled to its top: its title, bold, its metadata and age, a rule, and its body as
/// Markdown-lite, styled as the transcript styles a message, as wide and on as many rows as `fit` says, ending with "…"
/// where it is longer; a code panel's, its code face. A `highlighted` panel, pointed at or carried, is framed brightly,
/// and the `newest` on its wall in blue.
pub fn face(panel: &Panel, highlighted: bool, newest: bool, fit: Fit) -> Canvas {
    let layout = panel
        .code
        .as_ref()
        .map(|code| CodeLayout::new(panel, code, code_columns(fit)));
    scrolled_face(panel, highlighted, newest, fit, 0, layout.as_ref())
}

/// A list panel's face, as [`face`] draws it, scrolled `scroll` rows down, with the row whose id is `hovered` drawn as
/// this participant's ray points at it and the row `chosen` as they just chose it, as the headset draws them: for a
/// still picture of a list, such as a test's. It draws a text panel only, as a list is.
pub fn list_face(
    panel: &Panel,
    fit: Fit,
    scroll: usize,
    (hovered, chosen): (Option<&str>, Option<&str>),
) -> Canvas {
    let marks = list::RowMarks {
        hovered: hovered.map(|row| (panel.id.clone(), row.to_owned())),
        chosen: chosen.map(|row| (panel.id.clone(), row.to_owned())),
    };
    let (columns, body_count, lines) = fitted(panel, fit);
    text_face(
        panel,
        (columns, body_count),
        &lines,
        (scroll, marks.of(panel)),
        Some(frame(false, false)),
    )
}

/// What a panel's face shows, as [`face`] says, its body scrolled `scroll` rows down, as far as it goes, its code laid
/// out as `layout`. A body longer than its face has a slim bar at its right edge, showing how far down it is scrolled
/// and how much of it shows.
fn scrolled_face(
    panel: &Panel,
    highlighted: bool,
    newest: bool,
    fit: Fit,
    scroll: usize,
    layout: Option<&CodeLayout>,
) -> Canvas {
    let border = Some(frame(highlighted, newest));
    if let Some(layout) = layout {
        let mut canvas = code_face(panel, layout, fit, scroll);
        canvas.border = border;
        return canvas;
    }
    if let Some(chart) = &panel.chart {
        return chart::face(panel, chart, fit, border).0;
    }
    if let Some(image) = &panel.image {
        return image::face(panel, image, fit, border);
    }
    let (columns, body_count, lines) = fitted(panel, fit);
    text_face(
        panel,
        (columns, body_count),
        &lines,
        (scroll, list::Marked::default()),
        border,
    )
}

/// The colour a panel's face is framed in: brightly where it is `highlighted`, pointed at or carried, in blue where it
/// is the `newest` on its wall, and otherwise faintly.
fn frame(highlighted: bool, newest: bool) -> [f32; 4] {
    if highlighted {
        HIGHLIGHT
    } else if newest {
        NEWEST
    } else {
        FRAME
    }
}

/// A text panel's face, as [`scrolled_face`] says, of `columns` and `body_count` rows of body, its body laid out as
/// `lines`, scrolled `scroll` rows down and framed by `border`; a list's rows drawn as rows, those `marked` as this
/// participant marks them, and, where it is longer than its face, which of its rows show, after its metadata.
fn text_face(
    panel: &Panel,
    (columns, body_count): (usize, usize),
    lines: &[Line],
    (scroll, marked): (usize, list::Marked),
    border: Option<[f32; 4]>,
) -> Canvas {
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = border;
    let total = lines.len();
    let scroll = scroll.min(total.saturating_sub(body_count));
    let about = about(panel);
    let mut told = about.clone();
    if let Some(position) = list::position(&panel.rows, lines, (scroll, body_count)) {
        told = format!("{position} · {about}");
    }
    // Its context's chips, where it has any, before its metadata or on a row of their own, as laid out for its face.
    let (metadata, rule, chips) = chips::header(
        &panel.context,
        (
            &[crate::ui_text::Span::new(about, Style::plain(DIM))],
            &[crate::ui_text::Span::new(told, Style::plain(DIM))],
        ),
        columns,
        chips::today(),
    );
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), columns),
        metadata,
        rule,
    ];
    let mut body = shown(lines, scroll, body_count);
    if total > scroll + body_count
        && let Some(last) = body.last_mut()
        && last.table.is_none()
        && last.cells.len() < columns
    {
        last.cells.push(('…', Style::plain(TEXT)));
    }
    body.resize_with(body_count, Default::default);
    rows.extend(body.into_iter().map(|line| Row::new(line.cells)));
    canvas.rows = rows;
    canvas.shapes = chips.shapes;
    canvas.shapes.extend(list::shapes(
        panel,
        lines,
        (scroll, body_count),
        columns,
        marked,
    ));
    canvas
        .shapes
        .extend(scroll_bar(columns, body_count, total, scroll));
    canvas
}

/// The slim bar at the right edge of a body of `total` rows, `shown` of them at once in a face `columns` wide, scrolled
/// `scroll` rows down: a faint track as tall as the rows shown, and on it a brighter thumb, as tall and as far down as
/// the rows shown are of them all. None where every row shows.
fn scroll_bar(columns: usize, shown: usize, total: usize, scroll: usize) -> Vec<Shape> {
    if total <= shown {
        return Vec::new();
    }
    let x = (columns * CELL_WIDTH) as f32 + SCROLL_BAR.0;
    let top = (3 * CELL_HEIGHT) as f32;
    let track = (shown * CELL_HEIGHT) as f32;
    let height = (track * shown as f32 / total as f32).max(SCROLL_BAR.2);
    let y = (top + track * scroll as f32 / total as f32).min(top + track - height);
    vec![Shape::Bar {
        x,
        y: top,
        width: SCROLL_BAR.1,
        height: track,
        track: SCROLL_TRACK,
        fill: SCROLL_THUMB,
        from: (y - top) / track,
        to: (y - top + height) / track,
    }]
}

/// The colour of a wall's frame, and its slots' on a curved wall.
fn wall_border(wall: &Wall) -> [f32; 4] {
    match wall.kind {
        WallKind::Tomori => [0.35, 0.75, 0.95, 0.55],
        _ => [0.5, 0.55, 0.62, 0.45],
    }
}

/// A flat wall's own face, at scale 1: a translucent board labelled with whose it is, its slots, of its grid, faintly
/// marked, laid out as `tomori_protocol::wall` lays them out.
fn wall_face(wall: &Wall) -> Canvas {
    let grid = wall.grid();
    let (columns, rows) = wall.board_cells();
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = WALL_PADDING;
    canvas.background = [0.01, 0.02, 0.04, 0.5];
    let (label, color) = wall_label(wall);
    let border = wall_border(wall);
    canvas.border = Some(border);
    let label = Row::text(&label, Style::bold(color), columns);
    let mut cells: Vec<crate::ui_text::Cell> =
        std::iter::repeat_n((' ', Style::plain(color)), LABEL_INDENT / CELL_WIDTH)
            .chain(label.cells)
            .take(columns)
            .collect();
    // Its context's chips, where it has settings of its own, along its top edge after its label.
    let at = cells.len() + 2;
    let chips = chips::lay(
        &wall.context,
        columns.saturating_sub(at),
        at,
        0,
        chips::today(),
    );
    if !chips.cells.is_empty() {
        cells.extend([(' ', Style::plain(color)); 2]);
        cells.extend(chips.cells);
    }
    canvas.rows = vec![Row::new(cells)];
    canvas.rows.resize(rows, Row::default());
    canvas.shapes = chips.shapes;
    canvas.shapes.extend((0..grid.slots()).map(|slot| {
        let (left, top, width, height) = wall.slot_rect(slot);
        Shape::rect(
            left - WALL_PADDING.0 as f32,
            top - WALL_PADDING.1 as f32,
            width,
            height,
            SLOT,
        )
    }));
    canvas
}

/// A slot's own board on a curved wall: as large as a panel in a slot of `cells`, faintly marked.
fn slot_board(border: [f32; 4], cells: Cells) -> Canvas {
    let mut canvas = Canvas::new(cells.columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.01, 0.02, 0.04, 0.45];
    canvas.border = Some(border);
    canvas.rows = vec![Row::default(); cells.body + 3];
    canvas
}

/// The outline a panel carried shows in the slot of `cells` it would take if let go: as large as a panel in it, framed
/// brightly and faintly filled.
fn ghost(cells: Cells) -> Canvas {
    let mut canvas = slot_board(HIGHLIGHT, cells);
    canvas.background = GHOST;
    canvas
}

/// How far below a curved wall's rows its label's middle stands, in metres.
const ARC_LABEL_DROP: f32 = 0.04;

/// A curved wall's label, below the middle of its row of slots.
fn arc_label(wall: &Wall) -> Canvas {
    let (label, color) = wall_label(wall);
    let labelled = crate::ui_text::width(&label);
    let mut cells = Row::text(&label, Style::bold(color), labelled).cells;
    // Its context's chips, where it has settings of its own, after its label, no wider than the wall's row of slots.
    let room = arc_columns(wall).saturating_sub(labelled + 2);
    let chips = chips::lay(&wall.context, room, labelled + 2, 0, chips::today());
    if !chips.cells.is_empty() {
        cells.extend([(' ', Style::plain(color)); 2]);
        cells.extend(chips.cells);
    }
    let mut canvas = Canvas::new(cells.len(), PIXEL);
    canvas.padding = (CELL_WIDTH, 2);
    canvas.background = [0.01, 0.02, 0.04, 0.5];
    canvas.rows = vec![Row::new(cells)];
    canvas.shapes = chips.shapes;
    canvas
}

/// How many cells of a curved wall's label fit across its row of slots, from its leftmost slot's edge to its
/// rightmost's, as the slots stand along its arc, so that its chips never stand beyond it.
fn arc_columns(wall: &Wall) -> usize {
    let (left, right) = wall
        .slot_frames()
        .iter()
        .map(|slot| {
            (
                slot.position[0] - slot.size[0] / 2.0,
                slot.position[0] + slot.size[0] / 2.0,
            )
        })
        .fold((f32::MAX, f32::MIN), |(left, right), (from, to)| {
            (left.min(from), right.max(to))
        });
    ((right - left).max(0.0) / (CELL_WIDTH as f32 * PIXEL)) as usize
}

/// The most characters of the name an environment gives its wall that a client draws as its label.
const NAME_CELLS: usize = 24;

/// What a wall is labelled, and in which colour: the Tomori wall `TOMORI`, an environment's own wall, if its
/// environment named it, its name in capitals, and any other `WALL`.
fn wall_label(wall: &Wall) -> (String, [f32; 4]) {
    match (&wall.kind, &wall.name) {
        (WallKind::Tomori, _) => ("TOMORI".to_owned(), [0.6, 0.85, 1.0, 1.0]),
        (WallKind::Environment, Some(name)) if !name.trim().is_empty() => {
            // In capitals first, since a letter may become more than one, as `ß` becomes `SS`, then cut to a line of
            // cells as drawn, a wide character taking two.
            let upper = name
                .chars()
                .filter(|letter| !letter.is_control())
                .collect::<String>()
                .to_uppercase();
            let mut label = String::new();
            for letter in upper.chars() {
                label.push(letter);
                if crate::ui_text::width(&label) > NAME_CELLS {
                    label.pop();
                    break;
                }
            }
            (label, DIM)
        }
        _ => ("WALL".to_owned(), DIM),
    }
}

/// The radius of a curved wall's arc, if `wall` is one this client can draw curved: one whose radius is long enough
/// for its slots to stand side by side, around no more than half a circle. Any other shape is drawn flat.
fn arc(wall: &Wall) -> Option<f32> {
    wall.radius()
}

/// Where each of a wall's slots stands, given its `frame`: the middle of a panel in it, facing out, scaled as the
/// panel in it is drawn, as `tomori_protocol::wall` lays them out. A flat wall's are in its plane, just in front of it;
/// a curved wall's stand along its arc, left to right as seen from in front, each turned to face the line the arc
/// curves about, its rows one above another.
pub fn slot_frames(wall: &Wall, frame: Mat4) -> Vec<Mat4> {
    wall.slot_frames()
        .into_iter()
        .map(|slot| {
            frame
                * Mat4::from_translation(Vec3::from_array(slot.position))
                * Mat4::from_rotation_y(slot.yaw)
                * Mat4::from_scale(Vec3::splat(slot.scale))
        })
        .collect()
}

/// Where `ray` meets a panel of `size` drawn with `model`, as the distance along it, if it does.
fn hit(model: Mat4, size: (f32, f32), ray: &Ray) -> Option<f32> {
    hit_within(model, size, 0.0, ray)
}

/// Where `ray` meets a panel of `size` drawn with `model`, or within `margin` of its edges, in its own units, as the
/// distance along it, if it does.
fn hit_within(model: Mat4, (width, height): (f32, f32), margin: f32, ray: &Ray) -> Option<f32> {
    let local = model.inverse();
    let origin = local.transform_point3(ray.origin);
    let direction = local.transform_vector3(ray.direction);
    if direction.z.abs() < f32::EPSILON {
        return None;
    }
    let along = -origin.z / direction.z;
    let point = origin + direction * along;
    (along >= 0.0
        && point.x.abs() <= width / 2.0 + margin
        && point.y.abs() <= height / 2.0 + margin)
        .then_some(along)
}

/// Where `ray` meets `wall`, standing at `frame`, as the distance along it: its flat board, or, curved, one of its slots.
pub(crate) fn wall_hit(wall: &Wall, frame: Mat4, ray: &Ray) -> Option<f32> {
    let board = wall
        .board()
        .and_then(|size| hit_within(frame, size, 0.0, ray));
    slot_frames(wall, frame)
        .into_iter()
        .filter_map(|slot| hit(slot, slot_unit(wall), ray))
        .chain(board)
        .min_by(f32::total_cmp)
}

/// A slot a panel carried would be put in if let go: the wall, the slot, where it stands, and how a panel in it is laid
/// out.
#[derive(Clone, Debug, PartialEq)]
struct Aim {
    wall: String,
    slot: u32,
    model: Mat4,
    fit: Fit,
}

/// How near a slot's plane a panel carried must come to dock there, in metres, for a participant `distance` from the
/// slot: [`CAPTURE_SHARE`] of it, within [`CAPTURE_LEAST`] and [`CAPTURE_MOST`]. A share, since how well someone judges
/// how far away something is, and how far the right stick pushes a panel in a moment, both grow with its distance; so
/// that a wall at arm's length takes a panel brought within 30 cm of it, one 3.3 m away within half a metre, and one
/// 6.7 m away or more within a metre. See `design/projection.md`.
///
/// ```
/// use tomori_scene::projection::capture;
///
/// assert_eq!(capture(1.0), 0.3);
/// assert!((capture(4.0) - 0.6).abs() < 1e-6);
/// assert_eq!(capture(20.0), 1.0);
/// ```
pub fn capture(distance: f32) -> f32 {
    (CAPTURE_SHARE * distance).clamp(CAPTURE_LEAST, CAPTURE_MOST)
}

/// The slot a panel carried at `centre` docks in, for a participant at `viewer`, if it is near enough to one: its middle
/// in front of the slot's plane by at most [`capture`], or behind it by at most [`BEHIND`], and within [`SNAP_MARGIN`]
/// of its edges, measured from the panel's middle, never from where the ray points. Of those, the nearest across the
/// slots' planes, how far beyond each slot's edges it is, and of slots as near across, the nearest in depth. The slot
/// it docked in last frame, `shown`, holds it within [`CAPTURE_HOLD`] of those, and ranks [`SWITCH`] nearer across
/// than it is, however deep the panel is held, so that it neither flickers at the edge of reach nor between two slots.
fn dock(walls: &[(&Wall, Mat4)], centre: Vec3, viewer: Vec3, shown: Option<&Aim>) -> Option<Aim> {
    walls
        .iter()
        .flat_map(|(wall, frame)| {
            let (width, height) = slot_unit(wall);
            slot_frames(wall, *frame)
                .into_iter()
                .enumerate()
                .filter_map(move |(slot, model)| {
                    let held =
                        shown.is_some_and(|aim| aim.wall == wall.id && aim.slot == slot as u32);
                    let (hold, nearer) = if held {
                        (CAPTURE_HOLD, SWITCH)
                    } else {
                        (1.0, 0.0)
                    };
                    let scale = model.x_axis.truncate().length();
                    let local = model.inverse().transform_point3(centre) * scale;
                    let beyond = Vec2::new(
                        (local.x.abs() - width * scale / 2.0).max(0.0),
                        (local.y.abs() - height * scale / 2.0).max(0.0),
                    );
                    let reach = capture(viewer.distance(model.w_axis.truncate())) * hold;
                    let within = local.z <= reach
                        && local.z >= -BEHIND
                        && beyond.max_element() <= SNAP_MARGIN * hold;
                    within.then(|| {
                        (
                            (beyond.length() - nearer, local.z.abs()),
                            Aim {
                                wall: wall.id.clone(),
                                slot: slot as u32,
                                model,
                                fit: Fit::Slot(Cells::of(wall)),
                            },
                        )
                    })
                })
        })
        .min_by(|(a, _), (b, _)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .map(|(_, aim)| aim)
}

/// Where a panel carried is drawn docking in a slot standing at `slot`: as the slot stands, at its size, just in front
/// of it.
fn docked(slot: Mat4) -> Mat4 {
    let (scale, rotation, position) = slot.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(
        scale,
        rotation,
        position + rotation * Vec3::Z * DOCK_LIFT,
    )
}

/// Between `from` and `to`, `share` of the way.
fn blend(from: Mat4, to: Mat4, share: f32) -> Mat4 {
    let (scale, rotation, position) = from.to_scale_rotation_translation();
    let (to_scale, to_rotation, to_position) = to.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(
        scale.lerp(to_scale, share),
        rotation.slerp(to_rotation, share),
        position.lerp(to_position, share),
    )
}

/// Whether `place`, where the server says a panel is, is where it was put, `target`.
fn agrees(place: &Place, target: &Target) -> bool {
    match (place, target) {
        (Place::Wall { wall, slot }, Target::Wall(target, chosen)) => {
            wall == target && slot == chosen
        }
        (Place::Free { pose }, Target::Free(target) | Target::Sized(target, _)) => {
            Vec3::from_array(pose.position).distance(Vec3::from_array(target.position)) < 0.01
        }
        _ => false,
    }
}

/// `walls`, the Tomori wall standing where it is drawn while it travels, `travelling`.
fn travelled(mut walls: Vec<Wall>, travelling: Option<(Pose, f32)>) -> Vec<Wall> {
    if let Some((pose, _)) = travelling {
        for wall in walls.iter_mut().filter(|wall| wall.id == TOMORI_WALL) {
            wall.pose = pose;
        }
    }
    walls
}

/// Of `walls`, those a panel can be put on and taken hold of: not the Tomori wall while it is `travelling`.
fn pickable<'a>(walls: &[(&'a Wall, Mat4)], travelling: bool) -> Vec<(&'a Wall, Mat4)> {
    walls
        .iter()
        .filter(|(wall, _)| !(travelling && wall.id == TOMORI_WALL))
        .copied()
        .collect()
}

/// Where a wall added by a participant whose head is at `head` stands: [`WALL_DISTANCE`] ahead of them, level, a
/// little below their eyes, upright and facing them.
pub fn added_wall(head: Pose) -> Pose {
    let at = Vec3::from_array(head.position);
    let rotation = Quat::from_array(head.orientation);
    let forward = (rotation.is_finite() && rotation.length() > 0.5)
        .then(|| rotation.normalize() * Vec3::NEG_Z)
        .and_then(|forward| Vec3::new(forward.x, 0.0, forward.z).try_normalize())
        .unwrap_or(Vec3::NEG_Z);
    let position = at + forward * WALL_DISTANCE - Vec3::Y * WALL_DROP;
    Pose::upright_facing(position.to_array(), head.position)
}

/// The panel being carried.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    panel: String,
    /// How far along the ray it is carried, and how far it is turned from facing the participant.
    manipulation: Manipulation,
    /// Where it is carried, on the ray, which it stays at while the ray is not tracked.
    model: Mat4,
    /// How far it is drawn pulled from where it is carried towards the slot it docks in, from 0 to 1, and where it is
    /// pulled towards, which glides from slot to slot.
    pull: f32,
    toward: Mat4,
    /// Where it is drawn: where it is carried, pulled towards the slot it docks in; and how its face is laid out: as that
    /// slot lays it out while it docks, so that it is drawn at the slot's size, and fitting its text otherwise.
    drawn: Mat4,
    fit: Fit,
    /// How large it is, as a proportion of its own size: as it floated, or as both hands stretched it.
    size: f32,
    /// How far from its hand's ray it is carried, settling onto the ray, and how far it is turned beyond its snaps:
    /// where it was as the other of two hands let go of it, so that it does not jump.
    shift: Vec3,
    turn: f32,
    /// Its version as a second hand took hold of it, once both hands have held it, so that its size is sent with it.
    stretched: Option<u64>,
}

/// The way `model` faces about the vertical, in radians, from +Z towards +X.
fn yaw_of(model: Mat4) -> f32 {
    let facing = model.z_axis.truncate();
    facing.x.atan2(facing.z)
}

impl Held {
    /// Carried from `model`, docking nowhere yet, `size` times as large as its own size.
    fn new(panel: String, distance: f32, model: Mat4, size: f32) -> Self {
        Self {
            panel,
            manipulation: Manipulation::new(distance),
            model,
            pull: 0.0,
            toward: model,
            drawn: model,
            fit: Fit::Text,
            size,
            shift: Vec3::ZERO,
            turn: 0.0,
            stretched: None,
        }
    }

    /// Carried on along `ray` alone from where it stands, facing a participant at `viewer`, as the other of two hands
    /// let go: as far along the ray as it is, and as turned, settling onto the ray from where it is.
    fn anchor(&mut self, ray: Ray, viewer: Vec3) {
        let middle = self.model.w_axis.truncate();
        self.manipulation = Manipulation::new((middle - ray.origin).dot(ray.direction));
        self.shift = middle - ray.at(self.manipulation.distance());
        let square = facing(middle, Vec3::new(viewer.x, middle.y, viewer.z), 1.0);
        self.turn = yaw_of(self.model) - yaw_of(square);
    }

    /// Carried along `ray`, facing a participant at `viewer`, as far along it and as turned as its stick puts it, at its
    /// size, settling onto the ray over `dt` seconds.
    fn follow(&mut self, ray: Ray, viewer: Vec3, dt: f32) {
        self.shift *= (-dt.max(0.0) / SETTLE).exp();
        // Upright, as it floats: facing the viewer turned only about the vertical, then by its snaps.
        let at = ray.at(self.manipulation.distance()) + self.shift;
        self.model = facing(at, Vec3::new(viewer.x, at.y, viewer.z), 1.0)
            * Mat4::from_rotation_y(self.manipulation.yaw() + self.turn)
            * Mat4::from_scale(Vec3::splat(self.size));
    }

    /// Where it would go if let go now, floating: at its size, where both hands changed it.
    fn floating(&self) -> Target {
        let pose = pose(self.model);
        match self.stretched {
            Some(version) => Target::Sized(
                pose,
                Resized {
                    size: self.size,
                    version,
                },
            ),
            None => Target::Free(pose),
        }
    }

    /// Glide towards where it docks, `dock`, laid out as that slot lays it out, or back to where it is carried, fitting
    /// its text, over `dt` seconds.
    fn glide(&mut self, dock: Option<(Mat4, Fit)>, dt: f32) {
        self.fit = dock.map_or(Fit::Text, |(_, fit)| fit);
        let dock = dock.map(|(model, _)| model);
        let step = 1.0 - (-dt.max(0.0) / PULL_TIME).exp();
        if let Some(dock) = dock {
            // From where it is carried, it is pulled straight to the slot; between slots, it glides from one to the other.
            self.toward = if self.pull < 0.01 {
                dock
            } else {
                blend(self.toward, dock, step)
            };
        }
        let pull = if dock.is_some() { 1.0 } else { 0.0 };
        self.pull += (pull - self.pull) * step;
        self.drawn = blend(self.model, self.toward, self.pull);
    }
}

/// A panel both hands hold: the one carrying it, [`Projector::held`], and the other, `second`, which took hold of it
/// too, and where both took hold of it, with where it stood and how large it was then.
#[derive(Clone, Debug, PartialEq)]
struct Both {
    second: Side,
    hold: two_hands::Hold,
    model: Mat4,
    size: f32,
    /// When its size last changed, while both hold it, so that its size shows for [`SIZE_SHOWN`] after.
    changed: Option<Instant>,
    /// Whether it is at the least or the most it may be, so that reaching either ticks.
    bounded: bool,
}

/// Whether `point` is within `reach` of a face of `size` drawn with `model`: in front of it or behind it by at most that,
/// and within that of its edges, as a hand touching it is.
fn touches(model: Mat4, (width, height): (f32, f32), point: Vec3, reach: f32) -> bool {
    let scale = model.x_axis.truncate().length();
    let local = model.inverse().transform_point3(point) * scale;
    local.z.abs() <= reach
        && local.x.abs() <= width * scale / 2.0 + reach
        && local.y.abs() <= height * scale / 2.0 + reach
}

/// The marks at the corners of something both hands hold, a panel or a wall, drawn with `model`, its corners `corners`,
/// in its own frame, top left, top right, bottom left and bottom right, as [`editing`] numbers a wall's: each a small
/// bright angle just outside its corner, its arms along its edges, faintly, so that what both hands hold reads as held.
fn corner_marks(mark: &Arc<Canvas>, model: Mat4, corners: [Vec3; 4]) -> Vec<Sheet> {
    let scale = model.x_axis.truncate().length().max(1e-3);
    let span = corners[0].distance(corners[3]) * scale;
    let size = (span * MARK_SHARE).clamp(MARK_SIZES.0, MARK_SIZES.1) / scale;
    let gap = MARK_GAP / scale;
    let (pixels, _) = mark.size_in_pixels();
    // Each turned from the top left's, whose arms reach right and down, to reach along its own edges.
    let turns = [0.0, -0.5, 0.5, 1.0].map(|turn: f32| turn * std::f32::consts::PI);
    corners
        .into_iter()
        .zip(turns)
        .map(|(corner, turn)| Sheet {
            canvas: Some(mark.clone()),
            model: Some(
                model
                    * Mat4::from_translation(corner)
                    * Mat4::from_rotation_z(turn)
                    * Mat4::from_translation(Vec3::new(
                        size / 2.0 - gap,
                        gap - size / 2.0,
                        2.0 * LIFT / scale,
                    ))
                    * Mat4::from_scale(Vec3::splat(size / pixels as f32)),
            ),
        })
        .collect()
}

/// How large each corner mark is, as a proportion of the diagonal of what both hands hold, and at the least and most, in
/// metres, and how far outside its corner it stands.
const MARK_SHARE: f32 = 0.05;
const MARK_SIZES: (f32, f32) = (0.03, 0.25);
const MARK_GAP: f32 = 0.012;

/// The face of a corner mark: an angle, its arms reaching right and down from its top left, a pixel a font pixel.
fn mark_face() -> Canvas {
    let mut canvas = Canvas::new(4, 1.0);
    canvas.padding = (0, 0);
    canvas.background = [0.0; 4];
    canvas.rows = vec![Row::default(); 3];
    let (width, _) = canvas.size_in_pixels();
    let (length, thick) = (width as f32 * 0.8, width as f32 * 0.12);
    let color = [HIGHLIGHT[0], HIGHLIGHT[1], HIGHLIGHT[2], 0.85];
    canvas.shapes = vec![
        crate::shapes::Shape::Rule {
            from: [0.0, thick / 2.0],
            to: [length, thick / 2.0],
            width: thick,
            color,
        },
        crate::shapes::Shape::Rule {
            from: [thick / 2.0, 0.0],
            to: [thick / 2.0, length],
            width: thick,
            color,
        },
    ];
    canvas
}

/// The readout of how large a panel both hands hold is, as a proportion of its own size, such as `120%`, saying so at
/// the least or the most it may be.
pub fn size_readout(size: f32) -> Canvas {
    let percent = (size * 100.0).round();
    let text = if size <= tomori_protocol::FLOAT_SIZES.0 + 1e-4 {
        format!("{percent:.0}% · smallest")
    } else if size >= tomori_protocol::FLOAT_SIZES.1 - 1e-4 {
        format!("{percent:.0}% · largest")
    } else {
        format!("{percent:.0}%")
    };
    let columns = crate::ui_text::width(&text) + 2;
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(HIGHLIGHT);
    canvas.rows = vec![Row::text(
        &format!(" {text}"),
        crate::ui_text::Style::bold(crate::transcript::BRIGHT),
        columns,
    )];
    canvas
}

/// The readout `canvas` standing just above a face of `height` drawn with `model`, facing `viewer`, drawn at
/// [`HINT_ANGLE`] a font pixel whatever its distance.
fn readout_above(mut canvas: Canvas, model: Mat4, height: f32, viewer: Vec3) -> Sheet {
    let middle = model.w_axis.truncate();
    let distance = middle.distance(viewer).max(0.1);
    canvas.pixel = HINT_ANGLE * distance;
    let (_, tall) = canvas.size();
    let up = model.transform_vector3(Vec3::Y);
    let above = middle + up * (height / 2.0) + Vec3::Y * (HINT_GAP * distance + tall / 2.0);
    Sheet {
        model: Some(facing(above, viewer, 1.0)),
        canvas: Some(crate::frame_timing::drawn(canvas)),
    }
}

/// A panel let go, shown where it was let go until the server's projection agrees or [`PENDING`] passes.
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    panel: String,
    target: Target,
    model: Mat4,
    since: Instant,
}

/// What the projector needs this frame.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    /// What is projected in the space, while it is shown.
    pub projection: Option<&'a Projection>,
    /// The pointing ray, while it can point at panels.
    pub ray: Option<Ray>,
    /// The right grip's value, while its action is active.
    pub grip: Option<f32>,
    /// The point a carried panel faces.
    pub viewer: Option<Vec3>,
    /// Whether the ray and grip act on the panels: in a space, with nothing else taking them, such as the hand menu, the
    /// right wrist's transcript or a card carried. Otherwise a panel carried is let go where it is.
    pub active: bool,
    /// Whether the ray points at panels: not while a teleport is aimed, when a panel carried stays in the hand, and
    /// nothing is pointed at or pulled off. See `design/locomotion.md`.
    pub pointing: bool,
    /// How far along the ray something else it meets is, such as a card, which the grip picks up instead of a panel
    /// behind it.
    pub nearer: f32,
    /// The right stick, while what the right hand carries takes it: forward and back push it further or pull it closer,
    /// and a flick sideways turns it a snap. What the left hand carries takes the left stick, [`Hold::stick`].
    pub stick: Vec2,
    /// The right stick was clicked while a panel was carried: it turns back to face the participant square.
    pub straighten: bool,
    /// The trigger is held: a wall carried snaps to no other, so that let go it stays where it is.
    pub free: bool,
    /// The controls the hint beside a panel carried names.
    pub controls: Controls,
    /// A panel to scroll, this participant's own view of it, which nobody else sees: the one pointed at as the right
    /// stick was pushed or the mouse's wheel turned, and how many rows down, or up where fewer than none.
    pub scroll: Option<(&'a str, f32)>,
    /// What the participant may do with walls.
    pub rights: editing::Rights,
    /// The environment's declared surfaces, its floors and decks, carried into the space: a locked wall's padlock beyond
    /// one is not shown, and the ray takes no padlock, handle, bar or wall beyond one. `None` where nothing is declared.
    pub surfaces: Option<&'a crate::locomotion::Locomotion>,
    /// The left hand's ray, grip and trigger, which act on the panels and walls as the right's above do: it points,
    /// pulls off and carries a panel of its own, and takes hold of a wall and its handles. Each hand's stick acts on what
    /// that hand carries; the right stick's click on what the right hand carries, or else on what the left does. See
    /// `design/interaction.md`.
    pub left: Hold,
    pub now: Instant,
    /// Seconds since the last frame.
    pub dt: f32,
}

/// One hand's ray, grip and trigger, as [`Input`] gives the right's, for the left hand.
#[derive(Clone, Copy, Debug)]
pub struct Hold {
    /// The hand's ray, while it can point at panels.
    pub ray: Option<Ray>,
    /// The hand's grip's value, while its action is active.
    pub grip: Option<f32>,
    /// Whether the hand's ray and grip act on the panels, as [`Input::active`] says of the right's.
    pub active: bool,
    /// Whether the hand's ray points at panels: not while it aims a teleport, as [`Input::pointing`] says.
    pub pointing: bool,
    /// How far along the hand's ray something else it meets is, which its grip takes instead of a panel behind it.
    pub nearer: f32,
    /// The hand's trigger is held: a wall it carries snaps to no other.
    pub free: bool,
    /// The hand's own stick, while what the hand carries takes it, as [`Input::stick`] says of the right's.
    pub stick: Vec2,
}

impl Default for Hold {
    /// A hand that points at nothing and holds nothing.
    fn default() -> Self {
        Self {
            ray: None,
            grip: None,
            active: false,
            pointing: false,
            nearer: f32::INFINITY,
            free: false,
            stick: Vec2::ZERO,
        }
    }
}

/// What the projector did for the left hand this frame, as [`Output`]'s fields of the same names say for the right's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Left {
    pub pointed: Option<(String, f32, Vec3)>,
    pub item: Option<u32>,
    pub chip: Option<String>,
    pub part: Option<Part>,
    pub placed: Option<(String, Target)>,
    pub preview: Option<(String, u32)>,
    pub end: Option<Vec3>,
    pub hit: Option<crate::pointing::End>,
    pub hint: Option<Sheet>,
}

/// What the projector does for one hand in a frame, before it is told as the right's or the left's.
#[derive(Clone, Debug, Default)]
struct Told {
    pointed: Option<(String, f32, Vec3)>,
    item: Option<u32>,
    chip: Option<String>,
    part: Option<Part>,
    placed: Option<(String, Target)>,
    end: Option<Vec3>,
    hit: Option<crate::pointing::End>,
    hint: Option<Sheet>,
}

/// What the projector did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// The walls, each with the panels on it, and the floating panels, to draw, farthest first.
    pub sheets: Vec<Sheet>,
    /// The panel the ray points at, by identity, and how far along the ray and where it meets it.
    pub pointed: Option<(String, f32, Vec3)>,
    /// The line of the body of the panel pointed at that the list item the ray meets begins on, as [`item_at`] finds it.
    pub item: Option<u32>,
    /// The chip of the context of the panel pointed at that the ray meets, by its parameter, which words begun now
    /// name. See `design/panel-context.md`.
    pub chip: Option<String>,
    /// The node or edge of the volume pointed at that the ray meets, which words begun now name. See
    /// `design/diagrams.md`.
    pub part: Option<Part>,
    /// The volumes to draw, their solids among the opaque bodies; their labels are among [`Self::sheets`].
    pub volumes: Vec<volume::Drawn>,
    /// A panel was pulled off.
    pub picked: bool,
    /// The panel carried turned a snap, or back to square.
    pub turned: bool,
    /// A panel was let go, to send to the server: into a wall's slot, or floating.
    pub placed: Option<(String, Target)>,
    /// The wall and slot a panel carried would be put in if let go now, whose outline is drawn.
    pub preview: Option<(String, u32)>,
    /// Where the ray ends on a panel: where it meets the panel pointed at, or the middle of the one carried.
    pub end: Option<Vec3>,
    /// Where the ray ends on what is projected, and how far along it: on a panel carried, a wall carried or a handle
    /// dragged, held, or else the nearest of the panel, the handle and the wall it meets, where the participant may
    /// take hold of a wall.
    pub hit: Option<crate::pointing::End>,
    /// The hint beside the panel carried, saying what each control does to it, drawn over everything else.
    pub hint: Option<Sheet>,
    /// A change to a wall to send, made with its handles or by carrying it.
    pub wall: Option<editing::Change>,
    /// A wall to lock, unlock or put back, as an owner squeezed its padlock or `↺`.
    pub lock: Option<editing::LockChange>,
    /// A wall was taken hold of, or the handle the ray meets changed, or the wall carried turned: to tick.
    pub wall_ticked: bool,
    /// The readout over the chart pointed at, where the ray meets its plot, drawn in its plane over its face, in place
    /// of the label on the ray; and where across its time axis that is, in [`tomori_protocol::chart::ACROSS`] parts,
    /// which pointing says as the words begin. See `design/projection.md`.
    pub readout: Option<Sheet>,
    pub across: Option<u16>,
    /// The readouts linked to it, at the same time, over every other chart sharing its time, as their time axes say,
    /// never their labels: live charts on its wall, by how long before now, where it is live, or fixed windows anywhere,
    /// by the instant, where it is one, at most [`LINKED`], each drawn in its chart's plane over its face. See the
    /// linked time cursor in `design/projection.md`.
    pub linked: Vec<Sheet>,
    /// Which hand's ray the wall editor followed this frame, which [`Self::wall_ticked`] is felt on.
    pub wall_hand: Side,
    /// The hand that took hold, with the other, of what the other holds this frame, a panel or a wall, so that both
    /// hold it: to feel on it.
    pub joined: Option<Side>,
    /// What both hands hold reached the least or the most it may be, as they stretch or shrink it: to tick on the hand
    /// that took hold of it second.
    pub bounded: Option<Side>,
    /// What it did for the left hand: every field above but these is the right hand's.
    pub left: Left,
}

/// The panels across frames: the one carried, the slot it would be put in, and the one let go that the server has not
/// yet confirmed.
#[derive(Clone, Debug, Default)]
pub struct Projector {
    grip: Hands<Grip>,
    /// The panel each hand carries, the slot it would be put in, and the one each let go that the server has not yet
    /// confirmed.
    held: Hands<Option<Held>>,
    aim: Hands<Option<Aim>>,
    pending: Hands<Option<Pending>>,
    /// The hand whose ray and grip the wall editor follows: the one that took hold of the wall, or last squeezed.
    editor_hand: Side,
    /// A panel one hand carries that the other has taken hold of too, so that both hold it.
    both: Option<Both>,
    /// Whether each hand's squeeze would take hold, with it, of what the other hand holds, as things stood this frame:
    /// its ray on it, or its hand touching it, so that the squeeze is the carrying's. See [`Projector::joins`].
    joinable: Hands<bool>,
    /// The corner marks' face, drawn once.
    mark: Option<Arc<Canvas>>,
    /// A panel thrown away, hidden from when it was until the server's projection no longer shows it, or [`PENDING`]
    /// passes.
    thrown: Option<(String, Instant)>,
    /// How far down each panel is scrolled, in rows, this participant's own, kept while it is projected: in a slot again
    /// after floating, it is as it was.
    scrolls: HashMap<String, f32>,
    /// Each panel's faces laid out, and how far it scrolls as it stands this frame, laid out again only when what it
    /// shows changes, as the server's patch upserts it, or its size does. See `design/rendering.md`.
    faces: HashMap<String, faces::Faces>,
    /// Each wall's own faces, kept while what they are drawn from is the same, and forgotten once it is not shown.
    walls: walls::WallFaces,
    /// Whether it lays out at most [`faces::LAYOUTS`] panels a frame for what the server changed, as the frame step's
    /// does, or every one in the frame it changes, as by default. See [`Projector::paced`].
    paced: bool,
    /// Whether a paced projector draws a panel whose text is too small to resolve as its title alone. See [`PanelLod`].
    lod: PanelLod,
    /// Where among the projection's panels the next frame begins following them, after the last panel this one laid
    /// out in its turn, so that where more panels changed than a frame lays out, each is laid out in turn.
    turn: usize,
    /// The wall taken hold of, its handles, and a wall carried.
    editor: editing::Editor,
    /// The Tomori wall travelling, as an older server moves it, and how far folded it is this frame, from 0, unfolded,
    /// to 1.
    travel: travel::Travel,
    fold: f32,
    /// What this participant marks of the space's lists: the choosable row the ray points at, and the one just chosen.
    marks: list::RowMarks,
    /// The readout last drawn over a chart pointed at, with the face it was drawn over and what it shows, handed over
    /// again while it shows the same, as while the ray is still.
    readout: Option<(Arc<Canvas>, chart_readout::Readout, Arc<Canvas>)>,
    /// Each readout last linked to it, by the panel it was drawn over, handed over again while it shows the same.
    linked: HashMap<String, (Arc<Canvas>, chart_readout::Readout, Arc<Canvas>)>,
    /// What each volume draws, by its panel, built once for each diagram it shows. See `design/diagrams.md`.
    meshes: HashMap<String, Arc<volume::Mesh>>,
    /// The node this participant focuses on, of which volume, which only they see; and whether each hand's trigger was
    /// held last frame, which focuses.
    focus: Option<(String, usize)>,
    triggers: Hands<bool>,
    /// When the projector first drew, from which the flows' movement is timed.
    started: Option<Instant>,
}

/// What the ray meets of a volume pointed at: its node, by its id, or its edge, by its index, which words begun now
/// name to the agent. See `design/diagrams.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Node(String),
    Edge(u16),
    /// A world edge, by its index among the diagram's world edges.
    WorldEdge(u16),
}

/// A panel as it stands this frame: where, how its face is sized, and how large it is.
type Standing<'a> = (&'a Panel, Mat4, Fit);

/// Whether a paced projector draws a panel in a wall's slot as its title alone where a row of its text subtends too
/// small an angle from the eye to be resolved ([`PanelLod::Angle`], the default), or every panel whole however far
/// ([`PanelLod::Off`]), as a client's setting, the headset's `tomori.panel_lod` intent extra or `TOMORI_PANEL_LOD`,
/// chooses, so that a frame's cost can be measured with it and without. See `design/rendering.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelLod {
    Off,
    #[default]
    Angle,
}

impl PanelLod {
    /// What `value` asks for: `off`, `none`, `0` or `false` for [`PanelLod::Off`], and [`PanelLod::Angle`] for `angle`,
    /// `on`, when absent, and, with a warning, when not understood.
    ///
    /// ```
    /// use tomori_scene::projection::PanelLod;
    ///
    /// assert_eq!(PanelLod::parse(Some("off")), PanelLod::Off);
    /// assert_eq!(PanelLod::parse(Some("angle")), PanelLod::Angle);
    /// assert_eq!(PanelLod::parse(None), PanelLod::Angle);
    /// ```
    pub fn parse(value: Option<&str>) -> Self {
        match value
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            None | Some("" | "angle" | "on" | "1" | "true") => Self::Angle,
            Some("off" | "none" | "0" | "false") => Self::Off,
            Some(other) => {
                log::warn!("Panel level of detail {other:?} is not off or angle; drawing by angle");
                Self::Angle
            }
        }
    }

    /// How it is named in the log: `off` or `angle`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Angle => "angle",
        }
    }

    /// What it does, as a client logs it as it starts.
    ///
    /// ```
    /// use tomori_scene::projection::PanelLod;
    ///
    /// assert_eq!(PanelLod::Off.describe(), "Panel level of detail: off, every panel drawn whole");
    /// assert_eq!(
    ///     PanelLod::Angle.describe(),
    ///     "Panel level of detail: angle, a wall's panel drawn as its title once a row of its text subtends under 0.090°, whole again over 0.108°"
    /// );
    /// ```
    pub fn describe(self) -> String {
        match self {
            Self::Off => "Panel level of detail: off, every panel drawn whole".to_owned(),
            Self::Angle => format!(
                "Panel level of detail: angle, a wall's panel drawn as its title once a row of its text subtends under {:.3}°, whole again over {:.3}°",
                faces::FAR.to_degrees(),
                faces::NEAR.to_degrees()
            ),
        }
    }
}

impl std::fmt::Display for PanelLod {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

impl Projector {
    /// A projector that lays out at most a few panels a frame of those the server sent or changed, each in its turn:
    /// [`faces::ARRIVALS`] new ones, each standing from the frame it is laid out in, and [`faces::LAYOUTS`] changed
    /// ones, each showing what it showed until then, so that many panels arriving or changing together, as on entering
    /// a space or an environment's on one tick, never cost one frame all their layout. The frame step's is paced; [`Projector::default`] lays out every panel in the frame it
    /// arrives or changes, as a console or a still picture wants. See `design/rendering.md`.
    ///
    /// ```
    /// use tomori_protocol::{Panel, Place, Pose, Projection, Wall, WallKind};
    /// use tomori_scene::projection::{Controls, Input, Projector, editing::Rights};
    ///
    /// let pose = Pose::upright_facing([0.0, 1.5, -3.0], [0.0, 1.5, 0.0]);
    /// let panels = (0..24).map(|slot| {
    ///     let place = Place::Wall { wall: "wall-1".into(), slot };
    ///     Panel::new(format!("panel-{slot}"), "Notes", "A note.", place)
    /// });
    /// let projection = Projection::new("space", "sky")
    ///     .with_walls(vec![Wall::new("wall-1", WallKind::Added, pose).with_grid((8, 3).into())])
    ///     .with_panels(panels.collect());
    /// let input = || Input {
    ///     projection: Some(&projection),
    ///     ray: None,
    ///     grip: None,
    ///     viewer: None,
    ///     active: true,
    ///     pointing: false,
    ///     nearer: f32::INFINITY,
    ///     stick: glam::Vec2::ZERO,
    ///     straighten: false,
    ///     free: false,
    ///     controls: Controls::Controllers,
    ///     scroll: None,
    ///     rights: Rights::default(),
    ///     surfaces: None,
    ///     left: tomori_scene::projection::Hold::default(),
    ///     now: std::time::Instant::now(),
    ///     dt: 1.0 / 72.0,
    /// };
    /// let shown = |projector: &mut Projector| {
    ///     let sheets = projector.update(input()).sheets;
    ///     sheets.iter().filter(|sheet| sheet.canvas.is_some()).count()
    /// };
    /// // The wall, and every panel at once; or, paced, eight more panels each frame.
    /// assert_eq!(shown(&mut Projector::default()), 1 + 24);
    /// let mut paced = Projector::paced();
    /// assert_eq!([shown(&mut paced), shown(&mut paced), shown(&mut paced)], [1 + 8, 1 + 16, 1 + 24]);
    /// ```
    pub fn paced() -> Self {
        Self {
            paced: true,
            ..Self::default()
        }
    }

    /// Draw panels too small to read as their titles by angle, as by default, or every panel whole: from the next
    /// frame, a panel drawn as its title drawn whole again at once.
    pub fn set_lod(&mut self, lod: PanelLod) {
        self.lod = lod;
    }

    /// Whether panels too small to read are drawn as their titles. See [`Projector::set_lod`].
    pub fn lod(&self) -> PanelLod {
        self.lod
    }

    /// Follow `input`, and say what to draw and send.
    pub fn update(&mut self, input: Input<'_>) -> Output {
        let hands = Hands {
            right: Hold {
                ray: input.ray,
                grip: input.grip,
                active: input.active,
                pointing: input.pointing,
                nearer: input.nearer,
                free: input.free,
                stick: input.stick,
            },
            left: input.left,
        };
        // The grips are followed even while the panels do not take them, so that a squeeze held as they start to is not
        // a new one.
        let mut edges = Hands::from_fn(|side| self.grip[side].update(hands[side].grip));
        let Some(projection) = input.projection else {
            self.both = None;
            self.joinable = Hands::default();
            self.held = Hands::default();
            self.aim = Hands::default();
            self.pending = Hands::default();
            self.thrown = None;
            self.scrolls.clear();
            self.faces.clear();
            self.walls = walls::WallFaces::default();
            self.editor.stop();
            self.meshes.clear();
            self.focus = None;
            return Output::default();
        };
        let viewer = input.viewer.unwrap_or(HOME);
        self.lay_out(projection, viewer);
        self.keep_meshes(projection);
        // The Tomori wall an older server moves folds, travels and unfolds; meanwhile nothing is put on it by hand,
        // nor is it taken hold of, and a wall the editor held lets go of it, as of a wall no longer shown. See
        // `design/projection.md`.
        let travelling = self.travel.follow(projection, input.now);
        self.fold = travelling.map_or(0.0, |(_, fold)| fold);
        if let Some((panel, since)) = &self.thrown
            && (projection.panel(panel).is_none()
                || input.now.saturating_duration_since(*since) >= PENDING)
        {
            self.thrown = None;
        }
        for side in Side::BOTH {
            if let Some(pending) = &self.pending[side] {
                let confirmed = projection
                    .panel(&pending.panel)
                    .is_none_or(|panel| agrees(&panel.place, &pending.target));
                if confirmed || input.now.saturating_duration_since(pending.since) >= PENDING {
                    self.pending[side] = None;
                }
            }
            // A panel dismissed by someone else is no longer carried.
            if let Some(held) = &self.held[side]
                && projection.panel(&held.panel).is_none()
            {
                self.held[side] = None;
            }
        }
        let mut output = Output::default();
        // Held with both hands, letting go of one leaves it carried by the other from where it is now, so that it does
        // not jump; letting go of both puts it down, as one hand does.
        if let Some(both) = &self.both {
            let second = both.second;
            let first = second.other();
            let gone = |side: Side| edges[side].1 || !hands[side].active;
            match (self.held[first].is_some(), gone(first), gone(second)) {
                (false, ..) => self.both = None,
                (true, false, false) => {}
                (true, false, true) => {
                    self.both = None;
                    self.held[first]
                        .iter_mut()
                        .zip(hands[first].ray)
                        .for_each(|(held, ray)| held.anchor(ray, viewer));
                }
                (true, true, false) => {
                    self.both = None;
                    self.held[second] = self.held[first].take();
                    self.held[second]
                        .iter_mut()
                        .zip(hands[second].ray)
                        .for_each(|(held, ray)| held.anchor(ray, viewer));
                    self.aim[second] = self.aim[first].take();
                }
                (true, true, true) => self.both = None,
            }
        }
        let wall_both = self.editor.stretching().then_some(self.editor_hand);
        if let Some(first) = wall_both {
            let second = first.other();
            let gone = |side: Side| edges[side].1 || !hands[side].active;
            match (gone(first), gone(second)) {
                (false, true) => self.editor.let_one_go(hands[first].ray),
                // The first letting go, the second holds it on alone, and the editor follows it.
                (true, false) => {
                    self.editor.let_one_go(hands[second].ray);
                    self.editor_hand = second;
                }
                _ => {}
            }
        }
        // The walls as drawn: one being changed as it is being changed, and the Tomori wall where it travels.
        let shown_walls = travelled(self.editor.walls(projection), travelling);
        let walls: Vec<(&Wall, Mat4)> = shown_walls
            .iter()
            .filter_map(|wall| model(&wall.frame()).map(|frame| (wall, frame)))
            .collect();
        let pickable = pickable(&walls, travelling.is_some());
        let mut told: Hands<Told> = Hands::default();
        // Each hand's stick acts on what that hand carries, a panel or the wall the editor carries for it; the right
        // stick's click, on what the right hand carries, or else on what the left does.
        let carries = |side: Side| {
            self.held[side].is_some()
                || (self.editor_hand == side && self.editor.carrying().is_some())
        };
        let steered = Side::BOTH.into_iter().find(|side| carries(*side));
        for side in Side::BOTH {
            let hand = hands[side];
            let (_, released) = edges[side];
            let steers = steered == Some(side);
            // Held with both hands, it follows the middle of the points they took, turns as the line between them does,
            // and grows or shrinks about that middle as they spread or close, in steps, within its bounds; the sticks
            // move it no further meanwhile.
            if let Some(both) = self
                .both
                .as_mut()
                .filter(|both| both.second.other() == side)
                && let Some(held) = &mut self.held[side]
            {
                if let (Some(first), Some(second)) = (hand.ray, hands[both.second].ray) {
                    let moved = both.hold.follow([first, second]);
                    let bounds = tomori_protocol::FLOAT_SIZES;
                    let size = two_hands::settled(held.size, both.size * moved.spread, bounds);
                    if (size - held.size).abs() > 1e-4 {
                        both.changed = Some(input.now);
                    }
                    let bounded = (size - bounds.0).abs() < 1e-4 || (size - bounds.1).abs() < 1e-4;
                    if bounded && !both.bounded {
                        output.bounded = Some(both.second);
                    }
                    both.bounded = bounded;
                    held.size = size;
                    held.model = moved.model(both.model, size / both.size);
                }
            }
            // The panel carried follows its hand's ray, facing the viewer, as far along it and as turned as its hand's
            // stick puts it.
            else if let Some(held) = &mut self.held[side]
                && hand.active
            {
                output.turned |= held.manipulation.update(hand.stick, input.dt);
                if steers
                    && input.straighten
                    && !(held.manipulation.square() && held.turn.abs() < 1e-5)
                {
                    held.manipulation.straighten();
                    held.turn = 0.0;
                    output.turned = true;
                }
                if let Some(ray) = hand.ray {
                    held.follow(ray, viewer, input.dt);
                }
            }
            // The slot it would be put in: the one it is carried near, wherever the ray points, and it is drawn pulled
            // towards it. See `design/projection.md`.
            let shown = self.aim[side].take();
            self.aim[side] = self.held[side].as_ref().and_then(|held| {
                dock(
                    &pickable,
                    held.model.w_axis.truncate(),
                    viewer,
                    shown.as_ref(),
                )
            });
            if let Some(held) = &mut self.held[side] {
                held.glide(
                    self.aim[side]
                        .as_ref()
                        .map(|aim| (docked(aim.model), aim.fit)),
                    input.dt,
                );
            }
            // Let go, or taken by something else, it goes into that slot, or floats where it is, as it is turned.
            if (released || !hand.active)
                && let Some(held) = self.held[side].take()
            {
                let (target, model) = match self.aim[side].take() {
                    Some(aim) => (Target::Wall(aim.wall, aim.slot), aim.model),
                    None => (held.floating(), held.model),
                };
                self.pending[side] = Some(Pending {
                    panel: held.panel.clone(),
                    target: target.clone(),
                    model,
                    since: input.now,
                });
                told[side].placed = Some((held.panel, target));
            }
        }
        let standing: Vec<Standing> = projection
            .panels
            .iter()
            .filter_map(|panel| {
                self.stands(panel, &walls)
                    .map(|(model, fit)| (panel, model, fit))
            })
            .collect();
        // Which are drawn far, then how large each panel shown is, and how far it scrolls, as it is sized now, and the one
        // scrolled, within that.
        self.distances(&standing, viewer);
        let sizes = self.lay_out_standing(&standing, true);
        if let Some((panel, rows)) = input.scroll
            && let Some(limit) = self.faces.get(panel).and_then(|faces| faces.limit)
            && rows.is_finite()
        {
            let limit = limit as f32;
            let scroll = self.scrolls.entry(panel.to_owned()).or_default();
            *scroll = (scroll.min(limit) + rows).clamp(0.0, limit);
        }
        // The nearest panel each hand's ray meets, other than one that hand carries, is pointed at; one the other hand
        // carries is met, so that the ray ends on it, but its grip takes nothing there.
        let mut right_pointed = None;
        for side in Side::BOTH {
            let hand = hands[side];
            let carried = self.held_in(side).map(str::to_owned);
            let pointed = hand
                .ray
                .filter(|_| hand.active && hand.pointing)
                .and_then(|ray| {
                    standing
                        .iter()
                        .zip(&sizes)
                        .filter(|((panel, ..), _)| Some(panel.id.as_str()) != carried.as_deref())
                        // Nothing on the Tomori wall is pointed at while it travels.
                        .filter(|((panel, ..), _)| {
                            travelling.is_none()
                                || !matches!(&panel.place, Place::Wall { wall, .. } if wall == TOMORI_WALL)
                        })
                        // A volume is met where the ray meets its face or, nearer, a node or an edge of it.
                        .filter_map(|((panel, model, fit), size)| {
                            let part = self.meet(panel, *model, *fit, &ray).map(|(_, distance)| distance);
                            let distance = [hit(*model, *size, &ray), part]
                                .into_iter()
                                .flatten()
                                .reduce(f32::min)?;
                            Some((*panel, *model, *fit, distance, ray.at(distance)))
                        })
                        // Nothing of a wall beyond a floor or deck is taken, its panels included.
                        .filter(|(panel, _, _, distance, _)| {
                            !matches!(panel.place, Place::Wall { .. })
                                || !editing::beyond(input.surfaces, &ray, *distance)
                        })
                        .min_by(|a, b| a.3.total_cmp(&b.3))
                });
            told[side].item = pointed.and_then(|(panel, model, fit, _, point)| {
                let scroll = self.scroll(&panel.id);
                self.faces_for(panel).item(fit, model, scroll, point)
            });
            told[side].chip = pointed.and_then(|(panel, model, fit, _, point)| {
                self.faces_for(panel).chip(fit, model, point)
            });
            let met = pointed
                .zip(hand.ray)
                .and_then(|((panel, model, fit, ..), ray)| self.meet(panel, model, fit, &ray))
                .map(|(met, _)| met);
            told[side].part = pointed
                .zip(met)
                .map(|((panel, ..), met)| self.named(panel, met));
            // The trigger, pressed on a node, focuses on it, for this participant alone, or, on the node focused on,
            // ends the focus; not where something nearer, such as a card, takes the ray, as it takes the grip.
            let trigger = hand.free && hand.active && hand.pointing;
            let pressed = trigger && !self.triggers[side];
            self.triggers[side] = trigger;
            // On a leader, it focuses on the node it comes from.
            let node = pointed.zip(met).and_then(|((panel, ..), met)| match met {
                volume::Met::Node(index) => Some(index),
                volume::Met::Leader(leader) => self
                    .meshes
                    .get(&panel.id)
                    .and_then(|mesh| mesh.leader_node(leader)),
                volume::Met::Edge(_) => None,
            });
            if pressed
                && self.held[side].is_none()
                && let (Some((panel, _, _, distance, _)), Some(index)) = (pointed, node)
                && distance < hand.nearer
            {
                let focus = (panel.id.clone(), index);
                self.focus = (self.focus.as_ref() != Some(&focus)).then_some(focus);
            }
            told[side].pointed =
                pointed.map(|(panel, _, _, distance, point)| (panel.id.clone(), distance, point));
            if side == Side::Right {
                right_pointed = pointed;
            }
        }
        // A squeeze of one hand on what the other holds, its ray on it or the hand touching it, takes hold of it too, so
        // that both hold it: a panel the other carries, or a wall it carries or holds by its board. Its ray takes it
        // however far away it is, so that one hand points and the other stretches. See `design/projection.md`.
        self.joinable = Hands::default();
        for side in Side::BOTH {
            let hand = hands[side];
            let other = side.other();
            if self.both.is_some() || self.editor.stretching() || self.held[side].is_some() {
                continue;
            }
            let pointed = told[side]
                .pointed
                .clone()
                .filter(|(_, distance, _)| *distance < hand.nearer);
            let carried = self.held[other].as_ref();
            let panel = standing
                .iter()
                .zip(&sizes)
                .find(|((panel, ..), _)| carried.is_some_and(|held| held.panel == panel.id))
                .and_then(|((panel, model, _), size)| {
                    let met = pointed
                        .as_ref()
                        .filter(|(id, ..)| *id == panel.id)
                        .map(|(_, distance, _)| *distance);
                    let touched = hand
                        .ray
                        .filter(|ray| hand.active && touches(*model, *size, ray.origin, TOUCH))
                        .map(|_| 0.0);
                    met.or(touched)
                });
            let wall = self
                .editor
                .held_wall()
                .filter(|_| self.editor_hand == other && panel.is_none())
                .and_then(|id| pickable.iter().find(|(wall, _)| wall.id == id))
                .and_then(|(wall, frame)| {
                    let frame = self.editor.carried_frame().unwrap_or(*frame);
                    let ray = hand.ray.filter(|_| hand.active)?;
                    // A panel on it is the wall held, as much as its board, slot or bar is; one nearer, elsewhere, is
                    // pulled off instead.
                    let nearer = pointed
                        .as_ref()
                        .filter(|(id, ..)| {
                            !projection.panel(id).is_some_and(|panel| {
                                matches!(&panel.place, Place::Wall { wall: on, .. } if *on == wall.id)
                            })
                        })
                        .map_or(hand.nearer, |(_, distance, _)| distance.min(hand.nearer));
                    let met = hand
                        .pointing
                        .then(|| wall_hit(wall, frame, &ray))
                        .flatten()
                        .filter(|along| *along < nearer);
                    let touched = (slot_frames(wall, frame).into_iter().any(|slot| {
                        touches(slot, slot_unit(wall), ray.origin, TOUCH)
                    }) || wall
                        .board()
                        .is_some_and(|board| touches(frame, board, ray.origin, TOUCH)))
                    .then_some(0.0);
                    met.or(touched).map(|along| (along, *wall, frame))
                });
            self.joinable[side] = panel.is_some() || wall.is_some();
            if !edges[side].0 {
                continue;
            }
            let (Some(first), Some(second)) = (hands[other].ray, hand.ray) else {
                continue;
            };
            if let Some(along) = panel
                && let Some(held) = &mut self.held[other]
            {
                let middle = held.model.w_axis.truncate();
                let reach = (middle - first.origin).dot(first.direction).max(0.0);
                self.both = Some(Both {
                    second: side,
                    hold: two_hands::Hold::new([first, second], [reach, along]),
                    model: held.model,
                    size: held.size,
                    changed: None,
                    bounded: false,
                });
                held.stretched = projection.panel(&held.panel).map(|panel| panel.version);
                edges[side].0 = false;
                output.joined = Some(side);
            } else if let Some((along, wall, frame)) = wall {
                self.editor.join(first, second, along, wall, frame);
                edges[side].0 = false;
                output.joined = Some(side);
            }
        }
        // The wall editor follows one hand at a time: the one that carries the wall or drags its handle, until it lets
        // go, or else the one that squeezes now, the right first, of those carrying no panel.
        if !self.editor.busy()
            && let Some(side) = Side::BOTH
                .into_iter()
                .find(|side| edges[*side].0 && self.held[*side].is_none())
        {
            self.editor_hand = side;
        }
        let editing = self.editor_hand;
        let (pressed, released) = edges[editing];
        let hand = hands[editing];
        let picking = self.held[editing].is_none();
        // The wall taken hold of, and its handles, take the squeeze first, a handle before a panel and a panel before the
        // wall it is on; a wall carried or a handle dragged is let go as the grip is.
        let edited = self.editor.update(
            editing::Input {
                projection,
                ray: hand.ray,
                pressed: pressed && picking,
                released,
                active: hand.active,
                pointing: hand.pointing,
                nearer: hand.nearer,
                panel: told[editing]
                    .pointed
                    .as_ref()
                    .map_or(f32::INFINITY, |(_, distance, _)| *distance),
                stick: if picking { hand.stick } else { Vec2::ZERO },
                straighten: input.straighten && steered == Some(editing) && picking,
                second: hands[editing.other()]
                    .ray
                    .filter(|_| self.editor.stretching()),
                free: hand.free,
                viewer,
                surfaces: input.surfaces,
                rights: input.rights,
                controls: input.controls,
                now: input.now,
                dt: input.dt,
            },
            &pickable,
        );
        output.wall = edited.change;
        output.lock = edited.lock;
        output.wall_ticked = edited.ticked;
        output.wall_hand = editing;
        // What the editor changed this frame is drawn this frame, the panels on a wall with it.
        let redrawn = travelled(self.editor.walls(projection), travelling);
        let walls: Vec<(&Wall, Mat4)> = redrawn
            .iter()
            .filter_map(|wall| model(&wall.frame()).map(|frame| (wall, frame)))
            .collect();
        let standing: Vec<Standing> = projection
            .panels
            .iter()
            .filter_map(|panel| {
                self.stands(panel, &walls)
                    .map(|(model, fit)| (panel, model, fit))
            })
            .collect();
        self.lay_out_standing(&standing, false);
        for side in Side::BOTH {
            let hand = hands[side];
            let (pressed, _) = edges[side];
            // What the other hand's ray meets of the walls, which the editor does not follow: a handle, a bar or a wall,
            // where the participant may take hold of one.
            let (reach, wall_hit) = if side == editing {
                (edited.reach, edited.hit)
            } else {
                hand.ray
                    .filter(|_| hand.active && hand.pointing)
                    .map_or((None, None), |ray| {
                        self.editor.probe(&ray, &pickable, input.rights)
                    })
            };
            // A handle the ray meets is pointed at rather than a panel behind it.
            if reach.is_some_and(|reach| {
                told[side]
                    .pointed
                    .as_ref()
                    .is_none_or(|(_, distance, _)| reach < *distance)
            }) {
                told[side].pointed = None;
                told[side].item = None;
                told[side].chip = None;
                told[side].part = None;
            }
            // A squeeze pulls off the panel pointed at, unless something nearer takes it, or the other hand carries it.
            let other = self.held_in(side.other()).map(str::to_owned);
            let took = side == editing && edited.took;
            if pressed
                && !took
                && self.held[side].is_none()
                && told[side]
                    .pointed
                    .as_ref()
                    .is_some_and(|(panel, ..)| Some(panel) != other.as_ref())
                && let Some((panel, distance, _)) = told[side]
                    .pointed
                    .take_if(|(_, distance, _)| *distance < hand.nearer)
            {
                let (model, size) = standing
                    .iter()
                    .find(|(shown, ..)| shown.id == panel)
                    .map(|(shown, model, _)| {
                        let floating = matches!(shown.place, Place::Free { .. });
                        (*model, if floating { shown.floating_size() } else { 1.0 })
                    })
                    .expect("pointed at a panel shown");
                self.held[side] = Some(Held::new(panel, distance, model, size));
                self.pending[side] = None;
                output.picked = true;
            }
            // The ray ends at the middle of the panel carried, where it is drawn, or where it meets the panel pointed at.
            let editor_end = if side == editing { edited.end } else { None };
            told[side].end = match &self.held[side] {
                Some(held) => Some(held.drawn.w_axis.truncate()),
                None => editor_end.or(told[side].pointed.as_ref().map(|(_, _, point)| *point)),
            };
            told[side].hit = match (&self.held[side], hand.ray.filter(|_| hand.active)) {
                (Some(_), Some(ray)) => told[side]
                    .end
                    .map(|end| crate::pointing::End::held(ray.origin, end)),
                (Some(_), None) => None,
                (None, _) => crate::pointing::end([
                    wall_hit,
                    told[side]
                        .pointed
                        .as_ref()
                        .map(|(_, distance, point)| crate::pointing::End::new(*distance, *point)),
                ]),
            };
        }
        let pointed: Vec<String> = Side::BOTH
            .into_iter()
            .filter_map(|side| told[side].pointed.as_ref().map(|(id, ..)| id.clone()))
            .collect();
        let faces = self.faces_of(projection, &standing, &pointed);
        output.sheets = self.sheets(&walls, &standing, &faces, viewer);
        // Each volume's solids, and its labels over the panels, and the card of the node this participant focuses on.
        let time = input
            .now
            .saturating_duration_since(*self.started.get_or_insert(input.now))
            .as_secs_f32();
        for (panel, model, fit) in &standing {
            let Some(mesh) = self.meshes.get(&panel.id) else {
                continue;
            };
            let model = *model * volume::placed(panel, mesh.source(), *fit);
            let focus = self
                .focus
                .as_ref()
                .filter(|(id, _)| *id == panel.id)
                .map(|(_, node)| *node);
            mesh.sheets(model, viewer, focus, &mut output.sheets);
            output
                .sheets
                .extend(focus.and_then(|node| mesh.card(node, model, viewer)));
            output.volumes.push(volume::Drawn {
                mesh: mesh.clone(),
                model,
                focus,
                time,
                viewer,
            });
        }
        // Pointing at a chart's plot with the right ray, not carrying a panel in that hand, its readout follows the ray,
        // drawn over its face.
        let read = right_pointed
            .filter(|(panel, ..)| {
                self.held.right.is_none()
                    && told
                        .right
                        .pointed
                        .as_ref()
                        .is_some_and(|(id, ..)| *id == panel.id)
            })
            .and_then(|(panel, model, fit, _, point)| {
                self.faces_for(panel)
                    .readout(fit, model, point)
                    .map(|read| (panel, model, read))
            });
        output.across = read.as_ref().map(|(panel, _, (at, ..))| {
            let last = panel
                .chart
                .as_ref()
                .map_or(1, |chart| chart.points.max(2) - 1);
            (at / last as f32 * f32::from(tomori_protocol::chart::ACROSS)).round() as u16
        });
        output.linked = self.linked(
            &standing,
            read.as_ref().map(|(panel, _, (at, ..))| (*panel, *at)),
        );
        output.readout = read.map(|(_, model, (_, face, readout))| {
            let canvas = match self.readout.take() {
                Some((kept_face, kept, canvas))
                    if Arc::ptr_eq(&kept_face, &face) && kept == readout =>
                {
                    (kept_face, kept, canvas)
                }
                _ => {
                    let canvas = crate::frame_timing::drawn(readout.overlay(&face));
                    (face, readout, canvas)
                }
            };
            let sheet = Sheet {
                canvas: Some(canvas.2.clone()),
                model: Some(model * Mat4::from_translation(Vec3::Z * READOUT_LIFT)),
            };
            self.readout = Some(canvas);
            sheet
        });
        // The handles of the wall taken hold of, and the hint below it, over the walls.
        output.sheets.extend(edited.sheets);
        // What both hands hold shows small marks at its corners, and a panel its size, for a moment after it changes.
        let mark = self
            .mark
            .get_or_insert_with(|| crate::frame_timing::drawn(mark_face()))
            .clone();
        if let Some(both) = &self.both
            && let Some(held) = &self.held[both.second.other()]
            && let Some(faces) = self.faces.get_mut(&held.panel)
        {
            let (width, height) = faces.at(held.fit).size;
            let (x, y) = (width / 2.0, height / 2.0);
            let corners = [
                Vec3::new(-x, y, 0.0),
                Vec3::new(x, y, 0.0),
                Vec3::new(-x, -y, 0.0),
                Vec3::new(x, -y, 0.0),
            ];
            output
                .sheets
                .extend(corner_marks(&mark, held.drawn, corners));
            if both
                .changed
                .is_some_and(|at| input.now.saturating_duration_since(at) < SIZE_SHOWN)
            {
                output.sheets.push(readout_above(
                    size_readout(held.size),
                    held.drawn,
                    height,
                    viewer,
                ));
            }
        }
        if let Some((wall, frame)) = self.editor.stretched() {
            output
                .sheets
                .extend(corner_marks(&mark, frame, editing::corners(&wall)));
        }
        // While a panel is carried, the hint beside it says what each control does to it, and what letting go would do,
        // from the frame after it is pulled off, once where it would go is known: the controls only beside the one they
        // act on, and only what letting go does beside the other.
        // The panel carried is always shown, as one no longer shown is no longer carried.
        for side in Side::BOTH {
            let carried = self.held[side]
                .as_ref()
                .filter(|_| hands[side].active && !output.picked)
                .and_then(|held| {
                    projection
                        .panel(&held.panel)
                        .map(|panel| (panel, held.drawn, held.fit))
                });
            let controls = steered == Some(side);
            told[side].hint = carried
                .map(|(panel, model, fit)| (panel, model, self.faces_for(panel).at(fit).size))
                .map(|(panel, model, size)| {
                    let release = match &self.aim[side] {
                        None => Release::Float,
                        Some(aim) => {
                            let taken = projection.panels.iter().any(|other| {
                                other.id != panel.id
                                    && matches!(&other.place, Place::Wall { wall, slot } if *wall == aim.wall && *slot == aim.slot)
                            });
                            if taken { Release::Swap } else { Release::Slot }
                        }
                    };
                    let canvas = if controls {
                        hint(input.controls, release)
                    } else {
                        let_go_hint(input.controls, release)
                    };
                    hint_sheet(canvas, model, size, viewer)
                });
        }
        let preview = |aim: &Option<Aim>| aim.as_ref().map(|aim| (aim.wall.clone(), aim.slot));
        let left = std::mem::take(&mut told.left);
        let right = std::mem::take(&mut told.right);
        output.left = Left {
            part: left.part.filter(|_| left.pointed.is_some()),
            pointed: left.pointed,
            item: left.item,
            chip: left.chip.clone(),
            placed: left.placed,
            preview: preview(&self.aim.left),
            end: left.end,
            hit: left.hit,
            hint: left.hint,
        };
        output.part = right.part.filter(|_| right.pointed.is_some());
        output.pointed = right.pointed;
        output.item = right.item;
        output.chip = right.chip.clone();
        output.placed = right.placed;
        output.preview = preview(&self.aim.right);
        output.end = right.end;
        output.hit = right.hit;
        output.hint = right.hint;
        output
    }

    /// Throw away the panel the right hand carries, or else the left's, at `now`: it is no longer carried, nor let go
    /// anywhere, and is hidden until the server's projection no longer shows it, or for [`PENDING`]. Returns its
    /// identity, if one was carried.
    pub fn throw(&mut self, now: Instant) -> Option<String> {
        let (side, held) = Side::BOTH
            .into_iter()
            .find_map(|side| self.held[side].take().map(|held| (side, held)))?;
        self.aim[side] = None;
        self.both = None;
        self.thrown = Some((held.panel.clone(), now));
        Some(held.panel)
    }

    /// The face of each of `standing`'s panels this frame, as it is laid out, sized and scrolled: framed brightly where it
    /// is carried or `pointed` at, and in blue where it is the newest on its wall, a list's rows marked as
    /// [`Self::mark_rows`] says. Each is the face drawn last frame unless one of those changed, or what the panel shows
    /// did.
    fn faces_of(
        &mut self,
        projection: &Projection,
        standing: &[Standing],
        pointed: &[String],
    ) -> Vec<Arc<Canvas>> {
        let highlighted: Vec<String> = Side::BOTH
            .into_iter()
            .filter_map(|side| self.held_in(side).map(str::to_owned))
            .chain(pointed.iter().cloned())
            .collect();
        let newest = newest(projection);
        let marks = self.marks.clone();
        standing
            .iter()
            .map(|(panel, _, fit)| {
                let newest = match &panel.place {
                    Place::Wall { wall, .. } => {
                        newest.get(wall.as_str()) == Some(&panel.id.as_str())
                    }
                    _ => false,
                };
                let border = frame(highlighted.contains(&panel.id), newest);
                let scroll = self.scroll(&panel.id);
                let faces = self.faces_for(panel);
                if faces.far {
                    crate::frame_timing::titles(1);
                    let (columns, rows) = self::fit(panel, *fit);
                    return faces.far_face(sized(columns, rows), border);
                }
                faces.face(*fit, (scroll, &marks), border)
            })
            .collect()
    }

    /// What to draw: each wall with the panels in its layout, and the outline of the slot a panel carried would take,
    /// and each panel floating or carried, the groups farthest from `viewer` first, since panels blend over what is drawn
    /// before them and write depth only once drawn, so that a nearer one covers one behind it, and a wall's panels cover
    /// the wall. Each of `standing` shows its face of `faces`.
    fn sheets(
        &mut self,
        walls: &[(&Wall, Mat4)],
        standing: &[Standing],
        faces: &[Arc<Canvas>],
        viewer: Vec3,
    ) -> Vec<Sheet> {
        let sheet = |face: &Arc<Canvas>, model: Mat4| Sheet {
            canvas: Some(face.clone()),
            model: Some(model),
        };
        // A panel drawn in a wall's slot belongs to that wall's group, unless it is carried or was let go.
        let moved: Vec<String> = Side::BOTH
            .into_iter()
            .flat_map(|side| {
                [
                    self.held_in(side).map(str::to_owned),
                    self.pending[side]
                        .as_ref()
                        .map(|pending| pending.panel.clone()),
                ]
            })
            .flatten()
            .collect();
        let moved = moved.as_slice();
        fn in_layout<'a>(panel: &'a Panel, moved: &[String]) -> Option<&'a str> {
            match &panel.place {
                Place::Wall { wall, .. } if !moved.contains(&panel.id) => Some(wall),
                _ => None,
            }
        }
        let drawn: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        self.walls.retain(&drawn);
        let mut groups: Vec<(f32, Vec<Sheet>)> = walls
            .iter()
            .map(|(wall, frame)| {
                // A flat wall is one board, drawn edge by edge in a group; a curved one, a board for each slot, and its
                // label below their middle, once for a group. Each is the face drawn before while what it is drawn
                // from is the same.
                let mut sheets = match arc(wall) {
                    None if wall.group.is_some() => self
                        .walls
                        .group(wall, &drawn)
                        .iter()
                        .map(|(face, model)| sheet(face, *frame * *model))
                        .collect(),
                    None => vec![sheet(
                        &self.walls.board(wall),
                        *frame * Mat4::from_scale(Vec3::splat(wall.scale() * self.folded(wall))),
                    )],
                    Some(_) => {
                        let (_, height) = wall.size();
                        let board = self.walls.slot(wall);
                        let mut boards: Vec<Sheet> = self
                            .frames(wall, *frame)
                            .into_iter()
                            .map(|model| sheet(&board, model))
                            .collect();
                        // Below its rows, since the status pill stands just above the Tomori wall from home; not
                        // while it is folded.
                        if boards::labelled(wall, &drawn) && self.folded(wall) == 1.0 {
                            boards.push(sheet(
                                &self.walls.label(wall),
                                *frame
                                    * Mat4::from_translation(
                                        -Vec3::Y * (height / 2.0 + ARC_LABEL_DROP),
                                    ),
                            ));
                        }
                        boards
                    }
                };
                sheets.extend(
                    standing
                        .iter()
                        .zip(faces)
                        .filter(|((panel, ..), _)| {
                            in_layout(panel, moved) == Some(wall.id.as_str())
                        })
                        .map(|((_, model, _), face)| sheet(face, *model)),
                );
                // The outline of the slot a panel carried would take, over any panel in it.
                for model in Side::BOTH.into_iter().filter_map(|side| {
                    self.aim[side]
                        .as_ref()
                        .filter(|aim| aim.wall == wall.id)
                        .map(|aim| aim.model)
                }) {
                    sheets.push(sheet(
                        &self.walls.ghost(wall),
                        model * Mat4::from_translation(Vec3::Z * LIFT),
                    ));
                }
                (frame.w_axis.truncate().distance(viewer), sheets)
            })
            .collect();
        groups.extend(
            standing
                .iter()
                .zip(faces)
                .filter(|((panel, ..), _)| in_layout(panel, moved).is_none())
                .map(|((_, model, _), face)| {
                    (
                        model.w_axis.truncate().distance(viewer),
                        vec![sheet(face, *model)],
                    )
                }),
        );
        groups.sort_by(|a, b| b.0.total_cmp(&a.0));
        groups.into_iter().flat_map(|(_, sheets)| sheets).collect()
    }

    /// Where `panel` stands, and how its face is sized: carried, fitting its text; let go but not yet confirmed, in the
    /// slot it was put in, or floating where it was let go; or where the server says it is, in its wall's slot, or
    /// floating, fitting its text. `None` for a panel thrown away, on a wall not shown, in a slot past the last, or in a
    /// place this client does not know.
    fn stands(&self, panel: &Panel, walls: &[(&Wall, Mat4)]) -> Option<(Mat4, Fit)> {
        // A panel whose turn to be laid out has not come does not yet stand.
        if !self.faces.contains_key(&panel.id)
            || self
                .thrown
                .as_ref()
                .is_some_and(|(thrown, _)| *thrown == panel.id)
        {
            return None;
        }
        let held = Side::BOTH.into_iter().find_map(|side| {
            self.held[side]
                .as_ref()
                .filter(|held| held.panel == panel.id)
        });
        let pending = Side::BOTH.into_iter().find_map(|side| {
            self.pending[side]
                .as_ref()
                .filter(|pending| pending.panel == panel.id)
        });
        match (held, pending) {
            (Some(held), _) => return Some((held.drawn, held.fit)),
            (_, Some(pending)) => {
                let fit = match &pending.target {
                    Target::Wall(on, _) => walls
                        .iter()
                        .find(|(shown, _)| shown.id == *on)
                        .map_or(Fit::SLOT, |(wall, _)| Fit::Slot(Cells::of(wall))),
                    Target::Free(_) | Target::Sized(..) => Fit::Text,
                };
                return Some((pending.model, fit));
            }
            _ => {}
        }
        match &panel.place {
            Place::Wall { wall, slot } => walls
                .iter()
                .find(|(shown, _)| shown.id == *wall)
                .and_then(|(wall, frame)| {
                    self.frames(wall, *frame)
                        .get(*slot as usize)
                        .map(|model| (*model, Fit::Slot(Cells::of(wall))))
                }),
            Place::Free { pose } => model(pose).map(|model| {
                (
                    model * Mat4::from_scale(Vec3::splat(panel.floating_size())),
                    Fit::Text,
                )
            }),
            _ => None,
        }
    }

    /// Where each of `wall`'s slots stands, given its `frame`, as [`slot_frames`] says, and, for the Tomori wall while it
    /// travels, folded as far as it is.
    fn frames(&self, wall: &Wall, frame: Mat4) -> Vec<Mat4> {
        let slots = slot_frames(wall, frame);
        if wall.id == TOMORI_WALL && self.fold > 0.0 {
            travel::fold(&slots, frame, self.fold)
        } else {
            slots
        }
    }

    /// How large `wall`'s own board is drawn, as a proportion of its size: smaller as the Tomori wall folds.
    fn folded(&self, wall: &Wall) -> f32 {
        if wall.id == TOMORI_WALL {
            travel::board_scale(self.fold)
        } else {
            1.0
        }
    }

    /// Whether the Tomori wall is travelling, as an older server moves it, folded or folding.
    pub fn travelling(&self) -> bool {
        self.travel.travelling()
    }

    /// Mark the rows of the space's lists as `marks` says, from this frame: the choosable row this participant points
    /// at, as A or the trigger would choose it, and the row they just chose. See `design/panel-actions.md`.
    pub(crate) fn mark_rows(&mut self, marks: list::RowMarks) {
        self.marks = marks;
    }

    /// A panel being carried, by identity: the right hand's, or else the left's.
    pub fn held(&self) -> Option<&str> {
        self.held_in(Side::Right).or(self.held_in(Side::Left))
    }

    /// The panel `side`'s hand carries, by identity.
    pub fn held_in(&self, side: Side) -> Option<&str> {
        self.held[side].as_ref().map(|held| held.panel.as_str())
    }

    /// The wall `side`'s hand carries, by identity.
    pub fn carried_wall_in(&self, side: Side) -> Option<&str> {
        self.editor.carrying().filter(|_| self.editor_hand == side)
    }

    /// Whether `side`'s hand carries a panel or a wall, alone or with the other hand.
    pub fn carrying_in(&self, side: Side) -> bool {
        self.held[side].is_some()
            || self.carried_wall_in(side).is_some()
            || self.both.as_ref().is_some_and(|both| both.second == side)
            || (self.editor.stretching() && self.editor_hand.other() == side)
    }

    /// Whether a squeeze of `side`'s grip now would take hold of what the other hand holds, with it, its ray on it or
    /// the hand touching it, as things stood the frame before: a panel the other hand carries, or a wall it carries or
    /// holds by its board.
    pub fn joins(&self, side: Side) -> bool {
        self.joinable[side]
    }

    /// Whether both hands hold one panel or one wall.
    pub fn two_handed(&self) -> bool {
        self.both.is_some() || self.editor.stretching()
    }

    /// The server refused a change to the panel `panel`, as made against a version since changed, as a size sent with
    /// both hands: it is drawn as the server has it at once.
    pub fn refused_panel(&mut self, panel: &str) {
        for side in Side::BOTH {
            if self.pending[side]
                .as_ref()
                .is_some_and(|pending| pending.panel == panel)
            {
                self.pending[side] = None;
            }
        }
    }

    /// Whether `panel`, shown last frame, has more than its face shows at once, so that it scrolls.
    pub fn scrollable(&self, panel: &str) -> bool {
        self.limit(panel).is_some_and(|limit| limit > 0)
    }

    /// How many rows down `panel` is scrolled, this participant's own, as far as it goes as it was last sized.
    pub fn scroll(&self, panel: &str) -> usize {
        let limit = self.limit(panel).unwrap_or_default();
        let scroll = self.scrolls.get(panel).copied().unwrap_or_default();
        (scroll.round() as usize).min(limit)
    }

    /// How far down `panel` scrolls at most, as it was sized when it last stood, this frame or the last.
    fn limit(&self, panel: &str) -> Option<usize> {
        self.faces.get(panel).and_then(|faces| faces.limit)
    }

    /// Lay out each of `standing`'s panels as it is sized, where it is not already, and say how large each is; with
    /// `limits`, keep how far each scrolls at most, as it is sized now, for [`Self::scroll`].
    fn lay_out_standing(&mut self, standing: &[Standing], limits: bool) -> Vec<(f32, f32)> {
        standing
            .iter()
            .map(|(panel, _, fit)| {
                let faces = self
                    .faces
                    .get_mut(&panel.id)
                    .expect("a panel that stands has its faces");
                // Drawn far, as only a panel in a slot is, it is a slot's size, and scrolls not at all.
                if faces.far {
                    if limits {
                        faces.limit = Some(0);
                    }
                    let (columns, rows) = self::fit(panel, *fit);
                    return sized(columns, rows);
                }
                let layout = faces.at(*fit);
                let (size, most) = (layout.size, layout.most_scroll);
                if limits {
                    faces.limit = Some(most);
                }
                // A list laid out again is scrolled to the row that stood at the top of its face.
                if let Some(scroll) = faces.anchored(*fit, most) {
                    self.scrolls.insert(panel.id.clone(), scroll);
                }
                size
            })
            .collect()
    }

    /// Follow what each of `projection`'s panels shows, laying out again those whose content changed, and forget the
    /// faces of panels, and how far they are scrolled, no longer projected, and how far every panel scrolls, until it
    /// is known again as it stands this frame. Paced, at most [`faces::LAYOUTS`] of the panels the server changed
    /// take their turn this frame, laid out or drawn again, those drawn far following theirs without one, and at most
    /// [`faces::ARRIVALS`] of those it sent that show nothing yet are given their faces, so that they stand, the
    /// nearest `viewer` first; the rest wait for the frames after, each in turn from where this frame's ended, in the
    /// projection's order, so that none waits behind the same ones every frame.
    fn lay_out(&mut self, projection: &Projection, viewer: Vec3) {
        let shown: HashSet<&str> = projection
            .panels
            .iter()
            .map(|panel| panel.id.as_str())
            .collect();
        self.faces.retain(|id, faces| {
            faces.limit = None;
            shown.contains(id.as_str())
        });
        self.scrolls.retain(|id, _| shown.contains(id.as_str()));
        let count = projection.panels.len();
        let (mut turns, arrivals) = if self.paced {
            (faces::LAYOUTS, faces::ARRIVALS)
        } else {
            (count, count)
        };
        let from = self.turn;
        let order = (0..count).map(|offset| (from + offset) % count);
        // Of those that arrive, those nearest `viewer` stand first, the nearest wall's or the nearest floating, each
        // in turn among those as near.
        let mut arriving: Vec<(f32, usize)> = Vec::new();
        if self.faces.len() < count {
            arriving.extend(
                order
                    .clone()
                    .filter(|index| !self.faces.contains_key(&projection.panels[*index].id))
                    .map(|index| (0.0, index)),
            );
        }
        if self.paced && arriving.len() > arrivals {
            let walls: HashMap<&str, Vec3> = projection
                .walls
                .iter()
                .map(|wall| (wall.id.as_str(), Vec3::from(wall.pose.position)))
                .collect();
            for (distance, index) in &mut arriving {
                let at = match &projection.panels[*index].place {
                    Place::Wall { wall, .. } => walls.get(wall.as_str()).copied(),
                    Place::Free { pose } => Some(Vec3::from(pose.position)),
                    _ => None,
                };
                *distance = at.map_or(f32::INFINITY, |at| at.distance(viewer));
            }
        }
        arriving.sort_by(|a, b| a.0.total_cmp(&b.0));
        let arrive: HashSet<usize> = arriving
            .into_iter()
            .take(arrivals)
            .map(|(_, index)| index)
            .collect();
        for index in order {
            let panel = &projection.panels[index];
            let scroll = self.scrolls.get(&panel.id).copied().unwrap_or_default();
            let took = match self.faces.get_mut(&panel.id) {
                Some(faces) => {
                    let took = faces.follow(panel, turns > 0, scroll);
                    turns -= usize::from(took);
                    took
                }
                None if arrive.contains(&index) => {
                    self.faces
                        .insert(panel.id.clone(), faces::Faces::new(panel));
                    true
                }
                None => false,
            };
            if took {
                self.turn = index + 1;
            }
        }
    }

    /// Which of `standing`'s panels a paced projector draws far, as a row of each one's text subtends from `viewer`
    /// (see [`faces::FAR`]), at its size on its wall, its wall's text scale and its font's pixel included: only one in
    /// a wall's slot, and never a volume, whose diagram stands before its face, nor one carried or let go, nor any with
    /// [`PanelLod::Off`]; at most [`faces::NEARING`] drawn whole again in a frame, the nearest first, but all at once as
    /// it is turned off.
    fn distances(&mut self, standing: &[Standing], viewer: Vec3) {
        if !self.paced {
            return;
        }
        let moved: Vec<String> = Side::BOTH
            .into_iter()
            .flat_map(|side| {
                [
                    self.held_in(side).map(str::to_owned),
                    self.pending[side]
                        .as_ref()
                        .map(|pending| pending.panel.clone()),
                ]
            })
            .flatten()
            .collect();
        // Those that may come near, drawn far and now near enough, wait for one of a few a frame, the nearest first;
        // every other changes at once, if at all: drawn far as it goes far, or whole as it may not be drawn far, as a
        // panel taken hold of is laid out in its frame.
        let mut nearing: Vec<(f32, &Panel)> = Vec::new();
        for (panel, model, fit) in standing {
            let far = self.lod == PanelLod::Angle
                && matches!(fit, Fit::Slot(_))
                && panel.diagram.is_none()
                && !moved.contains(&panel.id);
            let pixel = if panel.code.is_some() {
                CODE_PIXEL
            } else {
                PIXEL
            };
            let row = CELL_HEIGHT as f32 * pixel * model.y_axis.truncate().length();
            let distance = model.w_axis.truncate().distance(viewer);
            let (angle, mut whole) = if far {
                (row / distance.max(f32::EPSILON), 0)
            } else {
                (f32::INFINITY, usize::MAX)
            };
            let faces = self.faces_for(panel);
            if far && faces.far && angle > faces::NEAR {
                nearing.push((angle, *panel));
            } else {
                faces.distance(angle, &mut whole);
            }
        }
        nearing.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut left = faces::NEARING;
        for (angle, panel) in nearing {
            self.faces_for(panel).distance(angle, &mut left);
        }
    }

    /// The readouts linked to the chart `read` points at, at the place across its axis it points at, over every other
    /// chart standing that shares its time, as [`Output::linked`] says, none carried: drawn from each face as it was
    /// last drawn, without laying anything out, each handed over again while it shows the same.
    fn linked(&mut self, standing: &[Standing], read: Option<(&Panel, f32)>) -> Vec<Sheet> {
        let mut kept = std::mem::take(&mut self.linked);
        let Some((pointed, time)) = read.and_then(|(panel, at)| {
            panel
                .chart
                .as_ref()
                .and_then(|chart| chart.time_of(at))
                .map(|time| (panel, time))
        }) else {
            return Vec::new();
        };
        let wall = |panel: &Panel| match &panel.place {
            Place::Wall { wall, .. } => Some(wall.clone()),
            _ => None,
        };
        let carried: Vec<String> = Side::BOTH
            .into_iter()
            .filter_map(|side| self.held_in(side).map(str::to_owned))
            .collect();
        let here = wall(pointed).filter(|_| !carried.contains(&pointed.id));
        let mut sheets = Vec::new();
        for (panel, model, fit) in standing {
            if sheets.len() == LINKED {
                break;
            }
            let shares = panel.id != pointed.id
                && !carried.contains(&panel.id)
                && (matches!(time, ChartTime::Instant { .. })
                    || (here.is_some() && wall(panel) == here));
            let Some((face, readout)) = panel
                .chart
                .as_ref()
                .filter(|_| shares)
                .and_then(|chart| chart.at_time(time))
                .and_then(|at| self.faces_for(panel).linked(*fit, at))
            else {
                continue;
            };
            let drawn = match kept.remove(&panel.id) {
                Some((kept_face, kept_readout, canvas))
                    if Arc::ptr_eq(&kept_face, &face) && kept_readout == readout =>
                {
                    (kept_face, kept_readout, canvas)
                }
                _ => {
                    let canvas = crate::frame_timing::drawn(readout.overlay(&face));
                    (face, readout, canvas)
                }
            };
            sheets.push(Sheet {
                canvas: Some(drawn.2.clone()),
                model: Some(*model * Mat4::from_translation(Vec3::Z * READOUT_LIFT)),
            });
            self.linked.insert(panel.id.clone(), drawn);
        }
        sheets
    }

    /// Keep what each of `projection`'s volumes draws, building it again where its diagram changed, and forget what
    /// volumes no longer shown drew, and the focus on one.
    fn keep_meshes(&mut self, projection: &Projection) {
        self.meshes.retain(|id, _| {
            projection
                .panel(id)
                .is_some_and(|panel| panel.diagram.is_some())
        });
        for panel in &projection.panels {
            if let Some(diagram) = &panel.diagram
                && self
                    .meshes
                    .get(&panel.id)
                    .is_none_or(|mesh| mesh.source() != &**diagram)
            {
                self.meshes
                    .insert(panel.id.clone(), Arc::new(volume::Mesh::new(diagram)));
            }
        }
        if let Some((panel, node)) = &self.focus
            && self
                .meshes
                .get(panel)
                .is_none_or(|mesh| *node >= mesh.diagram().nodes.len())
        {
            self.focus = None;
        }
    }

    /// The node or edge of `panel`, a volume standing at `model` sized as `fit` says, that `ray` meets first, and how
    /// far along it; `None` for a panel that is not a volume, or where the ray meets neither.
    fn meet(&self, panel: &Panel, model: Mat4, fit: Fit, ray: &Ray) -> Option<(volume::Met, f32)> {
        // Every volume shown has its mesh, kept as the projection is followed, and no other panel has one.
        let mesh = self.meshes.get(&panel.id)?;
        mesh.hit(model * volume::placed(panel, mesh.source(), fit), ray)
    }

    /// What `met` of the volume `panel`, which the ray met, is named: its node by its id, or its edge by its index.
    fn named(&self, panel: &Panel, met: volume::Met) -> Part {
        match met {
            volume::Met::Node(node) => {
                Part::Node(self.meshes[&panel.id].diagram().nodes[node].id.clone())
            }
            // At most `diagram::EDGES` of them, which a u16 holds.
            volume::Met::Edge(edge) => Part::Edge(edge as u16),
            // A leader is drawn for each world edge to a thing here, in order, which names it by its own index.
            volume::Met::Leader(leader) => {
                Part::WorldEdge(self.meshes[&panel.id].world_edge(leader))
            }
        }
    }

    /// The node this participant focuses on, by its volume's identity and its index, if any.
    pub fn focus(&self) -> Option<(&str, usize)> {
        self.focus
            .as_ref()
            .map(|(panel, node)| (panel.as_str(), *node))
    }

    /// The faces of `panel`, which stands, and so has them.
    fn faces_for(&mut self, panel: &Panel) -> &mut faces::Faces {
        self.faces
            .get_mut(&panel.id)
            .expect("a panel that stands has its faces")
    }

    /// The wall taken hold of, whose handles show, by identity.
    pub fn editing_wall(&self) -> Option<&str> {
        self.editor.editing()
    }

    /// The wall being carried, by identity.
    pub fn carried_wall(&self) -> Option<&str> {
        self.editor.carrying()
    }

    /// Whether a panel or a wall is carried, so that the right stick, its click and B are its.
    pub fn carrying(&self) -> bool {
        self.held.any(|_, held| held.is_some()) || self.editor.carrying().is_some()
    }

    /// The server refused the change sent to the wall `wall`, as made against a version since changed: it is drawn as
    /// the server has it at once. See [`editing::Editor::refused`].
    pub fn refused_wall(&mut self, wall: &str) {
        self.editor.refused(wall);
    }

    /// Let go of the wall taken hold of, as B does when nothing is carried. Returns whether one was held.
    pub fn stop_editing(&mut self) -> bool {
        self.editor.stop()
    }

    /// B pressed while carrying a wall at `now`: the wall to remove, or `None` while it waits for a second press to
    /// confirm removing one that holds panels, which then float.
    pub fn remove_wall(&mut self, projection: &Projection, now: Instant) -> Option<String> {
        let wall = self.editor.carrying()?.to_owned();
        let held = projection
            .panels
            .iter()
            .filter(|panel| matches!(&panel.place, Place::Wall { wall: on, .. } if *on == wall))
            .count();
        self.editor.remove(held, now)
    }
}

/// The newest panel on each wall of `projection`, by age, and of those projected together, the last the server lists,
/// as it lists them in the order it put them.
fn newest(projection: &Projection) -> HashMap<&str, &str> {
    let mut newest: HashMap<&str, (&str, u64)> = HashMap::new();
    for panel in &projection.panels {
        if let Place::Wall { wall, .. } = &panel.place {
            let entry = newest
                .entry(wall.as_str())
                .or_insert((&panel.id, panel.age));
            if panel.age <= entry.1 {
                *entry = (&panel.id, panel.age);
            }
        }
    }
    newest
        .into_iter()
        .map(|(wall, (panel, _))| (wall, panel))
        .collect()
}

mod boards;
mod chart;
pub mod chips;
mod code;
pub mod editing;
mod faces;
mod image;
mod labels;
mod list;
pub mod volume;
pub(crate) use list::RowMarks;

pub use code::CodeLayout;
use code::code_face;
#[cfg(test)]
use code::{indent_step, wrap_code};
pub(crate) use labels::KeptLabel;
use labels::hint_sheet;
pub use labels::{Controls, ItemAction, Release, hint, label, let_go_hint};
pub mod travel;
pub mod two_hands;
mod walls;

#[cfg(test)]
mod tests;
