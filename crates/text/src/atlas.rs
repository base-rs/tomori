// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The glyph atlas: one single-channel texture of signed distance fields, a tile per glyph, filled on demand as characters are first drawn and never moved or evicted, so that a mesh built from it stays valid. A fragment shader draws a glyph's edge where its field crosses one half, antialiased by the field's rate of change on screen, so that text stays crisp and smooth at any distance and tilt. Beside its solid block, for filled quads, it holds the fields of a square, a disc, a ring, a frame and a wedge, [`SQUARE`], [`DISC`], [`RING`], [`FRAME`] and [`WEDGE`], from which shapes are drawn as geometry, antialiased the same way.
//!
//! Tiles are packed on shelves, left to right and top to bottom, each shelf as tall as its tallest tile; the atlas is [`WIDTH`] pixels wide and grows downwards, doubling, up to [`MAX_HEIGHT`]. The texture only needs the rows from the open shelf down uploaded after a change, as earlier shelves are closed. One atlas, behind [`with`], is shared by every panel and label in the process, so that each glyph is drawn into it once.
//!
//! Drawing a glyph costs tens of microseconds, so a thread that draws frames, such as the headset's, sets itself a [`Budget`] for each frame: once a frame has spent it, a glyph not yet drawn is not drawn, as if it had no outline, and [`waited`] counts it, so that whoever built a mesh with it builds it again next frame, when the glyph is drawn. [`frame`] starts the next frame's budget and says what this one spent. Other threads, such as tests' and the console's, have no budget, and draw every glyph when it is first asked for.
use crate::{
    distance,
    fonts::{self, Kind},
};
use ab_glyph::{Font, PxScale, point};
use std::{
    cell::Cell,
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

/// The atlas's width, in pixels.
pub const WIDTH: u32 = 2048;
/// Its height when it starts, in pixels.
pub const START_HEIGHT: u32 = 256;
/// The most it grows to: 16 MiB, room for about 9,000 glyphs, nearly every one the bundled fonts have, including JIS X 0208's 6,355 kanji. It is the largest texture wgpu's default limits allow.
pub const MAX_HEIGHT: u32 = 8192;
/// Pixels of distance either side of an edge that a field holds, which is also the margin around each glyph's outline in its tile, so that a glyph can be made bolder by up to this much.
pub const SPREAD: f32 = 4.0;
/// The middle of a solid block of the atlas, in its pixels, for quads that are filled rather than drawn from a glyph, such as backgrounds, so that one pipeline draws both.
pub const SOLID: [f32; 2] = [2.0, 2.0];

/// A shape's tile, drawn into the atlas beside the solid block when it is made, from which lines, boxes, bars and dots
/// are drawn as shapes rather than glyphs (see `tomori_scene::shapes`): a distance field of a square, a disc or a ring,
/// with empty field all round it, so that a quad drawn from part of it, stretched where the field is flat, has the
/// shape's edge, antialiased by the same shader as a glyph's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Figure {
    /// The tile's middle, in the atlas's pixels.
    pub middle: [f32; 2],
    /// How far its edge is from its middle, in the atlas's pixels: half the square's side, or the disc's or ring's
    /// radius.
    pub reach: f32,
}

/// Pixels of empty field around a shape's tile, beyond the field's spread, so that where a pixel spans several of the
/// atlas's, as far away, the samples the shader averages across it find nothing rather than a neighbour.
const FIGURE_MARGIN: f32 = 8.0;
/// A square 32 pixels a side, whose field is flat at its middle: the ends and corners of lines and boxes, and the sides
/// of boxes too wide for [`WEDGE`].
pub const SQUARE: Figure = Figure {
    middle: [5.0 + 28.0, 28.0],
    reach: 16.0,
};
/// A disc of radius twelve pixels: dots and rounded corners.
pub const DISC: Figure = Figure {
    middle: [62.0 + 24.0, 24.0],
    reach: 12.0,
};
/// A ring of radius twelve pixels and [`RING_WIDTH`] wide, inside its radius.
pub const RING: Figure = Figure {
    middle: [111.0 + 24.0, 24.0],
    reach: 12.0,
};

/// A band that widens along its length, from nothing at its start to [`Wedge::widest`] pixels at its end, so that any
/// line up to that wide is drawn across from one column of it, where it is as wide, its field continuous from one side to
/// the other, so that where a pixel spans several of the atlas's, the samples the shader averages across it find the
/// line as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wedge {
    /// Where it starts, at its middle line, in the atlas's pixels.
    pub start: [f32; 2],
    pub length: f32,
    pub widest: f32,
}

impl Wedge {
    /// Where across the atlas the column is where it is `width` pixels wide.
    pub fn column(&self, width: f32) -> f32 {
        self.start[0] + width.clamp(0.0, self.widest) * self.length / self.widest
    }
}

/// A square's frame, [`FRAME_WIDTH`] wide inside a square 32 pixels a side: frames and checkboxes, their corners and
/// sides each drawn from it, so that the field is continuous across their width.
pub const FRAME: Figure = Figure {
    middle: [313.0 + 28.0, 28.0],
    reach: 16.0,
};
/// How wide [`FRAME`]'s band is, in the atlas's pixels.
pub const FRAME_WIDTH: f32 = 4.0;

/// The wedge, a quarter as wide as it is long, so that its sides lean by a fraction of a degree under 8 and its field is
/// within one percent of a band's.
pub const WEDGE: Wedge = Wedge {
    start: [160.0 + FIGURE_MARGIN + SPREAD, 28.0],
    length: 128.0,
    widest: 32.0,
};
/// How wide [`RING`]'s band is, in the atlas's pixels.
pub const RING_WIDTH: f32 = 4.0;

/// [`WEDGE`]'s tile, and its width and height.
fn wedge() -> (u32, u32, Vec<u8>) {
    let margin = FIGURE_MARGIN + SPREAD;
    let (width, height) = (WEDGE.length + 2.0 * margin, WEDGE.widest + 2.0 * margin);
    let (size, rows) = (width as u32, height as u32);
    // Its sides lean by `slope`, so a point's distance from one is its distance across, less the half width there, as
    // far as the side leans, scaled to the side's normal.
    let slope = WEDGE.widest / (2.0 * WEDGE.length);
    let normal = 1.0 / slope.hypot(1.0);
    let tile = (0..size * rows)
        .map(|index| {
            let (x, y) = (
                (index % size) as f32 + 0.5 - margin,
                (index / size) as f32 + 0.5,
            );
            let across = (y - height / 2.0).abs();
            // Before it starts, as far as its tip; after, as wide as its widest.
            let distance = if x < 0.0 {
                -x.hypot(across)
            } else {
                (x.min(WEDGE.length) * slope - across) * normal
            };
            ((0.5 + distance / (2.0 * SPREAD)).clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect();
    (size, rows, tile)
}

/// The tile of `figure`, whose signed distance from its edge, inside positive, at a point from its middle, is
/// `inside`: as wide and tall as its reach, the field's spread and [`FIGURE_MARGIN`] either side.
fn figure(figure: Figure, inside: impl Fn(f32, f32) -> f32) -> (u32, Vec<u8>) {
    let half = figure.reach + SPREAD + FIGURE_MARGIN;
    let size = (2.0 * half) as u32;
    let tile = (0..size * size)
        .map(|index| {
            let (x, y) = ((index % size) as f32 + 0.5, (index / size) as f32 + 0.5);
            let distance = inside(x - half, y - half);
            ((0.5 + distance / (2.0 * SPREAD)).clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect();
    (size, tile)
}

/// A glyph in the atlas, with where to draw its tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// The tile, in atlas pixels: left, top, width, height.
    pub tile: [f32; 4],
    /// The tile relative to the glyph's origin on the baseline, in ems, y up: left, top, width, height.
    pub bounds: [f32; 4],
    /// The pen's advance after it, in ems.
    pub advance: f32,
    pub kind: Kind,
    /// Atlas pixels per em.
    pub scale: f32,
}

impl Glyph {
    /// How far to move the edge outwards to make the glyph `ems` bolder on each side, in the field's units, as a shader adds it to the half where it draws the edge.
    pub fn weight(&self, ems: f32) -> f32 {
        (ems * self.scale / (2.0 * SPREAD)).min(0.5)
    }
}

/// How much the atlas holds and what drawing into it has cost.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub glyphs: usize,
    pub height: u32,
    /// The pixels the CPU copy holds, one byte each, as the texture does.
    pub bytes: usize,
    /// The time spent rasterising glyphs and computing their fields, in all.
    pub rasterising: Duration,
}

/// What drawing new glyphs into the atlas may cost a thread in one frame: it draws while it has spent less than both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub time: Duration,
    pub glyphs: usize,
}

impl Budget {
    /// No limit, as for a thread that draws no frames.
    pub const UNLIMITED: Self = Self {
        time: Duration::MAX,
        glyphs: usize::MAX,
    };

    /// About 1.5 ms a frame, a little over a tenth of a frame at 72 Hz, the headset's slowest rate: some 35 kanji a frame on a desktop, and perhaps a dozen on a Quest 3.
    pub const FRAME: Self = Self {
        time: Duration::from_micros(1500),
        glyphs: usize::MAX,
    };
}

/// What drawing glyphs cost a thread in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// Glyphs drawn into the atlas.
    pub drawn: usize,
    /// The time spent drawing them.
    pub time: Duration,
    /// Glyphs asked for once the budget was spent, which were left for a later frame.
    pub waiting: usize,
}

#[derive(Clone, Copy)]
struct Allowance {
    budget: Budget,
    spent: Frame,
    waited: u64,
}

thread_local! {
    static ALLOWANCE: Cell<Allowance> = const {
        Cell::new(Allowance {
            budget: Budget::UNLIMITED,
            spent: Frame {
                drawn: 0,
                time: Duration::ZERO,
                waiting: 0,
            },
            waited: 0,
        })
    };
}

fn allowance<R>(f: impl FnOnce(&mut Allowance) -> R) -> R {
    ALLOWANCE.with(|cell| {
        let mut allowance = cell.get();
        let result = f(&mut allowance);
        cell.set(allowance);
        result
    })
}

/// Set what drawing glyphs may cost this thread in each frame, from now on.
pub fn budget(budget: Budget) {
    allowance(|allowance| allowance.budget = budget);
}

/// End this thread's frame: what drawing glyphs cost it, and a fresh budget for the next.
pub fn frame() -> Frame {
    allowance(|allowance| std::mem::take(&mut allowance.spent))
}

/// How many glyphs this thread has asked for and been refused since it started, because its frame's budget was spent. A mesh built while this changed lacks glyphs, and should be built again next frame.
pub fn waited() -> u64 {
    allowance(|allowance| allowance.waited)
}

/// The atlas's pixels and the tiles in it.
pub struct Atlas {
    pixels: Vec<u8>,
    height: u32,
    limit: u32,
    /// The open shelf: its top, its height so far, and how far along it is filled.
    shelf: (u32, u32, u32),
    glyphs: HashMap<char, Option<Glyph>>,
    /// Drawn instead of a glyph when the atlas is full.
    replacement: Option<Glyph>,
    revision: u64,
    rasterising: Duration,
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new(MAX_HEIGHT)
    }
}

impl Atlas {
    /// An atlas that grows to at most `limit` pixels tall, with its solid block and the replacement glyph, `?`, drawn.
    pub fn new(limit: u32) -> Self {
        let mut atlas = Self {
            pixels: vec![0; (WIDTH * START_HEIGHT) as usize],
            height: START_HEIGHT,
            limit: limit.max(START_HEIGHT),
            shelf: (0, 0, 0),
            glyphs: HashMap::new(),
            replacement: None,
            revision: 0,
            rasterising: Duration::ZERO,
        };
        let solid = vec![255; 16];
        atlas.place(4, 4, &solid);
        // A point's signed distance from the edge of a square `reach` from its middle, inside positive.
        let box_distance = |x: f32, y: f32, reach: f32| {
            let (x, y) = (x.abs() - reach, y.abs() - reach);
            -(x.max(0.0).hypot(y.max(0.0)) + x.max(y).min(0.0))
        };
        let square = |x: f32, y: f32| box_distance(x, y, SQUARE.reach);
        let disc = |x: f32, y: f32| DISC.reach - x.hypot(y);
        let ring = |x: f32, y: f32| {
            RING_WIDTH / 2.0 - (x.hypot(y) - (RING.reach - RING_WIDTH / 2.0)).abs()
        };
        // Inside the outer square and outside the inner, the nearer of the two.
        let frame = |x: f32, y: f32| {
            box_distance(x, y, FRAME.reach).min(-box_distance(x, y, FRAME.reach - FRAME_WIDTH))
        };
        for (shape, inside) in [
            (SQUARE, &square as &dyn Fn(f32, f32) -> f32),
            (DISC, &disc),
            (RING, &ring),
        ] {
            // Where the constants say, as a test checks.
            let (size, tile) = figure(shape, inside);
            atlas.place(size, size, &tile);
        }
        let (width, height, tile) = wedge();
        atlas.place(width, height, &tile);
        let (size, tile) = figure(FRAME, frame);
        atlas.place(size, size, &tile);
        // Drawn whatever this thread's budget, as it stands in for every glyph once the atlas is full.
        let (face, id) = fonts::find('?').expect("Noto Sans Mono has `?`");
        atlas.replacement = atlas.add('?', face, id);
        atlas
    }

    /// The glyph drawn for `ch`, drawing it into the atlas the first time: `None` for a character with no outline, such as a space, or that no bundled face has; the replacement once the atlas is full; and `None`, counted by [`waited`], for one not yet drawn once this thread's frame has spent its [`Budget`].
    pub fn glyph(&mut self, ch: char) -> Option<Glyph> {
        if let Some(glyph) = self.glyphs.get(&ch) {
            return *glyph;
        }
        // Not remembered, so that text full of characters no face has cannot grow the map.
        let (face, id) = fonts::find(ch)?;
        let allowed = allowance(|allowance| {
            let allowed = allowance.spent.drawn < allowance.budget.glyphs
                && allowance.spent.time < allowance.budget.time;
            if !allowed {
                allowance.spent.waiting += 1;
                allowance.waited += 1;
            }
            allowed
        });
        if !allowed {
            return None;
        }
        let start = Instant::now();
        let glyph = self.add(ch, face, id);
        let time = start.elapsed();
        allowance(|allowance| {
            allowance.spent.drawn += 1;
            allowance.spent.time += time;
        });
        glyph
    }

    /// Draw `ch`, which is `id` in `face`, into the atlas, whatever the budget.
    fn add(&mut self, ch: char, face: &fonts::Face, id: ab_glyph::GlyphId) -> Option<Glyph> {
        let start = Instant::now();
        let drawn = self.draw(face, id);
        self.rasterising += start.elapsed();
        match drawn {
            Drawn::Glyph(glyph) => {
                self.glyphs.insert(ch, glyph);
                glyph
            }
            Drawn::Full => self.replacement,
        }
    }

    fn draw(&mut self, face: &fonts::Face, id: ab_glyph::GlyphId) -> Drawn {
        let em = face.em();
        let font = &face.font;
        let scale = PxScale::from(face.scale * font.height_unscaled() / em);
        let advance = font.h_advance_unscaled(id) / em;
        let Some(outlined) = font.outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0)))
        else {
            return Drawn::Glyph(None);
        };
        let bounds = outlined.px_bounds();
        let pad = SPREAD as u32;
        let (width, height) = (
            bounds.width() as u32 + 2 * pad,
            bounds.height() as u32 + 2 * pad,
        );
        // Once the atlas is full, a glyph that does not fit is never rasterised, which would cost the same every time it is asked for.
        if self.spot(width, height).is_none() {
            return Drawn::Full;
        }
        let mut coverage = vec![0.0; (width * height) as usize];
        outlined.draw(|x, y, alpha| {
            coverage[((y + pad) * width + x + pad) as usize] = alpha;
        });
        let field = distance::field(&coverage, width as usize, SPREAD);
        let (x, y) = self.place(width, height, &field);
        let per_em = face.scale;
        Drawn::Glyph(Some(Glyph {
            tile: [x as f32, y as f32, width as f32, height as f32],
            bounds: [
                (bounds.min.x - SPREAD) / per_em,
                (SPREAD - bounds.min.y) / per_em,
                width as f32 / per_em,
                height as f32 / per_em,
            ],
            advance,
            kind: face.kind,
            scale: per_em,
        }))
    }

    /// Where a tile of `width` × `height` pixels would go, on the open shelf or a new one below it, and how tall the atlas must grow to hold it: `None` if it cannot.
    fn spot(&self, width: u32, height: u32) -> Option<(u32, u32, u32)> {
        // A pixel between tiles, so that filtering never reads a neighbour.
        let (top, tallest, along) = self.shelf;
        let (x, y) = if along + width <= WIDTH {
            (along, top)
        } else {
            (0, top + tallest + 1)
        };
        let mut grown = self.height;
        while y + height > grown {
            if grown * 2 > self.limit {
                return None;
            }
            grown *= 2;
        }
        Some((x, y, grown))
    }

    /// Copy a tile of `width` × `height` pixels into the atlas, on the open shelf or a new one below it, growing the atlas if it must, where [`Atlas::spot`] found it fits: where it went.
    fn place(&mut self, width: u32, height: u32, tile: &[u8]) -> (u32, u32) {
        let (x, y, grown) = self
            .spot(width, height)
            .expect("a tile is placed only where it was found to fit");
        if grown > self.height {
            self.height = grown;
            self.pixels.resize((WIDTH * self.height) as usize, 0);
        }
        let (top, tallest, along) = self.shelf;
        self.shelf = if y == top {
            (top, tallest.max(height), along + width + 1)
        } else {
            (y, height, width + 1)
        };
        for row in 0..height {
            let start = ((y + row) * WIDTH + x) as usize;
            self.pixels[start..start + width as usize]
                .copy_from_slice(&tile[(row * width) as usize..((row + 1) * width) as usize]);
        }
        self.revision += 1;
        (x, y)
    }

    /// The atlas's pixels, [`WIDTH`] to a row, top to bottom.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The top of the open shelf: rows above it no longer change.
    pub fn open(&self) -> u32 {
        self.shelf.0
    }

    /// The bottom of the open shelf: rows below it are empty.
    pub fn filled(&self) -> u32 {
        self.shelf.0 + self.shelf.1
    }

    /// Counts every tile placed, so that a texture knows when to upload.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn stats(&self) -> Stats {
        Stats {
            glyphs: self.glyphs.values().flatten().count(),
            height: self.height,
            bytes: self.pixels.len(),
            rasterising: self.rasterising,
        }
    }
}

enum Drawn {
    Glyph(Option<Glyph>),
    Full,
}

static ATLAS: Mutex<Option<Atlas>> = Mutex::new(None);

/// Use the process's one atlas, made on first use.
pub fn with<R>(f: impl FnOnce(&mut Atlas) -> R) -> R {
    // The atlas only grows and each change is complete before the next, so a panic elsewhere while it was held leaves it whole.
    let mut atlas = ATLAS.lock().unwrap_or_else(PoisonError::into_inner);
    f(atlas.get_or_insert_with(Atlas::default))
}

/// The glyph drawn for `ch`, from the process's atlas.
pub fn glyph(ch: char) -> Option<Glyph> {
    with(|atlas| atlas.glyph(ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(atlas: &Atlas, x: f32, y: f32) -> u8 {
        atlas.pixels()[y as usize * WIDTH as usize + x as usize]
    }

    #[test]
    fn the_solid_block_and_the_replacement_are_drawn_first() {
        let atlas = Atlas::default();
        assert_eq!(pixel(&atlas, SOLID[0], SOLID[1]), 255);
        assert_eq!(pixel(&atlas, 0.0, 0.0), 255);
        let replacement = atlas.replacement.unwrap();
        assert_eq!(replacement.tile[0], 370.0);
        assert_eq!(atlas.stats().glyphs, 1);
        assert_eq!(atlas.stats().bytes, (WIDTH * START_HEIGHT) as usize);
    }

    #[test]
    fn the_shapes_tiles_are_where_their_constants_say_with_their_edges_at_their_reach() {
        let atlas = Atlas::default();
        // The field at a point `(x, y)` from a figure's middle, between the pixels about it, as a sampler reads it.
        let field = |figure: Figure, x: f32, y: f32| {
            let [u, v] = [figure.middle[0] + x - 0.5, figure.middle[1] + y - 0.5];
            let at = |u: f32, v: f32| f32::from(pixel(&atlas, u, v)) / 255.0;
            let (fu, fv) = (u.fract(), v.fract());
            let (u, v) = (u.floor(), v.floor());
            (at(u, v) * (1.0 - fu) + at(u + 1.0, v) * fu) * (1.0 - fv)
                + (at(u, v + 1.0) * (1.0 - fu) + at(u + 1.0, v + 1.0) * fu) * fv
        };
        for figure in [SQUARE, DISC] {
            // Full at the middle, a half at the edge, and nothing beyond the spread.
            assert_eq!(field(figure, 0.0, 0.0), 1.0);
            assert!((field(figure, figure.reach, 0.0) - 0.5).abs() < 0.01);
            assert!((field(figure, 0.0, -figure.reach) - 0.5).abs() < 0.01);
            assert_eq!(field(figure, figure.reach + SPREAD + 1.0, 0.0), 0.0);
            // Its corners are empty, out to its tile's.
            let half = figure.reach + SPREAD + FIGURE_MARGIN;
            assert_eq!(field(figure, half - 1.0, half - 1.0), 0.0);
        }
        // A disc's edge is round, and a square's corner square, inside.
        let diagonal = DISC.reach / 2f32.sqrt();
        assert!((field(DISC, diagonal, diagonal) - 0.5).abs() < 0.01);
        assert!((field(SQUARE, 15.5, 15.5) - 0.5625).abs() < 0.01);
        // A ring is full halfway through its band, and empty at its middle.
        let band = RING.reach - RING_WIDTH / 2.0;
        assert!(field(RING, band, 0.0) > 0.65);
        assert!((field(RING, RING.reach, 0.0) - 0.5).abs() < 0.01);
        assert!((field(RING, RING.reach - RING_WIDTH, 0.0) - 0.5).abs() < 0.01);
        assert_eq!(field(RING, 0.0, 0.0), 0.0);
        // The wedge's column for a band 4 pixels wide is a half at 2 pixels either side of its middle line, fuller between,
        // as full at its middle as a sampler reads a band's, and empty past the spread; and it is empty before it starts.
        let wedge = |x: f32, y: f32| {
            field(
                Figure {
                    middle: [x, WEDGE.start[1]],
                    reach: 0.0,
                },
                0.0,
                y,
            )
        };
        let column = WEDGE.column(4.0);
        assert!(
            (wedge(column, 2.0) - 0.5).abs() < 0.02 && (wedge(column, -2.0) - 0.5).abs() < 0.02
        );
        assert!(wedge(column, 0.0) > 0.65);
        assert_eq!(wedge(column, 2.0 + SPREAD + 0.5), 0.0);
        assert_eq!(wedge(WEDGE.start[0] - SPREAD - 1.0, 0.0), 0.0);
        assert_eq!(WEDGE.column(100.0), WEDGE.start[0] + WEDGE.length);
        assert_eq!(atlas.shelf.1, 56);
        // A frame is full halfway through its band, all round, and empty inside it.
        let band = FRAME.reach - FRAME_WIDTH / 2.0;
        assert!(field(FRAME, band, 0.3) > 0.65 && field(FRAME, 0.3, -band) > 0.65);
        assert!((field(FRAME, FRAME.reach, 0.3) - 0.5).abs() < 0.01);
        assert!((field(FRAME, FRAME.reach - FRAME_WIDTH, 0.3) - 0.5).abs() < 0.01);
        assert_eq!(field(FRAME, 0.0, 0.0), 0.0);
        assert!(field(FRAME, band, band) > 0.65);
    }

    #[test]
    fn a_glyph_is_drawn_once_and_lies_inside_its_tile() {
        let mut atlas = Atlas::default();
        let a = atlas.glyph('a').unwrap();
        let revision = atlas.revision();
        assert_eq!(atlas.glyph('a'), Some(a));
        assert_eq!(atlas.revision(), revision);
        // Noto Sans Mono's advance is 0.6 em, and its "a" sits on the baseline, below the x-height.
        assert_eq!(a.advance, 0.6);
        assert_eq!(a.kind, Kind::Mono);
        let [left, top, width, height] = a.bounds;
        assert!(left < 0.1 && left + width > 0.5);
        assert!(top > 0.5 && top < 0.75);
        assert!(top - height < 0.0);
        // The tile's edge is far outside the glyph, some of it inside a stroke, which is about two pixels wide, and some on the edge.
        let [x, y, w, h] = a.tile;
        assert_eq!(pixel(&atlas, x, y), 0);
        let tile: Vec<u8> = (y as usize..(y + h) as usize)
            .flat_map(|row| {
                atlas.pixels()[row * WIDTH as usize + x as usize..][..w as usize].to_vec()
            })
            .collect();
        assert!(tile.iter().any(|value| *value >= 150));
        assert!(tile.iter().any(|value| (100..156).contains(value)));
        // A space has no outline, and a character no face has, no glyph.
        assert_eq!(atlas.glyph(' '), None);
        assert_eq!(atlas.glyph('🎤'), None);
        assert!(!atlas.glyphs.contains_key(&'🎤'));
        assert_eq!(atlas.stats().glyphs, 2);
        assert!(atlas.stats().rasterising > Duration::ZERO);
    }

    #[test]
    fn cjk_glyphs_are_finer_and_a_full_em_wide() {
        let mut atlas = Atlas::default();
        let glyph = atlas.glyph('語').unwrap();
        assert_eq!(glyph.kind, Kind::Cjk);
        assert_eq!(glyph.scale, 40.0);
        assert_eq!(glyph.advance, 1.0);
        assert!(glyph.tile[2] > 40.0);
        // Making a glyph bolder is bounded by the field's spread.
        assert_eq!(glyph.weight(0.05), 0.05 * 40.0 / 8.0);
        assert_eq!(glyph.weight(1.0), 0.5);
    }

    #[test]
    fn a_glyphs_field_is_empty_away_from_its_outline() {
        // These left traces of coverage across their boxes, which raised the field there to nearly a half.
        let mut atlas = Atlas::default();
        for ch in "の気東京天本".chars() {
            let [x, y, width, height] = atlas.glyph(ch).unwrap().tile;
            let rows = y as usize..(y + height) as usize;
            let field: Vec<u8> = rows
                .flat_map(|row| {
                    atlas.pixels()[row * WIDTH as usize + x as usize..][..width as usize].to_vec()
                })
                .collect();
            // Only pixels within about half a texel of an edge are near a half: under a sixteenth of these tiles, where the traces made from a sixth to two fifths of them so.
            let near = field
                .iter()
                .filter(|value| (100..=120).contains(*value))
                .count();
            let all = field.len();
            assert!(near * 8 < all, "{ch}: {near} of {all}");
        }
    }

    #[test]
    fn tiles_never_overlap_and_the_atlas_grows_until_it_is_full() {
        let mut atlas = Atlas::new(512);
        let mut tiles: Vec<[f32; 4]> = Vec::new();
        let mut full = None;
        // JIS X 0208's first kanji, in order, until the atlas is full.
        for ch in ('\u{4e00}'..'\u{9fa0}').filter(|ch| fonts::covers(*ch)) {
            let before = atlas.stats().glyphs;
            let glyph = atlas.glyph(ch).unwrap();
            if atlas.stats().glyphs == before {
                full = Some((ch, glyph));
                break;
            }
            tiles.push(glyph.tile);
        }
        let (ch, glyph) = full.expect("the atlas filled");
        // Once full, the replacement is drawn, and the character is tried again later rather than remembered as missing.
        assert_eq!(Some(glyph), atlas.replacement);
        assert!(!atlas.glyphs.contains_key(&ch));
        assert_eq!(atlas.height(), 512);
        assert!(tiles.len() > 200);
        let overlap = |a: &[f32; 4], b: &[f32; 4]| {
            a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
        };
        for (index, a) in tiles.iter().enumerate() {
            assert!(a[0] + a[2] <= WIDTH as f32 && a[1] + a[3] <= 512.0);
            assert!(!overlap(a, &[0.0, 0.0, 4.0, 4.0]));
            for b in &tiles[index + 1..] {
                assert!(!overlap(a, b), "{a:?} {b:?}");
            }
        }
        // Rows above the open shelf are closed, and those below it empty.
        assert!(tiles.iter().all(|tile| tile[1] <= atlas.open() as f32));
        assert!(
            tiles
                .iter()
                .all(|tile| tile[1] + tile[3] <= atlas.filled() as f32)
        );
        assert!(atlas.filled() > atlas.open());
    }

    /// Restores this thread's budget when dropped, even if a test fails, as a test may run on a thread another used.
    struct Budgeted;

    impl Budgeted {
        fn new(limit: Budget) -> Self {
            budget(limit);
            frame();
            Self
        }
    }

    impl Drop for Budgeted {
        fn drop(&mut self) {
            budget(Budget::UNLIMITED);
            frame();
        }
    }

    #[test]
    fn a_frame_draws_glyphs_within_its_budget_and_the_rest_wait_for_the_next() {
        let mut atlas = Atlas::default();
        let _budget = Budgeted::new(Budget {
            time: Duration::MAX,
            glyphs: 2,
        });
        let before = waited();
        assert!(atlas.glyph('日').is_some());
        assert!(atlas.glyph('本').is_some());
        // The budget is spent: a glyph not yet drawn waits, and is not remembered as having no outline.
        assert_eq!(atlas.glyph('語'), None);
        assert!(!atlas.glyphs.contains_key(&'語'));
        assert_eq!(waited(), before + 1);
        // One drawn before, or a character no face has, costs nothing and does not wait.
        assert!(atlas.glyph('日').is_some());
        assert_eq!(atlas.glyph('🎤'), None);
        assert_eq!(waited(), before + 1);
        let spent = frame();
        assert_eq!((spent.drawn, spent.waiting), (2, 1));
        assert!(spent.time > Duration::ZERO);
        // The next frame draws it.
        assert!(atlas.glyph('語').is_some());
        assert_eq!(frame().drawn, 1);
        // With no time to spend, nothing new is drawn, but a new atlas still draws its replacement.
        budget(Budget {
            time: Duration::ZERO,
            glyphs: usize::MAX,
        });
        let mut atlas = Atlas::default();
        assert!(atlas.replacement.is_some());
        assert_eq!(atlas.glyph('a'), None);
        assert_eq!(frame().waiting, 1);
    }

    #[test]
    fn the_frame_budget_is_a_little_of_a_frame() {
        assert!(Budget::FRAME.time <= Duration::from_millis(2));
        assert!(Budget::UNLIMITED.time > Duration::from_secs(1));
        // A thread that never set one has none.
        std::thread::spawn(|| {
            let mut atlas = Atlas::default();
            for ch in ('\u{4e00}'..'\u{4e40}').filter(|ch| fonts::covers(*ch)) {
                assert!(atlas.glyph(ch).is_some());
            }
            assert_eq!(frame().waiting, 0);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_full_atlas_does_not_draw_a_glyph_it_cannot_place() {
        // What drawing one kanji costs here.
        let mut fresh = Atlas::default();
        let rasterising = fresh.stats().rasterising;
        fresh.glyph('語').unwrap();
        let one = fresh.stats().rasterising - rasterising;
        let mut atlas = Atlas::new(START_HEIGHT);
        let full = ('\u{4e00}'..'\u{9fa0}')
            .filter(|ch| fonts::covers(*ch))
            .find(|ch| atlas.glyph(*ch) == atlas.replacement)
            .expect("the atlas filled");
        // Asked for again and again, it is found not to fit before it is rasterised, each time costing a small part of drawing it.
        let rasterising = atlas.stats().rasterising;
        for _ in 0..100 {
            assert_eq!(atlas.glyph(full), atlas.replacement);
        }
        assert!(atlas.stats().rasterising - rasterising < one * 20);
    }

    #[test]
    fn the_shared_atlas_is_made_once() {
        let first = glyph('x').unwrap();
        assert_eq!(glyph('x'), Some(first));
        assert!(with(|atlas| atlas.stats().glyphs) >= 2);
    }
}
