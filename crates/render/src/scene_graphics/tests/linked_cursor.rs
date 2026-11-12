// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The linked time cursor as a participant sees it: pointing at the spike on one region's frozen latency chart draws
//! its readout there, and a fainter readout linked to it, at the same clock time, over another region's frozen chart
//! whose window overlaps it, so that a spike in one is read against the other at the same instant, drawn by the
//! headset's renderer over faces neither laid out nor meshed again. See `design/projection.md`.
use super::*;
use tomori_protocol::chart::{AxisLabel, Chart, Colour, Series, SeriesStyle, TimeAxis, ValueAxis};

/// The eye, wide enough for two charts side by side.
const EYE: (u32, u32) = (1600, 760);

/// Where the two charts float, side by side, facing home.
const LEFT: Vec3 = Vec3::new(-0.5, 1.6, -2.0);
const RIGHT: Vec3 = Vec3::new(0.5, 1.6, -2.0);

/// The minute 02:37, ten seconds a point, where both regions spike.
const SPIKE: f32 = 2.0 * 3600.0 + 37.0 * 60.0;

/// 12 November 2026, 00:00 UTC, in milliseconds since the Unix epoch.
const DAY_MS: i64 = 1_794_441_600_000;

/// A region's p99 latency, frozen over the half hour from `from`, in seconds after midnight UTC on 12 November 2026,
/// which its time axis says as absolute instants, every ten minutes labelled in clock times, which are only shown,
/// spiking to `peak` milliseconds about 02:37.
fn latency(from: f32, peak: f32) -> Chart {
    let values = (0..181)
        .map(|index| {
            let seconds = from + index as f32 * 10.0;
            let spike = (-((seconds - SPIKE) / 50.0).powi(2)).exp();
            Some(40.0 + 6.0 * (index as f32 / 7.0).sin() + peak * spike)
        })
        .collect();
    let label = |at: u32| {
        let seconds = from as u32 + at * 10;
        AxisLabel::new(
            at,
            format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60),
        )
    };
    Chart::new(181)
        .with_labels([0, 60, 120, 180].map(label).to_vec())
        .with_axis(ValueAxis::new(Some(0.0), None, "ms"))
        .with_series(Series::new("p99", Colour::Amber, SeriesStyle::Area, values))
        .with_time(TimeAxis::Fixed {
            from_ms: DAY_MS + from as i64 * 1000,
            to_ms: DAY_MS + from as i64 * 1000 + 1_800_000,
        })
}

/// Set `TOMORI_LINKED_CURSOR_PREVIEW` to a PNG path to save what this renders: US-EAST1's frozen latency, 02:20 to
/// 02:50, pointed at on its spike, and EU-WEST1's, 02:30 to 03:00, beside it, its readout linked at the same time.
#[test]
fn pointing_at_a_frozen_chart_draws_a_readout_linked_to_it_over_another_at_the_same_time() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("linked cursor eye"),
            size: wgpu::Extent3d {
                width: EYE.0,
                height: EYE.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut renderer = Renderer::with_samples(
            &device,
            &queue,
            format,
            EYE.0,
            EYE.1,
            crate::graphics::SAMPLES,
        );
        renderer.set_environment(tomori_scene::environment::Environment::MINIMAL);
        let mut panels = Panels::new(&device, &renderer);
        let panel = |id: &str, title: &str, at: Vec3, chart: Chart| {
            tomori_protocol::Panel::new(
                id,
                title,
                chart.summary(),
                tomori_protocol::Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        at.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )
            .with_chart(chart)
        };
        let east = 2.0 * 3600.0 + 20.0 * 60.0;
        let west = 2.0 * 3600.0 + 30.0 * 60.0;
        let mut conversation = joined();
        show_projection(
            &mut conversation,
            Vec::new(),
            vec![
                panel("east", "Latency p99 · US-EAST1", LEFT, latency(east, 180.0)),
                panel("west", "Latency p99 · EU-WEST1", RIGHT, latency(west, 70.0)),
            ],
        );
        let head = Mat4::from_translation(arrival::HOME);
        let hand = arrival::HOME + Vec3::new(0.18, -0.3, -0.25);
        let senses = |ray: Option<tomori_scene::pointing::Ray>| Senses {
            focused: true,
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(arrival::HOME),
            space_head: head,
            beam: ray.is_some(),
            ray,
            ..Default::default()
        };
        let mut step = Step::default();
        let mut frame_with = |ray, renderer: &mut Renderer, panels: &mut Panels| {
            let frame = step.update(
                &senses(ray),
                &mut conversation,
                &mut felt(),
                Instant::now(),
                0.01,
            );
            let uploads = draw(renderer, &device, &queue, panels, &frame);
            (
                frame
                    .projection
                    .iter()
                    .map(|sheet| (sheet.canvas.clone(), sheet.model))
                    .collect::<Vec<_>>(),
                uploads,
            )
        };
        let (sheets, _) = frame_with(None, &mut renderer, &mut panels);
        let face_of = |title: &str| {
            sheets
                .iter()
                .find_map(|(canvas, model)| {
                    canvas
                        .as_ref()
                        .filter(|canvas| {
                            canvas.rows.len() > 4 && canvas.rows[0].plain().starts_with(title)
                        })
                        .map(|canvas| (canvas.clone(), model.unwrap()))
                })
                .expect("the chart is drawn")
        };
        let (east_face, east_model) = face_of("Latency p99 · US-EAST1");
        let (west_face, west_model) = face_of("Latency p99 · EU-WEST1");
        let (width, height) = east_face.size();
        let lens = glam::camera::rh::proj::directx::perspective(
            44_f32.to_radians(),
            EYE.0 as f32 / EYE.1 as f32,
            0.05,
            50.0,
        );
        let middle = (LEFT + RIGHT) / 2.0;
        let matrix = lens * glam::camera::rh::view::look_at_mat4(arrival::HOME, middle, Vec3::Y);
        let see = |renderer: &mut Renderer, panels: &Panels| {
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            read_back(&device, &queue, &texture, EYE)
        };
        let bare = see(&mut renderer, &panels);
        // Scan across the plot until the readout says 02:37, then hold the ray there.
        let mut held = None;
        for step in 0..=200 {
            let x = width * (-0.45 + 0.9 * step as f32 / 200.0);
            let on_plot = east_model.transform_point3(Vec3::new(x, -height * 0.16, 0.0));
            let ray = tomori_scene::pointing::Ray::new(hand, on_plot - hand);
            let (sheets, _) = frame_with(ray, &mut renderer, &mut panels);
            let read: Vec<String> = sheets
                .iter()
                .filter_map(|(canvas, _)| canvas.as_ref())
                .filter(|canvas| canvas.background[3] == 0.0 && !canvas.labels.is_empty())
                .map(|canvas| tomori_scene::ui_text::plain(&canvas.labels[0].cells))
                .collect();
            if read.first().is_some_and(|time| time.starts_with("02:37")) {
                held = Some((ray, read));
                break;
            }
        }
        let (ray, read) = held.expect("the readout reaches 02:37");
        frame_with(ray, &mut renderer, &mut panels);
        let (sheets, _) = frame_with(ray, &mut renderer, &mut panels);
        // Both say the same time, the pointed one's over US-EAST1 and the linked one's over EU-WEST1.
        let overlays: Vec<(Vec<String>, Mat4)> = sheets
            .iter()
            .filter_map(|(canvas, model)| {
                canvas
                    .as_ref()
                    .filter(|canvas| canvas.background[3] == 0.0 && !canvas.labels.is_empty())
                    .map(|canvas| {
                        (
                            canvas
                                .labels
                                .iter()
                                .map(|label| tomori_scene::ui_text::plain(&label.cells))
                                .collect(),
                            model.unwrap(),
                        )
                    })
            })
            .collect();
        assert_eq!(overlays.len(), 2, "{read:?}");
        assert_eq!(overlays[0].0[0], overlays[1].0[0], "{overlays:?}");
        assert!(
            overlays
                .iter()
                .all(|(lines, _)| lines[1].starts_with("p99 "))
        );
        let lifted = |model: Mat4| model.w_axis.truncate();
        assert!(lifted(overlays[1].1).distance(lifted(west_model)) < 0.01);
        let pointed = see(&mut renderer, &panels);
        // Still, nothing more is drawn or meshed.
        let (_, uploads) = frame_with(ray, &mut renderer, &mut panels);
        assert_eq!(uploads.built, 0, "{uploads:?}");
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        let pixel = |point: Vec3| {
            let clip = matrix * point.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            (
                ((ndc.x + 1.0) / 2.0 * EYE.0 as f32) as u32,
                ((1.0 - ndc.y) / 2.0 * EYE.1 as f32) as u32,
            )
        };
        let (left, top) =
            pixel(east_model.transform_point3(Vec3::new(-width / 2.0, height / 2.0, 0.0)));
        let (right, bottom) = pixel(west_model.transform_point3(Vec3::new(
            west_face.size().0 / 2.0,
            -height / 2.0,
            0.0,
        )));
        let (right, bottom) = (right.min(EYE.0 - 1), bottom.min(EYE.1 - 1));
        let crop = |image: &image::RgbaImage, margin: u32| {
            let (x, y) = (left.saturating_sub(margin), top.saturating_sub(margin));
            let (far, low) = ((right + margin).min(EYE.0), (bottom + margin).min(EYE.1));
            image::imageops::crop_imm(image, x, y, far - x, low - y).to_image()
        };
        crate::test_support::preview("TOMORI_LINKED_CURSOR_PREVIEW", &crop(&pointed, 16));
        // EU-WEST1's chart, which nothing points at, is changed by its linked readout.
        let (west_left, _) = pixel(west_model.transform_point3(Vec3::new(-width / 2.0, 0.0, 0.0)));
        let changed = (west_left..right)
            .flat_map(|x| (top..bottom).map(move |y| (x, y)))
            .filter(|(x, y)| {
                let (a, b) = (bare.get_pixel(*x, *y).0, pointed.get_pixel(*x, *y).0);
                (0..3).any(|channel| a[channel].abs_diff(b[channel]) > 40)
            })
            .count();
        assert!(changed > 800, "{changed} pixels of EU-WEST1 changed");
    });
}
