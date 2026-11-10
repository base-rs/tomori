// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Text is drawn as text, and shapes as geometry (see `design/rendering.md`): the code that lays out what the headset,
//! the desktop window and the spectator draw, outside its tests and comments, holds no character of box drawing, blocks,
//! geometric shapes or ballot boxes, nor a run of dashes, equals signs or underscores, but those listed here, each a
//! symbol drawn inline with text, an icon that draws cleanly as one character, a stand-in for a shape that the console
//! prints, or what joins the box drawing that content brings. A rule, box, bar, checkbox or dot is a
//! `tomori_scene::shapes::Shape`, or a cell marked with a `Mark`.
use std::path::{Path, PathBuf};

/// Each character or run allowed in a file, by the end of its path, and why.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "scene/src/shapes.rs",
        "─│▏☐☑●○◉",
        "the characters that stand in for marks, which the console prints",
    ),
    (
        "scene/src/canvas.rs",
        "─│▏",
        "box drawing in content, such as a code block's, joined into one line where it runs",
    ),
    (
        "scene/src/transcript.rs",
        "▸▾",
        "arrows inline with text: the focused choice, and that there is more below",
    ),
    (
        "scene/src/transcript.rs",
        "○◐●",
        "the queue's and the tool chips' states, icons inline with their text beside a tick",
    ),
    (
        "scene/src/projection/list.rs",
        "▸",
        "the pointer the server writes before a list's highlighted row, which is found to be left out, not drawn",
    ),
    (
        "scene/src/projection/editing.rs",
        "●◠",
        "the knob's icon on its handle and in the hint, and the curve in its readout",
    ),
    (
        "client-desktop/src/console.rs",
        "\"-\".repeat(",
        "the console's frame, which is printed as text",
    ),
];

/// Runs of characters that draw a line as text.
const RUNS: &[&str] = &[
    "\"---",
    "\"===",
    "\"___",
    "\"-\".repeat(",
    "\"=\".repeat(",
    "\"_\".repeat(",
];

/// Whether `ch` draws a shape: box drawing, blocks, geometric shapes and ballot boxes.
fn shaped(ch: char) -> bool {
    matches!(ch, '\u{2500}'..='\u{25ff}' | '\u{2610}'..='\u{2612}')
}

/// The crates whose code lays out what is drawn: the client's, the renderer's, and environments' client sides.
fn sources(crates: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = [
        "scene",
        "render",
        "client-quest",
        "client-desktop",
        "client-spectator",
    ]
    .iter()
    .map(|name| crates.join(name).join("src"))
    .collect();
    let mut clients: Vec<PathBuf> = std::fs::read_dir(crates)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with("-client"))
        .map(|path| path.join("src"))
        .collect();
    clients.sort();
    roots.extend(clients);
    let mut files = Vec::new();
    while let Some(path) = roots.pop() {
        if path.is_dir() {
            // Tests, in a `tests` directory or a `tests.rs`, may draw shapes as text to check them.
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            roots.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_some_and(|name| name != "tests.rs")
        {
            files.push(path);
        }
    }
    files
}

/// The lines of `source` that are code, numbered from 1: up to its tests module, without comments.
fn code(source: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = source.lines().collect();
    let tests = lines
        .windows(2)
        .position(|pair| {
            pair[0].trim() == "#[cfg(test)]"
                && pair[1]
                    .trim_start()
                    .trim_start_matches("pub(crate) ")
                    .starts_with("mod tests")
        })
        .unwrap_or(lines.len());
    lines[..tests]
        .iter()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.starts_with("//"))
        .collect()
}

/// What in `source`, at `path`, draws a shape as text and is not allowed.
fn unlisted(path: &str, source: &str) -> Vec<String> {
    let allowed = |found: &str| {
        ALLOWED.iter().any(|(file, allowed, _)| {
            path.ends_with(file)
                && if found.chars().count() == 1 {
                    allowed.contains(found)
                } else {
                    *allowed == found
                }
        })
    };
    let mut unlisted = Vec::new();
    for (number, line) in code(source) {
        let found = line
            .chars()
            .filter(|&ch| shaped(ch))
            .map(String::from)
            .chain(
                RUNS.iter()
                    .filter(|run| line.contains(**run))
                    .map(|run| run.to_string()),
            );
        for found in found {
            if !allowed(&found) {
                unlisted.push(format!("{path}:{number}: {found} in {line}"));
            }
        }
    }
    unlisted
}

#[test]
fn shapes_are_drawn_as_geometry_and_the_glyphs_that_remain_are_listed() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let files = sources(&crates);
    assert!(files.len() > 50, "{}", files.len());
    let unlisted: Vec<String> = files
        .iter()
        .flat_map(|path| {
            let shown = path
                .strip_prefix(&crates)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            unlisted(&shown, &std::fs::read_to_string(path).unwrap())
        })
        .collect();
    assert!(
        unlisted.is_empty(),
        "Draw these as shapes (`tomori_scene::shapes`), or list them in ALLOWED with why:\n{}",
        unlisted.join("\n")
    );
}

#[test]
fn a_rule_drawn_as_text_is_caught_but_not_in_comments_or_tests() {
    let source = "// ─── a comment\nlet rule = \"─\".repeat(4);\nlet more = \"-\".repeat(3);\nlet fine = \"•\";\n#[cfg(test)]\nmod tests {\n    const RULE: &str = \"────\";\n}\n";
    let found = unlisted("scene/src/example.rs", source);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].starts_with("scene/src/example.rs:2: ─"));
    assert!(found[1].contains("\"-\".repeat("));
    // Listed, where it is allowed, it passes.
    assert!(unlisted("scene/src/shapes.rs", "let rule = '─';").is_empty());
    assert!(
        unlisted(
            "client-desktop/src/console.rs",
            "let frame = \"-\".repeat(4);"
        )
        .is_empty()
    );
}
