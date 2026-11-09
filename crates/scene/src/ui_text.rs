// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Styled text laid out on a grid of monospaced cells, for the local-space panels' [`Canvas`](crate::canvas::Canvas), free of rendering code.
//!
//! Text is given as [`Span`]s, each with a [`Style`]: a colour, whether it is bold, and an optional background. [`wrap`] lays spans out on rows of a given width, at words, breaking a word longer than a row, with a hanging indent for the rows after the first and, for code, a continuation mark where a line is broken rather than cut; [`fit`] keeps at most a number of rows, ending the last with "…" where text is left out, and [`cut`] keeps one row, ending with "…" where it is cut. [`lines`] and [`clip`] do the same for plain text. These are the one rule for every panel, label and menu row.
//!
//! Each grapheme drawn takes a cell, or two for a wide one, such as a CJK ideograph or a full-width form, whose second cell holds [`WIDE`], so that a row's cells are its columns. A grapheme is drawn as the one character its letter and marks compose to, leaving out marks that do not compose; [`shown`] is the character drawn for it, which is itself where a bundled font has it, its base letter where only that is, or `?`, so that the console shows what the headset draws. Every kind of line break, such as CRLF, a lone carriage return or a form feed, is one, as a line feed is. Other control characters, variation selectors and zero-width spaces and joiners take no cell. Lines break at spaces and, between CJK characters, wherever Unicode's line breaking rules (UAX #14) allow, so never before closing punctuation such as "。".
//!
//! [`glyph`] places a cell's glyph from `tomori-text`'s atlas, for the renderers, which draw it from the atlas's signed distance fields.
use crate::shapes::Mark;
use tomori_text::atlas;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

/// A colour with alpha, in linear 0 to 1.
pub type Rgba = [f32; 4];

/// The cell a glyph is drawn in, in font pixels: a glyph's cap height is about 8, with room between rows.
pub const CELL_WIDTH: usize = 9;
pub const CELL_HEIGHT: usize = 12;

/// How a span looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Rgba,
    /// Drawn with its edges moved outwards, since the font has one weight.
    pub bold: bool,
    pub background: Option<Rgba>,
    /// A shape drawn in place of each of its cells whose character stands in for it, rather than a glyph (see
    /// [`shapes`](crate::shapes)).
    pub mark: Option<Mark>,
}

impl Style {
    pub const fn plain(color: Rgba) -> Self {
        Self {
            color,
            bold: false,
            background: None,
            mark: None,
        }
    }

    pub const fn bold(color: Rgba) -> Self {
        Self {
            color,
            bold: true,
            background: None,
            mark: None,
        }
    }

    /// Drawn as `mark` where a cell holds the character that stands in for it.
    pub const fn marked(self, mark: Mark) -> Self {
        Self {
            mark: Some(mark),
            ..self
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

/// The second cell of a wide character, which draws nothing. It is a noncharacter, which text never draws, as [`shown`] makes it `?`.
pub const WIDE: char = '\u{fdd0}';

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

    /// The characters shown, as the console prints them, where a wide one takes two columns.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn plain(&self) -> String {
        plain(&self.cells)
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

const WHITE: Style = Style::plain([1.0; 4]);

/// `spans` laid out on rows of at most `width` cells. A line break of any kind starts a new row. Words longer than a row are broken across rows.
pub fn wrap(spans: &[Span], width: usize, options: Wrap) -> Vec<Vec<Cell>> {
    let width = width.max(options.indent.max(options.hanging) + 2);
    let graphemes = graphemes(spans.iter().map(|span| (span.text.as_str(), span.style)));
    let mut rows = Vec::new();
    for line in graphemes.split(|(ch, _)| *ch == '\n') {
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
                mark: None,
                ..style
            },
        ),
        count,
    )
}

fn wrap_code(line: &[Cell], width: usize, options: Wrap, mark: char, rows: &mut Vec<Vec<Cell>>) {
    let line: Vec<Cell> = line
        .iter()
        .map(|&(ch, style)| (if ch == '\t' { ' ' } else { ch }, style))
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
        let style = rest.first().map_or(WHITE, |(_, style)| *style);
        let mut row: Vec<Cell> = blank(indent, style).collect();
        if span(rest) <= room {
            extend(&mut row, rest);
            rows.push(row);
            return;
        }
        // As many as fit beside the mark. The rest is wider than the room, so some never fit.
        let mut taken = 0;
        let mut used = 0;
        while used + columns(rest[taken].0) < room {
            used += columns(rest[taken].0);
            taken += 1;
        }
        if taken == 0 {
            // A wide character with no room beside it for the mark fills the row alone.
            extend(&mut row, &rest[..1]);
            rest = &rest[1..];
        } else {
            extend(&mut row, &rest[..taken]);
            row.push((mark, rest[taken].1));
            rest = &rest[taken..];
        }
        rows.push(row);
        first = false;
    }
}

fn wrap_words(line: &[Cell], width: usize, options: Wrap, rows: &mut Vec<Vec<Cell>>) {
    let style = line.first().map_or(WHITE, |(_, style)| *style);
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
        for (index, piece) in pieces(word).into_iter().enumerate() {
            // A word's first piece follows a space; the rest of it follows on directly.
            let spaced = index == 0;
            let used = row.len() - indent;
            if used > 0 && row.len() + usize::from(spaced) + span(piece) > width {
                rows.push(std::mem::take(&mut row));
                indent = options.hanging;
                row.extend(blank(indent, style));
            }
            if spaced && row.len() > indent {
                // The space takes the background of the word before it, so a run of code stays one tinted block.
                let before = row.last().map_or(style, |(_, style)| *style);
                let after = piece[0].1;
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
            for &cell in piece {
                if row.len() + columns(cell.0) > width {
                    rows.push(std::mem::take(&mut row));
                    indent = options.hanging;
                    row.extend(blank(indent, style));
                }
                extend(&mut row, &[cell]);
            }
        }
    }
    rows.push(row);
}

/// `word` in the pieces a line may break between: where UAX #14 allows a break beside a wide character, as between CJK characters, which have no spaces between words, but never before closing punctuation or after opening punctuation. Elsewhere, as after a hyphen, a word is broken only where it is longer than a row.
fn pieces(word: &[Cell]) -> Vec<&[Cell]> {
    if !word.iter().any(|(ch, _)| columns(*ch) > 1) {
        return vec![word];
    }
    let text: String = word.iter().map(|(ch, _)| *ch).collect();
    let offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
    let mut pieces = Vec::new();
    let mut start = 0;
    for (offset, _) in unicode_linebreak::linebreaks(&text) {
        // Never at the start, and the end is not a break within the word.
        let index = offsets.partition_point(|at| *at < offset);
        if index == word.len() {
            continue;
        }
        if columns(word[index - 1].0) > 1 || columns(word[index].0) > 1 {
            pieces.push(&word[start..index]);
            start = index;
        }
    }
    pieces.push(&word[start..]);
    pieces
}

/// Whether a line may break at `ch`: at a space, but not a non-breaking one, which joins the words either side of it and is drawn as a space.
fn breaks(ch: char) -> bool {
    ch.is_whitespace() && !matches!(ch, '\u{a0}' | '\u{2007}' | '\u{202f}')
}

/// The cells `ch` takes as drawn: two for a wide character, and one otherwise.
fn columns(ch: char) -> usize {
    if shown(ch).width() == Some(2) { 2 } else { 1 }
}

/// The cells `cells` take as drawn.
fn span(cells: &[Cell]) -> usize {
    cells.iter().map(|(ch, _)| columns(*ch)).sum()
}

/// Append `cells` to `row` as drawn: each as [`shown`] draws it, a wide one followed by [`WIDE`].
fn extend<T: Copy>(row: &mut Vec<(char, T)>, cells: &[(char, T)]) {
    for &(ch, extra) in cells {
        let ch = shown(ch);
        row.push((ch, extra));
        if columns(ch) > 1 {
            row.push((WIDE, extra));
        }
    }
}

/// `cells` cut to at most `count`, without leaving half a wide character.
fn truncate(cells: &mut Vec<Cell>, count: usize) {
    // The first cell is never the second half of a wide character, so `count` is above 0 when it is.
    let half = cells.get(count).is_some_and(|(ch, _)| *ch == WIDE);
    cells.truncate(count - usize::from(half));
}

/// The characters of `cells`, as the console prints them, without [`WIDE`].
pub fn plain(cells: &[Cell]) -> String {
    cells
        .iter()
        .map(|(ch, _)| *ch)
        .filter(|ch| *ch != WIDE)
        .collect()
}

/// Whether every wide character in `cells` has its second cell after it, and every second cell a wide character before it: that no layout split one, or would draw one shrunk into a single cell. For tests of every surface's rows.
#[cfg(test)]
pub(crate) fn whole<T>(cells: &[(char, T)]) -> bool {
    cells.iter().enumerate().all(|(index, (ch, _))| {
        let next = cells.get(index + 1).map(|(next, _)| *next);
        let before = index.checked_sub(1).map(|before| cells[before].0);
        if *ch == WIDE {
            before.is_some_and(|before| before != WIDE && columns(before) > 1)
        } else {
            (columns(*ch) > 1) == (next == Some(WIDE))
        }
    })
}

/// At most `most` of `rows`, the last ending with "…" where any are left out: after its text if it is shorter than `width`, or in place of its last cell.
pub fn fit(rows: &mut Vec<Vec<Cell>>, most: usize, width: usize) {
    if rows.len() <= most {
        return;
    }
    rows.truncate(most);
    if let Some(last) = rows.last_mut() {
        let style = last.last().map_or(WHITE, |(_, style)| *style);
        truncate(last, width.saturating_sub(1));
        last.push(('…', style));
    }
}

/// Plain `text` wrapped at words to rows of at most `width` cells, as [`wrap`] lays it out, on at most `rows` rows, as [`fit`] keeps them. As with [`str::lines`], empty text has no rows and a final line break, of any kind, starts none.
pub fn lines(text: &str, width: usize, rows: usize) -> Vec<String> {
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(ends_line))
        .unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    let mut wrapped = wrap(&[Span::new(text, WHITE)], width, Wrap::default());
    fit(&mut wrapped, rows, width);
    wrapped.iter().map(|row| plain(row)).collect()
}

/// Plain `text` on one row of at most `width` cells, with its runs of spaces and line breaks made one space, ending with "…" where cut, as [`cut`] does.
pub fn clip(text: &str, width: usize) -> String {
    let line = text
        .split(breaks)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    plain(&cut(&[Span::new(line, WHITE)], width))
}

/// The cells one line of `text` takes as drawn, its display width: two for each wide character, such as a CJK ideograph, kana or a full-width form, one for each other grapheme, and none for those that take no cell. Every row budget is counted in these.
pub fn width(text: &str) -> usize {
    drawn(text).count()
}

/// The cells the widest piece of `text` takes that [`wrap`] keeps on one row where it can: its longest word, or, among
/// CJK characters, the longest run it may not break, such as one before closing punctuation. A column narrower than this
/// breaks a word.
pub fn longest_word(text: &str) -> usize {
    let cells = graphemes([(text, WHITE)]);
    cells
        .split(|(ch, _)| *ch == '\n' || breaks(*ch))
        .flat_map(pieces)
        .map(span)
        .max()
        .unwrap_or_default()
}

/// `text` on one row of exactly `width` cells: cut with "…", as [`clip`] does, and padded with spaces after it.
pub fn pad(text: &str, width: usize) -> String {
    let text = clip(text, width);
    let fill = width - self::width(&text);
    text + &" ".repeat(fill)
}

/// The cells drawn for one line of `text`: line breaks, each as one space whatever its kind, and tabs as spaces, each other grapheme as [`shown`] draws it, a wide one followed by [`WIDE`], leaving out those that take no cell.
pub fn drawn(text: &str) -> impl Iterator<Item = char> + use<> {
    let mut cells = Vec::new();
    extend(&mut cells, &spaced(graphemes([(text, ())])));
    cells.into_iter().map(|(ch, ())| ch)
}

/// `graphemes` with line breaks and tabs as spaces.
fn spaced<T: Copy>(graphemes: Vec<(char, T)>) -> Vec<(char, T)> {
    graphemes
        .into_iter()
        .map(|(ch, extra)| (if matches!(ch, '\n' | '\t') { ' ' } else { ch }, extra))
        .collect()
}

/// The character each grapheme of `pieces` is drawn as, with its piece's `extra`, such as a style: a line feed for a line break of any kind, as Unicode's line breaking rules (UAX #14) count them, including CRLF split between pieces; otherwise its letter with the marks that compose with it, leaving out those that take no cell.
fn graphemes<'a, T: Copy>(pieces: impl IntoIterator<Item = (&'a str, T)>) -> Vec<(char, T)> {
    let mut text = String::new();
    let mut starts = Vec::new();
    for (piece, extra) in pieces {
        starts.push((text.len(), extra));
        text.push_str(piece);
    }
    text.grapheme_indices(true)
        .filter_map(|(offset, grapheme)| {
            // The piece the grapheme starts in: the last to start at or before it, as an empty piece starts where the next does.
            let extra = starts[starts.partition_point(|(start, _)| *start <= offset) - 1].1;
            composed(grapheme).map(|ch| (ch, extra))
        })
        .collect()
}

/// The one character `grapheme` is drawn as: a line feed for a line break; the composition of its letter with its marks, without those that do not compose; or `None` for one that takes no cell, such as a control character, a zero-width joiner or a mark with no letter.
fn composed(grapheme: &str) -> Option<char> {
    if grapheme.chars().any(ends_line) {
        return Some('\n');
    }
    let first = grapheme.chars().filter(|ch| !invisible(*ch)).nfc().next()?;
    (!is_combining_mark(first)).then_some(first)
}

/// Whether `ch` ends a line, as [`graphemes`] counts line breaks.
fn ends_line(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Whether `ch` takes no cell: a control character that is not a space or line break, such as an escape; a variation selector; or a zero-width space, joiner or soft hyphen.
fn invisible(ch: char) -> bool {
    (ch.is_control() && !ch.is_whitespace())
        || matches!(
            ch,
            '\u{ad}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2060}'..='\u{2064}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{e0100}'..='\u{e01ef}'
        )
}

/// `spans` on one row of at most `width` cells, ending with "…" where cut.
pub fn cut(spans: &[Span], width: usize) -> Vec<Cell> {
    let mut cells = Vec::new();
    extend(
        &mut cells,
        &spaced(graphemes(
            spans.iter().map(|span| (span.text.as_str(), span.style)),
        )),
    );
    if cells.len() > width {
        truncate(&mut cells, width.saturating_sub(1));
        let style = cells.last().map_or(WHITE, |(_, style)| *style);
        if width > 0 {
            cells.push(('…', style));
        }
    }
    cells
}

/// The character drawn for `ch`: itself where a bundled font has it; a space for a space of any width; its base letter, or the first letter of its compatibility decomposition, where a font has only that, such as for an accented letter outside the fonts or the ligature "ﬁ"; and `?` otherwise, such as for emoji, Hangul or kanji outside JIS X 0208.
pub fn shown(ch: char) -> char {
    match ch {
        '\u{00a0}'
        | '\u{1680}'
        | '\u{2000}'..='\u{200a}'
        | '\u{202f}'
        | '\u{205f}'
        | '\u{3000}' => ' ',
        ch if tomori_text::fonts::covers(ch) => ch,
        ch => ch
            .nfkd()
            .next()
            .filter(|base| tomori_text::fonts::covers(*base))
            .unwrap_or('?'),
    }
}

/// A glyph's quad: where, in font pixels with y down, and its tile in the atlas, with how much bolder to draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// In atlas pixels: left, top, width, height.
    pub tile: [f32; 4],
    /// Added to the edge the shader draws, in the field's units.
    pub weight: f32,
}

/// Font pixels per em of a cell's glyph, which gives capitals about 8 of a row's 12, as the bitmap font had.
const EM: f32 = 11.0;
/// Of a wide character's, as a fraction of its row's height: an em is the whole of a CJK character's design, so this leaves about three quarters of a font pixel above and below it in a row of 12 or 11, and lines of Japanese never touch.
const WIDE_EM: f32 = 0.875;
/// How far above the baseline Noto Sans JP's em box reaches, in ems; the rest of it is below.
const CJK_ASCENT: f32 = 0.88;
/// Added to the weight of box drawing and blocks, which the shaders take as growing the glyph by half a screen pixel, so that where two meet each covers the pixels on their shared edge and no seam shows. A weight is otherwise at most a half, so the shaders tell the two apart at three quarters.
pub const JOIN: f32 = 1.0;

/// Whether `ch` is box drawing or a block, which fills its cells and joins its neighbours.
fn joins(ch: char) -> bool {
    ('\u{2500}'..='\u{259f}').contains(&ch)
}
/// How much bolder bold is on each side, in ems.
const BOLD: f32 = 0.04;
/// How far above and below the baseline Noto Sans Mono's box drawing and blocks reach, in ems, which a row is filled between.
const BOX_TOP: f32 = 0.979;
const BOX_BOTTOM: f32 = -0.246;

/// The glyph drawn for `ch` in `cells` cells from `x`, on a row `row` font pixels tall whose top is at `top`: centred in its cells on a baseline three quarters of the way down, and smaller where it is wider than its cells; a wide one sized to its row and with its em box centred in it, so that lines of Japanese keep apart; box drawing and blocks stretched to fill their cells and grown by half a screen pixel, so that lines join. `None` for one with no glyph, such as a space, or one the atlas has not drawn yet.
pub fn glyph(ch: char, cells: usize, x: f32, top: f32, row: f32, bold: bool) -> Option<Quad> {
    let glyph = atlas::glyph(ch)?;
    let room = (cells * CELL_WIDTH) as f32;
    let [left, up, width, height] = glyph.bounds;
    let weight = if bold { glyph.weight(BOLD) } else { 0.0 };
    if joins(ch) {
        // From the glyph's advance and the line box drawing reaches onto the cells.
        let across = room / glyph.advance;
        let down = row / (BOX_TOP - BOX_BOTTOM);
        return Some(Quad {
            x: x + left * across,
            y: top + (BOX_TOP - up) * down,
            width: width * across,
            height: height * down,
            tile: glyph.tile,
            weight: weight + JOIN,
        });
    }
    let wide = cells > 1;
    let scale = (if wide { row * WIDE_EM } else { EM }).min(room / glyph.advance);
    let baseline = if wide {
        top + row / 2.0 + (CJK_ASCENT - 0.5) * scale
    } else {
        top + row * 0.75
    };
    let pen = x + (room - glyph.advance * scale) / 2.0;
    Some(Quad {
        x: pen + left * scale,
        y: baseline - up * scale,
        width: width * scale,
        height: height * scale,
        tile: glyph.tile,
        weight,
    })
}

/// The glyphs of a row of `cells` whose first cell is at `x`, on a row `row` font pixels tall whose top is at `top`, each with its cell's `extra`, such as its style, drawn bold where `bold` says so.
pub fn quads<T: Copy>(
    cells: &[(char, T)],
    x: f32,
    top: f32,
    row: f32,
    bold: impl Fn(T) -> bool,
) -> impl Iterator<Item = (Quad, T)> {
    placed(cells).filter_map(move |(column, ch, cells, extra)| {
        let left = x + (column * CELL_WIDTH) as f32;
        glyph(ch, cells, left, top, row, bold(extra)).map(|quad| (quad, extra))
    })
}

/// The cells of a row that draw something, each with its column and how many cells it takes: every character but spaces and the second cells of wide ones. A wide character always takes two, as its width says, so that it is never drawn shrunk into one; layout never leaves one without its second cell.
pub fn placed<T: Copy>(cells: &[(char, T)]) -> impl Iterator<Item = (usize, char, usize, T)> + '_ {
    cells
        .iter()
        .enumerate()
        .filter(|(_, (ch, _))| *ch != ' ' && *ch != WIDE)
        .map(|(column, &(ch, extra))| (column, ch, columns(ch), extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Vec<Cell>]) -> Vec<String> {
        rows.iter().map(|row| plain(row)).collect()
    }

    fn drawn(text: &str) -> String {
        super::drawn(text).filter(|ch| *ch != WIDE).collect()
    }

    const CODE: Wrap = Wrap {
        indent: 0,
        hanging: 0,
        continuation: Some('↵'),
    };

    #[test]
    fn the_longest_word_is_the_widest_piece_wrap_keeps_whole() {
        assert_eq!(longest_word("Rebase, then Recommendation\nnow"), 14);
        assert_eq!(longest_word(""), 0);
        // Between CJK characters a line may break, but not before closing punctuation.
        assert_eq!(longest_word("日本語のテキスト"), 2);
        assert_eq!(longest_word("完了。"), 4);
        // A non-breaking space joins the words either side of it.
        assert_eq!(longest_word("10\u{a0}km away"), 5);
    }

    #[test]
    fn typographic_characters_are_drawn_as_themselves_and_tabs_and_line_breaks_as_spaces() {
        assert_eq!(shown('\u{201c}'), '\u{201c}');
        assert_eq!(shown('\u{2014}'), '\u{2014}');
        assert_eq!(shown('\u{a0}'), ' ');
        let cells = cut(&[Span::new("a\tb\nc", WHITE)], 10);
        assert_eq!(plain(&cells), "a b c");
        assert!(cut(&[Span::new("abc", WHITE)], 0).is_empty());
        let code = wrap(&[Span::new("\tx", WHITE)], 10, CODE);
        assert_eq!(code[0][0].0, ' ');
    }

    #[test]
    fn a_space_between_code_and_text_takes_no_background() {
        let code = Style {
            background: Some([0.1; 4]),
            ..WHITE
        };
        let rows = wrap(
            &[Span::new("code", code), Span::new(" text", WHITE)],
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
        // A hyphen is not a place to break, so words keep together as before.
        assert_eq!(lines("a well-known", 10, 9), ["a", "well-known"]);
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
        let rows = wrap(&[Span::new("let  x = 1234567890;", code)], 10, CODE);
        assert_eq!(text(&rows), ["let  x = ↵", "123456789↵", "0;"]);
        assert!(rows.iter().all(|row| row.len() <= 10));
        // A wide character is never split by the mark: it moves to the next row, and one with no room beside the mark fills a row alone.
        let rows = wrap(&[Span::new("let 名前 = 1;", code)], 6, CODE);
        assert_eq!(text(&rows), ["let ↵", "名前 ↵", "= 1;"]);
        assert!(rows.iter().all(|row| row.len() <= 6));
        let rows = wrap(&[Span::new("日本", code)], 2, CODE);
        assert_eq!(text(&rows), ["日", "本"]);
    }

    #[test]
    fn every_character_tomori_draws_has_a_glyph_and_the_rest_are_a_question_mark() {
        for ch in [
            'a', 'é', 'Ł', 'Œ', 'ễ', '│', '█', 'λ', 'Ж', '•', '✓', '↵', '→', '…', '⋯', '▸', '○',
            '◐', '●', '✕', '▾', '“', '—', 'こ', '語', '。',
        ] {
            assert_eq!(shown(ch), ch, "{ch}");
            assert!(atlas::glyph(ch).is_some(), "{ch}");
        }
        // An accented letter no font has is drawn as its base letter.
        assert_eq!(shown('ἀ'), 'α');
        for ch in ['🎤', '한', '𠮷', WIDE] {
            assert_eq!(shown(ch), '?', "{ch}");
        }
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
        for text in [
            "",
            "x",
            &"word ".repeat(50),
            &"x".repeat(100),
            "a\n\n\n\nb",
            &"日本語の文章。".repeat(20),
        ] {
            for width in [2, 3, 7, 40] {
                let rows = wrap(&[Span::new(text, WHITE)], width, Wrap::default());
                assert!(rows.iter().all(|row| row.len() <= width), "{rows:?}");
                let rows = lines(text, width, 3);
                assert!(rows.len() <= 3);
                assert!(rows.iter().all(|row| drawn_width(row) <= width), "{rows:?}");
            }
        }
    }

    fn drawn_width(text: &str) -> usize {
        super::drawn(text).count()
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
        assert_eq!(clip("e\u{301}".repeat(10).as_str(), 4), "ééé…");
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
        // Half a wide character is never left before the mark.
        let mut rows = wrap(&[Span::new("日本語の文章", red)], 6, Wrap::default());
        assert_eq!(text(&rows), ["日本語", "の文章"]);
        fit(&mut rows, 1, 6);
        assert_eq!(text(&rows), ["日本…"]);
        assert_eq!(rows[0].len(), 5);
    }

    #[test]
    fn accented_letters_and_typographic_punctuation_are_drawn_as_they_are() {
        // Every letter here is in the bundled fonts, so none needs a stand-in.
        for text in [
            "Łódź Dvořák Ştefan Ţară Erdoğan Đorđe",
            "Nguyễn Thị Hương",
            "Lǚ ƀ Ơ ơ Ư ư Ș ș Ț ț ſ",
            "Œuvre ĳ",
            "„quoted‟ ‚single‛ 5′ 6″ ‹a› ½⁄2",
            "a‐b‑c‒d–e—f―g−h",
            "wait… ⋯",
        ] {
            assert_eq!(drawn(text), text);
        }
        // Spaces of other widths are drawn as a space.
        assert_eq!(drawn("a\u{2009}b\u{3000}c"), "a b c");
        // Cyrillic and Japanese are drawn; emoji, which no font has, are still `?`.
        assert_eq!(drawn("Жこ🎤"), "Жこ?");
    }

    #[test]
    fn a_grapheme_is_one_character_and_characters_that_take_no_cell_are_left_out() {
        // A carriage return, an escape, a variation selector, zero-width joiners and spaces, a soft hyphen and a byte order mark.
        assert_eq!(drawn("a\r\nb"), "a b");
        assert_eq!(drawn("\u{1b}[31mred"), "[31mred");
        assert_eq!(drawn("✓\u{fe0f}"), "✓");
        assert_eq!(drawn("葛\u{e0100}"), "葛");
        assert_eq!(drawn("a\u{200b}b\u{200d}c\u{ad}d\u{feff}"), "abcd");
        // A letter and its marks compose to one character, in one cell, and a mark that does not compose is left out.
        assert_eq!(drawn("e\u{301}"), "é");
        assert_eq!(drawn("e\u{323}\u{302}"), "ệ");
        assert_eq!(drawn("x\u{301}"), "x");
        assert_eq!(drawn("\u{301}x"), "x");
        // An emoji with a skin tone or joined into a family is one grapheme, drawn `?`.
        assert_eq!(drawn("👍🏽 👨\u{200d}👩\u{200d}👧"), "? ?");
        assert_eq!(cut(&[Span::new("e\u{301}\u{301}x", WHITE)], 2).len(), 2);
        let rows = wrap(&[Span::new("ab\u{301}c\r\nd", WHITE)], 3, Wrap::default());
        assert_eq!(text(&rows), ["abc", "d"]);
    }

    #[test]
    fn cjk_takes_two_cells_and_breaks_between_characters_but_not_before_closing_punctuation() {
        // Each character takes two cells, the second holding the mark that keeps cells as columns.
        let cells: Vec<char> = super::drawn("a日b").collect();
        assert_eq!(cells, ['a', '日', WIDE, 'b']);
        let row = Row::text("日本", WHITE, 10);
        assert_eq!(row.cells.len(), 4);
        assert_eq!(row.plain(), "日本");
        // Without spaces, lines break between characters; never before "。", which takes the character before it to the next row.
        assert_eq!(
            lines("日本語のテキストです。", 10, 9),
            ["日本語のテ", "キストで", "す。"]
        );
        // Nor after opening punctuation.
        assert_eq!(lines("あい「うえ」", 6, 9), ["あい", "「う", "え」"]);
        // Latin words still break only at spaces, and a word beside a CJK character may break from it.
        assert_eq!(lines("Tomoriは日本語", 7, 9), ["Tomori", "は日本", "語"]);
        assert_eq!(lines("hello world 日本", 7, 9), ["hello", "world", "日本"]);
        // Within a word with CJK in it, a break UAX #14 allows away from it, as after a hyphen, is still not taken.
        assert_eq!(lines("x-y日本", 5, 9), ["x-y日", "本"]);
        // A cut never splits a wide character.
        assert_eq!(plain(&cut(&[Span::new("日本語", WHITE)], 5)), "日本…");
        assert_eq!(plain(&cut(&[Span::new("日本語", WHITE)], 4)), "日…");
        assert_eq!(cut(&[Span::new("日本語", WHITE)], 4).len(), 3);
    }

    #[test]
    fn every_kind_of_line_break_is_one_as_a_line_feed_is() {
        // CRLF, a lone carriage return, a vertical tab, a form feed, next line, and Unicode's line and paragraph separators.
        let breaks = [
            "\n", "\r\n", "\r", "\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}",
        ];
        for line_break in breaks {
            let text = format!("50%{line_break}100%");
            // A row draws each as one space, never joining the words either side of it.
            assert_eq!(drawn(&text), "50% 100%", "{line_break:?}");
            assert_eq!(clip(&text, 40), "50% 100%", "{line_break:?}");
            // Wrapped, each starts a row, and a final one starts none.
            assert_eq!(lines(&text, 40, 9), ["50%", "100%"], "{line_break:?}");
            let text = format!("a{line_break}");
            assert_eq!(lines(&text, 40, 9), ["a"], "{line_break:?}");
            let rows = wrap(&[Span::new(text, WHITE)], 40, Wrap::default());
            assert_eq!(rows.len(), 2, "{line_break:?}");
        }
        // Two in a row leave a blank row between, CRLF counting once.
        assert_eq!(lines("a\r\rb", 40, 9), ["a", "", "b"]);
        assert_eq!(lines("a\r\n\r\nb", 40, 9), ["a", "", "b"]);
        assert_eq!(lines("a\n\rb", 40, 9), ["a", "", "b"]);
        // CRLF split between spans is one break, and an empty span between them changes nothing.
        let rows = wrap(
            &[
                Span::new("a\r", WHITE),
                Span::new("", WHITE),
                Span::new("\nb", WHITE),
            ],
            40,
            Wrap::default(),
        );
        assert_eq!(text(&rows), ["a", "b"]);
        // In code, too, where a carriage return no longer overwrites the line.
        let rows = wrap(&[Span::new("let a;\rlet b;\u{c}", WHITE)], 40, CODE);
        assert_eq!(text(&rows), ["let a;", "let b;", ""]);
        // Other spaces join nothing either, and are drawn as a space.
        assert_eq!(drawn("a\u{1680}b"), "a b");
        assert_eq!(lines("a\u{1680}b", 1, 9), ["a", "b"]);
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

    #[test]
    fn glyphs_sit_in_their_cells_on_one_baseline() {
        let inside = |quad: Quad, x: f32, width: f32| {
            // The tile reaches past the outline by the field's spread, under a font pixel at this size.
            let margin = atlas::SPREAD / 32.0 * EM;
            quad.x >= x - margin && quad.x + quad.width <= x + width + margin
        };
        let a = glyph('a', 1, 9.0, 0.0, 12.0, false).unwrap();
        assert!(inside(a, 9.0, 9.0), "{a:?}");
        assert_eq!(a.weight, 0.0);
        // Its baseline is three quarters down the row: "a" is under the x-height above it, "p" reaches below it.
        let p = glyph('p', 1, 0.0, 0.0, 12.0, false).unwrap();
        assert!(p.y + p.height > a.y + a.height);
        let capital = glyph('H', 1, 0.0, 0.0, 12.0, false).unwrap();
        assert!(capital.y < a.y);
        // Bold moves the edge out.
        assert!(glyph('a', 1, 0.0, 0.0, 12.0, true).unwrap().weight > 0.0);
        // A wide character is centred across its two cells, and fills more of them.
        let wide = glyph('語', 2, 0.0, 0.0, 12.0, false).unwrap();
        assert!(inside(wide, 0.0, 18.0), "{wide:?}");
        assert!(((wide.x + wide.width / 2.0) - 9.0).abs() < 1.0, "{wide:?}");
        assert!(wide.width > a.width);
        // A symbol wider than its cell is made smaller to fit it.
        let check = glyph('✓', 1, 0.0, 0.0, 12.0, false).unwrap();
        assert!(inside(check, 0.0, 9.0), "{check:?}");
        // Box drawing is stretched across its cell and down its row, so that lines join.
        let line = glyph('─', 1, 0.0, 0.0, 12.0, false).unwrap();
        assert!(line.x < 0.0 && line.x + line.width > 9.0, "{line:?}");
        let bar = glyph('│', 1, 0.0, 0.0, 11.0, false).unwrap();
        assert!(bar.y < 0.0 && bar.y + bar.height > 11.0, "{bar:?}");
        // The block's outline, inside the field's margin, is the row exactly.
        let block = glyph('█', 1, 0.0, 0.0, 11.0, false).unwrap();
        let margin = atlas::SPREAD / 32.0 * 11.0 / (BOX_TOP - BOX_BOTTOM);
        assert!((block.y + margin).abs() < 0.5, "{block:?}");
        assert!(
            (block.y + block.height - margin - 11.0).abs() < 0.5,
            "{block:?}"
        );
        // A space has no glyph.
        assert_eq!(glyph(' ', 1, 0.0, 0.0, 12.0, false), None);
    }

    #[test]
    fn text_is_measured_and_padded_by_the_cells_it_takes_as_drawn() {
        assert_eq!(width(""), 0);
        assert_eq!(width("abc"), 3);
        // A CJK character takes two cells; an emoji, drawn `?`, and a composed accent, one; a zero-width joiner none.
        assert_eq!(width("日本語"), 6);
        assert_eq!(width("Tomoriは日本語"), 14);
        assert_eq!(width("👍🏽 e\u{301}\u{200d}"), 3);
        assert_eq!(pad("Ada", 6), "Ada   ");
        assert_eq!(pad("山田", 6), "山田  ");
        // Cut with "…" and padded where a wide character would cross the edge.
        assert_eq!(pad("山田太郎", 6), "山田… ");
        assert_eq!(pad("Alexander", 6), "Alexa…");
        for text in ["", "Ada", "山田太郎", "Bob 佐藤 🎤", "Ｔｏｍｏｒｉ"] {
            assert_eq!(width(&pad(text, 6)), 6, "{text}");
        }
    }

    #[test]
    fn compatibility_characters_are_drawn_as_their_letters() {
        assert_eq!(drawn("\u{fb01}x"), "fx");
        assert_eq!(shown('\u{2460}'), '1');
    }

    #[test]
    fn a_wide_character_always_takes_two_cells_and_is_never_drawn_shrunk() {
        // Even where its second cell was lost, as a cut by characters once did, it is drawn at its size across two cells.
        let cells = [('a', ()), ('日', ()), ('本', ()), (WIDE, ())];
        let placed: Vec<_> = placed(&cells).collect();
        assert_eq!(
            placed,
            [(0, 'a', 1, ()), (1, '日', 2, ()), (2, '本', 2, ())]
        );
        assert!(!whole(&cells));
        assert!(whole(&[('日', ()), (WIDE, ()), ('a', ())]));
        assert!(!whole(&[(WIDE, ()), ('a', ())]));
        assert!(!whole(&[('a', ()), (WIDE, ())]));
        let quads: Vec<Quad> = quads(&cells, 0.0, 0.0, 11.0, |()| false)
            .map(|(quad, ())| quad)
            .collect();
        // Both are drawn at a wide character's size, an em to seven eighths of the row.
        for (quad, ch) in quads[1..].iter().zip(['日', '本']) {
            let em = quad.width / atlas::glyph(ch).unwrap().bounds[2];
            assert!((em - 11.0 * WIDE_EM).abs() < 0.01, "{ch}: {em}");
        }
        // Every rule that lays out a row leaves every wide character whole.
        for text in [
            "a日本",
            "日本語のテキスト",
            "Tomoriは日本語 🎉 mixed",
            "x日y本z語",
        ] {
            for width in 1..12 {
                assert!(
                    whole(&cut(&[Span::new(text, WHITE)], width)),
                    "{text} {width}"
                );
                for row in wrap(&[Span::new(text, WHITE)], width, Wrap::default()) {
                    assert!(whole(&row), "{text} {width}");
                }
                for row in wrap(&[Span::new(text, WHITE)], width, CODE) {
                    assert!(whole(&row), "{text} {width}");
                }
                let mut rows = wrap(&[Span::new(text, WHITE)], width, Wrap::default());
                fit(&mut rows, 1, width);
                assert!(
                    rows.iter()
                        .all(|row| whole(row) && row.len() <= width.max(2))
                );
            }
        }
    }

    #[test]
    fn lines_of_japanese_keep_apart_in_rows_of_twelve_and_eleven() {
        for row in [12.0, 11.0] {
            for ch in "日本語のスペース。漢字龍鬱「」ぎゃ".chars() {
                let quad = glyph(ch, 2, 0.0, 0.0, row, false).unwrap();
                let glyph = atlas::glyph(ch).unwrap();
                // The em box, inside the tile's margin, lies within the row with room above and below it.
                let scale = quad.width / glyph.bounds[2];
                let margin = atlas::SPREAD / glyph.scale * scale;
                let (top, bottom) = (quad.y + margin, quad.y + quad.height - margin);
                assert!(
                    top >= -0.01 && bottom <= row + 0.01,
                    "{ch} in {row}: {top} to {bottom}"
                );
                // And the em box, which the next row's characters reach to, is at least half a font pixel from the row's edges, so that neighbouring rows' ink stays a font pixel apart.
                let baseline = quad.y + glyph.bounds[1] * scale;
                let (em_top, em_bottom) = (
                    baseline - CJK_ASCENT * scale,
                    baseline + (1.0 - CJK_ASCENT) * scale,
                );
                assert!(
                    em_top >= 0.5 && em_bottom <= row - 0.5,
                    "{ch} in {row}: {em_top} to {em_bottom}"
                );
                assert!((scale - row * WIDE_EM).abs() < 0.01);
            }
        }
    }

    #[test]
    fn box_drawing_and_blocks_are_marked_to_join() {
        for ch in ['─', '│', '┼', '█', '▌'] {
            let quad = glyph(ch, 1, 0.0, 0.0, 12.0, false).unwrap();
            assert!(quad.weight >= JOIN, "{ch}");
            let bold = glyph(ch, 1, 0.0, 0.0, 12.0, true).unwrap();
            assert!(
                bold.weight > quad.weight && bold.weight < JOIN + 0.5,
                "{ch}"
            );
        }
        assert!(glyph('a', 1, 0.0, 0.0, 12.0, true).unwrap().weight < JOIN);
    }

    #[test]
    fn placed_cells_skip_spaces_and_the_second_halves_of_wide_characters() {
        let cells: Vec<(char, ())> = super::drawn("a 日b").map(|ch| (ch, ())).collect();
        let placed: Vec<_> = placed(&cells).collect();
        assert_eq!(placed, [(0, 'a', 1, ()), (2, '日', 2, ()), (4, 'b', 1, ())]);
        // Their quads are where `glyph` puts them, from the row's first cell.
        let quads: Vec<_> = quads(&cells, 1.0, 2.0, 12.0, |()| false)
            .map(|(quad, ())| quad)
            .collect();
        assert_eq!(quads.len(), 3);
        assert_eq!(
            quads[1],
            glyph('日', 2, 1.0 + 18.0, 2.0, 12.0, false).unwrap()
        );
    }
}
