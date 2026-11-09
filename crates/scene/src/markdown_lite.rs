// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Markdown-lite: a message's Markdown as blocks of text with their emphasis, for the transcript to style, so that none of `**`, `#` or backticks shows. Parsed with `base-markdown`, as the speech crate splits replies.
//!
//! Headings, list items with their bullets or numbers, code blocks, quotes, rules and tables are blocks; strong text, emphasis and inline code are marked within them, as within each cell of a table; links show their text only, and a `<br>` breaks the line, as agents write one in a table's cell. A task item, `- [ ]` or `- [x]`, says whether it is ticked rather than showing its box as text, and every block of a list item knows the line of the source the item begins on, so that what is pointed at on a panel can be named.
use std::sync::LazyLock;

use base_markdown::{
    AlignKind, EventParser, ParseOptions,
    stream::{EventKind, Tag, TagKind},
};

/// GitHub Flavored Markdown, as assistant replies are written.
static OPTIONS: LazyLock<ParseOptions> = LazyLock::new(ParseOptions::gfm);

/// How a run of text within a block is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    Plain,
    Strong,
    Emphasis,
    Code,
}

/// What a block is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Heading(u8),
    /// A list item, `depth` lists deep from 1, starting with `marker`, such as "•" or "2.", or none for a later paragraph of the same item.
    Item {
        depth: usize,
        marker: Option<String>,
    },
    /// A code block, its lines kept as written.
    Code,
    Quote,
    Rule,
    /// A table, with its header, its rows and how each column is aligned; the block's own text is empty.
    Table(Table),
}

/// Text with its marks, as a block or a table's cell holds it.
pub type Text = Vec<(String, Inline)>;

/// How a table's column is aligned, as its delimiter row says: `:---` or `---` left, `:---:` centred and `---:` right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

/// A table: its header's cells, its rows' cells and its columns' alignments. Rows may have fewer cells than the header,
/// as GitHub's Markdown pads them, and the header is empty where its cells all are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub align: Vec<Align>,
    pub header: Vec<Text>,
    pub rows: Vec<Vec<Text>>,
}

impl Table {
    /// How many columns the table has: the most cells of any of its rows or its header.
    pub fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .chain([self.header.len(), self.align.len()])
            .max()
            .unwrap_or_default()
    }
}

/// `text` without its marks.
pub fn plain(text: &[(String, Inline)]) -> String {
    text.iter().map(|(text, _)| text.as_str()).collect()
}

/// One block of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub text: Vec<(String, Inline)>,
    /// For a task item, whether it is ticked.
    pub task: Option<bool>,
    /// For a block of a list item, the line of the source the item begins on, from 0.
    pub line: Option<usize>,
}

impl Block {
    /// The block's text without its marks.
    #[cfg(test)]
    pub fn plain(&self) -> String {
        plain(&self.text)
    }
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    current: Option<Block>,
    /// The next number of each open list, or `None` for a bulleted one.
    lists: Vec<Option<u64>>,
    /// The marker the next block in an item starts with.
    marker: Option<String>,
    strong: usize,
    emphasis: usize,
    quote: usize,
    /// The table being read, its row being read and whether that row is its header.
    table: Option<Table>,
    row: Option<Vec<Text>>,
    header: bool,
    /// The line each open list item begins on, the innermost last.
    items: Vec<usize>,
    /// A task's box was just read, so the space after it is not shown.
    boxed: bool,
}

impl Builder {
    fn inline(&self) -> Inline {
        if self.strong > 0 {
            Inline::Strong
        } else if self.emphasis > 0 {
            Inline::Emphasis
        } else {
            Inline::Plain
        }
    }

    fn open(&mut self, kind: Kind) {
        self.flush();
        let line = matches!(kind, Kind::Item { .. })
            .then(|| self.items.last().copied())
            .flatten();
        self.current = Some(Block {
            kind,
            text: Vec::new(),
            task: None,
            line,
        });
    }

    /// The block text goes into when none is open: a list item's, a quote's, or a paragraph.
    fn implied(&mut self) -> Kind {
        if !self.lists.is_empty() {
            Kind::Item {
                depth: self.lists.len(),
                marker: self.marker.take(),
            }
        } else if self.quote > 0 {
            Kind::Quote
        } else {
            Kind::Paragraph
        }
    }

    /// The open block, opening the one text goes into when none is.
    fn block(&mut self) -> &mut Block {
        if self.current.is_none() {
            let kind = self.implied();
            self.open(kind);
        }
        self.current.as_mut().expect("opened above")
    }

    fn push(&mut self, text: &str, inline: Inline) {
        // Text in a table goes into its cell.
        if let Some(cell) = self.row.as_mut().and_then(|row| row.last_mut()) {
            match cell.last_mut() {
                Some((last, mark)) if *mark == inline => last.push_str(text),
                _ => cell.push((text.to_owned(), inline)),
            }
            return;
        }
        let text = if std::mem::take(&mut self.boxed) {
            text.trim_start()
        } else {
            text
        };
        if text.is_empty() && self.current.is_some() {
            return;
        }
        let block = self.block();
        match block.text.last_mut() {
            Some((last, mark)) if *mark == inline => last.push_str(text),
            _ => block.text.push((text.to_owned(), inline)),
        }
    }

    /// Whether the open block's text ends with a line break.
    fn ends_line(&self) -> bool {
        self.current
            .as_ref()
            .and_then(|block| block.text.last())
            .is_some_and(|(text, _)| text.ends_with('\n'))
    }

    fn flush(&mut self) {
        if let Some(mut block) = self.current.take() {
            if let Some((last, _)) = block.text.last_mut() {
                let trimmed = last.trim_end().len();
                last.truncate(trimmed);
            }
            if !block.text.iter().all(|(text, _)| text.is_empty())
                || matches!(block.kind, Kind::Rule | Kind::Item { .. } | Kind::Table(_))
            {
                self.blocks.push(block);
            }
        }
    }
}

impl Builder {
    /// Ends the table's row being read, its cells trimmed, as its header or a row of its body.
    fn end_row(&mut self) {
        let (Some(table), Some(mut row)) = (&mut self.table, self.row.take()) else {
            return;
        };
        for cell in &mut row {
            trim(cell);
        }
        if self.header {
            // A header of empty cells is none, as nothing would show in it.
            if row.iter().any(|cell| !cell.is_empty()) {
                table.header = row;
            }
        } else {
            table.rows.push(row);
        }
    }
}

/// `text` without the spaces it starts and ends with, nor runs left empty by them.
fn trim(text: &mut Text) {
    if let Some((first, _)) = text.first_mut() {
        let start = first.len() - first.trim_start().len();
        first.drain(..start);
    }
    if let Some((last, _)) = text.last_mut() {
        let end = last.trim_end().len();
        last.truncate(end);
    }
    text.retain(|(text, _)| !text.is_empty());
}

/// Whether inline HTML is a line break, `<br>`, `<br/>` or `<br />`, in any case.
fn line_break(html: &str) -> bool {
    let tag = html.trim().to_ascii_lowercase();
    let Some(inner) = tag
        .strip_prefix("<br")
        .and_then(|tag| tag.strip_suffix('>'))
    else {
        return false;
    };
    matches!(inner.trim(), "" | "/")
}

/// `text`'s blocks.
///
/// The parser reports errors only from custom event producers, and a text's ranges only into that text, which is never one of them, so neither error happens here: an event that failed would be left out, and a range that failed would be empty.
pub fn parse(source: &str) -> Vec<Block> {
    let mut builder = Builder::default();
    for event in EventParser::new(source, &OPTIONS).filter_map(Result::ok) {
        match event.kind {
            EventKind::Start(tag) => match tag {
                Tag::Item { .. } => {
                    builder.flush();
                    let begins = source.get(..event.span.start).unwrap_or_default();
                    builder.items.push(begins.matches('\n').count());
                    let marker = match builder.lists.last_mut() {
                        Some(Some(number)) => {
                            let marker = format!("{number}.");
                            *number += 1;
                            marker
                        }
                        _ => "•".to_owned(),
                    };
                    builder.marker = Some(marker);
                }
                Tag::Paragraph { .. } => {
                    let kind = builder.implied();
                    builder.open(kind);
                }
                Tag::Heading(level) => builder.open(Kind::Heading(level)),
                Tag::BlockQuote => {
                    builder.flush();
                    builder.quote += 1;
                }
                Tag::List { start, .. } => {
                    builder.flush();
                    builder.lists.push(start.map(u64::from));
                }
                Tag::Strong => builder.strong += 1,
                Tag::Emphasis => builder.emphasis += 1,
                Tag::Table => {
                    builder.flush();
                    builder.table = Some(Table::default());
                }
                Tag::TableHead => builder.header = true,
                Tag::TableRow => builder.row = Some(Vec::new()),
                Tag::TableCell { alignment, .. } => {
                    if builder.header
                        && let Some(table) = &mut builder.table
                    {
                        table.align.push(match alignment {
                            AlignKind::Center => Align::Centre,
                            AlignKind::Right => Align::Right,
                            _ => Align::Left,
                        });
                    }
                    builder.row.get_or_insert_default().push(Vec::new());
                }
                _ => {}
            },
            EventKind::End(kind) => match kind {
                TagKind::Paragraph | TagKind::Heading => builder.flush(),
                TagKind::TableRow => builder.end_row(),
                TagKind::Table => {
                    builder.end_row();
                    // A table's end always follows its start, which began one.
                    let table = builder.table.take().unwrap_or_default();
                    builder.open(Kind::Table(table));
                    builder.flush();
                }
                TagKind::Item => {
                    // An item with nothing in it still shows its marker.
                    if let Some(marker) = builder.marker.take() {
                        builder.open(Kind::Item {
                            depth: builder.lists.len(),
                            marker: Some(marker),
                        });
                    }
                    builder.flush();
                    builder.items.pop();
                }
                TagKind::List => {
                    builder.flush();
                    builder.lists.pop();
                }
                TagKind::BlockQuote => {
                    builder.flush();
                    builder.quote = builder.quote.saturating_sub(1);
                }
                TagKind::Strong => builder.strong = builder.strong.saturating_sub(1),
                TagKind::Emphasis => builder.emphasis = builder.emphasis.saturating_sub(1),
                TagKind::TableHead => {
                    builder.end_row();
                    builder.header = false;
                }
                _ => {}
            },
            EventKind::Text(text) => {
                let text = text.resolve(source).unwrap_or_default().to_owned();
                let inline = builder.inline();
                builder.push(&text, inline);
            }
            // The parser reports a hard break's line ending as a soft break after it, which must not start the next line with a space.
            EventKind::SoftBreak(_) if builder.ends_line() => {}
            EventKind::SoftBreak(_) => builder.push(" ", Inline::Plain),
            EventKind::HardBreak => builder.push("\n", Inline::Plain),
            EventKind::Code { value, .. } => {
                let value = value.resolve(source).unwrap_or_default().to_owned();
                builder.push(&value, Inline::Code);
            }
            EventKind::CodeBlock { value, .. } => {
                let value = value.resolve(source).unwrap_or_default();
                builder.open(Kind::Code);
                builder.push(value.trim_end_matches('\n'), Inline::Code);
                builder.flush();
            }
            EventKind::Rule => {
                builder.open(Kind::Rule);
                builder.flush();
            }
            // The item's block says whether it is ticked; the space after the box is in the text that follows it, and is
            // not shown.
            EventKind::TaskMarker(done) => {
                builder.block().task = Some(done);
                builder.boxed = true;
            }
            // GitHub's Markdown, which this parses, has no math.
            EventKind::Html { value, .. } => {
                let value = value.resolve(source).unwrap_or_default().to_owned();
                if line_break(&value) {
                    builder.push("\n", Inline::Plain);
                } else {
                    builder.push(&value, Inline::Plain);
                }
            }
            _ => {}
        }
    }
    builder.flush();
    builder.blocks
}

/// The tables among `source`'s blocks, for tests.
#[cfg(test)]
pub fn tables(source: &str) -> Vec<Table> {
    parse(source)
        .into_iter()
        .filter_map(|block| match block.kind {
            Kind::Table(table) => Some(table),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(blocks: &[Block]) -> Vec<(Kind, String)> {
        blocks
            .iter()
            .map(|block| (block.kind.clone(), block.plain()))
            .collect()
    }

    #[test]
    fn markers_are_removed_and_emphasis_kept() {
        let blocks =
            parse("# Result\n\nAll **214** pass after fixing `span_end`, see [the PR](https://x).");
        assert_eq!(
            plain(&blocks),
            [
                (Kind::Heading(1), "Result".into()),
                (
                    Kind::Paragraph,
                    "All 214 pass after fixing span_end, see the PR.".into()
                ),
            ]
        );
        assert!(blocks[1].text.contains(&("214".into(), Inline::Strong)));
        assert!(blocks[1].text.contains(&("span_end".into(), Inline::Code)));
    }

    #[test]
    fn lists_have_bullets_or_numbers_and_depth() {
        let blocks = parse("- one\n- two\n  1. inner\n  2. next\n");
        assert_eq!(
            plain(&blocks),
            [
                (
                    Kind::Item {
                        depth: 1,
                        marker: Some("•".into())
                    },
                    "one".into()
                ),
                (
                    Kind::Item {
                        depth: 1,
                        marker: Some("•".into())
                    },
                    "two".into()
                ),
                (
                    Kind::Item {
                        depth: 2,
                        marker: Some("1.".into())
                    },
                    "inner".into()
                ),
                (
                    Kind::Item {
                        depth: 2,
                        marker: Some("2.".into())
                    },
                    "next".into()
                ),
            ]
        );
    }

    #[test]
    fn code_blocks_tables_and_quotes_are_blocks() {
        let source = "```rust\nfn main() {}\n  let x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted\n\n---\n";
        let blocks = parse(source);
        let cell = |text: &str| vec![(text.to_owned(), Inline::Plain)];
        let table = Table {
            align: vec![Align::Left; 2],
            header: vec![cell("a"), cell("b")],
            rows: vec![vec![cell("1"), cell("2")]],
        };
        assert_eq!(
            plain(&blocks),
            [
                (Kind::Code, "fn main() {}\n  let x = 1;".into()),
                (Kind::Table(table), String::new()),
                (Kind::Quote, "quoted".into()),
                (Kind::Rule, String::new()),
            ]
        );
        // Of all of them, one is a table.
        assert_eq!(tables(source).len(), 1);
    }

    fn table(source: &str) -> Table {
        tables(source).remove(0)
    }

    #[test]
    fn a_table_keeps_its_alignment_its_cells_marks_and_rows_shorter_than_its_header() {
        let table = table(
            "| PR | What | Recommendation | Size |\n|:---|:---:|---:|---|\n| 233 | **Your call:** `B` and *sticks* | Rebase | 3 |\n| 90 | Short \\| row |\n",
        );
        assert_eq!(
            table.align,
            [Align::Left, Align::Centre, Align::Right, Align::Left]
        );
        assert_eq!(table.columns(), 4);
        assert_eq!(
            table
                .header
                .iter()
                .map(|cell| super::plain(cell))
                .collect::<Vec<_>>(),
            ["PR", "What", "Recommendation", "Size"]
        );
        assert_eq!(
            table.rows[0][1],
            [
                ("Your call:".into(), Inline::Strong),
                (" ".into(), Inline::Plain),
                ("B".into(), Inline::Code),
                (" and ".into(), Inline::Plain),
                ("sticks".into(), Inline::Emphasis),
            ]
        );
        // An escaped pipe is the cell's text, and a row with fewer cells keeps them.
        assert_eq!(super::plain(&table.rows[1][1]), "Short | row");
        assert!(table.rows[1].len() <= 4);
    }

    #[test]
    fn a_break_in_a_cell_is_kept_and_a_header_of_empty_cells_is_none() {
        let table = table("| | |\n|---|---|\n| one<br>two | three<BR />four |\n");
        assert!(table.header.is_empty());
        assert_eq!(super::plain(&table.rows[0][0]), "one\ntwo");
        assert_eq!(super::plain(&table.rows[0][1]), "three\nfour");
        // A break elsewhere breaks the line, and other HTML is kept as text.
        let blocks = parse("first<br/>second <br x> third");
        assert_eq!(blocks[0].plain(), "first\nsecond <br x> third");
    }

    #[test]
    fn emphasis_html_and_empty_items_are_kept() {
        let blocks = parse("*soft* and **firm**\nnext <b>tag</b>\n\n-\n- after\n");
        assert_eq!(
            blocks[0].text,
            [
                ("soft".into(), Inline::Emphasis),
                (" and ".into(), Inline::Plain),
                ("firm".into(), Inline::Strong),
                (" next <b>tag</b>".into(), Inline::Plain),
            ]
        );
        let item = |marker: Option<&str>| Kind::Item {
            depth: 1,
            marker: marker.map(str::to_owned),
        };
        assert_eq!(
            plain(&blocks[1..]),
            [
                (item(Some("•")), String::new()),
                (item(Some("•")), "after".into()),
            ]
        );
    }

    #[test]
    fn raw_html_on_its_own_is_a_paragraph_and_an_empty_heading_is_left_out() {
        assert_eq!(
            plain(&parse("<div>raw</div>\n\n#\n")),
            [(Kind::Paragraph, "<div>raw</div>".into())]
        );
    }

    #[test]
    fn a_hard_break_starts_the_next_line_without_a_space() {
        let blocks = parse("first  \nsecond\\\nthird");
        assert_eq!(blocks[0].plain(), "first\nsecond\nthird");
    }

    #[test]
    fn a_task_item_says_whether_it_is_ticked_without_its_box_or_the_space_after_it() {
        let item = |depth| Kind::Item {
            depth,
            marker: Some("•".into()),
        };
        let blocks = parse("- [x] done\n- [ ] **todo**\n  - [X] nested\n- plain\n");
        assert_eq!(
            plain(&blocks),
            [
                (item(1), "done".into()),
                (item(1), "todo".into()),
                (item(2), "nested".into()),
                (item(1), "plain".into()),
            ]
        );
        assert_eq!(
            blocks.iter().map(|block| block.task).collect::<Vec<_>>(),
            [Some(true), Some(false), Some(true), None]
        );
        assert_eq!(blocks[1].text, [("todo".into(), Inline::Strong)]);
    }

    #[test]
    fn each_block_of_a_list_item_knows_the_line_its_item_begins_on() {
        let blocks = parse(
            "Topics:\n\n- one\n  continued\n\n  second paragraph\n- [ ] two\n  1. inner\n\nAfter.\n",
        );
        assert_eq!(
            blocks
                .iter()
                .map(|block| (block.plain(), block.line))
                .collect::<Vec<_>>(),
            [
                ("Topics:".into(), None),
                ("one continued".into(), Some(2)),
                ("second paragraph".into(), Some(2)),
                ("two".into(), Some(6)),
                ("inner".into(), Some(7)),
                ("After.".into(), None),
            ]
        );
    }

    #[test]
    fn a_message_still_streaming_parses_what_has_arrived() {
        let blocks = parse("Working on **it");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].plain().starts_with("Working on"));
        assert!(parse("").is_empty());
    }
}
