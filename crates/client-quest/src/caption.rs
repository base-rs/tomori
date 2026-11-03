// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panel's text as the console prints it, 40 columns by 12 rows of ASCII. On the headset and the desktop window, the status pill (`pill`) and the right wrist's transcript (`transcript`) show the conversation instead.

pub const COLUMNS: usize = 40;
pub const ROWS: usize = 12;

pub fn wrapped_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let word: String = word
                .chars()
                .map(|c| match c {
                    '\u{2018}' | '\u{2019}' => '\'',
                    '\u{201c}' | '\u{201d}' => '"',
                    '\u{2013}' | '\u{2014}' => '-',
                    c if c.is_ascii_graphic() => c,
                    _ => '?',
                })
                .collect();
            if !line.is_empty() && line.len() + 1 + word.len() > COLUMNS {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            for ch in word.chars() {
                if line.len() == COLUMNS {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
fn wrap(text: &str) -> Vec<String> {
    let mut lines = wrapped_lines(text);
    if lines.len() > ROWS {
        lines.truncate(ROWS);
        let last = lines.last_mut().unwrap();
        last.truncate(COLUMNS - 3);
        last.push_str("...");
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_layout_bounds_words_unicode_and_long_transcripts() {
        for text in [
            "".to_owned(),
            "VOICE\nReady.\n\nHello world!".to_owned(),
            "a".repeat(900),
            "Curly ‘quotes’ and emoji 🎤".repeat(100),
        ] {
            let lines = wrap(&text);
            assert!(lines.len() <= ROWS);
            assert!(
                lines
                    .iter()
                    .all(|line| line.is_ascii() && line.len() <= COLUMNS)
            );
        }
        assert_eq!(wrap("hello\n\nworld"), ["hello", "", "world"]);
    }
}
