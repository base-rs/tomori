// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Styled text laid out on a grid of monospaced cells, for the local-space panels' [`Canvas`](crate::canvas::Canvas), free of rendering code.
//!
//! Text is given as [`Span`]s, each with a [`Style`]: a colour, whether it is bold, and an optional background. [`wrap`] lays spans out on rows of a given width, at words, breaking a word longer than a row, with a hanging indent for the rows after the first and, for code, a continuation mark where a line is broken rather than cut; [`fit`] keeps at most a number of rows, ending the last with "…" where text is left out, and [`cut`] keeps one row, ending with "…" where it is cut. [`lines`] and [`clip`] do the same for plain text. These are the one rule for every panel, label and menu row.
//!
//! Every character drawn is one cell; [`glyph`] finds its bitmap in the `font8x8` tables, and [`shown`] the character that is drawn for it: itself, a close equivalent the tables have, such as the base letter of an accented one, or `?` for one with neither, so that the console shows what the headset draws. Control characters other than line breaks and tabs, combining marks, variation selectors and zero-width spaces and joiners take no cell.

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
        .flat_map(|span| {
            span.text
                .chars()
                .filter(|ch| !invisible(*ch))
                .map(move |ch| (ch, span.style))
        })
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
        .split(|(ch, _)| breaks(*ch))
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

/// Whether a line may break at `ch`: at a space, but not a non-breaking one, which joins the words either side of it and is drawn as a space.
fn breaks(ch: char) -> bool {
    ch.is_whitespace() && !matches!(ch, '\u{a0}' | '\u{2007}' | '\u{202f}')
}

/// At most `most` of `rows`, the last ending with "…" where any are left out: after its text if it is shorter than `width`, or in place of its last cell.
pub fn fit(rows: &mut Vec<Vec<Cell>>, most: usize, width: usize) {
    if rows.len() <= most {
        return;
    }
    rows.truncate(most);
    if let Some(last) = rows.last_mut() {
        let style = last
            .last()
            .map_or(Style::plain([1.0; 4]), |(_, style)| *style);
        last.truncate(width.saturating_sub(1));
        last.push(('…', style));
    }
}

/// Plain `text` wrapped at words to rows of at most `width` cells, as [`wrap`] lays it out, on at most `rows` rows, as [`fit`] keeps them. As with [`str::lines`], empty text has no rows and a final line break starts none.
pub fn lines(text: &str, width: usize, rows: usize) -> Vec<String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    let mut wrapped = wrap(
        &[Span::new(text, Style::plain([1.0; 4]))],
        width,
        Wrap::default(),
    );
    fit(&mut wrapped, rows, width);
    wrapped
        .iter()
        .map(|row| row.iter().map(|(ch, _)| *ch).collect())
        .collect()
}

/// Plain `text` on one row of at most `width` cells, with its runs of spaces and line breaks made one space, ending with "…" where cut, as [`cut`] does.
pub fn clip(text: &str, width: usize) -> String {
    let line = text
        .split(breaks)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    cut(&[Span::new(line, Style::plain([1.0; 4]))], width)
        .into_iter()
        .map(|(ch, _)| ch)
        .collect()
}

/// The characters drawn for one line of `text`, a cell each: line breaks and tabs as spaces, each other character as [`shown`] draws it, leaving out those that take no cell.
pub fn drawn(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().filter(|ch| !invisible(*ch)).map(|ch| {
        if ch == '\n' || ch == '\t' {
            ' '
        } else {
            shown(ch)
        }
    })
}

/// Whether `ch` takes no cell: a control character other than a line break or a tab, such as the carriage return of CRLF text; a combining mark or variation selector, which until text is shaped into graphemes is left out rather than drawn as `?` beside its character; or a zero-width space, joiner or soft hyphen.
fn invisible(ch: char) -> bool {
    (ch.is_control() && ch != '\n' && ch != '\t')
        || matches!(
            ch,
            '\u{ad}'
                | '\u{300}'..='\u{36f}'
                | '\u{1ab0}'..='\u{1aff}'
                | '\u{1dc0}'..='\u{1dff}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2060}'..='\u{2064}'
                | '\u{20d0}'..='\u{20ff}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{fe20}'..='\u{fe2f}'
                | '\u{feff}'
                | '\u{e0100}'..='\u{e01ef}'
        )
}

/// `spans` on one row of at most `width` cells, ending with "…" where cut.
pub fn cut(spans: &[Span], width: usize) -> Vec<Cell> {
    let mut cells: Vec<Cell> = spans
        .iter()
        .flat_map(|span| drawn(&span.text).map(move |ch| (ch, span.style)))
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

/// The character drawn for `ch`: itself where a table has it; a close equivalent for typographic quotes, dashes, ellipses and spaces, and the base letter of an accented Latin letter the tables lack; and `?` otherwise, such as for Cyrillic, Japanese (#103) or emoji, which need a glyph atlas.
pub fn shown(ch: char) -> char {
    match ch {
        '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' | '\u{2032}' => '\'',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => '"',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        '\u{2039}' => '<',
        '\u{203a}' => '>',
        '\u{2044}' => '/',
        '\u{22ef}' => '…',
        '\u{00a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => ' ',
        ch if lookup(ch).is_some() => ch,
        ch => base_letter(ch).unwrap_or('?'),
    }
}

/// The base letters of Latin Extended-A, U+0100 to U+017F, in order, `?` for a ligature.
const EXTENDED_A: &str = "AaAaAaCcCcCcCcDdDdEeEeEeEeEeGgGgGgGgHhHhIiIiIiIiIi??JjKkkLlLlLlLlLlNnNnNnnNnOoOoOo??RrRrRrSsSsSsSsTtTtTtUuUuUuUuUuUuWwYyYZzZzZzs";
/// The base letters of Pinyin's tone marks, U+01CD to U+01DC.
const PINYIN: &str = "AaIiOoUuUuUuUuUu";
/// The base letters of Vietnamese's, U+1EA0 to U+1EF9.
const VIETNAMESE: &str =
    "AaAaAaAaAaAaAaAaAaAaAaAaEeEeEeEeEeEeEeEeIiIiOoOoOoOoOoOoOoOoOoOoOoOoUuUuUuUuUuUuUuYyYyYyYy";

/// The ASCII letter `ch` is accented from, for the Latin letters the tables lack: those of Latin Extended-A, such as Polish, Czech and Turkish use, Romanian's commas below, and Pinyin's and Vietnamese's tone marks.
fn base_letter(ch: char) -> Option<char> {
    let from = |table: &str, first: u32| {
        table
            .chars()
            .nth((ch as u32 - first) as usize)
            .filter(|base| *base != '?')
    };
    match ch {
        '\u{100}'..='\u{17f}' => from(EXTENDED_A, 0x100),
        '\u{1cd}'..='\u{1dc}' => from(PINYIN, 0x1cd),
        '\u{1ea0}'..='\u{1ef9}' => from(VIETNAMESE, 0x1ea0),
        '\u{180}' => Some('b'),
        '\u{1a0}' => Some('O'),
        '\u{1a1}' => Some('o'),
        '\u{1af}' => Some('U'),
        '\u{1b0}' => Some('u'),
        '\u{218}' => Some('S'),
        '\u{219}' => Some('s'),
        '\u{21a}' => Some('T'),
        '\u{21b}' => Some('t'),
        _ => None,
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
        assert_eq!(shown('\u{a0}'), ' ');
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
    fn plain_lines_wrap_at_words_break_long_words_and_end_with_an_ellipsis_where_cut() {
        // Empty text has no rows, and a final line break starts none, as with `str::lines`.
        assert!(lines("", 10, 3).is_empty());
        assert!(lines("\n", 10, 3).is_empty());
        assert_eq!(lines("a\n", 10, 3), ["a"]);
        assert_eq!(lines("a\n\nb", 10, 3), ["a", "", "b"]);
        assert_eq!(lines("   ", 10, 3), [""]);
        // Exact fits stay on one row; one more cell wraps.
        assert_eq!(lines("abcd efgh", 9, 3), ["abcd efgh"]);
        assert_eq!(lines("abcd efghi", 9, 3), ["abcd", "efghi"]);
        assert_eq!(lines("abcdefghi", 9, 1), ["abcdefghi"]);
        // A word longer than a row is broken, never dropped.
        assert_eq!(lines("ab cdefghijkl", 4, 9), ["ab", "cdef", "ghij", "kl"]);
        // Runs of spaces are one.
        assert_eq!(lines("a   b", 10, 3), ["a b"]);
        // Rows past the most kept are left out, the last ending with "…": after its text where there is room, or in place of its last cell.
        assert_eq!(lines("one two three four", 9, 1), ["one two…"]);
        assert_eq!(lines("abcdefghijkl", 4, 2), ["abcd", "efg…"]);
        assert!(lines("one two", 9, 0).is_empty());
        for text in ["", "x", &"word ".repeat(50), &"x".repeat(100), "a\n\n\n\nb"] {
            for width in [2, 3, 7, 40] {
                let rows = lines(text, width, 3);
                assert!(rows.len() <= 3);
                assert!(
                    rows.iter().all(|row| row.chars().count() <= width),
                    "{rows:?}"
                );
            }
        }
    }

    #[test]
    fn a_plain_row_is_cut_at_its_width() {
        assert_eq!(clip("", 5), "");
        assert_eq!(clip("exact", 5), "exact");
        assert_eq!(clip("exactly", 5), "exac…");
        assert_eq!(clip("two\nlines  and\tspaces", 40), "two lines and spaces");
        assert_eq!(clip("anything", 1), "…");
        assert_eq!(clip("anything", 0), "");
        // A row is counted in cells, so a cut never splits a character.
        assert_eq!(clip(&"é".repeat(10), 4), "ééé…");
    }

    #[test]
    fn fitting_rows_keeps_the_style_of_the_text_cut() {
        let red = Style::plain([1.0, 0.0, 0.0, 1.0]);
        let mut rows = wrap(&[Span::new("one two three", red)], 4, Wrap::default());
        fit(&mut rows, 2, 4);
        assert_eq!(text(&rows), ["one", "two…"]);
        assert_eq!(rows[1][3], ('…', red));
        // Fewer rows than the most are kept as they are.
        let mut rows = wrap(&[Span::new("one", red)], 4, Wrap::default());
        fit(&mut rows, 2, 4);
        assert_eq!(text(&rows), ["one"]);
        // An empty row that is cut is only the mark.
        let mut rows = vec![Vec::new(), Vec::new()];
        fit(&mut rows, 1, 4);
        assert_eq!(rows, [vec![('…', WHITE)]]);
    }

    #[test]
    fn accented_letters_quotes_dashes_and_ellipses_draw_as_their_nearest() {
        // Latin-1 letters are in the tables, and drawn as themselves.
        for ch in ['é', 'ñ', 'ü', 'ø', 'ß', 'Å', '«', '»'] {
            assert_eq!(shown(ch), ch, "{ch}");
        }
        // Others are drawn as their base letter.
        let drawn = |text: &str| drawn(text).collect::<String>();
        assert_eq!(
            drawn("Łódź Dvořák Ştefan Ţară Erdoğan Đorđe"),
            "Lódz Dvorák Stefan Tara Erdogan Dorde"
        );
        assert_eq!(drawn("Nguyễn Thị Hương"), "Nguyen Thi Huong");
        assert_eq!(drawn("Lǚ ƀ Ơ ơ Ư ư Ș ș Ț ț ſ"), "Lu b O o U u S s T t s");
        // A ligature has no base letter.
        assert_eq!(drawn("Œuvre ĳ"), "?uvre ?");
        // Typographic punctuation is drawn as its ASCII equivalent, and an ellipsis as itself.
        assert_eq!(
            drawn("„quoted‟ ‚single‛ 5′ 6″ ‹a› ½⁄2"),
            "\"quoted\" 'single' 5' 6\" <a> ½/2"
        );
        assert_eq!(drawn("a‐b‑c‒d–e—f―g−h"), "a-b-c-d-e-f-g-h");
        assert_eq!(drawn("wait… ⋯"), "wait… …");
        assert_eq!(drawn("a\u{2009}b\u{3000}c"), "a b c");
        // What needs a glyph atlas, such as Cyrillic, Japanese (#103) and emoji, is still `?`.
        assert_eq!(drawn("Жこ🎤"), "???");
    }

    #[test]
    fn characters_that_take_no_cell_are_left_out() {
        let drawn = |text: &str| drawn(text).collect::<String>();
        // A carriage return, an escape, a combining mark, a variation selector, zero-width joiners and spaces, a soft hyphen and a byte order mark.
        assert_eq!(drawn("a\r\nb"), "a b");
        assert_eq!(drawn("\u{1b}[31mred"), "[31mred");
        assert_eq!(drawn("e\u{301}"), "e");
        assert_eq!(drawn("✓\u{fe0f}"), "✓");
        assert_eq!(drawn("葛\u{e0100}"), "?");
        assert_eq!(drawn("a\u{200b}b\u{200d}c\u{ad}d\u{feff}"), "abcd");
        assert_eq!(cut(&[Span::new("e\u{301}\u{301}x", WHITE)], 2).len(), 2);
        let rows = wrap(&[Span::new("ab\u{301}c\r\nd", WHITE)], 3, Wrap::default());
        assert_eq!(text(&rows), ["abc", "d"]);
    }

    #[test]
    fn a_non_breaking_space_joins_words() {
        let rows = wrap(&[Span::new("aa 10\u{a0}km bb", WHITE)], 6, Wrap::default());
        assert_eq!(text(&rows), ["aa", "10 km", "bb"]);
        assert_eq!(clip("10\u{202f}km", 10), "10 km");
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
