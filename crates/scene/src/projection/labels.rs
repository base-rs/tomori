// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The label on the ray naming the panel it points at, kept while it says the same, and the hint beside a panel
//! carried, saying what each control does to it. See `design/projection.md`.
use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3};
use tomori_protocol::{Panel, Place};

use super::{
    FRAME, HIGHLIGHT, HINT_ACTIONS, HINT_ANGLE, HINT_FLOOR, HINT_GAP, HINT_LOWEST, HINT_SPACE,
    LABEL_COLUMNS, LABEL_PIXEL, THROW, held, length, name,
};
use crate::{
    canvas::Canvas,
    controller_ui::Sheet,
    recording::facing,
    transcript::{BRIGHT, DIM, TEXT},
    ui_text::{Row, Style, cut},
};

/// What A does to the item of a panel the ray meets, which the label on the ray says: tick or untick a checklist item,
/// or choose a row of an environment's list, which, just chosen, it says it has. See `design/projection.md` and
/// `design/panel-actions.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemAction {
    Tick,
    Untick,
    Choose,
    Chosen,
}

/// The label on the ray naming `panel`, pointed at where the ray meets it, `at`, facing `viewer`: what it is called, its
/// metadata, and when and where it was projected; where it `scrolls`, how to scroll it with those controls; and, where
/// the ray meets an `item` A acts on, a checklist item it would tick or untick or a row it would choose, how to with
/// those, or that the row was just chosen.
pub fn label(
    panel: &Panel,
    at: Vec3,
    viewer: Vec3,
    scrolls: Option<Controls>,
    item: Option<(Controls, ItemAction)>,
) -> Sheet {
    label_sheet(
        crate::frame_timing::drawn(label_face(panel, scrolls, item)),
        at,
        viewer,
    )
}

/// The face of the label on the ray naming `panel`, as [`label`] draws it.
fn label_face(
    panel: &Panel,
    scrolls: Option<Controls>,
    item: Option<(Controls, ItemAction)>,
) -> Canvas {
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
    let place = place_named(&panel.place);
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
    if let Some((controls, action)) = item {
        let control = match (action, controls) {
            (ItemAction::Chosen, _) => "✓",
            (_, Controls::Controllers) => "A",
            (_, Controls::Keyboard) => "Enter",
        };
        let (does, style) = match action {
            ItemAction::Tick => ("tick", Style::plain(TEXT)),
            ItemAction::Untick => ("untick", Style::plain(TEXT)),
            ItemAction::Choose => ("choose", Style::plain(TEXT)),
            ItemAction::Chosen => ("chosen", Style::bold(HIGHLIGHT)),
        };
        canvas.rows.push(Row::new(cut(
            &[
                crate::ui_text::Span::new(format!("{control}  "), Style::bold(BRIGHT)),
                crate::ui_text::Span::new(does, style),
            ],
            LABEL_COLUMNS,
        )));
    }
    canvas
}

/// Where the label on the ray says a panel at `place` is.
fn place_named(place: &Place) -> &'static str {
    match place {
        Place::Wall { wall, .. } if wall == tomori_protocol::TOMORI_WALL => "on the Tomori wall",
        Place::Wall { .. } => "on a wall",
        _ => "floating",
    }
}

/// The label on the ray showing `canvas`, pointed at where the ray meets its panel, `at`, facing `viewer`.
fn label_sheet(canvas: Arc<Canvas>, at: Vec3, viewer: Vec3) -> Sheet {
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

/// What the label on the ray is drawn from: the panel it names, how it scrolls and the item it names.
type LabelKey = (Panel, Option<Controls>, Option<(Controls, ItemAction)>);

/// The label on the ray last drawn, with what it was drawn from, handed over again while it says the same, so that it
/// is not drawn again and its mesh is kept as it turns to face the viewer.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeptLabel(Option<(LabelKey, Arc<Canvas>)>);

impl KeptLabel {
    /// The label on the ray naming `panel`, as [`label`] draws it, its face the one kept while it says the same.
    pub(crate) fn sheet(
        &mut self,
        panel: &Panel,
        (at, viewer): (Vec3, Vec3),
        scrolls: Option<Controls>,
        item: Option<(Controls, ItemAction)>,
    ) -> Sheet {
        let canvas = match &self.0 {
            Some(((kept, was_scrolling, was_item), canvas))
                if *was_scrolling == scrolls && *was_item == item && says_same(kept, panel) =>
            {
                canvas.clone()
            }
            _ => {
                let canvas = crate::frame_timing::drawn(label_face(panel, scrolls, item));
                self.0 = Some(((panel.clone(), scrolls, item), canvas.clone()));
                canvas
            }
        };
        label_sheet(canvas, at, viewer)
    }
}

/// Whether the label on the ray naming `kept` says what one naming `panel` would, compared in place: its title, its
/// metadata, its code, whether it is pinned, where it is, and its age as it shows, and its body where its name or the
/// lines it holds come from it.
fn says_same(kept: &Panel, panel: &Panel) -> bool {
    kept.title == panel.title
        && kept.meta == panel.meta
        && kept.code == panel.code
        && kept.pinned == panel.pinned
        && place_named(&kept.place) == place_named(&panel.place)
        && (kept.age == panel.age || crate::board::age(kept.age) == crate::board::age(panel.age))
        && ((!panel.title.is_empty() && panel.code.is_none()) || kept.body == panel.body)
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
    hint_of(controls, release, true)
}

/// The hint beside a panel carried in the other hand while the stick, its click and B act on what the first carries:
/// only what letting go of it would do now, `release`, as [`hint`] says it.
pub fn let_go_hint(controls: Controls, release: Release) -> Canvas {
    hint_of(controls, release, false)
}

/// The hint [`hint`] draws, with the rows for the stick, its click and B only where `steered`.
fn hint_of(controls: Controls, release: Release, steered: bool) -> Canvas {
    let let_go = match release {
        Release::Slot => "put it in the slot",
        Release::Swap => "swap with that one",
        Release::Float => "leave it floating",
    };
    let all: [(&str, &str); 5] = match controls {
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
    let rows = if steered { &all[..] } else { &all[4..] };
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
                3 if steered => (Style::bold(THROW), Style::plain(THROW)),
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
pub(super) fn hint_sheet(
    mut canvas: Canvas,
    model: Mat4,
    (_, height): (f32, f32),
    viewer: Vec3,
) -> Sheet {
    let centre = model.w_axis.truncate();
    let distance = centre.distance(viewer).max(0.1);
    canvas.pixel = HINT_ANGLE * distance;
    let (_, tall) = canvas.size();
    // The panel's height as drawn, at a slot's scale while it docks in one.
    let height = height * model.y_axis.truncate().length();
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
        canvas: Some(crate::frame_timing::drawn(canvas)),
    }
}
