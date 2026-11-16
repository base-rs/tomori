// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tiling surface textures, generated in code: earth, stone and bark ([`Surface`]), each a base colour and a normal map
//! made from one seed, seamless, so that a tile laid beside itself shows no edge, with every mip level down to one texel,
//! made in linear light, and compressed to ASTC's 4×4 blocks, which the Quest samples, or BC7's, which desktop GPUs do,
//! each written as KTX2. The same recipe gives the same bytes on every machine, since everything is made by integer
//! hashes and arithmetic, never the platform's transcendental functions, so that a texture can be named by its digest
//! in code and checked against it.
//!
//! ```
//! use tomori_texture_gen::{Format, Recipe, Surface};
//!
//! let set = Recipe::new(Surface::Earth, 16, 7).generate();
//! assert_eq!(set.color.levels.len(), 5);
//! let ktx2 = set.color.ktx2(Format::Astc, true);
//! assert!(ktx2.starts_with(&tomori_texture_gen::KTX2));
//! ```
//!
//! An environment's server generates its own and serves them by digest, from Tomori's content store. See Tomori's
//! `design/rendering.md`.
mod astc;
mod bc7;
mod container;
mod fit;
pub mod noise;
pub mod srgb;
mod surface;

pub use container::IDENTIFIER as KTX2;
pub use surface::Surface;

/// The largest texture generated, in texels a side, as `tomori-render` loads one.
pub const LARGEST: u32 = 2048;

/// A block-compressed format a texture is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// ASTC's 4×4 blocks, 8 bits a texel, which the Quest's GPU samples, and most desktop ones do not.
    Astc,
    /// BC7, 8 bits a texel, which desktop GPUs sample, and the Quest's does not.
    Bc7,
}

impl Format {
    /// Both formats, in the order they were added.
    pub const ALL: [Format; 2] = [Format::Astc, Format::Bc7];

    /// Its name, as its textures are named.
    pub fn name(self) -> &'static str {
        match self {
            Format::Astc => "astc",
            Format::Bc7 => "bc7",
        }
    }
}

/// A texture's levels, RGBA, 8 bits a channel: the largest, `width` by `height`, then each half the last down to one
/// texel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

impl Chain {
    /// The size of level `level`.
    pub fn size(&self, level: usize) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    /// It in `format`'s blocks, as KTX2, its texels read as sRGB if `srgb`, as a base colour is, or as linear, as a
    /// normal map is.
    pub fn ktx2(&self, format: Format, srgb: bool) -> Vec<u8> {
        match format {
            Format::Astc => container::write(self, format, srgb, |block| astc::block(block, srgb)),
            Format::Bc7 => container::write(self, format, srgb, bc7::block),
        }
    }
}

/// What to generate: a surface, a square tile `size` texels a side, a power of two from 4 to [`LARGEST`], and the seed
/// that makes it one of its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Recipe {
    pub surface: Surface,
    pub size: u32,
    pub seed: u32,
}

/// A surface generated: its base colour, in sRGB, its normal map, tangent-space and linear, `u` across and `v` down its
/// texels, and the mean of its base colour in linear light, which a renderer may draw in its place.
#[derive(Clone, Debug, PartialEq)]
pub struct Set {
    pub color: Chain,
    pub normal: Chain,
    pub flat: [f32; 3],
}

impl Recipe {
    /// `surface`, `size` texels a side, from `seed`.
    ///
    /// # Panics
    ///
    /// If `size` is not a power of two from 4 to [`LARGEST`].
    pub fn new(surface: Surface, size: u32, seed: u32) -> Self {
        assert!(
            size.is_power_of_two() && (4..=LARGEST).contains(&size),
            "a tile is a power of two from 4 to {LARGEST} texels a side, not {size}"
        );
        Self {
            surface,
            size,
            seed,
        }
    }

    /// The tile's texels, each its colour in linear light and its height, row by row, made a band of rows on each of the
    /// machine's threads, which changes nothing of what each is.
    fn texels(self) -> Vec<([f32; 3], f32)> {
        let size = self.size as usize;
        let mut texels = vec![([0.0; 3], 0.0); size * size];
        let threads = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(size);
        let rows = size.div_ceil(threads);
        std::thread::scope(|scope| {
            for (band, chunk) in texels.chunks_mut(rows * size).enumerate() {
                scope.spawn(move || {
                    for (index, texel) in chunk.iter_mut().enumerate() {
                        let (x, y) = (index % size, band * rows + index / size);
                        let u = (x as f32 + 0.5) / size as f32;
                        let v = (y as f32 + 0.5) / size as f32;
                        *texel = self.surface.texel(u, v, self.seed);
                    }
                });
            }
        });
        texels
    }

    /// Generate it.
    pub fn generate(self) -> Set {
        let size = self.size as usize;
        let texels = self.texels();
        let colors: Vec<[f32; 3]> = texels.iter().map(|(color, _)| *color).collect();
        // Normals from the height's slope across and down, wrapping at the tile's edges; steeper for a larger tile, so
        // that the relief is as deep across the same surface whatever its size.
        let height = |x: usize, y: usize| texels[(y % size) * size + x % size].1;
        let relief = self.surface.relief() * self.size as f32 / 1024.0;
        let normals: Vec<[f32; 3]> = (0..size * size)
            .map(|index| {
                let (x, y) = (index % size, index / size);
                let across = (height(x + 1, y) - height(x + size - 1, y)) * 0.5 * relief;
                let down = (height(x, y + 1) - height(x, y + size - 1)) * 0.5 * relief;
                normalize([-across, -down, 1.0])
            })
            .collect();
        let color = chain(self.size, colors, average, |color| {
            let [red, green, blue] = color.map(srgb::encode);
            [red, green, blue, 255]
        });
        let normal = chain(
            self.size,
            normals,
            |texels| normalize(average(texels)),
            |normal| {
                let [x, y, z] = normal
                    .map(|value| ((value * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8);
                [x, y, z, 255]
            },
        );
        // The mean of the base colour as a GPU samples it, decoded from its bytes.
        let mut sum = [0.0f64; 3];
        for texel in color.levels[0].as_chunks::<4>().0 {
            for channel in 0..3 {
                sum[channel] += f64::from(srgb::decode(texel[channel]));
            }
        }
        let flat = sum.map(|channel| (channel / (size * size) as f64) as f32);
        Set {
            color,
            normal,
            flat,
        }
    }
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length <= 0.0 {
        [0.0, 0.0, 1.0]
    } else {
        vector.map(|value| value / length)
    }
}

fn average(texels: [[f32; 3]; 4]) -> [f32; 3] {
    std::array::from_fn(|channel| texels.iter().map(|texel| texel[channel]).sum::<f32>() / 4.0)
}

/// Every level of a square tile `size` texels a side, from its texels `level`, each next the `reduce` of each four of
/// the last, then each texel as bytes by `bytes`.
fn chain(
    size: u32,
    mut level: Vec<[f32; 3]>,
    reduce: impl Fn([[f32; 3]; 4]) -> [f32; 3],
    bytes: impl Fn([f32; 3]) -> [u8; 4],
) -> Chain {
    let mut levels = Vec::new();
    let mut side = size as usize;
    loop {
        levels.push(level.iter().flat_map(|texel| bytes(*texel)).collect());
        if side == 1 {
            break;
        }
        let half = side / 2;
        level = (0..half * half)
            .map(|index| {
                let (x, y) = (index % half * 2, index / half * 2);
                reduce([
                    level[y * side + x],
                    level[y * side + x + 1],
                    level[(y + 1) * side + x],
                    level[(y + 1) * side + x + 1],
                ])
            })
            .collect();
        side = half;
    }
    Chain {
        width: size,
        height: size,
        levels,
    }
}

#[cfg(test)]
mod tests;
