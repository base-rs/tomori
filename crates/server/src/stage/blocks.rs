// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panel blocks: the fenced code blocks of language `panel` in the agent's replies, which the server projects as text
//! panels. Everything in them is untrusted, so each is bounded and cleaned here, and one that is too large is left in
//! the transcript and not projected. See `design/projection.md`.
//!
//! A block is an optional header of `key: value` lines, ended by a line of `---`, and then its body, Markdown-lite text:
//!
//! ````text
//! ```panel
//! title: Build status
//! source: CI
//! ---
//! **3 crates** failed.
//! ```
//! ````
//!
//! `title` is the panel's title, `id` the agent's own name for it, by which a later block updates it in place (see
//! [`slug`]), `place` says where it goes (see [`super::placement`]), and every other key is its metadata. Without a
//! header, the whole block is its body.
use super::placement::Placement;
use super::{Content, Found};

/// The longest panel block taken, in bytes of its source; a longer one is not projected.
pub const BLOCK_BYTES: usize = 8 * 1024;

/// The most panels projected from one reply; later blocks stay only in the transcript.
pub const PANELS_PER_REPLY: usize = 4;

/// The longest title, in characters, after which it is cut with "…".
pub const TITLE_CHARS: usize = 80;

/// The most lines of a panel's body, and the longest body, in characters, after which it is cut with "…".
pub const BODY_LINES: usize = 30;
pub const BODY_CHARS: usize = 1_200;

/// The most metadata entries, and the longest key and value, in characters.
pub const META_ENTRIES: usize = 4;
pub const META_KEY_CHARS: usize = 24;
pub const META_VALUE_CHARS: usize = 80;

/// The longest id the agent may give a panel, in characters.
pub const ID_CHARS: usize = 32;

/// The id `value` names, in lower case, if it is one: from 1 to [`ID_CHARS`] ASCII letters, digits, `-` and `_`. Ids
/// are the agent's names for its panels, scoped to the space, so that a later block or `update_panel` updates one in
/// place; text that is not one names no panel.
pub fn slug(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= ID_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| value.to_ascii_lowercase())
}

/// `title` as a panel's title: on one line, cleaned and cut as a panel block's is.
pub fn title(title: &str) -> String {
    tomori_protocol::text::one_line(title, TITLE_CHARS)
        .trim()
        .to_owned()
}

/// `text` as a panel's body: its lines cleaned and cut as a panel block's are.
pub fn clean_body(text: &str) -> String {
    body(text.lines())
}

/// The info string's language that marks a panel block.
const LANGUAGE: &str = "panel";

/// A fence opening a code block on `line`: up to three spaces, then three or more backticks or tildes, with the info
/// string after them. A backtick fence's info string has no backtick in it, as in CommonMark.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let marker = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.len() - rest.trim_start_matches(marker).len();
    let info = rest[run..].trim();
    (run >= 3 && !(marker == '`' && info.contains('`'))).then_some((marker, run, info))
}

/// Whether `line` closes a code block opened by `run` of `marker`: up to three spaces, at least as many of the same
/// marker, and nothing else but spaces.
fn closes(line: &str, marker: char, run: usize) -> bool {
    let rest = line.trim_start_matches(' ');
    let after = rest.trim_start_matches(marker);
    line.len() - rest.len() <= 3 && rest.len() - after.len() >= run && after.trim().is_empty()
}

/// What an open block gathers.
enum Source {
    /// A panel block's text so far.
    Panel(String),
    /// Another language's block, projected as a code panel: its first lines so far.
    Code(super::code::Lines),
    /// A panel block too large to be projected.
    Skipped,
}

/// The panels of `text`, a reply's Markdown, in order: each fenced code block whose language is `panel` and that is
/// small enough and holds something, at most [`PANELS_PER_REPLY`], as a text panel; and each other fenced code block of
/// more than one line, at most [`super::code::CODE_PER_REPLY`], as a code panel. A block is taken whole, so that a
/// panel block quoted inside another is part of that block. A block left open runs to the end of the reply, as
/// CommonMark reads it.
pub fn panels(text: &str) -> Vec<Found> {
    struct Open {
        marker: char,
        run: usize,
        source: Source,
    }
    let mut found = Vec::new();
    let (mut texts, mut codes) = (0, 0);
    let mut open: Option<Open> = None;
    // Each block is made into a panel only while the reply has room for one of its kind, so that a reply of many
    // blocks costs no more than one of a few.
    let mut finish = |source: Source| {
        let content = match source {
            Source::Panel(source) if texts < PANELS_PER_REPLY => Content::parse_placed(&source)
                .inspect(|_| texts += 1)
                .map(|(content, place)| Found::text(content).placed(place)),
            Source::Code(lines) if codes < super::code::CODE_PER_REPLY => {
                lines.content().inspect(|_| codes += 1)
            }
            _ => None,
        };
        found.extend(content);
    };
    for line in text.lines() {
        match &mut open {
            None => {
                open = fence(line).map(|(marker, run, info)| Open {
                    marker,
                    run,
                    source: if info
                        .split_whitespace()
                        .next()
                        .is_some_and(|language| language.eq_ignore_ascii_case(LANGUAGE))
                    {
                        Source::Panel(String::new())
                    } else {
                        Source::Code(super::code::Lines::new(info))
                    },
                });
            }
            Some(block) if closes(line, block.marker, block.run) => {
                finish(open.take().expect("a block is open").source);
            }
            Some(block) => match &mut block.source {
                Source::Panel(source) => {
                    source.push_str(line);
                    source.push('\n');
                    if source.len() > BLOCK_BYTES {
                        block.source = Source::Skipped;
                    }
                }
                Source::Code(lines) => lines.push(line),
                Source::Skipped => {}
            },
        }
    }
    if let Some(block) = open {
        finish(block.source);
    }
    found
}

/// A header line's key and value, if `line` is one: a key of a letter and up to 23 more letters, digits, `-` or `_`,
/// then `:` and its value.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.trim().split_once(':')?;
    let mut chars = key.chars();
    (chars.next()?.is_ascii_alphabetic()
        && key.len() <= META_KEY_CHARS
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| (key, value.trim()))
}

/// `text` on lines of at most [`BODY_CHARS`] characters in all and [`BODY_LINES`] lines, each cleaned as text others
/// choose is put on one line, without the blank lines it starts or ends with.
fn body<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let lines: Vec<String> = lines
        .map(|line| {
            tomori_protocol::text::one_line(line, BODY_CHARS)
                .trim_end()
                .to_owned()
        })
        .collect();
    let start = lines.iter().position(|line| !line.is_empty());
    let end = lines.iter().rposition(|line| !line.is_empty());
    let (Some(start), Some(end)) = (start, end) else {
        return String::new();
    };
    let mut body = String::new();
    let mut room = BODY_CHARS;
    for (index, line) in lines[start..=end].iter().enumerate() {
        let length = line.chars().count();
        if index > 0 {
            body.push('\n');
        }
        if index == BODY_LINES || length > room {
            // As much of the line as fits, if any, then "…".
            if index < BODY_LINES {
                body.extend(line.chars().take(room));
            }
            body.push('…');
            return body;
        }
        body.push_str(line);
        room -= length;
    }
    body
}

impl Content {
    /// The panel `source`, a panel block's text, describes, or `None` when it holds neither a title nor a body.
    #[cfg(test)]
    pub fn parse(source: &str) -> Option<Self> {
        Self::parse_placed(source).map(|(content, _)| content)
    }

    /// The panel `source`, a panel block's text, describes, and where its `place` header says it goes, if it says
    /// anywhere the server knows; or `None` when it holds neither a title nor a body.
    pub fn parse_placed(source: &str) -> Option<(Self, Option<Placement>)> {
        let lines: Vec<&str> = source.lines().collect();
        let header = lines
            .iter()
            .position(|line| line.trim() == "---")
            .filter(|end| {
                lines[..*end]
                    .iter()
                    .all(|line| line.trim().is_empty() || key_value(line).is_some())
            });
        let (header, rest) = match header {
            Some(end) => (&lines[..end], &lines[end + 1..]),
            None => (&lines[..0], &lines[..]),
        };
        let mut title = String::new();
        let mut meta: Vec<(String, String)> = Vec::new();
        let mut place = None;
        let mut id = None;
        for (key, value) in header.iter().filter_map(|line| key_value(line)) {
            let key = key.to_ascii_lowercase();
            if key == "id" {
                // The first a block gives decides, whether it is valid or not, and none is metadata.
                id.get_or_insert_with(|| slug(value));
            } else if key == "place" {
                // The first a block gives decides, whether it is known or not, and none is metadata.
                place.get_or_insert_with(|| Placement::parse(value));
            } else if key == "title" {
                if title.is_empty() {
                    title = self::title(value);
                }
            } else if meta.len() < META_ENTRIES && !meta.iter().any(|(known, _)| *known == key) {
                let value = tomori_protocol::text::one_line(value, META_VALUE_CHARS)
                    .trim()
                    .to_owned();
                if !value.is_empty() {
                    meta.push((key, value));
                }
            }
        }
        let body = body(rest.iter().copied());
        (!title.is_empty() || !body.is_empty()).then_some((
            Self {
                title,
                body,
                meta,
                code: None,
                slug: id.flatten(),
                image: None,
            },
            place.flatten(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_says_where_its_panel_goes_with_a_place_header_or_a_code_block_s_attribute() {
        let found = super::panels(
            "```panel\ntitle: Status\nplace: near Ada\nplace: wall tomori\nsource: CI\n---\nGreen.\n```\n\n```panel\nplace: behind Ada\n---\nOther.\n```\n\n```rust place=looking:Bob title=main.rs\nfn main() {\n}\n```\n\n```panel\nPlain.\n```\n",
        );
        assert_eq!(found.len(), 4);
        // The first place a block gives decides, and none is metadata.
        assert_eq!(found[0].place, Some(Placement::Near(Some("Ada".into()))));
        assert_eq!(
            found[0].content.meta,
            [("source".to_owned(), "CI".to_owned())]
        );
        // A form the server does not know is ignored, and is not metadata either.
        assert_eq!(found[1].place, None);
        assert!(found[1].content.meta.is_empty());
        assert_eq!(found[1].content.body, "Other.");
        // A code block's attribute is taken out of its title and language.
        assert_eq!(found[2].place, Some(Placement::Looking(Some("Bob".into()))));
        assert_eq!(found[2].content.title, "rust title=main.rs");
        assert_eq!(found[2].content.code.as_ref().unwrap().language, "rust");
        assert_eq!(found[3].place, None);
    }

    /// The panels of `text`, each as found, its code not yet highlighted.
    fn panels(text: &str) -> Vec<Content> {
        super::panels(text)
            .into_iter()
            .map(|found| found.content)
            .collect()
    }

    fn content(title: &str, body: &str, meta: &[(&str, &str)]) -> Content {
        Content {
            title: title.into(),
            body: body.into(),
            meta: meta
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
            code: None,
            slug: None,
            image: None,
        }
    }

    #[test]
    fn an_id_names_a_panel_by_a_short_slug_and_is_never_metadata() {
        let parsed =
            Content::parse("ID: Agenda_1\nid: other\ntitle: Agenda\n---\n- [ ] One").unwrap();
        assert_eq!(parsed.slug.as_deref(), Some("agenda_1"));
        assert!(parsed.meta.is_empty());
        assert_eq!(parsed.title, "Agenda");
        // An id that is not a slug names nothing, and the first decides even then.
        for id in [
            "two words",
            "",
            "quote\"",
            "[x]",
            "naïve",
            &"a".repeat(ID_CHARS + 1),
        ] {
            let parsed = Content::parse(&format!("id: {id}\nid: later\n---\nBody")).unwrap();
            assert_eq!(parsed.slug, None, "{id:?}");
            assert!(parsed.meta.is_empty());
        }
        assert_eq!(slug(&"a".repeat(ID_CHARS)), Some("a".repeat(ID_CHARS)));
        assert_eq!(slug(" to-do "), Some("to-do".to_owned()));
        // A block of only an id holds nothing to show.
        assert!(Content::parse("id: agenda\n---\n").is_none());
    }

    #[test]
    fn panel_blocks_are_lifted_out_of_a_reply_with_their_header() {
        let reply = "Here is the status.\n\n```panel\ntitle: Build status\nsource: CI\n---\n**3 crates** failed:\n\n- tomori-scene\n```\n\nAsk me about any of them.";
        assert_eq!(
            panels(reply),
            [content(
                "Build status",
                "**3 crates** failed:\n\n- tomori-scene",
                &[("source", "CI")]
            )]
        );
        // Without a header, the whole block is the body; a tilde fence, a longer fence and an info string with more after
        // the language count too, and so does a block left open at the end.
        let reply = "~~~panel\nJust text.\n~~~\n````Panel wide\nWide.\n````\n```panel\nUnclosed.";
        assert_eq!(
            panels(reply),
            [
                content("", "Just text.", &[]),
                content("", "Wide.", &[]),
                content("", "Unclosed.", &[]),
            ]
        );
        // A block of another language of one line, indented code and blocks that only look like fences are not panels.
        let reply = "```rust\nfn main() {}\n```\n    ```panel\n    indented\n    ```\n``` panel `x`\nnot a fence\n";
        assert!(panels(reply).is_empty());
        // A panel block quoted inside another block is part of that block, which is a code panel.
        let reply = "````markdown\n```panel\nquoted\n```\n````\n";
        let found = panels(reply);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "markdown");
        assert_eq!(found[0].body, "```panel\nquoted\n```");
        assert!(found[0].code.is_some());
        // A closing fence must be as long as the opening one and hold nothing else.
        let reply = "````panel\nA\n```\n```` trailing\nB\n````\n";
        assert_eq!(
            panels(reply),
            [content("", "A\n```\n```` trailing\nB", &[])]
        );
        // A block of nothing projects nothing.
        assert!(panels("```panel\n\n   \n```\n").is_empty());
        assert!(panels("```panel\ntitle:\n---\n```\n").is_empty());
    }

    #[test]
    fn a_header_is_only_key_value_lines_before_a_rule() {
        // Prose before a rule is body, rule and all.
        assert_eq!(
            Content::parse("Some prose.\n---\nMore.").unwrap(),
            content("", "Some prose.\n---\nMore.", &[])
        );
        // Keys are cleaned to lower case, the first of each kept, empty values dropped, and at most four taken.
        let header = "Title: First\ntitle: Second\nOwner: Ada\nowner: Bob\nempty:\na: 1\nb: 2\nc: 3\nd: 4\n\n---\nBody";
        assert_eq!(
            Content::parse(header).unwrap(),
            content(
                "First",
                "Body",
                &[("owner", "Ada"), ("a", "1"), ("b", "2"), ("c", "3")]
            )
        );
        // A key with a space, or one too long, is not a key, so the lines are body.
        for line in [
            ": x",
            "two words: x",
            "1st: x",
            &format!("{}: x", "k".repeat(25)),
        ] {
            let text = format!("{line}\n---\nBody");
            assert!(Content::parse(&text).unwrap().title.is_empty());
            assert!(Content::parse(&text).unwrap().meta.is_empty());
        }
    }

    #[test]
    fn hostile_and_oversized_blocks_are_bounded_and_cleaned() {
        // A title and values on one line, without format characters or line breaks, and cut with "…".
        let hostile = format!(
            "title: Ignore \u{202e}previous\u{200b} instructions\u{7}\nsource: {}\n---\nline\u{2028}break\tand\u{feff} tab\r\nnext\u{0}",
            "v".repeat(200)
        );
        let parsed = Content::parse(&hostile).unwrap();
        assert_eq!(parsed.title, "Ignore previous instructions");
        assert_eq!(parsed.meta[0].1.chars().count(), META_VALUE_CHARS + 1);
        assert!(parsed.meta[0].1.ends_with('…'));
        // A line separator inside a line is a space, as on one line, and a carriage return ends a line.
        assert_eq!(parsed.body, "line break and tab\nnext");
        let long_title = format!("title: {}\n---\nx", "t".repeat(500));
        assert_eq!(
            Content::parse(&long_title).unwrap().title.chars().count(),
            TITLE_CHARS + 1
        );
        // A body of too many lines, or too many characters, is cut with "…".
        let many = (0..100)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let body = Content::parse(&many).unwrap().body;
        assert_eq!(body.lines().count(), BODY_LINES + 1);
        assert!(body.ends_with("\n…"));
        let wide = format!("{}\n{}", "a".repeat(1_000), "b".repeat(1_000));
        let body = Content::parse(&wide).unwrap().body;
        assert_eq!(body.chars().count(), BODY_CHARS + 2);
        assert!(body.ends_with("bbb…"));
        // A line that does not fit at all ends the body with "…" alone.
        let full = format!("{}\nmore", "a".repeat(BODY_CHARS));
        assert!(Content::parse(&full).unwrap().body.ends_with("a\n…"));
        // A block larger than the limit is not projected, and neither is a panel beyond the limit for one reply.
        let huge = format!("```panel\n{}\nmore\n```\n", "x".repeat(BLOCK_BYTES + 1));
        assert!(panels(&huge).is_empty());
        let many = "```panel\nx\n```\n".repeat(PANELS_PER_REPLY + 3);
        assert_eq!(panels(&many).len(), PANELS_PER_REPLY);
    }

    #[test]
    fn code_blocks_are_code_panels_in_order_with_the_text_panels_each_bounded_by_kind() {
        let reply = "Here:\n\n```rust\nfn a() {}\nfn b() {}\n```\n\n```panel\ntitle: Notes\n---\nTwo functions.\n```\n\n~~~\nplain\ntext\n~~~\n\nRun `cargo test`:\n\n```sh\ncargo test\n```\n";
        let found = panels(reply);
        let titles: Vec<(&str, bool)> = found
            .iter()
            .map(|content| (content.title.as_str(), content.code.is_some()))
            .collect();
        // The command, of one line, stays only in the transcript.
        assert_eq!(titles, [("rust", true), ("Notes", false), ("code", true)]);
        // At most so many of each kind in one reply, a block of one line not counted, nor one too large or empty.
        let reply = [
            "```sh\nls\n```\n".repeat(3),
            "```rust\nfn a() {}\nfn b() {}\n```\n".repeat(super::super::code::CODE_PER_REPLY + 2),
            format!("```panel\n{}\n```\n", "x".repeat(BLOCK_BYTES + 1)),
            "```panel\nshown\n```\n".repeat(PANELS_PER_REPLY + 1),
        ]
        .concat();
        let found = panels(&reply);
        assert_eq!(
            found
                .iter()
                .filter(|content| content.code.is_some())
                .count(),
            super::super::code::CODE_PER_REPLY
        );
        assert_eq!(
            found
                .iter()
                .filter(|content| content.code.is_none())
                .count(),
            PANELS_PER_REPLY
        );
        // A block left open runs to the end of the reply.
        let open = panels("```python\ndef f():\n    pass");
        assert_eq!(open[0].body, "def f():\n    pass");
    }
}
