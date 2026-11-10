// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::chart::fixtures;
use tomori_protocol::Place;

fn panel(chart: Chart) -> Panel {
    Panel::new(
        "panel-1",
        "Scaling pressure",
        chart.summary(),
        Place::Unknown,
    )
    .with_chart(chart)
}

#[test]
fn a_chart_panel_fills_its_slot_with_its_legend_and_chart() {
    let panel = panel(fixtures::pressure());
    let canvas = super::super::face(&panel, false, false, Fit::SLOT);
    // A slot's size, as a text panel's.
    assert_eq!(
        super::super::fitted_size(&panel, Fit::SLOT),
        super::super::panel_size()
    );
    assert_eq!(canvas.size(), super::super::panel_size());
    assert_eq!(canvas.rows[0].plain().trim_end(), "Scaling pressure");
    assert_eq!(canvas.rows[1].plain().trim_end(), "7 of 8 reporting");
    assert!(canvas.rows[3].plain().starts_with("── mean"));
    // Its chart in the rows below, its summary, for clients that cannot draw it, not shown.
    assert!(!canvas.plain().contains("now\n"));
    assert!(canvas.shapes.len() > 100);
    assert!(
        canvas
            .labels
            .iter()
            .any(|label| crate::ui_text::plain(&label.cells) == "now")
    );
    // A wider slot draws it wider; floating, it is as wide as reading allows.
    let wide = super::super::Cells {
        columns: 60,
        ..super::super::Cells::STANDARD
    };
    assert_eq!(
        super::super::face(&panel, false, false, Fit::Slot(wide)).columns,
        60
    );
    let floating = super::super::face(&panel, true, false, Fit::Text);
    assert_eq!(
        (floating.columns, floating.rows.len()),
        (FLOATING.0, 3 + FLOATING.1)
    );
    assert_eq!(super::super::fit(&panel, Fit::Text), FLOATING);
}

#[test]
fn a_chart_panel_s_heading_says_its_unit_and_without_a_legend_its_chart_takes_the_row() {
    let mut chart = fixtures::provisioning();
    chart.legend = false;
    let canvas = super::super::face(&panel(chart), false, true, Fit::SLOT);
    assert_eq!(canvas.rows[1].plain().trim_end(), "replicas");
    assert!(canvas.rows[3].plain().trim().is_empty());
    // With a status, its unit after it; its age never.
    let mut chart = fixtures::pressure();
    chart.axis.unit = "load".into();
    let canvas = super::super::face(&panel(chart), false, false, Fit::SLOT);
    assert_eq!(canvas.rows[1].plain().trim_end(), "7 of 8 reporting · load");
}

#[test]
fn a_chart_is_laid_out_again_only_when_it_or_its_size_changes() {
    let chart = fixtures::pressure();
    let first = drawn(&chart, 34, 12);
    assert!(Rc::ptr_eq(&first, &drawn(&chart, 34, 12)));
    // Another size, or another chart, is drawn anew.
    assert!(!Rc::ptr_eq(&first, &drawn(&chart, 60, 12)));
    let mut changed = chart.clone();
    changed.series[0].values[180] = Some(0.5);
    assert!(!Rc::ptr_eq(&first, &drawn(&changed, 34, 12)));
    // Beyond so many, as many as the panels an environment may publish, the least recent is forgotten.
    assert_eq!(DRAWN, tomori_environment::PANEL_LIMIT);
    for columns in 0..DRAWN {
        drawn(&chart, 100 + columns, 12);
    }
    assert!(!Rc::ptr_eq(&first, &drawn(&chart, 34, 12)));
}
