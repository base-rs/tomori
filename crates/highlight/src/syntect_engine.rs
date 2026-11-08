// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! syntect, for every other language its default syntaxes know: Sublime Text's syntax definitions, whose scopes, such as
//! `entity.name.function.c` or `keyword.operator.go`, are mapped onto [`TokenKind`]s by the innermost scope that one of
//! a few prefixes matches.
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::{Spans, TokenKind};

/// The scope prefixes that classify everything inside them, delimiters and escapes included, wherever they are in the
/// stack, the first found first.
const OUTER: usize = 3;

/// The scope prefixes recognised, each with the kind it maps onto: the [`OUTER`] first, then the rest, the most specific
/// first.
const SCOPES: &[(&str, TokenKind)] = &[
    ("comment", TokenKind::Comment),
    ("string", TokenKind::String),
    ("meta.annotation", TokenKind::Attribute),
    ("constant.numeric", TokenKind::Number),
    ("constant.character.escape", TokenKind::String),
    ("constant", TokenKind::Constant),
    ("support.constant", TokenKind::Constant),
    ("keyword.operator", TokenKind::Operator),
    ("keyword", TokenKind::Keyword),
    ("storage.type.function", TokenKind::Keyword),
    ("storage.type", TokenKind::Type),
    ("storage", TokenKind::Keyword),
    ("entity.name.function", TokenKind::Function),
    ("support.function", TokenKind::Function),
    ("variable.function", TokenKind::Function),
    ("entity.name.tag", TokenKind::Type),
    ("entity.other.attribute-name", TokenKind::Attribute),
    ("entity.name", TokenKind::Type),
    ("entity.other.inherited-class", TokenKind::Type),
    ("support.type", TokenKind::Type),
    ("support.class", TokenKind::Type),
    ("variable.language", TokenKind::Keyword),
    ("variable", TokenKind::Variable),
    ("punctuation", TokenKind::Punctuation),
    ("markup.heading", TokenKind::Keyword),
    ("markup.raw", TokenKind::String),
    ("markup.underline.link", TokenKind::String),
];

/// syntect's default syntaxes, loaded the first time they are needed and kept, with the scope prefixes made once.
fn syntaxes() -> &'static (SyntaxSet, Vec<(Scope, TokenKind)>) {
    static SYNTAXES: OnceLock<(SyntaxSet, Vec<(Scope, TokenKind)>)> = OnceLock::new();
    SYNTAXES.get_or_init(|| {
        let scopes = SCOPES
            .iter()
            .map(|(prefix, kind)| (Scope::new(prefix).expect("a valid scope"), *kind))
            .collect();
        (SyntaxSet::load_defaults_newlines(), scopes)
    })
}

/// Load the syntaxes now.
pub(crate) fn prepare() {
    syntaxes();
}

/// The syntax of the language `token` names, in lower case, by an extension or its name.
pub(crate) fn syntax(token: &str) -> Option<&'static SyntaxReference> {
    syntaxes().0.find_syntax_by_token(token)
}

/// What `stack` is: a comment, a string or an annotation, if it is inside one; otherwise what the innermost scope a
/// prefix matches says, or plain.
fn kind(stack: &ScopeStack, scopes: &[(Scope, TokenKind)]) -> TokenKind {
    let stack = stack.as_slice();
    let (outer, inner) = scopes.split_at(OUTER);
    let matching = |prefixes: &[(Scope, TokenKind)], scope: &Scope| {
        prefixes
            .iter()
            .find(|(prefix, _)| prefix.is_prefix_of(*scope))
            .map(|(_, kind)| *kind)
    };
    outer
        .iter()
        .find(|(prefix, _)| stack.iter().any(|scope| prefix.is_prefix_of(*scope)))
        .map(|(_, kind)| *kind)
        .or_else(|| stack.iter().rev().find_map(|scope| matching(inner, scope)))
        .unwrap_or_default()
}

/// Highlight `text` in the language `token` names, into `spans`, a line at a time, within `budget` of its syntax being
/// ready. Returns whether all of it was highlighted.
pub(crate) fn highlight(token: &str, text: &str, spans: &mut Spans<'_>, budget: Duration) -> bool {
    let (set, scopes) = syntaxes();
    let syntax = syntax(token).expect("a language syntect knows");
    // Made ready, as its contexts are read the first time it is used.
    let mut state = ParseState::new(syntax);
    let deadline = Instant::now() + budget;
    let mut stack = ScopeStack::new();
    let mut offset = 0;
    let mut complete = true;
    for line in LinesWithEndings::from(text) {
        if Instant::now() > deadline {
            return false;
        }
        // A line syntect cannot parse, such as one whose patterns backtrack too far, takes the kind of what it is in.
        let parsed = state.parse_line(line, set);
        complete &= parsed.is_ok();
        let mut from = 0;
        for (index, operation) in parsed.unwrap_or_default() {
            spans.push(offset + from, offset + index, kind(&stack, scopes));
            // An operation that does not apply, such as popping an empty stack, leaves the stack as it was.
            let _ = stack.apply(&operation);
            from = index;
        }
        spans.push(offset + from, offset + line.len(), kind(&stack, scopes));
        offset += line.len();
    }
    complete
}
