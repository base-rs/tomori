// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A locked wall's padlock beneath a deck is not drawn for an owner standing on it, even through a floor of glass,
//! which writes no depth. See `design/projection.md`.
use super::*;
use crate::scenery::{Sceneries, Scenery, Target};
use tomori_scene::locomotion::{Locomotion, Surface};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 720;

/// Glass, 6 m square, horizontal, centred at `centre`: translucent, so that it writes no depth and hides nothing
/// beneath it by depth alone.
struct Glass {
    glyphs: crate::glyphs::SharedGlyphs,
    pipeline: crate::ui_panel::Pipeline,
    panel: crate::ui_panel::Panel,
    model: Mat4,
}

impl Glass {
    fn new(target: &Target, centre: Vec3) -> Self {
        let pipeline = crate::ui_panel::Pipeline::shared(
            target.device,
            target.format,
            &target.glyphs,
            target.samples,
        );
        let mut panel = crate::ui_panel::Panel::new(target.device, &pipeline, 64);
        let mut canvas = crate::ui_panel::Canvas::new(1, 0.01);
        canvas.background = [0.45, 0.65, 0.85, 0.35];
        let (width, height) = canvas.size();
        let model = Mat4::from_translation(centre)
            * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
            * Mat4::from_scale(Vec3::new(6.0 / width, 6.0 / height, 1.0));
        panel.set(target.queue, &canvas);
        Self {
            glyphs: target.glyphs.clone(),
            pipeline,
            panel,
            model,
        }
    }
}

impl Scenery for Glass {
    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4) {
        self.panel.prepare(queue, eye, scene_to_clip * self.model);
    }

    fn draw(&self, _pass: &mut wgpu::RenderPass<'_>, _eye: usize) {}

    fn draw_translucent(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        self.pipeline.set(pass, &self.glyphs.binding());
        self.panel.draw(pass, &self.pipeline, eye);
    }

    fn set_reveal(&mut self, _reveal: Option<(Vec3, f32)>) {}
}

/// Set `TOMORI_PADLOCK_FLOOR_PREVIEW` to a PNG path to save what this renders: an environment's own wall, locked,
/// beneath a deck of glass 30 cm above its padlock, seen by an owner standing on the deck within 3 m of the padlock:
/// first as an environment that declared no deck would have it, the padlock seen through the floor, then with the deck
/// declared, the padlock hidden; and from the main floor in front of it, the padlock in clear view.
#[test]
fn a_locked_wall_s_padlock_beneath_a_deck_is_hidden_from_an_owner_on_it_and_shown_from_the_floor() {
    use tomori_protocol::{Panel, Place, Wall, WallKind};
    use tomori_scene::projection::editing::{
        LOCKED, LockControls, PADLOCK_NEAR, handle_points_with,
    };
    let middle = Vec3::new(0.0, 1.45, -2.5);
    let wall = Wall::new(
        "environment-0",
        WallKind::Environment,
        tomori_protocol::Pose::upright_facing(middle.to_array(), [0.0, 1.45, 0.0]),
    )
    .with_grid(tomori_protocol::Grid::new(2, 1).unwrap())
    .with_locked(true);
    let frame =
        Mat4::from_rotation_translation(glam::Quat::from_array(wall.pose.orientation), middle);
    let lock = LockControls {
        padlock: true,
        reset: false,
    };
    let padlock = handle_points_with(&wall, frame, lock).0[0].1;
    let height = padlock.y + 0.3;
    let centre = Vec3::new(0.0, height, -3.0);
    let floor = Surface::disc(Vec3::ZERO, 20.0);
    let deck = Surface::rectangle(centre, glam::Vec2::splat(6.0));
    // On the deck, over the wall, a metre and a half out from it and to its left.
    let above = Vec3::new(-0.6, height + 1.6, -1.2);
    assert!(above.distance(padlock) < PADLOCK_NEAR);
    let panels = vec![Panel::new(
        "panel-0",
        "Notices",
        "**Feed** the hens at six.",
        Place::Wall {
            wall: "environment-0".into(),
            slot: 0,
        },
    )];
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("padlock floor test eye"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
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
        let mut sceneries = Sceneries::empty();
        sceneries.register("sky", move |target| {
            Box::new(Glass::new(target, centre)) as Box<dyn Scenery>
        });
        let mut renderer =
            Renderer::with_sceneries(&device, &queue, format, WIDTH, HEIGHT, 1, &sceneries);
        let mut drawn_panels = Panels::new(&device, &renderer);
        let lens = glam::camera::rh::proj::directx::perspective(
            1.2,
            WIDTH as f32 / HEIGHT as f32,
            0.05,
            50.0,
        );
        let now = Instant::now();
        let mut capture = |surfaces: Vec<Surface>, eye: Vec3| {
            let mut conversation = joined();
            conversation.set_locomotion(vec![(
                tomori_scene::environment::Environment::new("sky"),
                Locomotion::teleport(surfaces),
            )]);
            show_lockable_walls(&mut conversation, vec![wall.clone()], panels.clone());
            let head = Mat4::from_translation(eye);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(eye),
                space_head: head,
                trigger: Some(0.0),
                grip: Some(0.0),
                ..Default::default()
            };
            // Stepped until the space's scenery has grown in.
            let mut step = Step::default();
            let mut frame = step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            for _ in 0..400 {
                frame = step.update(&senses, &mut conversation, &mut felt(), now, 1.0 / 72.0);
            }
            assert!(frame.scenery.reveal.is_none());
            draw(&mut renderer, &device, &queue, &mut drawn_panels, &frame);
            let view_matrix =
                lens * glam::camera::rh::view::look_at_mat4(eye, padlock - Vec3::Y * 0.3, Vec3::Y);
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[view_matrix],
                Mat4::IDENTITY,
                &drawn_panels.list(Mat4::IDENTITY),
            );
            read_back(&device, &queue, &texture, (WIDTH, HEIGHT))
        };
        let undeclared = capture(vec![floor.clone()], above);
        let hidden = capture(vec![floor.clone(), deck.clone()], above);
        let in_view = capture(vec![floor, deck], arrival::HOME + Vec3::X * 0.3);
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
        image::imageops::replace(&mut sheet, &undeclared, 0, 0);
        image::imageops::replace(&mut sheet, &hidden, 0, HEIGHT.into());
        image::imageops::replace(&mut sheet, &in_view, 0, (HEIGHT * 2).into());
        crate::test_support::preview("TOMORI_PADLOCK_FLOOR_PREVIEW", &sheet);
        // The closed padlock's warm colour: seen through glass where no deck is declared, nowhere once it is, and in
        // clear view from the floor.
        let warm = |image: &image::RgbaImage| {
            let [red, green, blue, _] =
                LOCKED.map(|channel| (channel.powf(1.0 / 2.2) * 255.0) as i32);
            image
                .pixels()
                .filter(|pixel| {
                    let [r, g, b, _] = pixel.0.map(i32::from);
                    (r - red).abs() < 40 && (g - green).abs() < 40 && (b - blue).abs() < 50
                })
                .count()
        };
        let (through, under, clear) = (warm(&undeclared), warm(&hidden), warm(&in_view));
        assert!(through > 4, "{through}");
        assert_eq!(under, 0);
        assert!(clear > 4, "{clear}");
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
