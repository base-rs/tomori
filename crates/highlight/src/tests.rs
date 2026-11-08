// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::{Duration, Instant};

use super::*;
use TokenKind::{
    Attribute, Comment, Constant, Function, Keyword, Number, Operator, Plain, Punctuation,
    String as Text, Type, Variable,
};

/// Whether `highlighted`'s spans are as [`Highlighted::spans`] promises for `text`: in order, none overlapping another,
/// none empty or plain, inside the text, none across a line break or starting or ending with a space, and at most
/// [`SPANS`].
fn assert_valid(text: &str, highlighted: &Highlighted) {
    let chars: Vec<char> = text.chars().collect();
    let mut end = 0;
    assert!(highlighted.spans.len() <= SPANS);
    for span in &highlighted.spans {
        assert!(span.length > 0 && u64::from(span.start) >= end, "{span:?}");
        assert!(span.end() <= chars.len() as u64, "{span:?}");
        assert_ne!(span.kind, Plain);
        let covered = &chars[span.start as usize..span.end() as usize];
        assert!(!covered.contains(&'\n'), "{span:?}");
        assert!(!covered[0].is_whitespace() && !covered[covered.len() - 1].is_whitespace());
        end = span.end();
    }
}

/// `text`'s spans in `language`, as the text each covers and its kind, checked valid, and by which highlighter.
fn tokens(language: &str, text: &str) -> (Engine, Vec<(String, TokenKind)>) {
    let chars: Vec<char> = text.chars().collect();
    // Generously, for a machine busy with other tests.
    let highlighted = highlight_within(language, text, Duration::from_secs(10));
    assert!(highlighted.complete, "{language}");
    assert_valid(text, &highlighted);
    let tokens = highlighted
        .spans
        .iter()
        .map(|span| {
            (
                chars[span.start as usize..span.end() as usize]
                    .iter()
                    .collect(),
                span.kind,
            )
        })
        .collect();
    (highlighted.engine, tokens)
}

/// That `text` in `language` is highlighted by `engine`, with each of `expected` among its spans.
fn assert_tokens(language: &str, engine: Engine, text: &str, expected: &[(&str, TokenKind)]) {
    let (found, tokens) = tokens(language, text);
    assert_eq!(found, engine, "{language}");
    for (token, kind) in expected {
        assert!(
            tokens.contains(&((*token).to_owned(), *kind)),
            "{language}: {token:?} is not {kind:?} in {tokens:?}"
        );
    }
}

#[test]
fn tree_sitter_classifies_each_of_its_languages_onto_the_shared_kinds() {
    use Engine::TreeSitter;
    assert_tokens(
        "rust",
        TreeSitter,
        "#[derive(Debug)]\nstruct Point { x: u32 }\nfn main() {\n    let p = Point { x: 42 }; // the origin\n    println!(\"{}\", p.x);\n}",
        &[
            ("derive", Attribute),
            ("struct", Keyword),
            ("Point", Type),
            ("u32", Type),
            ("fn", Keyword),
            ("main", Function),
            ("let", Keyword),
            ("42", Constant),
            ("// the origin", Comment),
            ("println!", Function),
            ("\"{}\"", Text),
            ("x", Variable),
            ("{", Punctuation),
        ],
    );
    assert_tokens(
        "ruby",
        TreeSitter,
        "class Greeter < Base\n  def hello(name)\n    puts \"hi #{name}\" # greet\n    :done\n  end\nend",
        &[
            ("class", Keyword),
            ("Greeter", Type),
            ("def", Keyword),
            ("hello", Function),
            ("name", Variable),
            ("\"hi", Text),
            ("# greet", Comment),
            (":done", Constant),
            ("end", Keyword),
        ],
    );
    assert_tokens(
        "javascript",
        TreeSitter,
        "const total = 1 + 2;\nfunction add(a) { return a + 'b'; } // sum\nclass A extends B {}\nconst e = <div className=\"x\">hi</div>;",
        &[
            ("const", Keyword),
            ("total", Variable),
            ("=", Operator),
            ("1", Number),
            ("add", Function),
            ("'b'", Text),
            ("// sum", Comment),
            ("A", Type),
            (";", Punctuation),
            ("div", Type),
            ("className", Attribute),
        ],
    );
    assert_tokens(
        "typescript",
        TreeSitter,
        "interface Shape { area: number }\nconst name: string = \"s\";\nfunction f(a: Shape): void {}",
        &[
            ("interface", Keyword),
            ("Shape", Type),
            ("number", Type),
            ("area", Variable),
            ("string", Type),
            ("\"s\"", Text),
            ("f", Function),
        ],
    );
    assert_tokens(
        "tsx",
        TreeSitter,
        "const A = (p: Props) => <div className=\"x\">hi</div>;",
        &[
            ("Props", Type),
            ("div", Type),
            ("className", Attribute),
            ("\"x\"", Text),
        ],
    );
    assert_tokens(
        "python",
        TreeSitter,
        "@cache\ndef area(r):\n    return 3.14 * r ** 2  # pi r squared\nprint(None, \"x\")",
        &[
            ("@cache", Function),
            ("def", Keyword),
            ("area", Function),
            ("r", Variable),
            ("3.14", Number),
            ("*", Operator),
            ("# pi r squared", Comment),
            ("None", Constant),
            ("\"x\"", Text),
        ],
    );
    assert_tokens(
        "json",
        TreeSitter,
        "{\"name\": \"tomori\", \"size\": 3, \"ok\": [true, null]}",
        &[
            ("\"tomori\"", Text),
            ("3", Number),
            ("true", Constant),
            ("null", Constant),
            ("\"name\"", Text),
        ],
    );
    assert_tokens(
        "yaml",
        TreeSitter,
        "name: tomori\nsizes:\n  - 3\n  - true # yes",
        &[
            ("name", Variable),
            ("tomori", Text),
            (":", Punctuation),
            ("3", Number),
            ("true", Constant),
            ("# yes", Comment),
        ],
    );
    assert_tokens(
        "markdown",
        TreeSitter,
        "# Title\n\n- item\n\n```rust\nfn main() {}\n```",
        &[
            ("#", Punctuation),
            ("Title", Keyword),
            ("-", Punctuation),
            ("```", Punctuation),
            ("rust", Text),
        ],
    );
    assert_tokens(
        "bash",
        TreeSitter,
        "echo \"hi $USER\" | grep -c x # count\nif [ -f x ]; then ls; fi",
        &[
            ("echo", Function),
            ("USER", Variable),
            ("|", Operator),
            ("-c", Constant),
            ("# count", Comment),
            ("if", Keyword),
        ],
    );
    assert_tokens(
        "toml",
        TreeSitter,
        "[package]\nname = \"tomori\"\nversion = 1 # one\nflag = true",
        &[
            ("package", Type),
            ("[", Punctuation),
            ("name", Variable),
            ("=", Operator),
            ("\"tomori\"", Text),
            ("1", Number),
            ("# one", Comment),
            ("true", Constant),
        ],
    );
}

#[test]
fn syntect_classifies_the_languages_tree_sitter_does_not_onto_the_same_kinds() {
    use Engine::Syntect;
    // A comment's or a string's delimiters are the comment's or the string's.
    assert_tokens(
        "c",
        Syntect,
        "#include <stdio.h>\nint main(void) { return 0; } // done",
        &[
            ("#include", Keyword),
            ("<stdio.h>", Text),
            ("int", Type),
            ("main", Function),
            ("return", Keyword),
            ("0", Number),
            ("(", Punctuation),
            ("// done", Comment),
        ],
    );
    assert_tokens(
        "go",
        Syntect,
        "package main\nfunc main() { fmt.Println(\"hi\", 42) }",
        &[
            ("package", Keyword),
            ("main", Function),
            ("Println", Function),
            ("\"hi\"", Text),
            ("42", Number),
        ],
    );
    assert_tokens(
        "java",
        Syntect,
        "@Override\npublic class A { boolean b = true; int[] xs = new int[3]; }",
        &[
            ("@Override", Attribute),
            ("public", Keyword),
            ("boolean", Type),
            ("new", Keyword),
            ("true", Constant),
            ("=", Operator),
            ("3", Number),
        ],
    );
    assert_tokens(
        "html",
        Syntect,
        "<a href=\"x\">link</a>",
        &[("a", Type), ("href", Attribute), ("\"x\"", Text)],
    );
    assert_tokens(
        "sql",
        Syntect,
        "SELECT name FROM users WHERE id = 1;",
        &[("SELECT", Keyword), ("1", Number)],
    );
    assert_tokens(
        "php",
        Syntect,
        "<?php $count = 1; ?>",
        &[("count", Variable), ("=", Operator)],
    );
}

#[test]
fn languages_neither_knows_and_plain_text_have_no_spans() {
    for language in ["", "text", "TXT", "plain", "plaintext", "klingon", "a b"] {
        assert_eq!(engine(language), Engine::Plain, "{language:?}");
        let highlighted = highlight(language, "fn main() { \"text\" }");
        assert_eq!(highlighted.engine, Engine::Plain);
        assert!(highlighted.spans.is_empty() && highlighted.complete);
    }
}

#[test]
fn languages_are_named_by_their_aliases_in_any_case() {
    for (language, engine_of) in [
        ("rs", Engine::TreeSitter),
        (" Rust ", Engine::TreeSitter),
        ("RB", Engine::TreeSitter),
        ("js", Engine::TreeSitter),
        ("jsx", Engine::TreeSitter),
        ("mjs", Engine::TreeSitter),
        ("cjs", Engine::TreeSitter),
        ("node", Engine::TreeSitter),
        ("TypeScript", Engine::TreeSitter),
        ("ts", Engine::TreeSitter),
        ("mts", Engine::TreeSitter),
        ("cts", Engine::TreeSitter),
        ("py", Engine::TreeSitter),
        ("python3", Engine::TreeSitter),
        ("jsonc", Engine::TreeSitter),
        ("json5", Engine::TreeSitter),
        ("yml", Engine::TreeSitter),
        ("md", Engine::TreeSitter),
        ("sh", Engine::TreeSitter),
        ("shell", Engine::TreeSitter),
        ("zsh", Engine::TreeSitter),
        ("cpp", Engine::Syntect),
        ("C++", Engine::Syntect),
        ("Haskell", Engine::Syntect),
        ("lua", Engine::Syntect),
    ] {
        assert_eq!(engine(language), engine_of, "{language}");
    }
    // Everything can be made ahead of time, and is then ready.
    prepare();
    assert_eq!(highlight("rs", "fn").spans, [Span::new(0, 2, Keyword)]);
}

#[test]
fn spans_are_gathered_in_order_bounded_by_line_and_joined_across_spaces() {
    let text = "ab  cd\n  é f\nxyz";
    let mut spans = Spans::new(text);
    // Plain and empty are ignored; joined across the spaces between them on a line, of the same kind.
    spans.push(0, 2, Plain);
    spans.push(2, 2, Keyword);
    spans.push(0, 2, Keyword);
    spans.push(2, 6, Keyword);
    // Out of order and off a character's boundary are ignored.
    spans.push(0, 1, Text);
    spans.push(10, 11, Text);
    spans.push(9, 10, Text);
    // Across a line break, a span is split, and its spaces dropped.
    spans.push(6, 14, Comment);
    // Not joined across a line break.
    spans.push(14, 15, Comment);
    spans.push(15, 16, Number);
    // Once there is no more room, nothing more is taken.
    spans.full = true;
    spans.push(16, 17, Number);
    assert_eq!(
        spans.finish(),
        [
            Span::new(0, 6, Keyword),
            Span::new(9, 3, Comment),
            Span::new(13, 1, Comment),
            Span::new(14, 1, Number),
        ]
    );
}

#[test]
fn highlighting_is_bounded_in_size_spans_and_time() {
    // Only the first so many bytes are highlighted, cut on a character's boundary, and the rest is plain.
    let long = format!("é{}", "let x = 1;\n".repeat(INPUT_BYTES / 8));
    let highlighted = highlight_within("rust", &long, Duration::from_secs(10));
    assert!(!highlighted.complete);
    assert_valid(&long, &highlighted);
    let last = highlighted.spans.last().unwrap();
    assert!(last.end() <= INPUT_BYTES as u64);
    let odd = format!("{}é", "a".repeat(INPUT_BYTES - 1));
    assert!(!highlight("rust", &odd).complete);
    // At most so many spans.
    let many = format!("[{}1]", "1,".repeat(INPUT_BYTES / 2));
    let highlighted = highlight_within("json", &many, Duration::from_secs(10));
    assert_eq!(highlighted.spans.len(), SPANS);
    assert!(!highlighted.complete);
    // With no time, nothing is highlighted, or little, and it is not complete, with either highlighter.
    for language in ["rust", "c", "markdown"] {
        let text = "fn main() {}\n".repeat(200);
        let highlighted = highlight_within(language, &text, Duration::ZERO);
        assert!(!highlighted.complete, "{language}");
        assert_valid(&text, &highlighted);
    }
}

#[test]
fn hostile_code_fails_safely_within_its_budget() {
    let nested = |open: &str, close: &str| {
        format!(
            "{}{}",
            open.repeat(INPUT_BYTES / 2),
            close.repeat(INPUT_BYTES / 2)
        )
    };
    let lossy = String::from_utf8_lossy(b"fn \xff\xfe main() { \"\xc3\x28\" }").into_owned();
    let controls: String = (0u8..32)
        .map(char::from)
        .chain(['\u{7f}', '\u{202e}', '\u{2028}', '\u{feff}'])
        .collect();
    let hostile = [
        nested("(", ")"),
        nested("[", "]"),
        nested("{", "}"),
        nested("/*", "*/"),
        nested("\"", ""),
        nested("<div>", "</div>"),
        "a".repeat(INPUT_BYTES * 4),
        "\n".repeat(INPUT_BYTES * 2),
        "\u{1F600}".repeat(INPUT_BYTES),
        lossy,
        controls.repeat(64),
        format!("x = \"{}", "\\\"".repeat(INPUT_BYTES)),
    ];
    let languages = [
        "rust",
        "ruby",
        "javascript",
        "typescript",
        "tsx",
        "python",
        "json",
        "yaml",
        "markdown",
        "bash",
        "toml",
        "c",
        "html",
        "perl",
        "latex",
        "unknown",
    ];
    for text in &hostile {
        for language in languages {
            let started = Instant::now();
            let highlighted = highlight(language, text);
            // Within its budget, give or take a parse and what a busy machine adds.
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "{language} took {:?}",
                started.elapsed()
            );
            assert_valid(text, &highlighted);
        }
    }
}
