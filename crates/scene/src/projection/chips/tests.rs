// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

fn chip(parameter: &str, label: &str, value: ContextValue, setting: ContextSetting) -> ContextChip {
    ContextChip::new(parameter, label, value, setting)
}

fn text(value: &str) -> ContextValue {
    ContextValue::Text {
        text: value.to_owned(),
    }
}

/// 12 November 2026, 00:00 UTC, in milliseconds, and as a day.
const DAY: i64 = 1_794_441_600_000;
const TODAY: i64 = DAY / 86_400_000;

fn plain(laid: &Laid) -> String {
    laid.cells.iter().map(|cell| cell.0).collect()
}

fn window(from: i64, to: i64) -> ContextValue {
    ContextValue::Window {
        from_ms: DAY + from * 60_000,
        to_ms: DAY + to * 60_000,
    }
}

#[test]
fn dates_and_times_are_written_in_utc_with_the_date_where_it_is_not_today() {
    assert_eq!(civil(0), (1970, 1, 1));
    assert_eq!(civil(TODAY), (2026, 11, 12));
    assert_eq!(civil(19_782), (2024, 2, 29));
    assert_eq!(civil(-1), (1969, 12, 31));
    assert_eq!(civil(-719_528), (0, 1, 1));
    assert_eq!(clock(DAY + 140 * 60_000 + 59_999), "02:20");
    assert_eq!(clock(-1), "23:59");
    let said = |value: ContextValue, today: i64| {
        super::value(&chip("time", "Time", value, ContextSetting::Fixed), today)
    };
    // Today, its times alone; another day this year, with its date; another year, with its year; across midnight, each
    // end with its date.
    assert_eq!(said(window(140, 170), TODAY), "02:20–02:50 UTC");
    assert_eq!(said(window(140, 170), TODAY + 1), "11-12 02:20–02:50 UTC");
    assert_eq!(
        said(window(140, 170), TODAY + 60),
        "2026-11-12 02:20–02:50 UTC"
    );
    assert_eq!(said(window(-20, 10), TODAY), "11-11 23:40–11-12 00:10 UTC");
    // Today is this device's day, by its clock.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert!((today() - day_of(now)).abs() <= 1);
}

#[test]
fn a_chip_says_its_value_a_live_window_in_minutes_or_hours() {
    let live = |minutes: u32| {
        value(
            &chip(
                "time",
                "Time",
                ContextValue::Live { minutes },
                ContextSetting::Follows,
            ),
            TODAY,
        )
    };
    assert_eq!(live(15), "last 15 min");
    assert_eq!(live(90), "last 90 min");
    assert_eq!(live(60), "last 1 h");
    assert_eq!(live(1_440), "last 24 h");
    let region = chip(
        "region",
        "Region",
        text("EU-WEST1"),
        ContextSetting::Follows,
    );
    assert_eq!(value(&region, TODAY), "EU-WEST1");
    let unknown = chip(
        "season",
        "Season",
        ContextValue::Unknown,
        ContextSetting::Follows,
    );
    assert_eq!(value(&unknown, TODAY), "?");
}

#[test]
fn at_most_three_are_shown_those_fixed_first_and_the_rest_counted() {
    let context = [
        chip("a", "A", text("1"), ContextSetting::Follows),
        chip("b", "B", text("2"), ContextSetting::Inherited),
        chip("c", "C", text("3"), ContextSetting::Fixed),
        chip(
            "time",
            "Time",
            ContextValue::Live { minutes: 15 },
            ContextSetting::Follows,
        ),
    ];
    let parameters: Vec<&str> = shown(&context)
        .iter()
        .map(|chip| chip.parameter.as_str())
        .collect();
    assert_eq!(parameters, ["c", "b", "a"]);
    assert!(shown(&[]).is_empty());
    let laid = lay(&context, 40, 0, 0, TODAY);
    assert_eq!(plain(&laid), " C 3   B 2   A 1  +1");
    assert_eq!(laid.left, 1);
}

#[test]
fn fixed_inherited_and_following_chips_look_apart() {
    let laid = lay(
        &[
            chip("a", "A", text("1"), ContextSetting::Fixed),
            chip("b", "B", text("2"), ContextSetting::Inherited),
            chip("c", "C", text("3"), ContextSetting::Follows),
        ],
        40,
        0,
        0,
        TODAY,
    );
    let backings: Vec<(Rgba, Option<f32>)> = laid
        .shapes
        .iter()
        .map(|shape| match *shape {
            Shape::Rect { color, outline, .. } => (color, outline),
            _ => panic!("a chip is drawn of rounded boxes"),
        })
        .collect();
    // Fixed: filled and outlined brightly, its text bold; inherited: filled fainter, no outline; following: outlined
    // faintly, unfilled.
    assert_eq!(
        backings,
        [
            (FIXED.fill.unwrap(), None),
            (FIXED.outline.unwrap(), Some(OUTLINE)),
            (INHERITED.fill.unwrap(), None),
            (FOLLOWS.outline.unwrap(), Some(OUTLINE)),
        ]
    );
    assert!(laid.cells[1].1.bold && !laid.cells[7].1.bold);
    assert_ne!(laid.cells[1].1.color, laid.cells[7].1.color);
}

#[test]
fn chips_are_laid_out_whole_or_as_values_or_cut_to_fit_each_with_its_columns() {
    let context = [
        chip("region", "Region", text("EU-WEST1"), ContextSetting::Fixed),
        chip("time", "Time", window(140, 170), ContextSetting::Inherited),
    ];
    // Wide enough, each its label and value, a cell of padding either side and one between.
    let laid = lay(&context, 60, 2, 1, TODAY);
    assert_eq!(plain(&laid), " Region EU-WEST1   Time 02:20–02:50 UTC ");
    assert_eq!(
        laid.spans,
        [("region".to_owned(), 2, 19), ("time".to_owned(), 20, 42)]
    );
    assert_eq!((laid.row, laid.left), (1, 0));
    let Shape::Rect { x, y, width, .. } = laid.shapes[0] else {
        panic!("a chip's backing is a rounded box");
    };
    assert_eq!(
        (x, y),
        (
            (2 * CELL_WIDTH) as f32 + INSET.0,
            CELL_HEIGHT as f32 + INSET.1
        )
    );
    assert_eq!(width, (17 * CELL_WIDTH) as f32 - 2.0 * INSET.0);
    assert_eq!(at(&laid, 2), Some("region"));
    assert_eq!(at(&laid, 18), Some("region"));
    assert_eq!(at(&laid, 19), None);
    assert_eq!(at(&laid, 41), Some("time"));
    assert_eq!(at(&laid, 42), None);
    // A slot's width: their values alone.
    assert_eq!(
        plain(&lay(&context, 34, 0, 1, TODAY)),
        " EU-WEST1   02:20–02:50 UTC "
    );
    // Narrower still: each cut to its share, ending with "…", within the row.
    let narrow = lay(&context, 20, 0, 1, TODAY);
    assert!(narrow.cells.len() <= 20, "{}", plain(&narrow));
    assert!(plain(&narrow).contains('…'), "{}", plain(&narrow));
    assert_eq!(narrow.spans.len(), 2);
    // Too narrow for even one: none, counted.
    let none = lay(&context, 2, 0, 1, TODAY);
    assert_eq!(
        (plain(&none), none.spans.len(), none.left),
        ("+2".to_owned(), 0, 2)
    );
    assert_eq!(
        lay(&[], 34, 0, 1, TODAY),
        Laid {
            row: 1,
            ..Laid::default()
        }
    );
}

#[test]
fn chips_stand_before_a_panel_s_metadata_where_both_fit_or_else_on_a_row_of_their_own() {
    let after = [Span::new("published 3 min ago", Style::plain(DIM))];
    let rows = |context: &[ContextChip], columns: usize| {
        let (metadata, rule, laid) = header(context, (&after, &after), columns, TODAY);
        (metadata.plain(), rule.plain(), laid)
    };
    let region = [chip(
        "region",
        "Region",
        text("EU-WEST1"),
        ContextSetting::Fixed,
    )];
    // Wide enough, on the metadata's row, before it.
    let (metadata, rule, laid) = rows(&region, 60);
    assert_eq!(metadata, " Region EU-WEST1  · published 3 min ago");
    assert_eq!(laid.row, 1);
    assert!(rule.chars().all(|cell| cell == '─'), "{rule:?}");
    // In a slot, the metadata whole on its row, the chips on their own in place of the rule, a thin rule below them.
    let (metadata, chips, laid) = rows(&region, 34);
    assert_eq!(metadata, "published 3 min ago");
    assert_eq!(chips, " Region EU-WEST1 ");
    assert_eq!(laid.row, 2);
    assert!(matches!(laid.shapes.last(), Some(Shape::Fill { .. })));
    // Without chips, the metadata and the rule; without metadata, the chips before nothing.
    let (metadata, _, laid) = rows(&[], 34);
    assert_eq!(
        (metadata.as_str(), laid.spans.len()),
        ("published 3 min ago", 0)
    );
    let (metadata, ..) = {
        let (metadata, rule, laid) = header(&region, (&[], &[]), 34, TODAY);
        (metadata.plain(), rule.plain(), laid)
    };
    assert_eq!(metadata, " Region EU-WEST1 ");
    // Where they stand is decided by what it always says, never by what is added to it as it is drawn.
    let longer = [Span::new(
        "rows 1–9 of 30 · published 3 min ago",
        Style::plain(DIM),
    )];
    let (metadata, _, laid) = header(&region, (&after, &longer), 40, TODAY);
    assert_eq!(laid.row, 1);
    assert!(
        metadata
            .plain()
            .starts_with(" Region EU-WEST1  · rows 1–9 of 30")
    );
}

#[test]
fn a_frozen_copy_s_time_chip_says_so_and_one_that_freezes_says_nothing_more() {
    // The bundled fonts draw the mark.
    assert!(tomori_text::fonts::covers('❄'));
    let time = |press: ChipPress| {
        chip("time", "Time", window(140, 170), ContextSetting::Fixed).with_press(press)
    };
    assert_eq!(
        plain(&lay(&[time(ChipPress::Unfreeze)], 60, 0, 0, TODAY)),
        " ❄ Time 02:20–02:50 UTC "
    );
    assert_eq!(
        plain(&lay(&[time(ChipPress::Unfreeze)], 22, 0, 0, TODAY)),
        " ❄ 02:20–02:50 UTC "
    );
    assert_eq!(
        plain(&lay(&[time(ChipPress::Freeze)], 60, 0, 0, TODAY)),
        " Time 02:20–02:50 UTC "
    );
}
