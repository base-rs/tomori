// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::{Duration, Instant};

use tomori_protocol::TokenKind;

use super::*;

/// The code panel a block of `info` and `source` makes, if any.
fn block(info: &str, source: &str) -> Option<Content> {
    let mut lines = Lines::new(info);
    for line in source.lines() {
        lines.push(line);
    }
    lines.content()
}

/// The text each of `content`'s spans covers, with its kind, checking that they are valid for its body.
fn tokens(content: &Content) -> Vec<(String, TokenKind)> {
    let code = content.code.as_ref().unwrap();
    assert!(tomori_protocol::valid_spans(&content.body, &code.spans));
    let chars: Vec<char> = content.body.chars().collect();
    code.spans
        .iter()
        .map(|span| {
            (
                chars[span.start as usize..span.end() as usize]
                    .iter()
                    .collect(),
                span.kind,
            )
        })
        .collect()
}

#[test]
fn a_code_block_is_a_code_panel_titled_with_its_info_string_and_highlighted() {
    let content = block(
        "rust",
        "fn main() {\n    // greet\n    println!(\"hi\");\n}",
    )
    .unwrap();
    assert_eq!(content.title, "rust");
    assert_eq!(
        content.body,
        "fn main() {\n    // greet\n    println!(\"hi\");\n}"
    );
    assert!(content.meta.is_empty());
    let code = content.code.as_ref().unwrap();
    assert_eq!((code.language.as_str(), code.lines), ("rust", 4));
    let found = tokens(&content);
    for expected in [
        ("fn", TokenKind::Keyword),
        ("main", TokenKind::Function),
        ("// greet", TokenKind::Comment),
        ("\"hi\"", TokenKind::String),
    ] {
        assert!(
            found.contains(&(expected.0.to_owned(), expected.1)),
            "{found:?}"
        );
    }
    // An info string with more after the language is the title, and its first word, in lower case, the language.
    let content = block("Python  title=area.py", "def f():\n    pass").unwrap();
    assert_eq!(content.title, "Python  title=area.py");
    assert_eq!(content.code.as_ref().unwrap().language, "python");
    assert!(!content.code.as_ref().unwrap().spans.is_empty());
    // A language only syntect knows is highlighted too; one neither knows, or none, is plain, and untitled is "code".
    let c = block("c", "int main(void) {\n    return 0;\n}").unwrap();
    assert!(tokens(&c).contains(&("return".to_owned(), TokenKind::Keyword)));
    for info in ["klingon", ""] {
        let plain = block(info, "qapla'\nbatlh").unwrap();
        assert!(plain.code.as_ref().unwrap().spans.is_empty());
        assert_eq!(plain.title, if info.is_empty() { "code" } else { info });
    }
    // A language too long to be one is cut, and so is plain.
    let long = block(&"x".repeat(100), "a\nb").unwrap();
    assert_eq!(
        long.code.unwrap().language.chars().count(),
        super::META_KEY_CHARS + 1
    );
    assert_eq!(long.title.chars().count(), TITLE_CHARS + 1);
}

#[test]
fn a_block_of_one_line_or_none_stays_in_the_transcript_without_its_blank_lines() {
    assert!(block("sh", "cargo test").is_none());
    assert!(block("sh", "\n\n   cargo test   \n\n\n").is_none());
    assert!(block("rust", "").is_none());
    assert!(block("rust", "\n \n\t\n").is_none());
    // Two lines are a panel, numbered from the first that is not blank, without the blank lines it ends with.
    let content = block("sh", "\n\ncargo build\n\ncargo test\n\n\n").unwrap();
    assert_eq!(content.body, "cargo build\n\ncargo test");
    assert_eq!(content.code.unwrap().lines, 3);
}

#[test]
fn a_long_block_keeps_its_first_lines_each_cut_and_says_how_long_it_is() {
    let source: Vec<String> = (1..=40)
        .map(|line| format!("let x{line} = {line};"))
        .collect();
    let content = block("rust", &source.join("\n")).unwrap();
    assert_eq!(content.body.lines().count(), CODE_LINES);
    assert_eq!(content.body.lines().last().unwrap(), "let x16 = 16;");
    assert_eq!(content.code.as_ref().unwrap().lines, 40);
    // Blank lines inside it, and many at its end, still count as the block's own, but not those at its end.
    let gappy = format!("{}\n\n\n{}\n\n\n\n", "a", "b");
    assert_eq!(block("text", &gappy).unwrap().code.unwrap().lines, 4);
    // A long line is cut with "…", and its spans with it: a span past the cut is dropped, one across it shortened.
    let long = format!(
        "let s = \"{}\"; let t = 1;\nfn f() {{}}",
        "a".repeat(CODE_LINE_CHARS)
    );
    let content = block("rust", &long).unwrap();
    let first = content.body.lines().next().unwrap();
    assert_eq!(first.chars().count(), CODE_LINE_CHARS + 1);
    assert!(first.ends_with('…'));
    let found = tokens(&content);
    let string = found
        .iter()
        .find(|(_, kind)| *kind == TokenKind::String)
        .unwrap();
    assert_eq!(string.0.chars().count(), CODE_LINE_CHARS - "let s = ".len());
    assert!(!found.contains(&("1".to_owned(), TokenKind::Constant)));
    // The next line's spans are where it is shown.
    assert!(found.contains(&("fn".to_owned(), TokenKind::Keyword)));
    // At most so many spans, of lines full of them.
    let dense = vec!["1,".repeat(CODE_LINE_CHARS / 2); CODE_LINES].join("\n");
    let content = block("json", &format!("[{dense}]")).unwrap();
    assert_eq!(content.code.as_ref().unwrap().spans.len(), CODE_SPANS);
    tokens(&content);
}

#[test]
fn code_is_cleaned_of_tabs_controls_and_format_characters() {
    let content = block(
        "rust\u{202e}",
        "\tlet a = 1;\u{7}\r\nab\tc \u{202e}// \u{200b}evil\u{2066}\nx\u{2028}y   ",
    )
    .unwrap();
    assert_eq!(content.body, "    let a = 1;\nab  c // evil\nx y");
    assert_eq!(content.title, "rust");
    assert!(tokens(&content).contains(&("// evil".to_owned(), TokenKind::Comment)));
}

#[test]
fn hostile_blocks_are_bounded_and_highlighted_within_a_budget() {
    let started = Instant::now();
    let mut lines = Lines::new(&"i".repeat(1_000_000));
    // A million lines of a megabyte each would be a terabyte: a block's lines are counted, and its first kept, cut.
    let huge = "(".repeat(1 << 20);
    for _ in 0..100_000 {
        lines.push(&huge[..1024]);
    }
    lines.push(&huge);
    assert_eq!(lines.kept.len(), CODE_LINES);
    assert!(
        lines
            .kept
            .iter()
            .all(|line| line.chars().count() <= HIGHLIGHTED_CHARS)
    );
    assert!(lines.info.chars().count() <= INFO_CHARS);
    let content = lines.content().unwrap();
    assert_eq!(content.code.as_ref().unwrap().lines, 100_001);
    assert!(content.body.chars().count() <= CODE_LINES * (CODE_LINE_CHARS + 2));
    tokens(&content);
    // Deeply nested, pathological for a parser, invalid UTF-8 read lossily, and every control character.
    let lossy = String::from_utf8_lossy(b"fn \xff main() {\n \"\xc3\x28\" }").into_owned();
    let controls: String = (0u8..32).map(char::from).collect();
    for (info, source) in [
        ("rust", vec!["{[(".repeat(80); CODE_LINES].join("\n")),
        ("json", vec!["[".repeat(250); CODE_LINES].join("\n")),
        ("markdown", vec![">".repeat(250); CODE_LINES].join("\n")),
        ("c", vec!["/*\"".repeat(80); CODE_LINES].join("\n")),
        ("rust", lossy),
        ("python", format!("{controls}\n{controls}")),
    ] {
        let content = block(info, &source).unwrap();
        tokens(&content);
        assert!(!content.body.contains(|c: char| c.is_control() && c != '\n'));
    }
    assert!(started.elapsed() < Duration::from_secs(10));
}
