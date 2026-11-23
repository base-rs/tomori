// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Generate each surface, say how long it took and how large each format is, and write its base colour and normal map,
//! each tiled two by two so that any edge would show, as PNGs into the folder given, or the current one:
//! `cargo run --release -p tomori-texture-gen --example sheet -- FOLDER [SIZE]`.
use std::time::Instant;

use tomori_texture_gen::{Chain, Format, Recipe, Surface};

fn tiled(chain: &Chain, path: &std::path::Path) {
    let (width, height) = (chain.width, chain.height);
    let mut image = image::RgbaImage::new(width * 2, height * 2);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let at = (((y % height) * width + x % width) * 4) as usize;
        pixel.0.copy_from_slice(&chain.levels[0][at..at + 4]);
    }
    image.save(path).expect("the picture is written");
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let folder = std::path::PathBuf::from(arguments.next().unwrap_or_else(|| ".".into()));
    let size = arguments
        .next()
        .map_or(1024, |size| size.parse().expect("a size"));
    for surface in Surface::ALL {
        let started = Instant::now();
        let set = Recipe::new(surface, size, 1).generate();
        let generated = started.elapsed();
        let mut line = format!(
            "{}: generated in {:.0} ms, flat {:.3?}",
            surface.name(),
            generated.as_secs_f64() * 1e3,
            set.flat
        );
        for format in Format::ALL {
            let started = Instant::now();
            let color = set.color.ktx2(format, true);
            let normal = set.normal.ktx2(format, false);
            line += &format!(
                "; {}: {} + {} bytes in {:.0} ms",
                format.name(),
                color.len(),
                normal.len(),
                started.elapsed().as_secs_f64() * 1e3
            );
        }
        println!("{line}");
        tiled(
            &set.color,
            &folder.join(format!("{}-colour.png", surface.name())),
        );
        tiled(
            &set.normal,
            &folder.join(format!("{}-normal.png", surface.name())),
        );
    }
}
