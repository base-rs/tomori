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
use tomori_protocol::{
    CODE_LINES, Code, Panel, Place, Pose, Projection, TokenKind, WALL_SLOTS, Wall, WallKind,
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

/// A panel's face, in cells: a title, a row of metadata, a rule, and its body.
pub const COLUMNS: usize = 34;
pub const BODY_ROWS: usize = 12;
const ROWS: usize = BODY_ROWS + 3;
/// The size of a font pixel on a wall, in metres: a panel is about 60 by 36 cm, its text legible from about 3 m.
pub const PIXEL: f32 = 0.0019;
const PADDING: (usize, usize) = (6, 5);

/// A code panel's face, in cells: a title, a row of its language, length and age, a rule, its lines, each after its
/// number, and a row saying how many more lines there are.
pub const CODE_COLUMNS: usize = 48;
const CODE_ROWS: usize = 3 + CODE_LINES + 1;
const CODE_PADDING: (usize, usize) = (6, 12);
/// The cells of a code panel's line number, and the space after it.
const GUTTER: usize = 3;
/// The size of a font pixel on a code panel, in metres, smaller than a text panel's so that the same panel holds its
/// [`CODE_COLUMNS`] and [`CODE_LINES`], legible from about 2 m.
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

const FRAME: [f32; 4] = [0.32, 0.38, 0.46, 0.9];
const NEWEST: [f32; 4] = [0.3, 0.75, 0.95, 1.0];
const HIGHLIGHT: [f32; 4] = [0.35, 0.95, 1.0, 1.0];
const SLOT: [f32; 4] = [0.0, 0.0, 0.0, 0.3];

/// Where a participant puts a panel they let go.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Into the layout of the wall with this identity.
    Wall(String),
    /// Floating free at this pose.
    Free(Pose),
}

/// The size of a panel, in metres.
pub fn panel_size() -> (f32, f32) {
    let width = COLUMNS * CELL_WIDTH + 2 * PADDING.0;
    let height = ROWS * CELL_HEIGHT + 2 * PADDING.1;
    (width as f32 * PIXEL, height as f32 * PIXEL)
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
    match code.lines {
        1 => "1 line".to_owned(),
        lines => format!("{lines} lines"),
    }
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

/// What a panel's face shows: its title, bold, its metadata and age, a rule, and its body as Markdown-lite, styled as
/// the transcript styles a message, on [`BODY_ROWS`] rows, ending with "…" where it is longer. A `highlighted` panel,
/// pointed at or carried, is framed brightly, and the `newest` on its wall in blue.
pub fn face(panel: &Panel, highlighted: bool, newest: bool) -> Canvas {
    let border = Some(if highlighted {
        HIGHLIGHT
    } else if newest {
        NEWEST
    } else {
        FRAME
    });
    if let Some(code) = &panel.code {
        let mut canvas = code_face(panel, code);
        canvas.border = border;
        return canvas;
    }
    let mut canvas = Canvas::new(COLUMNS, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = border;
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), COLUMNS),
        Row::text(&about(panel), Style::plain(DIM), COLUMNS),
        Row::text(&"─".repeat(COLUMNS), Style::plain(DIM), COLUMNS),
    ];
    let mut body = crate::transcript::parse_markdown(&panel.body, COLUMNS);
    if body.len() > BODY_ROWS {
        body.truncate(BODY_ROWS);
        let last = &mut body[BODY_ROWS - 1];
        last.truncate(COLUMNS - 1);
        last.push(('…', Style::plain(TEXT)));
    }
    body.resize(BODY_ROWS, Vec::new());
    rows.extend(body.into_iter().map(Row::new));
    canvas.rows = rows;
    canvas
}

/// A code panel's face: its title, bold, its language, length and age, a rule, and its lines in monospace, each after
/// its number, coloured by kind as [`palette`] colours them, a line longer than the panel ending with "…", then how many
/// more lines its block has. Spans that are not valid for its body, as `valid_spans` checks them, are ignored, and the
/// code drawn plain.
fn code_face(panel: &Panel, code: &Code) -> Canvas {
    let mut canvas = Canvas::new(CODE_COLUMNS, CODE_PIXEL);
    canvas.padding = CODE_PADDING;
    canvas.background = [0.015, 0.025, 0.045, 0.95];
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), CODE_COLUMNS),
        Row::text(&about(panel), Style::plain(DIM), CODE_COLUMNS),
        Row::text(&"─".repeat(CODE_COLUMNS), Style::plain(DIM), CODE_COLUMNS),
    ];
    // The kind of each character of the body, in order.
    let mut kinds = vec![TokenKind::Plain; panel.body.chars().count()];
    if tomori_protocol::valid_spans(&panel.body, &code.spans) {
        for span in &code.spans {
            kinds[span.start as usize..span.end() as usize].fill(span.kind);
        }
    }
    let mut at = 0;
    let lines: Vec<&str> = panel.body.split('\n').collect();
    for (number, line) in lines.iter().enumerate().take(CODE_LINES) {
        let mut spans = vec![crate::ui_text::Span::new(
            format!("{:>width$} ", number + 1, width = GUTTER - 1),
            Style::plain(DIM),
        )];
        for (index, c) in line.chars().enumerate() {
            let style = Style::plain(palette(kinds[at + index]));
            // Joined to the run before, of the same kind, but never to the line's number.
            if spans.len() > 1 && spans[spans.len() - 1].style == style {
                spans.last_mut().expect("a run before").text.push(c);
            } else {
                spans.push(crate::ui_text::Span::new(c.to_string(), style));
            }
        }
        at += line.chars().count() + 1;
        let mut cells = cut(&spans, CODE_COLUMNS);
        // A line cut, here or by the server, ends with a dim "…".
        if let Some(last) = cells.last_mut().filter(|(c, _)| *c == '…') {
            last.1 = Style::plain(DIM);
        }
        rows.push(Row::new(cells));
    }
    let shown = lines.len().min(CODE_LINES);
    rows.resize(CODE_ROWS - 1, Row::default());
    let more = (code.lines as usize).saturating_sub(shown);
    rows.push(match more {
        0 => Row::default(),
        1 => Row::text("…1 more line", Style::plain(DIM), CODE_COLUMNS),
        more => Row::text(
            &format!("…{more} more lines"),
            Style::plain(DIM),
            CODE_COLUMNS,
        ),
    });
    canvas.rows = rows;
    canvas
}

/// A wall's own face: a translucent board labelled with whose it is, its slots faintly marked.
fn wall_face(wall: &Wall) -> Canvas {
    let mut canvas = Canvas::new(WALL_COLUMNS, PIXEL);
    canvas.padding = WALL_PADDING;
    canvas.background = [0.01, 0.02, 0.04, 0.5];
    let (label, color, border) = match wall.kind {
        WallKind::Tomori => ("TOMORI", [0.6, 0.85, 1.0, 1.0], [0.35, 0.75, 0.95, 0.55]),
        _ => ("WALL", DIM, [0.5, 0.55, 0.62, 0.45]),
    };
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

/// The label on the ray naming `panel`, pointed at where the ray meets it, `at`, facing `viewer`: what it is called, its
/// metadata, and when and where it was projected.
pub fn label(panel: &Panel, at: Vec3, viewer: Vec3) -> Sheet {
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
        let mut about = vec![if code.lines as usize > CODE_LINES {
            format!("lines 1–{CODE_LINES} of {}", code.lines)
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

/// Where `ray` meets a panel drawn with `model`, as the distance along it, if it does.
fn hit(model: Mat4, ray: &Ray) -> Option<f32> {
    let (width, height) = panel_size();
    let local = model.inverse();
    let origin = local.transform_point3(ray.origin);
    let direction = local.transform_vector3(ray.direction);
    if direction.z.abs() < f32::EPSILON {
        return None;
    }
    let along = -origin.z / direction.z;
    let point = origin + direction * along;
    (along >= 0.0 && point.x.abs() <= width / 2.0 && point.y.abs() <= height / 2.0).then_some(along)
}

/// The wall a panel let go at `model` snaps onto, if it is near enough to one: within [`SNAP_DEPTH`] of its plane and
/// [`SNAP_MARGIN`] of its edges, the nearest such plane.
fn snap(walls: &[(&Wall, Mat4)], model: Mat4) -> Option<String> {
    let centre = model.w_axis.truncate();
    let (width, height) = wall_size();
    walls
        .iter()
        .filter_map(|(wall, frame)| {
            let local = frame.inverse().transform_point3(centre);
            (local.z.abs() <= SNAP_DEPTH
                && local.x.abs() <= width / 2.0 + SNAP_MARGIN
                && local.y.abs() <= height / 2.0 + SNAP_MARGIN)
                .then_some((wall.id.clone(), local.z.abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Whether `place`, where the server says a panel is, is where it was put, `target`.
fn agrees(place: &Place, target: &Target) -> bool {
    match (place, target) {
        (Place::Wall { wall, .. }, Target::Wall(target)) => wall == target,
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
    /// A panel was pulled off.
    pub picked: bool,
    /// The panel carried turned a snap, or back to square.
    pub turned: bool,
    /// A panel was let go, to send to the server: onto a wall, or floating.
    pub placed: Option<(String, Target)>,
    /// Where the ray ends on a panel: where it meets the panel pointed at, or the middle of the one carried.
    pub end: Option<Vec3>,
}

/// The panels across frames: the one carried, and the one let go that the server has not yet confirmed.
#[derive(Clone, Debug, Default)]
pub struct Projector {
    grip: Grip,
    held: Option<Held>,
    pending: Option<Pending>,
}

impl Projector {
    /// Follow `input`, and say what to draw and send.
    pub fn update(&mut self, input: Input<'_>) -> Output {
        // The grip is followed even while the panels do not take it, so that a squeeze held as they start to is not a new
        // one.
        let (pressed, released) = self.grip.update(input.grip);
        let Some(projection) = input.projection else {
            self.held = None;
            self.pending = None;
            return Output::default();
        };
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
        // Let go, or taken by something else, it snaps onto a wall near it, or floats where it is.
        if (released || !input.active)
            && let Some(held) = self.held.take()
        {
            let target = snap(&walls, held.model)
                .map_or_else(|| Target::Free(pose(held.model)), Target::Wall);
            self.pending = Some(Pending {
                panel: held.panel.clone(),
                target: target.clone(),
                model: held.model,
                since: input.now,
            });
            output.placed = Some((held.panel, target));
        }
        let models: Vec<(&Panel, Mat4)> = projection
            .panels
            .iter()
            .filter_map(|panel| self.stands(panel, &walls).map(|model| (panel, model)))
            .collect();
        // The nearest panel the ray meets, other than one carried, is pointed at.
        let carried = self.held();
        output.pointed = input
            .ray
            .filter(|_| input.active && input.pointing)
            .and_then(|ray| {
                models
                    .iter()
                    .filter(|(panel, _)| Some(panel.id.as_str()) != carried)
                    .filter_map(|(panel, model)| {
                        hit(*model, &ray)
                            .map(|distance| (panel.id.clone(), distance, ray.at(distance)))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            });
        // A squeeze pulls off the panel pointed at, unless something nearer takes it.
        if pressed
            && carried.is_none()
            && let Some((panel, distance, _)) = output
                .pointed
                .take_if(|(_, distance, _)| *distance < input.nearer)
        {
            let model = models
                .iter()
                .find(|(shown, _)| shown.id == panel)
                .map(|(_, model)| *model)
                .expect("pointed at a panel shown");
            self.held = Some(Held {
                panel,
                manipulation: Manipulation::new(distance),
                model,
            });
            self.pending = None;
            output.picked = true;
        }
        output.end = match &self.held {
            Some(held) => Some(held.model.w_axis.truncate()),
            None => output.pointed.as_ref().map(|(_, _, point)| *point),
        };
        output.sheets = self.sheets(
            projection,
            &walls,
            &models,
            viewer,
            output.pointed.as_ref().map(|(id, ..)| id.as_str()),
        );
        output
    }

    /// What to draw: each wall with the panels in its layout, and each panel floating or carried, the groups farthest
    /// from `viewer` first, since panels do not write depth, so that a nearer one covers one behind it, and a wall's
    /// panels cover the wall.
    fn sheets(
        &self,
        projection: &Projection,
        walls: &[(&Wall, Mat4)],
        models: &[(&Panel, Mat4)],
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
        let sheet = |panel: &Panel, model: Mat4| Sheet {
            canvas: Some(face(
                panel,
                highlighted == Some(panel.id.as_str()),
                newest(panel),
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
                let mut sheets = vec![Sheet {
                    canvas: Some(wall_face(wall)),
                    model: Some(*frame),
                }];
                sheets.extend(
                    models
                        .iter()
                        .filter(|(panel, _)| in_layout(panel, moved) == Some(wall.id.as_str()))
                        .map(|(panel, model)| sheet(panel, *model)),
                );
                (frame.w_axis.truncate().distance(viewer), sheets)
            })
            .collect();
        groups.extend(
            models
                .iter()
                .filter(|(panel, _)| in_layout(panel, moved).is_none())
                .map(|(panel, model)| {
                    (
                        model.w_axis.truncate().distance(viewer),
                        vec![sheet(panel, *model)],
                    )
                }),
        );
        groups.sort_by(|a, b| b.0.total_cmp(&a.0));
        groups.into_iter().flat_map(|(_, sheets)| sheets).collect()
    }

    /// Where `panel` stands: carried, let go but not yet confirmed, or where the server says it is, in its wall's slot
    /// or floating. `None` for a panel on a wall not shown, in a slot past the last, or in a place this client does not
    /// know.
    fn stands(&self, panel: &Panel, walls: &[(&Wall, Mat4)]) -> Option<Mat4> {
        match (&self.held, &self.pending) {
            (Some(held), _) if held.panel == panel.id => return Some(held.model),
            (_, Some(pending)) if pending.panel == panel.id => return Some(pending.model),
            _ => {}
        }
        match &panel.place {
            Place::Wall { wall, slot } if *slot < WALL_SLOTS => walls
                .iter()
                .find(|(shown, _)| shown.id == *wall)
                .map(|(_, frame)| *frame * Mat4::from_translation(slot_offset(*slot))),
            Place::Free { pose } => model(pose),
            _ => None,
        }
    }

    /// The panel being carried, by identity.
    pub fn held(&self) -> Option<&str> {
        self.held.as_ref().map(|held| held.panel.as_str())
    }
}

#[cfg(test)]
mod tests;
