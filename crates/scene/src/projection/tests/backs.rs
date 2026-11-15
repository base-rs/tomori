// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::back::{Back, BackCode, BackLine, DEFINITION};

/// The sheet of the panel titled `title`, which its face or its back begins with.
fn sheet_of<'a>(output: &'a Output, title: &str) -> &'a Sheet {
    output
        .sheets
        .iter()
        .find(|sheet| {
            sheet
                .canvas
                .as_deref()
                .is_some_and(|canvas| canvas.plain().starts_with(title))
        })
        .unwrap()
}

fn plain(sheet: &Sheet) -> String {
    sheet.canvas.as_deref().unwrap().plain()
}

/// What the viewer at `viewer` sees of `shown`, at `now`, pointing `ray`.
fn seen<'a>(shown: &'a Projection, viewer: Vec3, ray: Option<Ray>, now: Instant) -> Input<'a> {
    Input {
        viewer: Some(viewer),
        ..input(shown, ray, 0.0, now)
    }
}

const BACK: &str = "Back · configuration";

#[test]
fn a_floating_panel_seen_from_behind_shows_its_back_facing_out_of_it_drawn_once_while_it_says_the_same()
 {
    let ahead = HOME - Vec3::Z * 1.5;
    let behind = ahead - Vec3::Z * 1.5 + Vec3::X * 0.3;
    let backed = floating_at("a", ahead)
        .with_back(Some(
            Back::new()
                .with_line(DEFINITION, BackLine::new("Source", "Sky · Prometheus"))
                .with_code(DEFINITION, BackCode::new("promql", "up")),
        ))
        .with_updated(Some(12));
    // A face of a dozen lines, as large as a back needs.
    let mut backed = backed;
    backed.body = (0..12).map(|line| format!("- line {line}\n")).collect();
    let mut shown = projection(vec![backed]);
    let now = Instant::now();
    let mut projector = Projector::default();
    // From in front, its face.
    let front = projector.update(seen(&shown, HOME, None, now));
    let face = sheet_of(&front, "Panel a");
    assert!(!plain(face).contains(BACK));
    let model = face.model.unwrap();
    // From behind, its back, as large, in its place, turned to face the viewer, saying how it is configured.
    let output = projector.update(seen(&shown, behind, None, now));
    let back = sheet_of(&output, "Panel a").clone();
    let text = plain(&back);
    for said in [
        BACK,
        "Definition",
        "Status",
        "Prometheus",
        "12 s ago",
        "promql",
        "up",
    ] {
        assert!(text.contains(said), "{said} in {text}");
    }
    let turned = back.model.unwrap();
    assert_eq!(turned.w_axis, model.w_axis);
    assert!((turned.z_axis + model.z_axis).length() < 1e-5);
    assert!((turned.x_axis + model.x_axis).length() < 1e-5);
    assert!((behind - turned.w_axis.truncate()).dot(turned.z_axis.truncate()) > 0.0);
    let size = |canvas: &Canvas| {
        (canvas.columns * crate::ui_text::CELL_WIDTH + 2 * canvas.padding.0) as f32 * canvas.pixel
    };
    assert!(
        (size(back.canvas.as_deref().unwrap()) - size(face.canvas.as_deref().unwrap())).abs()
            < 1e-4
    );
    // Still, it is the back drawn before; a second on, its age is counted on and it is drawn again.
    let again = projector.update(seen(&shown, behind, None, now));
    assert!(Arc::ptr_eq(
        back.canvas.as_ref().unwrap(),
        sheet_of(&again, "Panel a").canvas.as_ref().unwrap()
    ));
    let later = projector.update(seen(&shown, behind, None, now + Duration::from_secs(1)));
    assert!(plain(sheet_of(&later, "Panel a")).contains("13 s ago"));
    // Changed by the server, it was updated just now.
    shown.panels[0].version = 7;
    let changed = projector.update(seen(&shown, behind, None, now + Duration::from_secs(2)));
    assert!(plain(sheet_of(&changed, "Panel a")).contains("0 s ago"));
    // Back in front, its face again.
    let front = projector.update(seen(&shown, HOME, None, now));
    assert!(!plain(sheet_of(&front, "Panel a")).contains(BACK));
}

#[test]
fn a_panel_in_a_wall_s_slot_shows_its_face_alone_its_wall_behind_it_until_it_is_carried_and_turned()
{
    let shown = projection(vec![panel("a", on(TOMORI_WALL, 0))]);
    let now = Instant::now();
    let mut projector = Projector::default();
    let behind = HOME - Vec3::Z * 8.0;
    let output = projector.update(seen(&shown, behind, None, now));
    assert!(!plain(sheet_of(&output, "Panel a")).contains(BACK));
    // Pulled off and turned half round with the stick, its back faces its carrier.
    let ray = at(slot_centre(&shown, 0));
    projector.update(input(&shown, ray, 0.0, now));
    projector.update(input(&shown, ray, 1.0, now));
    assert_eq!(projector.held(), Some("a"));
    // Pulled close, away from its wall, so that it docks in no slot.
    projector.update(Input {
        stick: Vec2::NEG_Y,
        dt: 10.0,
        ..input(&shown, ray, 1.0, now)
    });
    projector.update(input(&shown, ray, 1.0, now));
    let mut turned = false;
    for frame in 0..400 {
        let output = projector.update(Input {
            stick: Vec2::X,
            ..input(&shown, ray, 1.0, now + Duration::from_millis(frame * 14))
        });
        if plain(sheet_of(&output, "Panel a")).contains(BACK) {
            turned = true;
            break;
        }
    }
    assert!(turned, "turned half round, its back shows");
}

#[test]
fn a_panel_seen_from_behind_far_away_shows_its_title_alone_and_points_at_none_of_its_items() {
    let far = HOME - Vec3::Z * 40.0;
    let shown = projection(vec![floating_at("a", far)]);
    let now = Instant::now();
    let mut projector = Projector::paced();
    for _ in 0..4 {
        projector.update(seen(&shown, HOME, None, now));
    }
    let behind = far - Vec3::Z * 40.0;
    let output = projector.update(seen(&shown, behind, None, now));
    let sheet = sheet_of(&output, "Panel a");
    assert_eq!(plain(sheet).trim_end(), "Panel a");
    assert!((sheet.model.unwrap().z_axis.truncate() + Vec3::Z).length() < 0.2);
}

#[test]
fn pointing_at_a_panel_from_behind_names_it_but_none_of_its_items_nor_reads_its_chart() {
    let ahead = HOME - Vec3::Z * 1.2;
    let chart = tomori_protocol::chart::Chart::new(3)
        .with_time(tomori_protocol::chart::TimeAxis::Live { step_ms: 5_000 })
        .with_series(tomori_protocol::chart::Series::new(
            "p99",
            tomori_protocol::chart::Colour::Teal,
            tomori_protocol::chart::SeriesStyle::Line,
            vec![Some(1.0), Some(2.0), Some(3.0)],
        ));
    let list = floating_at("a", ahead);
    let plotted = floating_at("b", ahead + Vec3::X * 1.2).with_chart(chart);
    let shown = projection(vec![list, plotted]);
    let now = Instant::now();
    let mut projector = Projector::default();
    projector.update(seen(&shown, HOME, None, now));
    // Somewhere on its face, from in front, names an item, and a place on the chart reads it.
    let points: Vec<Vec3> = (0..60)
        .flat_map(|x| (0..40).map(move |y| (x, y)))
        .map(|(x, y)| ahead + Vec3::new(-0.35 + x as f32 * 0.012, -0.25 + y as f32 * 0.012, 0.0))
        .collect();
    let item = points
        .iter()
        .copied()
        .find(|point| {
            projector
                .update(seen(&shown, HOME, at(*point), now))
                .item
                .is_some()
        })
        .expect("an item somewhere on its face");
    let chart_point = ahead + Vec3::X * 1.2;
    assert!(
        projector
            .update(seen(&shown, HOME, at(chart_point), now))
            .readout
            .is_some()
    );
    // From behind, through the same points, it is pointed at, but neither the item nor the chart is.
    let behind = ahead - Vec3::Z * 1.5;
    let from_behind = |point: Vec3| Ray::new(behind, point - behind);
    let output = projector.update(seen(&shown, behind, from_behind(item), now));
    assert!(output.pointed.is_some());
    assert_eq!((output.item, output.chip), (None, None));
    let output = projector.update(seen(&shown, behind, from_behind(chart_point), now));
    assert_eq!(
        output.pointed.as_ref().map(|(id, ..)| id.as_str()),
        Some("b")
    );
    assert!(output.readout.is_none());
}
