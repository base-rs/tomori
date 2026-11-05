// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The glyph atlas: one single-channel texture of signed distance fields, a tile per glyph, filled on demand as characters are first drawn and never moved or evicted, so that a mesh built from it stays valid. A fragment shader draws a glyph's edge where its field crosses one half, antialiased by the field's rate of change on screen, so that text stays crisp and smooth at any distance and tilt.
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
        assert_eq!(replacement.tile[0], 5.0);
        assert_eq!(atlas.stats().glyphs, 1);
        assert_eq!(atlas.stats().bytes, (WIDTH * START_HEIGHT) as usize);
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
