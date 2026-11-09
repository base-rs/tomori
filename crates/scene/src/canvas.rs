// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a panel shows: a [`Canvas`] of rows of [`ui_text`](crate::ui_text) cells on a translucent backing, with a colour per span, row backgrounds, speaker bars, meters and shapes. [`Canvas::mesh`] turns it into quads in the panel's plane, in metres, centred on the panel's origin, with +X right and +Y up: a quad per glyph, drawn from its tile in `tomori-text`'s atlas, and filled quads, drawn from the atlas's solid block, so that one pipeline draws both. It needs no GPU, so that layouts are tested without one. The renderer's panels draw it.
use bytemuck::{Pod, Zeroable};
use tomori_text::atlas::SOLID;

use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Quad, Rgba, Row, quads};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    /// Where in the glyph atlas, in its pixels.
    pub uv: [f32; 2],
    /// How much bolder, in the atlas's distance field's units.
    pub weight: f32,
}

/// A rectangle in font pixels from the canvas's top-left corner, inside its padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: Rgba,
}

/// An image a canvas shows, by the SHA-256 that names it (see [`tomori_protocol::Image`]), in a rectangle in font pixels
/// from the canvas's top-left corner, inside its padding, as a shape's is. The renderer draws it over the canvas once it
/// has the image, and the canvas alone until then. See `design/projection.md`.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasImage {
    pub hash: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// What a panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    /// The width of the text, in cells; rows longer than this are cut.
    pub columns: usize,
    pub rows: Vec<Row>,
    /// The size of a font pixel, in metres.
    pub pixel: f32,
    /// Behind everything, usually translucent.
    pub background: Rgba,
    /// Font pixels around the text, horizontally and vertically.
    pub padding: (usize, usize),
    /// A frame around the panel, a font pixel wide.
    pub border: Option<Rgba>,
    /// Shapes drawn over the rows' backgrounds and under the text.
    pub shapes: Vec<Rect>,
    /// An image drawn over everything else, as an image panel shows.
    pub image: Option<CanvasImage>,
}

impl Canvas {
    pub fn new(columns: usize, pixel: f32) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            pixel,
            background: [0.008, 0.018, 0.035, 0.85],
            padding: (6, 4),
            border: None,
            shapes: Vec::new(),
            image: None,
        }
    }

    /// Its image, by hash, and where it is drawn, in metres from the canvas's middle, +X right and +Y up, as its mesh is:
    /// its left, bottom, right and top edges.
    pub fn image_rect(&self) -> Option<(&str, [f32; 4])> {
        let image = self.image.as_ref()?;
        let (width, height) = self.size_in_pixels();
        let (left, top) = (
            self.padding.0 as f32 + image.x - width as f32 / 2.0,
            height as f32 / 2.0 - self.padding.1 as f32 - image.y,
        );
        Some((
            &image.hash,
            [
                left * self.pixel,
                (top - image.height) * self.pixel,
                (left + image.width) * self.pixel,
                top * self.pixel,
            ],
        ))
    }

    /// The panel's size in font pixels.
    pub fn size_in_pixels(&self) -> (usize, usize) {
        (
            self.columns * CELL_WIDTH + 2 * self.padding.0,
            self.rows.len() * CELL_HEIGHT + 2 * self.padding.1,
        )
    }

    /// The panel's width and height in metres.
    pub fn size(&self) -> (f32, f32) {
        let (width, height) = self.size_in_pixels();
        (width as f32 * self.pixel, height as f32 * self.pixel)
    }

    /// The text, a row per line.
    #[cfg(test)]
    pub fn plain(&self) -> String {
        self.rows
            .iter()
            .map(Row::plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The quads that draw the canvas, centred on its origin.
    pub fn mesh(&self) -> Vec<Vertex> {
        let mut mesh = Mesh::new(self);
        let (width, height) = self.size_in_pixels();
        let (width, height) = (width as f32, height as f32);
        mesh.quad(0.0, 0.0, width, height, self.background);
        if let Some(border) = self.border {
            mesh.quad(0.0, 0.0, width, 1.0, border);
            mesh.quad(0.0, height - 1.0, width, 1.0, border);
            mesh.quad(0.0, 0.0, 1.0, height, border);
            mesh.quad(width - 1.0, 0.0, 1.0, height, border);
        }
        let (left, top) = (self.padding.0 as f32, self.padding.1 as f32);
        let text_width = (self.columns * CELL_WIDTH) as f32;
        for (index, row) in self.rows.iter().enumerate() {
            let y = top + (index * CELL_HEIGHT) as f32;
            if let Some(background) = row.background {
                mesh.quad(
                    left - 2.0,
                    y,
                    text_width + 4.0,
                    CELL_HEIGHT as f32,
                    background,
                );
            }
            // Runs of cells with the same background are one quad.
            let mut column = 0;
            let cells = &row.cells[..row.cells.len().min(self.columns)];
            while column < cells.len() {
                let background = cells[column].1.background;
                let start = column;
                while column < cells.len() && cells[column].1.background == background {
                    column += 1;
                }
                if let Some(background) = background {
                    mesh.quad(
                        left + (start * CELL_WIDTH) as f32,
                        y,
                        ((column - start) * CELL_WIDTH) as f32,
                        CELL_HEIGHT as f32,
                        background,
                    );
                }
            }
            if let Some(bar) = row.bar {
                mesh.quad(left + 1.0, y, 3.0, CELL_HEIGHT as f32, bar);
            }
            if let Some((fraction, color)) = row.meter {
                let fraction = fraction.clamp(0.0, 1.0);
                mesh.quad(
                    left,
                    y + CELL_HEIGHT as f32 - 1.5,
                    text_width * fraction,
                    1.5,
                    color,
                );
            }
        }
        for shape in &self.shapes {
            mesh.quad(
                left + shape.x,
                top + shape.y,
                shape.width,
                shape.height,
                shape.color,
            );
        }
        for (index, row) in self.rows.iter().enumerate() {
            let y = top + (index * CELL_HEIGHT) as f32;
            let cells = &row.cells[..row.cells.len().min(self.columns)];
            for (quad, style) in quads(cells, left, y, CELL_HEIGHT as f32, |style| style.bold) {
                mesh.glyph(quad, style.color);
            }
        }
        mesh.vertices
    }
}

/// Builds quads in font pixels from the top-left corner, placed in metres about the centre.
struct Mesh {
    vertices: Vec<Vertex>,
    pixel: f32,
    origin: (f32, f32),
}

impl Mesh {
    fn new(canvas: &Canvas) -> Self {
        let (width, height) = canvas.size_in_pixels();
        Self {
            vertices: Vec::new(),
            pixel: canvas.pixel,
            origin: (width as f32 / 2.0, height as f32 / 2.0),
        }
    }

    /// A filled quad.
    fn quad(&mut self, x: f32, y: f32, width: f32, height: f32, color: Rgba) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.textured(
            Quad {
                x,
                y,
                width,
                height,
                tile: [SOLID[0], SOLID[1], 0.0, 0.0],
                weight: 0.0,
            },
            color,
        );
    }

    /// A glyph's quad, drawn from its tile.
    fn glyph(&mut self, quad: Quad, color: Rgba) {
        self.textured(quad, color);
    }

    fn textured(&mut self, quad: Quad, color: Rgba) {
        if color[3] <= 0.0 {
            return;
        }
        let place = |x: f32, y: f32| {
            [
                (x - self.origin.0) * self.pixel,
                (self.origin.1 - y) * self.pixel,
            ]
        };
        let [u, v, tile_width, tile_height] = quad.tile;
        let (left, top) = (quad.x, quad.y);
        let (right, bottom) = (quad.x + quad.width, quad.y + quad.height);
        for (x, y, across, down) in [
            (left, top, 0.0, 0.0),
            (right, top, 1.0, 0.0),
            (left, bottom, 0.0, 1.0),
            (right, top, 1.0, 0.0),
            (right, bottom, 1.0, 1.0),
            (left, bottom, 0.0, 1.0),
        ] {
            self.vertices.push(Vertex {
                position: place(x, y),
                color,
                uv: [u + across * tile_width, v + down * tile_height],
                weight: quad.weight,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_text::{Span, Style, Wrap, wrap};

    #[test]
    fn the_mesh_stays_within_the_panel_and_is_centred() {
        let mut canvas = Canvas::new(10, 0.001);
        for line in wrap(
            &[Span::new(
                "Every row of this text fits within ten columns, wrapped.",
                Style::bold([1.0; 4]).on([0.2, 0.2, 0.2, 1.0]),
            )],
            10,
            Wrap::default(),
        ) {
            canvas
                .rows
                .push(Row::new(line).with_bar(Some([0.0, 1.0, 1.0, 1.0])));
        }
        canvas.rows[0].meter = Some((0.5, [1.0; 4]));
        canvas.border = Some([1.0, 0.6, 0.2, 1.0]);
        let (width, height) = canvas.size();
        let mesh = canvas.mesh();
        assert_eq!(mesh.len() % 6, 0);
        assert!(mesh.iter().all(|vertex| {
            vertex.position[0].abs() <= width / 2.0 + 1e-5
                && vertex.position[1].abs() <= height / 2.0 + 1e-5
        }));
        // The backing is the first quad, from corner to corner.
        assert_eq!(mesh[0].position, [-width / 2.0, height / 2.0]);
        assert_eq!(mesh[4].position, [width / 2.0, -height / 2.0]);
    }

    #[test]
    fn a_glyph_is_one_quad_from_its_tile_and_bold_is_weightier() {
        let mut canvas = Canvas::new(3, 1.0);
        canvas.background = [0.0; 4];
        canvas
            .rows
            .push(Row::text("a日", Style::plain([1.0; 4]), 3));
        let mesh = canvas.mesh();
        // One quad each, the wide one twice as far across, both drawn from their tiles rather than the solid block.
        assert_eq!(mesh.len(), 12);
        let width = |quad: &[Vertex]| quad[1].position[0] - quad[0].position[0];
        assert!(width(&mesh[6..]) > width(&mesh[..6]));
        assert!(mesh.iter().all(|vertex| vertex.uv != SOLID));
        assert!(mesh.iter().all(|vertex| vertex.weight == 0.0));
        // Its tile's corners map to the quad's.
        let tile = tomori_text::atlas::glyph('a').unwrap().tile;
        assert_eq!(mesh[0].uv, [tile[0], tile[1]]);
        assert_eq!(mesh[4].uv, [tile[0] + tile[2], tile[1] + tile[3]]);
        canvas.rows[0].cells[0].1.bold = true;
        assert!(canvas.mesh()[0].weight > 0.0);
        // Filled quads are drawn from the solid block.
        canvas.background = [0.1, 0.1, 0.1, 1.0];
        let mesh = canvas.mesh();
        assert!(mesh[..6].iter().all(|vertex| vertex.uv == SOLID));
    }

    #[test]
    fn an_image_is_placed_as_a_shape_is_in_metres_from_the_middle() {
        let mut canvas = Canvas::new(10, 0.5);
        canvas.padding = (6, 4);
        canvas.rows = vec![Row::new(Vec::new()); 2];
        assert!(canvas.image_rect().is_none());
        canvas.image = Some(CanvasImage {
            hash: "h".into(),
            x: 0.0,
            y: 0.0,
            width: 90.0,
            height: 24.0,
        });
        // The whole text area, which is 102 by 32 font pixels with its padding.
        let (hash, [left, bottom, right, top]) = canvas.image_rect().unwrap();
        assert_eq!(hash, "h");
        assert_eq!([left, bottom, right, top], [-22.5, -6.0, 22.5, 6.0]);
    }

    #[test]
    fn transparent_and_empty_parts_draw_nothing() {
        let mut canvas = Canvas::new(4, 1.0);
        canvas.background = [0.0; 4];
        canvas
            .rows
            .push(Row::new(vec![(' ', Style::plain([1.0; 4]))]));
        assert!(canvas.mesh().is_empty());
        assert_eq!(canvas.plain(), " ");
    }
}
