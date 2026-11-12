// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The chips of a wall's and its panels' context as a participant sees them: a wall fixed to EU-WEST1 and a window of
//! 02:20 to 02:50 UTC shows them after its label, a chart on it inherits them, and a list fixed to another region shows
//! its own, each chip on its own backing, drawn with the faces, which are meshed once. See `design/panel-context.md`.
use super::*;
use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};

/// The eye, wide enough for a wall of two slots.
const EYE: (u32, u32) = (1408, 720);

/// Where the wall stands, facing home.
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

/// Set `TOMORI_CHIPS_PREVIEW` to a PNG path to save what this renders: the wall and its two panels with their chips.
#[test]
fn a_wall_s_and_its_panels_context_is_drawn_as_chips_on_their_faces() {
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
        let wall = tomori_protocol::Wall::new(
            "wall-1",
            tomori_protocol::WallKind::Added,
            tomori_protocol::Pose::upright_facing(AHEAD.to_array(), arrival::HOME.to_array()),
        )
        .with_grid(tomori_protocol::Grid::from((2, 1)))
        .with_context(vec![
            region("EU-WEST1", ContextSetting::Fixed),
            ContextChip::new("time", "Time", WINDOW, ContextSetting::Fixed),
        ]);
        let on = |slot: u32| tomori_protocol::Place::Wall {
            wall: "wall-1".into(),
            slot,
        };
        let chart = tomori_scene::chart::fixtures::pressure();
        let shown = vec![
            tomori_protocol::Panel::new("panel-1", "Scaling pressure", chart.summary(), on(0))
                .with_chart(chart)
                .with_context(vec![
                    region("EU-WEST1", ContextSetting::Inherited),
                    ContextChip::new("time", "Time", WINDOW, ContextSetting::Inherited),
                ]),
            tomori_protocol::Panel::new(
                "panel-2",
                "Pods",
                "- pod-7 · 3 waiting\n- pod-9 · 41 waiting\n- pod-112 · 12 waiting",
                on(1),
            )
            .with_context(vec![region("AP-SOUTH1", ContextSetting::Fixed)]),
        ];
        let mut conversation = joined();
        show_projection(&mut conversation, vec![wall], shown);
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
        crate::test_support::preview("TOMORI_CHIPS_PREVIEW", &image);
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
        // The chips' fixed backing, a dark teal, is drawn: on the wall's strip and the list's chip.
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
