// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The back of a panel: its physical back, which shows how it is configured, drawn in place of its face when it is seen
//! from behind, as walking behind a floating panel or turning one round in the hand shows it, its text facing out of
//! the back, never mirrored. Everyone sees the same object, so there is no state of being flipped. What it says is
//! [`tomori_protocol::back::describe`]'s, in the sections of a workbook's view, inputs, outputs, definition and status,
//! with how long ago the panel last changed counted on here, and it is drawn once and kept while that is the same. See `design/panel-backs.md`.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};
use tomori_protocol::{
    Panel, Projection,
    back::{About, Described, describe, linked},
};

use super::{
    PADDING, PIXEL, chips,
    faces::{FAR, NEAR},
};
use crate::{
    canvas::{Canvas, Kind},
    shapes::Mark,
    transcript::{BRIGHT, DIM, TEXT},
    ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Span, Style, Wrap, cut, wrap},
};

/// The colour of a line of a back that warns, as an error does.
const WARNING: [f32; 4] = [1.0, 0.56, 0.46, 1.0];
/// The colour of a back's code blocks, monospaced as all its text is, and the tint behind their rows.
const CODE: [f32; 4] = [0.72, 0.88, 1.0, 1.0];
const CODE_BACKGROUND: [f32; 4] = [0.07, 0.1, 0.16, 1.0];
/// The colour a section's heading is drawn in.
const SECTION: [f32; 4] = [0.45, 0.8, 0.95, 1.0];
/// The most cells a back's keys take, past which a key is cut, so that its values have the room.
const KEY_CELLS: usize = 10;
/// What a back says below its title, so that it is known for the back at a glance.
const HEADING: &str = "Back · configuration";

/// The model of `model`'s back: the same place, turned half about its vertical, so that its text faces out of the back.
pub(super) fn turned(model: Mat4) -> Mat4 {
    model * Mat4::from_rotation_y(std::f32::consts::PI)
}

/// Whether `viewer` sees the panel standing at `model` from behind: on the far side of its plane from where its face
/// looks, along its +Z.
pub(super) fn seen_from_behind(model: Mat4, viewer: Vec3) -> bool {
    (viewer - model.w_axis.truncate()).dot(model.z_axis.truncate()) < 0.0
}

/// When what each panel shows last changed, as this client has followed it: from when the server says it did, as a
/// panel arrives, and from each change of its version or data seq after, by which its back says how long ago it was
/// updated.
#[derive(Clone, Debug, Default)]
pub(super) struct Updates {
    changed: HashMap<String, (u64, u64, Option<Instant>)>,
}

impl Updates {
    /// Follow `projection` at `now`, forgetting the panels it no longer shows.
    pub(super) fn follow(&mut self, projection: &Projection, now: Instant) {
        if self.changed.len() > projection.panels.len() {
            self.changed.retain(|id, _| projection.panel(id).is_some());
        }
        for panel in &projection.panels {
            match self.changed.get_mut(&panel.id) {
                Some((version, data_seq, at)) => {
                    if (*version, *data_seq) != (panel.version, panel.data_seq) {
                        (*version, *data_seq, *at) = (panel.version, panel.data_seq, Some(now));
                    }
                }
                None => {
                    let at = panel
                        .updated
                        .and_then(|ago| now.checked_sub(Duration::from_secs(ago)));
                    self.changed
                        .insert(panel.id.clone(), (panel.version, panel.data_seq, at));
                }
            }
        }
    }

    /// How long ago `panel` last changed at `now`, in whole seconds, if that is known.
    pub(super) fn ago(&self, panel: &str, now: Instant) -> Option<u64> {
        let (_, _, at) = self.changed.get(panel)?;
        at.map(|at| now.saturating_duration_since(at).as_secs())
    }
}

/// A back drawn: what it said, its title, size and frame, and the canvas drawn from them.
#[derive(Clone, Debug)]
struct Drawn {
    described: Described,
    title: String,
    size: (f32, f32),
    border: [f32; 4],
    canvas: Arc<Canvas>,
}

/// Each panel's back as last drawn, kept while it says the same, at the same size and frame, and forgotten once the
/// panel is no longer seen from behind.
#[derive(Clone, Debug, Default)]
pub(super) struct Backs {
    drawn: HashMap<String, Drawn>,
    /// The panels whose backs are drawn far, their titles alone.
    far: Vec<String>,
}

impl Backs {
    /// Forget every back but those of the panels `kept`.
    pub(super) fn retain(&mut self, kept: &[&str]) {
        if self.drawn.len() > kept.len() {
            self.drawn.retain(|id, _| kept.contains(&id.as_str()));
        }
        self.far.retain(|id| kept.contains(&id.as_str()));
    }

    /// Whether `panel`'s back is drawn far, its title alone, as a row of its text subtends `angle` from the viewer: below
    /// [`FAR`], as a panel's face in a slot is, and drawn whole again only above [`NEAR`], so that a viewer at the
    /// threshold does not switch it back and forth.
    pub(super) fn far(&mut self, panel: &str, angle: f32) -> bool {
        let was = self.far.iter().any(|id| id == panel);
        let far = angle < FAR || (was && angle < NEAR);
        if far && !was {
            self.far.push(panel.to_owned());
        } else if was && !far {
            self.far.retain(|id| id != panel);
        }
        far
    }

    /// `panel`'s back, among `projection`'s panels, `size` in metres and framed in `border`, updated `updated` seconds
    /// ago if that is known: the one last drawn while it says the same, and otherwise drawn now.
    pub(super) fn back(
        &mut self,
        projection: &Projection,
        panel: &Panel,
        updated: Option<u64>,
        size: (f32, f32),
        border: [f32; 4],
    ) -> Arc<Canvas> {
        let linked = linked(&projection.panels, panel);
        let today = chips::today();
        let value = |chip: &tomori_protocol::context::ContextChip| chips::value(chip, today);
        let described = describe(
            panel,
            &About {
                updated,
                linked: &linked,
                value: &value,
            },
        );
        if let Some(drawn) = self.drawn.get(&panel.id).filter(|drawn| {
            drawn.described == described
                && drawn.title == panel.title
                && drawn.size == size
                && drawn.border == border
        }) {
            return drawn.canvas.clone();
        }
        let canvas = crate::frame_timing::drawn(face(&panel.title, &described, size, border));
        self.drawn.insert(
            panel.id.clone(),
            Drawn {
                described,
                title: panel.title.clone(),
                size,
                border,
                canvas: canvas.clone(),
            },
        );
        canvas
    }
}

/// How much smaller than a panel's text a back's may be drawn, in steps, so that a back longer than its face, as a
/// small floating panel's is, shows what it says: at most two thirds as large, about as large as a code panel's.
const SCALES: [f32; 4] = [1.0, 0.85, 0.75, 0.67];

/// A panel's back, `size` in metres, as large as its face, framed in `border`: opaque, as its face is, its title, bold,
/// a heading saying it is its back, a rule, and then each section `described` says, its inputs, outputs, definition and
/// status, as a workbook's view has them, each headed, its lines, keys dim and values wrapped under themselves, a
/// warning's in its colour, and its code block, as [`rows_of`] lays them out. Its text is as large as a panel's, its columns filling
/// its width, or, where it would not all fit, a step smaller, down to the smallest of [`SCALES`]; what still does not
/// fit ends with `…`.
pub fn face(
    title: &str,
    described: &Described,
    (width, height): (f32, f32),
    border: [f32; 4],
) -> Canvas {
    let mut laid = None;
    for scale in SCALES {
        let pixel = PIXEL * scale;
        let across = width / pixel - (2 * PADDING.0) as f32;
        let columns = ((across / CELL_WIDTH as f32) as usize).max(12);
        let pixel = width / (columns * CELL_WIDTH + 2 * PADDING.0) as f32;
        let down = height / pixel;
        let rows = (((down - (2 * PADDING.1) as f32) / CELL_HEIGHT as f32) as usize).max(4);
        let body = rows_of(title, described, columns);
        let fits = body.len() <= rows;
        laid = Some((pixel, columns, rows, down, body));
        if fits {
            break;
        }
    }
    let (pixel, columns, rows, down, mut body) = laid.expect("a scale is tried");
    let vertical = ((down - (rows * CELL_HEIGHT) as f32) / 2.0)
        .round()
        .max(0.0) as usize;
    let mut canvas = Canvas::new(columns, pixel);
    canvas.kind = Kind::PanelFace;
    canvas.padding = (PADDING.0, vertical);
    canvas.background = [0.02, 0.035, 0.06, 1.0];
    canvas.border = Some(border);
    if body.len() > rows {
        // At least four rows, so that the last is one of them.
        body.truncate(rows);
        let last = &mut body[rows - 1].cells;
        last.truncate(columns - 1);
        last.push(('…', Style::plain(TEXT)));
    }
    body.resize_with(rows, || Row::new(Vec::new()));
    canvas.rows = body;
    canvas
}

/// What a section named `name` is headed: `Inputs`, `Outputs`, `Definition` or `Status`, or another's name with its
/// first letter capital and its dashes spaces.
fn heading(name: &str) -> String {
    let mut words = name.replace('-', " ");
    if let Some(first) = words.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    words
}

/// The rows of a back of `columns`, as [`face`] lays them out, however many they are: its title, its heading and a rule,
/// then each section, headed by its name in [`SECTION`], its lines, each key dim in a column as wide as the longest, at
/// most [`KEY_CELLS`], its value wrapped under itself, and its code block, its language dim, its text monospaced on a
/// tinted ground, each later row of a line marked as continued.
fn rows_of(title: &str, described: &Described, columns: usize) -> Vec<Row> {
    let mut body: Vec<Row> = vec![
        Row::text(title, Style::bold(BRIGHT), columns),
        Row::new(cut(&[Span::new(HEADING, Style::plain(DIM))], columns)),
        Mark::Rule.row(columns, Style::plain(DIM)),
    ];
    let keys = described
        .sections
        .iter()
        .flat_map(|section| &section.lines)
        .map(|line| crate::ui_text::width(&line.key))
        .max()
        .unwrap_or(0)
        .min(KEY_CELLS);
    let indent = keys + 2;
    for section in &described.sections {
        body.push(Row::text(
            &heading(&section.name),
            Style::bold(SECTION),
            columns,
        ));
        for line in &section.lines {
            let value = Style::plain(if line.warning { WARNING } else { TEXT });
            let key = cut(
                &[Span::new(
                    crate::ui_text::pad(&line.key, keys),
                    Style::plain(DIM),
                )],
                keys,
            );
            let wrapped = wrap(
                &[Span::new(line.value.as_str(), value)],
                columns.saturating_sub(indent),
                Wrap::default(),
            );
            for (index, cells) in wrapped.into_iter().enumerate() {
                let mut row = if index == 0 { key.clone() } else { Vec::new() };
                row.resize(indent, (' ', Style::plain(DIM)));
                row.extend(cells);
                body.push(Row::new(row));
            }
        }
        if let Some(code) = &section.code {
            if !code.language.is_empty() {
                body.push(Row::text(&code.language, Style::plain(DIM), columns));
            }
            let options = Wrap {
                continuation: Some(super::CONTINUED),
                ..Wrap::default()
            };
            body.extend(
                wrap(
                    &[Span::new(code.text.as_str(), Style::plain(CODE))],
                    columns,
                    options,
                )
                .into_iter()
                .map(|cells| Row::new(cells).with_background(Some(CODE_BACKGROUND))),
            );
        }
    }
    body
}

#[cfg(test)]
mod tests;
