// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panel's text as the console prints it, 40 columns by 12 rows of the characters the headset draws, laid out by [`crate::ui_text`]. On the headset and the desktop window, the status pill (`pill`) and the right wrist's transcript (`transcript`) show the conversation instead.

pub const COLUMNS: usize = 40;
pub const ROWS: usize = 12;

/// `text` wrapped at words to rows of [`COLUMNS`], as [`crate::ui_text::lines`] lays it out.
pub fn wrapped_lines(text: &str) -> Vec<String> {
    crate::ui_text::lines(text, COLUMNS, usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typographic_quotes_and_dashes_are_drawn_plainly() {
        assert_eq!(
            wrapped_lines("\u{201c}Yes\u{201d} \u{2013} it\u{2019}s done \u{1f600}"),
            ["\"Yes\" - it's done ?"]
        );
        // Accented letters are drawn as the headset draws them, rather than as `?`.
        assert_eq!(wrapped_lines("José Łódź"), ["José Lódz"]);
    }

    #[test]
    fn caption_layout_bounds_words_unicode_and_long_transcripts() {
        for text in [
            "".to_owned(),
            "VOICE\nReady.\n\nHello world!".to_owned(),
            "a".repeat(900),
            "Curly ‘quotes’ and emoji 🎤".repeat(100),
        ] {
            let lines = crate::ui_text::lines(&text, COLUMNS, ROWS);
            assert!(lines.len() <= ROWS);
            assert!(lines.iter().all(|line| line.chars().count() <= COLUMNS));
        }
        assert_eq!(wrapped_lines("hello\n\nworld"), ["hello", "", "world"]);
        assert_eq!(wrapped_lines("hello\r\nworld\n"), ["hello", "world"]);
        assert!(wrapped_lines("").is_empty());
    }
}
