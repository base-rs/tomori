// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A big locked wall's padlock is drawn for an owner at its place nearest them, within reach, rather than beyond its
//! top left corner. See `design/projection.md`.
use super::*;
use tomori_protocol::{Grid, Wall, WallKind};
use tomori_scene::projection::editing::{LOCKED, PADLOCK_NEAR, padlock_point, padlock_points};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 720;

/// Set `TOMORI_PADLOCK_CORNER_PREVIEW` to a PNG path to save what this renders: a wall someone added, locked, six slots
/// wide and two high at twice a standard slot's size, seen by an owner standing 1.5 m out from its right end, then from
/// its left end, its padlock drawn at the place nearest them, below its bottom corner there; and from far back, beyond
/// reach of every place, no padlock drawn.
#[test]
fn a_big_locked_wall_s_padlock_is_drawn_at_its_place_nearest_the_owner() {
    let middle = Vec3::new(0.0, 2.5, -2.0);
    let wall = Wall::new(
        "wall-big",
        WallKind::Added,
        tomori_protocol::Pose::upright_facing(middle.to_array(), arrival::HOME.to_array()),
    )
    .with_grid(Grid::new(6, 2).unwrap())
    .with_scale(2.0)
    .with_locked(true);
    let frame =
        Mat4::from_rotation_translation(glam::Quat::from_array(wall.pose.orientation), middle);
    let (width, height) = wall.size();
    let (points, _) = padlock_points(&wall, frame, &[&wall]);
    // 1.5 m out from either end of its bottom edge, the eyes 1.6 m up.
    let by = |side: f32| {
        let corner = frame.transform_point3(Vec3::new(side * width / 2.0, -height / 2.0, 0.0));
        Vec3::new(corner.x, 1.6, corner.z + 1.5)
    };
    let (right, left) = (by(1.0), by(-1.0));
    let far = Vec3::new(0.0, 1.6, 4.0);
    assert!(points[0].distance(right) > PADLOCK_NEAR);
    assert!(
        points
            .iter()
            .all(|point| point.distance(far) > PADLOCK_NEAR)
    );
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("padlock corner test eye"),
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
        let mut renderer = Renderer::new(&device, &queue, format, WIDTH, HEIGHT);
        renderer.set_environment(tomori_scene::environment::Environment::MINIMAL);
        let mut drawn_panels = Panels::new(&device, &renderer);
        let lens = glam::camera::rh::proj::directx::perspective(
            1.2,
            WIDTH as f32 / HEIGHT as f32,
            0.05,
            50.0,
        );
        let now = Instant::now();
        let look = |eye: Vec3, target: Vec3| {
            lens * glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y)
        };
        // Seen from `eye`, looking at `target`.
        let mut capture = |eye: Vec3, target: Vec3| {
            let mut conversation = joined();
            show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
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
            let mut step = Step::default();
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            draw(&mut renderer, &device, &queue, &mut drawn_panels, &frame);
            let view_matrix = look(eye, target);
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
        // The closed padlock's warm pixels: how many, and their middle.
        let warm = |image: &image::RgbaImage| {
            let [red, green, blue, _] =
                LOCKED.map(|channel| (channel.powf(1.0 / 2.2) * 255.0) as i32);
            let (mut count, mut sum) = (0u32, glam::Vec2::ZERO);
            for (x, y, pixel) in image.enumerate_pixels() {
                let [r, g, b, _] = pixel.0.map(i32::from);
                if (r - red).abs() < 40 && (g - green).abs() < 40 && (b - blue).abs() < 50 {
                    count += 1;
                    sum += glam::Vec2::new(x as f32, y as f32);
                }
            }
            (count, sum / count.max(1) as f32)
        };
        // Each looking a little above the place nearest them, towards the wall, so that both are seen.
        let nearest = |eye: Vec3| padlock_point(&wall, frame, &[&wall], eye).unwrap();
        let towards = |eye: Vec3| nearest(eye) + Vec3::Y * 0.5;
        let from_right = capture(right, towards(right));
        let from_left = capture(left, towards(left));
        let from_far = capture(far, middle);
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
        image::imageops::replace(&mut sheet, &from_right, 0, 0);
        image::imageops::replace(&mut sheet, &from_left, 0, HEIGHT.into());
        image::imageops::replace(&mut sheet, &from_far, 0, (HEIGHT * 2).into());
        crate::test_support::preview("TOMORI_PADLOCK_CORNER_PREVIEW", &sheet);
        // Drawn where the place nearest them is seen.
        for (name, image, eye) in [("right", &from_right, right), ("left", &from_left, left)] {
            let at = nearest(eye);
            let clip = look(eye, towards(eye)) * at.extend(1.0);
            let seen = glam::Vec2::new(
                (clip.x / clip.w + 1.0) / 2.0 * WIDTH as f32,
                (1.0 - clip.y / clip.w) / 2.0 * HEIGHT as f32,
            );
            assert!(at.distance(eye) <= PADLOCK_NEAR, "{name}");
            assert!(
                frame.inverse().transform_point3(at).y < -height / 2.0,
                "{name}"
            );
            let (count, middle) = warm(image);
            assert!(count > 20, "{name}: {count}");
            assert!(middle.distance(seen) < 40.0, "{name}: {middle} {seen}");
        }
        assert_eq!(warm(&from_far).0, 0);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
