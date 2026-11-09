// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An image panel's face: its title, its size and age, a rule, its image, and its caption below it. In a wall's slot it
//! is a slot's size, the image fitted inside, keeping its shape, with space either side or above and below; floating or
//! carried, it grows to the image's own shape, as large as a comfortable reading size allows. The renderer draws the
//! image over the face once it has it; until then, the face shows where it will be. See `design/projection.md`.
use tomori_protocol::{Image, Panel};

use super::{BRIGHT, DIM, FIT_COLUMNS, FIT_ROWS, Fit, PADDING, PIXEL, TEXT};
use crate::canvas::{Canvas, CanvasImage, Rect};
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Span, Style, Wrap, wrap};

/// The most rows of a caption, in a slot and floating.
const CAPTION_ROWS: (usize, usize) = (2, 3);

/// The fewest rows an image takes, so that a long caption never squeezes it away.
const IMAGE_ROWS: usize = 6;

/// Where the image is, before it arrives: a little lighter than the face, so that its shape is seen.
const WAITING: [f32; 4] = [0.09, 0.11, 0.15, 0.9];

/// The caption, wrapped at `columns`, on at most `most` rows, the last ending with "…" where it is longer.
fn caption(panel: &Panel, columns: usize, most: usize) -> Vec<Vec<crate::ui_text::Cell>> {
    if panel.body.trim().is_empty() {
        return Vec::new();
    }
    let mut rows = wrap(
        &[Span::new(panel.body.trim(), Style::plain(TEXT))],
        columns,
        Wrap::default(),
    );
    if rows.len() > most {
        rows.truncate(most);
        let last = &mut rows[most - 1];
        last.truncate(columns - 1);
        last.push(('…', Style::plain(TEXT)));
    }
    rows
}

/// How many columns and body rows `panel`'s face has, an image panel of `image`, sized as `fit` says: a slot's, or,
/// floating, as wide and tall as the image at the largest size within [`FIT_COLUMNS`] and [`FIT_ROWS`], with its
/// caption, and never narrower than its title and metadata, within the same bounds.
pub(super) fn fit(panel: &Panel, image: &Image, fit: Fit) -> (usize, usize) {
    match fit {
        Fit::Slot(cells) => (cells.columns, cells.body),
        Fit::Text => {
            let captioned = caption(panel, FIT_COLUMNS.1, CAPTION_ROWS.1).len();
            let (width, height) = shown(
                image,
                (FIT_COLUMNS.1 * CELL_WIDTH) as f32,
                ((FIT_ROWS.1 - captioned) * CELL_HEIGHT) as f32,
            );
            let heading = super::heading_width(panel);
            let columns = (width / CELL_WIDTH as f32)
                .ceil()
                .max(heading as f32)
                .clamp(FIT_COLUMNS.0 as f32, FIT_COLUMNS.1 as f32)
                as usize;
            let rows = (height / CELL_HEIGHT as f32).ceil() as usize;
            let captioned = caption(panel, columns, CAPTION_ROWS.1).len();
            (columns, (rows + captioned).clamp(FIT_ROWS.0, FIT_ROWS.1))
        }
    }
}

/// The largest size, in font pixels, of `image` within `width` by `height`, keeping its shape.
fn shown(image: &Image, width: f32, height: f32) -> (f32, f32) {
    let (image_width, image_height) = (image.width.max(1) as f32, image.height.max(1) as f32);
    let scale = (width / image_width).min(height / image_height);
    (image_width * scale, image_height * scale)
}

/// `panel`'s face, an image panel of `image`, framed by `border` and sized as `fit` says: its title, bold, its size,
/// metadata and age, a rule, the image, fitted to the rows left above its caption and centred there, and its caption.
pub(super) fn face(panel: &Panel, image: &Image, fit: Fit, border: Option<[f32; 4]>) -> Canvas {
    let (columns, body) = self::fit(panel, image, fit);
    let most = match fit {
        Fit::Slot(_) => CAPTION_ROWS.0,
        Fit::Text => CAPTION_ROWS.1,
    };
    let captioned = caption(panel, columns, most.min(body.saturating_sub(IMAGE_ROWS)));
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = [0.02, 0.035, 0.06, 0.94];
    canvas.border = border;
    let mut rows = vec![
        Row::text(&panel.title, Style::bold(BRIGHT), columns),
        Row::text(&super::about(panel), Style::plain(DIM), columns),
        Row::text(&"─".repeat(columns), Style::plain(DIM), columns),
    ];
    let image_rows = body - captioned.len();
    rows.extend(std::iter::repeat_n(Row::new(Vec::new()), image_rows));
    rows.extend(captioned.into_iter().map(Row::new));
    canvas.rows = rows;
    // The image, centred in the rows above its caption, with a font pixel's margin above and below.
    let (area_width, area_height) = (
        (columns * CELL_WIDTH) as f32,
        (image_rows * CELL_HEIGHT) as f32 - 2.0,
    );
    let (width, height) = shown(image, area_width, area_height);
    let (x, y) = (
        (area_width - width) / 2.0,
        (3 * CELL_HEIGHT) as f32 + 1.0 + (area_height - height) / 2.0,
    );
    canvas.shapes.push(Rect {
        x,
        y,
        width,
        height,
        color: WAITING,
    });
    canvas.image = Some(CanvasImage {
        hash: image.hash.clone(),
        x,
        y,
        width,
        height,
    });
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::Place;

    fn panel(width: u32, height: u32, caption: &str) -> Panel {
        Panel::new("panel-1", "Layout", caption, Place::Unknown).with_image(Image::new(
            "a".repeat(64),
            width,
            height,
            1000,
        ))
    }

    /// The image's rectangle, in font pixels, and the canvas's text area.
    fn placed(canvas: &Canvas) -> (CanvasImage, (f32, f32)) {
        (
            canvas.image.clone().unwrap(),
            (
                (canvas.columns * CELL_WIDTH) as f32,
                (canvas.rows.len() * CELL_HEIGHT) as f32,
            ),
        )
    }

    #[test]
    fn in_a_slot_an_image_fits_keeping_its_shape_centred_above_its_caption() {
        for (width, height) in [(1600, 900), (900, 1600), (500, 500), (4000, 100)] {
            let shown = panel(width, height, "The session browser, as a render.");
            let canvas = super::super::face(&shown, false, false, Fit::SLOT);
            // A slot's size, whatever the image.
            assert_eq!(
                canvas.size(),
                super::super::panel_size(),
                "{width}×{height}"
            );
            let (image, (area_width, area_height)) = placed(&canvas);
            // Within the text area, below the title, metadata and rule, and above the caption.
            assert!(image.x >= 0.0 && image.x + image.width <= area_width + 1e-3);
            assert!(image.y >= (3 * CELL_HEIGHT) as f32);
            assert!(image.y + image.height <= area_height - CELL_HEIGHT as f32 + 1e-3);
            // Its shape kept, and as large as fits: touching the sides or the top and bottom.
            let ratio = image.width / image.height;
            assert!((ratio - width as f32 / height as f32).abs() < 0.01 * ratio);
            let left = image.x;
            let above = image.y - (3 * CELL_HEIGHT) as f32 - 1.0;
            assert!(
                left < 0.01 || above < 0.01,
                "{width}×{height}: {left}, {above}"
            );
            // Where it will be is shown until it arrives.
            assert_eq!(canvas.shapes[0].width, image.width);
            assert!(canvas.plain().contains("The session browser, as a render."));
        }
    }

    #[test]
    fn floating_the_frame_hugs_the_image_and_in_a_slot_it_is_letterboxed() {
        for (width, height) in [(1024, 768), (768, 1024), (1600, 900), (800, 800)] {
            let shape = format!("{width} × {height}");
            let shown = panel(width, height, "Its caption.");
            let ratio = width as f32 / height as f32;
            // Floating, its image keeps its shape, as large as fits, and the frame is no larger than its image needs: at
            // most a cell wider, unless its title is wider, and a row taller, besides its title, metadata, rule and
            // caption.
            let canvas = super::super::face(&shown, false, false, Fit::Text);
            let (image, (area_width, area_height)) = placed(&canvas);
            assert!(
                (image.width / image.height - ratio).abs() < 0.01 * ratio,
                "{shape}"
            );
            let wide = area_width - image.width;
            let tall = area_height - image.height - (4 * CELL_HEIGHT) as f32 - 2.0;
            assert!(wide < CELL_WIDTH as f32, "{shape}: {wide} wider");
            assert!(tall < CELL_HEIGHT as f32, "{shape}: {tall} taller");
            assert!(
                canvas.columns == FIT_COLUMNS.1 || canvas.rows.len() == FIT_ROWS.1 + 3,
                "{shape}: as large as a floating panel may be"
            );
            // In a slot, a slot's size, the image letterboxed in it, keeping its shape.
            let slot = super::super::face(&shown, false, false, Fit::SLOT);
            assert_eq!(slot.size(), super::super::panel_size(), "{shape}");
            let (image, _) = placed(&slot);
            assert!(
                (image.width / image.height - ratio).abs() < 0.01 * ratio,
                "{shape}"
            );
        }
        // The headset's test image, as the maintainer saw it: 48 columns by 27 rows of image, its caption below, drawn
        // 429 by 322 font pixels, 4:3, in a face of 444 by 382.
        let shown = panel(1024, 768, "Its caption.");
        assert_eq!(super::super::fit(&shown, Fit::Text), (48, 28));
        let canvas = super::super::face(&shown, false, false, Fit::Text);
        let (image, _) = placed(&canvas);
        assert_eq!((image.width.round(), image.height), (429.0, 322.0));
        assert_eq!(canvas.size_in_pixels(), (444, 382));
    }

    #[test]
    fn floating_an_image_grows_to_its_own_shape_within_a_comfortable_size() {
        let wide = panel(1600, 900, "Wide.");
        let (columns, rows) = super::super::fit(&wide, Fit::Text);
        assert_eq!(columns, FIT_COLUMNS.1);
        let canvas = super::super::face(&wide, false, false, Fit::Text);
        assert_eq!(canvas.size(), super::super::fitted_size(&wide, Fit::Text));
        let (image, _) = placed(&canvas);
        // As wide as a floating panel may be, and as tall as its shape says.
        assert!((image.width - (FIT_COLUMNS.1 * CELL_WIDTH) as f32).abs() < 1.0);
        assert!((image.width / image.height - 16.0 / 9.0).abs() < 0.02);
        assert!(rows >= (image.height / CELL_HEIGHT as f32) as usize);
        // A tall one is as tall as a floating panel may be, and narrower, but no narrower than the narrowest.
        let tall = panel(900, 1600, "");
        let (columns, rows) = super::super::fit(&tall, Fit::Text);
        assert_eq!(rows, FIT_ROWS.1);
        assert!(columns < FIT_COLUMNS.1 && columns >= FIT_COLUMNS.0);
        let (image, _) = placed(&super::super::face(&tall, false, false, Fit::Text));
        assert!((image.width / image.height - 9.0 / 16.0).abs() < 0.02);
        // A long title widens it.
        let mut titled = panel(100, 1000, "");
        titled.title = "A title longer than the image is wide".into();
        assert!(super::super::fit(&titled, Fit::Text).0 >= titled.title.len());
        // A long caption is cut, leaving the image room.
        let long = panel(1600, 900, &"words ".repeat(200));
        let canvas = super::super::face(&long, false, false, Fit::SLOT);
        let (image, _) = placed(&canvas);
        assert!(image.height >= ((IMAGE_ROWS * CELL_HEIGHT) - 2) as f32 - 1e-3);
        assert!(canvas.plain().trim_end().ends_with('…'));
    }
}
