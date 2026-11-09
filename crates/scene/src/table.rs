// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A Markdown table laid out on rows of cells, for a panel's body and the transcript: a grid where its columns fit, and otherwise a card for each row.
//!
//! **The grid.** Each column is as wide as its widest cell where the table fits at that width; otherwise every column keeps at least [`NARROWEST`] cells, or its widest cell where that is narrower, then grows to fit its longest word, then shares what is left in proportion to what it lacks, so that a cell's text wraps within its column, at words, onto as many rows as it needs, and a word is broken only where it is wider than its column. Nothing is cut with "…". A cell is aligned as its column's delimiter row says. The header is bold on a tinted bar, with a rule below it, columns are separated by a faint line, and every other row of the body is faintly striped, so that a row that wraps reads as one.
//!
//! **Cards.** A table whose columns do not all fit at [`NARROWEST`] cells is laid out as a card for each row, one `Header: value` line for each of its cells, wrapped under a hanging indent, with a rule between cards, since a grid of columns too narrow for words reads worse than lines.
use crate::markdown_lite::{Align, Table, Text, plain};
use crate::shapes::Mark;
use crate::transcript::{BRIGHT, DIM, TEXT, styled};
use crate::ui_text::{Cell, Rgba, Span, Style, Wrap, longest_word, width, wrap};

/// The cells between two columns: a space, then a faint line at the left edge of the next, so that the line stands about
/// a cell from the text either side of it.
pub const GAP: usize = 2;
/// The line between columns, drawn near its cell's left edge, as a line rather than a glyph.
const LINE: char = Mark::Line.stand_in();
/// A cell of the rule below the header, or between cards, drawn as a line.
const RULE: Cell = (
    Mark::Rule.stand_in(),
    Style::plain(SEPARATOR).marked(Mark::Rule),
);
/// The fewest cells a column of the grid takes, unless its widest cell takes fewer: narrower, many words would break.
pub const NARROWEST: usize = 10;

/// The line between columns and the rule below the header.
const SEPARATOR: Rgba = [0.6, 0.66, 0.74, 0.45];
/// The header's bar.
const HEADER: Rgba = [0.18, 0.3, 0.46, 0.32];
/// Every other row of the body.
const STRIPE: Rgba = [0.6, 0.7, 0.85, 0.07];

/// A table laid out: its rows of cells, and how many of the first are its header and the rule below it, which a panel
/// keeps in view as its body scrolls; none for cards.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Laid {
    pub rows: Vec<Vec<Cell>>,
    pub header: usize,
}

/// `table` laid out on rows of at most `room` cells: a grid where its columns fit, and cards otherwise.
pub fn lay_out(table: &Table, room: usize) -> Laid {
    let count = table.columns();
    let every = || table.rows.iter().chain(std::iter::once(&table.header));
    let measure = |measure: &dyn Fn(&str) -> usize| -> Vec<usize> {
        (0..count)
            .map(|column| {
                every()
                    .filter_map(|row| row.get(column))
                    .map(|cell| measure(&plain(cell)))
                    .max()
                    .unwrap_or_default()
            })
            .collect()
    };
    let natural = measure(&|text| text.split('\n').map(width).max().unwrap_or_default());
    let words = measure(&longest_word);
    match widths(&natural, &words, room) {
        Some(widths) => grid(table, &widths),
        None => cards(table, room),
    }
}

/// Each column's width, in cells, for a table whose columns' widest cells take `natural` cells and longest words
/// `words`, in `room` cells: as [`lay_out`] says, or `None` where they do not all fit at [`NARROWEST`].
pub fn widths(natural: &[usize], words: &[usize], room: usize) -> Option<Vec<usize>> {
    let room = room.checked_sub(GAP * natural.len().saturating_sub(1))?;
    if natural.iter().sum::<usize>() <= room {
        return Some(natural.to_vec());
    }
    let mut widths: Vec<usize> = natural.iter().map(|&wide| wide.min(NARROWEST)).collect();
    let mut free = room.checked_sub(widths.iter().sum())?;
    let words: Vec<usize> = natural
        .iter()
        .zip(words)
        .map(|(&wide, &word)| wide.min(word))
        .collect();
    grow(&mut widths, &words, &mut free);
    grow(&mut widths, natural, &mut free);
    Some(widths)
}

/// `widths` grown towards `most`, sharing `free` cells in proportion to what each lacks, the cells left over by rounding
/// going to those that lack most of a cell, the leftmost first.
fn grow(widths: &mut [usize], most: &[usize], free: &mut usize) {
    let lack: Vec<usize> = widths
        .iter()
        .zip(most)
        .map(|(&width, &most)| most.saturating_sub(width))
        .collect();
    let total: usize = lack.iter().sum();
    if total <= *free {
        for (width, lack) in widths.iter_mut().zip(&lack) {
            *width += lack;
        }
        *free -= total;
        return;
    }
    let mut over: Vec<(usize, usize)> = Vec::new();
    let mut given = 0;
    for (column, (width, lack)) in widths.iter_mut().zip(&lack).enumerate() {
        let share = *free * lack;
        *width += share / total;
        given += share / total;
        over.push((share % total, column));
    }
    over.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, column) in over.iter().take(*free - given) {
        widths[column] += 1;
    }
    *free = 0;
}

/// The cell of `row` in `column`, empty where the row has fewer.
fn cell(row: &[Text], column: usize) -> &[(String, crate::markdown_lite::Inline)] {
    row.get(column).map(Vec::as_slice).unwrap_or_default()
}

/// `table` as a grid of columns `widths` wide.
fn grid(table: &Table, widths: &[usize]) -> Laid {
    let mut rows = Vec::new();
    if !table.header.is_empty() {
        row(
            &table.header,
            Style::bold(BRIGHT),
            Some(HEADER),
            widths,
            &table.align,
            &mut rows,
        );
        let mut rule = Vec::new();
        for (column, &width) in widths.iter().enumerate() {
            if column > 0 {
                rule.extend(std::iter::repeat_n(RULE, GAP));
            }
            rule.extend(std::iter::repeat_n(RULE, width));
        }
        rows.push(rule);
    }
    let header = rows.len();
    for (index, cells) in table.rows.iter().enumerate() {
        let stripe = (index % 2 == 1).then_some(STRIPE);
        row(
            cells,
            Style::plain(TEXT),
            stripe,
            widths,
            &table.align,
            &mut rows,
        );
    }
    Laid { rows, header }
}

/// The rows of one of a grid's rows, `cells` in `base` on `background`, each wrapped within its column, as tall as its
/// tallest cell.
fn row(
    cells: &[Text],
    base: Style,
    background: Option<Rgba>,
    widths: &[usize],
    align: &[Align],
    rows: &mut Vec<Vec<Cell>>,
) {
    let on = |style: Style| Style {
        background: style.background.or(background),
        ..style
    };
    let wrapped: Vec<Vec<Vec<Cell>>> = widths
        .iter()
        .enumerate()
        .map(|(column, &width)| wrap(&styled(cell(cells, column), base), width, Wrap::default()))
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or_default();
    let blank = (' ', on(Style::plain(base.color)));
    for line in 0..height {
        let mut out = Vec::new();
        for (column, &width) in widths.iter().enumerate() {
            if column > 0 {
                let style = on(Style::plain(SEPARATOR));
                out.extend([(' ', style), (LINE, style.marked(Mark::Line))]);
            }
            let text: Vec<Cell> = wrapped[column]
                .get(line)
                .into_iter()
                .flatten()
                .map(|&(ch, style)| (ch, on(style)))
                .collect();
            let pad = width.saturating_sub(text.len());
            let before = match align.get(column).copied().unwrap_or_default() {
                Align::Left => 0,
                Align::Centre => pad / 2,
                Align::Right => pad,
            };
            out.extend(std::iter::repeat_n(blank, before));
            out.extend(text);
            out.extend(std::iter::repeat_n(blank, pad - before));
        }
        rows.push(out);
    }
}

/// `table` as a card for each row, `room` cells wide.
fn cards(table: &Table, room: usize) -> Laid {
    let label = |column: usize| {
        let label = plain(cell(&table.header, column)).replace('\n', " ");
        (!label.is_empty()).then_some(label)
    };
    let hanging = Wrap {
        hanging: 2,
        ..Default::default()
    };
    let mut rows = Vec::new();
    if table.rows.is_empty() {
        // A header alone, its labels on a line.
        let labels: Vec<String> = (0..table.columns()).filter_map(label).collect();
        let spans = [Span::new(labels.join(" · "), Style::bold(BRIGHT))];
        rows.extend(wrap(&spans, room, hanging));
    }
    for (index, cells) in table.rows.iter().enumerate() {
        if index > 0 {
            rows.push(vec![RULE; room]);
        }
        for column in 0..table.columns() {
            let value = cell(cells, column);
            if value.is_empty() {
                continue;
            }
            let mut spans: Vec<Span> = label(column)
                .map(|label| Span::new(format!("{label}: "), Style::bold(DIM)))
                .into_iter()
                .collect();
            spans.extend(styled(value, Style::plain(TEXT)));
            rows.extend(wrap(&spans, room, hanging));
        }
    }
    Laid { rows, header: 0 }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::markdown_lite::Inline;
    use crate::ui_text::{WIDE, plain as shown};

    /// The table the maintainer could not read on the Quest 3, every row cut with "…" and nothing aligned.
    pub(crate) const MAINTAINERS: &str = "| PR | What | Recommendation |\n|---|---|---|\n| 233 | Environments get trigger, B, sticks | **Your call:** clashes with scrolling, wall B, checklist ticking |\n| 229 | Prometheus data source crate | **Your call:** own and publish it now? |\n| 231 | Environment anchors for swarm, workers | Rebase, then try on headset |\n| 234 | Backend names its avatar (on 231) | After 231, check the Space menu fits |\n| 232 | Help text, licences, program names | Rebase after 236, then merge |\n| 90 | Old environment framework proposal | Superseded: suggest closing |\n";

    fn table(source: &str) -> Table {
        crate::markdown_lite::tables(source).remove(0)
    }

    fn text(laid: &Laid) -> Vec<String> {
        laid.rows.iter().map(|row| shown(row)).collect()
    }

    /// Whether `row` is the rule below a header.
    fn rule(row: &[Cell]) -> bool {
        row.iter().all(|(ch, _)| *ch == '─')
    }

    /// Where each column's line is in each row of a grid, in cells as drawn.
    fn lines_at(row: &[Cell]) -> Vec<usize> {
        row.iter()
            .enumerate()
            .filter(|(_, (ch, _))| *ch == LINE)
            .map(|(at, _)| at)
            .collect()
    }

    /// The words of `cells`' column `column`, read down the grid's rows from `from`, as one text.
    fn column_words(
        laid: &Laid,
        from: usize,
        rows: std::ops::Range<usize>,
        column: usize,
    ) -> String {
        let mut words = Vec::new();
        for row in &laid.rows[from + rows.start..from + rows.end] {
            let text = shown(row);
            let part = text.split(LINE).nth(column).unwrap_or_default();
            words.extend(part.split_whitespace().map(str::to_owned));
        }
        words.join(" ")
    }

    #[test]
    fn a_table_that_fits_is_a_grid_at_its_natural_width_aligned_as_its_delimiters_say() {
        let laid = lay_out(
            &table(
                "| Name | Count | Note |\n|:---|---:|:---:|\n| a | 1 | mid |\n| bb | 200 | x |\n",
            ),
            40,
        );
        assert_eq!(
            text(&laid),
            [
                "Name ▏Count ▏Note",
                "─────────────────",
                "a    ▏    1 ▏mid ",
                "bb   ▏  200 ▏ x  ",
            ]
        );
        assert_eq!(laid.header, 2);
        // The header is bold on its bar, the body plain, every other row striped, and the lines faint.
        assert!(
            laid.rows[0]
                .iter()
                .all(|(_, style)| style.background == Some(HEADER))
        );
        assert!(laid.rows[0][0].1.bold && !laid.rows[2][0].1.bold);
        assert!(
            laid.rows[2]
                .iter()
                .all(|(_, style)| style.background.is_none())
        );
        assert!(
            laid.rows[3]
                .iter()
                .all(|(_, style)| style.background == Some(STRIPE))
        );
        // The line between columns is drawn as a line, as is the rule.
        assert_eq!(
            laid.rows[2][5],
            (LINE, Style::plain(SEPARATOR).marked(Mark::Line))
        );
        assert_eq!(laid.rows[1][0], RULE);
        for glyph in [LINE, '─'] {
            assert!(tomori_text::fonts::covers(glyph), "{glyph}");
        }
    }

    #[test]
    fn widths_are_natural_where_they_fit_then_shared_keeping_words_whole_where_they_can() {
        // Fitting, each its widest cell.
        assert_eq!(widths(&[3, 10, 12], &[3, 5, 6], 31), Some(vec![3, 10, 12]));
        // Not fitting: at least the narrowest, then each its longest word, then the rest shared by what each lacks.
        assert_eq!(
            widths(&[3, 38, 59], &[3, 12, 14], 60),
            Some(vec![3, 22, 31])
        );
        assert_eq!(
            widths(&[3, 38, 59], &[3, 12, 14], 60)
                .unwrap()
                .iter()
                .sum::<usize>()
                + 2 * GAP,
            60
        );
        // At a slot's 34 cells, every longest word fits, the header's "Recommendation" too; short of room for every
        // longest word, they share what there is.
        assert_eq!(
            widths(&[3, 38, 59], &[3, 12, 14], 34),
            Some(vec![3, 12, 15])
        );
        assert_eq!(
            widths(&[3, 38, 59], &[3, 12, 14], 30),
            Some(vec![3, 11, 12])
        );
        // Columns that cannot all have the narrowest are no grid, unless they need less.
        assert_eq!(
            widths(&[30, 30], &[5, 5], 2 * NARROWEST + GAP),
            Some(vec![NARROWEST; 2])
        );
        assert_eq!(widths(&[30, 30], &[5, 5], 2 * NARROWEST + GAP - 1), None);
        assert_eq!(widths(&[2, 2, 2, 2], &[2, 2, 2, 2], 17), Some(vec![2; 4]));
        assert_eq!(widths(&[2, 2, 2, 2], &[2, 2, 2, 2], 8), None);
        // Rounding gives its cells to those that lack most of one, the leftmost first.
        assert_eq!(
            widths(&[20, 20, 20], &[1, 1, 1], 36),
            Some(vec![11, 11, 10])
        );
        assert_eq!(widths(&[], &[], 10), Some(vec![]));
        // Too little room even for the lines between them.
        assert_eq!(widths(&[1, 1, 1, 1], &[1, 1, 1, 1], 5), None);
    }

    #[test]
    fn the_maintainers_table_wraps_within_its_columns_every_word_kept_and_nothing_cut() {
        let table = table(MAINTAINERS);
        for room in [34, 40, 48, 60] {
            let laid = lay_out(&table, room);
            assert_eq!(laid.header, 2, "{room}");
            let rows = text(&laid);
            assert!(
                rows.iter().all(|row| !row.contains('…')),
                "{room}: {rows:#?}"
            );
            assert!(laid.rows.iter().all(|row| row.len() <= room), "{room}");
            // Every row's lines are where the header's are, so the columns align.
            let lines = lines_at(&laid.rows[0]);
            assert_eq!(lines.len(), 2);
            assert!(
                laid.rows
                    .iter()
                    .filter(|row| !rule(row))
                    .all(|row| lines_at(row) == lines),
                "{room}: {rows:#?}"
            );
            // Reading each column down a row's lines gives every word of its cell, in order.
            let mut at = laid.header;
            for cells in &table.rows {
                let height = (at..laid.rows.len())
                    .take_while(|&row| {
                        row == at
                            || laid.rows[row].first().map(|(_, style)| style.background)
                                == laid.rows[at].first().map(|(_, style)| style.background)
                    })
                    .count();
                for (column, cell) in cells.iter().enumerate() {
                    let words = crate::markdown_lite::plain(cell)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let read = column_words(&laid, at, 0..height, column);
                    // A word broken across rows reads as two; joined, it is whole.
                    assert_eq!(read.replace(' ', ""), words.replace(' ', ""), "{room}");
                }
                at += height;
            }
            assert_eq!(at, laid.rows.len());
        }
        // At a slot's 34 cells, the long cells wrap onto rows of their own, and the bold text in a cell stays bold.
        let laid = lay_out(&table, 34);
        let rows = text(&laid);
        assert_eq!(rows[0], "PR  ▏What         ▏Recommendation ");
        assert_eq!(rows[2], "233 ▏Environments ▏Your call:     ");
        assert!(rows.len() > 2 + table.rows.len());
        let bold = laid.rows[2].iter().filter(|(_, style)| style.bold).count();
        assert_eq!(bold, "Your call:".len());
        // At 60 cells, the widest a table makes a floating panel, each row takes at most three.
        let rows = text(&lay_out(&table, 60));
        assert!(rows.len() <= 2 + 3 * table.rows.len(), "{rows:#?}");
    }

    #[test]
    fn a_word_wider_than_its_column_is_broken_never_cut_and_code_keeps_its_tint() {
        let laid = lay_out(
            &table("| A | B |\n|---|---|\n| `incomprehensibilities` | x |\n"),
            NARROWEST + GAP + 1,
        );
        let rows = text(&laid);
        assert_eq!(rows[2], "incomprehe ▏x");
        assert_eq!(rows[3], "nsibilitie ▏ ");
        assert_eq!(rows[4], "s          ▏ ");
        assert!(rows.iter().all(|row| !row.contains('…')));
        assert!(
            laid.rows[2][..NARROWEST]
                .iter()
                .all(|(_, style)| style.background.is_some_and(|tint| tint != STRIPE))
        );
    }

    #[test]
    fn wide_characters_and_emoji_keep_the_columns_aligned() {
        let table = table(
            "| 項目 | 状態 | Note |\n|---|:---:|---|\n| 表示する | 👍🏽 | done |\n| a | 完了しました | 日本語のテキストを折り返す |\n",
        );
        for room in [40, 3 * NARROWEST + 2 * GAP] {
            let laid = lay_out(&table, room);
            let rows = text(&laid);
            let lines = lines_at(&laid.rows[0]);
            assert!(
                laid.rows
                    .iter()
                    .filter(|row| !rule(row))
                    .all(|row| lines_at(row) == lines),
                "{rows:#?}"
            );
            assert!(laid.rows.iter().all(|row| crate::ui_text::whole(row)));
            assert!(laid.rows.iter().all(|row| row.len() <= room));
            // A CJK character takes two cells, and wraps between its characters within its column.
            assert_eq!(shown(&laid.rows[2][..lines[0]]), "表示する ");
            assert_eq!(laid.rows[2][1].0, WIDE);
            assert!(
                rows.iter().any(|row| row.contains("完了しまし")),
                "{rows:#?}"
            );
            // The emoji with its skin tone is one grapheme, drawn `?` in one cell, centred in its column.
            let column = &laid.rows[2][lines[0] + 1..lines[1] - 1];
            let at = column.iter().position(|(ch, _)| *ch == '?').unwrap();
            assert!(at.abs_diff(column.len() - 1 - at) <= 1, "{rows:#?}");
        }
    }

    #[test]
    fn a_table_too_wide_for_its_columns_is_a_card_for_each_row() {
        let table = table(
            "| Name | Owner | State | Next step | Due |\n|---|---|---|---|---|\n| Panel tables | Ada | **open** | Review the layout | Friday |\n| Scrolling | | merged | | |\n",
        );
        let laid = lay_out(&table, 28);
        assert_eq!(laid.header, 0);
        assert_eq!(
            text(&laid),
            [
                "Name: Panel tables",
                "Owner: Ada",
                "State: open",
                "Next step: Review the layout",
                "Due: Friday",
                "─".repeat(28).as_str(),
                "Name: Scrolling",
                "State: merged",
            ]
        );
        assert!(laid.rows[0][0].1.bold && laid.rows[0][0].1.color == DIM);
        assert!(!laid.rows[0][6].1.bold);
        assert!(laid.rows[2][7].1.bold, "the cell's own bold");
        // A long value hangs under its label.
        let laid = lay_out(&table, 16);
        let rows = text(&laid);
        assert_eq!(rows[4], "Next step:");
        assert_eq!(rows[5], "  Review the");
        // Wide enough for every column at the narrowest, or its widest cell where that is narrower, as Owner's, State's
        // and Due's are, it is a grid again.
        let least = NARROWEST + 5 + 6 + NARROWEST + 6 + 4 * GAP;
        assert_eq!(lay_out(&table, least).header, 2);
        assert_eq!(lay_out(&table, least - 1).header, 0);
        // A header alone shows its labels.
        let alone = lay_out(
            &crate::markdown_lite::Table {
                rows: Vec::new(),
                ..table.clone()
            },
            28,
        );
        assert_eq!(text(&alone)[0], "Name · Owner · State · Next");
        // A table without a header has cards of values alone, and a grid without its header and rule.
        let bare = Table {
            header: Vec::new(),
            ..table.clone()
        };
        assert_eq!(text(&lay_out(&bare, 28))[0], "Panel tables");
        let grid = lay_out(&bare, 80);
        assert_eq!(grid.header, 0);
        assert!(text(&grid)[0].starts_with("Panel tables ▏Ada"));
        assert_eq!(lay_out(&Table::default(), 28), Laid::default());
    }

    #[test]
    fn rows_shorter_than_the_table_and_columns_without_alignment_or_cells_are_laid_out_empty() {
        let cell = |text: &str| vec![(text.to_owned(), Inline::Plain)];
        let table = Table {
            align: vec![Align::Right],
            header: Vec::new(),
            rows: vec![vec![cell("a")], vec![cell("b"), cell("cc")]],
        };
        assert_eq!(text(&lay_out(&table, 20)), ["a ▏  ", "b ▏cc"]);
        let wider = Table {
            align: vec![Align::Left; 3],
            ..table
        };
        assert_eq!(text(&lay_out(&wider, 20)), ["a ▏   ▏", "b ▏cc ▏"]);
    }
}
