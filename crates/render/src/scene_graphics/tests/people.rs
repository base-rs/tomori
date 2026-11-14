// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The others in a space are drawn among Tomori's cubes, in their one draw, each person culled whole and then cube by
//! cube: their heads, the visor of one looking this way lit, and a ray drawn in full ending on the wall it meets. See
//! `design/people.md`.
use super::*;
use tomori_protocol::{Panel, Participant, Place, Pose as Placed, Ray, ServerMessage};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 720;

/// A head at `at` whose −Z, where it looks, faces `target`.
fn looking(at: [f32; 3], target: [f32; 3]) -> Placed {
    let at = Vec3::from_array(at);
    Placed::upright_facing(
        at.to_array(),
        (at * 2.0 - Vec3::from_array(target)).to_array(),
    )
}

/// Someone's presence, `source`'s, their head as `head`, and their right ray, if any.
fn presence(source: u32, head: Placed, ray: Option<Ray>) -> tomori_client::Event {
    tomori_client::Event::Message(ServerMessage::Presence {
        source,
        head,
        left: None,
        right: None,
        hands: None,
        pointing: ray.map(|ray| Box::new([None, Some(ray)])),
    })
}

/// How many of `image`'s pixels are within `within` of `color`, in linear light, written as sRGB.
fn near(image: &image::RgbaImage, color: [f32; 3], within: i32) -> usize {
    let [red, green, blue] = color.map(|channel| (channel.powf(1.0 / 2.2) * 255.0) as i32);
    image
        .pixels()
        .filter(|pixel| {
            let [r, g, b, _] = pixel.0.map(i32::from);
            (r - red).abs() < within && (g - green).abs() < within && (b - blue).abs() < within
        })
        .count()
}

/// Set `TOMORI_PEOPLE_PREVIEW` to a PNG path to save what this renders: two people at the Tomori wall, seen from home,
/// Bob on the left looking this way, his visor lit, and Cleo on the right, seen from behind, pointing at a panel on the
/// wall, her ray ending in a dot on it; then the two from beside them.
#[test]
fn two_people_at_a_wall_are_drawn_among_tomori_s_cubes_in_their_one_draw_culled_whole() {
    let eye = Vec3::new(0.0, 1.6, 0.0);
    let bob_head = [-0.55, 1.68, -1.35];
    let cleo_head = [0.6, 1.55, -1.45];
    // The panel Cleo points at, in the Tomori wall's fifth slot, right of its middle.
    let slot = tomori_wall().slot_pose(4).expect("a fifth slot");
    let facing = glam::Quat::from_array(slot.orientation) * Vec3::Z;
    let on_wall = (Vec3::from_array(slot.position) + facing * 0.03).to_array();
    let aim = Vec3::new(0.78, 1.25, -1.3);
    let along = Vec3::from_array(on_wall) - aim;
    let ray = Ray {
        aim: Placed {
            position: aim.to_array(),
            orientation: glam::Quat::from_rotation_arc(Vec3::NEG_Z, along.normalize()).to_array(),
        },
        length: along.length(),
        meets: true,
    };
    let panels = vec![Panel::new(
        "panel-1",
        "Build status",
        "**main** is green.",
        Place::Wall {
            wall: tomori_protocol::TOMORI_WALL.into(),
            slot: 4,
        },
    )];
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("people test eye"),
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
        let mut renderer = Renderer::with_samples(&device, &queue, format, WIDTH, HEIGHT, 4);
        let mut drawn_panels = Panels::new(&device, &renderer);
        let lens = glam::camera::rh::proj::directx::perspective(
            1.0,
            WIDTH as f32 / HEIGHT as f32,
            0.05,
            50.0,
        );
        let now = Instant::now();
        // Ada, at home, with Bob, Cleo, and Dan, who stands behind her, out of view, in the space; each of the others'
        // presence as `others` says.
        let culling = std::cell::Cell::new(true);
        let mut capture = |others: &[(u32, Placed, Option<Ray>)], looked_from: Vec3, at: Vec3| {
            renderer.set_culling(culling.get());
            let mut conversation = joined();
            show_projection(&mut conversation, Vec::new(), panels.clone());
            conversation.apply(space(|state| {
                state.projection = true;
                for (name, source) in [("Bob", 2), ("Cleo", 3), ("Dan", 4)] {
                    state.participants.push(Participant {
                        name: name.into(),
                        source,
                        engagement: None,
                    });
                }
            }));
            for (source, head, ray) in others {
                conversation.apply(presence(*source, *head, *ray));
            }
            let head = Mat4::from_translation(eye);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(eye),
                space_head: head,
                trigger: Some(0.0),
                ..Default::default()
            };
            let mut step = Step::default();
            let mut frame = step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            for _ in 0..400 {
                frame = step.update(&senses, &mut conversation, &mut felt(), now, 1.0 / 72.0);
            }
            assert_eq!(frame.people.len(), others.len());
            draw(&mut renderer, &device, &queue, &mut drawn_panels, &frame);
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[lens * glam::camera::rh::view::look_at_mat4(looked_from, at, Vec3::Y)],
                Mat4::IDENTITY,
                &drawn_panels.list(Mat4::IDENTITY),
            );
            (
                read_back(&device, &queue, &texture, (WIDTH, HEIGHT)),
                renderer.tally(),
                renderer.figures(),
            )
        };
        let ahead = Vec3::new(0.0, 1.4, -2.3);
        let bob = (2, looking(bob_head, eye.to_array()), None);
        let cleo = (3, looking(cleo_head, on_wall), Some(ray));
        let dan = (4, looking([0.0, 1.6, 2.5], [0.0, 1.6, 4.0]), None);
        let (nobody, alone, _) = capture(&[], eye, ahead);
        let (people, tally, figures) = capture(&[bob, cleo, dan], eye, ahead);
        let (beside, ..) = capture(
            &[bob, cleo],
            Vec3::new(-2.2, 1.7, -0.9),
            Vec3::new(0.0, 1.5, -1.5),
        );
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 2);
        image::imageops::replace(&mut sheet, &people, 0, 0);
        image::imageops::replace(&mut sheet, &beside, 0, HEIGHT.into());
        crate::test_support::preview("TOMORI_PEOPLE_PREVIEW", &sheet);
        // Bob looks at Ada, so his visor is lit, as it is not while he looks at the wall, as Cleo does.
        let (away, ..) = capture(
            &[(2, looking(bob_head, on_wall), None), cleo, dan],
            eye,
            ahead,
        );
        // The visor's glow is warm, where everything else here is grey or blue.
        let glow = |image: &image::RgbaImage| {
            image
                .pixels()
                .filter(|pixel| {
                    let [r, _, b, _] = pixel.0.map(i32::from);
                    r > 150 && r - b > 40
                })
                .count()
        };
        assert!(
            glow(&people) > glow(&away) + 400,
            "{} {}",
            glow(&people),
            glow(&away)
        );
        assert!(
            glow(&away) <= glow(&nobody) + 40,
            "{} {}",
            glow(&away),
            glow(&nobody)
        );
        let shell = |image: &image::RgbaImage| near(image, [0.5, 0.5, 0.55], 40);
        assert!(
            shell(&people) > shell(&nobody) + 400,
            "{} {}",
            shell(&people),
            shell(&nobody)
        );
        assert!(shell(&beside) > 400);
        // In the one draw Tomori's cubes already make, each person an item of the world of their own: Dan, behind Ada,
        // culled whole, and nothing drawn for whoever is not here.
        assert_eq!(tally.draws, alone.draws);
        let head = tomori_scene::people::CUBES - 4;
        assert_eq!(tally.culled, alone.culled + head, "{tally:?} {alone:?}");
        assert!(
            tally.drawn >= alone.drawn + head * 2 + 2,
            "{tally:?} {alone:?}"
        );
        assert_eq!(figures.len(), 3);
        assert!(figures.iter().all(|cubes| !cubes.is_empty()));
        // With one person left, the rest of the items are kept, holding nothing.
        let (_, after, figures) = capture(&[cleo], eye, ahead);
        assert_eq!(figures[1..], [0..0, 0..0]);
        assert_eq!(after.draws, alone.draws);
        // Culling changes nothing of what is seen of them, drawn after the rest of the cubes, culled or not.
        culling.set(false);
        let (whole, ..) = capture(&[bob, cleo, dan], eye, ahead);
        let (again, unculled, _) = capture(&[bob, cleo, dan], eye, ahead);
        assert_eq!(unculled.culled, 0);
        assert!(
            whole == people && again == people,
            "culling changed what is seen"
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn a_person_s_cubes_that_are_not_numbers_are_not_drawn_and_at_most_so_many_people_are() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let mut renderer =
            Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 16, 16);
        let cube = |position: [f32; 3], scale: f32| tomori_scene::delegation::Cube {
            position,
            scale: [scale; 3],
            color: [0.5; 3],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let figure = |source: u32| tomori_scene::people::Figure {
            source,
            cubes: vec![
                cube([0.0, 1.6, -2.0], 0.1),
                cube([f32::NAN, 1.6, -2.0], 0.1),
                cube([0.0, 1.6, -2.0], 0.0),
            ],
        };
        let many: Vec<_> = (0..crate::graphics::MOST_PEOPLE as u32 + 2)
            .map(figure)
            .collect();
        renderer.set_people(&many);
        let figures = renderer.figures();
        assert_eq!(figures.len(), crate::graphics::MOST_PEOPLE);
        assert_eq!(figures[3], 3..4);
        assert_eq!(figures.last().unwrap().end, crate::graphics::MOST_PEOPLE);
    });
}
