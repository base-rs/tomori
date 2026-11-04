// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Styled text laid out on a grid of monospaced cells, for the local-space panels in `ui_panel`, free of rendering code.
//!
//! Text is given as [`Span`]s, each with a [`Style`]: a colour, whether it is bold, and an optional background. [`wrap`] lays spans out on rows of a given width, at words, with a hanging indent for the rows after the first and, for code, a continuation mark where a line is broken rather than cut. Every character drawn is one cell; [`glyph`] finds its bitmap in the `font8x8` tables, and [`shown`] the character that is drawn for it, `?` for one outside them, so that the console shows what the headset draws.

use font8x8::{
    BASIC_FONTS, BLOCK_FONTS, BOX_FONTS, GREEK_FONTS, LATIN_FONTS, MISC_FONTS, UnicodeFonts,
};

/// A colour with alpha, in linear 0 to 1.
pub type Rgba = [f32; 4];

/// The cell a glyph is drawn in, in font pixels: 8 × 8 glyphs, a column apart and with room between rows.
pub const CELL_WIDTH: usize = 9;
pub const CELL_HEIGHT: usize = 12;

/// How a span looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Rgba,
    /// Drawn twice, a font pixel apart, since the font has one weight.
    pub bold: bool,
    pub background: Option<Rgba>,
}

impl Style {
    pub const fn plain(color: Rgba) -> Self {
        Self {
            color,
            bold: false,
            background: None,
        }
    }

    pub const fn bold(color: Rgba) -> Self {
        Self {
            color,
            bold: true,
            background: None,
        }
    }

    pub const fn on(self, background: Rgba) -> Self {
        Self {
            background: Some(background),
            ..self
        }
    }
}

/// Text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// One character in a cell, with its style.
pub type Cell = (char, Style);

/// One row of a panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Behind the whole row.
    pub background: Option<Rgba>,
    /// A bar down the row's left edge, such as a speaker's colour, drawn in the row's first column, which text leaves empty.
    pub bar: Option<Rgba>,
    /// A thin bar along the row's bottom, filled to the fraction given, such as the time left to undo.
    pub meter: Option<(f32, Rgba)>,
}

impl Row {
    pub fn new(cells: Vec<Cell>) -> Self {
        Self {
            cells,
            ..Default::default()
        }
    }

    /// A row of `text` in `style`, cut to `width` with "…" where it is longer.
    pub fn text(text: &str, style: Style, width: usize) -> Self {
        Self::new(cut(&[Span::new(text, style)], width))
    }

    /// The characters shown, as the console prints them.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn plain(&self) -> String {
        self.cells.iter().map(|(ch, _)| *ch).collect()
    }

    pub fn with_bar(self, bar: Option<Rgba>) -> Self {
        Self { bar, ..self }
    }

    pub fn with_background(self, background: Option<Rgba>) -> Self {
        Self { background, ..self }
    }
}

/// How [`wrap`] breaks lines.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wrap {
    /// Cells of space before the first row's text.
    pub indent: usize,
    /// Cells of space before every later row's text, such as to hang a list item's text under itself.
    pub hanging: usize,
    /// Keep spaces and break anywhere, marking each break with this character at the end of the row, as for code, rather than breaking at words.
    pub continuation: Option<char>,
}

/// `spans` laid out on rows of at most `width` cells. A newline starts a new row. Words longer than a row are broken across rows.
pub fn wrap(spans: &[Span], width: usize, options: Wrap) -> Vec<Vec<Cell>> {
    let width = width.max(options.indent.max(options.hanging) + 2);
    let cells: Vec<Cell> = spans
        .iter()
        .flat_map(|span| span.text.chars().map(move |ch| (ch, span.style)))
        .collect();
    let mut rows = Vec::new();
    for line in cells.split(|(ch, _)| *ch == '\n') {
        match options.continuation {
            Some(mark) => wrap_code(line, width, options, mark, &mut rows),
            None => wrap_words(line, width, options, &mut rows),
        }
    }
    rows
}

fn blank(count: usize, style: Style) -> impl Iterator<Item = Cell> {
    std::iter::repeat_n(
        (
            ' ',
            Style {
                background: None,
                ..style
            },
        ),
        count,
    )
}

fn wrap_code(line: &[Cell], width: usize, options: Wrap, mark: char, rows: &mut Vec<Vec<Cell>>) {
    let line: Vec<Cell> = line
        .iter()
        .map(|&(ch, style)| (if ch == '\t' { ' ' } else { shown(ch) }, style))
        .collect();
    let mut rest = line.as_slice();
    let mut first = true;
    loop {
        let indent = if first {
            options.indent
        } else {
            options.hanging
        };
        let room = width - indent;
        let style = rest
            .first()
            .map_or(Style::plain([1.0; 4]), |(_, style)| *style);
        let mut row: Vec<Cell> = blank(indent, style).collect();
        if rest.len() <= room {
            row.extend_from_slice(rest);
            rows.push(row);
            return;
        }
        row.extend_from_slice(&rest[..room - 1]);
        row.push((mark, rest[room - 1].1));
        rows.push(row);
        rest = &rest[room - 1..];
        first = false;
    }
}

fn wrap_words(line: &[Cell], width: usize, options: Wrap, rows: &mut Vec<Vec<Cell>>) {
    let style = line
        .first()
        .map_or(Style::plain([1.0; 4]), |(_, style)| *style);
    let mut row: Vec<Cell> = blank(options.indent, style).collect();
    let mut indent = options.indent;
    let mut words = line
        .split(|(ch, _)| ch.is_whitespace())
        .filter(|word| !word.is_empty())
        .peekable();
    if words.peek().is_none() {
        rows.push(Vec::new());
        return;
    }
    for word in words {
        let word: Vec<Cell> = word.iter().map(|&(ch, style)| (shown(ch), style)).collect();
        let used = row.len() - indent;
        if used > 0 && row.len() + 1 + word.len() > width {
            rows.push(std::mem::take(&mut row));
            indent = options.hanging;
            row.extend(blank(indent, style));
        }
        if row.len() > indent {
            // The space takes the background of the word before it, so a run of code stays one tinted block.
            let before = row.last().map_or(style, |(_, style)| *style);
            let after = word[0].1;
            let space = if before.background == after.background {
                before
            } else {
                Style {
                    background: None,
                    ..before
                }
            };
            row.push((' ', space));
        }
        for cell in word {
            if row.len() == width {
                rows.push(std::mem::take(&mut row));
                indent = options.hanging;
                row.extend(blank(indent, style));
            }
            row.push(cell);
        }
    }
    rows.push(row);
}

/// `spans` on one row of at most `width` cells, ending with "…" where cut.
pub fn cut(spans: &[Span], width: usize) -> Vec<Cell> {
    let mut cells: Vec<Cell> = spans
        .iter()
        .flat_map(|span| {
            span.text.chars().map(move |ch| {
                let ch = if ch == '\n' || ch == '\t' { ' ' } else { ch };
                (shown(ch), span.style)
            })
        })
        .collect();
    if cells.len() > width {
        cells.truncate(width.saturating_sub(1));
        let style = cells
            .last()
            .map_or(Style::plain([1.0; 4]), |(_, style)| *style);
        if width > 0 {
            cells.push(('…', style));
        }
    }
    cells
}

/// Glyphs the `font8x8` tables lack, as 8 rows of 8 pixels, the lowest bit leftmost.
const EXTRA: [(char, [u8; 8]); 14] = [
    ('•', [0x00, 0x00, 0x18, 0x3C, 0x3C, 0x18, 0x00, 0x00]),
    ('✓', [0x00, 0xC0, 0x60, 0x31, 0x1B, 0x0E, 0x04, 0x00]),
    ('●', [0x00, 0x3C, 0x7E, 0x7E, 0x7E, 0x7E, 0x3C, 0x00]),
    ('○', [0x00, 0x3C, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00]),
    ('◐', [0x00, 0x3C, 0x4E, 0x4E, 0x4E, 0x4E, 0x3C, 0x00]),
    ('↵', [0x00, 0x40, 0x40, 0x40, 0x42, 0x7F, 0x02, 0x00]),
    ('→', [0x00, 0x10, 0x20, 0x7F, 0x20, 0x10, 0x00, 0x00]),
    ('←', [0x00, 0x04, 0x02, 0x7F, 0x02, 0x04, 0x00, 0x00]),
    ('↑', [0x08, 0x1C, 0x2A, 0x08, 0x08, 0x08, 0x08, 0x00]),
    ('↓', [0x08, 0x08, 0x08, 0x08, 0x2A, 0x1C, 0x08, 0x00]),
    ('▾', [0x00, 0x00, 0x7F, 0x3E, 0x1C, 0x08, 0x00, 0x00]),
    ('…', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49, 0x00]),
    ('✕', [0x00, 0x63, 0x36, 0x1C, 0x1C, 0x36, 0x63, 0x00]),
    ('▸', [0x00, 0x04, 0x0C, 0x1C, 0x0C, 0x04, 0x00, 0x00]),
];

/// The character drawn for `ch`: itself where a table has it, a close equivalent for typographic quotes and dashes, and `?` otherwise.
pub fn shown(ch: char) -> char {
    match ch {
        '\u{2018}' | '\u{2019}' => '\'',
        '\u{201c}' | '\u{201d}' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
        '\u{00a0}' => ' ',
        ch if lookup(ch).is_some() => ch,
        _ => '?',
    }
}

fn lookup(ch: char) -> Option<[u8; 8]> {
    if ch.is_ascii() {
        return BASIC_FONTS.get(ch);
    }
    LATIN_FONTS
        .get(ch)
        .or_else(|| BOX_FONTS.get(ch))
        .or_else(|| BLOCK_FONTS.get(ch))
        .or_else(|| GREEK_FONTS.get(ch))
        .or_else(|| MISC_FONTS.get(ch))
        .or_else(|| {
            EXTRA
                .iter()
                .find(|(extra, _)| *extra == ch)
                .map(|(_, bits)| *bits)
        })
}

/// The bitmap drawn for `ch`, as [`shown`] chooses it.
pub fn glyph(ch: char) -> [u8; 8] {
    lookup(shown(ch)).unwrap_or([0; 8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typographic_characters_tabs_and_line_breaks_are_drawn_plainly() {
        assert_eq!(shown('\u{201c}'), '"');
        assert_eq!(shown('\u{2014}'), '-');
        let style = Style::plain([1.0; 4]);
        let cells = cut(&[Span::new("a\tb\nc", style)], 10);
        assert_eq!(cells.iter().map(|(ch, _)| *ch).collect::<String>(), "a b c");
        assert!(cut(&[Span::new("abc", style)], 0).is_empty());
        let code = wrap(
            &[Span::new("\tx", style)],
            10,
            Wrap {
                continuation: Some('↵'),
                ..Default::default()
            },
        );
        assert_eq!(code[0][0].0, ' ');
    }

    #[test]
    fn a_space_between_code_and_text_takes_no_background() {
        let code = Style {
            background: Some([0.1; 4]),
            ..Style::plain([1.0; 4])
        };
        let plain = Style::plain([1.0; 4]);
        let rows = wrap(
            &[Span::new("code", code), Span::new(" text", plain)],
            20,
            Wrap::default(),
        );
        assert_eq!(
            rows[0][4],
            (
                ' ',
                Style {
                    background: None,
                    ..code
                }
            )
        );
    }

    const WHITE: Style = Style::plain([1.0; 4]);

    fn text(rows: &[Vec<Cell>]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|(ch, _)| *ch).collect())
            .collect()
    }

    #[test]
    fn words_wrap_within_the_width_with_a_hanging_indent() {
        let rows = wrap(
            &[Span::new("• one two three four five six", WHITE)],
            12,
            Wrap {
                hanging: 2,
                ..Default::default()
            },
        );
        assert_eq!(text(&rows), ["• one two", "  three four", "  five six"]);
        assert!(rows.iter().all(|row| row.len() <= 12));
        // A word longer than a row is broken across rows, and a blank line kept.
        let rows = wrap(&[Span::new("abcdefghij\n\nok", WHITE)], 4, Wrap::default());
        assert_eq!(text(&rows), ["abcd", "efgh", "ij", "", "ok"]);
    }

    #[test]
    fn spans_keep_their_colours_across_rows() {
        let red = Style::plain([1.0, 0.0, 0.0, 1.0]);
        let rows = wrap(
            &[Span::new("plain ", WHITE), Span::new("red words here", red)],
            10,
            Wrap::default(),
        );
        assert_eq!(text(&rows), ["plain red", "words here"]);
        assert_eq!(rows[0][0].1, WHITE);
        assert_eq!(rows[0][6].1, red);
        assert!(rows[1].iter().all(|(_, style)| *style == red));
    }

    #[test]
    fn code_keeps_its_spaces_and_marks_where_it_is_broken() {
        let code = WHITE.on([0.2, 0.2, 0.2, 1.0]);
        let rows = wrap(
            &[Span::new("let  x = 1234567890;", code)],
            10,
            Wrap {
                continuation: Some('↵'),
                ..Default::default()
            },
        );
        assert_eq!(text(&rows), ["let  x = ↵", "123456789↵", "0;"]);
        assert!(rows.iter().all(|row| row.len() <= 10));
    }

    #[test]
    fn glyphs_come_from_the_wider_tables_and_fall_back_to_a_question_mark() {
        for ch in [
            'a', 'é', '│', '█', 'λ', '•', '✓', '↵', '→', '…', '▸', '○', '◐', '●', '✕', '▾',
        ] {
            assert_eq!(shown(ch), ch, "{ch}");
            assert_ne!(glyph(ch), [0; 8], "{ch}");
        }
        assert_eq!(shown('🎤'), '?');
        assert_eq!(shown('’'), '\'');
        assert_eq!(shown('—'), '-');
        assert_eq!(glyph('🎤'), glyph('?'));
    }

    #[test]
    fn a_cut_row_ends_with_an_ellipsis() {
        assert_eq!(
            Row::text("Fix the parser in the lexer", WHITE, 10).plain(),
            "Fix the p…"
        );
        assert_eq!(Row::text("short", WHITE, 10).plain(), "short");
    }
}
