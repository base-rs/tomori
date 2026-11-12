// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a wall's handles, its padlock and its bar draw, the readout above a handle dragged, the label of a bar, and the
//! hint beside a wall held, saying what each control does. See `design/projection.md`.
use glam::{Vec2, Vec3};
use tomori_protocol::Wall;

use super::{
    BAR_THICKNESS, Bar, Cells, Controls, FRAME, HIGHLIGHT, HINT_ANGLE, Handle, LockControls,
    Rights, THROW, WallKind, carried, shape_name,
};
use crate::{
    canvas::Canvas,
    controller_ui::Sheet,
    recording::facing,
    transcript::{BRIGHT, DIM, TEXT},
    ui_text::{Row, Style, cut},
};

/// What `handle` shows as text: the knob's dot, which its glyph centres on its cell; every other symbol is drawn as
/// shapes, by [`symbol`], centred on its handle, as a glyph's ink, sitting on its baseline, is not.
pub(super) fn glyph(handle: Handle) -> &'static str {
    match handle {
        Handle::Bend => "●",
        _ => " ",
    }
}

/// How thick the lines of a handle's symbol are, in font pixels: as a bold glyph's.
const STROKE: f32 = 1.3;

/// What `handle` shows drawn as shapes, in font pixels from its cell's top left corner, centred on the cell, and so on
/// the handle: the arrows of its edges, across or down; a stepper's minus or plus; the hinge; the padlock, closed while
/// its wall is `locked`; and a circle's arrow turning back for the reset. `behind` is the colour behind it, which
/// hides what a shape leaves out.
fn symbol(handle: Handle, locked: bool, behind: [f32; 4]) -> Vec<crate::shapes::Shape> {
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    let middle = Vec2::new(CELL_WIDTH as f32 / 2.0, CELL_HEIGHT as f32 / 2.0);
    let rule = |from: Vec2, to: Vec2| Shape::Rule {
        from: (middle + from).to_array(),
        to: (middle + to).to_array(),
        width: STROKE,
        color: BRIGHT,
    };
    // An arrow from end to end through the middle, along `along`, with a head at each end.
    let arrow = |along: Vec2| {
        let (tip, side) = (along * 4.5, along.perp() * 2.5);
        vec![
            rule(-tip, tip),
            rule(tip, tip - along * 2.5 + side),
            rule(tip, tip - along * 2.5 - side),
            rule(-tip, -tip + along * 2.5 + side),
            rule(-tip, -tip + along * 2.5 - side),
        ]
    };
    match handle {
        Handle::Columns => arrow(Vec2::X),
        Handle::Rows => arrow(Vec2::Y),
        Handle::Step(_, -1) => vec![rule(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0))],
        Handle::Step(..) => vec![
            rule(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0)),
            rule(Vec2::new(0.0, -3.0), Vec2::new(0.0, 3.0)),
        ],
        Handle::Tilt => hinge_shapes(),
        Handle::Text => text_shapes(rule),
        Handle::Lock => padlock(locked, behind),
        // A ring, its quarter at the top right left out, and a head at its right end, pointing up, the way it turns.
        // Its ring a little left of the middle, so that with its head it centres.
        Handle::Reset => {
            let (radius, left) = (3.6, Vec2::X * 0.9);
            let line = radius / 3.0;
            let centre = middle - left;
            let tip = Vec2::new(radius - line / 2.0, -1.6) - left;
            vec![
                Shape::Ring {
                    centre: centre.to_array(),
                    radius,
                    color: BRIGHT,
                },
                Shape::Fill {
                    x: centre.x + 0.3,
                    y: centre.y - radius - 0.5,
                    width: radius + 0.5,
                    height: radius - 0.3,
                    color: behind,
                },
                rule(tip, tip + Vec2::new(-2.2, 2.6)),
                rule(tip, tip + Vec2::new(2.2, 2.6)),
            ]
        }
        Handle::Corner(..) | Handle::Bend => Vec::new(),
    }
}

/// The text handle's face, in font pixels from its cell's middle, drawn with `rule`: a small `A` and a large one beside
/// it, `aA`, as text sizes are shown, so that it reads as the size of the text, centred on its handle.
fn text_shapes(rule: impl Fn(Vec2, Vec2) -> crate::shapes::Shape) -> Vec<crate::shapes::Shape> {
    // Each `A` from its apex to its feet, with its bar across at `bar` of the way down.
    let letter = |apex: Vec2, left: Vec2, right: Vec2, bar: f32| {
        [
            rule(apex, left),
            rule(apex, right),
            rule(apex.lerp(left, bar), apex.lerp(right, bar)),
        ]
    };
    letter(
        Vec2::new(-3.75, 0.5),
        Vec2::new(-5.5, 4.5),
        Vec2::new(-2.0, 4.5),
        0.62,
    )
    .into_iter()
    .chain(letter(
        Vec2::new(2.25, -4.5),
        Vec2::new(-1.0, 4.5),
        Vec2::new(5.5, 4.5),
        0.66,
    ))
    .collect()
}

/// The colour of a padlock closed, warm, so that a locked wall's is told from an open one's at a glance.
pub const LOCKED: [f32; 4] = [1.0, 0.8, 0.38, 1.0];

/// A handle's face, `size` across, highlighted while the ray meets it or it is dragged; the padlock closed while its
/// wall is `locked`, and open while it is not. Its symbol is centred on it, by what it draws.
pub fn handle_face(handle: Handle, size: f32, highlighted: bool, locked: bool) -> Canvas {
    let mut canvas = Canvas::new(1, 1.0);
    // About square, so that it is as easy to meet across as down.
    canvas.padding = (4, 2);
    canvas.background = if highlighted {
        [0.1, 0.3, 0.36, 0.95]
    } else {
        [0.02, 0.04, 0.07, 0.9]
    };
    canvas.border = Some(if highlighted { HIGHLIGHT } else { FRAME });
    canvas.rows = vec![Row::text(glyph(handle), Style::bold(BRIGHT), 1)];
    let [red, green, blue, _] = canvas.background;
    canvas.shapes = symbol(handle, locked, [red, green, blue, 1.0]);
    let (width, _) = canvas.size_in_pixels();
    canvas.pixel = size / width as f32;
    canvas
}

/// The hinge's face, in font pixels from its cell's top left corner, inside its canvas's padding, as every shape is:
/// the line the wall tilts about, level through the cell's middle and out into the padding, dim, and the wall's face
/// tilted across it, bright, as a door is drawn on its hinge, so that it is centred on its handle.
fn hinge_shapes() -> Vec<crate::shapes::Shape> {
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    let middle = [CELL_WIDTH as f32 / 2.0, CELL_HEIGHT as f32 / 2.0];
    vec![
        Shape::Rule {
            from: [middle[0] - 6.0, middle[1]],
            to: [middle[0] + 6.0, middle[1]],
            width: 1.0,
            color: DIM,
        },
        Shape::Rule {
            from: [middle[0] - 3.0, middle[1] + 5.0],
            to: [middle[0] + 3.0, middle[1] - 5.0],
            width: 1.6,
            color: BRIGHT,
        },
        Shape::Disc {
            centre: middle,
            radius: 1.6,
            color: BRIGHT,
        },
    ]
}

/// A padlock in a cell, of shapes, in font pixels from its top left corner: a body below and a shackle above it, closed
/// into the body while `locked`, in [`LOCKED`]'s warm colour, and, unlocked, raised out of it on one side, in the
/// handles' bright one. `behind` is the colour behind it, which hides the shackle's ring below its legs.
pub fn padlock(locked: bool, behind: [f32; 4]) -> Vec<crate::shapes::Shape> {
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    let (width, height) = (CELL_WIDTH as f32, CELL_HEIGHT as f32);
    let color = if locked { LOCKED } else { BRIGHT };
    // The body, a little rounded, across most of the cell's lower half, raised a little so that, heavier than the
    // shackle above it, the whole centres on the cell, and so on its handle.
    let body = (1.0, height * 0.5 - 1.5, width - 2.0, height * 0.42);
    let radius = (body.2 - 1.6) / 2.0;
    let middle = width / 2.0;
    // The shackle's top, a ring, its lower half hidden, with a leg down each side: open, it is raised, its right leg
    // short of the body.
    let lift = if locked { 0.0 } else { 2.0 };
    let top = body.1 - 0.4 - lift;
    let line = radius / 3.0;
    let (left, right) = (middle - radius + line / 2.0, middle + radius - line / 2.0);
    let right_end = if locked { body.1 + 0.5 } else { top + 0.8 };
    vec![
        Shape::Ring {
            centre: [middle, top],
            radius,
            color,
        },
        Shape::Fill {
            x: middle - radius - 0.5,
            y: top,
            width: 2.0 * radius + 1.0,
            height: radius + 0.5,
            color: behind,
        },
        Shape::Rule {
            from: [left, top - 0.05],
            to: [left, body.1 + 0.5],
            width: line,
            color,
        },
        Shape::Rule {
            from: [right, top - 0.05],
            to: [right, right_end],
            width: line,
            color,
        },
        Shape::Rect {
            x: body.0,
            y: body.1,
            width: body.2,
            height: body.3,
            color,
            radius: 0.8,
            outline: None,
        },
        // Its keyhole, dark.
        Shape::Disc {
            centre: [middle, body.1 + body.3 * 0.42],
            radius: 0.7,
            color: behind,
        },
    ]
}

/// What a bar's face is drawn from: its length in font pixels, and whether it is highlighted.
pub(super) type BarFace = (usize, bool);

/// Eight font pixels thick, and as many long as it is.
const BAR_PIXELS: usize = 8;
const BAR_PIXEL: f32 = BAR_THICKNESS / BAR_PIXELS as f32;

/// What `bar`'s face is drawn from, `highlighted` or not, and the scale it is drawn at: drawn at one size, whatever the
/// bar's, so that it is the same face for walls of any size, and only its model scales, unless its length in font
/// pixels changes.
pub(super) fn bar_key(bar: &Bar, highlighted: bool) -> (BarFace, f32) {
    let pixel = (BAR_THICKNESS * bar.size / BAR_PIXELS as f32).max(1e-5);
    let length = ((bar.half.length() / pixel).round() as usize).max(BAR_PIXELS);
    ((length, highlighted), pixel / BAR_PIXEL)
}

/// A bar's face, `length` font pixels either way and thick as a bar, subtle, and bright while `highlighted`.
pub(super) fn bar_face((length, highlighted): BarFace) -> Canvas {
    let mut canvas = Canvas::new(0, BAR_PIXEL);
    canvas.padding = (length, BAR_PIXELS / 2);
    canvas.background = if highlighted {
        HIGHLIGHT
    } else {
        [0.62, 0.68, 0.78, 0.42]
    };
    canvas
}

/// What the grip does on a bar of `wall`, as `controls` name it: takes hold of the wall to edit it, carrying one someone
/// added.
pub fn bar_label(controls: Controls, wall: &Wall) -> Canvas {
    bar_label_for(
        controls,
        wall,
        Rights {
            adjust: true,
            ..Rights::default()
        },
    )
}

/// What a locked wall the ray meets says, faintly, where the grip takes nothing: that it is locked, and, where coming
/// `closer` would show an owner its padlock, which they see only within [`PADLOCK_NEAR`](super::PADLOCK_NEAR) and in clear view, to come
/// closer to unlock it.
pub fn locked_label(closer: bool) -> Canvas {
    let text = if closer {
        "Locked: come closer to unlock"
    } else {
        "Locked"
    };
    let columns = crate::ui_text::width(text);
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.45];
    canvas.rows = vec![Row::new(cut(
        &[crate::ui_text::Span::new(text, Style::plain(DIM))],
        columns,
    ))];
    canvas
}

/// What the grip does on a bar of `wall` for a participant with `rights`: takes hold of it to edit it, carrying one it
/// carries; or, on the padlock of a locked wall, which only an owner sees, unlocks it.
pub fn bar_label_for(controls: Controls, wall: &Wall, rights: Rights) -> Canvas {
    let grip = if controls == Controls::Keyboard {
        "G"
    } else {
        "Grip"
    };
    let action = if wall.is_locked() {
        "unlock this wall"
    } else if carried(wall, rights) {
        "carry and edit this wall"
    } else {
        "edit this wall"
    };
    let columns = crate::ui_text::width(grip) + 2 + crate::ui_text::width(action);
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(FRAME);
    canvas.rows = vec![Row::new(cut(
        &[
            crate::ui_text::Span::new(
                crate::ui_text::pad(grip, crate::ui_text::width(grip) + 2),
                Style::bold(BRIGHT),
            ),
            crate::ui_text::Span::new(action, Style::plain(TEXT)),
        ],
        columns,
    ))];
    canvas
}

/// What a handle dragged makes of `wall`, as it would be let go now: for a corner, its slots' size and shape, such as
/// `60 × 107 cm · 9:16`; for the knob, its curve, such as `◠ radius 2.4 m`, or `flat`; for the hinge, its tilt, such
/// as `tilted 12° down`, or `upright`; for the text handle, the size of its content and how many columns and rows of a
/// panel's body each slot then shows, such as `Text 80% · 42 columns × 15 rows`; for an edge, its grid, such as
/// `4 × 2 slots`.
pub fn readout(handle: Handle, wall: &Wall) -> Canvas {
    let text = match handle {
        Handle::Corner(..) => {
            let (width, height) = wall.slot_size();
            format!(
                "{:.0} × {:.0} cm · {}",
                width * 100.0,
                height * 100.0,
                shape_name(wall)
            )
        }
        Handle::Bend => wall.radius().map_or("flat".to_owned(), |radius| {
            format!("◠ radius {radius:.1} m")
        }),
        Handle::Tilt => match wall.tilt() {
            tilt if tilt > 0.0 => format!("tilted {tilt:.0}° down"),
            tilt if tilt < 0.0 => format!("tilted {:.0}° up", -tilt),
            _ => "upright".to_owned(),
        },
        Handle::Text => {
            let cells = Cells::of(wall);
            format!(
                "Text {:.0}% · {} columns × {} rows",
                wall.content_scale() * 100.0,
                cells.columns,
                cells.body
            )
        }
        _ => {
            let grid = wall.grid();
            format!("{} × {} slots", grid.columns, grid.rows)
        }
    };
    let columns = crate::ui_text::width(&text) + 2;
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(HIGHLIGHT);
    canvas.rows = vec![Row::text(&format!(" {text}"), Style::bold(BRIGHT), columns)];
    canvas
}

/// The hint `canvas` for a wall, standing just above `above`, facing `viewer`, drawn at [`HINT_ANGLE`] a font pixel
/// whatever its distance.
pub(super) fn hint_above(canvas: Canvas, above: Vec3, viewer: Vec3) -> Sheet {
    hint_beside(canvas, above, viewer, 1.0)
}

/// The hint `canvas` standing just above `edge`, `side` 1, or just below it, -1, facing `viewer`, as [`hint_above`]
/// draws it.
pub(super) fn hint_beside(mut canvas: Canvas, edge: Vec3, viewer: Vec3, side: f32) -> Sheet {
    canvas.pixel = HINT_ANGLE * edge.distance(viewer).max(0.1);
    let (_, tall) = canvas.size();
    Sheet {
        model: Some(facing(edge + Vec3::Y * side * tall / 2.0, viewer, 1.0)),
        canvas: Some(crate::frame_timing::drawn(canvas)),
    }
}

/// How tall `canvas` is drawn as a hint at `edge`, seen from `viewer`, as [`hint_beside`] draws it.
pub(super) fn hint_height(canvas: &Canvas, edge: Vec3, viewer: Vec3) -> f32 {
    let mut canvas = canvas.clone();
    canvas.pixel = HINT_ANGLE * edge.distance(viewer).max(0.1);
    canvas.size().1
}

/// Where a wall carried stands with others: in a group, which it carries, and snapping beside another wall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grouping {
    pub grouped: bool,
    pub snapping: bool,
}

/// The hint beside a wall taken hold of, as [`hint_with`] says for a wall alone that would not snap.
pub fn hint(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    removes: bool,
    confirm: Option<usize>,
) -> Canvas {
    hint_with(
        controls,
        wall,
        carried,
        removes,
        confirm,
        Grouping::default(),
    )
}

/// The hint beside a wall taken hold of: what each control does to it, as `controls` name them, for `wall`, `carried`
/// or not, saying B removes it where the participant `removes` walls, and, once asked to with panels on it, `confirm`,
/// how many of them would float; and, carried, how to pull it out of its group, `grouped`, and that letting go snaps it
/// beside another, `snapping`.
pub fn hint_with(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    removes: bool,
    confirm: Option<usize>,
    grouping: Grouping,
) -> Canvas {
    let rights = Rights {
        adjust: true,
        remove: removes,
        ..Rights::default()
    };
    hint_for(controls, wall, carried, rights, confirm, grouping)
}

/// The hint beside a wall taken hold of, as [`hint_with`] says it, for a participant with `rights`: B removes one
/// someone added where they remove walls, and the bar carries one they carry. Where the server locks walls, it says too,
/// for an owner, that the grip on the padlock locks it, and, for an environment's own, that the grip on `↺` puts it back,
/// locked; a locked wall is never taken hold of.
pub fn hint_for(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    rights: Rights,
    confirm: Option<usize>,
    grouping: Grouping,
) -> Canvas {
    let keyboard = controls == Controls::Keyboard;
    let grip = if keyboard { "G" } else { "Grip" };
    let removes = rights.remove && wall.kind == WallKind::Added;
    let lock = LockControls::of(wall, rights);
    let mut rows: Vec<(String, String, bool)> = Vec::new();
    let done = (
        (if keyboard { "Backspace" } else { "B" }).to_owned(),
        "done".to_owned(),
        false,
    );
    if carried {
        rows.push((
            (if keyboard { "PgUp PgDn" } else { "Stick ↕" }).to_owned(),
            "further / closer".to_owned(),
            false,
        ));
        rows.push((
            (if keyboard { "[ ]" } else { "Stick ↔" }).to_owned(),
            "turn".to_owned(),
            false,
        ));
        if grouping.grouped {
            rows.push((
                (if keyboard {
                    "PgDn far"
                } else {
                    "Stick ↓ far"
                })
                .to_owned(),
                "take it out alone".to_owned(),
                false,
            ));
        }
        let back = if keyboard { "Backspace" } else { "B" };
        if removes {
            rows.push(match confirm {
                Some(held) => (
                    format!("{back} again"),
                    match held {
                        1 => "remove; 1 panel floats".to_owned(),
                        held => format!("remove; {held} panels float"),
                    },
                    true,
                ),
                None => (back.to_owned(), "remove, for all".to_owned(), true),
            });
        }
        if grouping.snapping {
            rows.push((
                (if keyboard {
                    "Hold click"
                } else {
                    "Hold trigger"
                })
                .to_owned(),
                "don't snap".to_owned(),
                false,
            ));
        }
        rows.push((
            format!("Let go {}", if keyboard { "G" } else { "" })
                .trim()
                .to_owned(),
            (if grouping.snapping {
                "snap it there"
            } else {
                "leave it here"
            })
            .to_owned(),
            false,
        ));
    } else {
        rows.push((
            format!("{grip} ↔ ↕"),
            "more / fewer slots".to_owned(),
            false,
        ));
        if wall.kind != WallKind::Tomori {
            rows.push((format!("{grip} corner"), "size and shape".to_owned(), false));
        }
        rows.push((format!("{grip} - +"), "one fewer / more".to_owned(), false));
        // The knob moved across bends, and the hinge moved up or down tilts.
        rows.push((format!("{grip} knob"), "↔ bend".to_owned(), false));
        rows.push((format!("{grip} hinge"), "↕ tilt".to_owned(), false));
        rows.push((format!("{grip} aA"), "↔ text size".to_owned(), false));
        if wall.kind == WallKind::Added || (rights.locks && self::carried(wall, rights)) {
            rows.push((format!("{grip} bar"), "carry it".to_owned(), false));
        }
        if lock.padlock && rights.lock {
            rows.push((format!("{grip} lock"), "lock it as it is".to_owned(), false));
        }
        if lock.reset {
            rows.push((format!("{grip} ↺"), "put back, locked".to_owned(), false));
        }
        rows.push(done);
    }
    let keys = rows
        .iter()
        .map(|(control, _, _)| crate::ui_text::width(control))
        .max()
        .unwrap_or_default()
        + 2;
    let columns = keys + 22;
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(FRAME);
    canvas.rows = rows
        .iter()
        .map(|(control, action, warm)| {
            let (key, text) = if *warm {
                (Style::bold(THROW), Style::plain(THROW))
            } else {
                (Style::bold(BRIGHT), Style::plain(TEXT))
            };
            Row::new(cut(
                &[
                    crate::ui_text::Span::new(crate::ui_text::pad(control, keys), key),
                    crate::ui_text::Span::new(action.as_str(), text),
                ],
                columns,
            ))
        })
        .collect();
    canvas
}
