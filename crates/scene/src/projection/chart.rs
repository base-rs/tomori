// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A chart panel's face: its title, its status and age, a rule, its legend, and its chart, drawn in the panel's own
//! mesh as [`crate::chart`] lays it out. In a wall's slot it is a slot's size, the chart filling the rows below its
//! legend, wider in a wide slot; floating or carried, it is as wide as a comfortable reading width and a little taller.
//! Its body, a summary of what the chart shows for a client that cannot draw it, is not shown. See
//! `design/projection.md`.
use tomori_protocol::Panel;
use tomori_protocol::chart::Chart;

use std::cell::RefCell;
use std::rc::Rc;

use super::{BRIGHT, DIM, Fit, PADDING, PIXEL};
use crate::canvas::Canvas;
use crate::chart::{Area, Drawing, PlotArea, draw, legend, tone};
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Cell, Row, Span, Style};

/// How many charts' drawings are kept, at the sizes they were drawn at, so that a chart is laid out again only when it,
/// or its size, changes, rather than every time its face is made: as many as the panels an environment publishes,
/// `tomori_environment::PANEL_LIMIT`, which this crate's tests check it against, one for each at the size it stands.
/// Beyond them the least recent is drawn anew when its face is next made, which is only as its panel changes, since
/// each panel's faces are kept (see [`super::faces`]).
pub const DRAWN: usize = tomori_environment::PANEL_LIMIT;

/// A chart's legend's rows and drawing, at a face of so many columns and rows below its heading.
type Drawn = (Chart, (usize, usize), Rc<(Vec<Vec<Cell>>, Drawing)>);

thread_local! {
    /// The charts drawn most recently, the newest last.
    static CACHE: RefCell<Vec<Drawn>> = const { RefCell::new(Vec::new()) };
}

/// `chart`'s legend's rows and drawing on a face `columns` wide with `body` rows below its heading, as [`face`] lays
/// them out, from [`CACHE`] where it has been drawn at that size, and drawn and kept otherwise, forgetting the least
/// recent beyond [`DRAWN`].
fn drawn(chart: &Chart, columns: usize, body: usize) -> Rc<(Vec<Vec<Cell>>, Drawing)> {
    CACHE.with_borrow_mut(|cache| {
        if let Some(index) = cache
            .iter()
            .position(|(kept, size, _)| *size == (columns, body) && kept == chart)
        {
            let entry = cache.remove(index);
            let found = entry.2.clone();
            cache.push(entry);
            return found;
        }
        // A legend of nothing still takes its row, as one of a row does.
        let legend_cells = if chart.legend {
            let mut rows = legend(chart, columns);
            rows.resize_with(rows.len().max(1), Vec::new);
            rows
        } else {
            Vec::new()
        };
        let top = 3 + legend_cells.len();
        let drawing = draw(
            chart,
            Area {
                x: 0.0,
                y: (top * CELL_HEIGHT) as f32,
                width: (columns * CELL_WIDTH) as f32,
                height: ((3 + body - top) * CELL_HEIGHT) as f32,
            },
        );
        let found = Rc::new((legend_cells, drawing));
        if cache.len() >= DRAWN {
            cache.remove(0);
        }
        cache.push((chart.clone(), (columns, body), found.clone()));
        found
    })
}

/// The columns and rows below its heading of a chart panel floating or carried: 48 columns, a comfortable reading
/// width, and as many rows as keep a chart of that width about as tall for its width as one in a standard slot.
pub const FLOATING: (usize, usize) = (48, 14);

/// How many columns and rows below its heading `panel`'s face has, sized as `fit` says.
pub(super) fn fit(fit: Fit) -> (usize, usize) {
    match fit {
        Fit::Slot(cells) => (cells.columns, cells.body),
        Fit::Text => FLOATING,
    }
}

/// What a chart's face says below its title: its status, in its tone, and its unit, but not its age, since a live chart
/// says how fresh it is in its status, and the time since it was first published says nothing of what it shows.
pub(super) fn metadata(chart: &Chart) -> Vec<Span> {
    let mut about = Vec::new();
    if let Some(status) = &chart.status {
        about.push(Span::new(
            status.text.clone(),
            Style::plain(tone(status.tone)),
        ));
    }
    if !chart.axis.unit.is_empty() {
        let gap = if about.is_empty() { "" } else { " · " };
        about.push(Span::new(
            format!("{gap}{}", chart.axis.unit),
            Style::plain(DIM),
        ));
    }
    about
}

/// `panel`'s face, a chart panel of `chart`, framed by `border` and sized as `fit` says: its title, bold, a row of its
/// status, in its tone, and its unit, a rule, its legend's rows, where it shows one, and its chart in the rows below;
/// and where its plot is drawn, which its readout reads.
pub(super) fn face(
    panel: &Panel,
    chart: &Chart,
    fit: Fit,
    border: Option<[f32; 4]>,
) -> (Canvas, Option<PlotArea>) {
    let chart = &crate::chart::bounded(chart);
    let (columns, body) = self::fit(fit);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = border;
    let about = metadata(chart);
    // Its context's chips, where it has any, before its status and unit or on a row of their own.
    let (metadata, rule, chips) = super::chips::header(
        &panel.context,
        (&about, &about),
        columns,
        super::chips::today(),
    );
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), columns),
        metadata,
        rule,
    ];
    let drawn = drawn(chart, columns, body);
    rows.extend(drawn.0.iter().cloned().map(Row::new));
    rows.resize_with(3 + body, Row::default);
    canvas.rows = rows;
    canvas.shapes = chips.shapes;
    canvas.shapes.extend(drawn.1.shapes.iter().cloned());
    canvas.labels = drawn.1.labels.clone();
    (canvas, drawn.1.plot.clone())
}

#[cfg(test)]
mod tests;
