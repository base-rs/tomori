// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The faces of a wall's handles, on lavapipe: each one's symbol centred on its handle by its ink, as the participant
//! sees it, and the grab bar's label centred down its frame.
use super::*;
use tomori_scene::projection::editing::{Axis, Handle, bar_label, handle_face};

/// How far from its face's middle a symbol's ink may centre, as a proportion of the face's width and of its height.
const CENTRED: f32 = 0.04;

/// How many screen pixels each font pixel of a face is drawn, so that its ink is measured finely.
const SCALE: f32 = 12.0;

/// Where `image`'s ink centres, weighted by how much brighter each pixel is than `behind`, in its pixels.
fn ink_centre(image: &image::RgbaImage, behind: u32) -> (f32, f32) {
    let (mut sum, mut x, mut y) = (0.0, 0.0, 0.0);
    for (column, row, _) in image.enumerate_pixels() {
        let weight = brightness(image, column, row).saturating_sub(behind) as f32;
        sum += weight;
        x += weight * column as f32;
        y += weight * row as f32;
    }
    assert!(sum > 0.0, "nothing is drawn");
    // A pixel's middle is half a pixel beyond its corner.
    (x / sum + 0.5, y / sum + 0.5)
}

/// `canvas` drawn face on, without its frame, so that only what it shows is seen against its background, and how far
/// from its middle its ink centres, as a proportion of its width and of its height.
fn off_centre(gpu: &crate::test_gpu::Gpu, mut canvas: Canvas) -> (image::RgbaImage, (f32, f32)) {
    canvas.border = None;
    let (image, place) = drawn_face_on(gpu, &canvas, SCALE, 0.37);
    let (width, height) = canvas.size_in_pixels();
    let (width, height) = (width as f32, height as f32);
    // Its background, just inside its corner, where nothing is drawn on it, a little brighter for what is not even.
    let (corner_x, corner_y) = place(1.0, 1.0);
    let behind = brightness(&image, corner_x as u32, corner_y as u32) + 6;
    let (middle_x, middle_y) = place(width / 2.0, height / 2.0);
    let (x, y) = ink_centre(&image, behind);
    let off = (
        (x - middle_x) / (width * SCALE),
        (y - middle_y) / (height * SCALE),
    );
    (image, off)
}

/// Set `TOMORI_HANDLES_PREVIEW` to a PNG path to save the faces this draws, side by side.
#[test]
fn every_handle_s_symbol_is_centred_on_its_handle_by_its_ink() {
    crate::test_gpu::with(|gpu| {
        let face = |handle, highlighted, locked| handle_face(handle, 0.1, highlighted, locked);
        let faces = [
            ("hinge", face(Handle::Tilt, false, false)),
            ("knob", face(Handle::Bend, false, false)),
            ("columns", face(Handle::Columns, false, false)),
            ("rows", face(Handle::Rows, false, false)),
            (
                "one fewer",
                face(Handle::Step(Axis::Columns, -1), false, false),
            ),
            ("one more", face(Handle::Step(Axis::Rows, 1), false, false)),
            ("padlock closed", face(Handle::Lock, false, true)),
            ("padlock open", face(Handle::Lock, false, false)),
            ("reset", face(Handle::Reset, true, false)),
        ];
        let mut images = Vec::new();
        for (name, face) in faces {
            let (image, (x, y)) = off_centre(&gpu, face);
            assert!(
                x.abs() < CENTRED && y.abs() < CENTRED,
                "{name} is off centre by {x:.3} across and {y:.3} down"
            );
            images.push(image);
        }
        let (width, height) = (images[0].width(), images[0].height());
        let mut sheet = image::RgbaImage::new(width * images.len() as u32, height);
        for (index, image) in images.iter().enumerate() {
            image::imageops::replace(&mut sheet, image, i64::from(width) * index as i64, 0);
        }
        crate::test_support::preview("TOMORI_HANDLES_PREVIEW", &sheet);
        // The grab bar's label, its text, which is not even across, centred down its frame.
        let wall = tomori_protocol::Wall::new(
            "w",
            tomori_protocol::WallKind::Added,
            tomori_protocol::Pose::default(),
        );
        let label = bar_label(tomori_scene::projection::Controls::Controllers, &wall);
        let (_, (_, y)) = off_centre(&gpu, label);
        assert!(
            y.abs() < CENTRED,
            "the bar's label is off centre by {y:.3} down"
        );
    });
}
