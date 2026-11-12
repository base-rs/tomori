// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The chips of a wall's or a panel's context, read-only: what each parameter its environment's panels depend on, and
//! their time, shows, as a small rounded backing behind its label and value, its look saying whether it follows the
//! environment, is fixed by its own setting or inherited from its wall's. At most [`SHOWN`] on a wall or a panel, those
//! fixed first, and `+N` for those left out, laid out in one row of cells beside a panel's metadata, or on a row of
//! their own where both would not fit, or after a wall's label, in the face drawn once and kept, so that they cost
//! nothing a frame. Times are written in UTC, saying so, with the date where it is not today. Only text, list and chart
//! panels draw chips, as only those are what an environment publishes. See `design/panel-context.md`.
use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};

use crate::shapes::{Mark, Shape};
use crate::transcript::DIM;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Cell, Rgba, Row, Span, Style, cut, width};

/// The most chips shown on a wall or a panel.
pub const SHOWN: usize = 3;

/// How a chip looks: its text, whether that is bold, its backing, if it has one, and its outline, if it has one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    ink: Rgba,
    bold: bool,
    fill: Option<Rgba>,
    outline: Option<Rgba>,
}

/// A chip following the environment: dim text, no backing, a faint outline.
const FOLLOWS: Look = Look {
    ink: [0.62, 0.68, 0.76, 1.0],
    bold: false,
    fill: None,
    outline: Some([0.36, 0.42, 0.5, 0.9]),
};
/// A chip fixed by its own setting: bold white text on a teal backing, outlined brightly.
const FIXED: Look = Look {
    ink: [1.0, 1.0, 1.0, 1.0],
    bold: true,
    fill: Some([0.06, 0.36, 0.42, 0.98]),
    outline: Some([0.35, 0.95, 1.0, 1.0]),
};
/// A chip inherited from its wall's setting: light text on a fainter, greyer backing, without an outline.
const INHERITED: Look = Look {
    ink: [0.8, 0.88, 0.92, 1.0],
    bold: false,
    fill: Some([0.13, 0.2, 0.25, 0.95]),
    outline: None,
};

/// How far a chip's backing stands inside its cells, across and down, in font pixels: little, so that its text has room
/// about it, while two side by side keep the cell between them.
const INSET: (f32, f32) = (0.5, 0.5);

/// How wide a chip's outline is, in font pixels.
const OUTLINE: f32 = 1.0;

/// A chip's look, by where its value comes from.
fn look(setting: ContextSetting) -> Look {
    match setting {
        ContextSetting::Fixed => FIXED,
        ContextSetting::Inherited => INHERITED,
        _ => FOLLOWS,
    }
}

/// The civil date of `days` since the Unix epoch, as year, month and day, in the proleptic Gregorian calendar.
fn civil(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The day the instant `ms`, milliseconds since the Unix epoch, falls on, in days since the epoch, in UTC.
fn day_of(ms: i64) -> i64 {
    ms.div_euclid(86_400_000)
}

/// Today, in UTC, in days since the Unix epoch, by this device's clock, as a chip's window is written against it.
pub fn today() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| day_of(since.as_millis() as i64))
}

/// The instant `ms` as its time of day in UTC, to the minute, `02:20`.
fn clock(ms: i64) -> String {
    let minutes = ms.div_euclid(60_000).rem_euclid(1_440);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The day `day` as a chip writes it before a time, against `today`: `11-12` this year, or `2025-11-12` another.
fn date(day: i64, today: i64) -> String {
    let ((year, month, date), (this_year, ..)) = (civil(day), civil(today));
    if year == this_year {
        format!("{month:02}-{date:02}")
    } else {
        format!("{year:04}-{month:02}-{date:02}")
    }
}

/// A window from `from_ms` to `to_ms` as a chip says it, in UTC, against `today`: `02:20–02:50 UTC` within today,
/// `11-12 02:20–02:50 UTC` within another day, or each end with its date, `11-11 23:40–11-12 00:10 UTC`, across
/// midnight.
fn window(from_ms: i64, to_ms: i64, today: i64) -> String {
    let (from_day, to_day) = (day_of(from_ms), day_of(to_ms));
    let (from, to) = (clock(from_ms), clock(to_ms));
    if from_day != to_day {
        format!(
            "{} {from}–{} {to} UTC",
            date(from_day, today),
            date(to_day, today)
        )
    } else if from_day == today {
        format!("{from}–{to} UTC")
    } else {
        format!("{} {from}–{to} UTC", date(from_day, today))
    }
}

/// What a chip says its parameter is, against `today`: its value, `last 30 min` or `last 2 h` for a live window, or its
/// window in UTC.
pub fn value(chip: &ContextChip, today: i64) -> String {
    match &chip.value {
        ContextValue::Text { text } => text.clone(),
        ContextValue::Live { minutes } if *minutes >= 60 && minutes % 60 == 0 => {
            format!("last {} h", minutes / 60)
        }
        ContextValue::Live { minutes } => format!("last {minutes} min"),
        ContextValue::Window { from_ms, to_ms } => window(*from_ms, *to_ms, today),
        _ => "?".to_owned(),
    }
}

/// The chips of `context` that are shown, at most [`SHOWN`]: those fixed, then those inherited, then those following,
/// each in the order given.
pub fn shown(context: &[ContextChip]) -> Vec<&ContextChip> {
    let rank = |chip: &&ContextChip| match chip.setting {
        ContextSetting::Fixed => 0,
        ContextSetting::Inherited => 1,
        _ => 2,
    };
    let mut shown: Vec<&ContextChip> = context.iter().collect();
    shown.sort_by_key(rank);
    shown.truncate(SHOWN);
    shown
}

/// A row of chips laid out: its cells, the backings and outlines drawn behind them, each chip's parameter with the
/// columns it takes, from its first to just past its last, so that the ray finds the chip it meets, the row of the face
/// it is on, and how many chips are left out, which `+N` after them says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Laid {
    pub cells: Vec<Cell>,
    pub shapes: Vec<Shape>,
    pub spans: Vec<(String, usize, usize)>,
    pub row: usize,
    pub left: usize,
}

/// The chips `context` shows, laid out in at most `columns` cells from column `at` of the row `row` of a face, their
/// windows written against `today`: each its label and value, `Region EU-WEST1`, with a cell of padding either side and
/// a cell between them; or, where they do not fit, their values alone; or else each cut to its share of the row, ending
/// with `…`; and `+N` after them for those left out, at most [`SHOWN`] shown, and those that do not fit at all. Nothing
/// for a context of none.
pub fn lay(context: &[ContextChip], columns: usize, at: usize, row: usize, today: i64) -> Laid {
    // Room for `+N`, where any is left out: beyond those shown, or, laid out again, those that do not fit.
    let beyond = context.len() > SHOWN;
    let laid = lay_within(context, columns, (at, row), today, beyond);
    if laid.left > 0 && !beyond {
        return lay_within(context, columns, (at, row), today, true);
    }
    laid
}

/// The chips `context` shows, laid out as [`lay`] says, keeping room for `+N` at the end where `counted`.
fn lay_within(
    context: &[ContextChip],
    columns: usize,
    (at, row): (usize, usize),
    today: i64,
    counted: bool,
) -> Laid {
    let shown = shown(context);
    let mut laid = Laid {
        row,
        ..Laid::default()
    };
    if shown.is_empty() {
        return laid;
    }
    let columns = columns.saturating_sub(if counted { 3 } else { 0 });
    let labelled: Vec<String> = shown
        .iter()
        .map(|chip| format!("{} {}", chip.label, value(chip, today)))
        .collect();
    let values: Vec<String> = shown.iter().map(|chip| value(chip, today)).collect();
    // Each chip takes its text and a cell either side, and one between it and the next.
    let room = |texts: &[String]| {
        texts.iter().map(|text| width(text) + 2).sum::<usize>() + texts.len() - 1
    };
    let texts = if room(&labelled) <= columns {
        labelled
    } else {
        values
    };
    let share = (columns + 1) / shown.len();
    let mut fitted = 0;
    for (chip, text) in shown.iter().zip(texts) {
        let look = look(chip.setting);
        let style = if look.bold {
            Style::bold(look.ink)
        } else {
            Style::plain(look.ink)
        };
        let most = if room(std::slice::from_ref(&text)) <= columns.saturating_sub(laid.cells.len())
        {
            usize::MAX
        } else {
            share.saturating_sub(3)
        };
        let cells = cut(&[Span::new(text, style)], most);
        let begin = laid.cells.len();
        if cells.is_empty() || begin + cells.len() + 2 > columns {
            break;
        }
        laid.cells.push((' ', style));
        laid.cells.extend(cells);
        laid.cells.push((' ', style));
        let end = laid.cells.len();
        let (x, y) = (
            ((at + begin) * CELL_WIDTH) as f32 + INSET.0,
            (row * CELL_HEIGHT) as f32 + INSET.1,
        );
        let (width, height) = (
            ((end - begin) * CELL_WIDTH) as f32 - 2.0 * INSET.0,
            CELL_HEIGHT as f32 - 2.0 * INSET.1,
        );
        let radius = height / 2.0;
        laid.shapes.extend(look.fill.map(|color| Shape::Rect {
            x,
            y,
            width,
            height,
            color,
            radius,
            outline: None,
        }));
        laid.shapes.extend(look.outline.map(|color| Shape::Rect {
            x,
            y,
            width,
            height,
            color,
            radius,
            outline: Some(OUTLINE),
        }));
        laid.spans
            .push((chip.parameter.clone(), at + begin, at + end));
        laid.cells.push((' ', style));
        fitted += 1;
    }
    laid.cells.pop_if(|cell| cell.0 == ' ');
    laid.left = context.len() - fitted;
    if laid.left > 0 {
        let more = format!(
            "{}+{}",
            if laid.cells.is_empty() { "" } else { " " },
            laid.left
        );
        laid.cells.extend(cut(
            &[Span::new(more, Style::plain(FOLLOWS.ink))],
            usize::MAX,
        ));
    }
    laid
}

/// What a panel's face shows on the two rows below its title: its metadata, `after`, such as its age or a chart's
/// status, and its context's chips, laid out on `columns`, their windows against `today`. The chips stand before the
/// metadata on the first, a dim ` · ` between them, where both fit whole, the rule below; or else on a row of their own,
/// the second, in place of the rule, a thin rule drawn below them, so that neither crowds the other. Without chips, the
/// metadata and the rule. Which is decided by `decided`, the metadata as it always is, so that where they stand never
/// changes with what `after` adds to it as it is drawn, such as which of a list's rows show as it scrolls. Returns the
/// two rows and the chips as laid out, whose shapes are drawn behind them.
pub fn header(
    context: &[ContextChip],
    (decided, after): (&[Span], &[Span]),
    columns: usize,
    today: i64,
) -> (Row, Row, Laid) {
    let rule = Mark::Rule.row(columns, Style::plain(DIM));
    let rest = cut(decided, usize::MAX);
    let inline = lay(context, columns, 0, 1, today);
    let gap = if rest.is_empty() { 0 } else { 3 };
    let whole = inline.spans.len() == shown(context).len()
        && inline.cells.len() + gap + rest.len() <= columns;
    let rest = cut(after, usize::MAX);
    if inline.cells.is_empty() || whole {
        let mut cells = inline.cells.clone();
        if !rest.is_empty() {
            if !cells.is_empty() {
                cells.extend(cut(
                    &[Span::new(" · ", Style::plain(FOLLOWS.ink))],
                    usize::MAX,
                ));
            }
            cells.extend(cut(after, columns.saturating_sub(cells.len())));
        }
        return (Row::new(cells), rule, inline);
    }
    let mut own = lay(context, columns, 0, 2, today);
    own.shapes.push(Shape::Fill {
        x: 0.0,
        y: (3 * CELL_HEIGHT) as f32 - 1.0,
        width: (columns * CELL_WIDTH) as f32,
        height: 1.0,
        color: DIM,
    });
    (
        Row::new(cut(after, columns)),
        Row::new(own.cells.clone()),
        own,
    )
}

/// The parameter of the chip at `column` of a row `laid` out, if one is there.
pub fn at(laid: &Laid, column: usize) -> Option<&str> {
    laid.spans
        .iter()
        .find(|(_, from, to)| (*from..*to).contains(&column))
        .map(|(parameter, ..)| parameter.as_str())
}

#[cfg(test)]
mod tests;
