// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Chart panels on lavapipe: a chart's lines, shaded areas and labels drawn in its panel's own mesh, legible in a wall's
//! slot, following a curved wall's slots, and, far away, steady as they move.
use super::*;
use tomori_protocol::chart::Chart;
use tomori_scene::chart::fixtures;
use tomori_scene::projection::{Fit, face};

/// A chart panel of `chart`, titled `title`, in a slot.
fn panel(title: &str, chart: Chart) -> tomori_protocol::Panel {
    tomori_protocol::Panel::new(
        "panel-1",
        title,
        chart.summary(),
        tomori_protocol::Place::Wall {
            wall: tomori_protocol::TOMORI_WALL.into(),
            slot: 0,
        },
    )
    .with_chart(chart)
}

/// The charts an operations console shows, in each state.
fn charts() -> Vec<(&'static str, Chart)> {
    vec![
        ("Scaling pressure", fixtures::pressure()),
        ("Provisioning", fixtures::provisioning()),
        ("Root admissions", fixtures::admissions()),
        ("Scaling pressure", fixtures::unmeasured()),
        ("Scaling pressure", fixtures::stale()),
        ("Shapes", fixtures::sampler()),
        ("Swinging pressure", fixtures::swinging()),
        ("Many steps", fixtures::stepping()),
        ("Replicas", fixtures::tracking()),
    ]
}

/// How much of `image`'s pixels is covered, in all, drawn into a linear target in white.
fn coverage(image: &image::RgbaImage) -> f32 {
    image
        .pixels()
        .map(|pixel| f32::from(pixel.0[0]) / 255.0)
        .sum()
}

/// `canvas` drawn face on, a font pixel `scale` screen pixels, its pixels off the screen's grid, into a panel whose
/// buffer holds what a wall's panel's does, as the renderer's `set_cards` makes them.
fn on_card(gpu: &crate::test_gpu::Gpu, canvas: &Canvas, scale: f32) -> image::RgbaImage {
    let (columns, rows) = canvas.size_in_pixels();
    let (width, height) = (
        ((columns as f32 * scale) as u32 + 16).next_multiple_of(64),
        (rows as f32 * scale) as u32 + 16,
    );
    let per_metre = scale / canvas.pixel;
    let shifted = 2.0 * 0.37 / glam::Vec2::new(width as f32, -(height as f32));
    let transform = Mat4::from_translation(shifted.extend(0.0))
        * Mat4::from_scale(glam::Vec3::new(
            2.0 * per_metre / width as f32,
            2.0 * per_metre / height as f32,
            1.0,
        ));
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let capacity = crate::scene_graphics::CARD_CAPACITY;
    drawn_within(gpu, canvas, &[transform], (width, height), format, capacity)
}

/// `images` side by side in rows of `across`.
fn sheet(images: &[image::RgbaImage], across: usize) -> image::RgbaImage {
    let (width, height) = (
        images.iter().map(|image| image.width()).max().unwrap(),
        images.iter().map(|image| image.height()).max().unwrap(),
    );
    let rows = images.len().div_ceil(across) as u32;
    let mut sheet = image::RgbaImage::new(width * across as u32, height * rows);
    for (index, image) in images.iter().enumerate() {
        let (x, y) = (
            (index % across) as u32 * width,
            (index / across) as u32 * height,
        );
        image::imageops::overlay(&mut sheet, image, i64::from(x), i64::from(y));
    }
    sheet
}

/// Set `TOMORI_CHART_PREVIEW` to a PNG path to save what this renders: each chart in a standard slot, face on, at about
/// the headset's resolution up close, its pixels off the screen's grid, in a panel whose buffer holds what a wall's
/// panel's does, so that nothing of it, its text last, is cut.
#[test]
fn a_chart_panel_draws_its_lines_shading_and_labels_in_its_slot() {
    crate::test_gpu::with(|gpu| {
        let images: Vec<image::RgbaImage> = charts()
            .into_iter()
            .map(|(title, chart)| {
                let canvas = face(&panel(title, chart), false, false, Fit::SLOT);
                on_card(&gpu, &canvas, 2.3)
            })
            .collect();
        let sheet = sheet(&images, 2);
        crate::test_support::preview("TOMORI_CHART_PREVIEW", &sheet);
        // The pressure chart: its teal mean, its amber greatest and the shaded area above its target all drawn.
        let canvas = face(
            &panel("Scaling pressure", fixtures::pressure()),
            false,
            false,
            Fit::SLOT,
        );
        let image = on_card(&gpu, &canvas, 2.3);
        // Its text is all drawn: the same pixels as a panel whose buffer holds far more.
        assert_eq!(image, drawn_face_on(&gpu, &canvas, 2.3, 0.37).0);
        let near = |want: [u8; 3], within: i32| {
            image.pixels().any(|pixel| {
                (0..3).all(|channel| {
                    (i32::from(pixel.0[channel]) - i32::from(want[channel])).abs() <= within
                })
            })
        };
        let rgb = |colour: tomori_protocol::chart::Colour| {
            colour
                .rgb()
                .map(|value| (value.powf(1.0 / 2.2) * 255.0) as u8)
        };
        assert!(
            near(rgb(tomori_protocol::chart::Colour::Teal), 24),
            "the mean is drawn"
        );
        assert!(
            near(rgb(tomori_protocol::chart::Colour::Amber), 24),
            "the greatest is drawn"
        );
        let reddish = image.pixels().filter(|pixel| {
            let [r, g, b, _] = pixel.0;
            r > 80 && r > g.saturating_add(30) && r > b.saturating_add(30)
        });
        assert!(reddish.count() > 200, "the area above the target is shaded");
    });
}

/// Set `TOMORI_CURVED_CHART_PREVIEW` to a PNG path to save what this renders: the console's charts in the slots of the
/// curved Tomori wall, from home, as a participant reads them, and `TOMORI_FAR_CHART_PREVIEW` the pressure chart on a
/// flat wall 4 m away, the walls' reading distance.
#[test]
fn chart_panels_follow_a_curved_wall_s_slots_and_read_from_the_reading_distance() {
    crate::test_gpu::with(|gpu| {
        let (width, height) = (1536, 1024);
        let wall = tomori_scene::scene::fixtures::tomori_wall();
        let frame = Mat4::from_rotation_translation(
            glam::Quat::from_array(wall.pose.orientation),
            glam::Vec3::from_array(wall.pose.position),
        );
        let slots = tomori_scene::projection::slot_frames(&wall, frame);
        let eye = glam::Vec3::new(0.0, 1.25, 0.0);
        let view = glam::camera::rh::proj::directx::perspective(
            0.9,
            width as f32 / height as f32,
            0.05,
            50.0,
        ) * glam::camera::rh::view::look_at_mat4(
            eye,
            glam::Vec3::new(0.0, 0.52, -2.3),
            glam::Vec3::Y,
        );
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let scale = Mat4::from_scale(glam::Vec3::splat(wall.scale()));
        let mut composed = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        for (slot, (title, chart)) in slots.iter().zip(charts()) {
            let canvas = face(&panel(title, chart), false, false, Fit::SLOT);
            let drawn = drawn_by(
                &gpu,
                &canvas,
                &[view * *slot * scale],
                (width, height),
                format,
            );
            for (pixel, drawn) in composed.pixels_mut().zip(drawn.pixels()) {
                if drawn.0[..3].iter().any(|&channel| channel > 0) {
                    *pixel = *drawn;
                }
            }
        }
        crate::test_support::preview("TOMORI_CURVED_CHART_PREVIEW", &composed);
        // Every slot shows its chart: something drawn about each slot's middle.
        for slot in &slots {
            let clip = view * slot.w_axis;
            let ndc = clip.truncate() / clip.w;
            let (x, y) = (
                ((ndc.x + 1.0) / 2.0 * width as f32) as u32,
                ((1.0 - ndc.y) / 2.0 * height as f32) as u32,
            );
            let lit = (x.saturating_sub(20)..x + 20)
                .flat_map(|x| (y.saturating_sub(20)..y + 20).map(move |y| (x, y)))
                .filter(|&(x, y)| brightness(&composed, x, y) > 60)
                .count();
            assert!(lit > 20, "a slot's chart is drawn: {lit}");
        }
        // A flat wall's slot 4 m away, face on.
        let canvas = face(
            &panel("Scaling pressure", fixtures::pressure()),
            false,
            false,
            Fit::SLOT,
        );
        let far = glam::camera::rh::proj::directx::perspective(
            0.9,
            width as f32 / height as f32,
            0.05,
            50.0,
        ) * glam::camera::rh::view::look_at_mat4(
            glam::Vec3::new(0.0, 1.6, 4.0),
            glam::Vec3::new(0.0, 1.6, 0.0),
            glam::Vec3::Y,
        ) * Mat4::from_translation(glam::Vec3::new(0.0, 1.6, 0.0));
        let capacity = crate::scene_graphics::CARD_CAPACITY;
        let drawn = drawn_within(&gpu, &canvas, &[far], (width, height), format, capacity);
        crate::test_support::preview("TOMORI_FAR_CHART_PREVIEW", &drawn);
        assert!(drawn.pixels().any(|pixel| pixel.0[1] > 120));
    });
}

#[test]
fn a_thin_slanted_chart_line_far_away_keeps_its_ink_as_it_moves_rather_than_shimmering() {
    crate::test_gpu::with(|gpu| {
        // A zigzag of thin lines at many angles, with round joins, on no backing, in white.
        let mut canvas = Canvas::new(40, 0.001);
        canvas.background = [0.0; 4];
        canvas.rows = vec![tomori_scene::ui_text::Row::default(); 6];
        // As a chart draws its lines: one stroke a segment, overlapping its neighbours by half its width.
        let points: Vec<[f32; 2]> = (0..12)
            .map(|index| {
                let x = 10.0 + index as f32 * 28.0;
                [
                    x,
                    if index % 2 == 0 {
                        10.0
                    } else {
                        10.0 + 6.0 * index as f32
                    },
                ]
            })
            .collect();
        for pair in points.windows(2) {
            canvas.shapes.push(tomori_scene::shapes::Shape::Stroke {
                from: pair[0],
                to: pair[1],
                width: 1.25,
                color: [1.0; 4],
            });
        }
        // At four tenths and a third of a screen pixel to a font pixel, as a chart across the room, moved by eighths of
        // a pixel: the lines cover as much wherever they fall, within a tenth.
        for scale in [0.4, 0.33] {
            let inks: Vec<f32> = (0..8)
                .map(|step| {
                    let format = wgpu::TextureFormat::Rgba8Unorm;
                    coverage(&face_on_in(&gpu, &canvas, scale, step as f32 / 8.0, format).0)
                })
                .collect();
            let mean = inks.iter().sum::<f32>() / inks.len() as f32;
            let (least, most) = inks
                .iter()
                .fold((f32::MAX, 0.0_f32), |(least, most), &ink| {
                    (least.min(ink), most.max(ink))
                });
            assert!(
                mean > 0.0 && (most - least) / mean < 0.1,
                "{scale}: {inks:?}"
            );
        }
    });
}

/// Every way a chart panel's face is sized: a standard slot, the widest and tallest slots a wall may have, and floating.
fn fits() -> Vec<Fit> {
    let wall = |aspect: f32| {
        tomori_protocol::Wall::new(
            "w",
            tomori_protocol::WallKind::Added,
            tomori_protocol::Pose::default(),
        )
        .with_aspect(Some(aspect))
    };
    let (wide, tall) = (
        tomori_scene::projection::Cells::of(&wall(tomori_protocol::wall::ASPECTS.1)),
        tomori_scene::projection::Cells::of(&wall(tomori_protocol::wall::ASPECTS.0)),
    );
    vec![Fit::SLOT, Fit::Slot(wide), Fit::Slot(tall), Fit::Text]
}

#[test]
fn every_chart_at_the_spec_s_bounds_fits_a_wall_panel_s_buffer_whole() {
    let charts = [
        fixtures::pressure(),
        fixtures::provisioning(),
        fixtures::largest(false),
        fixtures::largest(true),
        fixtures::swinging(),
        fixtures::stepping(),
        fixtures::tracking(),
        fixtures::regions(),
        fixtures::bars(2),
        fixtures::bars(4),
        fixtures::bars(8),
    ];
    // Uploaded into a buffer of a wall card's size, as the renderer makes one, none of it is cut.
    crate::test_gpu::with(|gpu| {
        let glyphs = crate::glyphs::Glyphs::new(&gpu.device);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let pipeline = Pipeline::new(&gpu.device, format, &glyphs, 1);
        let capacity = crate::scene_graphics::CARD_CAPACITY;
        let mut card = Panel::new(&gpu.device, &pipeline, capacity);
        for chart in charts {
            for fit in fits() {
                for highlighted in [false, true] {
                    let canvas = face(
                        &panel(&"W".repeat(80), chart.clone()),
                        highlighted,
                        true,
                        fit,
                    );
                    let vertices = canvas.mesh().len();
                    assert!(
                        vertices <= capacity,
                        "{vertices} vertices of {capacity} in {fit:?}"
                    );
                    card.set(&gpu.queue, &canvas);
                    assert!(!card.cut, "{fit:?}");
                    assert_eq!(card.count as usize, vertices);
                }
            }
        }
    });
}

#[test]
fn a_face_too_large_for_its_buffer_is_cut_and_said_once() {
    crate::test_gpu::with(|gpu| {
        let glyphs = crate::glyphs::Glyphs::new(&gpu.device);
        let pipeline = Pipeline::new(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, &glyphs, 1);
        let mut panel = Panel::new(&gpu.device, &pipeline, 64);
        let mut canvas = Canvas::new(20, 0.001);
        canvas.rows = vec![tomori_scene::ui_text::Row::text(
            "Too much text for its buffer",
            tomori_scene::ui_text::Style::plain([1.0; 4]),
            20,
        )];
        panel.set(&gpu.queue, &canvas);
        assert!(panel.cut);
        assert_eq!(panel.count, 60);
        canvas.rows.clear();
        panel.set(&gpu.queue, &canvas);
        assert!(!panel.cut);
    });
}

/// Set `TOMORI_WIDE_CHART_PREVIEW` to a PNG path to save what this renders: the pressure, provisioning and replicas charts in
/// wide slots, 3:1 and 4:1, as a console's charts want, face on, and `TOMORI_FIVE_CHART_PREVIEW` the pressure chart in a
/// 4:1 slot of a wall five times a standard slot's size, seen from 8 m.
#[test]
fn a_chart_in_a_wide_slot_and_on_a_wall_five_times_the_size_lays_out_its_width() {
    crate::test_gpu::with(|gpu| {
        let wide = |aspect: f32| {
            tomori_scene::projection::Cells::of(
                &tomori_protocol::Wall::new(
                    "w",
                    tomori_protocol::WallKind::Added,
                    tomori_protocol::Pose::default(),
                )
                .with_aspect(Some(aspect)),
            )
        };
        let mut images = Vec::new();
        for aspect in [3.0, 4.0] {
            for (title, chart) in [
                ("Scaling pressure", fixtures::pressure()),
                ("Provisioning", fixtures::provisioning()),
                ("Replicas", fixtures::tracking()),
            ] {
                let canvas = face(&panel(title, chart), false, false, Fit::Slot(wide(aspect)));
                let (width, height) = canvas.size();
                assert!(
                    (width / height - aspect).abs() < 0.35,
                    "{aspect}: {width} by {height}"
                );
                // Every axis label shows in a wide slot, none left out for want of room.
                if title == "Scaling pressure" {
                    let said: Vec<String> = canvas
                        .labels
                        .iter()
                        .map(|label| tomori_scene::ui_text::plain(&label.cells))
                        .collect();
                    for text in ["-15 min", "-10 min", "-5 min", "now"] {
                        assert!(said.contains(&text.to_owned()), "{aspect}: {said:?}");
                    }
                }
                images.push(on_card(&gpu, &canvas, 1.6));
            }
        }
        crate::test_support::preview("TOMORI_WIDE_CHART_PREVIEW", &sheet(&images, 3));
        // On a wall five times a standard slot's size, a 4:1 slot seen from 8 m.
        let (width, height) = (1536, 1024);
        let canvas = face(
            &panel("Scaling pressure", fixtures::pressure()),
            false,
            false,
            Fit::Slot(wide(4.0)),
        );
        let view = glam::camera::rh::proj::directx::perspective(
            0.9,
            width as f32 / height as f32,
            0.05,
            50.0,
        ) * glam::camera::rh::view::look_at_mat4(
            glam::Vec3::new(0.0, 1.6, 8.0),
            glam::Vec3::new(0.0, 1.6, 0.0),
            glam::Vec3::Y,
        ) * Mat4::from_translation(glam::Vec3::new(0.0, 1.6, 0.0))
            * Mat4::from_scale(glam::Vec3::splat(5.0));
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let capacity = crate::scene_graphics::CARD_CAPACITY;
        let drawn = drawn_within(&gpu, &canvas, &[view], (width, height), format, capacity);
        crate::test_support::preview("TOMORI_FIVE_CHART_PREVIEW", &drawn);
        assert!(drawn.pixels().filter(|pixel| pixel.0[1] > 120).count() > 1000);
    });
}

/// Set `TOMORI_EIGHT_CHART_PREVIEW` to a PNG path to save what this renders: requests in eight regions, a line each in
/// the palette's eight colours, the fifth to the eighth marked, read by pointing at its plot, in a standard slot, the
/// same in a 4:1 slot and floating, and eight regions' bars, each face on in a panel whose buffer holds what a wall's
/// panel's does.
#[test]
fn a_chart_of_eight_series_tells_each_apart_and_reads_them_all_where_pointed_at() {
    use tomori_scene::chart::{Area, draw, legend, readout::Readout};
    use tomori_scene::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    crate::test_gpu::with(|gpu| {
        let wide = tomori_scene::projection::Cells::of(
            &tomori_protocol::Wall::new(
                "w",
                tomori_protocol::WallKind::Added,
                tomori_protocol::Pose::default(),
            )
            .with_aspect(Some(4.0)),
        );
        let chart = fixtures::regions();
        let mut images = Vec::new();
        for fit in [Fit::SLOT, Fit::Slot(wide), Fit::Text] {
            let canvas = face(
                &panel("Requests by region", chart.clone()),
                false,
                false,
                fit,
            );
            assert!(canvas.mesh().len() <= crate::scene_graphics::CARD_CAPACITY);
            let mut image = on_card(&gpu, &canvas, 2.3);
            if fit == Fit::SLOT {
                // Its readout, as the face lays out its plot, over the face, at about two thirds of the way along.
                let columns = canvas.columns;
                let top = 3 + legend(&chart, columns).len();
                let plot = draw(
                    &chart,
                    Area {
                        x: 0.0,
                        y: (top * CELL_HEIGHT) as f32,
                        width: (columns * CELL_WIDTH) as f32,
                        height: ((canvas.rows.len() - top) * CELL_HEIGHT) as f32,
                    },
                )
                .plot
                .unwrap();
                let bounds = (
                    (columns * CELL_WIDTH) as f32,
                    (canvas.rows.len() * CELL_HEIGHT) as f32,
                );
                let readout = Readout::new(&chart, &plot, 120.0, bounds);
                assert_eq!(readout.lines.len(), tomori_scene::chart::readout::LINES);
                // Drawn over the face, as the renderer draws a readout's overlay over the face kept.
                let overlay = on_card(&gpu, &readout.overlay(&canvas), 2.3);
                for (pixel, over) in image.pixels_mut().zip(overlay.pixels()) {
                    if over.0[..3].iter().any(|&channel| channel > 0) {
                        *pixel = *over;
                    }
                }
            }
            images.push(image);
        }
        crate::test_support::preview("TOMORI_EIGHT_CHART_PREVIEW", &sheet(&images, 2));
        // Every series' colour is drawn in the standard slot.
        let image = &images[0];
        let rgb = |colour: tomori_protocol::chart::Colour| {
            colour
                .rgb()
                .map(|value| (value.powf(1.0 / 2.2) * 255.0) as i32)
        };
        for (name, colour) in tomori_protocol::chart::Colour::NAMED {
            let want = rgb(colour);
            let drawn = image
                .pixels()
                .filter(|pixel| {
                    (0..3).all(|channel| (i32::from(pixel.0[channel]) - want[channel]).abs() <= 24)
                })
                .count();
            assert!(drawn > 20, "{name}: {drawn}");
        }
    });
}

/// Set `TOMORI_BARS_CHART_PREVIEW` to a PNG path to save what this renders: requests in two, four and eight regions as
/// bars at every one of 256 points, side by side in each stretch, in a standard slot and a 4:1 slot, face on.
#[test]
fn several_series_of_bars_stand_side_by_side_and_each_is_seen() {
    crate::test_gpu::with(|gpu| {
        let wide = tomori_scene::projection::Cells::of(
            &tomori_protocol::Wall::new(
                "w",
                tomori_protocol::WallKind::Added,
                tomori_protocol::Pose::default(),
            )
            .with_aspect(Some(4.0)),
        );
        let mut images = Vec::new();
        for count in [2, 4, 8] {
            for fit in [Fit::SLOT, Fit::Slot(wide)] {
                let chart = fixtures::bars(count);
                let canvas = face(
                    &panel("Requests by region", chart.clone()),
                    false,
                    false,
                    fit,
                );
                let image = on_card(&gpu, &canvas, 2.3);
                // Every series' colour is seen, none hidden behind another's bars.
                for colour in &chart.series {
                    let want = colour
                        .colour
                        .rgb()
                        .map(|value| (value.powf(1.0 / 2.2) * 255.0) as i32);
                    let seen = image
                        .pixels()
                        .filter(|pixel| {
                            (0..3).all(|channel| {
                                (i32::from(pixel.0[channel]) - want[channel]).abs() <= 24
                            })
                        })
                        .count();
                    assert!(seen > 200, "{count} {}: {seen}", colour.name);
                }
                images.push(image);
            }
        }
        crate::test_support::preview("TOMORI_BARS_CHART_PREVIEW", &sheet(&images, 2));
    });
}
