// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Markdown-lite: a message's Markdown as blocks of text with their emphasis, for the transcript to style, so that none of `**`, `#` or backticks shows. Parsed with `base-markdown`, as the speech crate splits replies.
//!
//! Headings, list items with their bullets or numbers, code blocks, quotes, rules and table rows are blocks; strong text, emphasis and inline code are marked within them; links show their text only.

use std::sync::LazyLock;

use base_markdown::{
    EventParser, ParseOptions,
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
    /// A table's row, its cells separated by " │ ".
    TableRow {
        header: bool,
    },
}

/// One block of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub text: Vec<(String, Inline)>,
}

impl Block {
    /// The block's text without its marks.
    #[cfg(any(test, feature = "testing"))]
    pub fn plain(&self) -> String {
        self.text.iter().map(|(text, _)| text.as_str()).collect()
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
    header: bool,
    cells: usize,
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
        self.current = Some(Block {
            kind,
            text: Vec::new(),
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

    fn push(&mut self, text: &str, inline: Inline) {
        if self.current.is_none() {
            let kind = self.implied();
            self.open(kind);
        }
        let block = self.current.as_mut().expect("opened above");
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
                || matches!(block.kind, Kind::Rule | Kind::Item { .. })
            {
                self.blocks.push(block);
            }
        }
    }
}

/// `text`'s blocks.
///
/// The parser reports errors only from custom event producers, and a text's ranges only into that text, which is never one of them, so neither error happens here: an event that failed would be left out, and a range that failed would be empty.
pub fn parse(source: &str) -> Vec<Block> {
    let mut builder = Builder::default();
    for event in EventParser::new(source, &OPTIONS).filter_map(Result::ok) {
        match event.kind {
            EventKind::Start(tag) => match tag {
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
                Tag::Item { .. } => {
                    builder.flush();
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
                Tag::Strong => builder.strong += 1,
                Tag::Emphasis => builder.emphasis += 1,
                Tag::TableHead => builder.header = true,
                Tag::TableRow => {
                    let header = builder.header;
                    builder.open(Kind::TableRow { header });
                    builder.cells = 0;
                }
                Tag::TableCell { .. } => {
                    if builder.cells > 0 {
                        builder.push(" │ ", Inline::Plain);
                    }
                    builder.cells += 1;
                }
                _ => {}
            },
            EventKind::End(kind) => match kind {
                TagKind::Paragraph | TagKind::Heading | TagKind::TableRow => builder.flush(),
                TagKind::Item => {
                    // An item with nothing in it still shows its marker.
                    if let Some(marker) = builder.marker.take() {
                        builder.open(Kind::Item {
                            depth: builder.lists.len(),
                            marker: Some(marker),
                        });
                    }
                    builder.flush();
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
                    builder.flush();
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
            // The space after the marker is in the text that follows it.
            EventKind::TaskMarker(done) => {
                builder.push(if done { "[x]" } else { "[ ]" }, Inline::Plain)
            }
            // GitHub's Markdown, which this parses, has no math.
            EventKind::Html { value, .. } => {
                let value = value.resolve(source).unwrap_or_default().to_owned();
                builder.push(&value, Inline::Plain);
            }
            _ => {}
        }
    }
    builder.flush();
    builder.blocks
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
        let blocks = parse(
            "```rust\nfn main() {}\n  let x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted\n\n---\n",
        );
        assert_eq!(
            plain(&blocks),
            [
                (Kind::Code, "fn main() {}\n  let x = 1;".into()),
                (Kind::TableRow { header: true }, "a │ b".into()),
                (Kind::TableRow { header: false }, "1 │ 2".into()),
                (Kind::Quote, "quoted".into()),
                (Kind::Rule, String::new()),
            ]
        );
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
    fn a_task_marker_is_followed_by_one_space() {
        let item = Kind::Item {
            depth: 1,
            marker: Some("•".into()),
        };
        assert_eq!(
            plain(&parse("- [x] done\n- [ ] todo\n")),
            [(item.clone(), "[x] done".into()), (item, "[ ] todo".into())]
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
