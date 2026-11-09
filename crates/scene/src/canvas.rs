// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a panel shows: a [`Canvas`] of rows of [`ui_text`](crate::ui_text) cells on a translucent backing, with a colour per span, row backgrounds, speaker bars, meters and shapes. [`Canvas::mesh`] turns it into quads in the panel's plane, in metres, centred on the panel's origin, with +X right and +Y up: a quad per glyph, drawn from its tile in `tomori-text`'s atlas, but one for each run of box drawing's straight lines, such as a table's rules, so that a translucent line is as bright all along, and filled quads, drawn from the atlas's solid block, so that one pipeline draws both. It needs no GPU, so that layouts are tested without one. The renderer's panels draw it.
use bytemuck::{Pod, Zeroable};
use tomori_text::atlas::SOLID;

use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Cell, Quad, Rgba, Row, glyph, quads};

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
        let rows: Vec<&[Cell]> = self
            .rows
            .iter()
            .map(|row| &row.cells[..row.cells.len().min(self.columns)])
            .collect();
        // Lines of box drawing are drawn first, each run of them one quad, and their cells left out of the glyphs.
        let mut lined: Vec<Vec<bool>> = rows.iter().map(|cells| vec![false; cells.len()]).collect();
        let row_top = |index: usize| top + (index * CELL_HEIGHT) as f32;
        let cell_left = |column: usize| left + (column * CELL_WIDTH) as f32;
        let height = CELL_HEIGHT as f32;
        // A glyph the atlas has not drawn yet draws nothing, as in a row's glyphs, until the mesh is built again.
        let mut lines: Vec<(Quad, Rgba)> = Vec::new();
        for (index, cells) in rows.iter().enumerate() {
            for (run, (ch, style)) in runs(cells.iter().copied().map(Some), Stroke::Across) {
                let (from, to) = (cell_left(run.start), cell_left(run.end));
                let quad = glyph(ch, 1, from, row_top(index), height, style.bold);
                lines
                    .extend(quad.map(|quad| (joined(quad, Stroke::Across, from, to), style.color)));
                lined[index][run].fill(true);
            }
        }
        for column in 0..self.columns {
            let down = rows.iter().map(|cells| cells.get(column).copied());
            for (run, (ch, style)) in runs(down, Stroke::Down) {
                let (from, to) = (row_top(run.start), row_top(run.end));
                let quad = glyph(ch, 1, cell_left(column), from, height, style.bold);
                lines.extend(quad.map(|quad| (joined(quad, Stroke::Down, from, to), style.color)));
                for cells in &mut lined[run] {
                    cells[column] = true;
                }
            }
        }
        for (quad, color) in lines {
            mesh.glyph(quad, color);
        }
        for (index, cells) in rows.iter().enumerate() {
            let cells: Vec<Cell> = cells
                .iter()
                .zip(&lined[index])
                .map(|(&(ch, style), &lined)| (if lined { ' ' } else { ch }, style))
                .collect();
            for (quad, style) in quads(&cells, left, row_top(index), height, |style| style.bold) {
                mesh.glyph(quad, style.color);
            }
        }
        mesh.vertices
    }
}

/// Which way a character of box drawing draws a straight line through its cell, from edge to edge: across, as `─`, or
/// down, as `│` and `▏`.
///
/// A run of them, in the same colour and weight, along a row or down a column, is drawn as one quad rather than a glyph
/// for each, such as the rule below a table's header and the lines between its columns. Each glyph of box drawing is
/// grown by half a screen pixel so that it joins its neighbours, and, drawn in a translucent colour, as a table's lines
/// are, every join, where two overlap, would show brighter, so that the line looked made of short segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stroke {
    Across,
    Down,
}

impl Stroke {
    fn of(ch: char) -> Option<Self> {
        match ch {
            '─' => Some(Self::Across),
            '│' | '▏' => Some(Self::Down),
            _ => None,
        }
    }
}

/// The runs of `stroke`'s characters among `cells`, read along a row or down a column, `None` where a column's row is
/// shorter than it: where each starts and ends, and its character and style, each of the same character, colour and
/// weight.
fn runs(
    cells: impl Iterator<Item = Option<Cell>>,
    stroke: Stroke,
) -> Vec<(std::ops::Range<usize>, Cell)> {
    let mut runs: Vec<(std::ops::Range<usize>, Cell)> = Vec::new();
    let lines = cells
        .enumerate()
        .filter_map(|(at, cell)| Some((at, cell?)))
        .filter(|(_, (ch, _))| Stroke::of(*ch) == Some(stroke));
    for (at, (ch, style)) in lines {
        match runs.last_mut() {
            Some((run, (last, was)))
                if run.end == at
                    && *last == ch
                    && was.color == style.color
                    && was.bold == style.bold =>
            {
                run.end += 1
            }
            _ => runs.push((at..at + 1, (ch, style))),
        }
    }
    runs
}

/// The one quad that draws a run of `stroke`'s glyphs, from `from` to `to` along it, in font pixels, where the run's
/// first cell starts and its last ends, from `quad`, its first glyph's: as tall or as wide as that, and drawn from the
/// middle of the glyph's tile, where its field is that of the line's two sides alone, so that it is as bright all along
/// and ends where its cells do.
fn joined(quad: Quad, stroke: Stroke, from: f32, to: f32) -> Quad {
    let [u, v, width, height] = quad.tile;
    match stroke {
        Stroke::Across => Quad {
            x: from,
            width: to - from,
            tile: [u + width / 2.0, v, 0.0, height],
            ..quad
        },
        Stroke::Down => Quad {
            y: from,
            height: to - from,
            tile: [u, v + height / 2.0, width, 0.0],
            ..quad
        },
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
    fn a_run_of_box_drawing_s_lines_is_one_quad_from_end_to_end_of_its_cells() {
        let faint = Style::plain([0.6, 0.66, 0.74, 0.45]);
        let mut canvas = Canvas::new(6, 1.0);
        canvas.background = [0.0; 4];
        canvas.padding = (0, 0);
        let row = |text: &str, style: Style| Row::new(text.chars().map(|ch| (ch, style)).collect());
        canvas.rows = vec![
            row("──────", faint),
            row("a ▏b", faint),
            row("c ▏d", faint),
            // Too short to reach the line, which ends above it.
            row("e", faint),
            // Bold, and in another colour, each a line of its own.
            row(
                "  ▏",
                Style {
                    bold: true,
                    ..faint
                },
            ),
            row("  ▏─", Style::plain([1.0; 4])),
        ];
        let mesh = canvas.mesh();
        let quads: Vec<&[Vertex]> = mesh.chunks(6).collect();
        let extent = |quad: &[Vertex], axis: usize| {
            let values = quad.iter().map(|vertex| vertex.position[axis]);
            values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
        };
        // A line is drawn from the middle of its glyph's tile, the same across it or down it at every corner.
        let across: Vec<&[Vertex]> = quads
            .iter()
            .copied()
            .filter(|quad| quad.iter().all(|vertex| vertex.uv[0] == quad[0].uv[0]))
            .collect();
        let down: Vec<&[Vertex]> = quads
            .iter()
            .copied()
            .filter(|quad| quad.iter().all(|vertex| vertex.uv[1] == quad[0].uv[1]))
            .collect();
        assert_eq!(across.len(), 2);
        assert_eq!(down.len(), 3);
        // The rule spans its six cells, and the line down two rows spans both, from edge to edge.
        assert_eq!(extent(across[0], 0), 6.0 * CELL_WIDTH as f32);
        assert_eq!(extent(across[1], 0), CELL_WIDTH as f32);
        assert_eq!(extent(down[0], 1), 2.0 * CELL_HEIGHT as f32);
        assert_eq!(extent(down[1], 1), CELL_HEIGHT as f32);
        // Every other character is a glyph of its own: "abcde".
        assert_eq!(quads.len(), across.len() + down.len() + 5);
        assert!(down[1][0].weight > down[0][0].weight, "bold");
        assert_eq!(down[2][0].color, [1.0; 4]);
        assert_eq!(
            runs(
                [Some(('─', faint)), None, Some(('─', faint))].into_iter(),
                Stroke::Across
            ),
            [(0..1, ('─', faint)), (2..3, ('─', faint))]
        );
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
