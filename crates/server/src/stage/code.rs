// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Code blocks: the fenced code blocks of any language but `panel` in the agent's replies, which the server projects as
//! code panels, titled with their info string and highlighted here, on the server, never on a headset. Everything in
//! them is untrusted, so each is bounded and cleaned: only a block's first [`CODE_LINES`] lines are kept, cut where they
//! reach [`CODE_CHARS`] characters together, with at most [`CODE_SPANS`] spans. A projection that would not fit in one
//! frame sends some of them [`short`], as [`SHORT`] bounds them. See `design/projection.md`.
//!
//! A block of a single line, such as a command, is left in the transcript and not projected: it reads as well there,
//! and a panel for each would crowd the walls and replace the code panels of the reply before.
//!
//! A block is read into a plain code panel and its [`Source`], which is highlighted later, outside the hub's lock, since
//! highlighting takes far longer than reading; see `crate::highlighting`.
use tomori_protocol::{CODE_CHARS, CODE_LINES, CODE_SPANS, Code, Panel, Span};

use super::blocks::{META_KEY_CHARS, TITLE_CHARS};
use super::{Content, Found};

/// The most code panels projected from one reply; later blocks stay only in the transcript.
pub const CODE_PER_REPLY: usize = 4;

/// The most characters of a block's first lines highlighted, together: more than the [`CODE_CHARS`] shown, so that
/// lines cut where they are shown are still highlighted as they continue, as a string that closes after the cut.
const HIGHLIGHTED_CHARS: usize = 8_080;
const _: () = assert!(HIGHLIGHTED_CHARS > CODE_CHARS);
// The highlighter takes them all, with the breaks between them, but for characters of more than two bytes, whose code
// past its bound is plain.
#[cfg(feature = "highlight")]
const _: () = assert!(2 * HIGHLIGHTED_CHARS + CODE_LINES <= tomori_highlight::INPUT_BYTES);

/// How much of a block's code a panel holds: its first `lines`, cut where they reach `chars` characters together, with
/// at most `spans` spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub lines: usize,
    pub chars: usize,
    pub spans: usize,
}

/// What a code panel holds, and is sent where its projection fits in one frame.
pub const FULL: Bound = Bound {
    lines: CODE_LINES,
    chars: CODE_CHARS,
    spans: CODE_SPANS,
};

/// What a code panel is sent where its projection, with every code panel whole, would not fit in one frame: as much as
/// a slot shows, its first 16 lines and 720 characters, with at most 256 spans, as code panels held before they
/// scrolled, so that a projection of them all fits.
pub const SHORT: Bound = Bound {
    lines: 16,
    chars: 720,
    spans: 256,
};

/// The longest info string kept, in characters, more than a title shows.
const INFO_CHARS: usize = 2 * TITLE_CHARS;

/// How far apart tab stops are, in columns.
const TAB: usize = 4;

/// The title of a code panel whose block names no language.
const UNTITLED: &str = "code";

/// A code block's info string and its first lines, gathered as the block is read, bounded however long it is.
#[derive(Debug)]
pub struct Lines {
    info: String,
    /// Where its info string's `place=` attribute says it goes, taken out of `info`.
    place: Option<super::placement::Placement>,
    /// Its first [`CODE_LINES`] lines, from the first that is not blank, cut where they reach [`HIGHLIGHTED_CHARS`]
    /// together, and how many characters they have.
    kept: Vec<String>,
    characters: usize,
    /// How many lines it has, from the first that is not blank, and how many of the last are blank.
    total: u32,
    blank: u32,
}

impl Lines {
    /// A block whose info string is `info`.
    pub fn new(info: &str) -> Self {
        let (info, place) = super::placement::Placement::from_info(info);
        Self {
            info: info.chars().take(INFO_CHARS).collect(),
            place,
            kept: Vec::new(),
            characters: 0,
            total: 0,
            blank: 0,
        }
    }

    /// The block's next line, `line`.
    pub fn push(&mut self, line: &str) {
        let blank = line.trim().is_empty();
        if self.total == 0 && blank {
            return;
        }
        self.total = self.total.saturating_add(1);
        self.blank = if blank {
            self.blank.saturating_add(1)
        } else {
            0
        };
        let left = HIGHLIGHTED_CHARS - self.characters;
        if self.kept.len() < CODE_LINES && left > 0 {
            let kept: String = line.chars().take(left).collect();
            self.characters += kept.chars().count();
            self.kept.push(kept);
        }
    }

    /// The code panel the block makes, plain, with its code to highlight, or `None` for a block of one line or none,
    /// without its blank lines at either end.
    pub fn content(self) -> Option<Found> {
        let total = self.total - self.blank;
        if total <= 1 {
            return None;
        }
        let lines: Vec<String> = self
            .kept
            .iter()
            .take(total as usize)
            .map(|line| clean(line))
            .collect();
        let title = tomori_protocol::text::one_line(&self.info, TITLE_CHARS)
            .trim()
            .to_owned();
        let language = self
            .info
            .split_whitespace()
            .next()
            .map(|language| {
                tomori_protocol::text::one_line(language, META_KEY_CHARS).to_lowercase()
            })
            .unwrap_or_default();
        let (body, _) = shown(&lines, Vec::new(), FULL);
        let content = Content {
            title: if title.is_empty() {
                UNTITLED.to_owned()
            } else {
                title
            },
            body,
            meta: Vec::new(),
            code: Some(Code::new(language.clone(), total)),
        };
        Some(Found::code(content, Source { language, lines }).placed(self.place))
    }
}

/// A code panel's code, as it is highlighted: its language, and its first lines, cleaned, cut where they reach
/// [`HIGHLIGHTED_CHARS`] together, longer than they are shown.
#[derive(Clone, Debug)]
pub struct Source {
    language: String,
    lines: Vec<String>,
}

impl Source {
    /// The language and the text `highlight` is given.
    pub fn code(&self) -> (&str, String) {
        (&self.language, self.lines.join("\n"))
    }

    /// Its spans over what its panel shows, from `found`, the spans a highlighter found over its text.
    pub fn shown(&self, found: Vec<Span>) -> Vec<Span> {
        shown(&self.lines, found, FULL).1
    }
}

/// `panel`, a code panel, as [`SHORT`] bounds it: its first lines and the spans over them. Its block's length is kept, so
/// that a client says how many lines it was not sent. Returns whether it was made shorter.
pub fn short(panel: &mut Panel) -> bool {
    let Some(code) = panel.code.as_mut() else {
        return false;
    };
    let lines: Vec<String> = panel.body.split('\n').map(str::to_owned).collect();
    let found = std::mem::take(&mut code.spans);
    let count = found.len();
    let (body, spans) = shown(&lines, found, SHORT);
    let shorter = body != panel.body || spans.len() < count;
    panel.body = body;
    code.spans = spans;
    shorter
}

/// `line` as a code panel shows it: tabs expanded to the next stop, format characters removed, since they reorder or
/// hide text, and control characters and every other kind of space a plain space, without the spaces it ends with.
fn clean(line: &str) -> String {
    let mut cleaned = String::with_capacity(line.len());
    let mut column = 0;
    for c in line.chars() {
        match c {
            '\t' => {
                let stop = (column / TAB + 1) * TAB;
                cleaned.extend(std::iter::repeat_n(' ', stop - column));
                column = stop;
            }
            c if tomori_protocol::text::is_format(c) => {}
            c => {
                cleaned.push(if c.is_control() || c.is_whitespace() {
                    ' '
                } else {
                    c
                });
                column += 1;
            }
        }
    }
    cleaned.truncate(cleaned.trim_end().len());
    cleaned
}

/// `lines` as a code panel holds them, as `bound` bounds them: its first lines, cut with "…" where they reach its
/// characters together, without the lines after, and `found`, their spans over the lines joined, as spans over what is
/// held, at most its spans: a span's part past the cut is dropped.
fn shown(lines: &[String], found: Vec<Span>, bound: Bound) -> (String, Vec<Span>) {
    let mut body = String::new();
    let mut spans = Vec::new();
    let mut found = found.into_iter().peekable();
    // Where the line starts in the lines joined, and in what is shown, and how many characters are left to show.
    let (mut from, mut at) = (0_u32, 0_u32);
    let mut left = bound.chars as u32;
    for (index, line) in lines.iter().take(bound.lines).enumerate() {
        // A line after those that took every character could not be shown either.
        if left == 0 {
            break;
        }
        if index > 0 {
            body.push('\n');
        }
        let length = line.chars().count() as u32;
        let kept = length.min(left);
        left -= kept;
        body.extend(line.chars().take(kept as usize));
        let cut = length > kept;
        if cut {
            body.push('…');
        }
        // The highlighters' spans never cross a line break, so each is inside one line.
        while let Some(span) = found.next_if(|span| span.start < from + length) {
            let start = span.start.saturating_sub(from);
            let end = (start + span.length).min(kept);
            if start < end && spans.len() < bound.spans {
                spans.push(Span::new(at + start, end - start, span.kind));
            }
        }
        if cut {
            break;
        }
        from += length + 1;
        at += kept + 1;
    }
    (body, spans)
}

#[cfg(test)]
mod tests;
