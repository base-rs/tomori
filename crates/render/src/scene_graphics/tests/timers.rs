// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The space's timers draw as dials above the agent's home, beside its workers, each with its sign. See
//! `design/timers.md`.
use super::*;
use tomori_protocol::{Timer, TimerState, Worker, WorkerState};

/// When the timers count down from, in seconds since the Unix epoch.
const NOW: f64 = 1_800_000_000.0;

fn timers() -> Vec<Timer> {
    let at = |seconds: f64| Some((NOW + seconds) as u64);
    vec![
        Timer::new(
            "status",
            "every hour",
            "when present",
            at(12.0 * 60.0 + 5.0),
            TimerState::Waiting,
        ),
        Timer::new(
            "standup",
            "weekdays at 09:00",
            "when present or on arrival",
            at(3.0 * 3600.0 + 300.0),
            TimerState::Paused,
        ),
        Timer::new(
            "morning",
            "daily at 09:00",
            "when present or on arrival",
            None,
            TimerState::Awaiting,
        ),
        Timer::new(
            "check the build",
            "every 15 minutes",
            "always",
            at(15.0 * 60.0),
            TimerState::Fired,
        ),
    ]
}

/// Where the agent's home stands in a ring like the farmyard's, from home.
const RING_HOME: Vec3 = Vec3::new(-1.5, 1.5, -3.1);

/// Set `TOMORI_TIMERS_PREVIEW` to a PNG path to save what this renders: four timers, waiting, paused, waiting for an
/// arrival and just fired, above the agent's home, beside six workers on today's arc, from a step behind home, then
/// from home at about the headset's pixels to the degree, with the first pointed at.
#[test]
fn timers_draw_as_dials_above_the_agent_beside_its_workers() {
    render(false, "TOMORI_TIMERS_PREVIEW");
}

/// Set `TOMORI_TIMERS_RING_PREVIEW` to a PNG path to save what this renders: the same timers in a home whose workers
/// stand in a ring about the agent, as the farmyard's do, which keeps nothing above 2.5 m: beneath the agent.
#[test]
fn in_a_ring_timers_draw_as_dials_beneath_the_agent() {
    render(true, "TOMORI_TIMERS_RING_PREVIEW");
}

fn render(ring: bool, variable: &str) {
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 640;
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("timers test eye"),
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
        let mut panels = Panels::new(&device, &renderer);
        let mut conversation = joined();
        conversation.apply(space(|state| {
            state.workers = (0..6)
                .map(|index| Worker {
                    id: format!("ses_{index}"),
                    parent: None,
                    title: ["Review the pull request", "Run the tests", "Write the docs"]
                        [index % 3]
                        .into(),
                    state: [
                        WorkerState::Running,
                        WorkerState::Waiting,
                        WorkerState::Completed,
                    ][index % 3],
                })
                .collect();
            state.timers = timers();
            if ring {
                let pose = tomori_protocol::Pose::at(RING_HOME.to_array())
                    .facing_towards(arrival::HOME.to_array());
                state.anchors = tomori_protocol::Anchors::new()
                    .with_agent(pose)
                    .with_workers(pose)
                    .with_cluster(
                        tomori_protocol::cluster::Anchor::new(tomori_protocol::cluster::HOME, pose)
                            .with_formation(tomori_protocol::cluster::Formation::Ring)
                            .with_sweep(300.0),
                    );
                state.projection = true;
            }
        }));
        if ring {
            // The space's own status wall, where the server stands it, beneath the agent's home, with its panels.
            let middle = RING_HOME - Vec3::Y * 0.88;
            let wall = tomori_protocol::Wall::new(
                tomori_protocol::TOMORI_WALL,
                tomori_protocol::WallKind::Tomori,
                tomori_protocol::Pose::upright_facing(middle.to_array(), [0.0, middle.y, 0.0]),
            )
            .with_shape(tomori_protocol::WallShape::Arc { radius: 2.3 });
            let place = |slot: u32| tomori_protocol::Place::Wall {
                wall: tomori_protocol::TOMORI_WALL.into(),
                slot,
            };
            let panels = (0..6)
                .map(|slot| {
                    tomori_protocol::Panel::new(
                        format!("panel-{slot}"),
                        [
                            "Build status",
                            "Workers",
                            "Places",
                            "Release",
                            "Summary",
                            "Notes",
                        ][slot as usize],
                        "Tomori removed 2 finished workers.",
                        place(slot),
                    )
                })
                .collect();
            conversation.apply(tomori_client::Event::Message(
                tomori_protocol::ServerMessage::Projection {
                    projection: Box::new(
                        tomori_protocol::Projection::new(SPACE, "sky")
                            .with_walls(vec![wall])
                            .with_panels(panels),
                    ),
                },
            ));
        }
        conversation.take_actions();
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
        let start = Instant::now();
        step.set_clock(|| NOW);
        let frame_at =
            |step: &mut Step,
             senses: &Senses,
             frame: u32,
             renderer: &mut Renderer,
             panels: &mut Panels,
             conversation: &mut tomori_scene::conversation::Conversation| {
                let now = start + std::time::Duration::from_millis(u64::from(frame) * 50);
                let frame = step.update(senses, conversation, &mut felt(), now, 0.05);
                assert_eq!(frame.dials.len(), 4);
                draw(renderer, &device, &queue, panels, &frame);
                Vec3::from_array(frame.dials[0].position)
            };
        let mut first = Vec3::ZERO;
        for frame in 0..60 {
            first = frame_at(
                &mut step,
                &senses,
                frame,
                &mut renderer,
                &mut panels,
                &mut conversation,
            );
        }
        let agent = if ring {
            RING_HOME
        } else {
            tomori_scene::anchors::Anchor::AGENT.position
        };
        // From a step behind home, to see the dials at the agent and the workers about it.
        let overview = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            WIDTH as f32 / HEIGHT as f32,
            0.05,
            50.0,
        ) * glam::camera::rh::view::look_at_mat4(
            Vec3::new(0.0, 1.9, 1.6),
            agent + Vec3::Y * 0.1,
            Vec3::Y,
        );
        // From home, at about the headset's pixels to the degree: twenty, over a field 32° high, at the dials.
        let home = glam::camera::rh::proj::directx::perspective(
            32f32.to_radians(),
            WIDTH as f32 / HEIGHT as f32,
            0.05,
            50.0,
        ) * glam::camera::rh::view::look_at_mat4(
            arrival::HOME,
            Vec3::new(agent.x, first.y + 0.1, agent.z),
            Vec3::Y,
        );
        let capture = |renderer: &Renderer, panels: &Panels, matrix: Mat4| {
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            read_back(&device, &queue, &texture, (WIDTH, HEIGHT))
        };
        let wide = capture(&renderer, &panels, overview);
        let shown = capture(&renderer, &panels, home);
        // Pointing at the first dial highlights it.
        let pointing = Senses {
            ray: tomori_scene::pointing::Ray::new(arrival::HOME, first - arrival::HOME),
            ..senses
        };
        frame_at(
            &mut step,
            &pointing,
            60,
            &mut renderer,
            &mut panels,
            &mut conversation,
        );
        let pointed = capture(&renderer, &panels, home);
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 2);
        image::imageops::replace(&mut sheet, &wide, 0, 0);
        image::imageops::replace(&mut sheet, &pointed, 0, HEIGHT.into());
        crate::test_support::preview(variable, &sheet);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        assert!(pointed != shown, "the dial pointed at is drawn highlighted");
        // Above the agent on today's arc; beneath it in the ring.
        assert_eq!(first.y > agent.y, !ring);
    });
}
