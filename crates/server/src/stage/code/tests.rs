// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::{Duration, Instant};

use tomori_protocol::TokenKind;

use super::*;

/// The block the maintainer looked at on the Quest 3, whose sixth line showed cut with "…".
const MAINTAINERS_BLOCK: &str = r#"use std::collections::HashMap;

fn summarise(orders: &[(String, u32, f64)]) -> HashMap<String, (u32, f64)> {
    let mut totals = HashMap::new();
    for (customer, quantity, price) in orders {
        let entry = totals.entry(customer.clone()).or_insert((0, 0.0));
        entry.0 += quantity;
        entry.1 += *quantity as f64 * price; // running total of what each customer has spent across every order so far
    }
    totals
}
"#;

/// The code panel a block of `info` and `source` makes, if any.
fn block(info: &str, source: &str) -> Option<Content> {
    let mut lines = Lines::new(info);
    for line in source.lines() {
        lines.push(line);
    }
    lines.content().map(highlighted)
}

/// `found`, highlighted by the server's highlighters.
fn highlighted(found: Found) -> Content {
    let projecting = crate::stage::Projecting::new(vec![(vec![found], None)]);
    let highlighter = crate::highlighting::highlighter();
    let spans = crate::highlighting::highlight_all(&projecting.sources(), &highlighter);
    let mut replies = projecting.highlighted(spans);
    replies.remove(0).0.remove(0).0
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
#[cfg_attr(not(feature = "highlight"), ignore = "needs the server's highlighters")]
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
#[cfg_attr(not(feature = "highlight"), ignore = "needs the server's highlighters")]
fn a_long_block_keeps_its_first_lines_each_cut_and_says_how_long_it_is() {
    // The 17 lines of the block the maintainer could not scroll are all held, as are as many as a panel holds of a longer
    // one.
    let source: Vec<String> = (1..=17)
        .map(|line| format!("const x{line} = {line};"))
        .collect();
    let content = block("javascript", &source.join("\n")).unwrap();
    assert_eq!(content.body, source.join("\n"));
    assert_eq!(content.code.as_ref().unwrap().lines, 17);
    let source: Vec<String> = (1..=CODE_LINES + 40)
        .map(|line| format!("let x{line} = {line};"))
        .collect();
    let content = block("rust", &source.join("\n")).unwrap();
    assert_eq!(content.body.lines().count(), CODE_LINES);
    assert_eq!(
        content.body.lines().last().unwrap(),
        format!("let x{CODE_LINES} = {CODE_LINES};")
    );
    assert_eq!(content.code.as_ref().unwrap().lines, CODE_LINES as u32 + 40);
    // Highlighted to its last line.
    let found = tokens(&content);
    assert!(
        found
            .iter()
            .any(|(text, _)| *text == CODE_LINES.to_string()),
        "{found:?}"
    );
    // Blank lines inside it, and many at its end, still count as the block's own, but not those at its end.
    let gappy = format!("{}\n\n\n{}\n\n\n\n", "a", "b");
    assert_eq!(block("text", &gappy).unwrap().code.unwrap().lines, 4);
    // A line far longer than a row is kept whole, to be wrapped where it is drawn, as are the lines of the maintainer's
    // block, which were once each cut to 64 characters.
    let source = MAINTAINERS_BLOCK.trim_end();
    let content = block("rust", source).unwrap();
    assert_eq!(content.body, source);
    assert!(!content.body.contains('…'));
    let found = tokens(&content);
    assert!(found.iter().any(|(text, _)| text == "0.0"), "{found:?}");
    assert!(
        found.iter().any(|(text, kind)| *kind == TokenKind::Comment
            && text.ends_with("across every order so far")),
        "{found:?}"
    );
    // The lines are cut with "…" only where they reach as many characters together as a panel could show, and the
    // lines after are left out, counted among the block's; the spans with them: one past the cut is dropped, one
    // across it shortened.
    let first = format!("let s = \"{}\";", "a".repeat(CODE_CHARS / 2));
    let second = format!("let t = \"{}\"; let u = 1;", "b".repeat(CODE_CHARS / 2));
    let long = format!("{first}\n{second}\nfn f() {{}}");
    let content = block("rust", &long).unwrap();
    let shown: Vec<&str> = content.body.lines().collect();
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0], first);
    assert!(shown[1].ends_with('…'));
    assert_eq!(content.body.chars().count(), CODE_CHARS + 2);
    assert_eq!(content.code.as_ref().unwrap().lines, 3);
    let found = tokens(&content);
    let string = found
        .iter()
        .filter(|(_, kind)| *kind == TokenKind::String)
        .nth(1)
        .unwrap();
    assert_eq!(
        string.0.chars().count(),
        CODE_CHARS - first.chars().count() - "let t = ".len()
    );
    assert!(!found.iter().any(|(text, _)| text == "1"));
    assert!(!found.iter().any(|(text, _)| text == "fn"));
    // Lines that take every character exactly are not cut, and the lines after are left out without a mark.
    let exact = vec!["x".repeat(CODE_CHARS / 4); 5].join("\n");
    let content = block("text", &exact).unwrap();
    assert_eq!(content.body.lines().count(), 4);
    assert!(!content.body.contains('…'));
    // At most so many spans, of lines full of them.
    let dense = vec!["1,".repeat(CODE_CHARS / CODE_LINES / 2); CODE_LINES].join("\n");
    let content = block("json", &format!("[{dense}]")).unwrap();
    assert_eq!(content.code.as_ref().unwrap().spans.len(), CODE_SPANS);
    tokens(&content);
}

/// `content` as a client is sent it.
fn panel(content: &Content) -> Panel {
    let mut panel = Panel::new(
        "panel-1",
        content.title.clone(),
        content.body.clone(),
        tomori_protocol::Place::Unknown,
    );
    panel.code = content.code.clone();
    panel
}

#[test]
#[cfg_attr(not(feature = "highlight"), ignore = "needs the server's highlighters")]
fn a_code_panel_sent_short_keeps_its_first_lines_and_their_spans_and_its_length() {
    // A long block, highlighted to its end, sent short: its first 16 lines, with their spans, and its length, so that a
    // client says how many it was not sent.
    let source: Vec<String> = (1..=40)
        .map(|line| format!("let x{line} = {line};"))
        .collect();
    let content = block("rust", &source.join("\n")).unwrap();
    let mut sent = panel(&content);
    assert!(short(&mut sent));
    assert_eq!(sent.body, source[..SHORT.lines].join("\n"));
    let code = sent.code.as_ref().unwrap();
    assert_eq!(code.lines, 40);
    assert!(tomori_protocol::valid_spans(&sent.body, &code.spans));
    assert!(!code.spans.is_empty());
    assert!(code.spans.len() < content.code.as_ref().unwrap().spans.len());
    // Cut with "…" where its lines reach 720 characters, a span across the cut shortened and those past it dropped.
    let long = format!("let s = \"{}\";\nlet t = 1;", "a".repeat(SHORT.chars));
    let content = block("rust", &long).unwrap();
    let mut sent = panel(&content);
    assert!(short(&mut sent));
    assert_eq!(sent.body.chars().count(), SHORT.chars + 1);
    assert!(sent.body.ends_with('…'));
    let code = sent.code.as_ref().unwrap();
    assert!(tomori_protocol::valid_spans(&sent.body, &code.spans));
    assert!(
        code.spans
            .iter()
            .all(|span| span.end() <= SHORT.chars as u64)
    );
    // At most so many spans, of lines full of them.
    let dense = vec!["1,".repeat(20); SHORT.lines].join("\n");
    let content = block("json", &format!("[{dense}]")).unwrap();
    let mut sent = panel(&content);
    assert!(short(&mut sent));
    assert_eq!(sent.code.as_ref().unwrap().spans.len(), SHORT.spans);
    // One as short already is unchanged, as is a text panel.
    let content = block("rust", MAINTAINERS_BLOCK.trim_end()).unwrap();
    let mut sent = panel(&content);
    assert!(!short(&mut sent));
    assert_eq!(sent, panel(&content));
    let mut text = Panel::new("panel-2", "", "a\nb", tomori_protocol::Place::Unknown);
    assert!(!short(&mut text));
}

#[test]
#[cfg_attr(not(feature = "highlight"), ignore = "needs the server's highlighters")]
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
        lines.push(&huge[..100]);
    }
    lines.push(&huge);
    assert!(lines.kept.len() <= CODE_LINES);
    assert_eq!(
        lines
            .kept
            .iter()
            .map(|line| line.chars().count())
            .sum::<usize>(),
        HIGHLIGHTED_CHARS
    );
    assert!(lines.info.chars().count() <= INFO_CHARS);
    let content = highlighted(lines.content().unwrap());
    assert_eq!(content.code.as_ref().unwrap().lines, 100_001);
    assert!(content.body.chars().count() <= CODE_CHARS + CODE_LINES);
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
