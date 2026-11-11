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
    Cells, Controls, FRAME, HIGHLIGHT, Input, NEWEST, Projector, code_columns, editing, model,
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

#[test]
fn a_wall_s_content_scale_lays_its_panels_out_again_at_their_cells_drawn_at_its_text_s_size_and_kept()
 {
    let mut shown = projection(panels());
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let standard = face(&projector, "text");
    let (laid, drawn) = counts();
    let model_of = |output: &crate::projection::Output, face: &Arc<Canvas>| {
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
            .expect("drawn")
    };
    // Its text drawn at half, each of the wall's six panels is laid out again, once, at twice as many columns and
    // rows, and drawn at half the size, filling the same slot.
    shown.walls[1] = shown.walls[1].clone().with_content_scale(Some(0.5));
    let output = projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 6, drawn + 6));
    let cells = Cells::of(&shown.walls[1]);
    assert_eq!(projector.faces["text"].sizes[0].fit, Fit::Slot(cells));
    assert!(cells.columns >= 2 * Cells::STANDARD.columns, "{cells:?}");
    assert!(
        cells.code_columns >= 2 * Cells::STANDARD.code_columns,
        "{cells:?}"
    );
    let smaller = face(&projector, "text");
    let model = model_of(&output, &smaller);
    assert!((model.x_axis.truncate().length() - 0.5).abs() < 1e-4);
    let (slot_width, slot_height) = shown.walls[1].slot_size();
    let (width, height) = smaller.size();
    assert!(
        width * 0.5 <= slot_width + 1e-4 && height * 0.5 <= slot_height + 1e-4,
        "{width} {height}"
    );
    assert!(
        width * 0.5 > slot_width * 0.95,
        "it fills its slot: {width}"
    );
    // The same again lays out and draws nothing, and back as large as its slots give it, its faces at that size,
    // kept, are shown again as they were.
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 6, drawn + 6));
    shown.walls[1] = shown.walls[1].clone().with_content_scale(None);
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 6, drawn + 6));
    assert!(Arc::ptr_eq(&face(&projector, "text"), &standard));
    // Drawn larger, at twice, each slot holds fewer cells than a standard one, but always a few words and some code.
    shown.walls[1] = shown.walls[1].clone().with_content_scale(Some(2.0));
    projector.update(input(&shown, HOME, None));
    let larger = Cells::of(&shown.walls[1]);
    assert_eq!(projector.faces["text"].sizes[0].fit, Fit::Slot(larger));
    assert!(larger.columns < Cells::STANDARD.columns && larger.body < Cells::STANDARD.body);
    assert!(larger.code_columns > super::super::GUTTER + super::super::CODE_ROOM - 1);
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
        faces.at(*fit);
    }
    assert_eq!(counts().0, laid + 4);
    assert_eq!(faces.sizes.len(), SIZES);
    // The first is forgotten, the others kept: going back to one kept lays nothing out, and puts it first.
    faces.at(sizes[1]);
    assert_eq!(counts().0, laid + 4);
    assert_eq!(faces.sizes[0].fit, sizes[1]);
    faces.at(sizes[0]);
    assert_eq!(counts().0, laid + 5);
    // A code panel floating is as tall as its code, and points at no item.
    let code = Panel::new("code", "main.rs", "fn main() {}", on("wall-1", 1))
        .with_code(Code::new("rust", 1));
    let mut faces = Faces::new(&code);
    let floating = faces.at(Fit::Text);
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
    // At the same version, body and title, nothing else is looked at: its version says it is as it was.
    shown.panels[0].meta.push(tomori_protocol::Meta::new(
        "source",
        "as no server sends without a version",
    ));
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 2, drawn + 3));
    // A new version lays it out again.
    shown.panels[0].version = 8;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 3, drawn + 4));
    // Its title is followed at the same version, since a server before versions counted no change its environment
    // made of a panel.
    shown.panels[0].title = "Renamed".into();
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 4, drawn + 5));
    assert!(face(&projector, "text").plain().contains("Renamed"));
}

/// From a server before versions, which counted each panel's changes as its revision but not its environment's changes
/// of it, a chart panel with a revision and no data seq is followed as it shows, its points moving on drawn again.
#[test]
fn a_chart_panel_from_a_server_before_versions_is_followed_by_its_points() {
    let mut shown = projection(
        panels()
            .into_iter()
            .map(|panel| panel.with_version(1))
            .collect(),
    );
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let (laid, drawn) = counts();
    // Its points move on with its summary as it was, at the same revision.
    let mut chart = shown.panels[4].chart.clone().unwrap();
    chart.series[0].values.rotate_left(1);
    shown.panels[4].chart = Some(chart);
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid, drawn + 1));
}

/// As many charts as an environment may publish, 32, on two walls of 16 slots, versioned, each at the data seq `seq`
/// with a latest point of its own for it, and a text panel floating.
fn charts(seq: u64) -> Projection {
    let chart = |index: usize| {
        let mut chart = crate::chart::fixtures::pressure();
        *chart.series[0].values.last_mut().unwrap() =
            Some(0.5 + 0.01 * (seq as usize + index) as f32);
        chart
    };
    let mut panels: Vec<Panel> = (0..32)
        .map(|index| {
            let chart = chart(index);
            let place = on(&format!("wall-{}", 1 + index / 16), index as u32 % 16);
            Panel::new(format!("chart-{index}"), "Pressure", chart.summary(), place)
                .with_chart(chart)
                .with_version(5)
                .with_data_seq(seq)
        })
        .collect();
    let aside = Place::Free {
        pose: Pose::upright_facing([0.6, 1.5, -1.2], HOME.to_array()),
    };
    panels.push(Panel::new("aside", "Aside", "Floating.", aside).with_version(5));
    let wall = |id: &str, x: f32| {
        Wall::new(
            id,
            WallKind::Added,
            Pose::upright_facing([x, 1.5, -3.0], [x, 1.5, 0.0]),
        )
        .with_grid(tomori_protocol::Grid::from((8, 2)))
    };
    Projection::new("s", "sky")
        .with_walls(vec![wall("wall-1", -3.0), wall("wall-2", 3.0)])
        .with_panels(panels)
}

/// As many panels as an environment may publish, 32, each a list, on two walls of 16 slots, versioned `version`, their
/// rows saying so.
fn lists(version: u64) -> Projection {
    let mut shown = charts(5);
    shown.panels.truncate(32);
    for (index, panel) in shown.panels.iter_mut().enumerate() {
        panel.title = format!("Queue {index}");
        panel.body = format!("- [ ] first at {version}\n- [ ] second");
        panel.chart = None;
        panel.data_seq = 0;
        panel.version = version;
    }
    shown
}

/// Whether each panel's face is the one a projector that lays every panel out at once shows.
fn current(projector: &Projector, shown: &Projection) -> Vec<bool> {
    let mut fresh = Projector::default();
    fresh.update(input(shown, HOME, None));
    shown
        .panels
        .iter()
        .map(|panel| {
            projector.faces.contains_key(&panel.id)
                && *face(projector, &panel.id) == *face(&fresh, &panel.id)
        })
        .collect()
}

/// Paced, as the frame step's projector is, panels that arrive together, as on entering a space or as an environment
/// puts up its 32, are laid out a few a frame, [`ARRIVALS`], in the projection's order, each standing from the frame it
/// is laid out in, rather than all in one frame, which would cost that frame several times what it has.
#[test]
fn panels_arriving_together_are_laid_out_a_few_a_frame_in_the_projection_s_order() {
    let shown = lists(5);
    let mut projector = Projector::paced();
    let (laid, drawn) = counts();
    for frame in 1..=32 / ARRIVALS {
        let output = projector.update(input(&shown, HOME, None));
        // Each frame lays out and draws so many, no more: the next in order.
        assert_eq!(
            counts(),
            (laid + frame * ARRIVALS, drawn + frame * ARRIVALS)
        );
        let standing: Vec<bool> = shown
            .panels
            .iter()
            .map(|panel| projector.faces.contains_key(&panel.id))
            .collect();
        assert!(standing[..frame * ARRIVALS].iter().all(|stands| *stands));
        assert!(standing[frame * ARRIVALS..].iter().all(|stands| !stands));
        // Those laid out are drawn, with both walls; the rest are not yet.
        let faces = output.sheets.iter().filter(|sheet| sheet.canvas.is_some());
        assert_eq!(faces.count(), 2 + frame * ARRIVALS);
    }
    // Nothing more is laid out, and every one stands, as a projector that lays them out at once shows them.
    for _ in 0..3 {
        projector.update(input(&shown, HOME, None));
    }
    assert_eq!(counts(), (laid + 32, drawn + 32));
    assert!(current(&projector, &shown).iter().all(|current| *current));
}

/// Paced, as many panels as a space shows a client that takes them all, 900, stand within a bounded number of frames
/// of entering it, 113, about a second and a half, where two a frame took 450: panels that show nothing yet are laid
/// out [`ARRIVALS`] a frame, and no frame lays out more than that and the [`LAYOUTS`] that are laid out again of those
/// that already show something, however many of those the server changes meanwhile.
#[test]
fn every_panel_of_the_fullest_space_stands_within_a_bounded_number_of_frames() {
    let panels: Vec<Panel> = (0..900)
        .map(|index| {
            let (column, row) = ((index % 30) as f32, (index / 30) as f32);
            let at = [column * 0.7 - 10.0, 0.5 + row * 0.4, -6.0];
            let pose = Pose::upright_facing(at, [at[0], at[1], 0.0]);
            Panel::new(
                format!("panel-{index}"),
                "Note",
                format!("Note {index}."),
                Place::Free { pose },
            )
            .with_version(5)
        })
        .collect();
    let mut shown = Projection::new("s", "sky").with_panels(panels);
    let mut projector = Projector::paced();
    let (laid, _) = counts();
    let mut frames = 0;
    while projector.faces.len() < 900 {
        // Halfway, the server changes every panel that stands: they are laid out again two a frame, as ever, while the
        // rest go on arriving eight a frame.
        if frames == 50 {
            for panel in &mut shown.panels {
                panel.body.push_str(" Changed.");
                panel.version = 6;
            }
        }
        let before = (counts().0, projector.faces.len());
        let output = projector.update(input(&shown, HOME, None));
        frames += 1;
        let arrived = projector.faces.len() - before.1;
        let again = counts().0 - before.0 - arrived;
        assert_eq!(arrived, ARRIVALS.min(900 - before.1));
        assert_eq!(again, if frames > 50 { LAYOUTS } else { 0 });
        let standing = output.sheets.iter().filter(|sheet| sheet.canvas.is_some());
        assert_eq!(standing.count(), projector.faces.len());
        assert!(frames <= 900_usize.div_ceil(ARRIVALS));
    }
    assert_eq!(frames, 900_usize.div_ceil(ARRIVALS));
    // Every one stands, in the projection's order as it arrived; those changed are then laid out again in turn.
    let again = counts().0 - laid - 900;
    assert_eq!(again, (frames - 50) * LAYOUTS);
    assert!(face(&projector, "panel-899").plain().contains("Changed"));
}

/// Paced, panels the server changed together, as an environment's lists on one tick, are laid out again a few a frame,
/// each in turn, showing what they showed until then; what this client changed itself is laid out in its frame.
#[test]
fn panels_the_server_changed_together_are_laid_out_a_few_a_frame_each_in_turn() {
    let mut shown = lists(5);
    let mut projector = Projector::paced();
    for _ in 0..32 / LAYOUTS {
        projector.update(input(&shown, HOME, None));
    }
    let (laid, drawn) = counts();
    let first = every_face(&projector, &shown);
    // Every list changes in one patch, at a new version: the frame lays out only so many, the first in order, and the
    // rest show what they showed, the very faces.
    shown = lists(6);
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + LAYOUTS, drawn + LAYOUTS));
    let second = every_face(&projector, &shown);
    let again = kept(&second, &first);
    assert!(again[..LAYOUTS].iter().all(|kept| !kept));
    assert!(again[LAYOUTS..].iter().all(|kept| *kept));
    assert!(face(&projector, "chart-0").plain().contains("first at 6"));
    assert!(face(&projector, "chart-31").plain().contains("first at 5"));
    // They change again before the rest are laid out: the next frame goes on from where the last ended, rather than
    // with the first again, so that none waits behind the same ones every frame, and lays them out as they now are.
    shown = lists(7);
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 2 * LAYOUTS, drawn + 2 * LAYOUTS));
    let third = every_face(&projector, &shown);
    let again = kept(&third, &second);
    assert!(again[..LAYOUTS].iter().all(|kept| *kept));
    assert!(again[LAYOUTS..2 * LAYOUTS].iter().all(|kept| !kept));
    assert!(again[2 * LAYOUTS..].iter().all(|kept| *kept));
    assert!(face(&projector, "chart-0").plain().contains("first at 6"));
    assert!(face(&projector, "chart-2").plain().contains("first at 7"));
    // A tick this participant makes, shown at once at the version the panel has, is laid out in its frame, whatever
    // waits, and takes no other panel's turn; an age that shows otherwise waits its turn as the server's changes do.
    shown.panels[3].body = "- [x] first at 7\n- [ ] second".into();
    shown.panels[2].age = 3600;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 3 * LAYOUTS + 1, drawn + 3 * LAYOUTS + 1));
    assert!(face(&projector, "chart-3").plain().contains("☑ first at 7"));
    let now = current(&projector, &shown);
    assert_eq!(now[..3 * LAYOUTS], [false, false, false, true, true, true]);
    // Frame after frame, each in turn, the first ones last, none starved: within as many frames as lay every one out
    // once, each shows what laying it out afresh shows, and then nothing more is laid out.
    let mut frames = 0;
    let waiting = 32 - 2 * LAYOUTS + 1;
    while current(&projector, &shown).contains(&false) {
        let before = counts();
        projector.update(input(&shown, HOME, None));
        let laid = counts().0 - before.0;
        assert!((1..=LAYOUTS).contains(&laid) && counts().1 - before.1 == laid);
        frames += 1;
        assert!(frames <= waiting.div_ceil(LAYOUTS));
    }
    assert_eq!(frames, waiting.div_ceil(LAYOUTS));
    assert!(face(&projector, "chart-2").plain().contains("1h"));
    let settled = counts();
    for _ in 0..3 {
        projector.update(input(&shown, HOME, None));
    }
    assert_eq!(counts(), settled);
}

/// Paced, charts whose live data changed together are drawn again a few a frame, each in turn, without being laid
/// out, on the same turns as panels laid out.
#[test]
fn charts_whose_data_changed_together_are_drawn_again_a_few_a_frame_each_in_turn() {
    let mut shown = charts(5);
    let mut projector = Projector::paced();
    while current(&projector, &shown).contains(&false) {
        projector.update(input(&shown, HOME, None));
    }
    let (laid, drawn) = counts();
    let first = every_face(&projector, &shown);
    // Every chart's next point arrives in one patch: the frame draws only so many again, the first in order, and lays
    // nothing out; the rest show what they showed.
    shown = charts(6);
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid, drawn + LAYOUTS));
    let second = every_face(&projector, &shown);
    let again = kept(&second, &first);
    assert!(again[..LAYOUTS].iter().all(|kept| !kept));
    assert!(again[LAYOUTS..].iter().all(|kept| *kept));
    // A text panel's change at a new version takes a turn among them, laid out as its turn comes.
    shown.panels[32].body = "Floating, changed.".into();
    shown.panels[32].version = 8;
    for frame in 2..=32 / LAYOUTS {
        projector.update(input(&shown, HOME, None));
        assert_eq!(counts(), (laid, drawn + frame * LAYOUTS));
    }
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 1, drawn + 33));
    assert!(face(&projector, "aside").plain().contains("changed"));
    for _ in 0..3 {
        projector.update(input(&shown, HOME, None));
    }
    assert_eq!(counts(), (laid + 1, drawn + 33));
    assert!(current(&projector, &shown).iter().all(|current| *current));
}

/// While a panel's faces are behind the projection's panel, its turn not yet come, none of its items is pointed at,
/// since the lines it shows are no longer the panel's, so that no tick names another item than the one shown.
#[test]
fn a_panel_waiting_its_turn_points_at_no_item() {
    let panel = lists(5).panels.remove(0);
    let mut faces = Faces::new(&panel);
    let size = faces.at(Fit::SLOT).size;
    // The middle of the row of its first item, below its title, its age and the rule.
    let y = size.1 / 2.0
        - (super::super::PADDING.1 as f32 + 3.5 * CELL_HEIGHT as f32) * super::super::PIXEL;
    let item =
        |faces: &mut Faces| faces.item(Fit::SLOT, Mat4::IDENTITY, 0, Vec3::new(-0.05, y, 0.0));
    assert_eq!(item(&mut faces), Some(0));
    // The server changed it, and its turn has not come: it shows what it showed, and points at nothing.
    let changed = lists(6).panels.remove(0);
    assert!(!faces.follow(&changed, false, 0.0));
    assert_eq!(item(&mut faces), None);
    assert!(
        faces
            .face(Fit::SLOT, (0, &Default::default()), FRAME)
            .plain()
            .contains("first at 5")
    );
    // Its turn come, it shows the panel as it is, and points at its items again.
    assert!(faces.follow(&changed, true, 0.0));
    assert_eq!(item(&mut faces), Some(0));
    assert!(
        faces
            .face(Fit::SLOT, (0, &Default::default()), FRAME)
            .plain()
            .contains("first at 6")
    );
    assert!(!faces.follow(&changed, false, 0.0));
    assert_eq!(item(&mut faces), Some(0));
}

/// A chart panel sent as its summary, as a server sends it where a client's frame cannot hold its chart, at the version
/// and data seq it had, is laid out again as the text panel it then is, and as a chart once it is sent whole again.
#[test]
fn a_chart_panel_sent_as_its_summary_is_laid_out_again_as_text_and_as_a_chart_again() {
    let mut shown = charts(5);
    let mut projector = Projector::default();
    projector.update(input(&shown, HOME, None));
    let (laid, drawn) = counts();
    let chart = shown.panels[0].chart.take();
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 1, drawn + 1));
    assert!(face(&projector, "chart-0").shapes.is_empty());
    assert!(face(&projector, "chart-0").plain().contains("mean: "));
    // Whole again, with a point newer, it is a chart again, laid out as one rather than only drawn from its data.
    shown.panels[0].chart = chart;
    shown.panels[0].data_seq = 6;
    projector.update(input(&shown, HOME, None));
    assert_eq!(counts(), (laid + 2, drawn + 2));
    assert!(!face(&projector, "chart-0").shapes.is_empty());
}
