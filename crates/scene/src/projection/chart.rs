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
use crate::chart::{Area, Drawing, draw, legend, tone};
use crate::shapes::Mark;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Cell, Row, Span, Style, cut};

/// How many charts' drawings are kept, at the sizes they were drawn at, so that a chart is laid out again only when it,
/// or its size, changes, rather than every frame its face is made: more than the panels an environment publishes.
pub const DRAWN: usize = 32;

/// A chart's legend and drawing, at a face of so many columns and rows below its heading.
type Drawn = (Chart, (usize, usize), Rc<(Vec<Cell>, Drawing)>);

thread_local! {
    /// The charts drawn most recently, the newest last.
    static CACHE: RefCell<Vec<Drawn>> = const { RefCell::new(Vec::new()) };
}

/// `chart`'s legend and drawing on a face `columns` wide with `body` rows below its heading, as [`face`] lays them out,
/// from [`CACHE`] where it has been drawn at that size, and drawn and kept otherwise, forgetting the least recent beyond
/// [`DRAWN`].
fn drawn(chart: &Chart, columns: usize, body: usize) -> Rc<(Vec<Cell>, Drawing)> {
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
        let legend_cells = if chart.legend {
            legend(chart, columns)
        } else {
            Vec::new()
        };
        let top = 3 + usize::from(chart.legend);
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

/// `panel`'s face, a chart panel of `chart`, framed by `border` and sized as `fit` says: its title, bold, a row of its
/// status, in its tone, and its unit, a rule, its legend, where it shows one, and its chart in the rows below.
pub(super) fn face(panel: &Panel, chart: &Chart, fit: Fit, border: Option<[f32; 4]>) -> Canvas {
    let chart = &crate::chart::bounded(chart);
    let (columns, body) = self::fit(fit);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = border;
    // Its status, in its tone, and its unit, but not its age, since a live chart says how fresh it is in its status, and
    // the time since it was first published says nothing of what it shows.
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
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), columns),
        Row::new(cut(&about, columns)),
        Mark::Rule.row(columns, Style::plain(DIM)),
    ];
    let drawn = drawn(chart, columns, body);
    if chart.legend {
        rows.push(Row::new(drawn.0.clone()));
    }
    rows.resize_with(3 + body, Row::default);
    canvas.rows = rows;
    canvas.shapes = drawn.1.shapes.clone();
    canvas.labels = drawn.1.labels.clone();
    canvas
}

#[cfg(test)]
mod tests;
