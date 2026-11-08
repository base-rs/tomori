// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Syntax highlighting for Tomori's code panels, which runs on the server and never on a headset: the text of a block of
//! code and its language in, spans of [`TokenKind`] out, in characters, without colours, which a client maps onto one
//! palette. See `design/projection.md`.
//!
//! - **tree-sitter** highlights the languages Tomori knows deeply, with each language's official grammar and its own
//!   highlights query: Rust, Ruby, JavaScript, TypeScript and TSX, Python, JSON, YAML, Markdown, Bash and TOML.
//! - **syntect** highlights every other language its default syntaxes know, such as C, Go, Java, HTML or SQL.
//! - Any other language, or none, is plain text: no spans.
//!
//! Both map what they find onto the same kinds, so that code reads alike whichever highlighter classified it. The text is
//! untrusted: only its first [`INPUT_BYTES`] are highlighted, within a time budget, [`BUDGET`], and at most [`SPANS`] spans are
//! returned, so that a pathological block costs little and leaves the rest plain.
use std::time::Duration;

pub use tomori_protocol::{Span, TokenKind};

mod syntect_engine;
mod tree_sitter_engine;

/// The most of a text highlighted, in bytes; the rest is plain.
pub const INPUT_BYTES: usize = 16 * 1024;

/// The most spans returned; the text past the last is plain.
pub const SPANS: usize = 4096;

/// How long highlighting one text may take, once its grammar or syntax is loaded, which happens once; whatever is not
/// highlighted by then is plain.
pub const BUDGET: Duration = Duration::from_millis(50);

/// Which highlighter classifies a language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// tree-sitter, with the language's official grammar.
    TreeSitter,
    /// syntect, with its default syntaxes.
    Syntect,
    /// Neither: the text is plain.
    Plain,
}

/// A text's spans, as one highlighter found them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlighted {
    pub engine: Engine,
    /// In order, none overlapping another, none empty, none plain, none across a line break, and adjacent spans of the
    /// same kind joined.
    pub spans: Vec<Span>,
    /// Whether all of the text was highlighted: not cut at [`INPUT_BYTES`], [`SPANS`] or [`BUDGET`], nor stopped by an
    /// error.
    pub complete: bool,
}

/// `language` as a block's info string names it, in lower case, trimmed.
fn token(language: &str) -> String {
    language.trim().to_ascii_lowercase()
}

/// Languages that are plain text by name, which syntect would otherwise find a syntax of plain text for.
const PLAIN: &[&str] = &["", "text", "txt", "plain", "plaintext"];

/// Which highlighter classifies `language`, as a block's info string names it, such as `rust`, `rs` or `TypeScript`.
pub fn engine(language: &str) -> Engine {
    let token = token(language);
    if tree_sitter_engine::Grammar::named(&token).is_some() {
        Engine::TreeSitter
    } else if !PLAIN.contains(&token.as_str()) && syntect_engine::syntax(&token).is_some() {
        Engine::Syntect
    } else {
        Engine::Plain
    }
}

/// Load what highlighting needs, the grammars' queries and syntect's syntaxes, ahead of the first text, so that it does
/// not take the first text's time.
pub fn prepare() {
    tree_sitter_engine::prepare();
    syntect_engine::prepare();
}

/// The spans of `text`, code in `language`, within [`BUDGET`].
pub fn highlight(language: &str, text: &str) -> Highlighted {
    highlight_within(language, text, BUDGET)
}

/// The spans of `text`, code in `language`, within `budget`, counted from when its grammar or syntax is loaded, which
/// [`prepare`] may do ahead of time.
pub fn highlight_within(language: &str, text: &str, budget: Duration) -> Highlighted {
    let token = token(language);
    let mut end = text.len().min(INPUT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let input = &text[..end];
    let mut spans = Spans::new(input);
    let engine = engine(&token);
    let finished = match engine {
        Engine::TreeSitter => tree_sitter_engine::highlight(&token, input, &mut spans, budget),
        Engine::Syntect => syntect_engine::highlight(&token, input, &mut spans, budget),
        Engine::Plain => true,
    };
    let complete = finished && end == text.len() && !spans.full;
    Highlighted {
        engine,
        spans: spans.finish(),
        complete,
    }
}

/// Spans gathered from a highlighter's byte ranges, in order, as characters.
pub(crate) struct Spans<'a> {
    text: &'a str,
    /// The byte up to which the text is counted, and how many characters it holds.
    counted: (usize, u32),
    spans: Vec<Span>,
    full: bool,
}

impl<'a> Spans<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            counted: (0, 0),
            spans: Vec::new(),
            full: false,
        }
    }

    /// The character at byte `index`, counting on from where the last was counted, for indices in order.
    fn character(&mut self, index: usize) -> u32 {
        let (from, count) = self.counted;
        let count = count + self.text[from..index].chars().count() as u32;
        self.counted = (index, count);
        count
    }

    /// The bytes from `start` to `end` of the text are of `kind`. Ranges come in order and do not overlap; one out of
    /// order, past the text or not on a character's boundaries is ignored, as is a plain one, and every one once
    /// [`SPANS`] are taken.
    pub(crate) fn push(&mut self, start: usize, end: usize, kind: TokenKind) {
        if self.full
            || kind == TokenKind::Plain
            || start >= end
            || start < self.counted.0
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
        {
            return;
        }
        // Each line's part on its own, without the spaces it starts or ends with, so that no span crosses a line break
        // or covers only spaces; joined to the span before it, if that is of the same kind and only spaces are
        // between them on the line.
        let mut from = start;
        for piece in self.text[start..end].split_inclusive('\n') {
            let content = piece.trim();
            if !content.is_empty() {
                let begin = from + piece.len() - piece.trim_start().len();
                // Where the last span ended, as the text is counted only as far as each span ends.
                let blank = self.text[self.counted.0..begin]
                    .chars()
                    .all(|c| c.is_whitespace() && c != '\n');
                let first = self.character(begin);
                let last = self.character(begin + content.len());
                if let Some(previous) = self.spans.last_mut()
                    && previous.kind == kind
                    && blank
                {
                    previous.length = last - previous.start;
                } else if self.spans.len() == SPANS {
                    self.full = true;
                    return;
                } else {
                    self.spans.push(Span::new(first, last - first, kind));
                }
            }
            from += piece.len();
        }
    }

    fn finish(self) -> Vec<Span> {
        self.spans
    }
}

#[cfg(test)]
mod tests;
