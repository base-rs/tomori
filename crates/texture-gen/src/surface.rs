// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The surfaces generated: what each texel of a tile is, its colour in linear light and its height, from its place in
//! the tile and a seed. Each is made of [`crate::noise`], so each tiles.
use crate::noise::{cells, fractal, hash, ridged, smoothstep, unit, value};

/// A tiling surface the generator makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Packed earth, a farmyard's or a trail's: mottled, with fine grit, pebbles pressed into it and a few dry cracks.
    Earth,
    /// Weathered rock: broad undulations, granular speckle, fissures and a little lichen.
    Stone,
    /// A tree's bark: plates split by long grooves running up the trunk, and fibres along them.
    Bark,
}

impl Surface {
    /// Every surface, in the order they were added.
    pub const ALL: [Surface; 3] = [Surface::Earth, Surface::Stone, Surface::Bark];

    /// Its name, as its textures are named.
    pub fn name(self) -> &'static str {
        match self {
            Surface::Earth => "earth",
            Surface::Stone => "stone",
            Surface::Bark => "bark",
        }
    }

    /// How steeply its height bends its normals: the slope of a normal for a rise of its whole height over one texel of
    /// a 1,024 texel tile.
    pub(crate) fn relief(self) -> f32 {
        match self {
            Surface::Earth => 9.0,
            Surface::Stone => 7.0,
            Surface::Bark => 11.0,
        }
    }

    /// The texel at `(u, v)` of the tile, each from 0 to 1: its colour, linear RGB, and its height, from 0 to about 1.
    pub(crate) fn texel(self, u: f32, v: f32, seed: u32) -> ([f32; 3], f32) {
        match self {
            Surface::Earth => earth(u, v, seed),
            Surface::Stone => stone(u, v, seed),
            Surface::Bark => bark(u, v, seed),
        }
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|channel| a[channel] + (b[channel] - a[channel]) * t)
}

fn scale(color: [f32; 3], by: f32) -> [f32; 3] {
    color.map(|channel| channel * by)
}

/// Pebbles on a lattice of `cells`, about a `fill` of the cells holding one: how much of one covers `(u, v)`, from 0 to
/// 1, how high it stands there, and its own shade.
fn pebbles(u: f32, v: f32, cells_across: u32, fill: f32, seed: u32) -> (f32, f32, f32) {
    let near = cells(u, v, (cells_across, cells_across), seed);
    let size = unit(hash(near.id, 1, seed));
    if size > fill {
        return (0.0, 0.0, 0.0);
    }
    let radius = 0.22 + 0.2 * unit(hash(near.id, 2, seed));
    // Each pebble a little oval, turned its own way.
    let turn = unit(hash(near.id, 3, seed));
    let (ox, oy) = near.offset;
    let (along, across) = (
        ox * turn + oy * (1.0 - turn),
        (oy * turn - ox * (1.0 - turn)) * 1.35,
    );
    let distance = (along * along + across * across).sqrt() / radius;
    let cover = smoothstep(1.0, 0.82, distance);
    let dome = (1.0 - distance.min(1.0) * distance.min(1.0)).sqrt();
    (cover, dome * cover, unit(hash(near.id, 4, seed)))
}

/// Packed earth: brown, mottled darker and lighter over tens of centimetres, gritty, with a scatter of small grey-brown
/// pebbles pressed into it and a few fine dry cracks.
fn earth(u: f32, v: f32, seed: u32) -> ([f32; 3], f32) {
    let soil = [0.26, 0.17, 0.10];
    let dark = [0.19, 0.125, 0.075];
    let dusty = [0.31, 0.235, 0.16];
    let mottle = fractal(u, v, (4, 4), 5, 0.55, seed);
    let dust = fractal(u, v, (7, 7), 3, 0.5, seed ^ 0x11);
    let grit =
        value(u, v, (256, 256), seed ^ 0x22) * 0.6 + value(u, v, (128, 128), seed ^ 0x33) * 0.4;
    let mut color = mix(dark, soil, smoothstep(0.25, 0.65, mottle));
    color = mix(color, dusty, smoothstep(0.55, 0.8, dust) * 0.5);
    color = scale(color, 0.88 + 0.24 * grit);
    let mut height = 0.35 * mottle + 0.12 * grit;
    // Pebbles, larger and fewer, then small and a few more, a little paler and greyer than the earth.
    for (cells_across, fill, lift, stone_seed) in [(30, 0.22, 0.55, 0x44), (72, 0.16, 0.3, 0x55)] {
        let (cover, dome, shade) = pebbles(u, v, cells_across, fill, seed ^ stone_seed);
        let pebble = mix([0.25, 0.20, 0.15], [0.31, 0.26, 0.20], shade);
        color = mix(color, pebble, cover * 0.85);
        height += dome * lift;
    }
    // Dry cracks, thin and wandering, only where the earth is broken.
    let wobble = (
        fractal(u, v, (12, 12), 3, 0.5, seed ^ 0x88) - 0.5,
        fractal(u, v, (12, 12), 3, 0.5, seed ^ 0x99) - 0.5,
    );
    let split = cells(
        u + 0.04 * wobble.0,
        v + 0.04 * wobble.1,
        (6, 6),
        seed ^ 0x66,
    );
    let edge = split.second - split.nearest;
    let broken = smoothstep(0.55, 0.75, fractal(u, v, (3, 3), 2, 0.5, seed ^ 0x77));
    let crack = smoothstep(0.025, 0.0, edge) * broken;
    color = scale(color, 1.0 - 0.35 * crack);
    height -= 0.25 * crack;
    (color, height)
}

/// Weathered grey rock: broad undulations, granular light and dark speckle, thin jagged fissures darker and deeper, and
/// soft patches of pale lichen.
fn stone(u: f32, v: f32, seed: u32) -> ([f32; 3], f32) {
    let grey = [0.27, 0.27, 0.26];
    let warm = [0.30, 0.28, 0.25];
    let cool = [0.22, 0.23, 0.24];
    let broad = fractal(u, v, (3, 3), 6, 0.55, seed);
    let tint = fractal(u, v, (5, 5), 3, 0.5, seed ^ 0x101);
    let speckle = value(u, v, (300, 300), seed ^ 0x202);
    let grain = value(u, v, (150, 150), seed ^ 0x303);
    let mut color = mix(cool, warm, smoothstep(0.3, 0.7, tint));
    color = mix(color, grey, 0.4);
    color = scale(color, 0.82 + 0.3 * broad);
    // Grains of light and dark minerals.
    color = scale(
        color,
        1.0 + 0.2 * smoothstep(0.75, 0.95, speckle) - 0.16 * smoothstep(0.3, 0.1, grain),
    );
    let fissure = smoothstep(0.94, 0.99, ridged(u, v, (4, 4), 6, 0.6, seed ^ 0x404));
    color = scale(color, 1.0 - 0.45 * fissure);
    // Lichen, in a few soft patches, pale green-grey, a little raised.
    let lichen = smoothstep(0.6, 0.72, fractal(u, v, (6, 6), 5, 0.55, seed ^ 0x505))
        * smoothstep(0.35, 0.65, fractal(u, v, (24, 24), 3, 0.5, seed ^ 0x606));
    color = mix(color, [0.32, 0.33, 0.27], lichen * 0.6);
    let height = 0.6 * broad + 0.08 * speckle + 0.06 * grain - 0.3 * fissure + 0.05 * lichen;
    (color, height)
}

/// Bark: long furrows running up the trunk, `v` being up, wandering a little, dark and deep, between ridges whose tops
/// are paler and greyer, with fine fibres along them.
fn bark(u: f32, v: f32, seed: u32) -> ([f32; 3], f32) {
    let brown = [0.15, 0.10, 0.07];
    let furrow_color = [0.07, 0.045, 0.035];
    let weathered = [0.20, 0.165, 0.135];
    // Furrows where noise across the trunk, hardly changing up it, crosses its middle, wandering sideways as it climbs.
    let wander = fractal(u, v, (3, 6), 3, 0.5, seed ^ 0x111) - 0.5;
    let ridges = ridged(u + 0.05 * wander, v, (10, 2), 3, 0.45, seed);
    let furrow = smoothstep(0.78, 0.97, ridges);
    // The ridges between them split now and then across.
    let plates = cells(u, v, (10, 3), seed ^ 0x222);
    let split = smoothstep(0.06, 0.0, plates.second - plates.nearest)
        * smoothstep(0.55, 0.75, value(u, v, (12, 12), seed ^ 0x333));
    let fibres =
        value(u, v, (180, 6), seed ^ 0x444) * 0.6 + value(u, v, (90, 4), seed ^ 0x555) * 0.4;
    let shade = fractal(u, v, (4, 2), 4, 0.5, seed ^ 0x666);
    let mut color = mix(
        brown,
        weathered,
        smoothstep(0.3, 0.8, shade) * 0.6 * (1.0 - furrow),
    );
    color = scale(color, 0.86 + 0.28 * fibres);
    color = mix(color, furrow_color, (furrow + split * 0.6).min(1.0) * 0.85);
    let height = 0.75 * (1.0 - furrow) - 0.25 * split + 0.12 * fibres + 0.13 * shade;
    (color, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_surface_is_named_and_has_its_own_look() {
        assert_eq!(Surface::ALL.map(Surface::name), ["earth", "stone", "bark"]);
        for surface in Surface::ALL {
            assert!(surface.relief() > 0.0);
            // Its texels vary, and differ from another seed's.
            let texels: Vec<_> = (0..64)
                .map(|step| surface.texel(step as f32 / 64.0, (step * 7 % 64) as f32 / 64.0, 1))
                .collect();
            let heights: Vec<f32> = texels.iter().map(|(_, height)| *height).collect();
            let low = heights.iter().copied().fold(f32::MAX, f32::min);
            let high = heights.iter().copied().fold(f32::MIN, f32::max);
            assert!(high - low > 0.1, "{surface:?}");
            assert_ne!(surface.texel(0.3, 0.3, 1), surface.texel(0.3, 0.3, 2));
            for (color, _) in texels {
                assert!(color.iter().all(|channel| (0.0..=1.0).contains(channel)));
            }
        }
    }
}
