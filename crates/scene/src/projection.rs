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
use std::collections::HashMap;
use std::sync::Arc;
use tomori_protocol::{
    CODE_LINES, Code, Panel, Place, Pose, Projection, TokenKind, WALL_SLOTS, Wall, WallKind,
    WallShape,
};

use crate::{
    arrival::HOME,
    board::{Grip, model, pose},
    canvas::{Canvas, Rect},
    controller_ui::Sheet,
    manipulation::Manipulation,
    pointing::Ray,
    recording::facing,
    transcript::{BRIGHT, DIM, TEXT},
    ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Style, cut},
};

/// A panel's face in a wall's slot, in cells: a title, a row of metadata, a rule, and its body.
pub const COLUMNS: usize = 34;
pub const BODY_ROWS: usize = 12;
const ROWS: usize = BODY_ROWS + 3;
/// A floating panel's face fits its text, within these bounds, in cells: never narrower or wider, so that it reads at a
/// comfortable width, nor shorter or taller.
pub const FIT_COLUMNS: (usize, usize) = (24, 48);
pub const FIT_ROWS: (usize, usize) = (3, 30);
/// How a curved wall's slots are sized, against a flat wall's: smaller, as they stand nearer, and a gap between them, in
/// metres.
pub const ARC_SCALE: f32 = 0.7;
const ARC_GAP: f32 = 0.03;
/// The furthest along the ray a slot is aimed at to put a panel carried in it, in metres.
const AIM_REACH: f32 = 8.0;
/// How far beyond a slot's edges the ray still aims at it, so that the gaps between slots aim at one, in metres.
const AIM_MARGIN: f32 = 0.03;
/// The size of a font pixel on a wall, in metres: a panel is about 60 by 36 cm, its text legible from about 3 m.
pub const PIXEL: f32 = 0.0019;
const PADDING: (usize, usize) = (6, 5);

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

/// A wall's slots: [`WALL_SLOTS`] in columns of this many, the first at the top left as seen from in front.
const SLOT_COLUMNS: u32 = 3;
/// A wall's face, in cells, and its padding, which make it three panels wide and two high, with [`GAP`] between and
/// around them and its label above them.
const WALL_COLUMNS: usize = 116;
const WALL_ROWS: usize = 38;
const WALL_PADDING: (usize, usize) = (3, 3);
/// Font pixels between slots and around them, and between a wall's label and its first row of slots.
const GAP: usize = 24;
const LABEL_GAP: usize = 12;
/// How far in front of its wall a panel stands, so that it is drawn over it.
const LIFT: f32 = 0.01;

/// How far from a wall's plane, and beyond its edges, a panel let go snaps back into its layout, in metres.
const SNAP_DEPTH: f32 = 0.4;
const SNAP_MARGIN: f32 = 0.25;

/// How long a panel let go is shown where it was let go, while the server's projection does not yet say so.
pub const PENDING: Duration = Duration::from_secs(3);

/// How far ahead of the participant's head, in metres, a wall they add stands, and how far below their eyes its middle
/// is.
pub const WALL_DISTANCE: f32 = 1.8;
const WALL_DROP: f32 = 0.1;

/// The label on the ray that names a panel pointed at.
const LABEL_COLUMNS: usize = 32;
const LABEL_PIXEL: f32 = 0.0017;

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
const HIGHLIGHT: [f32; 4] = [0.35, 0.95, 1.0, 1.0];
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
}

/// How a panel's face is sized: to its wall's slot, or, floating, to fit its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Slot,
    Text,
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
fn body_rows(panel: &Panel, columns: usize) -> Vec<Vec<crate::ui_text::Cell>> {
    crate::transcript::parse_markdown(&panel.body, columns)
}

/// The line of `panel`'s body, from 0, that the list item `point` is on begins on, where the ray meets a text panel drawn
/// with `model` and sized as `fit` and its body scrolled `scroll` rows down: found from the body row under the point, as its face lays the body out, so that what
/// the participant points at as they speak names the item, such as one of a checklist. `None` off its body, on a row of
/// no list item, and on a code panel.
pub fn item_at(panel: &Panel, model: Mat4, fit: Fit, scroll: usize, point: Vec3) -> Option<u32> {
    if panel.code.is_some() {
        return None;
    }
    let (columns, body_count) = self::fit(panel, fit);
    let (_, height) = sized(columns, body_count);
    let local = model.inverse().transform_point3(point);
    let from_top = (height / 2.0 - local.y) / PIXEL - PADDING.1 as f32;
    if from_top.is_nan() || from_top < 0.0 {
        return None;
    }
    // Below the title, the metadata and the rule.
    let row = (from_top / CELL_HEIGHT as f32) as usize;
    let body = row.checked_sub(3).filter(|body| *body < body_count)?;
    let (_, line) = crate::transcript::markdown_lines(&panel.body, columns)
        .into_iter()
        .nth(scroll + body)?;
    line.and_then(|line| u32::try_from(line).ok())
}

/// How many columns and body rows a text panel's face has when it is sized as `fit` says: a slot's, or, to fit its
/// text, as wide as its widest row, title and metadata, within [`FIT_COLUMNS`], its body wrapped at that width, as many
/// rows as that takes, within [`FIT_ROWS`]. A code panel's columns are always a slot's, and its rows of code are its
/// [`CodeLayout`]'s.
pub fn fit(panel: &Panel, fit: Fit) -> (usize, usize) {
    if let Some(image) = &panel.image {
        return image::fit(panel, image, fit);
    }
    match fit {
        Fit::Slot => (COLUMNS, BODY_ROWS),
        Fit::Text => {
            let widest = body_rows(panel, FIT_COLUMNS.1)
                .iter()
                .map(Vec::len)
                .chain([heading_width(panel)])
                .max()
                .unwrap_or_default();
            let columns = widest.clamp(FIT_COLUMNS.0, FIT_COLUMNS.1);
            let rows = body_rows(panel, columns)
                .len()
                .clamp(FIT_ROWS.0, FIT_ROWS.1);
            (columns, rows)
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
    let layout = panel.code.as_ref().map(|code| CodeLayout::new(panel, code));
    size_of(panel, fit, layout.as_ref())
}

/// The size of `panel`, in metres, sized as `fit` says, its code laid out as `layout`.
fn size_of(panel: &Panel, fit: Fit, layout: Option<&CodeLayout>) -> (f32, f32) {
    match (layout, fit) {
        (Some(layout), Fit::Text) => {
            let width = CODE_COLUMNS * CELL_WIDTH + 2 * CODE_PADDING.0;
            let height = layout.face_rows(fit) * CELL_HEIGHT + 2 * CODE_PADDING.1;
            (width as f32 * CODE_PIXEL, height as f32 * CODE_PIXEL)
        }
        _ => {
            let (columns, body) = self::fit(panel, fit);
            sized(columns, body)
        }
    }
}

/// How many rows down `panel`'s body scrolls at most, sized as `fit` says, its code laid out as `layout`: as many as it
/// has that its face does not show at once.
fn most_scroll(panel: &Panel, fit: Fit, layout: Option<&CodeLayout>) -> usize {
    match layout {
        Some(layout) => layout.most_scroll(fit),
        None => {
            let (columns, count) = self::fit(panel, fit);
            body_rows(panel, columns).len().saturating_sub(count)
        }
    }
}

/// The size of a wall, in metres.
pub fn wall_size() -> (f32, f32) {
    let width = WALL_COLUMNS * CELL_WIDTH + 2 * WALL_PADDING.0;
    let height = WALL_ROWS * CELL_HEIGHT + 2 * WALL_PADDING.1;
    (width as f32 * PIXEL, height as f32 * PIXEL)
}

/// Where `slot` is on its wall, in font pixels from the wall's top left corner: its left and top edges.
fn slot_pixels(slot: u32) -> (usize, usize) {
    let (width, height) = (
        COLUMNS * CELL_WIDTH + 2 * PADDING.0,
        ROWS * CELL_HEIGHT + 2 * PADDING.1,
    );
    let (column, row) = (
        (slot % SLOT_COLUMNS) as usize,
        (slot / SLOT_COLUMNS) as usize,
    );
    (
        GAP + column * (width + GAP),
        WALL_PADDING.1 + CELL_HEIGHT + LABEL_GAP + row * (height + GAP),
    )
}

/// The middle of `slot`, in its wall's frame, just in front of it.
pub fn slot_offset(slot: u32) -> Vec3 {
    let (left, top) = slot_pixels(slot);
    let (wall_width, wall_height) = wall_size();
    let (width, height) = panel_size();
    Vec3::new(
        left as f32 * PIXEL + width / 2.0 - wall_width / 2.0,
        wall_height / 2.0 - top as f32 * PIXEL - height / 2.0,
        LIFT,
    )
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
    let layout = panel.code.as_ref().map(|code| CodeLayout::new(panel, code));
    scrolled_face(panel, highlighted, newest, fit, 0, layout.as_ref())
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
    let border = Some(if highlighted {
        HIGHLIGHT
    } else if newest {
        NEWEST
    } else {
        FRAME
    });
    if let Some(layout) = layout {
        let mut canvas = code_face(panel, layout, fit, scroll);
        canvas.border = border;
        return canvas;
    }
    if let Some(image) = &panel.image {
        return image::face(panel, image, fit, border);
    }
    let (columns, body_count) = self::fit(panel, fit);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = border;
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), columns),
        Row::text(&about(panel), Style::plain(DIM), columns),
        Row::text(&"─".repeat(columns), Style::plain(DIM), columns),
    ];
    let mut body = body_rows(panel, columns);
    let total = body.len();
    let scroll = scroll.min(total.saturating_sub(body_count));
    body.drain(..scroll);
    if body.len() > body_count {
        body.truncate(body_count);
        let last = &mut body[body_count - 1];
        last.truncate(columns - 1);
        last.push(('…', Style::plain(TEXT)));
    }
    body.resize(body_count, Vec::new());
    rows.extend(body.into_iter().map(Row::new));
    canvas.rows = rows;
    canvas.shapes = scroll_bar(columns, body_count, total, scroll);
    canvas
}

/// The slim bar at the right edge of a body of `total` rows, `shown` of them at once in a face `columns` wide, scrolled
/// `scroll` rows down: a faint track as tall as the rows shown, and on it a brighter thumb, as tall and as far down as
/// the rows shown are of them all. None where every row shows.
fn scroll_bar(columns: usize, shown: usize, total: usize, scroll: usize) -> Vec<Rect> {
    if total <= shown {
        return Vec::new();
    }
    let x = (columns * CELL_WIDTH) as f32 + SCROLL_BAR.0;
    let top = (3 * CELL_HEIGHT) as f32;
    let track = (shown * CELL_HEIGHT) as f32;
    let height = (track * shown as f32 / total as f32).max(SCROLL_BAR.2);
    let y = (top + track * scroll as f32 / total as f32).min(top + track - height);
    vec![
        Rect {
            x,
            y: top,
            width: SCROLL_BAR.1,
            height: track,
            color: SCROLL_TRACK,
        },
        Rect {
            x,
            y,
            width: SCROLL_BAR.1,
            height,
            color: SCROLL_THUMB,
        },
    ]
}

/// One row of a code panel's code, as wrapped: the line it is of, from 0, whether it is that line's first, and its
/// cells after the gutter.
#[derive(Clone, Debug, PartialEq)]
struct CodeRow {
    line: usize,
    first: bool,
    cells: Vec<crate::ui_text::Cell>,
}

/// A code panel's code laid out: every row its lines take, each wrapped as [`wrap_code`] wraps it at its columns of code,
/// coloured by kind as [`palette`] colours them, and how many of its block's lines it holds and was not sent, so that
/// its face shows any part of it. Spans that are not valid for its body, as `valid_spans` checks them, are ignored, and
/// the code laid out plain. Laid out once for each panel's code, rather than each frame, as a panel holds up to
/// [`CODE_LINES`] lines.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeLayout {
    rows: Vec<CodeRow>,
    lines: usize,
    unsent: usize,
}

impl CodeLayout {
    /// The layout of `panel`'s code, `code`.
    pub fn new(panel: &Panel, code: &Code) -> Self {
        // The kind of each character of the body, in order.
        let mut kinds = vec![TokenKind::Plain; panel.body.chars().count()];
        if tomori_protocol::valid_spans(&panel.body, &code.spans) {
            for span in &code.spans {
                kinds[span.start as usize..span.end() as usize].fill(span.kind);
            }
        }
        let mut at = 0;
        let lines: Vec<&str> = panel.body.split('\n').take(CODE_LINES).collect();
        let step = indent_step(&lines);
        let room = CODE_COLUMNS - GUTTER;
        let mut rows = Vec::new();
        for (number, line) in lines.iter().enumerate() {
            let mut spans: Vec<crate::ui_text::Span> = Vec::new();
            for (index, c) in line.chars().enumerate() {
                let style = Style::plain(palette(kinds[at + index]));
                // Joined to the run before, of the same kind.
                match spans.last_mut() {
                    Some(last) if last.style == style => last.text.push(c),
                    _ => spans.push(crate::ui_text::Span::new(c.to_string(), style)),
                }
            }
            at += line.chars().count() + 1;
            let mut wrapped = wrap_code(&cut(&spans, usize::MAX), room, step);
            // A line cut by the server ends with a dim "…": past what the panel holds, or, from an older server, at 64
            // characters.
            if let Some(last) = wrapped
                .last_mut()
                .and_then(|row| row.last_mut())
                .filter(|(c, _)| *c == '…')
            {
                last.1 = Style::plain(DIM);
            }
            rows.extend(
                wrapped
                    .into_iter()
                    .enumerate()
                    .map(|(index, cells)| CodeRow {
                        line: number,
                        first: index == 0,
                        cells,
                    }),
            );
        }
        Self {
            rows,
            lines: lines.len(),
            unsent: (code.lines as usize).saturating_sub(lines.len()),
        }
    }

    /// How many rows of code its face has, sized as `fit` says: a slot's, or, floating, as many as its code takes, within
    /// [`FIT_CODE_ROWS`].
    fn height(&self, fit: Fit) -> usize {
        match fit {
            Fit::Slot => SLOT_CODE_ROWS,
            Fit::Text => self.rows.len().clamp(FIT_CODE_ROWS.0, FIT_CODE_ROWS.1),
        }
    }

    /// Whether its face, sized as `fit` says, has a row below its code saying what there is above and below it: always
    /// in a slot, and floating, only where it does not show all of its block.
    fn footed(&self, fit: Fit) -> bool {
        fit == Fit::Slot || self.rows.len() > self.height(fit) || self.unsent > 0
    }

    /// How many rows its face has, sized as `fit` says: its title, metadata and rule, its rows of code, and the row
    /// below them, where it has one.
    fn face_rows(&self, fit: Fit) -> usize {
        3 + self.height(fit) + usize::from(self.footed(fit))
    }

    /// How many rows down it scrolls at most, sized as `fit` says: so far that its last row is its face's last.
    fn most_scroll(&self, fit: Fit) -> usize {
        self.rows.len().saturating_sub(self.height(fit))
    }
}

/// A code panel's face, its code laid out as `layout`, sized as `fit` says and scrolled `scroll` rows down, as far as it
/// goes: its title, bold, its language, length and age, a rule, and its rows of code from there, each line's first after
/// its number and its later rows marked as continued in the gutter, then a row saying how many lines are above and below
/// them, and how many are only in the transcript. A line that does not fit whole below the first shown is left for the
/// row below, and the first, were it alone too tall for what is left, ends with a dim "…".
fn code_face(panel: &Panel, layout: &CodeLayout, fit: Fit, scroll: usize) -> Canvas {
    let height = layout.height(fit);
    let scroll = scroll.min(layout.most_scroll(fit));
    let room = CODE_COLUMNS - GUTTER;
    let mut canvas = Canvas::new(CODE_COLUMNS, CODE_PIXEL);
    canvas.padding = CODE_PADDING;
    canvas.background = [0.015, 0.025, 0.045, 0.95];
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), CODE_COLUMNS),
        Row::text(&about(panel), Style::plain(DIM), CODE_COLUMNS),
        Row::text(&"─".repeat(CODE_COLUMNS), Style::plain(DIM), CODE_COLUMNS),
    ];
    let first = layout.rows.get(scroll).map(|row| row.line);
    let mut at = scroll;
    let mut shown: Vec<&CodeRow> = Vec::new();
    while let Some(row) = layout.rows.get(at)
        && shown.len() < height
    {
        // A later line that does not fit whole is left for the row below, which counts it.
        if row.first && Some(row.line) != first {
            let tall = layout.rows[at..]
                .iter()
                .take_while(|next| next.line == row.line)
                .count();
            if tall > height - shown.len() {
                break;
            }
        }
        shown.push(row);
        at += 1;
    }
    // The first line shown, cut by the face's end, ends with a dim "…".
    let cut_short = layout.rows.get(at).is_some_and(|row| !row.first);
    for (index, row) in shown.iter().enumerate() {
        // The line's number on its first row only; its later rows are marked as continued.
        let gutter = match row.first {
            true => crate::ui_text::Span::new(
                format!("{:>width$} ", row.line + 1, width = GUTTER - 1),
                Style::plain(DIM),
            ),
            false => crate::ui_text::Span::new(
                format!("{CONTINUED:>width$} ", width = GUTTER - 1),
                Style::plain(CONTINUED_COLOR),
            ),
        };
        let mut cells = row.cells.clone();
        if cut_short && index == shown.len() - 1 {
            let mut last = vec![cells, Vec::new()];
            crate::ui_text::fit(&mut last, 1, room);
            cells = last.remove(0);
            // `fit` ends it with "…", the last cell.
            let end = cells.len() - 1;
            cells[end].1 = Style::plain(DIM);
        }
        let mut drawn = cut(&[gutter], GUTTER);
        drawn.extend(cells);
        rows.push(Row::new(drawn));
    }
    rows.resize(3 + height, Row::default());
    if layout.footed(fit) {
        rows.push(Row::text(
            &around(layout, scroll, at),
            Style::plain(DIM),
            CODE_COLUMNS,
        ));
    }
    canvas.rows = rows;
    canvas.shapes = scroll_bar(CODE_COLUMNS, height, layout.rows.len(), scroll);
    canvas
}

/// What the row below a code panel's code says of `layout`, its rows shown from `scroll` until `end`: how many lines
/// start above them, as `↑ 3 above`; how many are not shown whole below them, those only in the transcript among them,
/// as `↓ 4 more`; or, below the last it holds, how many it was not sent, as `180 more lines in the transcript`.
fn around(layout: &CodeLayout, scroll: usize, end: usize) -> String {
    let above = layout
        .rows
        .get(scroll)
        .filter(|_| scroll > 0)
        .map_or(0, |row| row.line + usize::from(!row.first));
    let below = layout
        .rows
        .get(end)
        .map_or(0, |row| layout.lines - row.line);
    let mut parts = Vec::new();
    if above > 0 {
        parts.push(format!("↑ {above} above"));
    }
    match (below, layout.unsent) {
        (0, 0) => {}
        (0, 1) => parts.push("1 more line in the transcript".to_owned()),
        (0, unsent) => parts.push(format!("{unsent} more lines in the transcript")),
        (below, unsent) => parts.push(format!("↓ {} more", below + unsent)),
    }
    parts.join(" · ")
}

/// The indent step of code's `lines`: the least indentation any line has, of two to four spaces, or four where none is
/// indented.
fn indent_step(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|c| *c == ' ').count())
        .filter(|indent| *indent > 0)
        .min()
        .map_or(4, |step| step.clamp(2, 4))
}

/// Whether a line of code may break between `before` and `after`: after a space, or after punctuation that ends or
/// opens a part of it, such as a comma, an operator or an opening bracket, unless a closing one follows.
fn code_breaks(before: char, after: char) -> bool {
    before == ' '
        || (",;([{.:=&|+-*/<>!?".contains(before) && !")]}>,;.:".contains(after) && after != ' ')
}

/// One line of code's `cells`, as drawn, on rows of `room` cells: as it is where it fits, and otherwise broken where
/// [`code_breaks`] allows, in the latter half of the row, or else at the row's end, never through a wide character.
/// Its later rows hang one indent `step` under its own indentation, keeping at least [`CODE_ROOM`] cells for its code,
/// and the spaces at each break are dropped. Each cell keeps its style, so that highlighting carries across a break.
fn wrap_code(
    cells: &[crate::ui_text::Cell],
    room: usize,
    step: usize,
) -> Vec<Vec<crate::ui_text::Cell>> {
    use crate::ui_text::WIDE;
    let indent = cells.iter().take_while(|(c, _)| *c == ' ').count();
    let hanging = (indent + step).min(room.saturating_sub(CODE_ROOM));
    let mut rows = Vec::new();
    let mut rest = cells;
    let mut width = room;
    loop {
        let prefix = if rows.is_empty() { 0 } else { hanging };
        if rest.len() <= width {
            rows.push(blank(prefix).chain(rest.iter().copied()).collect());
            return rows;
        }
        // The last break the line allows in the latter half of the row, after its indentation, or else the row's end,
        // before a wide character that would not fit.
        let first = (width / 2).max(if rows.is_empty() { indent + 1 } else { 1 });
        let at = (first..=width)
            .rev()
            .find(|&at| rest[at].0 != WIDE && code_breaks(rest[at - 1].0, rest[at].0))
            .unwrap_or(if rest[width].0 == WIDE {
                width - 1
            } else {
                width
            });
        let mut row: Vec<_> = blank(prefix).chain(rest[..at].iter().copied()).collect();
        while row.len() > prefix && row.last().is_some_and(|(c, _)| *c == ' ') {
            row.pop();
        }
        rows.push(row);
        rest = &rest[at..];
        let spaces = rest.iter().take_while(|(c, _)| *c == ' ').count();
        rest = &rest[spaces..];
        if rest.is_empty() {
            return rows;
        }
        width = room - hanging;
    }
}

/// `count` plain spaces.
fn blank(count: usize) -> impl Iterator<Item = crate::ui_text::Cell> {
    std::iter::repeat_n((' ', Style::plain(TEXT)), count)
}

/// The colour of a wall's frame, and its slots' on a curved wall.
fn wall_border(wall: &Wall) -> [f32; 4] {
    match wall.kind {
        WallKind::Tomori => [0.35, 0.75, 0.95, 0.55],
        _ => [0.5, 0.55, 0.62, 0.45],
    }
}

/// A wall's own face: a translucent board labelled with whose it is, its slots faintly marked.
fn wall_face(wall: &Wall) -> Canvas {
    let mut canvas = Canvas::new(WALL_COLUMNS, PIXEL);
    canvas.padding = WALL_PADDING;
    canvas.background = [0.01, 0.02, 0.04, 0.5];
    let (label, color) = wall_label(wall);
    let border = wall_border(wall);
    canvas.border = Some(border);
    let label = Row::text(label, Style::bold(color), WALL_COLUMNS);
    canvas.rows = vec![Row::new(
        std::iter::repeat_n((' ', Style::plain(color)), GAP / CELL_WIDTH)
            .chain(label.cells)
            .collect(),
    )];
    canvas.rows.resize(WALL_ROWS, Row::default());
    let (width, height) = (
        COLUMNS * CELL_WIDTH + 2 * PADDING.0,
        ROWS * CELL_HEIGHT + 2 * PADDING.1,
    );
    canvas.shapes = (0..WALL_SLOTS)
        .map(|slot| {
            let (left, top) = slot_pixels(slot);
            Rect {
                x: (left - WALL_PADDING.0) as f32,
                y: (top - WALL_PADDING.1) as f32,
                width: width as f32,
                height: height as f32,
                color: SLOT,
            }
        })
        .collect();
    canvas
}

/// A slot's own board on a curved wall: as large as a panel in it, faintly marked.
fn slot_board(border: [f32; 4]) -> Canvas {
    let mut canvas = Canvas::new(COLUMNS, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.01, 0.02, 0.04, 0.45];
    canvas.border = Some(border);
    canvas.rows = vec![Row::default(); ROWS];
    canvas
}

/// The outline a panel carried shows in the slot it would take if let go: as large as a panel in it, framed brightly
/// and faintly filled.
fn ghost() -> Canvas {
    let mut canvas = slot_board(HIGHLIGHT);
    canvas.background = GHOST;
    canvas
}

/// A curved wall's label, below the middle of its row of slots.
fn arc_label(wall: &Wall) -> Canvas {
    let (label, color) = wall_label(wall);
    let mut canvas = Canvas::new(label.len(), PIXEL);
    canvas.padding = (CELL_WIDTH, 2);
    canvas.background = [0.01, 0.02, 0.04, 0.5];
    canvas.rows = vec![Row::text(label, Style::bold(color), label.len())];
    canvas
}

/// What a wall is labelled, and in which colour.
fn wall_label(wall: &Wall) -> (&'static str, [f32; 4]) {
    match wall.kind {
        WallKind::Tomori => ("TOMORI", [0.6, 0.85, 1.0, 1.0]),
        _ => ("WALL", DIM),
    }
}

/// The radius of a curved wall's arc, if `wall` is one this client can draw curved: one whose radius is long enough
/// for its slots to stand side by side. Any other shape is drawn flat.
fn arc(wall: &Wall) -> Option<f32> {
    match wall.shape {
        WallShape::Arc { radius } if radius.is_finite() && radius >= 1.0 => Some(radius),
        _ => None,
    }
}

/// Where each of a wall's slots stands, given its `frame`: the middle of a panel in it, facing out, scaled as the
/// panel in it is drawn. A flat wall's are in its plane, just in front of it; a curved wall's stand along its arc,
/// left to right as seen from in front, each turned to face the point the arc curves about.
pub fn slot_frames(wall: &Wall, frame: Mat4) -> Vec<Mat4> {
    let Some(radius) = arc(wall) else {
        return (0..WALL_SLOTS)
            .map(|slot| frame * Mat4::from_translation(slot_offset(slot)))
            .collect();
    };
    let pitch = (panel_size().0 * ARC_SCALE + ARC_GAP) / radius;
    (0..WALL_SLOTS)
        .map(|slot| {
            let angle = (slot as f32 - (WALL_SLOTS as f32 - 1.0) / 2.0) * pitch;
            let at = Vec3::new(radius * angle.sin(), 0.0, radius * (1.0 - angle.cos()));
            frame
                * Mat4::from_translation(at)
                * Mat4::from_rotation_y(-angle)
                * Mat4::from_translation(Vec3::Z * LIFT)
                * Mat4::from_scale(Vec3::splat(ARC_SCALE))
        })
        .collect()
}

/// The label on the ray naming `panel`, pointed at where the ray meets it, `at`, facing `viewer`: what it is called, its
/// metadata, and when and where it was projected; and, where it `scrolls`, how to scroll it with those controls.
pub fn label(panel: &Panel, at: Vec3, viewer: Vec3, scrolls: Option<Controls>) -> Sheet {
    let mut canvas = Canvas::new(LABEL_COLUMNS, LABEL_PIXEL);
    canvas.background = [0.0, 0.0, 0.0, 0.88];
    canvas.border = Some(HIGHLIGHT);
    canvas.rows = vec![Row::text(&name(panel), Style::bold(BRIGHT), LABEL_COLUMNS)];
    canvas.rows.extend(panel.meta.iter().map(|meta| {
        Row::new(cut(
            &[
                crate::ui_text::Span::new(format!("{}: ", meta.key), Style::plain(DIM)),
                crate::ui_text::Span::new(&meta.value, Style::plain(TEXT)),
            ],
            LABEL_COLUMNS,
        ))
    }));
    if let Some(code) = &panel.code {
        let held = held(panel);
        let mut about = vec![if code.lines as usize > held {
            format!("lines 1–{held} of {}", code.lines)
        } else {
            length(code)
        }];
        if !code.language.is_empty() {
            about.insert(0, code.language.clone());
        }
        if panel.pinned {
            about.push("pinned".to_owned());
        }
        canvas.rows.push(Row::text(
            &about.join(" · "),
            Style::plain(TEXT),
            LABEL_COLUMNS,
        ));
    }
    let place = match &panel.place {
        Place::Wall { wall, .. } if wall == tomori_protocol::TOMORI_WALL => "on the Tomori wall",
        Place::Wall { .. } => "on a wall",
        _ => "floating",
    };
    canvas.rows.push(Row::text(
        &format!("{} ago, {place}", crate::board::age(panel.age)),
        Style::plain(DIM),
        LABEL_COLUMNS,
    ));
    if let Some(controls) = scrolls {
        let control = match controls {
            Controls::Controllers => "Stick ↕",
            Controls::Keyboard => "Wheel",
        };
        canvas.rows.push(Row::new(cut(
            &[
                crate::ui_text::Span::new(format!("{control}  "), Style::bold(BRIGHT)),
                crate::ui_text::Span::new("scroll", Style::plain(TEXT)),
            ],
            LABEL_COLUMNS,
        )));
    }
    let towards = (viewer - at).try_normalize().unwrap_or(Vec3::Z);
    let (_, height) = canvas.size();
    Sheet {
        model: Some(facing(
            at + towards * 0.08 + Vec3::Y * (height / 2.0 + 0.03),
            viewer,
            1.0,
        )),
        canvas: Some(canvas),
    }
}

/// Which controls the participant has, which the hint beside a panel carried names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Controls {
    /// A headset's controllers.
    #[default]
    Controllers,
    /// The desktop window's keyboard and mouse, as `design/desktop.md` maps them.
    Keyboard,
}

/// What letting go of a panel carried would do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Put it in the empty slot outlined.
    Slot,
    /// Put it in the slot outlined, changing places with the panel there.
    Swap,
    /// Leave it floating where it is.
    Float,
}

/// The hint beside a panel carried: what each control does to it, for someone carrying one for the first time, in a
/// few short rows of the control and what it does, as `controls` name them, the last saying what letting go would do
/// now, `release`.
pub fn hint(controls: Controls, release: Release) -> Canvas {
    let let_go = match release {
        Release::Slot => "put it in the slot",
        Release::Swap => "swap with that one",
        Release::Float => "leave it floating",
    };
    let rows: [(&str, &str); 5] = match controls {
        Controls::Controllers => [
            ("Stick ↕", "further / closer"),
            ("Stick ↔", "turn"),
            ("Click", "square up"),
            ("B", "throw away, for all"),
            ("Let go", let_go),
        ],
        Controls::Keyboard => [
            ("PgUp PgDn", "further / closer"),
            ("[ ]", "turn"),
            ("R", "square up"),
            ("Backspace", "throw away, for all"),
            ("Let go G", let_go),
        ],
    };
    let keys = rows
        .iter()
        .map(|(control, _)| crate::ui_text::width(control))
        .max()
        .unwrap_or_default()
        + HINT_SPACE;
    let columns = keys + HINT_ACTIONS;
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(FRAME);
    canvas.rows = rows
        .iter()
        .enumerate()
        .map(|(index, (control, action))| {
            let (key, text) = match index {
                3 => (Style::bold(THROW), Style::plain(THROW)),
                _ => (Style::bold(BRIGHT), Style::plain(TEXT)),
            };
            let control = crate::ui_text::pad(control, keys);
            Row::new(cut(
                &[
                    crate::ui_text::Span::new(control, key),
                    crate::ui_text::Span::new(*action, text),
                ],
                columns,
            ))
        })
        .collect();
    canvas
}

/// The hint beside a panel carried with `model`, `size` large, seen from `viewer`: just below it, or above it where
/// below would be under the floor or far below the eyes, facing the viewer, drawn at [`HINT_ANGLE`] a font pixel
/// whatever its distance, so that it reads alike at arm's length and across the room.
fn hint_sheet(mut canvas: Canvas, model: Mat4, (_, height): (f32, f32), viewer: Vec3) -> Sheet {
    let centre = model.w_axis.truncate();
    let distance = centre.distance(viewer).max(0.1);
    canvas.pixel = HINT_ANGLE * distance;
    let (_, tall) = canvas.size();
    let offset = height / 2.0 + HINT_GAP * distance + tall / 2.0;
    let below = centre - Vec3::Y * offset;
    let bottom = below.y - tall / 2.0;
    let level = Vec2::new(below.x - viewer.x, below.z - viewer.z).length();
    let hidden = bottom < viewer.y - HINT_FLOOR || viewer.y - bottom > HINT_LOWEST * level;
    let at = if hidden {
        centre + Vec3::Y * offset
    } else {
        below
    };
    Sheet {
        model: Some(facing(at, viewer, 1.0)),
        canvas: Some(canvas),
    }
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

/// A slot a panel carried would be put in if let go: the wall, the slot, where it stands, and where the ray meets it,
/// when the ray chose it.
#[derive(Clone, Debug, PartialEq)]
struct Aim {
    wall: String,
    slot: u32,
    model: Mat4,
    point: Option<Vec3>,
}

/// The slot `ray` meets first, within [`AIM_REACH`] and [`AIM_MARGIN`] of its edges, in each slot's own plane, so that
/// a curved wall's slots are aimed at as they stand.
fn aim(walls: &[(&Wall, Mat4)], ray: &Ray) -> Option<Aim> {
    let margin = AIM_MARGIN;
    walls
        .iter()
        .flat_map(|(wall, frame)| {
            slot_frames(wall, *frame)
                .into_iter()
                .enumerate()
                .filter_map(move |(slot, model)| {
                    let scale = model.x_axis.truncate().length();
                    hit_within(model, panel_size(), margin / scale, ray)
                        .filter(|distance| *distance <= AIM_REACH)
                        .map(|distance| {
                            (
                                distance,
                                Aim {
                                    wall: wall.id.clone(),
                                    slot: slot as u32,
                                    model,
                                    point: Some(ray.at(distance)),
                                },
                            )
                        })
                })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, aim)| aim)
}

/// The slot a panel carried at `centre` snaps into, if it is near enough to one: within [`SNAP_DEPTH`] of its plane and
/// [`SNAP_MARGIN`] of its edges, the nearest such slot.
fn snap(walls: &[(&Wall, Mat4)], centre: Vec3) -> Option<Aim> {
    let (width, height) = panel_size();
    walls
        .iter()
        .flat_map(|(wall, frame)| {
            slot_frames(wall, *frame)
                .into_iter()
                .enumerate()
                .filter_map(move |(slot, model)| {
                    let scale = model.x_axis.truncate().length();
                    let local = model.inverse().transform_point3(centre) * scale;
                    (local.z.abs() <= SNAP_DEPTH
                        && local.x.abs() <= width * scale / 2.0 + SNAP_MARGIN
                        && local.y.abs() <= height * scale / 2.0 + SNAP_MARGIN)
                        .then(|| {
                            (
                                local.length(),
                                Aim {
                                    wall: wall.id.clone(),
                                    slot: slot as u32,
                                    model,
                                    point: None,
                                },
                            )
                        })
                })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, aim)| aim)
}

/// Whether `place`, where the server says a panel is, is where it was put, `target`.
fn agrees(place: &Place, target: &Target) -> bool {
    match (place, target) {
        (Place::Wall { wall, slot }, Target::Wall(target, chosen)) => {
            wall == target && slot == chosen
        }
        (Place::Free { pose }, Target::Free(target)) => {
            Vec3::from_array(pose.position).distance(Vec3::from_array(target.position)) < 0.01
        }
        _ => false,
    }
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
    /// Where it was last drawn, which it stays at while the ray is not tracked.
    model: Mat4,
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
    /// The right stick, while a panel carried takes it: forward and back push it further or pull it closer, and a flick
    /// sideways turns it a snap.
    pub stick: Vec2,
    /// The right stick was clicked while a panel was carried: it turns back to face the participant square.
    pub straighten: bool,
    /// The controls the hint beside a panel carried names.
    pub controls: Controls,
    /// A panel to scroll, this participant's own view of it, which nobody else sees: the one pointed at as the right
    /// stick was pushed or the mouse's wheel turned, and how many rows down, or up where fewer than none.
    pub scroll: Option<(&'a str, f32)>,
    pub now: Instant,
    /// Seconds since the last frame.
    pub dt: f32,
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
    /// The hint beside the panel carried, saying what each control does to it, drawn over everything else.
    pub hint: Option<Sheet>,
}

/// The panels across frames: the one carried, the slot it would be put in, and the one let go that the server has not
/// yet confirmed.
#[derive(Clone, Debug, Default)]
pub struct Projector {
    grip: Grip,
    held: Option<Held>,
    aim: Option<Aim>,
    pending: Option<Pending>,
    /// A panel thrown away, hidden from when it was until the server's projection no longer shows it, or [`PENDING`]
    /// passes.
    thrown: Option<(String, Instant)>,
    /// How far down each panel is scrolled, in rows, this participant's own, kept while it is projected: in a slot again
    /// after floating, it is as it was.
    scrolls: HashMap<String, f32>,
    /// Each code panel's code laid out, and the code it was laid out from, laid out again only when that changes.
    layouts: HashMap<String, Laid>,
    /// How far down each panel shown this frame scrolls at most, in rows, as it is sized.
    limits: HashMap<String, usize>,
}

/// A code panel's code laid out, and the body and code it was laid out from.
#[derive(Clone, Debug)]
struct Laid {
    body: String,
    code: Code,
    layout: Arc<CodeLayout>,
}

/// A panel as it stands this frame: where, how its face is sized, and how large it is.
type Standing<'a> = (&'a Panel, Mat4, Fit);

impl Projector {
    /// Follow `input`, and say what to draw and send.
    pub fn update(&mut self, input: Input<'_>) -> Output {
        // The grip is followed even while the panels do not take it, so that a squeeze held as they start to is not a new
        // one.
        let (pressed, released) = self.grip.update(input.grip);
        let Some(projection) = input.projection else {
            self.held = None;
            self.aim = None;
            self.pending = None;
            self.thrown = None;
            self.scrolls.clear();
            self.layouts.clear();
            self.limits.clear();
            return Output::default();
        };
        self.lay_out(projection);
        if let Some((panel, since)) = &self.thrown
            && (projection.panel(panel).is_none()
                || input.now.saturating_duration_since(*since) >= PENDING)
        {
            self.thrown = None;
        }
        if let Some(pending) = &self.pending {
            let confirmed = projection
                .panel(&pending.panel)
                .is_none_or(|panel| agrees(&panel.place, &pending.target));
            if confirmed || input.now.saturating_duration_since(pending.since) >= PENDING {
                self.pending = None;
            }
        }
        // A panel dismissed by someone else is no longer carried.
        if let Some(held) = &self.held
            && projection.panel(&held.panel).is_none()
        {
            self.held = None;
        }
        let viewer = input.viewer.unwrap_or(HOME);
        let walls: Vec<(&Wall, Mat4)> = projection
            .walls
            .iter()
            .filter_map(|wall| model(&wall.pose).map(|frame| (wall, frame)))
            .collect();
        let mut output = Output::default();
        // The panel carried follows the ray, facing the viewer, as far along it and as turned as the right stick puts it.
        if let Some(held) = &mut self.held
            && input.active
        {
            output.turned = held.manipulation.update(input.stick, input.dt);
            if input.straighten && !held.manipulation.square() {
                held.manipulation.straighten();
                output.turned = true;
            }
            if let Some(ray) = input.ray {
                // Upright, as it floats: facing the viewer turned only about the vertical, then by its snaps.
                let at = ray.at(held.manipulation.distance());
                held.model = facing(at, Vec3::new(viewer.x, at.y, viewer.z), 1.0)
                    * Mat4::from_rotation_y(held.manipulation.yaw());
            }
        }
        // The slot it would be put in: the one the ray meets, or else one it is carried near. While a teleport is aimed,
        // the ray chooses none.
        self.aim = self.held.as_ref().and_then(|held| {
            input
                .ray
                .filter(|_| input.active && input.pointing)
                .and_then(|ray| aim(&walls, &ray))
                .or_else(|| snap(&walls, held.model.w_axis.truncate()))
        });
        // Let go, or taken by something else, it goes into that slot, or floats where it is, as it is turned.
        if (released || !input.active)
            && let Some(held) = self.held.take()
        {
            let (target, model) = match self.aim.take() {
                Some(aim) => (Target::Wall(aim.wall, aim.slot), aim.model),
                None => (Target::Free(pose(held.model)), held.model),
            };
            self.pending = Some(Pending {
                panel: held.panel.clone(),
                target: target.clone(),
                model,
                since: input.now,
            });
            output.placed = Some((held.panel, target));
        }
        let standing: Vec<Standing> = projection
            .panels
            .iter()
            .filter_map(|panel| {
                self.stands(panel, &walls)
                    .map(|(model, fit)| (panel, model, fit))
            })
            .collect();
        // How far each panel shown scrolls, as it is sized now, and the one scrolled, within that.
        self.limits = standing
            .iter()
            .map(|(panel, _, fit)| {
                (
                    panel.id.clone(),
                    most_scroll(panel, *fit, self.layout(&panel.id)),
                )
            })
            .collect();
        if let Some((panel, rows)) = input.scroll
            && let Some(limit) = self.limits.get(panel)
            && rows.is_finite()
        {
            let limit = *limit as f32;
            let scroll = self.scrolls.entry(panel.to_owned()).or_default();
            *scroll = (scroll.min(limit) + rows).clamp(0.0, limit);
        }
        // The nearest panel the ray meets, other than one carried, is pointed at.
        let carried = self.held();
        let pointed = input
            .ray
            .filter(|_| input.active && input.pointing)
            .and_then(|ray| {
                standing
                    .iter()
                    .filter(|(panel, ..)| Some(panel.id.as_str()) != carried)
                    .filter_map(|(panel, model, fit)| {
                        hit(*model, self.size(panel, *fit), &ray)
                            .map(|distance| (*panel, *model, *fit, distance, ray.at(distance)))
                    })
                    .min_by(|a, b| a.3.total_cmp(&b.3))
            });
        output.item = pointed.and_then(|(panel, model, fit, _, point)| {
            item_at(panel, model, fit, self.scroll(&panel.id), point)
        });
        output.pointed =
            pointed.map(|(panel, _, _, distance, point)| (panel.id.clone(), distance, point));
        // A squeeze pulls off the panel pointed at, unless something nearer takes it.
        if pressed
            && carried.is_none()
            && let Some((panel, distance, _)) = output
                .pointed
                .take_if(|(_, distance, _)| *distance < input.nearer)
        {
            let model = standing
                .iter()
                .find(|(shown, ..)| shown.id == panel)
                .map(|(_, model, _)| *model)
                .expect("pointed at a panel shown");
            self.held = Some(Held {
                panel,
                manipulation: Manipulation::new(distance),
                model,
            });
            self.pending = None;
            output.picked = true;
        }
        output.preview = self.aim.as_ref().map(|aim| (aim.wall.clone(), aim.slot));
        // The ray ends where it aims at a slot, or else at the middle of the panel carried, or where it meets the panel
        // pointed at.
        output.end = match &self.held {
            Some(held) => Some(
                self.aim
                    .as_ref()
                    .and_then(|aim| aim.point)
                    .unwrap_or(held.model.w_axis.truncate()),
            ),
            None => output.pointed.as_ref().map(|(_, _, point)| *point),
        };
        output.sheets = self.sheets(
            projection,
            &walls,
            &standing,
            viewer,
            output.pointed.as_ref().map(|(id, ..)| id.as_str()),
        );
        // While a panel is carried, the hint beside it says what each control does to it, and what letting go would do,
        // from the frame after it is pulled off, once where it would go is known.
        let carried = self
            .held
            .as_ref()
            .filter(|_| input.active && !output.picked);
        // The panel carried is always shown, as one no longer shown is no longer carried.
        output.hint = carried
            .and_then(|held| projection.panel(&held.panel).map(|panel| (held, panel)))
            .map(|(held, panel)| {
                let release = match &self.aim {
                    None => Release::Float,
                    Some(aim) => {
                        let taken = projection.panels.iter().any(|other| {
                            other.id != held.panel
                                && matches!(&other.place, Place::Wall { wall, slot } if *wall == aim.wall && *slot == aim.slot)
                        });
                        if taken { Release::Swap } else { Release::Slot }
                    }
                };
                hint_sheet(
                    hint(input.controls, release),
                    held.model,
                    self.size(panel, Fit::Text),
                    viewer,
                )
            });
        output
    }

    /// Throw away the panel carried, at `now`: it is no longer carried, nor let go anywhere, and is hidden until the
    /// server's projection no longer shows it, or for [`PENDING`]. Returns its identity, if one was carried.
    pub fn throw(&mut self, now: Instant) -> Option<String> {
        let held = self.held.take()?;
        self.aim = None;
        self.thrown = Some((held.panel.clone(), now));
        Some(held.panel)
    }

    /// What to draw: each wall with the panels in its layout, and the outline of the slot a panel carried would take,
    /// and each panel floating or carried, the groups farthest from `viewer` first, since panels blend over what is drawn
    /// before them and write depth only once drawn, so that a nearer one covers one behind it, and a wall's panels cover
    /// the wall.
    fn sheets(
        &self,
        projection: &Projection,
        walls: &[(&Wall, Mat4)],
        standing: &[Standing],
        viewer: Vec3,
        pointed: Option<&str>,
    ) -> Vec<Sheet> {
        let highlighted = self.held().or(pointed);
        // The newest panel on each wall, by age, then by its place in the list.
        let newest = |panel: &Panel| match &panel.place {
            Place::Wall { wall, .. } => projection
                .panels
                .iter()
                .filter(|other| matches!(&other.place, Place::Wall { wall: on, .. } if on == wall))
                // Of those projected together, the last the server lists, as it lists them in the order it put them.
                .rev()
                .min_by_key(|other| other.age)
                .is_some_and(|other| other.id == panel.id),
            _ => false,
        };
        let sheet = |panel: &Panel, model: Mat4, fit: Fit| Sheet {
            canvas: Some(scrolled_face(
                panel,
                highlighted == Some(panel.id.as_str()),
                newest(panel),
                fit,
                self.scroll(&panel.id),
                self.layout(&panel.id),
            )),
            model: Some(model),
        };
        // A panel drawn in a wall's slot belongs to that wall's group.
        let moved = self
            .held()
            .or(self.pending.as_ref().map(|pending| pending.panel.as_str()));
        fn in_layout<'a>(panel: &'a Panel, moved: Option<&str>) -> Option<&'a str> {
            match &panel.place {
                Place::Wall { wall, .. } if moved != Some(panel.id.as_str()) => Some(wall),
                _ => None,
            }
        }
        let mut groups: Vec<(f32, Vec<Sheet>)> = walls
            .iter()
            .map(|(wall, frame)| {
                // A flat wall is one board; a curved one, a board for each slot, and its label below their middle.
                let mut sheets = match arc(wall) {
                    None => vec![Sheet {
                        canvas: Some(wall_face(wall)),
                        model: Some(*frame),
                    }],
                    Some(_) => {
                        let (_, height) = panel_size();
                        let mut boards: Vec<Sheet> = slot_frames(wall, *frame)
                            .into_iter()
                            .map(|model| Sheet {
                                canvas: Some(slot_board(wall_border(wall))),
                                model: Some(model),
                            })
                            .collect();
                        // Below the row, since the status pill stands just above it from home.
                        boards.push(Sheet {
                            canvas: Some(arc_label(wall)),
                            model: Some(
                                *frame
                                    * Mat4::from_translation(
                                        -Vec3::Y * (height * ARC_SCALE / 2.0 + 0.04),
                                    ),
                            ),
                        });
                        boards
                    }
                };
                sheets.extend(
                    standing
                        .iter()
                        .filter(|(panel, ..)| in_layout(panel, moved) == Some(wall.id.as_str()))
                        .map(|(panel, model, fit)| sheet(panel, *model, *fit)),
                );
                // The outline of the slot a panel carried would take, over any panel in it.
                if let Some(aim) = self.aim.as_ref().filter(|aim| aim.wall == wall.id) {
                    sheets.push(Sheet {
                        canvas: Some(ghost()),
                        model: Some(aim.model * Mat4::from_translation(Vec3::Z * LIFT)),
                    });
                }
                (frame.w_axis.truncate().distance(viewer), sheets)
            })
            .collect();
        groups.extend(
            standing
                .iter()
                .filter(|(panel, ..)| in_layout(panel, moved).is_none())
                .map(|(panel, model, fit)| {
                    (
                        model.w_axis.truncate().distance(viewer),
                        vec![sheet(panel, *model, *fit)],
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
        if self
            .thrown
            .as_ref()
            .is_some_and(|(thrown, _)| *thrown == panel.id)
        {
            return None;
        }
        match (&self.held, &self.pending) {
            (Some(held), _) if held.panel == panel.id => return Some((held.model, Fit::Text)),
            (_, Some(pending)) if pending.panel == panel.id => {
                let fit = match pending.target {
                    Target::Wall(..) => Fit::Slot,
                    Target::Free(_) => Fit::Text,
                };
                return Some((pending.model, fit));
            }
            _ => {}
        }
        match &panel.place {
            Place::Wall { wall, slot } if *slot < WALL_SLOTS => walls
                .iter()
                .find(|(shown, _)| shown.id == *wall)
                .map(|(wall, frame)| (slot_frames(wall, *frame)[*slot as usize], Fit::Slot)),
            Place::Free { pose } => model(pose).map(|model| (model, Fit::Text)),
            _ => None,
        }
    }

    /// The panel being carried, by identity.
    pub fn held(&self) -> Option<&str> {
        self.held.as_ref().map(|held| held.panel.as_str())
    }

    /// Whether `panel`, shown last frame, has more than its face shows at once, so that it scrolls.
    pub fn scrollable(&self, panel: &str) -> bool {
        self.limits.get(panel).is_some_and(|limit| *limit > 0)
    }

    /// How many rows down `panel` is scrolled, this participant's own, as far as it goes as it was last sized.
    pub fn scroll(&self, panel: &str) -> usize {
        let limit = self.limits.get(panel).copied().unwrap_or_default();
        let scroll = self.scrolls.get(panel).copied().unwrap_or_default();
        (scroll.round() as usize).min(limit)
    }

    /// Lay out the code of each of `projection`'s code panels whose code has changed, or is new, and forget those of
    /// panels, and how far they are scrolled, no longer projected.
    fn lay_out(&mut self, projection: &Projection) {
        self.layouts.retain(|id, _| {
            projection
                .panel(id)
                .is_some_and(|panel| panel.code.is_some())
        });
        self.scrolls.retain(|id, _| projection.panel(id).is_some());
        for panel in &projection.panels {
            let Some(code) = &panel.code else {
                continue;
            };
            let current = self
                .layouts
                .get(&panel.id)
                .is_some_and(|laid| laid.body == panel.body && laid.code == *code);
            if !current {
                let laid = Laid {
                    body: panel.body.clone(),
                    code: code.clone(),
                    layout: Arc::new(CodeLayout::new(panel, code)),
                };
                self.layouts.insert(panel.id.clone(), laid);
            }
        }
    }

    /// The layout of `panel`'s code, if it is a code panel.
    fn layout(&self, panel: &str) -> Option<&CodeLayout> {
        self.layouts.get(panel).map(|laid| laid.layout.as_ref())
    }

    /// The size of `panel`, in metres, sized as `fit` says, as it is drawn.
    fn size(&self, panel: &Panel, fit: Fit) -> (f32, f32) {
        size_of(panel, fit, self.layout(&panel.id))
    }
}

mod image;

#[cfg(test)]
mod tests;
