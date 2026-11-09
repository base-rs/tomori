// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::{Grid, Meta, TOMORI_WALL, WALL_SLOTS, WallShape};

/// How a curved wall's slots are sized unless it says otherwise.
const ARC_SCALE: f32 = layout::ARC_SCALE;

/// The middle of `slot` of a wall like `wall`, in its own frame.
fn slot_offset(wall: &Wall, slot: u32) -> Vec3 {
    Vec3::from_array(wall.slot_frames()[slot as usize].position)
}

/// A wall `distance` in front of home, facing it.
fn wall(id: &str, kind: WallKind, x: f32, distance: f32) -> Wall {
    Wall::new(
        id,
        kind,
        Pose::upright_facing([x, 1.5, -distance], [x, 1.6, 0.0]),
    )
}

fn on(wall: &str, slot: u32) -> Place {
    Place::Wall {
        wall: wall.into(),
        slot,
    }
}

fn panel(id: &str, place: Place) -> Panel {
    Panel::new(
        id,
        format!("Panel {id}"),
        "**3 crates** failed.\n\n- one\n- two",
        place,
    )
    .with_meta(vec![Meta::new("source", "CI")])
    .with_age(120)
}

fn projection(panels: Vec<Panel>) -> Projection {
    Projection::new("s", "sky")
        .with_walls(vec![
            wall(TOMORI_WALL, WallKind::Tomori, 0.0, 3.0),
            wall("wall-1", WallKind::Added, 4.0, 3.0),
        ])
        .with_panels(panels)
}

fn input(projection: &Projection, ray: Option<Ray>, grip: f32, now: Instant) -> Input<'_> {
    Input {
        projection: Some(projection),
        ray,
        grip: Some(grip),
        viewer: Some(HOME),
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        controls: Controls::Controllers,
        scroll: None,
        now,
        dt: 1.0 / 72.0,
    }
}

/// A ray from home at `point`.
fn at(point: Vec3) -> Option<Ray> {
    Ray::new(HOME, point - HOME)
}

/// Where the middle of `slot` of the Tomori wall is, in the scene.
fn slot_centre(projection: &Projection, slot: u32) -> Vec3 {
    let wall = projection.wall(TOMORI_WALL).unwrap();
    model(&wall.pose)
        .unwrap()
        .transform_point3(slot_offset(wall, slot))
}

#[test]
fn a_wall_holds_its_slots_in_three_columns_of_two_the_first_at_the_top_left() {
    let flat = wall(TOMORI_WALL, WallKind::Tomori, 0.0, 3.0);
    let (wall_width, wall_height) = wall_size(&flat);
    let (width, height) = panel_size();
    assert!((wall_width - 1.995).abs() < 0.001, "{wall_width}");
    assert!((wall_height - 0.878).abs() < 0.001, "{wall_height}");
    assert!((width - 0.604).abs() < 0.001 && (height - 0.361).abs() < 0.001);
    // A panel drawn here is the size of a slot as the server lays walls out.
    assert_eq!(panel_size(), layout::SLOT_SIZE);
    let offsets: Vec<Vec3> = (0..WALL_SLOTS)
        .map(|slot| slot_offset(&flat, slot))
        .collect();
    for (index, offset) in offsets.iter().enumerate() {
        // Within the wall, in front of it.
        assert!(offset.x.abs() + width / 2.0 < wall_width / 2.0);
        assert!(offset.y.abs() + height / 2.0 < wall_height / 2.0);
        assert_eq!(offset.z, LIFT);
        // Clear of every other slot.
        for other in &offsets[index + 1..] {
            assert!(
                (offset.x - other.x).abs() >= width || (offset.y - other.y).abs() >= height,
                "{offset} and {other} overlap"
            );
        }
    }
    // Read left to right, then down.
    assert!(offsets[0].x < offsets[1].x && offsets[1].x < offsets[2].x);
    assert!(offsets[0].y > offsets[3].y);
    assert_eq!(offsets[0].x, offsets[3].x);
    // The wall's face is laid out to match: its slots marked where the panels go.
    let face = wall_face(&wall(TOMORI_WALL, WallKind::Tomori, 0.0, 3.0));
    let (size_width, size_height) = face.size();
    assert!((size_width - wall_width).abs() < 1e-6 && (size_height - wall_height).abs() < 1e-6);
    assert_eq!(face.shapes.len(), WALL_SLOTS as usize);
    assert!(face.plain().starts_with("  TOMORI"));
    assert!(
        wall_face(&wall("w", WallKind::Environment, 0.0, 3.0))
            .plain()
            .starts_with("  WALL")
    );
}

#[test]
fn a_flat_wall_of_any_grid_draws_its_board_and_marks_its_slots_where_its_panels_stand() {
    for (columns, rows) in [(4, 1), (1, 1), (8, 3), (2, 4)] {
        let grid = Grid::new(columns, rows).unwrap();
        for scale in [0.5, 1.0, 1.5] {
            let wall = wall("wall-1", WallKind::Added, 0.0, 3.0)
                .with_grid(grid)
                .with_scale(scale);
            let face = wall_face(&wall);
            // Its face is its board, drawn at its scale.
            let (width, height) = face.size();
            let (board_width, board_height) = wall.board().unwrap();
            assert!((width * scale - board_width).abs() < 1e-5);
            assert!((height * scale - board_height).abs() < 1e-5);
            assert_eq!(face.shapes.len() as u32, grid.slots());
            // Each slot marked where a panel in it stands, in the face's own pixels from its top left.
            let frames = slot_frames(&wall, Mat4::IDENTITY);
            for (shape, frame) in face.shapes.iter().zip(&frames) {
                let middle = Vec3::new(
                    (shape.x + shape.width / 2.0 + WALL_PADDING.0 as f32) * PIXEL - width / 2.0,
                    height / 2.0 - (shape.y + shape.height / 2.0 + WALL_PADDING.1 as f32) * PIXEL,
                    0.0,
                ) * scale;
                let at = frame.w_axis.truncate();
                assert!((at.x - middle.x).abs() < 1e-4 && (at.y - middle.y).abs() < 1e-4);
                assert!((frame.x_axis.truncate().length() - scale).abs() < 1e-5);
            }
            // Its label fits, however narrow.
            assert!(face.plain().contains("WALL"));
        }
    }
}

#[test]
fn panels_stand_in_any_slot_of_a_walls_grid_and_none_past_its_last() {
    let wide = wall("wall-1", WallKind::Added, 0.0, 3.0).with_grid(Grid::new(4, 1).unwrap());
    let projection = Projection::new("s", "sky")
        .with_walls(vec![wide.clone()])
        .with_panels(vec![
            panel("a", on("wall-1", 3)),
            panel("b", on("wall-1", 4)),
        ]);
    let mut projector = Projector::default();
    let output = projector.update(input(&projection, None, 0.0, Instant::now()));
    let frame = model(&wide.pose).unwrap();
    let fourth = slot_frames(&wide, frame)[3];
    // The board and the panel in its fourth slot, and nothing for the slot past its last.
    assert_eq!(output.sheets.len(), 2);
    assert_eq!(output.sheets[1].model, Some(fourth));
}

#[test]
fn a_panel_shows_its_title_metadata_and_body_as_markdown_lite() {
    let shown = face(&panel("1", on(TOMORI_WALL, 0)), false, false, Fit::SLOT);
    let text = shown.plain();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), ROWS);
    assert_eq!(lines[0].trim_end(), "Panel 1");
    assert_eq!(lines[1].trim_end(), "source: CI · 2m");
    assert!(lines[2].starts_with('─'));
    assert_eq!(lines[3].trim_end(), "3 crates failed.");
    assert!(lines.iter().any(|line| line.trim_end() == "• one"));
    assert!(!text.contains("**"));
    assert_eq!(shown.border, Some(FRAME));
    assert_eq!(
        face(&panel("1", on(TOMORI_WALL, 0)), true, true, Fit::SLOT).border,
        Some(HIGHLIGHT)
    );
    assert_eq!(
        face(&panel("1", on(TOMORI_WALL, 0)), false, true, Fit::SLOT).border,
        Some(NEWEST)
    );
    // A body longer than the panel ends with "…" on its last row, and nothing is drawn past the panel's columns.
    let long = Panel::new(
        "2",
        "",
        (0..40)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Place::Unknown,
    );
    let text = face(&long, false, false, Fit::SLOT).plain();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), ROWS);
    assert!(lines[ROWS - 1].trim_end().ends_with('…'), "{text}");
    assert!(
        lines
            .iter()
            .all(|line| crate::ui_text::width(line) <= COLUMNS)
    );
    // Untitled, it is named by the start of its body.
    assert_eq!(name(&long), "line 0");
    let heading = Panel::new("3", "", "\n## Steps\n1. Build", Place::Unknown);
    assert_eq!(name(&heading), "Steps");
    assert_eq!(name(&Panel::new("4", "", "", Place::Unknown)), "");
}

/// A checklist the agent calls `agenda`, some of its items ticked.
#[test]
fn a_panel_in_a_tall_or_wide_slot_shows_more_rows_or_columns_at_its_text_s_size_and_fills_it() {
    let shaped = |aspect: f32| {
        wall("wall-1", WallKind::Added, 0.0, 3.0)
            .with_grid(Grid::from((3, 1)))
            .with_scale(1.0)
            .with_aspect(Some(aspect))
    };
    assert_eq!(
        Cells::of(&wall("w", WallKind::Added, 0.0, 3.0)),
        Cells::STANDARD
    );
    assert_eq!(
        Cells::of(
            &wall(TOMORI_WALL, WallKind::Tomori, 0.0, 3.0)
                .with_shape(WallShape::Arc { radius: 2.3 })
        ),
        Cells::STANDARD
    );
    let tall = Cells::of(&shaped(9.0 / 16.0));
    let wide = Cells::of(&shaped(3.0));
    // Tall: its text wraps at a standard slot's width and it holds nearly three times the rows; wide, the reverse.
    assert_eq!((tall.columns, tall.code_columns), (COLUMNS, CODE_COLUMNS));
    assert!(tall.body >= 40 && tall.code_rows >= 50, "{tall:?}");
    assert_eq!((wide.body, wide.code_rows), (BODY_ROWS, SLOT_CODE_ROWS));
    assert!(wide.columns >= 60 && wide.code_columns >= 85, "{wide:?}");
    // Each face fits within its slot and fills it to within a cell, whatever its shape.
    for aspect in [0.25, 9.0 / 16.0, 0.75, 1.0, 16.0 / 9.0, 3.0, 4.0] {
        let wall = shaped(aspect);
        let fit = Fit::Slot(Cells::of(&wall));
        let (width, height) = slot_unit(&wall);
        let mut code = code_panel("c", 3);
        code.place = on("wall-1", 0);
        for panel in [panel("t", on("wall-1", 0)), code] {
            let (face_width, face_height) = fitted_size(&panel, fit);
            let drawn = face(&panel, false, false, fit).size();
            assert!(
                face_width <= width + 1e-4 && face_height <= height + 1e-4,
                "{aspect}"
            );
            assert!(
                width - face_width < 10.0 * PIXEL && height - face_height < 13.0 * PIXEL,
                "{aspect}: {face_width}x{face_height} in {width}x{height}"
            );
            // A code panel's face is drawn in its own smaller cells, within one of them of its slot's.
            let cell = 12.0 * CODE_PIXEL;
            assert!(
                (drawn.0 - face_width).abs() < cell && (drawn.1 - face_height).abs() < cell,
                "{aspect}: {drawn:?}"
            );
        }
    }
    // A long list shows more of itself in a tall slot, cut with "…" only where it ends.
    let mut long = panel("t", on("wall-1", 0));
    long.body = (1..=30)
        .map(|item| format!("- item {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let standard = face(&long, false, false, Fit::SLOT).plain();
    let taller = face(&long, false, false, Fit::Slot(tall)).plain();
    assert!(standard.contains('…') && !standard.contains("item 30"));
    assert!(
        !taller.contains('…') && taller.contains("item 30"),
        "{taller}"
    );
    // A long line of code wraps on fewer rows in a wide slot.
    let mut code = code_panel("c", 1);
    code.body = format!(
        "let total = {};",
        (0..30)
            .map(|n| format!("value_{n}"))
            .collect::<Vec<_>>()
            .join(" + ")
    );
    code.code = Some(Code::new("rust", 1));
    let rows = |columns: usize| {
        CodeLayout::new(&code, code.code.as_ref().unwrap(), columns)
            .rows
            .len()
    };
    assert!(rows(wide.code_columns) < rows(CODE_COLUMNS));
    // Panels on such a wall stand in its slots, at their size, and are pointed at across them.
    let projection = Projection::new("s", "sky")
        .with_walls(vec![shaped(9.0 / 16.0)])
        .with_panels(vec![long.clone()]);
    let mut projector = Projector::default();
    let frame = model(&projection.walls[0].pose).unwrap();
    let middle = frame.transform_point3(slot_offset(&projection.walls[0], 0));
    let (_, height) = projection.walls[0].slot_size();
    // Near the bottom of the tall slot, below where a standard slot would end.
    let low = middle - Vec3::Y * (height / 2.0 - 0.05);
    let output = projector.update(input(&projection, at(low), 0.0, Instant::now()));
    assert_eq!(
        output.pointed.as_ref().map(|(id, ..)| id.as_str()),
        Some("t")
    );
    assert_eq!(
        projector
            .stands(&long, &[(&projection.walls[0], frame)])
            .map(|(_, fit)| fit),
        Some(Fit::Slot(tall))
    );
    // A code panel on a wall of wide slots is laid out at as many columns as they hold, and kept so.
    code.place = on("wall-1", 0);
    let projection = Projection::new("s", "sky")
        .with_walls(vec![shaped(3.0)])
        .with_panels(vec![code.clone()]);
    let mut projector = Projector::default();
    projector.update(input(&projection, None, 0.0, Instant::now()));
    let laid = projector.layout("c", Fit::Slot(wide)).unwrap();
    assert_eq!(laid.columns, wide.code_columns);
    assert_eq!(
        projector.layout("c", Fit::Text).unwrap().columns,
        CODE_COLUMNS
    );
    projector.update(input(&projection, None, 0.0, Instant::now()));
    assert_eq!(projector.layouts["c"].layouts.len(), 2);
}

fn checklist(id: &str, place: Place) -> Panel {
    Panel::new(
        id,
        "Agenda",
        "To cover:\n\n- [x] Panel ids\n- [ ] Decide the pairing window length before the next release\n  - [ ] Nested\n\nThen questions.",
        place,
    )
    .with_slug(Some("agenda".into()))
}

#[test]
fn a_checklist_shows_its_boxes_and_the_ray_names_the_item_it_meets() {
    let text = face(&checklist("1", on(TOMORI_WALL, 0)), false, false, Fit::SLOT).plain();
    let lines: Vec<&str> = text.split('\n').map(str::trim_end).collect();
    assert_eq!(lines[3], "To cover:");
    assert_eq!(lines[4], "☑ Panel ids");
    assert_eq!(lines[5], "☐ Decide the pairing window length");
    assert_eq!(lines[6], "  before the next release");
    assert_eq!(lines[7], "  ☐ Nested");
    for (place, fit) in [
        (on(TOMORI_WALL, 0), Fit::SLOT),
        (
            Place::Free {
                pose: Pose::upright_facing([0.5, 1.5, -1.5], [0.0, 1.6, 0.0]),
            },
            Fit::Text,
        ),
    ] {
        let panel = checklist("agenda", place.clone());
        let shown = projection(vec![panel.clone()]);
        let mut projector = Projector::default();
        let output = projector.update(input(&shown, None, 0.0, Instant::now()));
        assert!(output.item.is_none());
        let model = match &place {
            Place::Wall { slot, .. } => slot_frames(
                shown.wall(TOMORI_WALL).unwrap(),
                model(&shown.wall(TOMORI_WALL).unwrap().pose).unwrap(),
            )[*slot as usize],
            Place::Free { pose } => model(pose).unwrap(),
            _ => unreachable!("placed on a wall or floating"),
        };
        let (columns, body) = self::fit(&panel, fit);
        let (_, height) = sized(columns, body);
        // The middle of each row, from the title down, a little left of the middle.
        let mut item_on = |row: usize| {
            let y =
                height / 2.0 - (PADDING.1 as f32 + (row as f32 + 0.5) * CELL_HEIGHT as f32) * PIXEL;
            let point = model.transform_point3(Vec3::new(-0.05, y, 0.0));
            let output = projector.update(input(&shown, at(point), 0.0, Instant::now()));
            assert_eq!(output.pointed.as_ref().unwrap().0, "agenda");
            assert_eq!(
                output.item,
                item_at(&panel, model, fit, 0, output.pointed.unwrap().2)
            );
            output.item
        };
        // Not on the title, the metadata, the rule, nor a row of no item; each row of an item, wrapped or nested, names
        // the line it begins on.
        assert_eq!(
            (0..8).map(&mut item_on).collect::<Vec<_>>(),
            [None, None, None, None, Some(2), Some(3), Some(3), Some(4)],
            "{fit:?}"
        );
        assert_eq!(item_on(8), None, "{fit:?}");
    }
    // Scrolled, the row under the ray is the one scrolled to: the first body row, scrolled 10 rows down, is the 11th item.
    let items: Vec<String> = (0..30).map(|item| format!("- [ ] Item {item}")).collect();
    let long = Panel::new("long", "Long", items.join("\n"), on(TOMORI_WALL, 0));
    let (columns, body) = self::fit(&long, Fit::SLOT);
    let (_, height) = sized(columns, body);
    let first = Vec3::new(
        -0.05,
        height / 2.0 - (PADDING.1 as f32 + 3.5 * CELL_HEIGHT as f32) * PIXEL,
        0.0,
    );
    assert_eq!(item_at(&long, Mat4::IDENTITY, Fit::SLOT, 0, first), Some(0));
    assert_eq!(
        item_at(&long, Mat4::IDENTITY, Fit::SLOT, 10, first),
        Some(10)
    );
    // A code panel names no item.
    let code = code_panel("c", 4);
    assert_eq!(
        item_at(&code, Mat4::IDENTITY, Fit::SLOT, 0, Vec3::ZERO),
        None
    );
    // Off the panel, above it, none either.
    assert_eq!(
        item_at(
            &checklist("1", Place::Unknown),
            Mat4::IDENTITY,
            Fit::SLOT,
            0,
            Vec3::new(0.0, 1.0, 0.0)
        ),
        None
    );
}

/// A code panel of Rust, its block `lines` long, with a keyword and a function highlighted on its first line.
fn code_panel(id: &str, lines: u32) -> Panel {
    use tomori_protocol::Span;
    Panel::new(
        id,
        "rust",
        "fn main() {\n    let x = \"日本\";\n}",
        on(TOMORI_WALL, 1),
    )
    .with_code(Code::new("rust", lines).with_spans(vec![
        Span::new(0, 2, TokenKind::Keyword),
        Span::new(3, 4, TokenKind::Function),
        Span::new(24, 4, TokenKind::String),
    ]))
    .with_age(300)
}

/// The colours of the cells of `row` that hold `text`, where it first is.
fn colours(row: &crate::ui_text::Row, text: &str) -> Vec<crate::ui_text::Rgba> {
    let plain: String = row.cells.iter().map(|(c, _)| *c).collect();
    let start = plain.find(text).expect("the row holds the text");
    let start = plain[..start].chars().count();
    row.cells[start..start + text.chars().count()]
        .iter()
        .map(|(_, style)| style.color)
        .collect()
}

#[test]
fn a_code_panel_shows_its_lines_numbered_and_coloured_by_kind() {
    let canvas = face(&code_panel("c", 3), false, false, Fit::SLOT);
    // The same size as a text panel, at a finer pixel, so that it fits its slot.
    let (width, height) = canvas.size();
    let (panel_width, panel_height) = panel_size();
    assert!((width - panel_width).abs() < 1e-4, "{width}");
    assert!((height - panel_height).abs() < 0.002, "{height}");
    assert_eq!(canvas.pixel, CODE_PIXEL);
    let text = canvas.plain();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.len(), CODE_ROWS);
    assert_eq!(lines[0].trim_end(), "rust");
    assert_eq!(lines[1].trim_end(), "rust · 3 lines · 5m");
    assert!(lines[2].starts_with('─'));
    assert_eq!(lines[3].trim_end(), " 1 fn main() {");
    assert_eq!(lines[4].trim_end(), " 2     let x = \"日本\";");
    assert_eq!(lines[5].trim_end(), " 3 }");
    // Nothing more to say past the block's end.
    assert!(lines[CODE_ROWS - 1].trim().is_empty());
    // The line numbers dim, and each span in its kind's colour, the rest plain.
    let rows = &canvas.rows;
    assert_eq!(colours(&rows[3], " 1"), [DIM; 2]);
    assert_eq!(colours(&rows[3], "fn"), [palette(TokenKind::Keyword); 2]);
    assert_eq!(colours(&rows[3], "main"), [palette(TokenKind::Function); 4]);
    assert_eq!(colours(&rows[3], "()"), [TEXT; 2]);
    // A wide character takes two cells, both its kind's colour.
    let wide = format!("\"日{}", crate::ui_text::WIDE);
    assert_eq!(colours(&rows[4], &wide), [palette(TokenKind::String); 3]);
    assert!(crate::ui_text::whole(&rows[4].cells));
    // Highlighted or newest, it is framed as a text panel is.
    assert_eq!(
        face(&code_panel("c", 3), true, false, Fit::SLOT).border,
        Some(HIGHLIGHT)
    );
    assert_eq!(
        face(&code_panel("c", 3), false, true, Fit::SLOT).border,
        Some(NEWEST)
    );
}

/// The text of `row`'s cells, without the second cells of wide characters.
fn row_text(row: &crate::ui_text::Row) -> String {
    crate::ui_text::plain(&row.cells)
}

#[test]
fn a_long_line_of_code_wraps_under_a_hanging_indent_with_its_highlighting() {
    use tomori_protocol::Span;
    // Indented by four, as the block's other lines are, and longer than the panel's 45 columns of code.
    let line = "    let total = compute(alpha, beta, \"gamma delta\", epsilon);";
    let body = format!("fn main() {{\n{line}\n    short();\n}}");
    let start = body.find("\"gamma").unwrap();
    let start = body[..start].chars().count() as u32;
    let panel = Panel::new("c", "rust", body, Place::Unknown).with_code(
        Code::new("rust", 4).with_spans(vec![
            Span::new(0, 2, TokenKind::Keyword),
            Span::new(start, 13, TokenKind::String),
        ]),
    );
    let canvas = face(&panel, false, false, Fit::SLOT);
    let rows: Vec<String> = canvas.rows.iter().map(row_text).collect();
    // Lines that fit are as they were.
    assert_eq!(rows[3], " 1 fn main() {");
    // The long line breaks at a space within the row, its number on the first row only, and continues under a
    // hanging indent of one more step, marked as continued in the gutter, without the space it broke at.
    assert_eq!(rows[4], " 2     let total = compute(alpha, beta, \"gamma");
    assert_eq!(rows[5], " ↪         delta\", epsilon);");
    assert!(
        rows.iter()
            .all(|row| crate::ui_text::width(row) <= CODE_COLUMNS)
    );
    // The marker is dim, and the string's colour carries across the break.
    assert_eq!(colours(&canvas.rows[5], "↪"), [CONTINUED_COLOR]);
    assert_eq!(
        colours(&canvas.rows[4], "\"gamma"),
        [palette(TokenKind::String); 6]
    );
    assert_eq!(
        colours(&canvas.rows[5], "delta\""),
        [palette(TokenKind::String); 6]
    );
    assert_eq!(colours(&canvas.rows[5], ", epsilon"), [TEXT; 9]);
    // The lines after it follow, numbered as before.
    assert_eq!(rows[6], " 3     short();");
    assert_eq!(rows[7], " 4 }");
}

/// The block the maintainer looked at on the Quest 3, whose sixth line showed cut with "…" by the server.
const MAINTAINERS_BLOCK: &str = r#"use std::collections::HashMap;

fn summarise(orders: &[(String, u32, f64)]) -> HashMap<String, (u32, f64)> {
    let mut totals = HashMap::new();
    for (customer, quantity, price) in orders {
        let entry = totals.entry(customer.clone()).or_insert((0, 0.0));
        entry.0 += quantity;
        entry.1 += *quantity as f64 * price; // running total of what each customer has spent across every order so far
    }
    totals
}"#;

/// The rows of code of `canvas`, as drawn, without its header and its last row.
fn code_rows(canvas: &Canvas) -> Vec<String> {
    canvas.rows[3..CODE_ROWS - 1].iter().map(row_text).collect()
}

/// Each line of code `rows` show: its number, and the code of each of its rows, without the gutter, and the later rows
/// without their hanging indent. Rows scrolled to partway through a line are of a line numbered 0.
fn joined(rows: &[String]) -> Vec<(usize, Vec<String>)> {
    let mut lines: Vec<(usize, Vec<String>)> = Vec::new();
    for row in rows.iter().filter(|row| !row.is_empty()) {
        let (gutter, code) = row.split_at(
            row.char_indices()
                .nth(GUTTER)
                .map_or(row.len(), |(at, _)| at),
        );
        match (gutter.trim(), lines.last_mut()) {
            ("↪", Some(last)) => last.1.push(code.trim_start().to_owned()),
            ("↪", None) => lines.push((0, vec![code.trim_start().to_owned()])),
            (number, _) => lines.push((number.parse().unwrap(), vec![code.to_owned()])),
        }
    }
    lines
}

#[test]
fn the_maintainers_block_wraps_every_line_whole_onto_as_many_rows_as_it_needs() {
    use tomori_protocol::Span;
    let comment = MAINTAINERS_BLOCK.find("// running").unwrap();
    let start = MAINTAINERS_BLOCK[..comment].chars().count() as u32;
    let length = MAINTAINERS_BLOCK[comment..].find('\n').unwrap() as u32;
    let panel = Panel::new("c", "rust", MAINTAINERS_BLOCK, on(TOMORI_WALL, 0)).with_code(
        Code::new("rust", 11).with_spans(vec![Span::new(start, length, TokenKind::Comment)]),
    );
    // In a slot, as on the headset.
    let canvas = face(&panel, false, false, Fit::SLOT);
    let rows = code_rows(&canvas);
    assert_eq!(
        rows,
        [
            " 1 use std::collections::HashMap;",
            " 2 ",
            " 3 fn summarise(orders: &[(String, u32, f64)])",
            " ↪     -> HashMap<String, (u32, f64)> {",
            " 4     let mut totals = HashMap::new();",
            " 5     for (customer, quantity, price) in",
            " ↪         orders {",
            " 6         let entry = totals.entry(customer.",
            " ↪             clone()).or_insert((0, 0.0));",
            " 7         entry.0 += quantity;",
            " 8         entry.1 += *quantity as f64 * price;",
            " ↪             // running total of what each",
            " ↪             customer has spent across every",
            " ↪             order so far",
            " 9     }",
            "10     totals",
        ]
    );
    // Nothing is cut mid-line: every line shown is whole, and the last, which no longer fits, is counted.
    assert!(rows.iter().all(|row| !row.contains('…')), "{rows:#?}");
    let source: Vec<&str> = MAINTAINERS_BLOCK.lines().collect();
    for (number, parts) in joined(&rows) {
        let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(
            words(&parts.join(" ")).replace(". clone", ".clone"),
            words(source[number - 1])
        );
    }
    assert_eq!(row_text(&canvas.rows[CODE_ROWS - 1]), "↓ 1 more");
    // Every row, its gutter and hanging indent counted, is within the panel's columns, and the comment's colour carries
    // across each of its rows, the marker dim.
    assert!(
        canvas
            .rows
            .iter()
            .all(|row| row.cells.len() <= CODE_COLUMNS)
    );
    for row in &canvas.rows[3 + 11..3 + 14] {
        assert_eq!(colours(row, "↪"), [CONTINUED_COLOR]);
        let code = row_text(row);
        let text = code.trim_start_matches([' ', '↪']);
        assert!(
            colours(row, text)
                .iter()
                .all(|colour| *colour == palette(TokenKind::Comment))
        );
    }
}

#[test]
fn a_line_of_code_wraps_onto_as_many_rows_as_it_needs_within_the_panel_s_columns() {
    // Words, then a token wider than a whole row, which breaks hard at each row's end rather than being cut.
    let words = vec!["word"; 40].join(" ");
    let token = "t".repeat(100);
    let line = format!("    call({words}, {token});");
    let panel = Panel::new(
        "c",
        "rust",
        format!(
            "fn f() {{
{line}
}}"
        ),
        Place::Unknown,
    )
    .with_code(Code::new("rust", 3));
    let canvas = face(&panel, false, false, Fit::SLOT);
    let rows = code_rows(&canvas);
    let lines = joined(&rows);
    assert_eq!(
        lines.iter().map(|(number, _)| *number).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    // The long line takes many rows, each after the first marked and hanging one step under it, its gutter and
    // indent counted in the panel's columns, and every character of it is shown, none cut with "…".
    let parts = &lines[1].1;
    assert!(parts.len() >= 7, "{rows:#?}");
    let later: Vec<&String> = rows.iter().filter(|row| row.starts_with(" ↪ ")).collect();
    assert_eq!(later.len(), parts.len() - 1);
    assert!(
        later
            .iter()
            .all(|row| row.starts_with(&format!(" ↪ {}", " ".repeat(8))))
    );
    assert!(
        later
            .iter()
            .all(|row| !row[" ↪ ".len()..].starts_with(&" ".repeat(9)))
    );
    assert!(
        rows.iter()
            .all(|row| crate::ui_text::width(row) <= CODE_COLUMNS)
    );
    assert!(
        rows.iter()
            .any(|row| crate::ui_text::width(row) == CODE_COLUMNS)
    );
    assert!(!canvas.plain().contains('…'));
    assert_eq!(
        parts.concat().replace(' ', ""),
        line.replace(' ', ""),
        "{rows:#?}"
    );
}

#[test]
fn where_the_server_cuts_code_is_never_shown_but_as_the_panel_running_out() {
    // The server cuts a code panel's lines where they reach as many characters together as its rows can show.
    // Lines cut so, however long and however many, and however deeply indented: the "…" the cut ends with is never
    // drawn but as the first line's last cell, where it alone is too tall for the panel, and every line drawn is whole.
    for (lines, length, indent) in [
        (1, 2_000, 0),
        (1, 2_000, 12),
        (2, 400, 4),
        (3, 300, 8),
        (16, 60, 4),
        (16, 45, 0),
        (5, 200, 40),
    ] {
        let mut left = tomori_protocol::CODE_CHARS;
        let mut body = Vec::new();
        for _ in 0..lines {
            if left == 0 {
                break;
            }
            let line = format!("{}{}", " ".repeat(indent), vec!["ab"; length].join(" "));
            let line: String = line.chars().take(length).collect();
            let kept: String = line.chars().take(left).collect();
            left -= kept.chars().count();
            let cut = kept.chars().count() < line.chars().count();
            body.push(if cut { format!("{kept}…") } else { kept });
            if cut {
                break;
            }
        }
        let shown = body.len();
        let cut = body.last().unwrap().ends_with('…');
        let panel = Panel::new("c", "text", body.join("\n"), Place::Unknown)
            .with_code(Code::new("", lines as u32 + u32::from(cut)));
        let canvas = face(&panel, false, false, Fit::SLOT);
        let rows = code_rows(&canvas);
        let drawn = joined(&rows);
        for (row, text) in rows.iter().enumerate() {
            assert!(
                !text.contains('…') || (drawn.len() == 1 && row == SLOT_CODE_ROWS - 1),
                "{lines} × {length}: {rows:#?}"
            );
        }
        if cut {
            assert!(drawn.len() < shown || shown == 1, "{rows:#?}");
        }
    }
}

#[test]
fn code_breaks_at_spaces_or_punctuation_and_hard_breaks_a_long_token_never_through_a_wide_character()
 {
    let cells = |text: &str| {
        cut(
            &[crate::ui_text::Span::new(text, Style::plain(TEXT))],
            usize::MAX,
        )
    };
    let rows = |text: &str, room, step| -> Vec<String> {
        wrap_code(&cells(text), room, step)
            .iter()
            .map(|row| crate::ui_text::plain(row))
            .collect()
    };
    // A line that fits is one row, untouched, spaces and all.
    assert_eq!(rows("  a  b", 20, 2), ["  a  b"]);
    assert_eq!(rows("", 20, 2), [""]);
    // Spaces left after a break are dropped, however many.
    assert_eq!(rows(&format!("abc{}", " ".repeat(30)), 20, 4), ["abc"]);
    // After punctuation, where there is no space in the latter half of the row.
    assert_eq!(
        rows("call(first,second,third,fourth)", 20, 4),
        ["call(first,second,", "    third,fourth)"]
    );
    // Never between a punctuation mark and the one closing it.
    assert_eq!(
        rows("aaaaaaaaaaaaaaaaa();b", 20, 4),
        ["aaaaaaaaaaaaaaaaa();", "    b"]
    );
    // A token longer than the row breaks at the row's end, and its rest at each later row's.
    let token = "x".repeat(50);
    let wrapped = rows(&token, 20, 4);
    assert_eq!(wrapped[0], "x".repeat(20));
    assert_eq!(wrapped[1], format!("    {}", "x".repeat(16)));
    assert_eq!(wrapped.concat().replace(' ', ""), token);
    // A wide character that would be cut by the row's end starts the next.
    let wide = format!("{}日本", "x".repeat(19));
    let wrapped = wrap_code(&cells(&wide), 20, 4);
    assert_eq!(crate::ui_text::plain(&wrapped[0]), "x".repeat(19));
    assert!(wrapped.iter().all(|row| crate::ui_text::whole(row)));
    // A letter and the marks it composes with take one cell, and are never parted by a break, nor is an emoji, wide.
    let marked = "e\u{301}".repeat(30);
    let wrapped = rows(&marked, 20, 4);
    assert_eq!(wrapped[0], "\u{e9}".repeat(20));
    assert_eq!(wrapped.concat().replace(' ', ""), "\u{e9}".repeat(30));
    let emoji = format!("{}😀😀", "x".repeat(19));
    let wrapped = wrap_code(&cells(&emoji), 20, 4);
    assert!(wrapped.iter().all(|row| crate::ui_text::whole(row)));
    assert!(wrapped.iter().all(|row| row.len() <= 20));
    // The hanging indent keeps some room for code, however deeply the line is indented.
    let deep = format!("{}{}", " ".repeat(40), "y ".repeat(20));
    let wrapped = wrap_code(&cells(&deep), 45, 4);
    assert!(
        wrapped[1..]
            .iter()
            .all(|row| { row.iter().take_while(|(c, _)| *c == ' ').count() == 45 - CODE_ROOM })
    );
    // The indent step is the least indentation the block's lines have, of two to four.
    assert_eq!(indent_step(&["a", "  b", "    c"]), 2);
    assert_eq!(indent_step(&["a", "\tb", "        c"]), 4);
    assert_eq!(indent_step(&["a", " b"]), 2);
    assert_eq!(indent_step(&["a", "", "b"]), 4);
}

#[test]
fn a_first_line_of_code_taller_than_the_panel_is_cut_and_says_there_is_more() {
    // Wide characters, which take two cells each, wrap onto more rows than the panel has with a hanging indent this
    // deep.
    let line = format!("{}{}", " ".repeat(28), "語".repeat(200));
    let panel = Panel::new("c", "text", format!("{line}\nnext"), Place::Unknown)
        .with_code(Code::new("", 2));
    let canvas = face(&panel, false, false, Fit::SLOT);
    let rows: Vec<String> = canvas.rows.iter().map(row_text).collect();
    assert_eq!(rows.len(), CODE_ROWS);
    // Every row of code is the first line's, the last ending with a dim "…", and the next line counted as more.
    assert!(rows[3].starts_with(" 1 "));
    assert!(
        rows[4..3 + SLOT_CODE_ROWS]
            .iter()
            .all(|row| row.starts_with(" ↪ "))
    );
    let last = &canvas.rows[3 + SLOT_CODE_ROWS - 1];
    assert_eq!(last.cells.last().unwrap(), &('…', Style::plain(DIM)));
    assert!(
        canvas
            .rows
            .iter()
            .all(|row| crate::ui_text::whole(&row.cells))
    );
    // The first line, not shown whole, is counted with the next.
    assert_eq!(rows[CODE_ROWS - 1].trim_end(), "↓ 2 more");
}

#[test]
fn a_code_panel_floating_fits_its_code_and_takes_a_slot_s_size_on_a_wall() {
    // A short block floating is as tall as its lines, at least a few rows, with no row below them, as it shows them all.
    let short = code_panel("c", 3);
    let canvas = face(&short, false, false, Fit::Text);
    assert_eq!(canvas.rows.len(), 3 + FIT_CODE_ROWS.0);
    assert_eq!(canvas.size(), fitted_size(&short, Fit::Text));
    assert_eq!(fitted_size(&short, Fit::Text).0, panel_size().0);
    assert!(fitted_size(&short, Fit::Text).1 < panel_size().1);
    assert!(canvas.shapes.is_empty());
    // The 17 lines the maintainer could not see, which wrap onto more rows than a slot has, grow it to show them all.
    let lines: Vec<String> = (1..=17)
        .map(|line| {
            format!("    const value{line} = compute(input, options, {line}); // step {line} of the build")
        })
        .collect();
    let long = Panel::new("c", "javascript", lines.join("\n"), on(TOMORI_WALL, 0))
        .with_code(Code::new("javascript", 17));
    let slot = face(&long, false, false, Fit::SLOT);
    assert_eq!(slot.rows.len(), CODE_ROWS);
    assert!(row_text(&slot.rows[CODE_ROWS - 1]).starts_with("↓ "));
    let floating = face(&long, false, false, Fit::Text);
    let drawn = joined(&floating.rows[3..].iter().map(row_text).collect::<Vec<_>>());
    assert_eq!(drawn.len(), 17);
    assert_eq!(drawn.last().unwrap().0, 17);
    assert!(floating.rows.len() > CODE_ROWS && floating.rows.len() <= 3 + FIT_CODE_ROWS.1);
    assert_eq!(floating.size(), fitted_size(&long, Fit::Text));
    assert!(floating.shapes.is_empty());
    // Taller than a panel floating may be, it shows as many rows as it may, scrolls, and says what is below.
    let many: Vec<String> = (1..=120).map(|line| format!("line {line}")).collect();
    let tall =
        Panel::new("c", "text", many.join("\n"), Place::Unknown).with_code(Code::new("", 120));
    let canvas = face(&tall, false, false, Fit::Text);
    assert_eq!(canvas.rows.len(), 3 + FIT_CODE_ROWS.1 + 1);
    assert_eq!(row_text(canvas.rows.last().unwrap()), "↓ 80 more");
    assert_eq!(canvas.size(), fitted_size(&tall, Fit::Text));
    let (_, height) = fitted_size(&tall, Fit::Text);
    assert!(height > 0.7 && height < 0.8, "{height}");
    // On a wall, every code panel is a slot's size.
    for panel in [&short, &long, &tall] {
        assert_eq!(fitted_size(panel, Fit::SLOT), panel_size());
        assert_eq!(face(panel, false, false, Fit::SLOT).rows.len(), CODE_ROWS);
    }
}

#[test]
fn a_long_code_panel_says_how_many_more_lines_it_has_and_wraps_long_lines() {
    // An older server cut each line to 64 characters.
    let long = "x".repeat(63);
    let body = std::iter::once(format!("{long}…"))
        .chain((2..=SLOT_CODE_ROWS).map(|line| format!("line {line}")))
        .collect::<Vec<_>>()
        .join("\n");
    let mut panel = Panel::new("c", "text", body, Place::Unknown)
        .with_code(Code::new("", 40))
        .with_pinned(true);
    let canvas = face(&panel, false, false, Fit::SLOT);
    let text = canvas.plain();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[1].trim_end(), "40 lines · 0m · pinned");
    // A line cut by an older server wraps onto a second row, the panel's width, and ends with a dim "…".
    assert_eq!(lines[3].chars().count(), CODE_COLUMNS);
    assert_eq!(
        lines[3],
        format!(" 1 {}", "x".repeat(CODE_COLUMNS - GUTTER))
    );
    assert!(lines[4].starts_with(" ↪     x"), "{}", lines[4]);
    assert!(lines[4].ends_with('…'));
    assert_eq!(canvas.rows[4].cells.last().unwrap().1.color, DIM);
    // So the last line no longer fits, and is counted among those not shown.
    assert_eq!(lines[3 + SLOT_CODE_ROWS - 1].trim_end(), "15 line 15");
    assert_eq!(lines[CODE_ROWS - 1].trim_end(), "↓ 25 more");
    // Without the long line, every line fits, and the rest are counted.
    let body = (1..=SLOT_CODE_ROWS)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    panel.body = body;
    let text = face(&panel, false, false, Fit::SLOT).plain();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[3 + SLOT_CODE_ROWS - 1].trim_end(), "16 line 16");
    // Every line it holds fits, and the rest of its block, which it was not sent, is only in the transcript.
    assert_eq!(
        lines[CODE_ROWS - 1].trim_end(),
        "24 more lines in the transcript"
    );
    // One more line, which it was not sent, is said as one.
    panel.code = Some(Code::new("", SLOT_CODE_ROWS as u32 + 1));
    assert!(
        face(&panel, false, false, Fit::SLOT)
            .plain()
            .ends_with("1 more line in the transcript")
    );
    // A code panel of one line says so; a server's lines beyond what a panel holds are not drawn.
    let one = Panel::new("c", "sh", "ls", Place::Unknown).with_code(Code::new("sh", 1));
    assert!(
        face(&one, false, false, Fit::SLOT)
            .plain()
            .contains("sh · 1 line · 0m")
    );
    let many = Panel::new("c", "sh", vec!["x"; 40].join("\n"), Place::Unknown)
        .with_code(Code::new("sh", 40));
    let text = face(&many, false, false, Fit::SLOT).plain();
    assert_eq!(text.split('\n').count(), CODE_ROWS);
    assert!(text.ends_with("↓ 24 more"));
}

#[test]
fn a_code_panel_whose_spans_are_not_valid_is_drawn_plain() {
    use tomori_protocol::Span;
    for spans in [
        vec![Span::new(0, 99, TokenKind::Keyword)],
        vec![
            Span::new(3, 4, TokenKind::Function),
            Span::new(0, 2, TokenKind::Keyword),
        ],
        vec![Span::new(0, 0, TokenKind::Keyword)],
    ] {
        let mut panel = code_panel("c", 3);
        panel.code = Some(Code::new("rust", 3).with_spans(spans));
        let canvas = face(&panel, false, false, Fit::SLOT);
        assert_eq!(colours(&canvas.rows[3], "fn main"), [TEXT; 7]);
    }
    // Every kind has its colour, distinct from the others, and plain, as a kind this client does not know, is text.
    let colours: Vec<_> = TokenKind::ALL.iter().map(|kind| palette(*kind)).collect();
    for (index, colour) in colours.iter().enumerate() {
        assert!(!colours[index + 1..].contains(colour), "{index}");
    }
    assert_eq!(palette(TokenKind::Plain), TEXT);
}

#[test]
fn the_ray_s_label_names_a_code_panel_with_its_language_and_lines() {
    let lines = |panel: &Panel| -> Vec<String> {
        label(panel, Vec3::new(0.0, 1.5, -3.0), HOME, None)
            .canvas
            .unwrap()
            .plain()
            .lines()
            .map(|line| line.trim_end().to_owned())
            .collect()
    };
    assert_eq!(
        lines(&code_panel("c", 3)),
        ["rust", "rust · 3 lines", "5m ago, on the Tomori wall"]
    );
    assert_eq!(
        lines(&code_panel("c", 40).with_pinned(true)),
        [
            "rust",
            "rust · lines 1–3 of 40 · pinned",
            "5m ago, on the Tomori wall"
        ]
    );
    let mut plain = code_panel("c", 2);
    plain.code = Some(Code::new("", 2));
    assert_eq!(lines(&plain)[1], "2 lines");
    // A text panel pinned by being put back says nothing of it, as only code panels are replaced.
    let text = panel("1", on(TOMORI_WALL, 0)).with_pinned(true);
    assert!(!lines(&text).concat().contains("pinned"));
    assert!(
        !face(&text, false, false, Fit::SLOT)
            .plain()
            .contains("pinned")
    );
}

#[test]
fn the_ray_s_label_names_a_panel_with_its_metadata_and_where_it_is() {
    let point = Vec3::new(0.0, 1.5, -3.0);
    let label_of = |place: Place| {
        let sheet = label(&panel("1", place), point, HOME, None);
        (sheet.canvas.unwrap().plain(), sheet.model.unwrap())
    };
    let (text, model) = label_of(on(TOMORI_WALL, 0));
    assert_eq!(
        text.lines().map(str::trim_end).collect::<Vec<_>>(),
        ["Panel 1", "source: CI", "2m ago, on the Tomori wall"]
    );
    // Just in front of the panel, towards the viewer, and above where the ray meets it.
    let centre = model.w_axis.truncate();
    assert!(centre.z > point.z && centre.y > point.y);
    assert!(label_of(on("wall-1", 0)).0.contains("on a wall"));
    let free = Pose::default();
    assert!(label_of(Place::Free { pose: free }).0.contains("floating"));
}

#[test]
fn panels_are_drawn_in_their_slots_wall_by_wall_and_floating_ones_where_they_float() {
    let free = Pose::upright_facing([-1.0, 1.4, -1.0], [0.0, 1.6, 0.0]);
    let shown = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on(TOMORI_WALL, 4)).with_age(10),
        panel("c", on("wall-1", 0)),
        panel("d", Place::Free { pose: free }),
        // Not drawn: a wall not shown, a slot past the last and a place this client does not know.
        panel("e", on("wall-9", 0)),
        panel("f", on(TOMORI_WALL, WALL_SLOTS)),
        panel("g", Place::Unknown),
    ]);
    let mut projector = Projector::default();
    let output = projector.update(input(&shown, None, 0.0, Instant::now()));
    assert_eq!(output.sheets.len(), 2 + 4);
    let drawn: Vec<(String, Vec3)> = output
        .sheets
        .iter()
        .map(|sheet| {
            let canvas = sheet.canvas.as_ref().unwrap();
            (
                canvas.plain().lines().next().unwrap().trim().to_owned(),
                sheet.model.unwrap().w_axis.truncate(),
            )
        })
        .collect();
    // The farther wall first, the added one, with its panel; then the Tomori wall with its two; then the floating one,
    // nearest.
    let titles: Vec<&str> = drawn.iter().map(|(title, _)| title.as_str()).collect();
    assert_eq!(
        titles,
        ["WALL", "Panel c", "TOMORI", "Panel a", "Panel b", "Panel d"]
    );
    assert!(drawn[3].1.distance(slot_centre(&shown, 0)) < 1e-5);
    assert!(drawn[4].1.distance(slot_centre(&shown, 4)) < 1e-5);
    assert!(drawn[5].1.distance(Vec3::from_array(free.position)) < 1e-5);
    // The newest on the Tomori wall is framed in blue.
    let border = |index: usize| output.sheets[index].canvas.as_ref().unwrap().border;
    assert_eq!((border(3), border(4)), (Some(FRAME), Some(NEWEST)));
    // Of panels projected together, the last listed, the last the server put there, is the newest.
    let together = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on(TOMORI_WALL, 1)),
    ]);
    let output = Projector::default().update(input(&together, None, 0.0, Instant::now()));
    let borders: Vec<_> = output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_ref())
        .filter(|canvas| canvas.plain().starts_with("Panel"))
        .map(|canvas| canvas.border)
        .collect();
    assert_eq!(borders, [Some(FRAME), Some(NEWEST)]);
    // Without a projection, nothing is drawn.
    let none = Input {
        projection: None,
        ..input(&shown, None, 0.0, Instant::now())
    };
    assert_eq!(projector.update(none), Output::default());
}

#[test]
fn a_panel_is_pointed_at_pulled_off_carried_and_left_floating() {
    let shown = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on(TOMORI_WALL, 1)),
    ]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = at(slot_centre(&shown, 1));
    let output = projector.update(input(&shown, ray, 0.0, now));
    let (pointed, distance, point) = output.pointed.clone().unwrap();
    assert_eq!(pointed, "b");
    assert!((distance - HOME.distance(slot_centre(&shown, 1))).abs() < 0.01);
    assert_eq!(output.end, Some(point));
    // Highlighted where it stands.
    let highlighted = output
        .sheets
        .iter()
        .filter(|sheet| sheet.canvas.as_ref().unwrap().border == Some(HIGHLIGHT))
        .count();
    assert_eq!(highlighted, 1);
    // Squeezed, it is pulled off and follows the ray.
    let output = projector.update(input(&shown, ray, 1.0, now));
    assert!(output.picked);
    assert_eq!(projector.held(), Some("b"));
    let moved = at(Vec3::new(-1.0, 1.4, -2.0));
    let output = projector.update(input(&shown, moved, 1.0, now));
    let held_at = output.end.unwrap();
    assert!((held_at - moved.unwrap().at(distance)).length() < 1e-4);
    // Let go away from every wall, it floats where it is, shown there until the server agrees.
    let output = projector.update(input(&shown, moved, 0.0, now));
    let Some((panel_id, Target::Free(left))) = output.placed else {
        unreachable!("expected a floating panel, not {:?}", output.placed);
    };
    assert_eq!(panel_id, "b");
    assert!(Vec3::from_array(left.position).distance(held_at) < 1e-4);
    // Upright, though it was carried below the eyes: turned only about the vertical.
    assert!(left.orientation[0].abs() < 1e-5 && left.orientation[2].abs() < 1e-5);
    let drawn_at = |output: &Output, title: &str| {
        output
            .sheets
            .iter()
            .find(|sheet| sheet.canvas.as_ref().unwrap().plain().starts_with(title))
            .map(|sheet| sheet.model.unwrap().w_axis.truncate())
    };
    let output = projector.update(input(&shown, None, 0.0, now));
    assert!(drawn_at(&output, "Panel b").unwrap().distance(held_at) < 1e-4);
    // Until the time runs out, when it is drawn where the server says.
    let output = projector.update(input(&shown, None, 0.0, now + PENDING));
    assert!(
        drawn_at(&output, "Panel b")
            .unwrap()
            .distance(slot_centre(&shown, 1))
            < 1e-5
    );
    // Or until the server says it floats there.
    projector.update(input(&shown, ray, 1.0, now));
    projector.update(input(&shown, moved, 1.0, now));
    projector.update(input(&shown, moved, 0.0, now));
    let floating = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", Place::Free { pose: left }),
    ]);
    let output = projector.update(input(&floating, None, 0.0, now));
    assert!(projector.pending.is_none());
    assert!(drawn_at(&output, "Panel b").unwrap().distance(held_at) < 1e-4);
}

#[test]
fn a_panel_let_go_near_a_wall_snaps_into_its_layout() {
    let shown = projection(vec![panel("a", on(TOMORI_WALL, 0))]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = at(slot_centre(&shown, 0));
    projector.update(input(&shown, ray, 1.0, now));
    assert_eq!(projector.held(), Some("a"));
    // Carried over to the added wall, a little in front of it and past its edge, and let go.
    let added = model(&shown.wall("wall-1").unwrap().pose).unwrap();
    let width = wall_size(shown.wall("wall-1").unwrap()).0;
    let near = added.transform_point3(Vec3::new(width / 2.0 + 0.1, 0.0, 0.3));
    let distance = HOME.distance(slot_centre(&shown, 0));
    let direction = (near - HOME).normalize();
    let toward = Ray::new(near - direction * distance, direction);
    projector.update(input(&shown, toward, 1.0, now));
    let output = projector.update(input(&shown, toward, 0.0, now));
    assert_eq!(
        output.placed,
        Some(("a".to_owned(), Target::Wall("wall-1".to_owned(), 5)))
    );
    // Shown where it was let go until the server puts it in the wall's layout.
    let moved = projection(vec![panel("a", on("wall-1", 5))]);
    projector.update(input(&moved, None, 0.0, now));
    assert!(projector.pending.is_none());
}

#[test]
fn the_grip_pulls_off_nothing_behind_something_nearer_or_while_the_panels_do_not_take_it() {
    let shown = projection(vec![panel("a", on(TOMORI_WALL, 0))]);
    let now = Instant::now();
    let ray = at(slot_centre(&shown, 0));
    // Something nearer on the ray, such as a card, takes the grip.
    let mut projector = Projector::default();
    let nearer = Input {
        nearer: 1.0,
        ..input(&shown, ray, 1.0, now)
    };
    let output = projector.update(nearer);
    assert!(!output.picked && projector.held().is_none());
    assert!(output.pointed.is_some());
    // Inactive, as while the hand menu shows, nothing is pointed at or pulled off, and a panel carried is let go.
    let mut projector = Projector::default();
    projector.update(input(&shown, ray, 0.0, now));
    let inactive = Input {
        active: false,
        ..input(&shown, ray, 1.0, now)
    };
    let output = projector.update(inactive);
    assert_eq!((output.pointed, output.picked), (None, false));
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    assert_eq!(projector.held(), Some("a"));
    let output = projector.update(inactive);
    assert!(output.placed.is_some());
    assert!(projector.held().is_none());
    // A panel dismissed while it is carried is no longer carried.
    let mut projector = Projector::default();
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    let empty = projection(Vec::new());
    projector.update(input(&empty, ray, 1.0, now));
    assert!(projector.held().is_none());
    // A ray along the panel's plane meets it nowhere.
    let edge = Ray::new(slot_centre(&shown, 0) + Vec3::X * 2.0, -Vec3::X);
    let mut projector = Projector::default();
    assert!(
        projector
            .update(input(&shown, edge, 0.0, now))
            .pointed
            .is_none()
    );
}

#[test]
fn a_panel_carried_stays_in_the_hand_while_a_teleport_is_aimed() {
    let shown = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on(TOMORI_WALL, 1)),
    ]);
    let now = Instant::now();
    let aiming = |ray: Option<Ray>, grip: f32| Input {
        pointing: false,
        ..input(&shown, ray, grip, now)
    };
    // Aiming, the ray points at no panel and the grip pulls none off.
    let mut projector = Projector::default();
    let ray = at(slot_centre(&shown, 0));
    let output = projector.update(aiming(ray, 0.0));
    assert_eq!(output.pointed, None);
    let output = projector.update(aiming(ray, 1.0));
    assert!(!output.picked && projector.held().is_none());
    // A panel carried as aiming starts stays carried, following the ray at its distance, wherever the rig takes the ray.
    let mut projector = Projector::default();
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    assert_eq!(projector.held(), Some("a"));
    let distance = HOME.distance(slot_centre(&shown, 0));
    let travelled = Ray::new(HOME + Vec3::new(3.0, 0.0, -4.0), Vec3::new(0.3, -0.5, -1.0));
    let output = projector.update(aiming(travelled, 1.0));
    assert_eq!(output.placed, None);
    assert_eq!(projector.held(), Some("a"));
    assert!((output.end.unwrap() - travelled.unwrap().at(distance)).length() < 1e-4);
    assert_eq!(output.pointed, None);
    // Without the ray, as when the controller is not tracked, it stays where it was.
    let output = projector.update(aiming(None, 1.0));
    assert!((output.end.unwrap() - travelled.unwrap().at(distance)).length() < 1e-4);
    assert_eq!(projector.held(), Some("a"));
    // Let go while aiming, it floats where it is.
    let output = projector.update(aiming(travelled, 0.0));
    assert!(matches!(output.placed, Some((ref panel, Target::Free(_))) if panel == "a"));
}

#[test]
fn the_right_stick_pushes_pulls_and_turns_a_panel_carried_in_snaps_and_it_is_let_go_so() {
    use crate::manipulation::{FARTHEST, NEAREST, RATE, YAW_STEP};
    let shown = projection(vec![panel("a", on(TOMORI_WALL, 0))]);
    let now = Instant::now();
    let ray = at(slot_centre(&shown, 0));
    let distance = HOME.distance(slot_centre(&shown, 0));
    let stick = |stick: Vec2, dt: f32| Input {
        stick,
        dt,
        ..input(&shown, ray, 1.0, now)
    };
    let mut projector = Projector::default();
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    let carried_at = |output: &Output| {
        output
            .sheets
            .iter()
            .find(|sheet| {
                sheet
                    .canvas
                    .as_ref()
                    .unwrap()
                    .plain()
                    .starts_with("Panel a")
            })
            .unwrap()
            .model
            .unwrap()
            .w_axis
            .truncate()
    };
    // Forward for a fifth of a second pushes it further along the ray.
    let output = projector.update(stick(Vec2::Y, 0.2));
    let pushed = distance * (RATE * 0.2).exp();
    assert!((carried_at(&output) - ray.unwrap().at(pushed)).length() < 1e-3);
    // Back, long enough, pulls it to the nearest it goes; forward, to the farthest.
    projector.update(stick(Vec2::NEG_Y, 10.0));
    let output = projector.update(stick(Vec2::ZERO, 0.0));
    assert!((carried_at(&output) - ray.unwrap().at(NEAREST)).length() < 1e-3);
    let output = projector.update(stick(Vec2::Y, 10.0));
    assert!((carried_at(&output) - ray.unwrap().at(FARTHEST)).length() < 1e-3);
    projector.update(stick(Vec2::ZERO, 0.0));
    // A flick right turns it a snap clockwise, seen from above, once, and says so.
    let facing_of = |output: &Output| {
        let model = output
            .sheets
            .iter()
            .find(|sheet| {
                sheet
                    .canvas
                    .as_ref()
                    .unwrap()
                    .plain()
                    .starts_with("Panel a")
            })
            .unwrap()
            .model
            .unwrap();
        model.z_axis.truncate()
    };
    let square = facing_of(&projector.update(stick(Vec2::ZERO, 0.0)));
    let output = projector.update(stick(Vec2::X, 0.1));
    assert!(output.turned);
    let turned = facing_of(&output);
    assert!((square.angle_between(turned) - YAW_STEP).abs() < 1e-4);
    assert!(square.cross(turned).y < 0.0, "clockwise");
    assert!(turned.y.abs() < 1e-5, "upright");
    assert!(!projector.update(stick(Vec2::X, 0.1)).turned);
    projector.update(stick(Vec2::ZERO, 0.0));
    assert!(projector.update(stick(Vec2::X, 0.1)).turned);
    projector.update(stick(Vec2::ZERO, 0.0));
    // A click turns it back square, once; square already, it says nothing.
    let click = Input {
        straighten: true,
        ..stick(Vec2::ZERO, 0.0)
    };
    let output = projector.update(click);
    assert!(output.turned);
    assert!(facing_of(&output).distance(square) < 1e-4);
    assert!(!projector.update(click).turned);
    // Turned and let go away from any wall, as it is behind the wall the ray meets while a teleport is aimed, it floats
    // turned, upright, as the server is told.
    projector.update(stick(Vec2::NEG_X, 0.1));
    projector.update(stick(Vec2::ZERO, 0.0));
    let output = projector.update(Input {
        pointing: false,
        ..input(&shown, ray, 0.0, now)
    });
    let Some((_, Target::Free(left))) = output.placed else {
        unreachable!("floating, not {:?}", output.placed);
    };
    let facing = Quat::from_array(left.orientation) * Vec3::Z;
    assert!((square.angle_between(facing) - YAW_STEP).abs() < 1e-4);
    assert!(square.cross(facing).y > 0.0, "anticlockwise");
    // Without a panel carried, the stick does nothing here.
    let output = projector.update(Input {
        grip: Some(0.0),
        ..stick(Vec2::X, 0.1)
    });
    assert!(!output.turned && projector.held().is_none());
}

#[test]
fn the_nearest_panel_on_the_ray_is_pointed_at_and_a_panel_snaps_onto_the_nearest_wall() {
    let floating = |id: &str, distance: f32| {
        panel(
            id,
            Place::Free {
                pose: Pose::upright_facing((HOME - Vec3::Z * distance).to_array(), HOME.to_array()),
            },
        )
    };
    // Two walls a hand's width apart, one behind the other, and two panels floating on the ray before them.
    let walls = vec![
        wall("near", WallKind::Added, 0.0, 3.0),
        wall("far", WallKind::Added, 0.0, 3.3),
    ];
    let shown = Projection::new("s", "sky")
        .with_walls(walls)
        .with_panels(vec![floating("behind", 2.0), floating("before", 1.5)]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = Ray::new(HOME, -Vec3::Z);
    let output = projector.update(input(&shown, ray, 0.0, now));
    assert_eq!(output.pointed.unwrap().0, "before");
    // Pulled off and let go between the walls, nearer the far one, while the ray aims at no slot, as while a teleport is
    // aimed, it snaps into the nearest slot of that, the slot it is carried in front of.
    projector.update(input(&shown, ray, 1.0, now));
    let between = Ray::new(HOME - Vec3::Z * (3.2 - 1.5), -Vec3::Z);
    let aiming = |grip: f32| Input {
        pointing: false,
        ..input(&shown, between, grip, now)
    };
    let output = projector.update(aiming(1.0));
    assert_eq!(output.preview, Some(("far".to_owned(), 1)));
    let output = projector.update(aiming(0.0));
    assert_eq!(
        output.placed,
        Some(("before".to_owned(), Target::Wall("far".to_owned(), 1)))
    );
    // Pointing, the slot the ray meets first decides, on the near wall.
    let mut projector = Projector::default();
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    projector.update(input(&shown, between, 1.0, now));
    let output = projector.update(input(&shown, between, 0.0, now));
    assert_eq!(
        output.placed,
        Some(("before".to_owned(), Target::Wall("near".to_owned(), 1)))
    );
}

#[test]
fn where_the_server_says_a_panel_is_agrees_with_where_it_was_put() {
    let pose = Pose::upright_facing([1.0, 1.5, -1.0], [0.0, 1.5, 0.0]);
    assert!(agrees(&on("w", 0), &Target::Wall("w".into(), 0)));
    assert!(!agrees(&on("w", 0), &Target::Wall("v".into(), 0)));
    assert!(!agrees(&on("w", 0), &Target::Wall("w".into(), 1)));
    assert!(agrees(&Place::Free { pose }, &Target::Free(pose)));
    let moved = Pose {
        position: [1.5, 1.5, -1.0],
        ..pose
    };
    assert!(!agrees(&Place::Free { pose: moved }, &Target::Free(pose)));
    assert!(!agrees(&Place::Free { pose }, &Target::Wall("w".into(), 0)));
    assert!(!agrees(&Place::Unknown, &Target::Free(pose)));
}

#[test]
fn a_wall_added_stands_ahead_of_the_participant_facing_them() {
    let head = Pose {
        position: [1.0, 1.7, 2.0],
        // Turned 90° to the left, so looking along −X.
        orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array(),
    };
    let wall = added_wall(head);
    let position = Vec3::from_array(wall.position);
    assert!(
        position.distance(Vec3::new(1.0 - WALL_DISTANCE, 1.6, 2.0)) < 1e-4,
        "{position}"
    );
    // Its +Z faces back towards the participant.
    let facing = Quat::from_array(wall.orientation) * Vec3::Z;
    assert!(facing.distance(Vec3::X) < 1e-4, "{facing}");
    // Looking straight down, or with no orientation, it stands ahead along −Z.
    for orientation in [
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2).to_array(),
        [0.0; 4],
    ] {
        let wall = added_wall(Pose {
            orientation,
            ..head
        });
        assert!(
            Vec3::from_array(wall.position).distance(Vec3::new(1.0, 1.6, 2.0 - WALL_DISTANCE))
                < 1e-4
        );
    }
}

/// The Tomori wall where the server stands it by default: below the swarm, curving about home.
fn curved() -> Wall {
    Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
    )
    .with_shape(WallShape::Arc { radius: 2.3 })
}

/// The corners of a panel in a slot standing at `model`, in the scene.
fn corners(model: Mat4) -> [Vec3; 4] {
    let (width, height) = panel_size();
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(x, y)| model.transform_point3(Vec3::new(x * width / 2.0, y * height / 2.0, 0.0)))
}

#[test]
fn a_curved_wall_stands_its_slots_in_a_row_along_its_arc_each_facing_home() {
    let wall = curved();
    let frame = model(&wall.pose).unwrap();
    let slots = slot_frames(&wall, frame);
    assert_eq!(slots.len(), WALL_SLOTS as usize);
    let (width, height) = panel_size();
    for (index, slot) in slots.iter().enumerate() {
        let centre = slot.w_axis.truncate();
        // On the arc about home, at the wall's height, just in front of it.
        let across = Vec2::new(centre.x, centre.z).length();
        assert!((across - (2.3 - LIFT)).abs() < 1e-4, "{index}: {across}");
        assert!((centre.y - 0.52).abs() < 1e-5);
        // Upright, facing home, and smaller than a flat wall's.
        let facing = slot.z_axis.truncate().normalize();
        assert!(facing.y.abs() < 1e-5);
        let towards = (Vec3::new(0.0, 0.52, 0.0) - centre).normalize();
        assert!(facing.dot(towards) > 0.9999, "{index}");
        assert!((slot.x_axis.truncate().length() - ARC_SCALE).abs() < 1e-5);
        assert!(slot.y_axis.truncate().normalize().dot(Vec3::Y) > 0.9999);
    }
    // Read left to right as seen from home, symmetric about straight ahead, side by side without overlapping.
    let xs: Vec<f32> = slots.iter().map(|slot| slot.w_axis.x).collect();
    assert!(xs.windows(2).all(|pair| pair[0] < pair[1]));
    assert!((xs[0] + xs[5]).abs() < 1e-4 && (xs[2] + xs[3]).abs() < 1e-4);
    for pair in slots.windows(2) {
        let gap = pair[0]
            .w_axis
            .truncate()
            .distance(pair[1].w_axis.truncate());
        assert!(gap > width * ARC_SCALE, "{gap}");
    }
    // Below the swarm: its top edge lower than the swarm's flock ever comes, and every slot within the cone the workers
    // leave clear in front of home, 40° either side.
    for slot in &slots {
        for corner in corners(*slot) {
            assert!(corner.y < crate::swarm::CENTER.y - 0.6, "{corner}");
            assert!(corner.y > 0.3, "{corner}");
            let bearing = corner.x.atan2(-corner.z).to_degrees();
            assert!(bearing.abs() < 34.0, "{bearing}");
            // And below the status pill, which stands about 20° below the eyes from home.
            let down = (HOME.y - corner.y)
                .atan2(Vec2::new(corner.x, corner.z).length())
                .to_degrees();
            assert!(down > 22.0, "{down}");
        }
    }
    assert!(height * ARC_SCALE < 0.3);
    // A shape this client cannot draw curved, or a nonsense radius, is drawn flat.
    for shape in [
        WallShape::Unknown,
        WallShape::Arc { radius: 0.2 },
        WallShape::Arc { radius: f32::NAN },
    ] {
        let flat = curved().with_shape(shape);
        assert_eq!(
            slot_frames(&flat, frame)[0],
            frame * Mat4::from_translation(slot_offset(&flat, 0))
        );
    }
}

#[test]
fn the_default_tomori_wall_hides_neither_the_workers_nor_the_swarm() {
    let wall = curved();
    let slots = slot_frames(&wall, model(&wall.pose).unwrap());
    // Whether something seen from home at `point` is hidden by a slot, or `point` is inside one's panel.
    let hidden = |point: Vec3| {
        let ray = Ray::new(HOME, point - HOME).unwrap();
        slots.iter().any(|slot| {
            hit_within(*slot, panel_size(), 0.0, &ray)
                .is_some_and(|distance| distance < point.distance(HOME))
        })
    };
    // Every worker's station, anywhere it may stand, to the edge of what it draws, in every direction.
    let around: Vec<Vec3> = [
        Vec3::ZERO,
        Vec3::X,
        -Vec3::X,
        Vec3::Y,
        -Vec3::Y,
        Vec3::Z,
        -Vec3::Z,
    ]
    .map(|direction| direction * crate::delegation::POINTING_RADIUS)
    .to_vec();
    for station in 0..crate::delegation::SLOT_COUNT {
        let anchor = crate::delegation::slot_position(station);
        for offset in &around {
            assert!(!hidden(anchor + *offset), "station {station} at {anchor}");
        }
        // And nowhere near a slot.
        for slot in &slots {
            assert!(
                slot.w_axis.truncate().distance(anchor) > 0.6,
                "station {station}"
            );
        }
    }
    // The swarm's flock, as far as it spreads in any of its states.
    for x in [-1.35, -0.7, 0.0, 0.7, 1.35] {
        for y in [-0.52, 0.0, 0.52] {
            for z in [-0.42, 0.0, 0.42] {
                let point = crate::swarm::CENTER + Vec3::new(x, y, z);
                assert!(!hidden(point), "{point}");
            }
        }
    }
}

#[test]
fn a_curved_wall_draws_a_board_for_each_slot_its_label_and_its_panels_in_them() {
    let shown = Projection::new("s", "sky")
        .with_walls(vec![curved()])
        .with_panels(vec![panel("a", on(TOMORI_WALL, 5))]);
    let mut projector = Projector::default();
    let output = projector.update(input(&shown, None, 0.0, Instant::now()));
    // Six boards, the label, and the panel.
    assert_eq!(output.sheets.len(), WALL_SLOTS as usize + 2);
    let wall = curved();
    let slots = slot_frames(&wall, model(&wall.pose).unwrap());
    for (sheet, slot) in output.sheets.iter().zip(&slots) {
        assert_eq!(sheet.model, Some(*slot));
        assert_eq!(sheet.canvas.as_ref().unwrap().size(), panel_size());
    }
    let label = &output.sheets[WALL_SLOTS as usize];
    assert_eq!(label.canvas.as_ref().unwrap().plain().trim(), "TOMORI");
    let below = label.model.unwrap().w_axis.truncate();
    assert!(below.x.abs() < 1e-5 && below.y < slots[2].w_axis.y - panel_size().1 * ARC_SCALE / 2.0);
    let drawn = output.sheets.last().unwrap();
    assert!(
        drawn
            .canvas
            .as_ref()
            .unwrap()
            .plain()
            .starts_with("Panel a")
    );
    assert_eq!(drawn.model, Some(slots[5]));
    // Another kind of wall is labelled so.
    let other = Wall::new("w", WallKind::Added, curved().pose).with_shape(curved().shape);
    assert_eq!(arc_label(&other).plain().trim(), "WALL");
}

#[test]
fn the_ray_aims_at_each_slot_of_a_curved_wall_in_its_own_plane() {
    let wall = curved();
    let frame = model(&wall.pose).unwrap();
    let slots = slot_frames(&wall, frame);
    let walls = [(&wall, frame)];
    for (index, slot) in slots.iter().enumerate() {
        // At its middle, and at its outer edge, where a flat plane through the wall's middle would miss it or name
        // another.
        let (width, _) = panel_size();
        for local in [Vec3::ZERO, Vec3::X * width * 0.48, -Vec3::X * width * 0.48] {
            let point = slot.transform_point3(local);
            let aimed = aim(&walls, &Ray::new(HOME, point - HOME).unwrap()).unwrap();
            assert_eq!(
                (aimed.wall.as_str(), aimed.slot),
                (TOMORI_WALL, index as u32)
            );
            assert!(aimed.point.unwrap().distance(point) < 1e-3);
            assert_eq!(aimed.model, *slot);
        }
    }
    // A panel pointed at in a curved wall's slot is hit in that slot's plane too.
    let shown = Projection::new("s", "sky")
        .with_walls(vec![wall.clone()])
        .with_panels(vec![panel("a", on(TOMORI_WALL, 0))]);
    let edge = slots[0].transform_point3(-Vec3::X * panel_size().0 * 0.45);
    let output = Projector::default().update(input(&shown, at(edge), 0.0, Instant::now()));
    assert_eq!(output.pointed.unwrap().0, "a");
    // Above the row, beyond the reach, or from behind a nearer slot, nothing else is aimed at.
    let above = slots[2].transform_point3(Vec3::Y * panel_size().1);
    assert_eq!(aim(&walls, &Ray::new(HOME, above - HOME).unwrap()), None);
    let far = Ray::new(Vec3::new(0.0, 0.52, 9.0), -Vec3::Z).unwrap();
    assert_eq!(aim(&walls, &far), None);
}

#[test]
fn carrying_a_panel_onto_a_wall_shows_where_it_will_go_and_puts_it_in_the_slot_under_the_ray() {
    let shown = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on("wall-1", 4)),
    ]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let pick = at(slot_centre(&shown, 0));
    projector.update(input(&shown, pick, 0.0, now));
    projector.update(input(&shown, pick, 1.0, now));
    // Away from every wall, nothing is shown.
    let away = Ray::new(HOME, Vec3::new(-1.0, 0.2, 0.3));
    let output = projector.update(input(&shown, away, 1.0, now));
    assert_eq!(output.preview, None);
    let ghosts = |output: &Output| {
        output
            .sheets
            .iter()
            .filter(|sheet| sheet.canvas.as_ref().unwrap().background == GHOST)
            .map(|sheet| sheet.model.unwrap())
            .collect::<Vec<_>>()
    };
    assert!(ghosts(&output).is_empty());
    // Pointed at the added wall's slot 4, which another panel holds, its outline is drawn there, over that panel, and
    // the ray ends on the slot.
    let added = &shown.wall("wall-1").unwrap();
    let frame = model(&added.pose).unwrap();
    let target = slot_frames(added, frame)[4];
    let toward = target.w_axis.truncate();
    let ray = at(toward);
    let output = projector.update(input(&shown, ray, 1.0, now));
    assert_eq!(output.preview, Some(("wall-1".to_owned(), 4)));
    assert_eq!(
        ghosts(&output),
        [target * Mat4::from_translation(Vec3::Z * LIFT)]
    );
    let order: Vec<String> = output
        .sheets
        .iter()
        .map(|sheet| {
            let canvas = sheet.canvas.as_ref().unwrap();
            if canvas.background == GHOST {
                "ghost".to_owned()
            } else {
                canvas
                    .plain()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            }
        })
        .collect();
    let b = order.iter().position(|title| title == "Panel b").unwrap();
    let ghost = order.iter().position(|title| title == "ghost").unwrap();
    assert!(ghost > b, "{order:?}");
    assert!(output.end.unwrap().distance(toward) < 1e-3);
    // The panel carried still floats in the hand, fitting its text.
    let carried = output
        .sheets
        .iter()
        .find(|sheet| {
            sheet
                .canvas
                .as_ref()
                .unwrap()
                .plain()
                .starts_with("Panel a")
        })
        .unwrap();
    assert_eq!(
        carried.canvas.as_ref().unwrap().size(),
        fitted_size(&shown.panels[0], Fit::Text)
    );
    // Turned and pushed, it still goes where the ray points: let go, it is put in that slot, and shown there at once,
    // at the slot's size, until the server agrees.
    projector.update(Input {
        stick: Vec2::X,
        ..input(&shown, ray, 1.0, now)
    });
    projector.update(input(&shown, ray, 1.0, now));
    let output = projector.update(input(&shown, ray, 0.0, now));
    assert_eq!(
        output.placed,
        Some(("a".to_owned(), Target::Wall("wall-1".to_owned(), 4)))
    );
    assert_eq!(output.preview, None);
    let placed = output
        .sheets
        .iter()
        .find(|sheet| {
            sheet
                .canvas
                .as_ref()
                .unwrap()
                .plain()
                .starts_with("Panel a")
        })
        .unwrap();
    assert_eq!(placed.model, Some(target));
    assert_eq!(placed.canvas.as_ref().unwrap().size(), panel_size());
    assert!(ghosts(&output).is_empty());
    // The server swaps them: then it agrees, and what it says is drawn.
    let swapped = projection(vec![
        panel("a", on("wall-1", 4)),
        panel("b", on(TOMORI_WALL, 0)),
    ]);
    projector.update(input(&swapped, None, 0.0, now));
    assert!(projector.pending.is_none());
}

#[test]
fn a_floating_panel_fits_its_text_within_bounds_and_takes_a_slot_s_size_on_a_wall() {
    let floating = Place::Free {
        pose: Pose::default(),
    };
    // Short text: as narrow and as short as a panel goes.
    let short = Panel::new("s", "Hi", "Yes.", floating.clone());
    assert_eq!(fit(&short, Fit::Text), (FIT_COLUMNS.0, FIT_ROWS.0));
    let face_of = |panel: &Panel| face(panel, false, false, Fit::Text);
    assert_eq!(face_of(&short).size(), fitted_size(&short, Fit::Text));
    // A long line: as wide as it reads comfortably, wrapped there by the shared rule, with every word kept.
    let words: Vec<String> = (0..60).map(|n| format!("word{n}")).collect();
    let long_line = Panel::new("l", "Wide", words.join(" "), floating.clone());
    let (columns, rows) = fit(&long_line, Fit::Text);
    assert_eq!(columns, FIT_COLUMNS.1);
    let wrapped = crate::transcript::parse_markdown(&long_line.body, columns);
    assert_eq!(rows, wrapped.len());
    let text = face_of(&long_line).plain();
    for word in &words {
        assert!(text.contains(word.as_str()), "{word}");
    }
    assert!(
        text.lines()
            .all(|line| crate::ui_text::width(line) <= columns)
    );
    assert!(!text.contains('…'));
    // A middling body: as wide as its widest line, or its title, or its metadata.
    let middling = Panel::new(
        "m",
        "A title that is thirty-three long",
        "Short.\n\nA line of exactly forty characters long!",
        floating.clone(),
    );
    let (columns, rows) = fit(&middling, Fit::Text);
    assert_eq!(columns, 40);
    assert_eq!(
        rows,
        crate::transcript::parse_markdown(&middling.body, 40)
            .len()
            .max(FIT_ROWS.0)
    );
    let titled = Panel::new(
        "t",
        "A title that is thirty-three long",
        "x",
        floating.clone(),
    );
    assert_eq!(fit(&titled, Fit::Text).0, 33);
    // Very long: as tall as a panel goes, ending with "…".
    let tall = Panel::new(
        "t",
        "",
        (0..60)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        floating.clone(),
    );
    assert_eq!(fit(&tall, Fit::Text).1, FIT_ROWS.1);
    let text = face_of(&tall).plain();
    assert_eq!(text.lines().count(), FIT_ROWS.1 + 3);
    assert!(text.lines().last().unwrap().trim_end().ends_with('…'));
    // On a wall, any of them is a slot's size, cut as before.
    for panel in [&short, &long_line, &tall] {
        assert_eq!(fit(panel, Fit::SLOT), (COLUMNS, BODY_ROWS));
        assert_eq!(face(panel, false, false, Fit::SLOT).size(), panel_size());
    }
    // Floating in the space, a panel is drawn and pointed at at its fitted size.
    let shown = Projection::new("s", "sky").with_panels(vec![Panel::new(
        "w",
        "Wide",
        words.join(" "),
        Place::Free {
            pose: Pose::upright_facing([0.0, 1.6, -2.0], HOME.to_array()),
        },
    )]);
    let (width, _) = fitted_size(&shown.panels[0], Fit::Text);
    assert!(width > panel_size().0);
    let edge = Vec3::new(width / 2.0 - 0.02, 1.6, -2.0);
    let output = Projector::default().update(input(&shown, at(edge), 0.0, Instant::now()));
    assert_eq!(output.pointed.unwrap().0, "w");
    assert_eq!(
        output.sheets[0].canvas.as_ref().unwrap().size(),
        fitted_size(&shown.panels[0], Fit::Text)
    );
}

#[test]
fn the_hint_says_what_each_control_does_to_a_panel_carried_and_what_letting_go_would_do() {
    let text = |controls, release| hint(controls, release).plain();
    assert_eq!(
        text(Controls::Controllers, Release::Float),
        [
            "Stick ↕  further / closer",
            "Stick ↔  turn",
            "Click    square up",
            "B        throw away, for all",
            "Let go   leave it floating",
        ]
        .join("\n")
    );
    // Letting go says where the panel would go: into the slot outlined, or changing places with the panel in it.
    assert!(text(Controls::Controllers, Release::Slot).ends_with("Let go   put it in the slot"));
    assert!(text(Controls::Controllers, Release::Swap).ends_with("Let go   swap with that one"));
    // The desktop window names the keys that stand for the controls.
    assert_eq!(
        text(Controls::Keyboard, Release::Float),
        [
            "PgUp PgDn  further / closer",
            "[ ]        turn",
            "R          square up",
            "Backspace  throw away, for all",
            "Let go G   leave it floating",
        ]
        .join("\n")
    );
    // Every row fits, and throwing away, which is for everyone, is in a colour of its own.
    for controls in [Controls::Controllers, Controls::Keyboard] {
        let canvas = hint(controls, Release::Swap);
        assert!(
            canvas
                .rows
                .iter()
                .all(|row| row.cells.len() <= canvas.columns)
        );
        assert!(!canvas.plain().contains('…'));
        assert!(
            canvas.rows[3]
                .cells
                .iter()
                .all(|(_, style)| style.color == THROW)
        );
        assert!(
            canvas.rows[0]
                .cells
                .iter()
                .all(|(_, style)| style.color != THROW)
        );
    }
}

#[test]
fn the_hint_shows_only_while_a_panel_is_carried_beside_it_and_legible_at_any_distance() {
    let shown = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on(TOMORI_WALL, 1)),
    ]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = at(slot_centre(&shown, 1));
    // Pointed at, a panel has no hint.
    let output = projector.update(input(&shown, ray, 0.0, now));
    assert!(output.pointed.is_some());
    assert_eq!(output.hint, None);
    // Carried, it has, below it, facing the viewer, and naming what letting go would do, from the frame after it is
    // pulled off, once where it would go is known.
    let output = projector.update(input(&shown, ray, 1.0, now));
    assert!(output.picked);
    assert_eq!(output.hint, None);
    let held = |projector: &Projector| projector.held.as_ref().unwrap().model;
    let text = |output: &Output| {
        output
            .hint
            .as_ref()
            .unwrap()
            .canvas
            .as_ref()
            .unwrap()
            .plain()
    };
    // Still over its own slot, from the next frame, letting go puts it back there.
    let output = projector.update(input(&shown, ray, 1.0, now));
    assert!(
        text(&output).ends_with("put it in the slot"),
        "{}",
        text(&output)
    );
    // Over the other panel's slot, they would change places.
    let output = projector.update(input(&shown, at(slot_centre(&shown, 0)), 1.0, now));
    assert!(
        text(&output).ends_with("swap with that one"),
        "{}",
        text(&output)
    );
    // Away from the walls, it would float.
    let away = at(HOME + Vec3::new(-2.0, 0.0, -1.0));
    for distance in [0.0, 2.0] {
        let mut projector = Projector::default();
        projector.update(input(&shown, ray, 0.0, now));
        projector.update(input(&shown, ray, 1.0, now));
        let output = projector.update(Input {
            stick: Vec2::new(0.0, distance),
            dt: 0.5,
            ..input(&shown, away, 1.0, now)
        });
        assert!(text(&output).ends_with("leave it floating"));
        let hint = output.hint.as_ref().unwrap();
        let model = hint.model.unwrap();
        let canvas = hint.canvas.as_ref().unwrap();
        let centre = held(&projector).w_axis.truncate();
        let at = model.w_axis.truncate();
        let far = centre.distance(HOME);
        // Below the panel's lower edge, clear of it, and just under its middle.
        let (_, panel_height) = fitted_size(&shown.panels[1], Fit::Text);
        let (width, height) = canvas.size();
        assert!(
            at.y + height / 2.0 < centre.y - panel_height / 2.0,
            "{at} {centre}"
        );
        assert!(at.y + height / 2.0 > centre.y - panel_height / 2.0 - 0.05 * far);
        assert!(Vec2::new(at.x - centre.x, at.z - centre.z).length() < 1e-3);
        // Facing the viewer.
        let normal = model.z_axis.truncate().normalize();
        assert!(normal.dot((HOME - at).normalize()) > 0.999);
        // Its font pixel takes the same angle from the eyes however far it is carried, an x-height of about half a
        // degree, and it is not too wide to take in at a glance.
        assert!((canvas.pixel / at.distance(HOME) - HINT_ANGLE).abs() < HINT_ANGLE * 0.1);
        let x_height = (canvas.pixel * 5.6 / at.distance(HOME)).to_degrees();
        assert!(x_height >= 0.45, "{x_height}");
        assert!((width / at.distance(HOME)).atan().to_degrees() < 25.0);
        // Drawn only while the panels take the ray and grip.
        let inactive = projector.update(Input {
            active: false,
            ..input(&shown, away, 1.0, now)
        });
        assert_eq!(inactive.hint, None);
    }
    // Let go, it goes.
    let mut projector = Projector::default();
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    let output = projector.update(input(&shown, ray, 0.0, now));
    assert!(output.placed.is_some());
    assert_eq!(output.hint, None);
}

#[test]
fn the_hint_stands_above_a_panel_carried_low_where_below_would_be_hidden() {
    let shown = projection(vec![panel("a", on(TOMORI_WALL, 0))]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = at(slot_centre(&shown, 0));
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    // Carried low and near, far below the eyes, the hint goes above it.
    let low = at(HOME + Vec3::new(0.0, -1.3, -0.6));
    let output = projector.update(input(&shown, low, 1.0, now));
    let centre = projector.held.as_ref().unwrap().model.w_axis.truncate();
    let hint = output.hint.unwrap().model.unwrap().w_axis.truncate();
    assert!(hint.y > centre.y, "{hint} {centre}");
    // Ahead at the eyes' height, it is below.
    let ahead = at(HOME + Vec3::new(0.0, 0.0, -2.0));
    let output = projector.update(input(&shown, ahead, 1.0, now));
    let centre = projector.held.as_ref().unwrap().model.w_axis.truncate();
    let hint = output.hint.unwrap().model.unwrap().w_axis.truncate();
    assert!(hint.y < centre.y, "{hint} {centre}");
}

#[test]
fn a_panel_thrown_away_is_no_longer_carried_put_down_or_drawn_while_the_server_agrees() {
    let shown = projection(vec![
        panel("a", on(TOMORI_WALL, 0)),
        panel("b", on(TOMORI_WALL, 1)),
    ]);
    let now = Instant::now();
    let mut projector = Projector::default();
    // Nothing carried, nothing is thrown.
    assert_eq!(projector.throw(now), None);
    let ray = at(slot_centre(&shown, 1));
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    assert_eq!(projector.throw(now), Some("b".into()));
    assert_eq!(projector.held(), None);
    // Not drawn, pointed at or put down as the grip is let go.
    let drawn = |output: &Output| {
        output
            .sheets
            .iter()
            .filter(|sheet| {
                sheet
                    .canvas
                    .as_ref()
                    .unwrap()
                    .plain()
                    .starts_with("Panel b")
            })
            .count()
    };
    let output = projector.update(input(&shown, ray, 0.0, now));
    assert_eq!(
        (&output.placed, &output.pointed, &output.hint),
        (&None, &None, &None)
    );
    assert_eq!(drawn(&output), 0);
    // Once the server no longer shows it, it is forgotten; were the server to show it after all, it is drawn again
    // after a while.
    let output = projector.update(input(&shown, ray, 0.0, now + PENDING));
    assert_eq!(drawn(&output), 1);
    projector.update(input(&shown, ray, 1.0, now + PENDING));
    projector.throw(now + PENDING);
    let gone = projection(vec![panel("a", on(TOMORI_WALL, 0))]);
    projector.update(input(&gone, ray, 0.0, now + PENDING));
    assert_eq!(projector.thrown, None);
    // Without a projection, nothing is remembered.
    projector.update(input(&shown, ray, 1.0, now + PENDING));
    projector.throw(now + PENDING);
    projector.update(Input {
        projection: None,
        ..input(&shown, ray, 0.0, now)
    });
    assert_eq!(projector.thrown, None);
}

#[test]
fn the_server_takes_each_slot_to_be_where_it_is_drawn() {
    // The server reasons about what people look at, and where it puts the agent's panels, from where it takes each
    // slot to be, so it must agree with where this draws a panel in it, flat or curved, however the wall stands.
    let turned = |mut wall: Wall, at: [f32; 3], towards: [f32; 3]| {
        wall.pose = Pose::upright_facing(at, towards);
        wall
    };
    let flat = Wall::new("wall-1", WallKind::Added, Pose::default());
    for wall in [
        curved(),
        turned(curved(), [1.0, 0.8, 4.0], [-2.0, 0.8, 1.0]),
        turned(flat.clone(), [0.0, 1.5, -2.0], [0.0, 1.5, 0.0]),
        turned(flat, [3.0, 1.4, 1.0], [0.0, 1.6, -1.0]),
    ] {
        let drawn: Vec<Vec3> = slot_frames(&wall, model(&wall.pose).unwrap())
            .iter()
            .map(|frame| frame.transform_point3(Vec3::ZERO))
            .collect();
        let taken = tomori_server::presence::slot_centres(&wall);
        assert_eq!(drawn.len(), taken.len());
        for (drawn, taken) in drawn.iter().zip(taken) {
            assert!(
                drawn.distance(Vec3::from_array(taken)) < 0.001,
                "{drawn} {taken:?}"
            );
        }
    }
}

/// The 17 lines of JavaScript the maintainer could not see all of on the Quest 3, long enough that several wrap.
fn reported_block() -> Panel {
    let lines: Vec<String> = (1..=17)
        .map(|line| match line % 3 {
            0 => format!(
                "  const step{line} = await build(project, {{ target: 'quest', release: true }});"
            ),
            _ => format!("  log('step {line}');"),
        })
        .collect();
    Panel::new("c", "javascript", lines.join("\n"), on(TOMORI_WALL, 0))
        .with_code(Code::new("javascript", 17))
}

/// What the row below `canvas`'s code says.
fn footer(canvas: &Canvas) -> String {
    row_text(canvas.rows.last().unwrap()).trim_end().to_owned()
}

/// `panel`'s face sized as `fit` says, scrolled `scroll` rows down.
fn scrolled(panel: &Panel, fit: Fit, scroll: usize) -> Canvas {
    let layout = panel
        .code
        .as_ref()
        .map(|code| CodeLayout::new(panel, code, CODE_COLUMNS));
    scrolled_face(panel, false, false, fit, scroll, layout.as_ref())
}

#[test]
fn the_right_stick_scrolls_faster_the_further_it_is_pushed_up_for_forward() {
    assert_eq!(scroll_rate(0.0), 0.0);
    assert_eq!(scroll_rate(SCROLL_DEAD), 0.0);
    assert_eq!(scroll_rate(-SCROLL_DEAD), 0.0);
    assert_eq!(scroll_rate(1.0), -SCROLL_SPEED);
    assert_eq!(scroll_rate(-1.0), SCROLL_SPEED);
    // Past the end, as fast as at it, and as the square of how far past the dead zone in between.
    assert_eq!(scroll_rate(-2.0), SCROLL_SPEED);
    assert!((scroll_rate(-0.6) - SCROLL_SPEED / 4.0).abs() < 1e-5);
    assert!(scroll_rate(-0.4) < scroll_rate(-0.6));
    assert_eq!(scroll_rate(f32::NAN), 0.0);
}

#[test]
fn a_code_panel_scrolls_within_its_rows_saying_what_is_above_and_below() {
    let panel = reported_block();
    let layout = CodeLayout::new(&panel, panel.code.as_ref().unwrap(), CODE_COLUMNS);
    let most = layout.most_scroll(Fit::SLOT);
    assert!(most > 0);
    // At the top, the lines below that do not fit, as on the headset, and the bar's thumb at the top of its track.
    let top = scrolled(&panel, Fit::SLOT, 0);
    assert!(footer(&top).starts_with("↓ "), "{}", footer(&top));
    assert!(!footer(&top).contains('↑'));
    let [track, thumb] = top.shapes.as_slice() else {
        panic!("a bar: {:?}", top.shapes);
    };
    assert_eq!((track.color, thumb.color), (SCROLL_TRACK, SCROLL_THUMB));
    assert_eq!(thumb.y, track.y);
    assert!(thumb.height < track.height);
    // Its track is beside the rows of code, inside the face's padding.
    assert!(track.x > (CODE_COLUMNS * CELL_WIDTH) as f32);
    assert!(track.x + track.width <= (CODE_COLUMNS * CELL_WIDTH + CODE_PADDING.0) as f32);
    assert_eq!(track.y, (3 * CELL_HEIGHT) as f32);
    assert_eq!(track.height, (SLOT_CODE_ROWS * CELL_HEIGHT) as f32);
    // Scrolled to the end, the last line is the last row, nothing is below, and the lines above are counted.
    let end = scrolled(&panel, Fit::SLOT, most);
    let rows: Vec<String> = end.rows[3..CODE_ROWS - 1].iter().map(row_text).collect();
    assert_eq!(joined(&rows).last().unwrap().0, 17);
    assert!(!rows.last().unwrap().is_empty());
    let above = footer(&end);
    assert!(
        above.starts_with("↑ ") && above.ends_with(" above"),
        "{above}"
    );
    assert!(!above.contains('↓'));
    let [track, thumb] = end.shapes.as_slice() else {
        panic!("a bar");
    };
    assert!((thumb.y + thumb.height - (track.y + track.height)).abs() < 1e-3);
    // No further, however far it is asked.
    assert_eq!(scrolled(&panel, Fit::SLOT, most + 50).plain(), end.plain());
    // In between, both, every line counted once, those not whole below and those starting above.
    let middle = scrolled(&panel, Fit::SLOT, most / 2);
    let text = footer(&middle);
    let (up, down) = text.split_once(" · ").expect("above and below");
    let count = |part: &str| -> usize { part.split(' ').nth(1).unwrap().parse().unwrap() };
    assert!(up.starts_with("↑ ") && down.starts_with("↓ "), "{text}");
    let drawn = joined(
        &middle.rows[3..CODE_ROWS - 1]
            .iter()
            .map(row_text)
            .collect::<Vec<_>>(),
    );
    let whole = drawn.iter().filter(|(number, _)| *number > 0).count();
    assert!(count(up) + count(down) + whole >= 17);
    // A row scrolled to partway through a wrapped line begins with its continuation, unnumbered.
    let wrapped = layout
        .rows
        .iter()
        .position(|row| !row.first)
        .expect("a line wraps");
    let partway = scrolled(&panel, Fit::SLOT, wrapped);
    assert!(row_text(&partway.rows[3]).starts_with(" ↪ "));
    assert!(footer(&partway).starts_with(&format!("↑ {} above", layout.rows[wrapped].line + 1)));
    // Every row of every scroll is within the panel's columns, and the face the same size.
    for scroll in 0..=most {
        let canvas = scrolled(&panel, Fit::SLOT, scroll);
        assert_eq!(canvas.rows.len(), CODE_ROWS);
        assert!(
            canvas
                .rows
                .iter()
                .all(|row| row.cells.len() <= CODE_COLUMNS)
        );
    }
    // A panel that shows all of itself has no bar and nothing to scroll.
    let short = code_panel("s", 3);
    let layout = CodeLayout::new(&short, short.code.as_ref().unwrap(), CODE_COLUMNS);
    assert_eq!(layout.most_scroll(Fit::SLOT), 0);
    assert!(scrolled(&short, Fit::SLOT, 5).shapes.is_empty());
    assert_eq!(footer(&scrolled(&short, Fit::SLOT, 5)), "");
}

#[test]
fn a_code_panel_says_how_many_lines_it_was_not_sent_below_the_last_it_holds() {
    // A block of 200 lines of which the server sent 30, as where a projection would not fit in a frame.
    let body: Vec<String> = (1..=30).map(|line| format!("line {line}")).collect();
    let panel =
        Panel::new("c", "text", body.join("\n"), Place::Unknown).with_code(Code::new("", 200));
    // Those below, sent or not, are counted as more until the last it holds is shown; then those it was not sent, as
    // only in the transcript.
    assert_eq!(footer(&scrolled(&panel, Fit::SLOT, 0)), "↓ 184 more");
    assert_eq!(
        footer(&scrolled(&panel, Fit::SLOT, usize::MAX)),
        "↑ 14 above · 170 more lines in the transcript"
    );
    // Floating, it shows every line it holds, and says so.
    let floating = scrolled(&panel, Fit::Text, 0);
    assert_eq!(floating.rows.len(), 3 + 30 + 1);
    assert_eq!(footer(&floating), "170 more lines in the transcript");
    assert!(floating.shapes.is_empty());
    // A line the server cut ends with a dim "…".
    let cut = Panel::new("c", "text", "a\nbcd…", Place::Unknown).with_code(Code::new("", 9));
    let canvas = scrolled(&cut, Fit::SLOT, 0);
    assert_eq!(
        canvas.rows[4].cells.last().unwrap(),
        &('…', Style::plain(DIM))
    );
    assert_eq!(footer(&canvas), "7 more lines in the transcript");
}

#[test]
fn a_text_panel_longer_than_its_face_scrolls_too() {
    let body: Vec<String> = (1..=20).map(|line| format!("- step {line}")).collect();
    let panel = Panel::new("t", "Steps", body.join("\n"), on(TOMORI_WALL, 0));
    let most = most_scroll(&panel, Fit::SLOT, None);
    assert_eq!(most, 20 - BODY_ROWS);
    // At the top, cut with "…", with a bar beside the body; at the end, its last row shown whole.
    let top = scrolled(&panel, Fit::SLOT, 0);
    assert!(row_text(&top.rows[3 + BODY_ROWS - 1]).ends_with('…'));
    assert_eq!(top.shapes.len(), 2);
    assert_eq!(top.shapes[0].height, (BODY_ROWS * CELL_HEIGHT) as f32);
    let end = scrolled(&panel, Fit::SLOT, most + 3);
    assert_eq!(row_text(&end.rows[3]).trim(), "• step 9");
    assert_eq!(row_text(&end.rows[3 + BODY_ROWS - 1]).trim(), "• step 20");
    assert_eq!(end.rows.len(), 3 + BODY_ROWS);
    // Floating, it fits, and has nothing to scroll.
    assert_eq!(most_scroll(&panel, Fit::Text, None), 0);
    assert!(scrolled(&panel, Fit::Text, 4).shapes.is_empty());
}

/// The faces drawn in `output`, as text.
fn faces(output: &Output) -> Vec<String> {
    output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_ref())
        .map(Canvas::plain)
        .collect()
}

#[test]
fn each_participant_scrolls_a_panel_for_themselves_alone() {
    let now = Instant::now();
    let shown = projection(vec![reported_block(), panel("t", on(TOMORI_WALL, 1))]);
    // Two participants see the same projection.
    let (mut mine, mut theirs) = (Projector::default(), Projector::default());
    let before = faces(&mine.update(input(&shown, None, 0.0, now)));
    theirs.update(input(&shown, None, 0.0, now));
    assert!(mine.scrollable("c"));
    assert!(!mine.scrollable("t"));
    assert!(!mine.scrollable("nothing"));
    // Scrolling mine moves only my view of it, sends nothing, and leaves the other panel and theirs where they were.
    let scroll = |rows: f32| Input {
        scroll: Some(("c", rows)),
        ..input(&shown, None, 0.0, now)
    };
    let output = mine.update(scroll(2.4));
    assert_eq!(mine.scroll("c"), 2);
    assert_eq!(output.placed, None);
    assert!(!output.picked);
    let after = faces(&output);
    assert_ne!(after, before);
    assert!(after.iter().any(|face| face.contains("↑ ")));
    assert_eq!(
        after
            .iter()
            .filter(|face| face.starts_with("Panel t"))
            .collect::<Vec<_>>(),
        before
            .iter()
            .filter(|face| face.starts_with("Panel t"))
            .collect::<Vec<_>>()
    );
    assert_eq!(faces(&theirs.update(input(&shown, None, 0.0, now))), before);
    assert_eq!(theirs.scroll("c"), 0);
    // It stops at either end, however far it is asked to go, and scrolls back from there at once.
    let most = mine.limits["c"];
    mine.update(scroll(1_000.0));
    assert_eq!(mine.scroll("c"), most);
    mine.update(scroll(-1.0));
    assert_eq!(mine.scroll("c"), most - 1);
    mine.update(scroll(-1_000.0));
    assert_eq!(mine.scroll("c"), 0);
    mine.update(scroll(f32::NAN));
    assert_eq!(mine.scroll("c"), 0);
    // A panel that shows all of itself does not scroll.
    mine.update(Input {
        scroll: Some(("t", 5.0)),
        ..input(&shown, None, 0.0, now)
    });
    assert_eq!(mine.scroll("t"), 0);
    // Its code is laid out once, and again only when it changes.
    let laid = Arc::clone(&mine.layouts["c"].layouts[0]);
    mine.update(input(&shown, None, 0.0, now));
    assert!(Arc::ptr_eq(&laid, &mine.layouts["c"].layouts[0]));
    let mut changed = shown.clone();
    changed.panels[0].body.push_str("\n  done();");
    mine.update(input(&changed, None, 0.0, now));
    assert!(!Arc::ptr_eq(&laid, &mine.layouts["c"].layouts[0]));
    // Gone, it is forgotten, as is everything once nothing is projected.
    mine.update(scroll(3.0));
    let gone = projection(vec![panel("t", on(TOMORI_WALL, 1))]);
    mine.update(input(&gone, None, 0.0, now));
    assert!(!mine.scrolls.contains_key("c") && mine.layouts.is_empty());
    mine.update(Input {
        projection: None,
        ..input(&shown, None, 0.0, now)
    });
    assert!(mine.limits.is_empty());
}

#[test]
fn a_code_panel_grows_to_show_its_code_floating_and_takes_its_slot_s_size_and_scroll_again_on_a_wall()
 {
    let now = Instant::now();
    let mut on_wall = reported_block();
    let shown = projection(vec![on_wall.clone()]);
    let mut projector = Projector::default();
    projector.update(input(&shown, None, 0.0, now));
    projector.update(Input {
        scroll: Some(("c", 4.0)),
        ..input(&shown, None, 0.0, now)
    });
    assert_eq!(projector.scroll("c"), 4);
    // Pulled off and carried, it is as tall as its code, and pointed at at that size, its hint below it.
    let centre = slot_centre(&shown, 0);
    projector.update(input(&shown, at(centre), 0.0, now));
    let output = projector.update(input(&shown, at(centre), 1.0, now));
    assert!(output.picked);
    let output = projector.update(input(&shown, at(centre), 1.0, now));
    let carried = output
        .sheets
        .iter()
        .find(|sheet| {
            sheet
                .canvas
                .as_ref()
                .unwrap()
                .plain()
                .starts_with("javascript")
        })
        .unwrap();
    let canvas = carried.canvas.as_ref().unwrap();
    assert!(canvas.size().1 > panel_size().1);
    assert_eq!(canvas.size(), fitted_size(&on_wall, Fit::Text));
    assert!(!projector.scrollable("c"));
    let drawn = joined(&canvas.rows[3..].iter().map(row_text).collect::<Vec<_>>());
    assert_eq!(drawn.first().unwrap().0, 1);
    assert_eq!(drawn.last().unwrap().0, 17);
    let hint = output.hint.unwrap().model.unwrap().w_axis.truncate();
    assert!(hint.y < centre.y - canvas.size().1 / 2.0);
    // Left floating, it stays so; the server agrees.
    projector.update(input(&shown, at(centre + Vec3::X * 2.0), 0.0, now));
    on_wall.place = Place::Free {
        pose: Pose::upright_facing((centre + Vec3::X * 2.0).to_array(), HOME.to_array()),
    };
    let floating = projection(vec![on_wall.clone()]);
    projector.update(input(&floating, None, 0.0, now + PENDING));
    assert!(!projector.scrollable("c"));
    assert_eq!(projector.scroll("c"), 0);
    // Put back on the wall, it is a slot's size again, scrolled as it was.
    on_wall.place = on(TOMORI_WALL, 0);
    let back = projection(vec![on_wall]);
    let output = projector.update(input(&back, None, 0.0, now + PENDING));
    assert!(projector.scrollable("c"));
    assert_eq!(projector.scroll("c"), 4);
    let canvas = output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_ref())
        .find(|canvas| canvas.plain().starts_with("javascript"))
        .unwrap();
    assert_eq!(canvas.rows.len(), CODE_ROWS);
    assert!(footer(canvas).starts_with("↑ "));
}

#[test]
fn the_ray_s_label_says_how_to_scroll_a_panel_that_scrolls() {
    let lines = |scrolls| -> Vec<String> {
        label(&reported_block(), Vec3::new(0.0, 1.5, -3.0), HOME, scrolls)
            .canvas
            .unwrap()
            .plain()
            .lines()
            .map(|line| line.trim_end().to_owned())
            .collect()
    };
    assert_eq!(lines(None).len(), 3);
    assert_eq!(
        lines(Some(Controls::Controllers)).last().unwrap(),
        "Stick ↕  scroll"
    );
    assert_eq!(
        lines(Some(Controls::Keyboard)).last().unwrap(),
        "Wheel  scroll"
    );
}
