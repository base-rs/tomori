// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The faces of projected panels are laid out once for what each shows, and drawn again only as that, its size, its
//! scroll or its frame changes: never as walls move or tilt, the head moves or the ray sweeps.
use std::time::Instant;

use glam::{Mat4, Vec2, Vec3};
use tomori_protocol::{
    Code, Image, Panel, Place, Pose, Projection, Span, TOMORI_WALL, TokenKind, Wall, WallKind,
    WallShape,
};

use super::super::{
    Controls, FRAME, HIGHLIGHT, Input, NEWEST, Projector, code_columns, editing, model,
    scrolled_face, slot_frames,
};
use super::*;
use crate::{arrival::HOME, pointing::Ray};

/// How many layouts this thread has laid out, and how many faces it has drawn from them.
fn counts() -> (usize, usize) {
    COUNTS.with(std::cell::Cell::get)
}

fn on(wall: &str, slot: u32) -> Place {
    Place::Wall {
        wall: wall.into(),
        slot,
    }
}

const LONG: &str = "## Notes\n\nA paragraph long enough to wrap across the panel's rows many times over, so that the panel has more than it shows at once and scrolls, which it does by a whole row at a time, for whoever scrolls it alone, while everyone else sees it from its top.\n\n- one\n- two\n- three\n- four\n- five\n- six\n- seven\n- eight";

/// Text, code, a table, a checklist, a chart and an image, on the wall `wall-1`, and a text panel floating.
fn panels() -> Vec<Panel> {
    let code_body = "fn main() {\n    let answer = 42;\n    println!(\"{answer}\");\n}";
    let code = Code::new("rust", 4).with_spans(vec![Span::new(0, 2, TokenKind::Keyword)]);
    let chart = crate::chart::fixtures::pressure();
    vec![
        Panel::new("text", "Notes", LONG, on("wall-1", 0)).with_age(30),
        Panel::new("code", "main.rs", code_body, on("wall-1", 1)).with_code(code),
        Panel::new(
            "table",
            "Tests",
            "| A | B |\n|---|---|\n| 1 | 2 |",
            on("wall-1", 2),
        ),
        Panel::new("list", "Agenda", "- [x] one\n- [ ] two", on("wall-1", 3)).with_age(60),
        Panel::new("chart", "Pressure", chart.summary(), on("wall-1", 4)).with_chart(chart),
        Panel::new("image", "Diagram", "A diagram.", on("wall-1", 5)).with_image(Image::new(
            "0123456789abcdef".repeat(4),
            800,
            600,
            1_000,
        )),
        Panel::new(
            "floating",
            "Aside",
            "Floating, it fits its text.",
            Place::Free {
                pose: Pose::upright_facing([0.6, 1.5, -1.2], HOME.to_array()),
            },
        ),
    ]
}

fn wall(pose: Pose) -> Wall {
    Wall::new("wall-1", WallKind::Added, pose)
}

fn projection(panels: Vec<Panel>) -> Projection {
    Projection::new("s", "sky")
        .with_walls(vec![
            Wall::new(
                TOMORI_WALL,
                WallKind::Tomori,
                Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
            )
            .with_shape(WallShape::Arc { radius: 2.3 }),
            wall(Pose::upright_facing([0.0, 1.5, -3.0], [0.0, 1.5, 0.0])),
        ])
        .with_panels(panels)
}

fn input(projection: &Projection, viewer: Vec3, ray: Option<Ray>) -> Input<'_> {
    Input {
        projection: Some(projection),
        ray,
        grip: Some(0.0),
        viewer: Some(viewer),
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        free: false,
        controls: Controls::Controllers,
        scroll: None,
        rights: editing::Rights::default(),
        now: Instant::now(),
        dt: 1.0 / 72.0,
    }
}

/// The face `panel` showed last, as it was last sized.
fn face(projector: &Projector, panel: &str) -> Arc<Canvas> {
    let face = projector.faces[panel].sizes[0].face.as_ref();
    face.expect("drawn").canvas.clone()
}

/// The faces of every panel, by identity, as they showed last.
fn every_face(projector: &Projector, projection: &Projection) -> Vec<Arc<Canvas>> {
    projection
        .panels
        .iter()
        .map(|panel| face(projector, &panel.id))
        .collect()
}

/// Whether each of `faces` is, as shared, the one before it in `before`.
fn kept(faces: &[Arc<Canvas>], before: &[Arc<Canvas>]) -> Vec<bool> {
    faces
        .iter()
        .zip(before)
        .map(|(face, before)| Arc::ptr_eq(face, before))
        .collect()
}

/// Where the middle of `wall-1`'s `slot` is.
fn slot_middle(projection: &Projection, slot: usize) -> Vec3 {
    let wall = projection.wall("wall-1").unwrap();
    slot_frames(wall, model(&wall.frame()).unwrap())[slot]
        .w_axis
        .truncate()
}

#[test]
fn a_frame_lays_out_and_draws_nothing_unless_something_changed() {
    let shown = projection(panels());
    let mut projector = Projector::default();
    let (laid, drawn) = counts();
    let output = projector.update(input(&shown, HOME, None));
    // Each panel is laid out once, at the size it stands, and its face drawn once.
    assert_eq!(counts(), (laid + 7, drawn + 7));
    let before = every_face(&projector, &shown);
    // The frame hands over each face as it is kept, shared, rather than a copy.
    for face in &before {
        assert!(output.sheets.iter().any(|sheet| {
            sheet
                .canvas
                .as_ref()
                .is_some_and(|canvas| Arc::ptr_eq(canvas, face))
        }));
    }
    // Each face is what drawing it afresh draws.
    for panel in &shown.panels {
        let fit = projector.faces[&panel.id].sizes[0].fit;
        let layout = panel
            .code
            .as_ref()
            .map(|code| CodeLayout::new(panel, code, code_columns(fit)));
        let fresh = scrolled_face(panel, false, panel.id == "image", fit, 0, layout.as_ref());
        assert_eq!(*face(&projector, &panel.id), fresh, "{}", panel.id);
    }
    // Frame after frame, as the head moves, nothing is laid out or drawn again.
    for frame in 0..30 {
        let bob = Vec3::new(
            0.02 * (frame as f32).sin(),
            0.01 * (frame as f32).cos(),
            0.0,
        );
        let output = projector.update(input(&shown, HOME + bob, None));
        assert!(!output.sheets.is_empty());
    }
    assert_eq!(counts(), (laid + 7, drawn + 7));
    assert!(
        kept(&every_face(&projector, &shown), &before)
            .iter()
            .all(|kept| *kept)
    );
}

#[test]
fn moving_or_tilting_a_wall_moves_its_panels_without_laying_them_out() {
    let mut shown = projection(panels());
    let mut projector = Projector::default();
    let first = projector.update(input(&shown, HOME, None));
    let before = every_face(&projector, &shown);
    let counted = counts();
    let models = |output: &crate::projection::Output, face: &Arc<Canvas>| {
        output
            .sheets
            .iter()
            .find(|sheet| {
                sheet
                    .canvas
                    .as_ref()
                    .is_some_and(|canvas| Arc::ptr_eq(canvas, face))
            })
            .and_then(|sheet| sheet.model)
    };
    shown.walls[1] =
        wall(Pose::upright_facing([1.0, 1.4, -2.5], [0.0, 1.5, 0.0])).with_tilt(Some(0.3));
    let moved = projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), counted);
    assert!(
        kept(&every_face(&projector, &shown), &before)
            .iter()
            .all(|kept| *kept)
    );
    // Its panels' faces stand elsewhere, only their models changed.
    assert_ne!(models(&first, &before[0]), models(&moved, &before[0]));
}

#[test]
fn pointing_at_a_panel_recolours_its_frame_without_drawing_it_again() {
    let shown = projection(panels());
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let before = every_face(&projector, &shown);
    let counted = counts();
    // The ray meets the table, the third panel.
    let ray = Ray::new(HOME, slot_middle(&shown, 2) - HOME);
    let output = projector.update(input(&shown, HOME, ray));
    assert_eq!(
        output.pointed.as_ref().map(|(id, ..)| id.as_str()),
        Some("table")
    );
    assert_eq!(counts(), counted, "nothing laid out or drawn again");
    let pointed = every_face(&projector, &shown);
    assert_eq!(
        kept(&pointed, &before),
        [true, true, false, true, true, true, true]
    );
    assert_eq!(pointed[2].border, Some(HIGHLIGHT));
    let mut framed = (*before[2]).clone();
    framed.border = Some(HIGHLIGHT);
    assert_eq!(*pointed[2], framed);
    // Pointed away, it is framed as before, and the newest panel on the wall, the image, in blue.
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), counted);
    assert_eq!(face(&projector, "table").border, Some(FRAME));
    assert_eq!(face(&projector, "image").border, Some(NEWEST));
    // A panel newer still takes the blue frame from it, which is only recoloured.
    let mut newer = shown.clone();
    newer
        .panels
        .push(Panel::new("new", "New", "Just now.", on(TOMORI_WALL, 0)));
    newer
        .panels
        .push(Panel::new("newest", "Newest", "Just now.", on("wall-1", 5)).with_age(0));
    newer.panels.retain(|panel| panel.id != "image");
    projector.update(input(&newer, HOME, None));
    assert_eq!(
        counts(),
        (counted.0 + 2, counted.1 + 2),
        "the new panels alone"
    );
    assert_eq!(face(&projector, "newest").border, Some(NEWEST));
    assert!(
        !projector.faces.contains_key("image"),
        "forgotten once removed"
    );
}

#[test]
fn scrolling_draws_the_face_again_from_its_layout_without_laying_it_out() {
    let shown = projection(panels());
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let before = face(&projector, "text");
    let (laid, drawn) = counts();
    let mut scrolled = |rows: f32| {
        projector.update(Input {
            scroll: Some(("text", rows)),
            ..input(&shown, HOME, None)
        });
    };
    // Less than a row: nothing changes.
    scrolled(0.3);
    assert_eq!(counts(), (laid, drawn));
    scrolled(1.0);
    assert_eq!(counts(), (laid, drawn + 1));
    assert_eq!(projector.scroll("text"), 1);
    let after = face(&projector, "text");
    assert_ne!(*after, *before);
    assert_eq!(
        *after,
        scrolled_face(
            &shown.panels[0],
            false,
            false,
            projector.faces["text"].sizes[0].fit,
            1,
            None
        )
    );
}

#[test]
fn what_a_panel_shows_or_its_size_changing_lays_it_out_again() {
    let mut shown = projection(panels());
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let before = every_face(&projector, &shown);
    let (laid, drawn) = counts();
    // The server's patch upserts the text panel with another body: it alone is laid out again.
    shown.panels[0].body.push_str("\n- nine");
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 1, drawn + 1));
    assert_eq!(
        kept(&every_face(&projector, &shown), &before),
        [false, true, true, true, true, true, true]
    );
    // Only its chart's data changing, the chart panel is drawn again, but not laid out.
    let mut plotted = shown.clone();
    plotted.panels[4].chart = Some(Box::new(crate::chart::fixtures::provisioning()));
    projector.update(input(&plotted, HOME, None));
    assert_eq!(counts(), (laid + 1, drawn + 2));
    assert_ne!(face(&projector, "chart"), before[4]);
    // Its summary changing with its data, it is laid out again too.
    plotted.panels[4].body = "Provisioning.".into();
    projector.update(input(&plotted, HOME, None));
    assert_eq!(counts(), (laid + 2, drawn + 3));
    // Its wall's slots made taller, each of its panels is laid out at their size.
    plotted.walls[1] = plotted.walls[1].clone().with_aspect(Some(0.5));
    projector.update(input(&plotted, HOME, None));
    assert_eq!(counts(), (laid + 8, drawn + 9));
}

/// A patch counts on the age of every panel held, in seconds, but a panel shows its age in minutes, hours or days: one
/// whose age shows the same is not laid out again, and one whose age passes another minute is.
#[test]
fn an_age_counted_on_lays_a_panel_out_again_only_where_it_shows_otherwise() {
    let mut shown = projection(panels());
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let before = every_face(&projector, &shown);
    let (laid, drawn) = counts();
    // A few seconds on, as when the next patch arrives, every panel shows the same age.
    for panel in &mut shown.panels {
        panel.age += 5;
    }
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid, drawn));
    assert!(
        kept(&every_face(&projector, &shown), &before)
            .iter()
            .all(|kept| *kept)
    );
    // The checklist, a minute old, passes to two minutes: it alone is laid out again, and shows it.
    shown.panels[3].age = 120;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 1, drawn + 1));
    assert_eq!(
        kept(&every_face(&projector, &shown), &before),
        [true, true, true, false, true, true, true]
    );
    assert!(face(&projector, "list").plain().contains("2m"));
}

#[test]
fn a_panel_keeps_its_faces_at_a_few_sizes_the_latest_first() {
    let panel = Panel::new("text", "Notes", LONG, on("wall-1", 0));
    let mut faces = Faces::new(&panel);
    let sizes: Vec<Fit> = [34, 40, 46, 52]
        .into_iter()
        .map(|columns| {
            Fit::Slot(super::super::Cells {
                columns,
                ..super::super::Cells::STANDARD
            })
        })
        .collect();
    let (laid, _) = counts();
    for fit in &sizes {
        faces.at(&panel, *fit);
    }
    assert_eq!(counts().0, laid + 4);
    assert_eq!(faces.sizes.len(), SIZES);
    // The first is forgotten, the others kept: going back to one kept lays nothing out, and puts it first.
    faces.at(&panel, sizes[1]);
    assert_eq!(counts().0, laid + 4);
    assert_eq!(faces.sizes[0].fit, sizes[1]);
    faces.at(&panel, sizes[0]);
    assert_eq!(counts().0, laid + 5);
    // A code panel floating is as tall as its code, and points at no item.
    let code = Panel::new("code", "main.rs", "fn main() {}", on("wall-1", 1))
        .with_code(Code::new("rust", 1));
    let mut faces = Faces::new(&code);
    let floating = faces.at(&code, Fit::Text);
    assert_eq!(floating.size, super::super::fitted_size(&code, Fit::Text));
    assert_eq!(floating.item(Mat4::IDENTITY, 0, Vec3::ZERO), None);
}

/// From a server that versions panels, a panel's version and data seq are its faces' keys: a new version lays it out
/// again, and a chart's new data seq draws its face again from its new points without laying it out, while what the
/// client changes itself, a tick shown at once, is still followed.
#[test]
fn a_versioned_panel_is_laid_out_again_for_its_version_and_drawn_again_for_its_data() {
    let mut shown = projection(
        panels()
            .into_iter()
            .map(|panel| {
                let seq = if panel.chart.is_some() { 5 } else { 0 };
                panel.with_version(5).with_data_seq(seq)
            })
            .collect(),
    );
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let before = every_face(&projector, &shown);
    let (laid, drawn) = counts();
    // The chart's points and summary move on at a new data seq, its version as it was: its face is drawn again from
    // them, and nothing laid out.
    let chart = crate::chart::fixtures::provisioning();
    shown.panels[4].body = chart.summary();
    shown.panels[4].chart = Some(Box::new(chart));
    shown.panels[4].data_seq = 6;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid, drawn + 1));
    assert_eq!(
        kept(&every_face(&projector, &shown), &before),
        [true, true, true, true, false, true, true]
    );
    // The checklist ticked at once, its version as the server sent it, is laid out again, as it then is at its next
    // version.
    shown.panels[3].body = "- [x] one\n- [x] two".into();
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 1, drawn + 2));
    shown.panels[3].version = 7;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 2, drawn + 3));
    // At the same version and body, nothing else is looked at: its version says it is as it was.
    shown.panels[0].title = "Renamed, as no server sends without a version".into();
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 2, drawn + 3));
    // A new version, as of its title, lays it out again.
    shown.panels[0].version = 8;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 3, drawn + 4));
    assert!(face(&projector, "text").plain().contains("Renamed"));
}
