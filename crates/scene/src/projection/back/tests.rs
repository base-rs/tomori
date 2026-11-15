// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::back::{Back, BackCode, BackLine, BackSection, DEFINITION, INPUTS, STATUS};
use tomori_protocol::{Place, Pose};

fn floating(id: &str) -> Panel {
    Panel::new(
        id,
        "Latency",
        "",
        Place::Free {
            pose: Pose::default(),
        },
    )
}

/// What a back describes: `sections`, as given.
fn described(sections: Vec<BackSection>) -> Described {
    let mut described = describe(
        &floating("x"),
        &About {
            updated: None,
            linked: &[],
            value: &|_| String::new(),
        },
    );
    described.sections = sections;
    described
}

/// The section `name` of `lines`, `code` its code block.
fn section(name: &str, lines: Vec<BackLine>, code: Option<BackCode>) -> BackSection {
    let mut section = BackSection::new(name);
    section.lines = lines;
    section.code = code;
    section
}

#[test]
fn when_a_panel_last_changed_is_followed_from_what_the_server_says_and_each_change_after() {
    let now = Instant::now() + Duration::from_secs(3_600);
    let mut updates = Updates::default();
    let mut panel = floating("a").with_updated(Some(30));
    panel.version = 1;
    let unknown = floating("b");
    let mut shown = Projection::new("s", "sky").with_panels(vec![panel.clone(), unknown]);
    updates.follow(&shown, now);
    assert_eq!(updates.ago("a", now + Duration::from_secs(5)), Some(35));
    assert_eq!(updates.ago("b", now), None);
    assert_eq!(updates.ago("c", now), None);
    // The same again changes nothing; a new version or data seq is a change now.
    updates.follow(&shown, now + Duration::from_secs(5));
    assert_eq!(updates.ago("a", now + Duration::from_secs(5)), Some(35));
    shown.panels[0].data_seq = 4;
    updates.follow(&shown, now + Duration::from_secs(10));
    assert_eq!(updates.ago("a", now + Duration::from_secs(12)), Some(2));
    shown.panels[1].version = 9;
    updates.follow(&shown, now + Duration::from_secs(10));
    assert_eq!(updates.ago("b", now + Duration::from_secs(10)), Some(0));
    // A panel no longer shown is forgotten.
    shown.panels.remove(1);
    updates.follow(&shown, now);
    assert_eq!(updates.ago("b", now), None);
}

#[test]
fn a_back_is_its_title_heading_and_sections_as_large_as_its_face_opaque_and_framed() {
    let size = (0.62, 0.4);
    let described = described(vec![
        section(
            INPUTS,
            vec![
                BackLine::new("Region", "us-east1 · its own"),
                BackLine::new("A key far too long", "cut to the keys' room"),
            ],
            None,
        ),
        section(
            DEFINITION,
            vec![BackLine::new("Source", "Observe · Prometheus")],
            Some(BackCode::new(
                "promql",
                "sum by (pod) (\n  rate(requests_total[5m])\n)",
            )),
        ),
        section(
            STATUS,
            vec![
                BackLine::new(
                    "Error",
                    "the query timed out after 30 s, and the panel shows what it had",
                )
                .warning(),
            ],
            None,
        ),
        section(
            "shared-channels",
            vec![BackLine::new("Cursor", "region")],
            None,
        ),
    ]);
    let canvas = face("Latency", &described, size, [1.0; 4]);
    assert_eq!(canvas.kind, Kind::PanelFace);
    assert_eq!(canvas.background[3], 1.0);
    assert_eq!(canvas.border, Some([1.0; 4]));
    let width = (canvas.columns * CELL_WIDTH + 2 * canvas.padding.0) as f32 * canvas.pixel;
    let height = (canvas.rows.len() * CELL_HEIGHT + 2 * canvas.padding.1) as f32 * canvas.pixel;
    assert!((width - size.0).abs() < 1e-4, "{width}");
    assert!((height - size.1).abs() < 2.0 * canvas.pixel, "{height}");
    let rows: Vec<String> = canvas
        .rows
        .iter()
        .map(|row| row.plain().trim_end().to_owned())
        .collect();
    assert_eq!(rows[0], "Latency");
    assert_eq!(rows[1], HEADING);
    // Each section headed, in its colour, its keys in one column.
    assert_eq!(rows[3], "Inputs");
    assert!(
        canvas.rows[3]
            .cells
            .iter()
            .all(|(_, style)| style.color == SECTION)
    );
    assert!(rows[4].starts_with("Region      us-east1"), "{}", rows[4]);
    assert!(rows[5].starts_with("A key far…  cut"), "{}", rows[5]);
    assert_eq!(rows[6], "Definition");
    assert!(
        rows[7].starts_with("Source      Observe · Prometheus"),
        "{}",
        rows[7]
    );
    // The code block's language, then its lines monospaced on their tint, indents kept.
    assert_eq!(rows[8], "promql");
    assert_eq!(canvas.rows[8].background, None);
    assert_eq!(rows[10], "  rate(requests_total[5m])");
    assert_eq!(canvas.rows[9].background, Some(CODE_BACKGROUND));
    // A long value wraps under itself, in its colour where it warns.
    assert_eq!(rows[12], "Status");
    assert!(rows[13].starts_with("Error"));
    assert!(rows[14].starts_with(&" ".repeat(12)), "{}", rows[14]);
    assert!(
        canvas.rows[13]
            .cells
            .iter()
            .any(|(_, style)| style.color == WARNING)
    );
    // A section of another name is headed by it.
    assert!(rows.contains(&"Shared channels".to_owned()), "{rows:?}");
}

#[test]
fn a_back_longer_than_its_face_ends_with_an_ellipsis_and_code_without_a_language_shows_its_text() {
    let lines = (0..40)
        .map(|index| BackLine::new(format!("Line {index}"), "value"))
        .collect();
    let canvas = face(
        "Long",
        &described(vec![section(STATUS, lines, None)]),
        (0.62, 0.4),
        [1.0; 4],
    );
    let last = canvas.rows.last().unwrap().plain();
    assert!(last.trim_end().ends_with('…'), "{last}");
    let canvas = face(
        "Query",
        &described(vec![section(
            DEFINITION,
            Vec::new(),
            Some(BackCode::new("", "up")),
        )]),
        (0.62, 0.4),
        [1.0; 4],
    );
    let rows: Vec<String> = canvas.rows.iter().map(Row::plain).collect();
    assert_eq!(rows[4].trim_end(), "up");
}

#[test]
fn a_back_is_drawn_once_while_it_says_the_same_and_again_as_it_changes() {
    let panel = floating("a").with_back(Some(
        Back::new().with_line(DEFINITION, BackLine::new("Source", "Sky")),
    ));
    let shown = Projection::new("s", "sky").with_panels(vec![panel.clone()]);
    let mut backs = Backs::default();
    let first = backs.back(&shown, &panel, Some(3), (0.6, 0.4), [1.0; 4]);
    assert!(Arc::ptr_eq(
        &first,
        &backs.back(&shown, &panel, Some(3), (0.6, 0.4), [1.0; 4])
    ));
    for (updated, size, border) in [
        (Some(4), (0.6, 0.4), [1.0; 4]),
        (Some(4), (0.7, 0.4), [1.0; 4]),
        (Some(4), (0.7, 0.4), [0.5; 4]),
    ] {
        let again = backs.back(&shown, &panel, updated, size, border);
        assert!(!Arc::ptr_eq(&first, &again));
    }
    let mut renamed = panel.clone();
    renamed.title = "Errors".into();
    let again = backs.back(&shown, &renamed, Some(4), (0.7, 0.4), [0.5; 4]);
    assert!(again.plain().starts_with("Errors"));
    // Those no longer seen from behind are forgotten.
    backs.retain(&["a"]);
    assert_eq!(backs.drawn.len(), 1);
    backs.retain(&[]);
    assert!(backs.drawn.is_empty());
}

#[test]
fn a_panel_is_seen_from_behind_from_beyond_its_plane_and_its_back_is_turned_about_its_vertical() {
    let model = Mat4::from_translation(Vec3::new(0.0, 1.5, -2.0));
    assert!(!seen_from_behind(model, Vec3::new(0.0, 1.6, 0.0)));
    assert!(seen_from_behind(model, Vec3::new(0.3, 1.6, -4.0)));
    let back = turned(model);
    assert_eq!(back.w_axis, model.w_axis);
    assert!((back.z_axis.truncate() + Vec3::Z).length() < 1e-6);
    assert!((back.x_axis.truncate() + Vec3::X).length() < 1e-6);
    assert!((back.y_axis.truncate() - Vec3::Y).length() < 1e-6);
}

#[test]
fn a_back_far_away_is_its_title_alone_and_whole_again_only_a_fifth_nearer() {
    let mut backs = Backs::default();
    assert!(!backs.far("a", FAR * 1.1));
    assert!(backs.far("a", FAR * 0.9));
    assert!(backs.far("b", FAR * 0.5));
    // Between the two, it stays as it was.
    assert!(backs.far("a", FAR * 1.1));
    assert!(!backs.far("c", FAR * 1.1));
    assert!(!backs.far("a", NEAR * 1.01));
    assert!(!backs.far("a", FAR * 1.1));
    assert_eq!(backs.far, ["b"]);
    backs.retain(&[]);
    assert!(backs.far.is_empty());
}

#[test]
fn a_back_writes_its_context_as_its_chips_do() {
    use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};
    let panel = floating("a").with_context(vec![ContextChip::new(
        "time",
        "Time",
        ContextValue::Live { minutes: 120 },
        ContextSetting::Fixed,
    )]);
    let shown = Projection::new("s", "sky").with_panels(vec![panel.clone()]);
    let back = Backs::default().back(&shown, &panel, None, (0.6, 0.4), [1.0; 4]);
    assert!(
        back.plain().contains("last 2 h · its own"),
        "{}",
        back.plain()
    );
}

#[test]
fn a_section_is_headed_by_its_name_its_first_letter_capital_and_its_dashes_spaces() {
    assert_eq!(heading("inputs"), "Inputs");
    assert_eq!(heading("shared-channels"), "Shared channels");
    assert_eq!(heading(""), "");
}
