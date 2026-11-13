// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pin and freeze as a participant sees them: a copy frozen to a window, its axis in UTC clock times and its time chip
//! marked `❄`, which unfreezes it pressed, beside a copy pinned to another region, following the time, its time chip
//! freezing it pressed. See `design/panel-context.md`.
use super::*;
use tomori_protocol::context::{ChipPress, ContextChip, ContextSetting, ContextValue};

/// The eye, wide enough for two panels side by side.
const EYE: (u32, u32) = (1408, 720);

/// Where the copies float, between them, facing home.
const AHEAD: Vec3 = Vec3::new(0.0, 1.5, -1.6);

/// 12 November 2026, 02:20 to 02:50 UTC.
const WINDOW: ContextValue = ContextValue::Window {
    from_ms: 1_794_450_000_000,
    to_ms: 1_794_451_800_000,
};

fn region(value: &str, setting: ContextSetting) -> ContextChip {
    ContextChip::new(
        "region",
        "Region",
        ContextValue::Text { text: value.into() },
        setting,
    )
}

/// Set `TOMORI_FREEZE_PREVIEW` to a PNG path to save what this renders: a frozen copy and a pinned one floating.
#[test]
fn a_copy_frozen_and_one_pinned_show_their_state_on_their_chips_and_axis() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chips eye"),
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
        // Two copies floating side by side: one frozen to 02:20–02:50 UTC, its axis in UTC clock times, its time chip
        // saying it unfreezes; one pinned to AP-SOUTH1, following the time, its time chip freezing it.
        let floating = |x: f32| tomori_protocol::Place::Free {
            pose: tomori_protocol::Pose::upright_facing(
                (AHEAD + Vec3::new(x, 0.0, 0.0)).to_array(),
                arrival::HOME.to_array(),
            ),
        };
        let pressure = tomori_scene::chart::fixtures::pressure();
        let points = pressure.points;
        let frozen = pressure
            .clone()
            .with_time(tomori_protocol::chart::TimeAxis::Fixed {
                from_ms: 1_794_450_000_000,
                to_ms: 1_794_451_800_000,
            })
            .with_labels(vec![
                tomori_protocol::chart::AxisLabel::new(0, "11-12 02:20"),
                tomori_protocol::chart::AxisLabel::new(points / 2, "02:35"),
                tomori_protocol::chart::AxisLabel::new(points - 1, "02:50 UTC"),
            ]);
        let live = pressure.with_time(tomori_protocol::chart::TimeAxis::Live { step_ms: 10_000 });
        let shown = vec![
            tomori_protocol::Panel::new(
                "panel-1",
                "Scaling pressure",
                frozen.summary(),
                floating(-0.45),
            )
            .with_chart(frozen)
            .with_context(vec![
                region("EU-WEST1", ContextSetting::Fixed),
                ContextChip::new("time", "Time", WINDOW, ContextSetting::Fixed)
                    .with_press(ChipPress::Unfreeze),
            ]),
            tomori_protocol::Panel::new(
                "panel-2",
                "Scaling pressure",
                live.summary(),
                floating(0.45),
            )
            .with_chart(live)
            .with_context(vec![
                region("AP-SOUTH1", ContextSetting::Fixed),
                ContextChip::new(
                    "time",
                    "Time",
                    ContextValue::Live { minutes: 30 },
                    ContextSetting::Follows,
                )
                .with_press(ChipPress::Freeze),
            ]),
        ];
        let mut conversation = joined();
        show_projection(&mut conversation, Vec::new(), shown);
        let head = Mat4::from_translation(arrival::HOME);
        let senses = Senses {
            focused: true,
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(arrival::HOME),
            space_head: head,
            ..Default::default()
        };
        let mut step = Step::default();
        let first = step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            0.01,
        );
        draw(&mut renderer, &device, &queue, &mut panels, &first);
        let lens = glam::camera::rh::proj::directx::perspective(
            42_f32.to_radians(),
            EYE.0 as f32 / EYE.1 as f32,
            0.05,
            50.0,
        );
        let matrix = lens * glam::camera::rh::view::look_at_mat4(arrival::HOME, AHEAD, Vec3::Y);
        renderer.render(
            &device,
            &queue,
            std::slice::from_ref(&view),
            &[matrix],
            Mat4::IDENTITY,
            &panels.list(Mat4::IDENTITY),
        );
        let image = read_back(&device, &queue, &texture, EYE);
        crate::test_support::preview("TOMORI_FREEZE_PREVIEW", &image);
        // A frame later, nothing is meshed again.
        let again = step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            0.01,
        );
        let uploads = draw(&mut renderer, &device, &queue, &mut panels, &again);
        assert_eq!(uploads.built, 0, "{uploads:?}");
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        // The chips' fixed backing, a dark teal, is drawn on both.
        let teal = image
            .pixels()
            .filter(|pixel| {
                let [red, green, blue, _] = pixel.0.map(u32::from);
                green > red + 25 && blue > red + 30 && green > 40 && red < 60
            })
            .count();
        assert!(teal > 400, "{teal} pixels of a fixed chip's backing");
    });
}
