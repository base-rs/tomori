// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The back of a chart panel as a participant walking behind it sees it: opaque, in its place, its text facing out of
//! the back and reading left to right, never the chart mirrored, saying where the chart comes from, its query, context,
//! refresh and how long ago it was updated. See `design/panel-backs.md`.
use super::*;
use tomori_protocol::back::{Back, BackCode, BackLine, DEFINITION, OUTPUTS, STATUS};
use tomori_protocol::chart::{AxisLabel, Chart, Colour, Series, SeriesStyle, TimeAxis, ValueAxis};
use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};

/// Each eye's view, wide enough for the panel and a margin.
const EYE: (u32, u32) = (1152, 768);

/// Where the chart floats, behind home and facing it, clear of the swarm ahead.
const AT: Vec3 = Vec3::new(0.0, 1.6, 1.6);

/// A live p99 latency chart of the last 15 minutes at 5 s, as Observe's terminals show one.
fn latency() -> Chart {
    let values = (0..181)
        .map(|index| Some(40.0 + 8.0 * (index as f32 / 9.0).sin()))
        .collect();
    Chart::new(181)
        .with_labels(vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(180, "now"),
        ])
        .with_axis(ValueAxis::new(Some(0.0), None, "ms"))
        .with_series(Series::new("p99", Colour::Amber, SeriesStyle::Area, values))
        .with_time(TimeAxis::Live { step_ms: 5_000 })
}

/// What `canvas` says, a line a row.
fn plain(canvas: &tomori_scene::canvas::Canvas) -> String {
    let rows: Vec<String> = canvas.rows.iter().map(|row| row.plain()).collect();
    rows.join("\n")
}

/// Set `TOMORI_PANEL_BACK_PREVIEW` to a PNG path to save what this renders: a live chart floating ahead, from in front,
/// and then from behind it, its back, side by side.
#[test]
fn a_chart_seen_from_behind_shows_its_back_opaque_reading_left_to_right_and_not_the_chart_mirrored()
{
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("panel back eye"),
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
        let chart = latency();
        let panel = tomori_protocol::Panel::new(
            "latency",
            "Latency p99 · US-EAST1",
            chart.summary(),
            tomori_protocol::Place::Free {
                pose: tomori_protocol::Pose::upright_facing(AT.to_array(), arrival::HOME.to_array()),
            },
        )
        .with_chart(chart)
        .with_context(vec![
            ContextChip::new(
                "region",
                "Region",
                ContextValue::Text {
                    text: "US-EAST1".into(),
                },
                ContextSetting::Inherited,
            ),
            ContextChip::new(
                tomori_protocol::context::TIME,
                "Time",
                ContextValue::Live { minutes: 15 },
                ContextSetting::Follows,
            ),
        ])
        .with_back(Some(
            Back::new()
                .with_line(DEFINITION, BackLine::new("Source", "Observe · Prometheus"))
                .with_line(DEFINITION, BackLine::new("Scope", "production, all pods"))
                .with_code(
                    DEFINITION,
                    BackCode::new(
                        "promql",
                        "histogram_quantile(0.99, sum by (le) (rate(http_request_duration_seconds_bucket{region=\"us-east1\"}[5m])))",
                    ),
                )
                .with_line(STATUS, BackLine::new("Refresh", "every 15 s"))
                .with_line(OUTPUTS, BackLine::new("Copy of", "Latency p99")),
        ))
        .with_updated(Some(12));
        let mut conversation = joined();
        show_projection(&mut conversation, Vec::new(), vec![panel]);
        let lens = glam::camera::rh::proj::directx::perspective(
            44_f32.to_radians(),
            EYE.0 as f32 / EYE.1 as f32,
            0.05,
            50.0,
        );
        let behind = AT + Vec3::Z * 1.6;
        let mut step = Step::default();
        let start = Instant::now();
        let mut look_from = |eye: Vec3, renderer: &mut Renderer, panels: &mut Panels| {
            let head = glam::camera::rh::view::look_at_mat4(eye, AT, Vec3::Y).inverse();
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(eye),
                space_head: head,
                ..Default::default()
            };
            let mut frame = None;
            for _ in 0..3 {
                frame = Some(step.update(&senses, &mut conversation, &mut felt(), start, 0.01));
            }
            let frame = frame.unwrap();
            draw(renderer, &device, &queue, panels, &frame);
            let sheet = frame
                .projection
                .iter()
                .find(|sheet| {
                    sheet
                        .canvas
                        .as_deref()
                        .is_some_and(|canvas| plain(canvas).starts_with("Latency p99"))
                })
                .map(|sheet| (sheet.canvas.clone().unwrap(), sheet.model.unwrap()))
                .expect("the chart is drawn");
            let matrix = lens * glam::camera::rh::view::look_at_mat4(eye, AT, Vec3::Y);
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            (read_back(&device, &queue, &texture, EYE), sheet, matrix)
        };
        let (front, (face, _), _) = look_from(arrival::HOME, &mut renderer, &mut panels);
        let (back, (canvas, model), matrix) = look_from(behind, &mut renderer, &mut panels);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        let mut sheet = image::RgbaImage::new(EYE.0 * 2, EYE.1);
        image::imageops::replace(&mut sheet, &front, 0, 0);
        image::imageops::replace(&mut sheet, &back, i64::from(EYE.0), 0);
        crate::test_support::preview("TOMORI_PANEL_BACK_PREVIEW", &sheet);
        // What is drawn from behind is the panel's back, as large as its face, saying how it is configured.
        let text = plain(&canvas);
        for said in [
            "Back · configuration",
            "Inputs",
            "Outputs",
            "Definition",
            "Status",
            "Copy of",
            "Observe · Prometheus",
            "US-EAST1 · inherited from its wall",
            "every 5 s",
            "every 15 s",
            "12 s ago",
            "promql",
            "histogram_quantile",
        ] {
            assert!(text.contains(said), "{said} in {text}");
        }
        assert!((canvas.size().0 - face.size().0).abs() < 1e-3);
        // Where it stands on the image, from behind.
        let pixel = |point: Vec3| {
            let clip = matrix * point.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            (
                ((ndc.x + 1.0) / 2.0 * EYE.0 as f32) as u32,
                ((1.0 - ndc.y) / 2.0 * EYE.1 as f32) as u32,
            )
        };
        let (width, height) = canvas.size();
        let (left, top) = pixel(model.transform_point3(Vec3::new(-width / 2.0, height / 2.0, 0.0)));
        let (right, bottom) =
            pixel(model.transform_point3(Vec3::new(width / 2.0, -height / 2.0, 0.0)));
        assert!(
            left < right && top < bottom,
            "facing the viewer: {left}..{right}"
        );
        let count = |image: &image::RgbaImage,
                     (x0, x1): (u32, u32),
                     (y0, y1): (u32, u32),
                     test: &dyn Fn([u8; 4]) -> bool| {
            (x0..x1)
                .flat_map(|x| (y0..y1).map(move |y| (x, y)))
                .filter(|(x, y)| test(image.get_pixel(*x, *y).0))
                .count()
        };
        let bright = |pixel: [u8; 4]| pixel[..3].iter().all(|channel| *channel > 170);
        let amber =
            |pixel: [u8; 4]| pixel[0] > 100 && u16::from(pixel[0]) > u16::from(pixel[2]) + 50;
        let area = ((right - left) * (bottom - top)) as usize;
        // Opaque: its plate covers it, the black of the space behind showing nowhere inside its frame.
        let plate = count(
            &back,
            (left + 4, right - 4),
            (top + 4, bottom - 4),
            &|pixel| pixel[2] > pixel[0] && pixel[..3].iter().any(|channel| *channel > 0),
        );
        assert!(plate > area * 6 / 10, "{plate} of {area}");
        // Its title reads left to right: its row's ink is at its left, where a mirrored face's would be at its right.
        let row = (bottom - top) / 14;
        let title = (top + row / 2, top + row * 2);
        let middle = (left + right) / 2;
        let (on_left, on_right) = (
            count(&back, (left, middle), title, &bright),
            count(&back, (middle, right), title, &bright),
        );
        assert!(
            on_left > 50 && on_left > on_right * 3,
            "{on_left} {on_right}"
        );
        // None of the chart's amber shows through from its face, which the front draws plainly.
        let front_amber = count(&front, (0, EYE.0), (0, EYE.1), &amber);
        assert!(front_amber > 20_000, "{front_amber}");
        assert!(count(&back, (0, EYE.0), (0, EYE.1), &amber) < front_amber / 50);
    });
}
