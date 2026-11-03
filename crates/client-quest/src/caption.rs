// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The floating panel in a space: the conversation's text, 40 columns by 12 rows, drawn as [`ui_panel`](crate::ui_panel) canvases in the scene, so that it is translucent and a nearer hand or controller covers it.
use glam::{Mat4, Vec3};

use crate::ui_panel::{Canvas, Panel};
use crate::ui_text::{Row, Style};

pub const COLUMNS: usize = 40;
pub const ROWS: usize = 12;
const PIXEL: f32 = 0.0034;
/// Where the panel's centre stands from the pose it is placed at: below and in front of it.
const OFFSET: Vec3 = Vec3::new(0.0, -0.32, -1.7);
const HEADER: [f32; 4] = [0.12, 0.85, 1.0, 1.0];
const TEXT: [f32; 4] = [0.92, 0.96, 1.0, 1.0];
const BACKING: [f32; 4] = [0.008, 0.018, 0.035, 0.88];
/// Each canvas's most vertices: every cell lit in every row of its glyph, which the merged runs never reach.
const MAX_VERTICES: usize = (COLUMNS * ROWS * 8 * 2 + 64) * 6;

/// The panel.
pub struct Caption {
    text: Panel,
    shown: Option<String>,
}

impl Caption {
    pub fn new(device: &wgpu::Device, renderer: &crate::graphics::Renderer) -> Self {
        Self {
            text: renderer.panel(device, MAX_VERTICES),
            shown: None,
        }
    }

    pub fn set_text(&mut self, queue: &wgpu::Queue, text: &str) {
        if self.shown.as_deref() == Some(text) {
            return;
        }
        self.text.set(queue, &canvas(text));
        self.shown = Some(text.to_owned());
    }

    /// The panels to draw, placed relative to `pose`, in the space the eyes' matrices take.
    pub fn panels(&self, pose: Mat4) -> Vec<(&Panel, Mat4)> {
        vec![(&self.text, pose * Mat4::from_translation(OFFSET))]
    }
}

/// The panel showing `text`: the first row in cyan, the rest in white.
fn canvas(text: &str) -> Canvas {
    let mut canvas = Canvas::new(COLUMNS, PIXEL);
    canvas.background = BACKING;
    canvas.padding = (18, 12);
    for (row, line) in wrap(text).iter().enumerate() {
        let style = Style::plain(if row == 0 { HEADER } else { TEXT });
        canvas.rows.push(Row::text(line, style, COLUMNS));
    }
    // The panel keeps its size however few rows there are.
    canvas.rows.resize(ROWS, Row::default());
    canvas
}

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
            let canvas = canvas(&text);
            assert_eq!(canvas.rows.len(), ROWS);
            assert!(canvas.mesh().len() <= MAX_VERTICES);
        }
        assert_eq!(wrap("hello\n\nworld"), ["hello", "", "world"]);
    }
}
