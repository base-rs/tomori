// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The shapes panels draw besides their text, on lavapipe: rules, bars, quote bars, checkboxes and the scroll bar, each
//! as bright all along, without seams, and antialiased; and at a distance, steady as they move.
use super::*;
use tomori_scene::shapes::{Mark, Shape};
use tomori_scene::ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Style};

/// A translucent line's colour, as a table's.
const FAINT: [f32; 4] = [0.6, 0.66, 0.74, 0.45];
const TEXT: [f32; 4] = [0.86, 0.9, 0.95, 1.0];
const GREEN: [f32; 4] = [0.3, 0.9, 0.5, 1.0];

/// A canvas `columns` wide of `rows`, on no backing, so that only what is drawn on it shows.
fn canvas(columns: usize, rows: Vec<Row>) -> Canvas {
    let mut canvas = Canvas::new(columns, 0.001);
    canvas.background = [0.0; 4];
    canvas.rows = rows;
    canvas
}

/// A row of `text` with `mark`'s cells before it.
fn marked(mark: Mark, text: &str, style: Style) -> Row {
    let mut cells = vec![(mark.stand_in(), style.marked(mark)), (' ', style)];
    cells.extend(text.chars().map(|ch| (ch, Style::plain(TEXT))));
    Row::new(cells)
}

/// Where the cell `(column, row)`'s top-left corner falls, by `place`.
fn cell(
    place: &impl Fn(f32, f32) -> (f32, f32),
    canvas: &Canvas,
    column: f32,
    row: f32,
) -> (f32, f32) {
    place(
        canvas.padding.0 as f32 + column * CELL_WIDTH as f32,
        canvas.padding.1 as f32 + row * CELL_HEIGHT as f32,
    )
}

/// Set `TOMORI_SHAPES_PREVIEW` to a PNG path to save what this renders: a rule, a quote's bar down rows that wrap, a
/// speaker's bar, a bar of time used and a scroll bar, at about the headset's resolution, its pixels off the screen's
/// grid, and then a checklist close up.
#[test]
fn rules_quote_bars_speakers_bars_and_meters_are_as_bright_all_along() {
    crate::test_gpu::with(|gpu| {
        let mut rows = vec![Mark::Rule.row(30, Style::plain(FAINT)), Row::default()];
        for line in ["A quote long enough", "to wrap onto", "three rows"] {
            rows.push(marked(Mark::Quote, line, Style::plain(FAINT)));
        }
        rows.push(Row::default());
        for line in ["Ada said this,", "over two rows"] {
            rows.push(
                Row::text(&format!("  {line}"), Style::plain(TEXT), 30).with_bar(Some(FAINT)),
            );
        }
        let mut canvas = canvas(30, rows);
        let rows = canvas.rows.len() as f32;
        canvas.shapes.push(Shape::Bar {
            x: 0.0,
            y: rows * CELL_HEIGHT as f32 + 2.0,
            width: 30.0 * CELL_WIDTH as f32,
            height: 1.5,
            track: [0.4, 0.45, 0.5, 0.35],
            fill: GREEN,
            from: 0.0,
            to: 0.6,
        });
        canvas.rows.push(Row::default());
        let (image, place) = drawn_face_on(&gpu, &canvas, 2.3, 0.37);
        crate::test_support::preview("TOMORI_SHAPES_PREVIEW", &image);
        let cell = |column: f32, row: f32| cell(&place, &canvas, column, row);
        // The rule along its row, from end to end of its thirty cells.
        let (start, top) = cell(0.0, 0.0);
        let (end, bottom) = cell(30.0, 1.0);
        let (added, ratio) = evenness(
            &image,
            start.ceil() as u32 + 3..end.floor() as u32 - 3,
            top.ceil() as u32..bottom.floor() as u32,
            |x, y| (x, y),
        );
        assert!(added.iter().all(|&added| added > 0), "the rule has gaps");
        assert!(ratio < 1.1, "the rule is uneven along its length: {ratio}");
        // The quote's bar down its three rows, with no seam where they meet.
        let (left, first) = cell(0.0, 2.0);
        let (right, last) = cell(1.0, 5.0);
        let (added, ratio) = evenness(
            &image,
            first.ceil() as u32 + 3..last.floor() as u32 - 3,
            left as u32..right as u32,
            |y, x| (x, y),
        );
        assert!(
            added.iter().all(|&added| added > 0),
            "the quote's bar has gaps"
        );
        assert!(ratio < 1.1, "the quote's bar is uneven: {ratio}");
        // The speaker's bar down their two rows, drawn in the row's first column.
        let (left, first) = cell(0.0, 6.0);
        let (_, last) = cell(0.0, 8.0);
        let (added, ratio) = evenness(
            &image,
            first.ceil() as u32 + 3..last.floor() as u32 - 3,
            left as u32 - 2..left as u32 + 12,
            |y, x| (x, y),
        );
        assert!(
            added.iter().all(|&added| added > 0),
            "the speaker's bar has gaps"
        );
        assert!(ratio < 1.1, "the speaker's bar is uneven: {ratio}");
        // The bar of time used, filled to six tenths and its track after, each even along itself.
        let (start, top) = cell(0.0, 8.0);
        let (end, bottom) = cell(30.0, 9.0);
        let (fill, track) = (
            start + (end - start) * 0.6,
            top.ceil() as u32..bottom.floor() as u32,
        );
        let (filled, ratio) = evenness(
            &image,
            start as u32 + 4..fill as u32 - 4,
            track.clone(),
            |x, y| (x, y),
        );
        assert!(ratio < 1.1, "the bar's fill is uneven: {ratio}");
        let (rest, ratio) = evenness(&image, fill as u32 + 4..end as u32 - 4, track, |x, y| {
            (x, y)
        });
        assert!(ratio < 1.1, "the bar's track is uneven: {ratio}");
        assert!(2 * filled[0] > 3 * rest[0], "{} {}", filled[0], rest[0]);
    });
}

/// How much brighter than `background` each pixel of `image` is from `(x0, y0)` to `(x1, y1)`.
fn lit(image: &image::RgbaImage, (x0, y0): (u32, u32), (x1, y1): (u32, u32)) -> Vec<Vec<u32>> {
    (y0..y1)
        .map(|y| (x0..x1).map(|x| brightness(image, x, y)).collect())
        .collect()
}

#[test]
fn checkboxes_are_crisp_square_frames_and_a_ticked_one_is_filled_with_its_tick() {
    crate::test_gpu::with(|gpu| {
        let canvas = canvas(
            8,
            vec![
                marked(Mark::Check { ticked: false }, "todo", Style::bold(TEXT)),
                marked(Mark::Check { ticked: true }, "done", Style::bold(GREEN)),
            ],
        );
        let scale = 6.3;
        let (image, place) = drawn_face_on(&gpu, &canvas, scale, 0.41);
        crate::test_support::preview("TOMORI_CHECKBOX_PREVIEW", &image);
        let cell = |column: f32, row: f32| cell(&place, &canvas, column, row);
        // Each box stands from half a font pixel in to its cell's right, and a font pixel down to the baseline.
        let corner = |row: f32| {
            let (x, y) = cell(0.0, row);
            ((x + 0.5 * scale) as u32, (y + scale) as u32)
        };
        let side = (8.0 * scale) as u32;
        let empty = lit(
            &image,
            corner(0.0),
            (corner(0.0).0 + side, corner(0.0).1 + side),
        );
        // Its frame is as bright down its left side as down its right, and along its top as its bottom, and its middle
        // is empty.
        let column = |rows: &[Vec<u32>], at: usize| {
            rows[4..rows.len() - 4]
                .iter()
                .map(|row| row[at])
                .collect::<Vec<_>>()
        };
        let (left, right) = (column(&empty, 3), column(&empty, empty[0].len() - 4));
        let even = |values: &[u32]| {
            let (most, least) = (values.iter().max().unwrap(), values.iter().min().unwrap());
            *most as f32 / (*least).max(1) as f32
        };
        assert!(
            even(&left) < 1.1 && even(&right) < 1.1,
            "{left:?} {right:?}"
        );
        let sum = |values: &[u32]| values.iter().sum::<u32>() as f32;
        assert!((sum(&left) / sum(&right) - 1.0).abs() < 0.15);
        let middle = empty[empty.len() / 2][empty[0].len() / 2];
        assert!(middle < 30, "the box is hollow: {middle}");
        // Crisp: across its left edge, from nothing to its full brightness within two pixels.
        let (x, y) = corner(0.0);
        let row = y + side / 2;
        let across: Vec<u32> = (x - 4..x + 6).map(|x| brightness(&image, x, row)).collect();
        let full = *across.iter().max().unwrap();
        let rising = across
            .iter()
            .filter(|&&value| value > full / 10 && value < full * 9 / 10)
            .count();
        assert!(rising <= 2, "{across:?}");
        assert!(full > 600, "{across:?}");
        // Ticked, the box is filled green, its tick dark across its middle.
        let ticked = lit(
            &image,
            corner(1.0),
            (corner(1.0).0 + side, corner(1.0).1 + side),
        );
        let (x, y) = corner(1.0);
        let green = image.get_pixel(x + side / 8, y + side / 8).0;
        assert!(green[1] > 200 && green[1] > green[0] + 60, "{green:?}");
        let dark = ticked
            .iter()
            .flatten()
            .filter(|&&value| value < 150)
            .count();
        assert!(dark > (side * side / 25) as usize, "a tick: {dark}");
    });
}

#[test]
fn the_scroll_bar_s_track_is_even_and_its_thumb_shows_where_the_view_is() {
    crate::test_gpu::with(|gpu| {
        let mut canvas = canvas(10, vec![Row::default(); 10]);
        let height = (10 * CELL_HEIGHT) as f32;
        canvas.shapes.push(Shape::Bar {
            x: (10 * CELL_WIDTH) as f32 + 2.0,
            y: 0.0,
            width: 2.0,
            height,
            track: [0.6, 0.66, 0.75, 0.12],
            fill: [0.6, 0.7, 0.82, 0.75],
            from: 0.4,
            to: 0.6,
        });
        let scale = 2.3;
        let (image, place) = drawn_face_on(&gpu, &canvas, scale, 0.37);
        let (x, top) = place(
            canvas.padding.0 as f32 + (10 * CELL_WIDTH) as f32 + 2.0,
            canvas.padding.1 as f32,
        );
        let across = x as u32 - 2..x as u32 + 8;
        let at = |y: f32| top + y * scale;
        let even = |from: f32, to: f32| {
            evenness(
                &image,
                at(from) as u32 + 3..at(to) as u32 - 3,
                across.clone(),
                |y, x| (x, y),
            )
        };
        let (above, ratio) = even(0.0, 0.4 * height);
        assert!(ratio < 1.1, "the track is uneven above the thumb: {ratio}");
        let (below, ratio) = even(0.6 * height, height);
        assert!(ratio < 1.1, "the track is uneven below the thumb: {ratio}");
        let (thumb, ratio) = even(0.4 * height, 0.6 * height);
        assert!(ratio < 1.1, "the thumb is uneven: {ratio}");
        assert!(
            thumb[0] > 2 * above[0] && thumb[0] > 2 * below[0],
            "{thumb:?} {above:?}"
        );
        assert!(above[0] > 0, "the track shows");
    });
}

/// How much of `image`'s pixels is covered, in all, drawn into a linear target in white.
fn coverage(image: &image::RgbaImage) -> f32 {
    image
        .pixels()
        .map(|pixel| f32::from(pixel.0[0]) / 255.0)
        .sum()
}

#[test]
fn a_thin_rule_and_a_border_far_away_keep_their_ink_as_they_move_rather_than_shimmering() {
    crate::test_gpu::with(|gpu| {
        let rule = canvas(
            40,
            vec![Mark::Rule.row(40, Style::plain([1.0; 4])), Row::default()],
        );
        let mut border = canvas(40, vec![Row::default(); 2]);
        border.border = Some([1.0; 4]);
        // At four tenths and a third of a screen pixel to a font pixel, as a panel across the room, moved by eighths of a
        // pixel across and down, as distant text is tested: the rule covers as much wherever it falls, within a tenth,
        // and as much as a font pixel's width along it; the border, whose sides meet at its corners along hard inner
        // edges, so that they never overlap, within three twentieths.
        for scale in [0.4, 0.33] {
            for (canvas, length, within) in
                [(&rule, 40.0 * CELL_WIDTH as f32, 0.1), (&border, 0.0, 0.15)]
            {
                let inks: Vec<f32> = (0..8)
                    .map(|step| {
                        let shift = step as f32 / 8.0;
                        let format = wgpu::TextureFormat::Rgba8Unorm;
                        coverage(&face_on_in(&gpu, canvas, scale, shift, format).0)
                    })
                    .collect();
                let mean = inks.iter().sum::<f32>() / inks.len() as f32;
                let (least, most) = inks
                    .iter()
                    .fold((f32::MAX, 0.0_f32), |(least, most), &ink| {
                        (least.min(ink), most.max(ink))
                    });
                assert!((most - least) / mean < within, "{scale}: {inks:?}");
                if length > 0.0 {
                    let expected = length * scale * scale;
                    assert!(
                        (mean - expected).abs() < expected * 0.15,
                        "{scale}: {mean} of {expected}"
                    );
                }
            }
        }
    });
}

/// Set `TOMORI_CURVED_RULE_PREVIEW` to a PNG path to save what this renders: three panels with rules in neighbouring
/// slots of the Tomori wall, which curves, seen from in front of it and to one side.
#[test]
fn rules_on_a_curved_wall_s_panels_follow_each_slot_without_seams() {
    crate::test_gpu::with(|gpu| {
        let (width, height) = (960, 640);
        let wall = tomori_scene::scene::fixtures::tomori_wall();
        let frame = Mat4::from_rotation_translation(
            glam::Quat::from_array(wall.pose.orientation),
            glam::Vec3::from_array(wall.pose.position),
        );
        let slots = tomori_scene::projection::slot_frames(&wall, frame);
        let eye = glam::Vec3::new(0.35, 0.7, -0.9);
        let view = glam::camera::rh::proj::directx::perspective(
            0.9,
            width as f32 / height as f32,
            0.05,
            50.0,
        ) * glam::camera::rh::view::look_at_mat4(
            eye,
            slots[2].w_axis.truncate(),
            glam::Vec3::Y,
        );
        let panel = |rule: bool| {
            let mut rows = vec![Row::text("Release steps", Style::bold(TEXT), 30)];
            rows.push(if rule {
                Mark::Rule.row(30, Style::plain(FAINT))
            } else {
                Row::default()
            });
            rows.push(Row::text("Bump the version", Style::plain(TEXT), 30));
            let mut canvas = canvas(30, rows);
            canvas.background = [0.01, 0.02, 0.04, 0.85];
            canvas.pixel =
                tomori_scene::projection::panel_size().0 / canvas.size_in_pixels().0 as f32;
            canvas
        };
        let shown = &slots[1..4];
        let draw = |rule: bool| {
            let transforms: Vec<Mat4> = shown.iter().map(|slot| view * *slot).collect();
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            drawn_by(&gpu, &panel(rule), &transforms, (width, height), format)
        };
        let (with, without) = (draw(true), draw(false));
        crate::test_support::preview("TOMORI_CURVED_RULE_PREVIEW", &with);
        let added =
            |x: u32, y: u32| brightness(&with, x, y).saturating_sub(brightness(&without, x, y));
        let canvas = panel(true);
        let (w, h) = canvas.size_in_pixels();
        // The rule's middle line, in the panel's plane, at `share` of its length.
        let y = (canvas.padding.1 + CELL_HEIGHT) as f32 + 6.0;
        let along = |share: f32| {
            let x = canvas.padding.0 as f32 + share * (30 * CELL_WIDTH) as f32;
            glam::Vec3::new(
                (x - w as f32 / 2.0) * canvas.pixel,
                (h as f32 / 2.0 - y) * canvas.pixel,
                0.0,
            )
        };
        for slot in shown {
            let screen = |point: glam::Vec3| {
                let clip = view * *slot * point.extend(1.0);
                let ndc = clip.truncate() / clip.w;
                glam::Vec2::new(
                    (ndc.x + 1.0) / 2.0 * width as f32,
                    (1.0 - ndc.y) / 2.0 * height as f32,
                )
            };
            let (start, end) = (screen(along(0.0)), screen(along(1.0)));
            assert!(
                end.x - start.x > 60.0,
                "the panel is in view: {start} {end}"
            );
            // Down each column the rule crosses, near where its slot puts it, what it adds: some all along, and no
            // column much more or less than its neighbour, as a seam would be, perspective changing it only slowly.
            let sums: Vec<u32> = (start.x as u32 + 4..end.x as u32 - 4)
                .map(|x| {
                    let t = (x as f32 - start.x) / (end.x - start.x);
                    let middle = start.y + (end.y - start.y) * t;
                    (middle as u32 - 4..middle as u32 + 5)
                        .map(|y| added(x, y))
                        .sum()
                })
                .collect();
            assert!(
                sums.iter().all(|&sum| sum > 40),
                "the rule follows its slot: {sums:?}"
            );
            for pair in sums.windows(2) {
                let (a, b) = (pair[0] as f32, pair[1] as f32);
                assert!((a / b - 1.0).abs() < 0.2, "a seam: {sums:?}");
            }
        }
    });
}
