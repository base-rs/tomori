// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Face panels on lavapipe: an environment's drawing, here a world clock, filling the whole of its panel's frame, in a
//! standard slot and floating, opaque, legible, its text the atlas's at every size.
use super::*;
use tomori_scene::face::fixtures::{Clock, europe};
use tomori_scene::projection::{Fit, drawn_face};

/// The world clock as a face panel in a slot.
fn clock() -> tomori_protocol::Panel {
    tomori_protocol::Panel::new(
        "panel-1",
        "World clock · Europe",
        "World clock Europe (Paris): 13:20 local, day.",
        tomori_protocol::Place::Wall {
            wall: tomori_protocol::TOMORI_WALL.into(),
            slot: 0,
        },
    )
    .with_face(Some(
        tomori_protocol::Face::new("world-clock", europe())
            .with_items(vec![tomori_protocol::ListRow::new("dial", true)]),
    ))
}

/// How many of `image`'s pixels are near `want`.
fn near(image: &image::RgbaImage, want: [u8; 3], within: i32) -> usize {
    image
        .pixels()
        .filter(|pixel| {
            (0..3).all(|channel| {
                (i32::from(pixel.0[channel]) - i32::from(want[channel])).abs() <= within
            })
        })
        .count()
}

/// Set `TOMORI_FACE_PREVIEW` to a PNG path to save what this renders: the world clock drawn by its drawing in a
/// standard slot and floating, at about the headset's resolution up close, side by side.
#[test]
fn a_face_panel_s_drawing_fills_its_frame_in_a_slot_and_floating() {
    crate::test_gpu::with(|gpu| {
        let images: Vec<image::RgbaImage> = [Fit::SLOT, Fit::Text]
            .into_iter()
            .map(|fit| {
                let canvas = drawn_face(&clock(), &Clock, (false, false), fit).expect("drawn");
                drawn_face_on(&gpu, &canvas, 2.3, 0.37).0
            })
            .collect();
        let width = images.iter().map(image::RgbaImage::width).sum();
        let height = images.iter().map(image::RgbaImage::height).max().unwrap();
        let mut sheet = image::RgbaImage::from_pixel(width, height, image::Rgba([40, 40, 40, 255]));
        let mut x = 0;
        for image in &images {
            image::imageops::overlay(&mut sheet, image, i64::from(x), 0);
            x += image.width();
        }
        crate::test_support::preview("TOMORI_FACE_PREVIEW", &sheet);
        let srgb = |rgb: [f32; 3]| rgb.map(|value| (value.powf(1.0 / 2.2) * 255.0) as u8);
        for (image, fit) in images.iter().zip(["slot", "floating"]) {
            // The day's hours in amber, the night's in deep blue, and the needle and time in near white, all drawn.
            assert!(
                near(image, srgb([0.95, 0.66, 0.24]), 20) > 200,
                "{fit}: day"
            );
            assert!(
                near(image, srgb([0.06, 0.1, 0.26]), 12) > 200,
                "{fit}: night"
            );
            assert!(
                near(image, [250, 250, 255], 20) > 40,
                "{fit}: needle and time"
            );
            // Its plate, opaque, fills its frame: the middle of every edge's inside is the plate's colour, not what is
            // behind it.
            let plate = srgb([0.02, 0.035, 0.06]);
            assert!(
                near(image, plate, 6) > (image.width() * image.height()) as usize / 3,
                "{fit}: its plate"
            );
        }
    });
}
