// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! tree-sitter, for the languages Tomori knows deeply: each language's official grammar crate and its own highlights
//! query, whose capture names, such as `function.method` or `punctuation.bracket`, are mapped onto [`TokenKind`]s by
//! their most specific recognised prefix.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};
use tree_sitter_language::LanguageFn;

use crate::{Spans, TokenKind};

/// The capture names recognised, each with the kind it maps onto. A capture is classified by the longest of these that
/// is a prefix of its name, part by part, so that `function.method.builtin` is a function; one with none, such as
/// `text.emphasis`, is plain.
const NAMES: &[(&str, TokenKind)] = &[
    ("attribute", TokenKind::Attribute),
    ("boolean", TokenKind::Constant),
    ("comment", TokenKind::Comment),
    ("constant", TokenKind::Constant),
    ("constructor", TokenKind::Type),
    ("embedded", TokenKind::Plain),
    ("escape", TokenKind::String),
    ("function", TokenKind::Function),
    ("keyword", TokenKind::Keyword),
    ("label", TokenKind::Attribute),
    ("none", TokenKind::Plain),
    ("number", TokenKind::Number),
    ("operator", TokenKind::Operator),
    ("property", TokenKind::Variable),
    ("punctuation", TokenKind::Punctuation),
    ("string", TokenKind::String),
    ("string.special.key", TokenKind::Variable),
    ("string.special.symbol", TokenKind::Constant),
    ("tag", TokenKind::Type),
    ("text.literal", TokenKind::String),
    ("text.reference", TokenKind::Function),
    ("text.title", TokenKind::Keyword),
    ("text.uri", TokenKind::String),
    ("type", TokenKind::Type),
    ("variable", TokenKind::Variable),
];

/// A grammar Tomori highlights with tree-sitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grammar {
    Rust,
    Ruby,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Json,
    Yaml,
    Markdown,
    Bash,
    Toml,
}

impl Grammar {
    const ALL: [Grammar; 11] = [
        Grammar::Rust,
        Grammar::Ruby,
        Grammar::JavaScript,
        Grammar::TypeScript,
        Grammar::Tsx,
        Grammar::Python,
        Grammar::Json,
        Grammar::Yaml,
        Grammar::Markdown,
        Grammar::Bash,
        Grammar::Toml,
    ];

    /// The grammar of the language `token` names, in lower case, by its name or a common alias or extension, as an info
    /// string or a Markdown fence names it.
    pub(crate) fn named(token: &str) -> Option<Self> {
        Some(match token {
            "rust" | "rs" => Self::Rust,
            "ruby" | "rb" => Self::Ruby,
            "javascript" | "js" | "jsx" | "mjs" | "cjs" | "node" => Self::JavaScript,
            "typescript" | "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "python" | "py" | "python3" => Self::Python,
            "json" | "jsonc" | "json5" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "markdown" | "md" => Self::Markdown,
            "bash" | "sh" | "shell" | "zsh" => Self::Bash,
            "toml" => Self::Toml,
            _ => return None,
        })
    }

    /// Its highlighting configuration, made the first time it is needed and kept.
    pub(crate) fn configuration(self) -> &'static HighlightConfiguration {
        static CONFIGURATIONS: [OnceLock<HighlightConfiguration>; 11] =
            [const { OnceLock::new() }; 11];
        CONFIGURATIONS[self as usize].get_or_init(|| self.configure())
    }

    fn configure(self) -> HighlightConfiguration {
        use tree_sitter_javascript as javascript;
        use tree_sitter_typescript as typescript;
        let (language, highlights, injections, locals): (LanguageFn, &[&str], &str, &str) =
            match self {
                Self::Rust => (
                    tree_sitter_rust::LANGUAGE,
                    &[tree_sitter_rust::HIGHLIGHTS_QUERY],
                    tree_sitter_rust::INJECTIONS_QUERY,
                    "",
                ),
                Self::Ruby => (
                    tree_sitter_ruby::LANGUAGE,
                    &[tree_sitter_ruby::HIGHLIGHTS_QUERY],
                    "",
                    tree_sitter_ruby::LOCALS_QUERY,
                ),
                Self::JavaScript => (
                    javascript::LANGUAGE,
                    &[javascript::HIGHLIGHT_QUERY, javascript::JSX_HIGHLIGHT_QUERY],
                    javascript::INJECTIONS_QUERY,
                    javascript::LOCALS_QUERY,
                ),
                // TypeScript's queries extend JavaScript's, which come first, as its grammar's own configuration lists them.
                Self::TypeScript => (
                    typescript::LANGUAGE_TYPESCRIPT,
                    &[javascript::HIGHLIGHT_QUERY, typescript::HIGHLIGHTS_QUERY],
                    javascript::INJECTIONS_QUERY,
                    typescript::LOCALS_QUERY,
                ),
                Self::Tsx => (
                    typescript::LANGUAGE_TSX,
                    &[
                        javascript::HIGHLIGHT_QUERY,
                        javascript::JSX_HIGHLIGHT_QUERY,
                        typescript::HIGHLIGHTS_QUERY,
                    ],
                    javascript::INJECTIONS_QUERY,
                    typescript::LOCALS_QUERY,
                ),
                Self::Python => (
                    tree_sitter_python::LANGUAGE,
                    &[tree_sitter_python::HIGHLIGHTS_QUERY],
                    "",
                    "",
                ),
                Self::Json => (
                    tree_sitter_json::LANGUAGE,
                    &[tree_sitter_json::HIGHLIGHTS_QUERY],
                    "",
                    "",
                ),
                Self::Yaml => (
                    tree_sitter_yaml::LANGUAGE,
                    &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
                    "",
                    "",
                ),
                // The block grammar alone: headings, lists, quotes, rules and fences. tree-sitter-highlight does not yet
                // layer the inline grammar, or a fenced block's own language, correctly over it, so neither is injected.
                Self::Markdown => (
                    tree_sitter_md::LANGUAGE,
                    &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
                    "",
                    "",
                ),
                Self::Bash => (
                    tree_sitter_bash::LANGUAGE,
                    &[tree_sitter_bash::HIGHLIGHT_QUERY],
                    "",
                    "",
                ),
                Self::Toml => (
                    tree_sitter_toml_ng::LANGUAGE,
                    &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
                    "",
                    "",
                ),
            };
        let mut configuration = HighlightConfiguration::new(
            language.into(),
            format!("{self:?}"),
            &highlights.join("\n"),
            injections,
            locals,
        )
        .expect("a grammar's own queries compile against it");
        configuration.configure(&NAMES.iter().map(|(name, _)| *name).collect::<Vec<_>>());
        configuration
    }
}

/// Make every grammar's configuration now.
pub(crate) fn prepare() {
    for grammar in Grammar::ALL {
        grammar.configuration();
    }
}

/// Highlight `text` in the language `token` names, into `spans`, within `budget` of its grammar being ready. Returns
/// whether all of it was highlighted.
pub(crate) fn highlight(token: &str, text: &str, spans: &mut Spans<'_>, budget: Duration) -> bool {
    let grammar = Grammar::named(token).expect("a language tree-sitter knows");
    let configuration = grammar.configuration();
    let deadline = Instant::now() + budget;
    let mut highlighter = Highlighter::new();
    // Set once the deadline passes, which tree-sitter checks as it parses an injected layer and as it iterates, and
    // then ends the events with an error.
    let cancelled = AtomicUsize::new(0);
    // A Markdown block's inline text, and a fenced block of a language it knows, are highlighted by their own grammars.
    let injected =
        |name: &str| Grammar::named(&name.to_ascii_lowercase()).map(Grammar::configuration);
    let events = highlighter
        .highlight(
            configuration,
            text.as_bytes(),
            None,
            Some(&cancelled),
            injected,
        )
        .into_iter()
        .flatten()
        .map_while(Result::ok);
    let mut kinds: Vec<TokenKind> = Vec::new();
    for event in events {
        if Instant::now() > deadline {
            cancelled.store(1, Ordering::Relaxed);
        }
        match event {
            HighlightEvent::HighlightStart(highlight) => kinds.push(NAMES[highlight.0].1),
            HighlightEvent::HighlightEnd => {
                kinds.pop();
            }
            HighlightEvent::Source { start, end } => {
                spans.push(start, end, kinds.last().copied().unwrap_or_default());
            }
        }
    }
    cancelled.load(Ordering::Relaxed) == 0
}
