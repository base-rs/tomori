// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::chart::{Chart, Colour, Series, SeriesStyle};
use crate::{Code, Image, Place, Pose};

fn chart(time: TimeAxis, series: usize) -> Chart {
    (0..series).fold(Chart::new(3).with_time(time), |chart, index| {
        chart.with_series(Series::new(
            format!("s{index}"),
            Colour::Teal,
            SeriesStyle::Line,
            vec![Some(1.0); 3],
        ))
    })
}

fn on(wall: &str) -> Place {
    Place::Wall {
        wall: wall.into(),
        slot: 0,
    }
}

fn floating() -> Place {
    Place::Free {
        pose: Pose::default(),
    }
}

fn written(chip: &ContextChip) -> String {
    match &chip.value {
        ContextValue::Text { text } => text.clone(),
        ContextValue::Live { minutes } => format!("last {minutes} min"),
        ContextValue::Window { from_ms, to_ms } => format!("{from_ms}–{to_ms}"),
        _ => "?".into(),
    }
}

/// Each section's name and its lines, `key: value`.
fn said(described: &Described) -> Vec<(String, Vec<String>)> {
    described
        .sections
        .iter()
        .map(|section| {
            (
                section.name.clone(),
                section
                    .lines
                    .iter()
                    .map(|line| format!("{}: {}", line.key, line.value))
                    .collect(),
            )
        })
        .collect()
}

fn about() -> About<'static> {
    About {
        updated: None,
        linked: &[],
        value: &written,
    }
}

#[test]
fn a_back_is_cleaned_and_bounded_each_part_cut_or_left_out() {
    let long = "x".repeat(400);
    let mut back = Back::new()
        .with_line(DEFINITION, BackLine::new(&long, &long).warning())
        .with_line(DEFINITION, BackLine::new("\u{202e}Empty", " "))
        .with_code(DEFINITION, BackCode::new("p".repeat(25), "x".repeat(2_000)))
        .with_line(
            DEFINITION,
            BackLine::new("Again", "a second definition is the first's"),
        );
    back.sections.push(BackSection::new(DEFINITION));
    back.sections[1]
        .lines
        .push(BackLine::new("Ignored", "a second section of a name"));
    let back = back.cleaned();
    assert_eq!(back.sections.len(), 1);
    let section = &back.sections[0];
    assert_eq!(section.lines[0].key.chars().count(), BACK_KEY_CHARS + 1);
    assert_eq!(section.lines[0].value.chars().count(), BACK_VALUE_CHARS + 1);
    assert!(section.lines[0].warning);
    assert_eq!(section.lines.len(), 2, "the empty value is left out");
    let code = section.code.as_ref().unwrap();
    assert_eq!(code.language, "");
    assert_eq!(code.text.chars().count(), BACK_CODE_CHARS + 1);
    // A code block of many lines keeps its first, their indents, and says it was cut; its controls and format
    // characters are left out and tabs are spaces.
    let lines: String = (0..40).map(|line| format!("\n  line {line}\r\n")).collect();
    let code = Back::new()
        .with_code(STATUS, BackCode::new("promql", lines))
        .cleaned()
        .sections[0]
        .code
        .clone()
        .unwrap();
    assert_eq!(code.language, "promql");
    assert_eq!(code.text.lines().count(), BACK_CODE_LINES);
    assert!(code.text.starts_with("  line 0\n"));
    assert!(code.text.ends_with('…'));
    let code = Back::new()
        .with_code(STATUS, BackCode::new("", "a\tb\u{202e}c\u{7}"))
        .cleaned()
        .sections[0]
        .code
        .clone()
        .unwrap();
    assert_eq!(code.text, "a    bc");
    // At most so many sections, each of a sound name; a section of nothing is none.
    let mut many = Back::new();
    for index in 0..BACK_SECTIONS + 2 {
        many = many.with_line(&format!("extra-{index}"), BackLine::new("Key", "value"));
    }
    let many = many
        .with_code("empty", BackCode::new("promql", "\n\u{7}\n"))
        .cleaned();
    assert_eq!(many.sections.len(), BACK_SECTIONS);
    assert!(many.sections.iter().all(|section| section.name != "empty"));
}

#[test]
fn a_back_is_part_of_a_panel_s_version_and_its_update_age_is_not() {
    let panel = crate::Panel::new("p", "Latency", "", floating());
    let backed = panel.clone().with_back(Some(
        Back::new().with_line(DEFINITION, BackLine::new("Source", "Observe · Prometheus")),
    ));
    assert!(!panel.same_structure(&backed));
    assert!(panel.same_structure(&panel.clone().with_updated(Some(12))));
    // On the wire only where it has one, and an older server's panel has neither.
    let json = serde_json::to_value(&panel).unwrap();
    assert!(json.get("back").is_none() && json.get("updated").is_none());
    let json = serde_json::to_value(backed.clone().with_updated(Some(3))).unwrap();
    assert_eq!(json["back"]["sections"][0]["name"], "definition");
    assert_eq!(json["updated"], 3);
    let read: crate::Panel = serde_json::from_value(json).unwrap();
    assert_eq!(read.back, backed.back);
    assert_eq!(read.updated, Some(3));
    assert_eq!(panel.with_back(None).back, None);
}

#[test]
fn a_live_chart_s_back_is_its_inputs_outputs_definition_and_status_its_environment_s_after_tomori_s()
 {
    let chips = vec![
        ContextChip::new(
            "region",
            "Region",
            ContextValue::Text {
                text: "us-east1".into(),
            },
            ContextSetting::Inherited,
        ),
        ContextChip::new(
            crate::context::TIME,
            "Time",
            ContextValue::Live { minutes: 15 },
            ContextSetting::Follows,
        ),
        ContextChip::new(
            "tier",
            "Tier",
            ContextValue::Text { text: "web".into() },
            ContextSetting::Fixed,
        ),
    ];
    let panel = crate::Panel::new("p", "Latency", "", on("w"))
        .with_chart(chart(TimeAxis::Live { step_ms: 5_000 }, 3))
        .with_context(chips)
        .with_back(Some(
            Back::new()
                .with_line("workbook", BackLine::new("Cell", "latency"))
                .with_line(DEFINITION, BackLine::new("Source", "Observe · Prometheus"))
                .with_code(DEFINITION, BackCode::new("promql", "rate(x[5m])"))
                .with_line(STATUS, BackLine::new("Refresh", "every 15 s"))
                .with_line(STATUS, BackLine::new("Error", "timeout").warning())
                .with_line(OUTPUTS, BackLine::new("Copy of", "Latency")),
        ));
    let linked: Vec<String> = (0..10).map(|index| format!("Chart {index}")).collect();
    let described = describe(
        &panel,
        &About {
            updated: Some(12),
            linked: &linked,
            value: &written,
        },
    );
    let section = |name: &str, lines: &[&str]| {
        (
            name.to_owned(),
            lines
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        said(&described),
        [
            section(
                INPUTS,
                &[
                    "Region: us-east1 · inherited from its wall",
                    "Time: last 15 min · following the space",
                    "Tier: web · its own",
                ]
            ),
            section(
                OUTPUTS,
                &[
                    "Linked: time cursor with Chart 0, Chart 1, Chart 2, Chart 3, Chart 4, Chart 5, Chart 6, Chart 7, +2 more",
                    "Copy of: Latency",
                ]
            ),
            section(
                DEFINITION,
                &[
                    "Shows: Chart of 3 series",
                    "Points: every 5 s",
                    "Source: Observe · Prometheus",
                ]
            ),
            section(
                STATUS,
                &["Updated: 12 s ago", "Refresh: every 15 s", "Error: timeout"]
            ),
            section("workbook", &["Cell: latency"]),
        ]
    );
    let definition = &described.sections[2];
    assert_eq!(
        definition.code,
        Some(BackCode::new("promql", "rate(x[5m])"))
    );
    assert!(described.sections[3].lines[2].warning);
}

#[test]
fn a_back_says_what_kind_of_panel_it_is_and_whose_and_a_live_chart_refreshes_live() {
    let about = about();
    let definition = |panel: crate::Panel| {
        describe(&panel, &about)
            .sections
            .into_iter()
            .find(|section| section.name == DEFINITION)
            .unwrap()
            .lines[0]
            .value
            .clone()
    };
    let text = crate::Panel::new("t", "Notes", "Hello", floating());
    assert_eq!(
        said(&describe(&text, &about)),
        [(
            DEFINITION.to_owned(),
            vec![
                "Shows: Text".to_owned(),
                "Source: Tomori's agent".to_owned()
            ]
        )]
    );
    assert_eq!(
        definition(
            text.clone()
                .with_chart(chart(TimeAxis::Live { step_ms: 250 }, 1))
        ),
        "Chart of 1 series"
    );
    let rows = vec![crate::ListRow::new("a", false)];
    assert_eq!(
        definition(text.clone().with_rows(rows.clone())),
        "List of 1 row"
    );
    assert_eq!(
        definition(
            text.clone()
                .with_rows(rows)
                .with_columns(vec![crate::ListColumn::new("A")])
        ),
        "Table of 1 row"
    );
    let mut face = text.clone();
    face.face = Some(Box::new(crate::Face::new("clock", serde_json::Value::Null)));
    assert_eq!(definition(face), "Face");
    assert_eq!(
        definition(text.clone().with_code(Code::new("rust", 40))),
        "Code, rust, 40 lines"
    );
    assert_eq!(
        definition(text.clone().with_code(Code::new("", 1))),
        "Code, 1 line"
    );
    assert_eq!(
        definition(text.clone().with_image(Image::new("h", 640, 480, 9))),
        "Image, 640 × 480"
    );
    let (diagram, _) = crate::diagram::parse("a[A] --> b[B]\n");
    assert_eq!(
        definition(text.clone().with_diagram(diagram)),
        "Diagram of 2 nodes"
    );
    // An environment's panel says no source of Tomori's; a live chart refreshes live, its spacing in its unit.
    let live = text
        .clone()
        .with_back(Some(Back::default()))
        .with_chart(chart(TimeAxis::Live { step_ms: 250 }, 2));
    let lines = said(&describe(&live, &about));
    assert_eq!(
        lines,
        [
            (
                DEFINITION.to_owned(),
                vec![
                    "Shows: Chart of 2 series".to_owned(),
                    "Points: every 250 ms".to_owned()
                ]
            ),
            (STATUS.to_owned(), vec!["Refresh: live".to_owned()]),
        ]
    );
    for (step, said) in [(60_000, "1 min"), (3_600_000, "1 h"), (1_500, "1.5 s")] {
        let chart = text
            .clone()
            .with_chart(chart(TimeAxis::Live { step_ms: step }, 1));
        let described = describe(&chart, &about);
        assert_eq!(
            described.sections[0].lines[2].value,
            format!("every {said}")
        );
    }
}

#[test]
fn a_fixed_window_says_its_window_as_an_input_unless_a_chip_says_it_and_never_refreshes_live() {
    let about = About {
        updated: Some(7_200),
        ..about()
    };
    let fixed = crate::Panel::new("f", "Then", "", floating()).with_chart(chart(
        TimeAxis::Fixed {
            from_ms: 0,
            to_ms: 60_000,
        },
        1,
    ));
    let described = said(&describe(&fixed, &about));
    assert_eq!(
        described[0],
        (INPUTS.to_owned(), vec!["Window: 0–60000".to_owned()])
    );
    assert_eq!(
        described[2],
        (STATUS.to_owned(), vec!["Updated: 2 h ago".to_owned()])
    );
    let chipped = fixed.with_context(vec![ContextChip::new(
        crate::context::TIME,
        "Time",
        ContextValue::Window {
            from_ms: 0,
            to_ms: 60_000,
        },
        ContextSetting::Fixed,
    )]);
    assert_eq!(
        said(&describe(&chipped, &about))[0],
        (
            INPUTS.to_owned(),
            vec!["Time: 0–60000 · its own".to_owned()]
        )
    );
}

#[test]
fn a_chart_is_linked_to_the_live_charts_on_its_wall_or_the_windows_overlapping_its_own() {
    let live = |id: &str, title: &str, place: Place| {
        crate::Panel::new(id, title, "", place)
            .with_chart(chart(TimeAxis::Live { step_ms: 5_000 }, 1))
    };
    let window = |id: &str, from_ms: i64, to_ms: i64| {
        crate::Panel::new(id, id.to_uppercase(), "", floating())
            .with_chart(chart(TimeAxis::Fixed { from_ms, to_ms }, 1))
    };
    let panels = vec![
        live("a", "A", on("w")),
        live("b", "", on("w")),
        live("c", "C", on("other")),
        live("d", "D", floating()),
        crate::Panel::new("e", "Notes", "", on("w")),
        window("f", 0, 100),
        window("g", 50, 150),
        window("h", 200, 300),
        crate::Panel::new("i", "Untimed", "", on("w")).with_chart(Chart::new(3)),
    ];
    assert_eq!(linked(&panels, &panels[0]), ["b"]);
    assert_eq!(linked(&panels, &panels[1]), ["A"]);
    assert!(linked(&panels, &panels[3]).is_empty());
    assert_eq!(linked(&panels, &panels[5]), ["G"]);
    assert!(linked(&panels, &panels[7]).is_empty());
    assert!(linked(&panels, &panels[4]).is_empty());
    assert!(linked(&panels, &panels[8]).is_empty());
}

#[test]
fn how_long_ago_is_said_in_its_largest_whole_unit_and_a_few_links_all_by_name() {
    assert_eq!(
        [0, 59, 60, 3_599, 3_600, 86_399, 86_400].map(ago),
        [
            "0 s ago",
            "59 s ago",
            "1 min ago",
            "59 min ago",
            "1 h ago",
            "23 h ago",
            "1 d ago"
        ]
    );
    let linked = ["Errors".to_owned(), "x".repeat(40)];
    let described = describe(
        &crate::Panel::new("p", "Latency", "", floating()),
        &About {
            linked: &linked,
            ..about()
        },
    );
    assert_eq!(
        described.sections[0].lines[0].value,
        format!("time cursor with Errors, {}…", "x".repeat(BACK_KEY_CHARS))
    );
}

#[test]
fn a_back_s_sections_are_found_and_made_by_name() {
    let mut back = Back::new();
    assert!(back.section(INPUTS).is_none());
    back.section_mut(INPUTS)
        .lines
        .push(BackLine::new("Region", "eu"));
    back.section_mut(INPUTS)
        .lines
        .push(BackLine::new("Pod", "7"));
    assert_eq!(back.sections.len(), 1);
    assert_eq!(back.line(INPUTS, "Pod"), Some("7"));
    assert_eq!(back.line(OUTPUTS, "Pod"), None);
}

#[test]
fn sections_are_named_and_code_tagged_by_a_few_safe_characters_and_a_section_of_nothing_is_none() {
    assert!(valid_name("shared-channels") && valid_name("v2"));
    assert!(!valid_name("") && !valid_name("Status") && !valid_name("a b"));
    assert!(valid_language("promql") && valid_language("c++") && valid_language("objective_c-2"));
    assert!(!valid_language("") && !valid_language("prom ql") && !valid_language(&"a".repeat(25)));
    let empty = Back::new()
        .with_code(STATUS, BackCode::new("promql", "\n\u{7}\n"))
        .with_line(INPUTS, BackLine::new("", "no key"))
        .cleaned();
    assert_eq!(empty, Back::default());
}
