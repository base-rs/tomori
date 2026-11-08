// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::{Meta, TOMORI_WALL};

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
    model(&projection.wall(TOMORI_WALL).unwrap().pose)
        .unwrap()
        .transform_point3(slot_offset(slot))
}

#[test]
fn a_wall_holds_its_slots_in_three_columns_of_two_the_first_at_the_top_left() {
    let (wall_width, wall_height) = wall_size();
    let (width, height) = panel_size();
    assert!((wall_width - 1.995).abs() < 0.001, "{wall_width}");
    assert!((wall_height - 0.878).abs() < 0.001, "{wall_height}");
    assert!((width - 0.604).abs() < 0.001 && (height - 0.361).abs() < 0.001);
    let offsets: Vec<Vec3> = (0..WALL_SLOTS).map(slot_offset).collect();
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
fn a_panel_shows_its_title_metadata_and_body_as_markdown_lite() {
    let shown = face(&panel("1", on(TOMORI_WALL, 0)), false, false);
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
        face(&panel("1", on(TOMORI_WALL, 0)), true, true).border,
        Some(HIGHLIGHT)
    );
    assert_eq!(
        face(&panel("1", on(TOMORI_WALL, 0)), false, true).border,
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
    let text = face(&long, false, false).plain();
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
    let canvas = face(&code_panel("c", 3), false, false);
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
        face(&code_panel("c", 3), true, false).border,
        Some(HIGHLIGHT)
    );
    assert_eq!(face(&code_panel("c", 3), false, true).border, Some(NEWEST));
}

#[test]
fn a_long_code_panel_says_how_many_more_lines_it_has_and_cuts_long_lines() {
    let long = "x".repeat(tomori_protocol::CODE_LINE_CHARS);
    let body = std::iter::once(format!("{long}…"))
        .chain((2..=CODE_LINES).map(|line| format!("line {line}")))
        .collect::<Vec<_>>()
        .join("\n");
    let mut panel = Panel::new("c", "text", body, Place::Unknown)
        .with_code(Code::new("", 40))
        .with_pinned(true);
    let canvas = face(&panel, false, false);
    let text = canvas.plain();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines[1].trim_end(), "40 lines · 0m · pinned");
    // Cut to the panel, with a dim "…".
    assert_eq!(lines[3].chars().count(), CODE_COLUMNS);
    assert!(lines[3].ends_with('…'));
    assert_eq!(canvas.rows[3].cells.last().unwrap().1.color, DIM);
    assert_eq!(lines[3 + CODE_LINES - 1].trim_end(), "16 line 16");
    assert_eq!(lines[CODE_ROWS - 1].trim_end(), "…24 more lines");
    // One more line is said as one.
    panel.code = Some(Code::new("", CODE_LINES as u32 + 1));
    assert!(face(&panel, false, false).plain().ends_with("…1 more line"));
    // A code panel of one line says so; a server's lines beyond what a panel holds are not drawn.
    let one = Panel::new("c", "sh", "ls", Place::Unknown).with_code(Code::new("sh", 1));
    assert!(
        face(&one, false, false)
            .plain()
            .contains("sh · 1 line · 0m")
    );
    let many = Panel::new("c", "sh", vec!["x"; 40].join("\n"), Place::Unknown)
        .with_code(Code::new("sh", 40));
    let text = face(&many, false, false).plain();
    assert_eq!(text.split('\n').count(), CODE_ROWS);
    assert!(text.ends_with("…24 more lines"));
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
        let canvas = face(&panel, false, false);
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
        label(panel, Vec3::new(0.0, 1.5, -3.0), HOME)
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
            "rust · lines 1–16 of 40 · pinned",
            "5m ago, on the Tomori wall"
        ]
    );
    let mut plain = code_panel("c", 2);
    plain.code = Some(Code::new("", 2));
    assert_eq!(lines(&plain)[1], "2 lines");
    // A text panel pinned by being put back says nothing of it, as only code panels are replaced.
    let text = panel("1", on(TOMORI_WALL, 0)).with_pinned(true);
    assert!(!lines(&text).concat().contains("pinned"));
    assert!(!face(&text, false, false).plain().contains("pinned"));
}

#[test]
fn the_ray_s_label_names_a_panel_with_its_metadata_and_where_it_is() {
    let point = Vec3::new(0.0, 1.5, -3.0);
    let label_of = |place: Place| {
        let sheet = label(&panel("1", place), point, HOME);
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
    let near = added.transform_point3(Vec3::new(wall_size().0 / 2.0 + 0.1, 0.0, 0.3));
    let distance = HOME.distance(slot_centre(&shown, 0));
    let direction = (near - HOME).normalize();
    let toward = Ray::new(near - direction * distance, direction);
    projector.update(input(&shown, toward, 1.0, now));
    let output = projector.update(input(&shown, toward, 0.0, now));
    assert_eq!(
        output.placed,
        Some(("a".to_owned(), Target::Wall("wall-1".to_owned())))
    );
    // Shown where it was let go until the server puts it in the wall's layout.
    let moved = projection(vec![panel("a", on("wall-1", 0))]);
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
    // Forward for a fifth of a second pushes it further along the ray.
    let output = projector.update(stick(Vec2::Y, 0.2));
    let pushed = distance * (RATE * 0.2).exp();
    assert!((output.end.unwrap() - ray.unwrap().at(pushed)).length() < 1e-3);
    // Back, long enough, pulls it to the nearest it goes; forward, to the farthest.
    projector.update(stick(Vec2::NEG_Y, 10.0));
    let output = projector.update(stick(Vec2::ZERO, 0.0));
    assert!((output.end.unwrap() - ray.unwrap().at(NEAREST)).length() < 1e-3);
    let output = projector.update(stick(Vec2::Y, 10.0));
    assert!((output.end.unwrap() - ray.unwrap().at(FARTHEST)).length() < 1e-3);
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
    // Turned and let go away from any wall, it floats turned, upright, as the server is told.
    projector.update(stick(Vec2::NEG_X, 0.1));
    projector.update(stick(Vec2::ZERO, 0.0));
    let output = projector.update(input(&shown, ray, 0.0, now));
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
    // Pulled off and let go between the walls, nearer the far one, it snaps onto that.
    projector.update(input(&shown, ray, 1.0, now));
    let between = Ray::new(HOME - Vec3::Z * (3.2 - 1.5), -Vec3::Z);
    projector.update(input(&shown, between, 1.0, now));
    let output = projector.update(input(&shown, between, 0.0, now));
    assert_eq!(
        output.placed,
        Some(("before".to_owned(), Target::Wall("far".to_owned())))
    );
}

#[test]
fn where_the_server_says_a_panel_is_agrees_with_where_it_was_put() {
    let pose = Pose::upright_facing([1.0, 1.5, -1.0], [0.0, 1.5, 0.0]);
    assert!(agrees(&on("w", 0), &Target::Wall("w".into())));
    assert!(!agrees(&on("w", 0), &Target::Wall("v".into())));
    assert!(agrees(&Place::Free { pose }, &Target::Free(pose)));
    let moved = Pose {
        position: [1.5, 1.5, -1.0],
        ..pose
    };
    assert!(!agrees(&Place::Free { pose: moved }, &Target::Free(pose)));
    assert!(!agrees(&Place::Free { pose }, &Target::Wall("w".into())));
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
