// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::projection::{
    BODY_ROWS, COLUMNS, Controls, Fit, Input, Projector, editing, faces, text_face,
};
use glam::{Vec2, Vec3};
use tomori_protocol::{Place, Pose, Projection};

/// A queue as the server writes it: `pod112` highlighted, so written with its pointer, each of the others in a tone,
/// and two with values against their limits.
fn queue() -> Panel {
    Panel::new(
        "panel-1",
        "Attention",
        "- pod7 · idle\n- **▸** pod112 · 41 waiting\n- pod9 · 9 waiting\n- pod3 · stuck",
        Place::Free {
            pose: Pose::default(),
        },
    )
    .with_rows(vec![
        ListRow::new("pod7", false),
        ListRow::new("pod112", true)
            .with_highlight(true)
            .with_tone(Tone::Bad)
            .with_value(0.82),
        ListRow::new("pod9", true)
            .with_tone(Tone::Warn)
            .with_value(0.45),
        ListRow::new("pod3", true).with_tone(Tone::Good),
    ])
}

/// The text of each of `lines`.
fn texts(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.cells.iter().map(|(ch, _)| ch).collect::<String>())
        .collect()
}

/// The boxes in `shapes`, by their colour, from the top, and how far down each begins.
fn boxes(shapes: &[Shape]) -> Vec<(Rgba, f32, Option<f32>)> {
    shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Rect {
                color, y, outline, ..
            } => Some((color, y, outline)),
            _ => None,
        })
        .collect()
}

/// The lines in `shapes`: where each begins and ends, and its colour.
fn strokes(shapes: &[Shape]) -> Vec<([f32; 2], [f32; 2], Rgba)> {
    shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Stroke {
                from, to, color, ..
            } => Some((from, to, color)),
            _ => None,
        })
        .collect()
}

/// Where a body line `line` lines down on a face shows, in font pixels, below its title, its metadata and its rule.
fn top(line: usize) -> f32 {
    ((3 + line) * CELL_HEIGHT) as f32
}

#[test]
fn a_list_s_rows_show_their_tone_as_a_mark_and_its_highlighted_row_without_its_pointer() {
    let lines = lines(&queue(), COLUMNS);
    assert_eq!(
        texts(&lines),
        [
            "• pod7 · idle",
            "✕ pod112 · 41 waiting",
            "! pod9 · 9 waiting",
            "✓ pod3 · stuck"
        ]
    );
    // Each mark is bold, in its tone's colour, so that it reads by its shape and its colour alike.
    let marks: Vec<_> = lines.iter().map(|line| line.cells[0].1).collect();
    assert_eq!(marks[1], Style::bold(crate::chart::tone(Tone::Bad)));
    assert_eq!(marks[2], Style::bold(crate::chart::tone(Tone::Warn)));
    assert_eq!(marks[3], Style::bold(crate::chart::tone(Tone::Good)));
    assert!(!marks[0].bold);
    // A row that wraps shows its mark once, on its first line; and a pointer on a row the list does not highlight is
    // its text, as it is a line past the rows.
    let mut long = queue();
    long.body = format!(
        "- **▸** {}\n- **▸** pod112\n- pod9\n- pod3\n- **▸** beyond",
        "word ".repeat(12)
    );
    long.rows[0].tone = Tone::Good;
    let lines = super::lines(&long, COLUMNS);
    let text = texts(&lines);
    assert!(text[0].starts_with("✓ ▸ word"), "{text:?}");
    assert!(!text[1].contains('✓'));
    assert_eq!(text[2], "✕ pod112");
    assert_eq!(text.last().unwrap(), "• ▸ beyond");
}

#[test]
fn a_list_draws_its_highlighted_row_on_a_bar_and_its_tones_and_values_as_strips_and_thin_bars() {
    let panel = queue();
    let lines = lines(&panel, COLUMNS);
    let shapes = shapes(
        &panel.rows,
        &lines,
        (0, BODY_ROWS),
        COLUMNS,
        Marked::default(),
    );
    // One bar, the highlighted row's, across the face.
    let width = (COLUMNS * CELL_WIDTH) as f32;
    assert_eq!(boxes(&shapes), [(HIGHLIGHTED, top(1), None)]);
    assert!(shapes.iter().any(|shape| matches!(
        *shape,
        Shape::Rect { x, width: across, .. } if x == -BAR_LEFT && across == width + BAR_LEFT + BAR_RIGHT
    )));
    // A strip at the start of each row in a tone, none for a neutral one, and a thin bar under each row with a value:
    // its track the width of its text, and its fill as far along as its value, in its tone.
    let strokes = strokes(&shapes);
    let strips: Vec<_> = strokes
        .iter()
        .filter(|(from, ..)| from[0] == STRIP_X)
        .map(|(from, _, color)| (from[1] - 1.5, *color))
        .collect();
    assert_eq!(
        strips,
        [
            (top(1), crate::chart::tone(Tone::Bad)),
            (top(2), crate::chart::tone(Tone::Warn)),
            (top(3), crate::chart::tone(Tone::Good)),
        ]
    );
    let values: Vec<_> = strokes
        .iter()
        .filter(|(from, ..)| from[0] == VALUE_X)
        .map(|(from, to, color)| {
            (
                from[1],
                (to[0] - VALUE_X) / (width - VALUE_END - VALUE_X),
                *color,
            )
        })
        .collect();
    let under = |line: usize| top(line + 1) - VALUE_Y;
    assert_eq!(values.len(), 4);
    assert_eq!(values[0], (under(1), 1.0, VALUE_TRACK));
    assert_eq!(values[1].2, crate::chart::tone(Tone::Bad));
    assert!((values[1].1 - 0.82).abs() < 1e-5);
    assert_eq!(values[3].0, under(2));
    assert!((values[3].1 - 0.45).abs() < 1e-5);
    // A neutral row's value is drawn in the list's own blue.
    let mut neutral = panel.clone();
    neutral.rows[1].tone = Tone::Neutral;
    let shapes = super::shapes(
        &neutral.rows,
        &lines,
        (0, BODY_ROWS),
        COLUMNS,
        Marked::default(),
    );
    assert!(strokes_of(&shapes).contains(&VALUE_NEUTRAL));
    // None for a panel without rows.
    assert!(super::shapes(&[], &lines, (0, BODY_ROWS), COLUMNS, Marked::default()).is_empty());
}

impl super::Marked {
    fn hovering(row: usize) -> Self {
        Self {
            hovered: Some(row),
            chosen: None,
        }
    }
}

/// The colours of the lines in `shapes`.
fn strokes_of(shapes: &[Shape]) -> Vec<Rgba> {
    strokes(shapes)
        .into_iter()
        .map(|(.., color)| color)
        .collect()
}

#[test]
fn the_row_pointed_at_is_outlined_on_a_fainter_bar_and_the_row_just_chosen_on_a_brighter_one() {
    let panel = queue();
    let lines = lines(&panel, COLUMNS);
    let draw = |marked: Marked| {
        boxes(&shapes(
            &panel.rows,
            &lines,
            (0, BODY_ROWS),
            COLUMNS,
            marked,
        ))
    };
    // Pointed at, a row's bar is outlined; on the highlighted row, over its bar.
    assert_eq!(
        draw(Marked::hovering(2)),
        [
            (HIGHLIGHTED, top(1), None),
            (HOVERED, top(2), Some(HOVERED_WIDTH)),
        ]
    );
    assert_eq!(
        draw(Marked::hovering(1)),
        [
            (HIGHLIGHTED, top(1), None),
            (HOVERED, top(1), Some(HOVERED_WIDTH)),
        ]
    );
    // Just chosen, it is brighter, without its outline, still pointed at or not.
    let chosen = Marked {
        hovered: Some(2),
        chosen: Some(2),
    };
    assert_eq!(
        draw(chosen),
        [
            (HIGHLIGHTED, top(1), None),
            (CHOSEN, top(2), Some(CHOSEN_WIDTH))
        ]
    );
}

#[test]
fn a_row_keeps_its_marks_by_its_id_wherever_its_list_puts_it() {
    let marks = RowMarks {
        hovered: Some(("panel-1".into(), "pod9".into())),
        chosen: Some(("panel-1".into(), "pod3".into())),
    };
    let panel = queue();
    assert_eq!(
        marks.of(&panel),
        Marked {
            hovered: Some(2),
            chosen: Some(3)
        }
    );
    // Reordered, the marks follow the rows; gone, a row is marked nowhere; and another panel's rows are not marked.
    let mut reordered = panel.clone();
    reordered.rows.reverse();
    assert_eq!(
        marks.of(&reordered),
        Marked {
            hovered: Some(1),
            chosen: Some(0)
        }
    );
    reordered.rows.retain(|row| row.id != "pod9");
    assert_eq!(marks.of(&reordered).hovered, None);
    let mut other = panel;
    other.id = "panel-2".into();
    assert_eq!(marks.of(&other), Marked::default());
    assert_eq!(RowMarks::default().of(&queue()), Marked::default());
}

/// A list of `count` rows, `row-0` first, each its own line, in the order `order` gives them.
fn long(order: impl Iterator<Item = usize>) -> Panel {
    let order: Vec<usize> = order.collect();
    let body = order
        .iter()
        .map(|row| format!("- row {row}"))
        .collect::<Vec<_>>()
        .join("\n");
    Panel::new(
        "panel-1",
        "Queue",
        body,
        Place::Wall {
            wall: "wall-1".into(),
            slot: 0,
        },
    )
    .with_rows(
        order
            .iter()
            .map(|row| ListRow::new(format!("row-{row}"), true))
            .collect(),
    )
}

#[test]
fn a_list_longer_than_its_face_says_which_of_its_rows_show() {
    let panel = long(0..30);
    let lines = lines(&panel, COLUMNS);
    assert_eq!(
        position(&panel.rows, &lines, (0, BODY_ROWS)).as_deref(),
        Some("rows 1–12 of 30")
    );
    assert_eq!(
        position(&panel.rows, &lines, (18, BODY_ROWS)).as_deref(),
        Some("rows 19–30 of 30")
    );
    // A list that shows whole says nothing, nor does a panel that is not a list.
    let short = long(0..5);
    assert_eq!(
        position(&short.rows, &super::lines(&short, COLUMNS), (0, BODY_ROWS)),
        None
    );
    assert_eq!(position(&[], &lines, (0, BODY_ROWS)), None);
    // The face says it after its metadata, above the scroll bar the body already had, and as it scrolls.
    let face = |scroll: usize| {
        text_face(
            &panel,
            (COLUMNS, BODY_ROWS),
            &lines,
            (scroll, Marked::default()),
            None,
        )
    };
    let about = |canvas: &crate::canvas::Canvas| canvas.plain().lines().nth(1).unwrap().to_owned();
    assert!(
        about(&face(0)).starts_with("rows 1–12 of 30 · "),
        "{}",
        about(&face(0))
    );
    assert!(about(&face(5)).starts_with("rows 6–17 of 30 · "));
    assert!(about(&face(100)).starts_with("rows 19–30 of 30 · "));
    assert!(
        face(5)
            .shapes
            .iter()
            .any(|shape| matches!(shape, Shape::Bar { .. }))
    );
    // A row that wraps counts once, by the rows at the face's top and bottom; a line of no row says nothing.
    let mut wrapped = panel.clone();
    wrapped.body = wrapped.body.replacen("row 0", &"word ".repeat(12), 1);
    let lines = super::lines(&wrapped, COLUMNS);
    assert_eq!(
        position(&wrapped.rows, &lines, (1, BODY_ROWS)).as_deref(),
        Some("rows 1–12 of 30")
    );
    let mut prose = panel.clone();
    prose.body = format!("Before the list.\n\n{}", prose.body);
    let lines = super::lines(&prose, COLUMNS);
    assert_eq!(position(&prose.rows, &lines, (0, BODY_ROWS)), None);
    // Nor does a face whose last line is of no row.
    let mut after = panel.clone();
    after.body = format!("{}\n\nAfter the list.", after.body);
    let lines = super::lines(&after, COLUMNS);
    let bottom = lines.len() - BODY_ROWS;
    assert_eq!(position(&after.rows, &lines, (bottom, BODY_ROWS)), None);
}

#[test]
fn a_list_scrolled_is_anchored_on_its_top_row_by_its_id() {
    let panel = long(0..30);
    let lines = lines(&panel, COLUMNS);
    // At its top, it stays at its top as rows come and go; scrolled, it is anchored on the row at its top.
    assert_eq!(anchor(&panel.rows, &lines, 0.2), None);
    assert_eq!(
        anchor(&panel.rows, &lines, 7.25),
        Some(("row-7".to_owned(), 0.25))
    );
    // Reversed, that row is 22 lines down; gone, there is nowhere to scroll to.
    let reversed = long((0..30).rev());
    let reordered = super::lines(&reversed, COLUMNS);
    let anchored_at = anchored(&reversed.rows, &reordered, &("row-7".to_owned(), 0.25));
    assert_eq!(anchored_at, Some(22.25));
    assert_eq!(
        anchored(&reversed.rows, &reordered, &("row-99".to_owned(), 0.0)),
        None
    );
    // Past its lines, or on a line of no row, it has no anchor.
    assert_eq!(anchor(&panel.rows, &lines, 99.0), None);
    let mut prose = panel.clone();
    prose.body = format!("Before.\n\nAlso before.\n\n{}", prose.body);
    let lines = super::lines(&prose, COLUMNS);
    assert_eq!(anchor(&prose.rows, &lines, 1.0), None);
    let mut fewer = panel.clone();
    fewer.rows.truncate(3);
    assert_eq!(
        anchor(&fewer.rows, &super::lines(&panel, COLUMNS), 7.0),
        None
    );
    // A row the list has, but whose line shows nothing of it, is nowhere to scroll to.
    assert_eq!(anchored(&panel.rows, &[], &("row-7".to_owned(), 0.0)), None);
}

/// What the projector needs to show `projection`, scrolling `scroll`.
fn input<'a>(projection: &'a Projection, scroll: Option<(&'a str, f32)>) -> Input<'a> {
    Input {
        projection: Some(projection),
        ray: None,
        grip: None,
        viewer: Some(Vec3::new(0.0, 1.6, 0.0)),
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        free: false,
        controls: Controls::Controllers,
        scroll,
        rights: editing::Rights::default(),
        now: std::time::Instant::now(),
        dt: 1.0 / 72.0,
    }
}

/// `panel` on a wall ahead.
fn shown(panel: Panel) -> Projection {
    let pose = Pose::upright_facing([0.0, 1.5, -3.0], [0.0, 1.5, 0.0]);
    Projection::new("s", "sky")
        .with_walls(vec![tomori_protocol::Wall::new(
            "wall-1",
            tomori_protocol::WallKind::Added,
            pose,
        )])
        .with_panels(vec![panel])
}

/// The face of the one panel `projector` drew, as text.
fn face_of(projector: &mut Projector, projection: &Projection) -> String {
    let output = projector.update(input(projection, None));
    output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref())
        .map(crate::canvas::Canvas::plain)
        .find(|face| face.starts_with("Queue"))
        .expect("the list's face")
}

#[test]
fn a_list_reordered_keeps_the_row_its_participant_scrolled_to_at_the_top_of_its_face() {
    let mut projector = Projector::default();
    let before = shown(long(0..30));
    projector.update(input(&before, None));
    projector.update(input(&before, Some(("panel-1", 7.0))));
    assert_eq!(projector.scroll("panel-1"), 7);
    // The environment reverses its list, at a new version: row 7 stands at the top still, now 22 lines down.
    let after = shown(long((0..30).rev()).with_version(2));
    let face = face_of(&mut projector, &after);
    assert_eq!(projector.scroll("panel-1"), 18);
    assert!(face.contains("rows 19–30 of 30"), "{face}");
    let reversed = shown(long((0..30).rev()).with_version(3));
    projector.update(input(&reversed, Some(("panel-1", -10.0))));
    let mut moved = long((0..30).rev()).with_version(4);
    moved.rows.rotate_left(1);
    moved.body = moved
        .rows
        .iter()
        .map(|row| format!("- {}", row.id.replace('-', " ")))
        .collect::<Vec<_>>()
        .join("\n");
    let face = face_of(&mut projector, &shown(moved));
    assert_eq!(projector.scroll("panel-1"), 7);
    assert!(face.lines().nth(3).unwrap().contains("row 21"), "{face}");
    // Its anchor's row gone, it stays where it was; at its top, it stays at its top.
    let mut gone = long(1..30).with_version(5);
    gone.rows.retain(|row| row.id != "row-21");
    gone.body = gone
        .rows
        .iter()
        .map(|row| format!("- {}", row.id.replace('-', " ")))
        .collect::<Vec<_>>()
        .join("\n");
    face_of(&mut projector, &shown(gone.clone()));
    assert_eq!(projector.scroll("panel-1"), 7);
    projector.update(input(&shown(gone.clone()), Some(("panel-1", -100.0))));
    face_of(&mut projector, &shown(long((0..30).rev()).with_version(6)));
    assert_eq!(projector.scroll("panel-1"), 0);
}

#[test]
fn the_projector_draws_the_rows_its_participant_marks_once_until_they_change() {
    let mut projector = Projector::default();
    let projection = shown(long(0..5).with_version(1));
    projector.update(input(&projection, None));
    let drawn = faces::COUNTS.with(std::cell::Cell::get).1;
    projector.update(input(&projection, None));
    assert_eq!(faces::COUNTS.with(std::cell::Cell::get).1, drawn);
    let shapes = |projector: &mut Projector| {
        let output = projector.update(input(&projection, None));
        output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .find(|canvas| canvas.plain().starts_with("Queue"))
            .map(|canvas| boxes(&canvas.shapes))
            .unwrap()
    };
    projector.mark_rows(RowMarks {
        hovered: Some(("panel-1".into(), "row-2".into())),
        chosen: None,
    });
    assert_eq!(
        shapes(&mut projector),
        [(HOVERED, top(2), Some(HOVERED_WIDTH))]
    );
    let drawn = faces::COUNTS.with(std::cell::Cell::get).1;
    shapes(&mut projector);
    assert_eq!(faces::COUNTS.with(std::cell::Cell::get).1, drawn);
    projector.mark_rows(RowMarks {
        hovered: None,
        chosen: Some(("panel-1".into(), "row-2".into())),
    });
    assert_eq!(
        shapes(&mut projector),
        [(CHOSEN, top(2), Some(CHOSEN_WIDTH))]
    );
    projector.mark_rows(RowMarks::default());
    assert!(shapes(&mut projector).is_empty());
    // Its face in its slot is drawn as any panel's is, and as a still picture draws it, unmarked.
    let still = crate::projection::face(&long(0..5), false, false, Fit::SLOT);
    assert!(still.plain().starts_with("Queue"));
}

#[test]
fn a_still_picture_of_a_list_draws_the_rows_it_names_as_pointed_at_and_chosen() {
    let panel = queue();
    let still = |marks: (Option<&str>, Option<&str>)| {
        boxes(&crate::projection::list_face(&panel, Fit::SLOT, 0, marks).shapes)
    };
    assert_eq!(still((None, None)), [(HIGHLIGHTED, top(1), None)]);
    assert_eq!(
        still((Some("pod9"), Some("pod3"))),
        [
            (HIGHLIGHTED, top(1), None),
            (HOVERED, top(2), Some(HOVERED_WIDTH)),
            (CHOSEN, top(3), Some(CHOSEN_WIDTH)),
        ]
    );
    // A row it does not have is marked nowhere.
    assert_eq!(still((Some("pod99"), None)), [(HIGHLIGHTED, top(1), None)]);
}

/// The largest lists there can be, 32 of 30 rows, every row in a tone with a value, one highlighted, as an environment
/// puts them up together.
fn largest_lists(version: u64) -> Projection {
    let tones = [Tone::Neutral, Tone::Good, Tone::Warn, Tone::Bad];
    let panels = (0..32)
        .map(|index| {
            let mut panel = long(0..30).with_version(version);
            panel.id = format!("panel-{index}");
            panel.place = Place::Wall {
                wall: if index < 16 { "wall-1" } else { "wall-2" }.into(),
                slot: index % 16,
            };
            for (at, row) in panel.rows.iter_mut().enumerate() {
                *row = row
                    .clone()
                    .with_tone(tones[at % 4])
                    .with_value(at as f32 / 29.0)
                    .with_highlight(at == 3);
            }
            panel.body = panel.body.replacen("- row 3", "- **▸** row 3", 1);
            panel
        })
        .collect();
    let wall = |id: &str, x: f32| {
        let pose = Pose::upright_facing([x, 1.5, -3.0], [x, 1.5, 0.0]);
        tomori_protocol::Wall::new(id, tomori_protocol::WallKind::Added, pose)
            .with_grid(tomori_protocol::Grid::from((4, 4)))
    };
    Projection::new("s", "sky")
        .with_walls(vec![wall("wall-1", -3.0), wall("wall-2", 3.0)])
        .with_panels(panels)
}

/// Drawn as rows, the largest lists are laid out within the frame's budget, as many a frame as any panel, and each
/// face's rows are a few shapes each, so that drawing them costs a frame no more than a text panel's lines do.
#[test]
fn the_largest_lists_are_laid_out_a_few_a_frame_with_a_few_shapes_a_row() {
    let shown = largest_lists(1);
    let mut projector = Projector::paced();
    let mut frames = 0;
    while projector.faces.len() < 32 {
        let (laid, _) = faces::COUNTS.with(std::cell::Cell::get);
        projector.update(input(&shown, None));
        frames += 1;
        assert!(faces::COUNTS.with(std::cell::Cell::get).0 - laid <= faces::ARRIVALS);
    }
    assert_eq!(frames, 32 / faces::ARRIVALS);
    let output = projector.update(input(&shown, None));
    let faces: Vec<_> = output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref())
        .filter(|canvas| canvas.plain().starts_with("Queue"))
        .collect();
    assert_eq!(faces.len(), 32);
    for face in faces {
        // A row's highlight, its strip and its value's track and fill, for as many rows as show, and the scroll bar.
        assert!(
            face.shapes.len() <= 4 * BODY_ROWS + 2,
            "{}",
            face.shapes.len()
        );
        assert!(face.plain().contains("rows 1–"), "{}", face.plain());
    }
    // Every list changing at once is laid out again a few a frame, as any panel is.
    let changed = largest_lists(2);
    let (laid, _) = faces::COUNTS.with(std::cell::Cell::get);
    projector.update(input(&changed, None));
    assert_eq!(
        faces::COUNTS.with(std::cell::Cell::get).0 - laid,
        faces::LAYOUTS
    );
}

#[test]
fn a_list_reordered_while_it_waits_its_turn_marks_the_row_named_where_it_is_drawn() {
    let before = long(0..5).with_version(1);
    let mut faces = faces::Faces::new(&before);
    let marks = RowMarks {
        hovered: None,
        chosen: Some(("panel-1".into(), "row-1".into())),
    };
    let chosen =
        |faces: &mut faces::Faces| boxes(&faces.face(Fit::SLOT, (0, &marks), [1.0; 4]).shapes);
    assert_eq!(chosen(&mut faces), [(CHOSEN, top(1), Some(CHOSEN_WIDTH))]);
    // Reversed, its turn not yet come: it still draws the rows as they were, and marks row 1 where it is drawn.
    let after = long((0..5).rev()).with_version(2);
    assert!(!faces.follow(&after, false, 0.0));
    let face = faces.face(Fit::SLOT, (0, &marks), [1.0; 4]);
    assert!(face.plain().lines().nth(3).unwrap().contains("row 0"));
    assert_eq!(chosen(&mut faces), [(CHOSEN, top(1), Some(CHOSEN_WIDTH))]);
    // Laid out, it marks row 1 where it now stands, fourth.
    assert!(faces.follow(&after, true, 0.0));
    assert_eq!(chosen(&mut faces), [(CHOSEN, top(3), Some(CHOSEN_WIDTH))]);
}

#[test]
fn a_change_of_only_rows_tones_and_values_draws_the_list_again_without_laying_it_out() {
    let mut projector = Projector::default();
    let shown = |panel: Panel| {
        let mut projection = largest_lists(1);
        projection.panels = vec![panel];
        projection
    };
    let mut panel = queue().with_version(1);
    panel.place = Place::Wall {
        wall: "wall-1".into(),
        slot: 0,
    };
    projector.update(input(&shown(panel.clone()), None));
    let counts = || faces::COUNTS.with(std::cell::Cell::get);
    let marks = |projector: &mut Projector, panel: &Panel| -> Vec<char> {
        let output = projector.update(input(&shown(panel.clone()), None));
        let face = output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .find(|canvas| canvas.plain().starts_with("Attention"))
            .unwrap();
        face.rows[3..7].iter().map(|row| row.cells[0].0).collect()
    };
    assert_eq!(marks(&mut projector, &panel), ['•', '✕', '!', '✓']);
    // A row turning bad, another neutral and a value moving, at a new version: drawn again, not laid out.
    let mut restyled = panel.clone().with_version(2);
    restyled.rows[0].tone = Tone::Bad;
    restyled.rows[3].tone = Tone::Neutral;
    restyled.rows[2].value = Some(0.9);
    let before = counts();
    assert_eq!(marks(&mut projector, &restyled), ['✕', '✕', '!', '•']);
    assert_eq!((counts().0, counts().1), (before.0, before.1 + 1));
    // The neutral row's bullet is as the body's list draws it.
    let fresh = super::lines(&restyled, COLUMNS);
    let mut kept = super::lines(&panel, COLUMNS);
    retone(&mut kept, &restyled.rows);
    assert_eq!(kept[3].cells, fresh[3].cells);
    // Anything else changing with them, such as the order, lays it out again.
    let mut reordered = restyled.clone().with_version(3);
    reordered.rows.swap(0, 2);
    let before = counts();
    marks(&mut projector, &reordered);
    assert_eq!(counts().0, before.0 + 1);
    // Rows only of the same ids, choosable and highlighted alike, and of a list, are restyled.
    assert!(!super::restyled(&panel, &panel.clone().with_version(9)));
    let mut fewer = restyled.clone();
    fewer.rows.pop();
    assert!(!super::restyled(&panel, &fewer));
    let mut title = restyled.clone();
    title.title = "Other".into();
    assert!(!super::restyled(&panel, &title));
    let mut none = panel.clone();
    none.rows.clear();
    assert!(!super::restyled(&none, &none.clone().with_version(9)));
}
