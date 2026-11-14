// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The others in a space are drawn among Tomori's cubes, in their one draw, each person culled whole and then cube by
//! cube: their swarms, the eyes of one looking this way warm, and a ray drawn in full ending on the wall it meets. See
//! `design/people.md`.
use super::*;
use tomori_client::connection::{Report, Session};
use tomori_protocol::{
    Activity, AgentState, AgentStatus, Panel, Participant, Place, Pose as Placed, Ray,
    ServerMessage,
};

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

/// Someone in the space: their number, their head, their controllers, left then right, and their right ray, if any.
#[derive(Clone, Copy)]
struct Someone {
    source: u32,
    head: Placed,
    controllers: [Option<Placed>; 2],
    ray: Option<Ray>,
}

impl Someone {
    /// Their presence.
    fn presence(&self) -> tomori_client::Event {
        tomori_client::Event::Message(ServerMessage::Presence {
            source: self.source,
            head: self.head,
            left: self.controllers[0],
            right: self.controllers[1],
            hands: None,
            pointing: self.ray.map(|ray| Box::new([None, Some(ray)])),
        })
    }

    /// Their controller at `at` from their head, in its own coordinates, turned as it is.
    fn holding(head: Placed, at: Vec3) -> Option<Placed> {
        let turn = glam::Quat::from_array(head.orientation);
        Some(Placed {
            position: (Vec3::from_array(head.position) + turn * at).to_array(),
            orientation: head.orientation,
        })
    }
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
/// with the agent's swarm at work behind them, Bob on the left looking this way, his eyes warm, gesturing as he speaks,
/// his orbit quick, and Cleo on the right pointing at a panel on the wall, her ray ending in a dot on it; then the two
/// from beside them; then Bob's head up close, looking this way as he speaks.
#[test]
fn two_people_at_a_wall_are_drawn_among_tomori_s_cubes_in_their_one_draw_culled_whole() {
    let eye = Vec3::new(0.0, 1.6, 0.4);
    let bob_head = [-0.6, 1.68, -1.35];
    let cleo_head = [0.62, 1.55, -1.5];
    // The panel Cleo points at, in the Tomori wall's fifth slot, right of its middle.
    let slot = tomori_wall().slot_pose(4).expect("a fifth slot");
    let facing = glam::Quat::from_array(slot.orientation) * Vec3::Z;
    let on_wall = (Vec3::from_array(slot.position) + facing * 0.03).to_array();
    let aim = Vec3::new(0.86, 1.32, -1.8);
    let along = Vec3::from_array(on_wall) - aim;
    let pointing = glam::Quat::from_rotation_arc(Vec3::NEG_Z, along.normalize());
    let ray = Ray {
        aim: Placed {
            position: aim.to_array(),
            orientation: pointing.to_array(),
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
        // Ada, at `seen_from`, with Bob, Cleo, and Dan, who stands behind her, out of view, in the space, the agent at
        // work; each of the others as `others` says, Bob's voice playing; seen from `looked_from`, looking at `at`.
        let culling = std::cell::Cell::new(true);
        let mut capture = |others: &[Someone], seen_from: Vec3, looked_from: Vec3, at: Vec3| {
            renderer.set_culling(culling.get());
            let mut conversation = joined();
            show_projection(&mut conversation, Vec::new(), panels.clone());
            conversation.apply(space(|state| {
                state.projection = true;
                state.activity = Activity::Running;
                state.agent = Some(AgentStatus {
                    state: AgentState::Working,
                    detail: None,
                    sequence: 1,
                });
                for (name, source) in [("Bob", 2), ("Cleo", 3), ("Dan", 4)] {
                    state.participants.push(Participant {
                        name: name.into(),
                        source,
                        engagement: None,
                    });
                }
            }));
            for someone in others {
                conversation.apply(someone.presence());
            }
            conversation.report(Report::Voices([2].into_iter().collect()));
            let head = Mat4::from_translation(seen_from);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(seen_from),
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
            // What of each person the renderer can draw: every cube but those of no size, as the halo of eyes not
            // warm.
            let drawable: Vec<usize> = frame
                .people
                .iter()
                .map(|figure| {
                    figure
                        .cubes
                        .iter()
                        .filter(|cube| cube.scale.iter().all(|side| *side > 0.0))
                        .count()
                })
                .collect();
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
                drawable,
            )
        };
        let ahead = Vec3::new(0.0, 1.35, -2.3);
        let bob_looking = looking(bob_head, eye.to_array());
        let bob = Someone {
            source: 2,
            head: bob_looking,
            controllers: [
                Someone::holding(bob_looking, Vec3::new(-0.2, -0.52, -0.26)),
                Someone::holding(bob_looking, Vec3::new(0.24, -0.42, -0.3)),
            ],
            ray: None,
        };
        let cleo_looking = looking(cleo_head, on_wall);
        let cleo = Someone {
            source: 3,
            head: cleo_looking,
            controllers: [
                None,
                Some(Placed {
                    position: (aim - along.normalize() * 0.06).to_array(),
                    orientation: pointing.to_array(),
                }),
            ],
            ray: Some(ray),
        };
        let dan = Someone {
            source: 4,
            head: looking([0.0, 1.6, 2.9], [0.0, 1.6, 4.0]),
            controllers: [None, None],
            ray: None,
        };
        let (nobody, alone, ..) = capture(&[], eye, eye, ahead);
        let (people, tally, figures, drawable) = capture(&[bob, cleo, dan], eye, eye, ahead);
        let (beside, ..) = capture(
            &[bob, cleo],
            eye,
            Vec3::new(-2.4, 1.55, -0.3),
            Vec3::new(0.0, 1.3, -1.5),
        );
        // Bob up close, looking at Ada a little under a metre away as he speaks.
        let close = Vec3::from_array(bob_head) + Vec3::new(0.25, -0.05, 0.75);
        let (head, ..) = capture(
            &[Someone {
                head: looking(bob_head, close.to_array()),
                ..bob
            }],
            close,
            close,
            Vec3::from_array(bob_head) + Vec3::new(0.0, -0.12, 0.0),
        );
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
        image::imageops::replace(&mut sheet, &people, 0, 0);
        image::imageops::replace(&mut sheet, &beside, 0, HEIGHT.into());
        image::imageops::replace(&mut sheet, &head, 0, (HEIGHT * 2).into());
        crate::test_support::preview("TOMORI_PEOPLE_PREVIEW", &sheet);
        // Bob looks at Ada, so his eyes are warm, as they are not while he looks at the wall, as Cleo does.
        let (away, ..) = capture(
            &[
                Someone {
                    head: looking(bob_head, on_wall),
                    ..bob
                },
                cleo,
                dan,
            ],
            eye,
            eye,
            ahead,
        );
        // The eyes' glow is warm and bright, where the rest of a person glows in their colour, dimmer at their outline.
        let glow = |image: &image::RgbaImage| {
            image
                .pixels()
                .filter(|pixel| {
                    let [r, g, b, _] = pixel.0.map(i32::from);
                    g > 190 && r - b > 40
                })
                .count()
        };
        assert!(
            glow(&people) > glow(&away) + 150,
            "{} {}",
            glow(&people),
            glow(&away)
        );
        let body = |image: &image::RgbaImage| {
            near(
                image,
                tomori_scene::people::swarm::COLOUR.map(|value| value * 0.75),
                30,
            )
        };
        assert!(
            body(&people) > body(&nobody) + 2_000,
            "{} {}",
            body(&people),
            body(&nobody)
        );
        assert!(body(&beside) > 2_000);
        // In the one draw Tomori's cubes already make, each person an item of the world of their own: Dan, behind Ada,
        // culled whole, with no more than a few of the others' falling cubes, below the view, and nothing drawn for
        // whoever is not here.
        assert_eq!(tally.draws, alone.draws);
        let others = tally.culled - alone.culled - drawable[2];
        assert!(others < 20, "{tally:?} {alone:?} {drawable:?}");
        assert!(
            tally.drawn >= alone.drawn + drawable[0] + drawable[1] * 3 / 4,
            "{tally:?} {alone:?} {drawable:?}"
        );
        assert_eq!(figures.len(), 3);
        assert!(figures.iter().all(|cubes| !cubes.is_empty()));
        // With one person left, the rest of the items are kept, holding nothing.
        let (_, after, figures, _) = capture(&[cleo], eye, eye, ahead);
        assert_eq!(figures[1..], [0..0, 0..0]);
        assert_eq!(after.draws, alone.draws);
        // Culling changes nothing of what is seen of them, drawn after the rest of the cubes, culled or not.
        culling.set(false);
        let (whole, ..) = capture(&[bob, cleo, dan], eye, eye, ahead);
        let (again, unculled, ..) = capture(&[bob, cleo, dan], eye, eye, ahead);
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
        // Each glows, its colour its own light rather than shaded by the light from above the rest are drawn with.
        assert!(
            renderer
                .people
                .iter()
                .all(|instance| instance.glow == crate::graphics::PEOPLE_GLOW)
        );
    });
}
