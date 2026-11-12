// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A code panel's code laid out, wrapped and numbered, and its face drawn from that layout, scrolled. See
//! `design/projection.md`.
use tomori_protocol::{CODE_LINES, Code, Panel, TokenKind};

use super::{
    CODE_PADDING, CODE_PIXEL, CODE_ROOM, CONTINUED, CONTINUED_COLOR, FIT_CODE_ROWS, Fit, GUTTER,
    about, palette, scroll_bar,
};
use crate::{
    canvas::Canvas,
    shapes::Mark,
    transcript::{BRIGHT, DIM, TEXT},
    ui_text::{Row, Style, cut},
};

/// One row of a code panel's code, as wrapped: the line it is of, from 0, whether it is that line's first, and its
/// cells after the gutter.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CodeRow {
    pub(super) line: usize,
    pub(super) first: bool,
    cells: Vec<crate::ui_text::Cell>,
}

/// A code panel's code laid out: every row its lines take, each wrapped as [`wrap_code`] wraps it at its columns of code,
/// coloured by kind as [`palette`] colours them, and how many of its block's lines it holds and was not sent, so that
/// its face shows any part of it. Spans that are not valid for its body, as `valid_spans` checks them, are ignored, and
/// the code laid out plain. Laid out once for each panel's code, rather than each frame, as a panel holds up to
/// [`CODE_LINES`] lines.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeLayout {
    pub(super) rows: Vec<CodeRow>,
    lines: usize,
    unsent: usize,
    /// The columns of its face, its gutter and its code.
    pub(super) columns: usize,
}

impl CodeLayout {
    /// The layout of `panel`'s code, `code`, on a face `columns` wide, its gutter and its code.
    pub fn new(panel: &Panel, code: &Code, columns: usize) -> Self {
        // The kind of each character of the body, in order.
        let mut kinds = vec![TokenKind::Plain; panel.body.chars().count()];
        if tomori_protocol::valid_spans(&panel.body, &code.spans) {
            for span in &code.spans {
                kinds[span.start as usize..span.end() as usize].fill(span.kind);
            }
        }
        let mut at = 0;
        let lines: Vec<&str> = panel.body.split('\n').take(CODE_LINES).collect();
        let step = indent_step(&lines);
        let room = columns - GUTTER;
        let mut rows = Vec::new();
        for (number, line) in lines.iter().enumerate() {
            let mut spans: Vec<crate::ui_text::Span> = Vec::new();
            for (index, c) in line.chars().enumerate() {
                let style = Style::plain(palette(kinds[at + index]));
                // Joined to the run before, of the same kind.
                match spans.last_mut() {
                    Some(last) if last.style == style => last.text.push(c),
                    _ => spans.push(crate::ui_text::Span::new(c.to_string(), style)),
                }
            }
            at += line.chars().count() + 1;
            let mut wrapped = wrap_code(&cut(&spans, usize::MAX), room, step);
            // A line cut by the server ends with a dim "…": past what the panel holds, or, from an older server, at 64
            // characters.
            if let Some(last) = wrapped
                .last_mut()
                .and_then(|row| row.last_mut())
                .filter(|(c, _)| *c == '…')
            {
                last.1 = Style::plain(DIM);
            }
            rows.extend(
                wrapped
                    .into_iter()
                    .enumerate()
                    .map(|(index, cells)| CodeRow {
                        line: number,
                        first: index == 0,
                        cells,
                    }),
            );
        }
        Self {
            rows,
            lines: lines.len(),
            unsent: (code.lines as usize).saturating_sub(lines.len()),
            columns,
        }
    }

    /// How many rows of code its face has, sized as `fit` says: a slot's, or, floating, as many as its code takes, within
    /// [`FIT_CODE_ROWS`].
    fn height(&self, fit: Fit) -> usize {
        match fit {
            Fit::Slot(cells) => cells.code_rows,
            Fit::Text => self.rows.len().clamp(FIT_CODE_ROWS.0, FIT_CODE_ROWS.1),
        }
    }

    /// Whether its face, sized as `fit` says, has a row below its code saying what there is above and below it: always
    /// in a slot, and floating, only where it does not show all of its block.
    fn footed(&self, fit: Fit) -> bool {
        matches!(fit, Fit::Slot(_)) || self.rows.len() > self.height(fit) || self.unsent > 0
    }

    /// How many rows its face has, sized as `fit` says: its title, metadata and rule, its rows of code, and the row
    /// below them, where it has one.
    pub(super) fn face_rows(&self, fit: Fit) -> usize {
        3 + self.height(fit) + usize::from(self.footed(fit))
    }

    /// How many rows down it scrolls at most, sized as `fit` says: so far that its last row is its face's last.
    pub(super) fn most_scroll(&self, fit: Fit) -> usize {
        self.rows.len().saturating_sub(self.height(fit))
    }
}

/// A code panel's face, its code laid out as `layout`, sized as `fit` says and scrolled `scroll` rows down, as far as it
/// goes: its title, bold, its language, length and age, a rule, and its rows of code from there, each line's first after
/// its number and its later rows marked as continued in the gutter, then a row saying how many lines are above and below
/// them, and how many are only in the transcript. A line that does not fit whole below the first shown is left for the
/// row below, and the first, were it alone too tall for what is left, ends with a dim "…".
pub(super) fn code_face(panel: &Panel, layout: &CodeLayout, fit: Fit, scroll: usize) -> Canvas {
    let height = layout.height(fit);
    let scroll = scroll.min(layout.most_scroll(fit));
    let columns = layout.columns;
    let room = columns - GUTTER;
    let mut canvas = Canvas::new(columns, CODE_PIXEL);
    canvas.padding = CODE_PADDING;
    canvas.background = [0.015, 0.025, 0.045, 0.95];
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), columns),
        Row::text(&about(panel), Style::plain(DIM), columns),
        Mark::Rule.row(columns, Style::plain(DIM)),
    ];
    let first = layout.rows.get(scroll).map(|row| row.line);
    let mut at = scroll;
    let mut shown: Vec<&CodeRow> = Vec::new();
    while let Some(row) = layout.rows.get(at)
        && shown.len() < height
    {
        // A later line that does not fit whole is left for the row below, which counts it.
        if row.first && Some(row.line) != first {
            let tall = layout.rows[at..]
                .iter()
                .take_while(|next| next.line == row.line)
                .count();
            if tall > height - shown.len() {
                break;
            }
        }
        shown.push(row);
        at += 1;
    }
    // The first line shown, cut by the face's end, ends with a dim "…".
    let cut_short = layout.rows.get(at).is_some_and(|row| !row.first);
    for (index, row) in shown.iter().enumerate() {
        // The line's number on its first row only; its later rows are marked as continued.
        let gutter = match row.first {
            true => crate::ui_text::Span::new(
                format!("{:>width$} ", row.line + 1, width = GUTTER - 1),
                Style::plain(DIM),
            ),
            false => crate::ui_text::Span::new(
                format!("{CONTINUED:>width$} ", width = GUTTER - 1),
                Style::plain(CONTINUED_COLOR),
            ),
        };
        let mut cells = row.cells.clone();
        if cut_short && index == shown.len() - 1 {
            let mut last = vec![cells, Vec::new()];
            crate::ui_text::fit(&mut last, 1, room);
            cells = last.remove(0);
            // `fit` ends it with "…", the last cell.
            let end = cells.len() - 1;
            cells[end].1 = Style::plain(DIM);
        }
        let mut drawn = cut(&[gutter], GUTTER);
        drawn.extend(cells);
        rows.push(Row::new(drawn));
    }
    rows.resize(3 + height, Row::default());
    if layout.footed(fit) {
        rows.push(Row::text(
            &around(layout, scroll, at),
            Style::plain(DIM),
            columns,
        ));
    }
    canvas.rows = rows;
    canvas.shapes = scroll_bar(columns, height, layout.rows.len(), scroll);
    canvas
}

/// What the row below a code panel's code says of `layout`, its rows shown from `scroll` until `end`: how many lines
/// start above them, as `↑ 3 above`; how many are not shown whole below them, those only in the transcript among them,
/// as `↓ 4 more`; or, below the last it holds, how many it was not sent, as `180 more lines in the transcript`.
fn around(layout: &CodeLayout, scroll: usize, end: usize) -> String {
    let above = layout
        .rows
        .get(scroll)
        .filter(|_| scroll > 0)
        .map_or(0, |row| row.line + usize::from(!row.first));
    let below = layout
        .rows
        .get(end)
        .map_or(0, |row| layout.lines - row.line);
    let mut parts = Vec::new();
    if above > 0 {
        parts.push(format!("↑ {above} above"));
    }
    match (below, layout.unsent) {
        (0, 0) => {}
        (0, 1) => parts.push("1 more line in the transcript".to_owned()),
        (0, unsent) => parts.push(format!("{unsent} more lines in the transcript")),
        (below, unsent) => parts.push(format!("↓ {} more", below + unsent)),
    }
    parts.join(" · ")
}

/// The indent step of code's `lines`: the least indentation any line has, of two to four spaces, or four where none is
/// indented.
pub(super) fn indent_step(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|c| *c == ' ').count())
        .filter(|indent| *indent > 0)
        .min()
        .map_or(4, |step| step.clamp(2, 4))
}

/// Whether a line of code may break between `before` and `after`: after a space, or after punctuation that ends or
/// opens a part of it, such as a comma, an operator or an opening bracket, unless a closing one follows.
fn code_breaks(before: char, after: char) -> bool {
    before == ' '
        || (",;([{.:=&|+-*/<>!?".contains(before) && !")]}>,;.:".contains(after) && after != ' ')
}

/// One line of code's `cells`, as drawn, on rows of `room` cells: as it is where it fits, and otherwise broken where
/// [`code_breaks`] allows, in the latter half of the row, or else at the row's end, never through a wide character.
/// Its later rows hang one indent `step` under its own indentation, keeping at least [`CODE_ROOM`] cells for its code,
/// and the spaces at each break are dropped. Each cell keeps its style, so that highlighting carries across a break.
pub(super) fn wrap_code(
    cells: &[crate::ui_text::Cell],
    room: usize,
    step: usize,
) -> Vec<Vec<crate::ui_text::Cell>> {
    use crate::ui_text::WIDE;
    let indent = cells.iter().take_while(|(c, _)| *c == ' ').count();
    let hanging = (indent + step).min(room.saturating_sub(CODE_ROOM));
    let mut rows = Vec::new();
    let mut rest = cells;
    let mut width = room;
    loop {
        let prefix = if rows.is_empty() { 0 } else { hanging };
        if rest.len() <= width {
            rows.push(blank(prefix).chain(rest.iter().copied()).collect());
            return rows;
        }
        // The last break the line allows in the latter half of the row, after its indentation, or else the row's end,
        // before a wide character that would not fit.
        let first = (width / 2).max(if rows.is_empty() { indent + 1 } else { 1 });
        let at = (first..=width)
            .rev()
            .find(|&at| rest[at].0 != WIDE && code_breaks(rest[at - 1].0, rest[at].0))
            .unwrap_or(if rest[width].0 == WIDE {
                width - 1
            } else {
                width
            });
        let mut row: Vec<_> = blank(prefix).chain(rest[..at].iter().copied()).collect();
        while row.len() > prefix && row.last().is_some_and(|(c, _)| *c == ' ') {
            row.pop();
        }
        rows.push(row);
        rest = &rest[at..];
        let spaces = rest.iter().take_while(|(c, _)| *c == ' ').count();
        rest = &rest[spaces..];
        if rest.is_empty() {
            return rows;
        }
        width = room - hanging;
    }
}

/// `count` plain spaces.
fn blank(count: usize) -> impl Iterator<Item = crate::ui_text::Cell> {
    std::iter::repeat_n((' ', Style::plain(TEXT)), count)
}
