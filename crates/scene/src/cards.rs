// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The result cards' pages and their navigation with the right stick: a card's content is wrapped and paged here, once, so that navigation and the renderer use the same pages.
use crate::delegation::{Kind, Presentation, Workspace};

/// The most result cards shown.
pub const MAX_RESULTS: usize = 64;
/// A card's width, in characters.
pub const COLUMNS: usize = 44;
/// A card's height, in rows.
pub const ROWS: usize = 18;
/// The rows of a card that its content fills.
pub const CONTENT_ROWS: usize = ROWS - 4;

/// A bounded cached copy of the focused card's formatted content. It is shared
/// by the navigation and rendering paths so both use exactly the same pages.
#[derive(Default)]
pub struct ContentCache {
    pub card: Option<Presentation>,
    pub pages: Vec<Vec<String>>,
}

fn same_card(a: &Presentation, b: &Presentation) -> bool {
    a.kind == b.kind && a.text == b.text && a.columns == b.columns && a.rows == b.rows
}

impl ContentCache {
    pub fn update(&mut self, card: Option<&Presentation>) {
        // Never clone unbounded or structurally malformed network data.
        let valid = card.is_none_or(|card| {
            !(card.text.len() > 4096
                || card.columns.len() > 6
                || card.rows.len() > 16
                || card.columns.iter().any(|s| s.len() > 80)
                || card.rows.iter().any(|row| {
                    row.len() > 6
                        || row.iter().any(|s| s.len() > 160)
                        || (card.kind == Kind::Table && row.len() != card.columns.len())
                }))
        });
        if valid
            && match (&self.card, card) {
                (Some(a), Some(b)) => same_card(a, b),
                (None, None) => true,
                _ => false,
            }
        {
            return;
        }
        let bounded = card.map(|card| {
            if !valid {
                Presentation {
                    text: "[INVALID CONTENT]".into(),
                    ..Default::default()
                }
            } else {
                Presentation {
                    kind: card.kind,
                    text: card.text.clone(),
                    columns: card.columns.clone(),
                    rows: card.rows.clone(),
                    ..Default::default()
                }
            }
        });
        // Neither is `None` here: no card at all was kept above.
        if self
            .card
            .as_ref()
            .zip(bounded.as_ref())
            .is_some_and(|(a, b)| same_card(a, b))
        {
            return;
        }
        self.pages = bounded.as_ref().map(content_pages).unwrap_or_default();
        self.card = bounded;
    }
}

pub fn results(workspace: &Workspace) -> Vec<&Presentation> {
    let mut results: Vec<_> = workspace.presentations.iter().take(MAX_RESULTS).collect();
    results.sort_by(|a, b| a.id.cmp(&b.id));
    results
}

/// Right-stick navigation, reset on root change and clamped on content edits.
#[derive(Default)]
pub struct CardNavigation {
    root: Option<String>,
    pub card: usize,
    pub content: usize,
    latched: bool,
    cache: ContentCache,
}

impl CardNavigation {
    pub fn release(&mut self) {
        self.latched = false;
    }

    pub fn update(&mut self, root: Option<&str>, workspace: &Workspace, x: f32, y: f32) {
        let changed_root = self.root.as_deref() != root;
        if changed_root {
            self.root = root.map(str::to_owned);
            self.card = 0;
            self.content = 0;
            self.latched = false;
        }
        let cards = results(workspace);
        self.card = self.card.min(cards.len().saturating_sub(1));
        let moving = x.is_finite() && y.is_finite() && (x.abs() > 0.6 || y.abs() > 0.6);
        let action = moving && !self.latched && !changed_root;
        self.latched = moving;
        if action && x.abs() > y.abs() && !cards.is_empty() {
            self.card = step(self.card, cards.len(), x > 0.0);
            self.content = 0;
        }
        self.cache.update(cards.get(self.card).copied());
        let count = self.cache.pages.len();
        self.content = self.content.min(count.saturating_sub(1));
        if action && y.abs() >= x.abs() && count > 0 {
            self.content = step(self.content, count, y < 0.0);
        }
    }
}

fn step(current: usize, count: usize, forward: bool) -> usize {
    if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
}

/// `text` on one row of at most `columns` cells, ending with "…" where cut, by
/// the one rule every panel uses, [`crate::ui_text::clip`]: counted in cells as
/// drawn, so that a Japanese title, two cells a character, is cut where it
/// reaches the card's edge rather than losing its end unmarked.
pub fn clip(text: &str, columns: usize) -> String {
    crate::ui_text::clip(text, columns)
}

/// Cards lay out ASCII, a byte to a column. Show other Unicode scalars and
/// control characters as explicit code-point escapes rather than losing them.
pub fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut just_wrapped = false;
    for ch in text.chars() {
        if ch == '\n' {
            if !just_wrapped || !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            just_wrapped = false;
            continue;
        }
        let token = if ch.is_control() || !ch.is_ascii() {
            format!("\\u{{{:X}}}", ch as u32)
        } else {
            ch.to_string()
        };
        // Never split an escape, so it remains identifiable across pages.
        if line.len() + token.len() > COLUMNS {
            lines.push(std::mem::take(&mut line));
        }
        line.push_str(&token);
        just_wrapped = false;
        if line.len() == COLUMNS {
            lines.push(std::mem::take(&mut line));
            just_wrapped = true;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub fn content_pages(card: &Presentation) -> Vec<Vec<String>> {
    let mut lines = Vec::new();
    match card.kind {
        Kind::Markdown => lines.extend(wrap(&card.text)),
        Kind::Table => {
            if card.columns.is_empty() {
                lines.push("[Empty table]".into());
            } else {
                // Tag every source line so a continuation page identifies the
                // cell even when a long value crosses a page boundary.
                let mut tagged = Vec::new();
                for (column, header) in card.columns.iter().enumerate() {
                    let label = format!("COLUMN {}/{} HEADER", column + 1, card.columns.len());
                    tagged.push((label.clone(), label.clone()));
                    tagged.extend(wrap(header).into_iter().map(|line| (label.clone(), line)));
                    for (row, values) in card.rows.iter().enumerate() {
                        let label =
                            format!("COLUMN {}  ROW {}/{}", column + 1, row + 1, card.rows.len());
                        tagged.push((label.clone(), label.clone()));
                        tagged.extend(
                            wrap(&values[column])
                                .into_iter()
                                .map(|line| (label.clone(), line)),
                        );
                    }
                }
                return tagged
                    .chunks(CONTENT_ROWS - 1)
                    .map(|chunk| {
                        let mut page = vec![format!("[{}]", chunk[0].0)];
                        page.extend(chunk.iter().map(|(_, line)| line.clone()));
                        page
                    })
                    .collect();
            }
        }
    }
    if lines.is_empty() {
        lines.push("[Empty content]".into());
    }
    lines
        .chunks(CONTENT_ROWS)
        .map(|page| page.to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> Presentation {
        Presentation {
            id: id.into(),
            title: format!("Result {id}"),
            source_id: "worker-123".into(),
            source_message_id: "message-456".into(),
            text: "Curated result text".into(),
            ..Default::default()
        }
    }

    #[test]
    fn table_all_cells_and_headers_are_reachable_once() {
        let mut table = card("table");
        table.kind = Kind::Table;
        table.columns = (0..6)
            .map(|i| format!("HEADER{i}{}", "h".repeat(73)))
            .collect();
        table.rows = (0..16)
            .map(|i| {
                (0..6)
                    .map(|j| format!("{i:02}:{j}:{}", "v".repeat(155)))
                    .collect()
            })
            .collect();
        let pages = content_pages(&table);
        assert!(pages.len() > 10);
        assert!(pages.iter().all(
            |page| page.len() <= CONTENT_ROWS && page.iter().all(|line| line.len() <= COLUMNS)
        ));
        let lines: Vec<_> = pages
            .iter()
            .flat_map(|page| page.iter().skip(1))
            .cloned()
            .collect();
        let mut expected = Vec::new();
        for (c, header) in table.columns.iter().enumerate() {
            expected.push(format!("COLUMN {}/6 HEADER", c + 1));
            expected.extend(wrap(header));
            for (r, values) in table.rows.iter().enumerate() {
                expected.push(format!("COLUMN {}  ROW {}/16", c + 1, r + 1));
                expected.extend(wrap(&values[c]));
            }
        }
        assert_eq!(lines, expected);
        for (r, values) in table.rows.iter().enumerate() {
            for (c, value) in values.iter().enumerate() {
                let label = format!("COLUMN {}  ROW {}/16", c + 1, r + 1);
                let start = lines.iter().position(|line| line == &label).unwrap() + 1;
                let count = wrap(value).len();
                assert_eq!(lines[start..start + count].concat(), *value);
            }
        }
    }

    #[test]
    fn maximum_markdown_and_unicode_are_reachable_without_clipping() {
        let text = (0..4096)
            .map(|i| (b'!' + (i % 90) as u8) as char)
            .collect::<String>();
        let card = Presentation {
            text: text.clone(),
            ..Default::default()
        };
        let pages = content_pages(&card);
        assert_eq!(pages.iter().flatten().cloned().collect::<String>(), text);
        assert!(
            pages
                .iter()
                .all(|p| p.len() <= CONTENT_ROWS && p.iter().all(|s| s.len() <= COLUMNS))
        );
        assert_eq!(
            wrap("a🦀é中\t\rb").concat(),
            "a\\u{1F980}\\u{E9}\\u{4E2D}\\u{9}\\u{D}b"
        );
        let unicode = "🦀".repeat(1024);
        let unicode_card = Presentation {
            text: unicode,
            ..Default::default()
        };
        let displayed = content_pages(&unicode_card)
            .iter()
            .flatten()
            .cloned()
            .collect::<String>();
        assert_eq!(displayed, "\\u{1F980}".repeat(1024));
        assert_eq!(wrap(&"x\n".repeat(2048)).len(), 2048);
        // Titles are cut by display width, with "…": a CJK character takes two cells, and an emoji, drawn `?`, one.
        assert_eq!(clip("é中🦀xyz", 4), "é中…");
        assert_eq!(clip("é中🦀xyz", 7), "é中?xyz");
        assert_eq!(clip("é中🦀xyz", 3), "é…");
        assert!(wrap("").is_empty());
    }

    #[test]
    fn stick_navigation_wraps_latches_clamps_and_resets_on_root_change() {
        let mut a = card("a");
        a.text = "x".repeat(4096);
        let mut b = card("b");
        b.text = "y".repeat(4096);
        let mut workspace = Workspace {
            presentations: vec![b, a],
            ..Default::default()
        };
        let mut nav = CardNavigation::default();
        nav.update(Some("root"), &workspace, 0.0, 0.0);
        let pages = nav.cache.pages.len();
        assert!(pages > 1);
        nav.update(Some("root"), &workspace, 0.0, -1.0);
        assert_eq!(nav.content, 1);
        nav.update(Some("root"), &workspace, 0.0, -1.0);
        assert_eq!(nav.content, 1, "holding does not repeat");
        nav.release();
        nav.update(Some("root"), &workspace, 0.0, 1.0);
        assert_eq!(nav.content, 0);
        nav.release();
        nav.update(Some("root"), &workspace, 0.0, 1.0);
        assert_eq!(nav.content, pages - 1);
        nav.release();
        nav.update(Some("root"), &workspace, 1.0, 0.7);
        assert_eq!((nav.card, nav.content), (1, 0));
        nav.release();
        nav.update(Some("root"), &workspace, -1.0, 0.0);
        assert_eq!(nav.card, 0);
        nav.release();
        nav.update(Some("root"), &workspace, -1.0, 0.0);
        assert_eq!(nav.card, 1);
        workspace.presentations.truncate(1);
        nav.update(Some("root"), &workspace, 0.0, 0.0);
        assert_eq!(nav.card, 0);
        nav.content = usize::MAX;
        workspace.presentations[0].text = "short".into();
        nav.update(Some("root"), &workspace, 0.0, 0.0);
        assert_eq!(nav.content, 0);
        nav.card = 10;
        nav.content = 10;
        nav.update(Some("other"), &workspace, 0.0, -1.0);
        assert_eq!((nav.card, nav.content), (0, 0));
        nav.update(Some("other"), &workspace, 1.0, 0.0);
        assert_eq!((nav.card, nav.content), (0, 0));
    }

    #[test]
    fn malformed_content_is_rejected_without_allocating_unbounded_pages() {
        let mut card = card("bad");
        card.text = "x".repeat(4097);
        let mut cache = ContentCache::default();
        cache.update(Some(&card));
        assert_eq!(cache.pages, vec![vec!["[INVALID CONTENT]".to_string()]]);
        card.kind = Kind::Table;
        card.text.clear();
        card.columns = vec!["a".into()];
        card.rows = vec![vec![]];
        cache.update(Some(&card));
        assert_eq!(cache.pages, vec![vec!["[INVALID CONTENT]".to_string()]]);
    }

    #[test]
    fn maximum_multibyte_table_cells_survive_wrapping_and_pagination() {
        let mut table = card("unicode");
        table.kind = Kind::Table;
        table.columns = (0..6).map(|_| "é".repeat(40)).collect();
        table.rows = (0..16)
            .map(|_| (0..6).map(|_| "🦀".repeat(40)).collect())
            .collect();
        let pages = content_pages(&table);
        let lines: Vec<_> = pages
            .iter()
            .flat_map(|page| page.iter().skip(1))
            .cloned()
            .collect();
        for (row, values) in table.rows.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                let marker = format!("COLUMN {}  ROW {}/16", column + 1, row + 1);
                let start = lines.iter().position(|line| line == &marker).unwrap() + 1;
                let n = wrap(value).len();
                assert_eq!(lines[start..start + n].concat(), "\\u{1F980}".repeat(40));
            }
        }
    }

    #[test]
    fn an_unchanged_card_or_none_keeps_its_pages_and_a_card_without_content_says_so() {
        let mut cache = ContentCache::default();
        cache.update(None);
        cache.update(None);
        assert!(cache.card.is_none() && cache.pages.is_empty());
        let card = Presentation {
            id: "p1".into(),
            ..Default::default()
        };
        cache.update(Some(&card));
        cache.update(Some(&card));
        assert!(cache.card.is_some());
        assert_eq!(content_pages(&card), [["[Empty content]"]]);
    }

    #[test]
    fn a_line_break_after_a_full_line_adds_no_empty_line() {
        let full = "x".repeat(COLUMNS);
        assert_eq!(
            wrap(&format!("{full}y\nz")),
            [full.clone(), "y".into(), "z".into()]
        );
        assert_eq!(
            wrap(&format!("{full}{full}\nz")),
            [full.clone(), full, "z".into()]
        );
    }
}
