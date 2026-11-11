// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A chart's readout as a participant sees it, pointing at the plot of a chart floating in front of them: the line
//! down the plot at the time pointed at, a dot on the series there, and the label beside it saying the time and the
//! value, drawn by the headset's renderer over the chart's face, which is neither laid out nor meshed again. See
//! `design/projection.md`.
use super::*;

/// The eye, a little wider than tall.
const EYE: (u32, u32) = (1280, 896);

/// Where the chart floats, facing home.
const AHEAD: Vec3 = Vec3::new(0.0, 1.6, -1.1);

/// Set `TOMORI_CHART_READOUT_PREVIEW` to a PNG path to save what this renders: the root admissions chart floating in
/// front of home, the ray from the right hand on its plot, about six minutes ago, and its readout there.
#[test]
fn pointing_at_a_chart_s_plot_draws_its_readout_over_its_face() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chart readout eye"),
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
        let chart = tomori_scene::chart::fixtures::admissions();
        let panel = tomori_protocol::Panel::new(
            "panel-1",
            "Root admissions",
            chart.summary(),
            tomori_protocol::Place::Free {
                pose: tomori_protocol::Pose::upright_facing(
                    AHEAD.to_array(),
                    arrival::HOME.to_array(),
                ),
            },
        )
        .with_chart(chart);
        let mut conversation = joined();
        show_projection(&mut conversation, Vec::new(), vec![panel]);
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
        // The chart's face, where it floats, and a place on its plot, 60% across and two thirds down its face.
        let (sheets, _) = frame_with(None, &mut renderer, &mut panels);
        let (face, model) = sheets
            .iter()
            .find_map(|(canvas, model)| {
                canvas
                    .as_ref()
                    .filter(|canvas| {
                        canvas.rows.len() > 4
                            && canvas.rows[0].plain().starts_with("Root admissions")
                    })
                    .map(|canvas| (canvas.clone(), model.unwrap()))
            })
            .expect("the chart is drawn");
        let (width, height) = face.size();
        let on_plot = model.transform_point3(Vec3::new(width * 0.1, -height * 0.16, 0.0));
        let lens =
            Mat4::perspective_rh(46_f32.to_radians(), EYE.0 as f32 / EYE.1 as f32, 0.05, 50.0);
        let matrix = lens * Mat4::look_at_rh(arrival::HOME, AHEAD, Vec3::Y);
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
        // Pointed at from the hand, its readout is drawn over it, in its plane, beside the label on the ray's place.
        let ray = tomori_scene::pointing::Ray::new(hand, on_plot - hand);
        frame_with(ray, &mut renderer, &mut panels);
        let (sheets, _) = frame_with(ray, &mut renderer, &mut panels);
        let readout = sheets
            .iter()
            .find_map(|(canvas, model)| {
                canvas
                    .as_ref()
                    .filter(|canvas| canvas.background[3] == 0.0 && !canvas.labels.is_empty())
                    .map(|canvas| (canvas.clone(), model.unwrap()))
            })
            .expect("the readout is drawn");
        let lines: Vec<String> = readout
            .0
            .labels
            .iter()
            .map(|label| tomori_scene::ui_text::plain(&label.cells))
            .collect();
        assert!(lines[0].starts_with("-6 min"), "{lines:?}");
        assert!(
            lines[1].starts_with("admitted ") && lines[1].ends_with(" a minute"),
            "{lines:?}"
        );
        assert_eq!(readout.0.size(), face.size());
        let pointed = see(&mut renderer, &panels);
        // Still, nothing more is drawn or meshed: neither the face nor the readout.
        let (_, uploads) = frame_with(ray, &mut renderer, &mut panels);
        assert_eq!(uploads.built, 0, "{uploads:?}");
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        // The chart where it is seen, a little beyond its edges.
        let pixel = |point: Vec3| {
            let clip = matrix * point.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            (
                ((ndc.x + 1.0) / 2.0 * EYE.0 as f32) as u32,
                ((1.0 - ndc.y) / 2.0 * EYE.1 as f32) as u32,
            )
        };
        let (left, top) = pixel(model.transform_point3(Vec3::new(-width / 2.0, height / 2.0, 0.0)));
        let (right, bottom) =
            pixel(model.transform_point3(Vec3::new(width / 2.0, -height / 2.0, 0.0)));
        let crop = |image: &image::RgbaImage, margin: u32| {
            let (x, y) = (left.saturating_sub(margin), top.saturating_sub(margin));
            image::imageops::crop_imm(image, x, y, right + margin - x, bottom + margin - y)
                .to_image()
        };
        crate::test_support::preview("TOMORI_CHART_READOUT_PREVIEW", &crop(&pointed, 12));
        let (bare, pointed) = (crop(&bare, 0), crop(&pointed, 0));
        // On the chart, the readout changes it only about where it points: its line, its dot and its label.
        let changed: Vec<(u32, u32)> = bare
            .enumerate_pixels()
            .filter(|(x, y, pixel)| {
                let other = pointed.get_pixel(*x, *y);
                (0..3).any(|channel| pixel.0[channel].abs_diff(other.0[channel]) > 40)
            })
            .map(|(x, y, _)| (x, y))
            .collect();
        assert!(changed.len() > 1500, "{} pixels changed", changed.len());
        let (line, _) = pixel(on_plot);
        let line = line - left;
        let near = changed
            .iter()
            .filter(|(x, _)| x.abs_diff(line) < 420)
            .count();
        assert!(
            near * 10 > changed.len() * 9,
            "{near} of {} near the line",
            changed.len()
        );
        // Its line is drawn bright down the plot where it points.
        let bright = (0..pointed.height())
            .filter(|y| {
                (line.saturating_sub(3)..=line + 3).any(|x| {
                    let pixel = pointed.get_pixel(x, *y).0;
                    pixel[0] > 150 && pixel[1] > 150 && pixel[2] > 150
                })
            })
            .count();
        assert!(bright > 120, "{bright} rows of the line");
    });
}
