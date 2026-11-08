// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Drawing what the frame step decided: [`draw`] applies a [`Frame`] to the renderer, and [`Panels`] holds the shared panels' GPU buffers, for both the headset and the desktop window.
use glam::Mat4;

use crate::{
    arrival_graphics,
    controller_ui::{CAPACITY, Sheet},
    graphics::Renderer,
    scene::{self, Frame},
    ui_panel::Panel,
};

/// The shared panels' GPU buffers, with where each is drawn: the status pill, the recording indicator and the `LIVE` badge, and the right wrist's transcript.
pub struct Panels {
    pill: Panel,
    card: Panel,
    badge: Panel,
    wrist: Panel,
    /// The pill's model in the eyes' space, while it is drawn.
    pill_model: Option<Mat4>,
    /// The card's, the badge's and the transcript's models in the scene, while they are drawn.
    models: [Option<Mat4>; 3],
    /// The space's projection walls and panels, then the GitHub environment's inbox and cards, one panel each, kept as
    /// their number falls so that they need not be made again, with the models of those drawn now.
    cards: Vec<Panel>,
    card_models: Vec<Mat4>,
}

/// The most vertices one card's face takes, about three times what a full face of text does.
const CARD_CAPACITY: usize = CAPACITY / 2;

impl Panels {
    pub fn new(device: &wgpu::Device, renderer: &Renderer) -> Self {
        Self {
            pill: renderer.panel(device, CAPACITY),
            card: renderer.panel(device, CAPACITY),
            badge: renderer.panel(device, CAPACITY / 8),
            wrist: renderer.panel(device, CAPACITY * 4),
            pill_model: None,
            models: [None; 3],
            cards: Vec::new(),
            card_models: Vec::new(),
        }
    }

    /// Upload the projection's walls and panels and the inbox and cards, `sheets`, in the order to draw them, making
    /// panels for them as needed.
    pub fn set_cards<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        sheets: impl IntoIterator<Item = &'a Sheet>,
    ) {
        self.card_models.clear();
        for sheet in sheets {
            let (Some(canvas), Some(model)) = (&sheet.canvas, sheet.model) else {
                continue;
            };
            let index = self.card_models.len();
            if index == self.cards.len() {
                self.cards.push(renderer.panel(device, CARD_CAPACITY));
            }
            self.cards[index].set(queue, canvas);
            self.card_models.push(model);
        }
    }

    /// Upload what each panel shows now, and remember where it is drawn.
    pub fn set(&mut self, queue: &wgpu::Queue, panels: &scene::Panels) {
        let upload = |panel: &mut Panel, sheet: &Sheet| {
            if let Some(canvas) = &sheet.canvas {
                panel.set(queue, canvas);
            }
            sheet.model
        };
        self.pill_model = upload(&mut self.pill, &panels.pill);
        let [card, badge] = &panels.indicator;
        self.models = [
            upload(&mut self.card, card),
            upload(&mut self.badge, badge),
            upload(&mut self.wrist, &panels.wrist),
        ];
    }

    /// The panels to draw, each with its model in the eyes' space, which `scene_to_space` takes the scene to: the
    /// projection's walls and panels, then the inbox and cards, in the order the frame step gave them, farthest first, so
    /// that a nearer card covers one behind it, as panels do not write depth; then the pill, card, badge and transcript.
    pub fn list(&self, scene_to_space: Mat4) -> Vec<(&Panel, Mat4)> {
        let cards = self
            .cards
            .iter()
            .zip(&self.card_models)
            .map(|(panel, model)| (panel, scene_to_space * *model));
        let scene = [&self.card, &self.badge, &self.wrist]
            .into_iter()
            .zip(self.models)
            .filter_map(|(panel, model)| Some((panel, scene_to_space * model?)));
        cards
            .chain(self.pill_model.map(|model| (&self.pill, model)))
            .chain(scene)
            .collect()
    }
}

/// Apply `frame` to `renderer` and `panels`, ready to render.
pub fn draw(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    panels: &mut Panels,
    frame: &Frame,
) {
    renderer.set_scenery(frame.scenery);
    if let Some(drawn) = &frame.environment {
        renderer.animate(device, queue, drawn.as_ref());
    }
    renderer.set_landing(frame.landing);
    renderer.set_blink(frame.blink);
    renderer.update(queue, frame.swarm, frame.marker, frame.attracting);
    renderer.set_presentation_page(frame.card);
    renderer.set_presentation_content_page(frame.content);
    renderer.update_workspace(device, queue, frame.cubes, frame.stations, frame.workspace);
    if let Some(arrival) = &frame.arrival {
        renderer.update_arrival(
            device,
            queue,
            &arrival_graphics::Frame {
                scene: &arrival.scene,
                hovered: arrival.hovered.as_ref(),
                ray: arrival.ray,
                time: arrival.time,
            },
        );
    }
    panels.set(queue, &frame.panels);
    // The walls stand farther away than the inbox, beside the swarm, so they go first.
    panels.set_cards(
        device,
        queue,
        renderer,
        frame.projection.iter().chain(&frame.cards),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_gpu;
    use crate::{
        arrival,
        controller::Pose,
        hand_menu::{HOLD, Palm},
        scene::{Senses, Step, fixtures::*},
    };
    use glam::Vec3;
    use std::time::Instant;

    #[test]
    fn a_frame_draws_its_panels_where_the_step_placed_them() {
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut renderer =
                Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
            let mut panels = Panels::new(&device, &renderer);
            assert!(panels.list(Mat4::IDENTITY).is_empty(), "nothing drawn yet");
            let head = Mat4::from_translation(arrival::HOME);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(arrival::HOME),
                space_head: head,
                trigger: Some(0.0),
                // A right palm turned up, which shows the transcript.
                right_palm: Some(Palm {
                    wrist: arrival::HOME + Vec3::new(0.1, -0.35, -0.3),
                    normal: Vec3::Y,
                }),
                ..Default::default()
            };
            let mut conversation = joined();
            let mut step = Step::default();
            let start = Instant::now();
            step.update(&senses, &mut conversation, &mut felt(), start, 0.01);
            let mut frame =
                step.update(&senses, &mut conversation, &mut felt(), start + HOLD, 0.01);
            let pill = frame.panels.pill.model;
            let wrist = frame.panels.wrist.model.expect("the transcript shows");
            // What an environment's client side shows is handed to the renderer, which here has no scenery for it.
            frame.environment = Some(std::sync::Arc::new(1usize));
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            // The pill is hidden while a hand's panel shows; the transcript is moved from the scene to the eyes' space.
            assert!(pill.is_none());
            let scene_to_space = Mat4::from_translation(Vec3::X);
            let list = panels.list(scene_to_space);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].1, scene_to_space * wrist);
            // In arrival, the objects are drawn, with the pill hidden and the transcript gone.
            let mut lobby = lobby(tomori_protocol::ConnectionRole::Owner);
            let mut step = Step::default();
            let mut frame = step.update(&senses, &mut lobby, &mut felt(), start, 1.0);
            assert!(frame.arrival.is_some());
            // The grid has no scenery to show it.
            frame.environment = Some(std::sync::Arc::new(1usize));
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            assert!(panels.list(scene_to_space).is_empty());
            // In a space with nothing on the hands, only the pill, already in the eyes' space.
            let mut step = Step::default();
            let plain = Senses {
                right_palm: None,
                ..senses
            };
            let frame = step.update(&plain, &mut conversation, &mut felt(), start, 0.01);
            let pill = frame.panels.pill.model.expect("the pill");
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let list = panels.list(scene_to_space);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].1, pill);
            // Recording, the pill comes first, then the indicator, moved into the eyes' space.
            let recording = Senses {
                trackers: tomori_scene::recording::Trackers {
                    view: Some(arrival::HOME - Vec3::Z * 0.5),
                    ..Default::default()
                },
                ..plain
            };
            conversation.talk(true);
            let frame = step.update(&recording, &mut conversation, &mut felt(), start, 0.01);
            let card = frame.panels.indicator[0].model.expect("the recording card");
            let pill = frame.panels.pill.model.expect("the pill");
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let models: Vec<Mat4> = panels
                .list(scene_to_space)
                .into_iter()
                .map(|(_, model)| model)
                .collect();
            assert_eq!(models, [pill, scene_to_space * card]);
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }

    #[test]
    fn the_inbox_and_cards_are_drawn_first_in_the_order_the_step_gave() {
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut renderer =
                Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
            let mut panels = Panels::new(&device, &renderer);
            let card = |number: u64| tomori_protocol::Card {
                id: format!("github://o/r/issues/{number}"),
                number,
                kind: tomori_protocol::CardKind::Issue,
                title: "A card".into(),
                author: "ada".into(),
                labels: Vec::new(),
                checks: tomori_protocol::Checks::Passing,
                age: 0,
                placement: None,
            };
            let mut conversation = joined();
            let show = |conversation: &mut tomori_scene::conversation::Conversation, count: u64| {
                show_cards(conversation, (1..=count).map(card).collect());
            };
            show(&mut conversation, 3);
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
            let frame = step.update(
                &senses,
                &mut conversation,
                &mut felt(),
                Instant::now(),
                0.01,
            );
            let models: Vec<Mat4> = frame
                .cards
                .iter()
                .map(|sheet| sheet.model.unwrap())
                .collect();
            assert_eq!(models.len(), 4);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let list = panels.list(Mat4::IDENTITY);
            let listed: Vec<Mat4> = list.iter().take(4).map(|(_, model)| *model).collect();
            assert_eq!(listed, models);
            // Fewer cards keep the panels made, and draw only those shown.
            show(&mut conversation, 1);
            let frame = step.update(
                &senses,
                &mut conversation,
                &mut felt(),
                Instant::now(),
                0.01,
            );
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            assert_eq!(panels.cards.len(), 4);
            assert_eq!(panels.card_models.len(), 2);
            // A sheet without a canvas is not drawn.
            panels.set_cards(&device, &queue, &renderer, &[Sheet::default()]);
            assert!(panels.card_models.is_empty());
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }

    /// Set `TOMORI_CARDS_PREVIEW` to a PNG path to save what this renders: the inbox from home, with a card carried.
    #[test]
    fn the_inbox_and_its_cards_render_over_the_scene() {
        const WIDTH: u32 = 960;
        const HEIGHT: u32 = 720;
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cards test eye"),
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
            let titles = [
                (
                    164,
                    tomori_protocol::CardKind::PullRequest,
                    tomori_protocol::Checks::Passing,
                    "Make the client crates ready to publish",
                ),
                (
                    163,
                    tomori_protocol::CardKind::Draft,
                    tomori_protocol::Checks::Failing,
                    "Move the headset app to com.codeotaku.tomori",
                ),
                (
                    149,
                    tomori_protocol::CardKind::Issue,
                    tomori_protocol::Checks::None,
                    "Triage boxes that apply labels on GitHub",
                ),
                (
                    148,
                    tomori_protocol::CardKind::Issue,
                    tomori_protocol::Checks::None,
                    "An inbox of issue and pull request cards",
                ),
                (
                    146,
                    tomori_protocol::CardKind::Issue,
                    tomori_protocol::Checks::None,
                    "Text rendering: a glyph atlas",
                ),
                (
                    97,
                    tomori_protocol::CardKind::Issue,
                    tomori_protocol::Checks::None,
                    "Physics with Rapier",
                ),
            ];
            let cards: Vec<tomori_protocol::Card> = titles
                .iter()
                .map(|(number, kind, checks, title)| tomori_protocol::Card {
                    id: format!("github://ioquatix/tomori/issues/{number}"),
                    number: *number,
                    kind: *kind,
                    title: (*title).into(),
                    author: "ioquatix".into(),
                    labels: vec![tomori_protocol::Label {
                        name: "enhancement".into(),
                        color: "a2eeef".into(),
                    }],
                    checks: *checks,
                    age: 3 * 86_400,
                    placement: None,
                })
                .collect();
            let mut conversation = joined();
            show_cards(&mut conversation, cards);
            let head = Mat4::from_translation(arrival::HOME);
            let inbox = tomori_scene::board::INBOX;
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(arrival::HOME),
                space_head: head,
                trigger: Some(0.0),
                beam: true,
                ray: tomori_scene::pointing::Ray::new(
                    arrival::HOME,
                    inbox + Vec3::new(0.0, 0.05, 0.0) - arrival::HOME,
                ),
                ..Default::default()
            };
            let mut step = Step::default();
            let now = Instant::now();
            let capture = |renderer: &Renderer, panels: &Panels| {
                // About the field of view the headset gives the inbox, to judge its text at the headset's resolution.
                let matrix = Mat4::perspective_rh(0.45, WIDTH as f32 / HEIGHT as f32, 0.05, 50.0)
                    * Mat4::look_at_rh(arrival::HOME, inbox + Vec3::new(0.1, 0.0, 0.0), Vec3::Y);
                renderer.render(
                    &device,
                    &queue,
                    std::slice::from_ref(&view),
                    &[matrix],
                    Mat4::IDENTITY,
                    &panels.list(Mat4::IDENTITY),
                );
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cards test readback"),
                    size: (WIDTH * HEIGHT * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(WIDTH * 4),
                            rows_per_image: Some(HEIGHT),
                        },
                    },
                    texture.size(),
                );
                queue.submit([encoder.finish()]);
                let (sender, receiver) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        let _ = sender.send(result);
                    });
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                receiver.recv().unwrap().unwrap();
                let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
                buffer.unmap();
                image::RgbaImage::from_raw(WIDTH, HEIGHT, data).unwrap()
            };
            let bare = capture(&renderer, &panels);
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let pointed = capture(&renderer, &panels);
            assert_ne!(pointed, bare, "the inbox must draw over the bare scene");
            // Picked up and carried to the right.
            let squeezed = Senses {
                grip: Some(1.0),
                ..senses
            };
            step.update(&squeezed, &mut conversation, &mut felt(), now, 0.01);
            let carried = Senses {
                ray: tomori_scene::pointing::Ray::new(
                    arrival::HOME,
                    inbox + Vec3::new(0.2, 0.1, 0.1) - arrival::HOME,
                ),
                ..squeezed
            };
            let frame = step.update(&carried, &mut conversation, &mut felt(), now, 0.01);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let moved = capture(&renderer, &panels);
            assert_ne!(moved, pointed);
            let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 2);
            image::imageops::replace(&mut sheet, &pointed, 0, 0);
            image::imageops::replace(&mut sheet, &moved, 0, HEIGHT.into());
            crate::test_support::preview("TOMORI_CARDS_PREVIEW", &sheet);
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }

    /// Set `TOMORI_PROJECTION_PREVIEW` to a PNG path to save what this renders: the Tomori wall where the server stands it
    /// by default, curving below the swarm, with four text panels and two code panels, from home, then with one pointed
    /// at and named on the ray, and one pulled off and carried, fitting its text, towards a slot whose outline shows
    /// where it would go, and the Rust code panel close up, highlighted.
    #[test]
    fn a_projection_wall_and_its_panels_render_over_the_scene() {
        const WIDTH: u32 = 960;
        const HEIGHT: u32 = 720;
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("projection test eye"),
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
            let place = |slot: u32| tomori_protocol::Place::Wall {
                wall: tomori_protocol::TOMORI_WALL.into(),
                slot,
            };
            let shown = vec![
                tomori_protocol::Panel::new(
                    "panel-1",
                    "Build status",
                    "**3 crates** failed:\n\n- `tomori-scene`: 2 tests\n- `tomori-render`: lavapipe\n- `tomori-server`: coverage",
                    place(0),
                )
                .with_meta(vec![tomori_protocol::Meta::new("source", "CI")])
                .with_age(300),
                tomori_protocol::Panel::new(
                    "panel-2",
                    "Steps to release",
                    "1. Bump the version.\n2. Run `cargo bake test:coverage`.\n3. Tag and push.\n4. Upload the APK.",
                    place(1),
                )
                .with_age(120),
                tomori_protocol::Panel::new(
                    "panel-3",
                    "Summary",
                    "Projection walls hold **text panels** the agent writes in `panel` blocks. Anyone can pull one off, put it back, or carry it and press B to throw it away.",
                    place(3),
                )
                .with_meta(vec![tomori_protocol::Meta::new("issue", "#82")])
                .with_age(30),
                tomori_protocol::Panel::new(
                    "panel-4",
                    "日本語のパネル",
                    "壁に投影されたテキスト。\n\n| crate | regions |\n|---|---|\n| scene | 100% |",
                    place(4),
                )
                .with_age(10),
            ];
            // Two code panels, as the server highlights them, with spans of every kind.
            let span = tomori_protocol::Span::new;
            use tomori_protocol::TokenKind as Kind;
            let rust = "#[derive(Debug)]\nstruct Point { x: f32 }\n\n/// The origin.\nfn origin() -> Point {\n    let x = 0.0 + 1.5 * 2.0; // half\n    println!(\"{x}\");\n    Point { x: X_MIN }\n}";
            let code = vec![
                tomori_protocol::Panel::new("panel-5", "rust", rust, place(2))
                    .with_code(tomori_protocol::Code::new("rust", 9).with_spans(vec![
                        span(0, 16, Kind::Attribute),
                        span(17, 6, Kind::Keyword),
                        span(24, 5, Kind::Type),
                        span(30, 1, Kind::Punctuation),
                        span(32, 1, Kind::Variable),
                        span(35, 3, Kind::Type),
                        span(42, 15, Kind::Comment),
                        span(58, 2, Kind::Keyword),
                        span(61, 6, Kind::Function),
                        span(70, 2, Kind::Operator),
                        span(73, 5, Kind::Type),
                        span(85, 3, Kind::Keyword),
                        span(91, 1, Kind::Operator),
                        span(93, 3, Kind::Number),
                        span(97, 1, Kind::Operator),
                        span(99, 3, Kind::Number),
                        span(103, 1, Kind::Operator),
                        span(105, 3, Kind::Number),
                        span(108, 1, Kind::Punctuation),
                        span(110, 7, Kind::Comment),
                        span(122, 8, Kind::Function),
                        span(131, 5, Kind::String),
                        span(143, 5, Kind::Type),
                        span(154, 5, Kind::Constant),
                    ]))
                    .with_age(5),
                tomori_protocol::Panel::new(
                    "panel-6",
                    "python",
                    (1..=16)
                        .map(|line| format!("total += values[{line}]  # step {line}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    place(5),
                )
                .with_code(tomori_protocol::Code::new("python", 40))
                .with_pinned(true)
                .with_age(60),
            ];
            let plain: Vec<tomori_protocol::Panel> = shown
                .iter()
                .cloned()
                .chain(code.iter().cloned().map(|mut panel| {
                    panel.code = Some(tomori_protocol::Code::new("rust", 9));
                    panel
                }))
                .collect();
            let shown: Vec<tomori_protocol::Panel> = shown.into_iter().chain(code).collect();
            let mut conversation = joined();
            show_projection(&mut conversation, Vec::new(), shown);
            let head = Mat4::from_translation(arrival::HOME);
            let wall = tomori_wall();
            let wall_at = Vec3::from_array(wall.pose.position);
            let wall_frame = Mat4::from_rotation_translation(
                glam::Quat::from_array(wall.pose.orientation),
                wall_at,
            );
            let slots = tomori_scene::projection::slot_frames(&wall, wall_frame);
            let slot = |index: u32| slots[index as usize].w_axis.truncate();
            // Looking ahead at the swarm, with the wall below it.
            let looking = Vec3::new(0.0, 1.0, -2.3);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(arrival::HOME),
                space_head: head,
                trigger: Some(0.0),
                beam: true,
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, slot(0) - arrival::HOME),
                ..Default::default()
            };
            let mut step = Step::default();
            let now = Instant::now();
            // About the field of view the headset gives the wall, to judge its text at the headset's resolution.
            let lens = Mat4::perspective_rh(1.1, WIDTH as f32 / HEIGHT as f32, 0.05, 50.0);
            let from_home = lens * Mat4::look_at_rh(arrival::HOME, looking, Vec3::Y);
            // Close up: the Rust code panel, in the third slot of the curved wall, from 0.8 m in front of it.
            let facing = slots[2].transform_vector3(Vec3::Z).normalize();
            let close = lens * Mat4::look_at_rh(slot(2) + facing * 0.8, slot(2), Vec3::Y);
            let capture_from = |renderer: &Renderer, panels: &Panels, matrix: Mat4| {
                renderer.render(
                    &device,
                    &queue,
                    std::slice::from_ref(&view),
                    &[matrix],
                    Mat4::IDENTITY,
                    &panels.list(Mat4::IDENTITY),
                );
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("projection test readback"),
                    size: (WIDTH * HEIGHT * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(WIDTH * 4),
                            rows_per_image: Some(HEIGHT),
                        },
                    },
                    texture.size(),
                );
                queue.submit([encoder.finish()]);
                let (sender, receiver) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        let _ = sender.send(result);
                    });
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                receiver.recv().unwrap().unwrap();
                let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
                buffer.unmap();
                image::RgbaImage::from_raw(WIDTH, HEIGHT, data).unwrap()
            };
            let capture =
                |renderer: &Renderer, panels: &Panels| capture_from(renderer, panels, from_home);
            let bare = capture(&renderer, &panels);
            // Looking at the wall, pointing at nothing.
            let away = Senses {
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, -Vec3::Y),
                ..senses
            };
            let frame = step.update(&away, &mut conversation, &mut felt(), now, 0.01);
            let wall_sheets = tomori_protocol::WALL_SLOTS as usize + 1;
            assert_eq!(
                frame.projection.len(),
                wall_sheets + 6,
                "the wall's slots, its label and its panels"
            );
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let wall_drawn = capture(&renderer, &panels);
            assert_ne!(wall_drawn, bare, "the wall must draw over the bare scene");
            let highlighted = capture_from(&renderer, &panels, close);
            // The same code without its spans is drawn plain, which differs: its kinds are coloured.
            let mut plain_conversation = joined();
            show_projection(&mut plain_conversation, Vec::new(), plain);
            let mut plain_step = Step::default();
            let frame = plain_step.update(&away, &mut plain_conversation, &mut felt(), now, 0.01);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let unhighlighted = capture_from(&renderer, &panels, close);
            assert_ne!(highlighted, unhighlighted, "code must be coloured by kind");
            // Coloured pixels, such as the keywords' violet, where plain code has none.
            let violet = |image: &image::RgbaImage| {
                image
                    .pixels()
                    .filter(|pixel| {
                        let [r, g, b, _] = pixel.0.map(i32::from);
                        b > 150 && r > 120 && g + 30 < r
                    })
                    .count()
            };
            assert!(violet(&highlighted) > 20 + violet(&unhighlighted));
            // Pointing at the first panel names it on the ray.
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            assert_eq!(
                frame.projection.len(),
                wall_sheets + 6 + 1,
                "with the label"
            );
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let pointed = capture(&renderer, &panels);
            assert_ne!(pointed, wall_drawn);
            // Pulled off and carried towards the last slot, whose outline shows where it would go, over the code panel there.
            let squeezed = Senses {
                grip: Some(1.0),
                ..senses
            };
            step.update(&squeezed, &mut conversation, &mut felt(), now, 0.01);
            let carried = Senses {
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, slot(5) - arrival::HOME),
                ..squeezed
            };
            let frame = step.update(&carried, &mut conversation, &mut felt(), now, 0.01);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let moved = capture(&renderer, &panels);
            assert_ne!(moved, pointed);
            let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 4);
            image::imageops::replace(&mut sheet, &wall_drawn, 0, 0);
            image::imageops::replace(&mut sheet, &pointed, 0, HEIGHT.into());
            image::imageops::replace(&mut sheet, &moved, 0, (2 * HEIGHT).into());
            image::imageops::replace(&mut sheet, &highlighted, 0, (3 * HEIGHT).into());
            crate::test_support::preview("TOMORI_PROJECTION_PREVIEW", &sheet);
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }

    /// What `texture`, `width` by `height`, holds once drawn.
    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        (width, height): (u32, u32),
    ) -> image::RgbaImage {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test readback"),
            size: (width * height * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receiver.recv().unwrap().unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        buffer.unmap();
        image::RgbaImage::from_raw(width, height, data).unwrap()
    }

    /// Set `TOMORI_CARRY_HINT_PREVIEW` to a PNG path to save what this renders, at about the headset's resolution: a
    /// panel carried 1.6 m ahead, with the hint below it saying what each control does, then carried over a wall's slot,
    /// the hint saying it would swap, and a Rust code panel on the Tomori wall whose long lines wrap under a hanging
    /// indent, close up from 0.5 m, then the same from home.
    #[test]
    fn a_panel_carried_shows_its_hint_and_a_code_panel_wraps_its_long_lines() {
        const WIDTH: u32 = 960;
        const HEIGHT: u32 = 720;
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("carry hint test eye"),
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
            // Lines of Rust longer than a code panel's 45 columns, with spans of their kinds that cross where they wrap.
            let rust = [
                "async fn fetch(client: &Client, request: Request) -> Result<()> {",
                "    let response = client.send(request, timeout(30)).await?;",
                "    log::info!(\"sent {} and got {}\", response.id, response.code);",
                "    Ok(())",
                "}",
            ]
            .join("\n");
            let mut spans = Vec::new();
            let mut from = 0;
            for (text, kind) in [
                ("async", tomori_protocol::TokenKind::Keyword),
                ("fn", tomori_protocol::TokenKind::Keyword),
                ("fetch", tomori_protocol::TokenKind::Function),
                ("Client", tomori_protocol::TokenKind::Type),
                ("Request", tomori_protocol::TokenKind::Type),
                ("Result", tomori_protocol::TokenKind::Type),
                ("let", tomori_protocol::TokenKind::Keyword),
                ("send", tomori_protocol::TokenKind::Function),
                ("timeout", tomori_protocol::TokenKind::Function),
                ("30", tomori_protocol::TokenKind::Number),
                ("await", tomori_protocol::TokenKind::Keyword),
                ("info!", tomori_protocol::TokenKind::Function),
                ("\"sent {} and got {}\"", tomori_protocol::TokenKind::String),
                ("Ok", tomori_protocol::TokenKind::Type),
            ] {
                let at = from + rust[from..].find(text).unwrap();
                spans.push(tomori_protocol::Span::new(
                    rust[..at].chars().count() as u32,
                    text.chars().count() as u32,
                    kind,
                ));
                from = at + text.len();
            }
            assert!(tomori_protocol::valid_spans(&rust, &spans));
            let code = tomori_protocol::Panel::new(
                "panel-2",
                "rust",
                rust.clone(),
                tomori_protocol::Place::Wall {
                    wall: tomori_protocol::TOMORI_WALL.into(),
                    slot: 2,
                },
            )
            .with_code(tomori_protocol::Code::new("rust", 5).with_spans(spans))
            .with_age(5);
            let ahead = arrival::HOME + Vec3::new(0.0, -0.1, -1.6);
            let shown = vec![
                tomori_protocol::Panel::new(
                    "panel-1",
                    "Steps to release",
                    "1. Bump the version.\n2. Run `cargo bake test:coverage`.\n3. Tag and push.",
                    tomori_protocol::Place::Free {
                        pose: tomori_protocol::Pose::upright_facing(
                            ahead.to_array(),
                            arrival::HOME.to_array(),
                        ),
                    },
                )
                .with_age(60),
                code.clone(),
            ];
            let mut conversation = joined();
            show_projection(&mut conversation, Vec::new(), shown);
            let wall = tomori_wall();
            let wall_frame = Mat4::from_rotation_translation(
                glam::Quat::from_array(wall.pose.orientation),
                Vec3::from_array(wall.pose.position),
            );
            let slots = tomori_scene::projection::slot_frames(&wall, wall_frame);
            let slot = |index: usize| slots[index].w_axis.truncate();
            let head = Mat4::from_translation(arrival::HOME);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(arrival::HOME),
                space_head: head,
                trigger: Some(0.0),
                beam: true,
                grip: Some(0.0),
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, ahead - arrival::HOME),
                ..Default::default()
            };
            let mut step = Step::default();
            let now = Instant::now();
            // About the headset's field of view and resolution, to judge the text as it would be read.
            let lens = Mat4::perspective_rh(1.1, WIDTH as f32 / HEIGHT as f32, 0.05, 50.0);
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
            // Pulled off and carried, the hint shows below it.
            step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            let carried = Senses {
                grip: Some(1.0),
                ..senses
            };
            step.update(&carried, &mut conversation, &mut felt(), now, 0.01);
            let frame = step.update(&carried, &mut conversation, &mut felt(), now, 0.01);
            let hint = frame.projection.last().unwrap().canvas.as_ref().unwrap();
            let text = |canvas: &tomori_scene::canvas::Canvas| {
                canvas
                    .rows
                    .iter()
                    .map(|row| row.plain())
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            assert!(text(hint).contains("throw away, for all"));
            let hint_at = frame
                .projection
                .last()
                .unwrap()
                .model
                .unwrap()
                .w_axis
                .truncate();
            assert!(hint_at.y < ahead.y);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let looking = ahead - Vec3::Y * 0.2;
            let from_home = lens * Mat4::look_at_rh(arrival::HOME, looking, Vec3::Y);
            let hinted = capture(&renderer, &panels, from_home);
            // Without its hint, the same frame differs: the hint draws, and its warm row with it.
            let mut bare = frame.projection.clone();
            bare.pop();
            let without = tomori_scene::scene::Frame {
                projection: bare,
                ..frame
            };
            draw(&mut renderer, &device, &queue, &mut panels, &without);
            let unhinted = capture(&renderer, &panels, from_home);
            assert_ne!(hinted, unhinted, "the hint must draw");
            let warm = |image: &image::RgbaImage| {
                image
                    .pixels()
                    .filter(|pixel| {
                        let [r, g, b, _] = pixel.0.map(i32::from);
                        r > 150 && g + 25 < r && b + 25 < g
                    })
                    .count()
            };
            assert!(
                warm(&hinted) > 20 + warm(&unhinted),
                "the throw row is warm"
            );
            // Carried over the wall's slot holding the code panel, the hint says the two would change places.
            let over = Senses {
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, slot(2) - arrival::HOME),
                ..carried
            };
            let frame = step.update(&over, &mut conversation, &mut felt(), now, 0.01);
            let hint = frame.projection.last().unwrap().canvas.as_ref().unwrap();
            let said = text(hint);
            assert!(said.ends_with("swap with that one"), "{said}");
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let toward = lens * Mat4::look_at_rh(arrival::HOME, slot(2) + Vec3::Y * 0.25, Vec3::Y);
            let swapping = capture(&renderer, &panels, toward);
            // The code panel on its own, close up, from 0.5 m, and from home.
            let mut code_only = joined();
            show_projection(&mut code_only, Vec::new(), vec![code]);
            let away = Senses {
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, -Vec3::Y),
                ..senses
            };
            let mut code_step = Step::default();
            let frame = code_step.update(&away, &mut code_only, &mut felt(), now, 0.01);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let facing = slots[2].transform_vector3(Vec3::Z).normalize();
            let close = lens * Mat4::look_at_rh(slot(2) + facing * 0.5, slot(2), Vec3::Y);
            let wrapped = capture(&renderer, &panels, close);
            let from_home =
                lens * Mat4::look_at_rh(arrival::HOME, slot(2) + Vec3::Y * 0.1, Vec3::Y);
            let wrapped_from_home = capture(&renderer, &panels, from_home);
            // The string's green carries onto the row its line wraps onto.
            let green = |image: &image::RgbaImage| {
                image
                    .pixels()
                    .filter(|pixel| {
                        let [r, g, b, _] = pixel.0.map(i32::from);
                        g > 120 && r + 15 < g && b + 20 < g
                    })
                    .count()
            };
            let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 4);
            image::imageops::replace(&mut sheet, &hinted, 0, 0);
            image::imageops::replace(&mut sheet, &swapping, 0, HEIGHT.into());
            image::imageops::replace(&mut sheet, &wrapped, 0, (2 * HEIGHT).into());
            image::imageops::replace(&mut sheet, &wrapped_from_home, 0, (3 * HEIGHT).into());
            crate::test_support::preview("TOMORI_CARRY_HINT_PREVIEW", &sheet);
            assert!(green(&wrapped) > 20, "the string is coloured");
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }

    /// Set `TOMORI_CODE_WRAP_PREVIEW` to a PNG path to save what this renders, at about the headset's resolution: the
    /// block of Rust the maintainer looked at on the Quest 3, on the Tomori wall from 0.5 m, as an older server sent it,
    /// each line cut to 64 characters with "…", then as it is sent now, every line whole, a long one wrapping onto as many
    /// rows as it needs, then the same from home.
    #[test]
    fn a_code_panel_shows_every_line_whole_however_many_rows_it_wraps_onto() {
        const WIDTH: u32 = 960;
        const HEIGHT: u32 = 720;
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("code wrap test eye"),
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
            let rust = [
                "use std::collections::HashMap;",
                "",
                "fn summarise(orders: &[(String, u32, f64)]) -> HashMap<String, (u32, f64)> {",
                "    let mut totals = HashMap::new();",
                "    for (customer, quantity, price) in orders {",
                "        let entry = totals.entry(customer.clone()).or_insert((0, 0.0));",
                "        entry.0 += quantity;",
                "        entry.1 += *quantity as f64 * price; // running total of what each customer has spent across every order so far",
                "    }",
                "    totals",
                "}",
            ];
            // The spans of `body`, of these texts in order, as a highlighter would find them, those past where a line
            // is cut left out.
            let spans = |body: &str| {
                let mut spans = Vec::new();
                let mut from = 0;
                for (text, kind) in [
                    ("use", tomori_protocol::TokenKind::Keyword),
                    ("HashMap", tomori_protocol::TokenKind::Type),
                    ("fn", tomori_protocol::TokenKind::Keyword),
                    ("summarise", tomori_protocol::TokenKind::Function),
                    ("String", tomori_protocol::TokenKind::Type),
                    ("HashMap", tomori_protocol::TokenKind::Type),
                    ("let", tomori_protocol::TokenKind::Keyword),
                    ("mut", tomori_protocol::TokenKind::Keyword),
                    ("new", tomori_protocol::TokenKind::Function),
                    ("for", tomori_protocol::TokenKind::Keyword),
                    ("in", tomori_protocol::TokenKind::Keyword),
                    ("let", tomori_protocol::TokenKind::Keyword),
                    ("entry", tomori_protocol::TokenKind::Function),
                    ("clone", tomori_protocol::TokenKind::Function),
                    ("or_insert", tomori_protocol::TokenKind::Function),
                    ("0", tomori_protocol::TokenKind::Number),
                    ("0.0", tomori_protocol::TokenKind::Number),
                    ("as", tomori_protocol::TokenKind::Keyword),
                    (
                        "// running total of what each customer has spent across every order so far",
                        tomori_protocol::TokenKind::Comment,
                    ),
                ] {
                    let Some(at) = body[from..].find(text).map(|at| from + at) else {
                        continue;
                    };
                    spans.push(tomori_protocol::Span::new(
                        body[..at].chars().count() as u32,
                        text.chars().count() as u32,
                        kind,
                    ));
                    from = at + text.len();
                }
                assert!(tomori_protocol::valid_spans(body, &spans));
                spans
            };
            let panel = |body: String| {
                let spans = spans(&body);
                tomori_protocol::Panel::new(
                    "panel-1",
                    "rust",
                    body,
                    tomori_protocol::Place::Wall {
                        wall: tomori_protocol::TOMORI_WALL.into(),
                        slot: 2,
                    },
                )
                .with_code(tomori_protocol::Code::new("rust", rust.len() as u32).with_spans(spans))
                .with_age(5)
            };
            let cut = rust
                .iter()
                .map(|line| match line.char_indices().nth(64) {
                    Some((at, _)) => format!("{}…", &line[..at]),
                    None => (*line).to_owned(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let whole = rust.join("\n");
            let wall = tomori_wall();
            let wall_frame = Mat4::from_rotation_translation(
                glam::Quat::from_array(wall.pose.orientation),
                Vec3::from_array(wall.pose.position),
            );
            let slots = tomori_scene::projection::slot_frames(&wall, wall_frame);
            let slot = slots[2].w_axis.truncate();
            let facing = slots[2].transform_vector3(Vec3::Z).normalize();
            let head = Mat4::from_translation(arrival::HOME);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(arrival::HOME),
                space_head: head,
                ray: tomori_scene::pointing::Ray::new(arrival::HOME, -Vec3::Y),
                ..Default::default()
            };
            let lens = Mat4::perspective_rh(1.1, WIDTH as f32 / HEIGHT as f32, 0.05, 50.0);
            let close = lens * Mat4::look_at_rh(slot + facing * 0.5, slot, Vec3::Y);
            let from_home = lens * Mat4::look_at_rh(arrival::HOME, slot + Vec3::Y * 0.1, Vec3::Y);
            let mut capture = |body: String, matrix: Mat4| {
                let mut conversation = joined();
                show_projection(&mut conversation, Vec::new(), vec![panel(body)]);
                let mut step = Step::default();
                let frame = step.update(
                    &senses,
                    &mut conversation,
                    &mut felt(),
                    Instant::now(),
                    0.01,
                );
                draw(&mut renderer, &device, &queue, &mut panels, &frame);
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
            let before = capture(cut, close);
            let after = capture(whole.clone(), close);
            let after_from_home = capture(whole, from_home);
            // What was cut is drawn, the rest of the sixth line and of the comment, so there is more text on the panel.
            let ink = |image: &image::RgbaImage| {
                image
                    .pixels()
                    .filter(|pixel| pixel.0[..3].iter().any(|channel| *channel > 120))
                    .count()
            };
            let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
            image::imageops::replace(&mut sheet, &before, 0, 0);
            image::imageops::replace(&mut sheet, &after, 0, HEIGHT.into());
            image::imageops::replace(&mut sheet, &after_from_home, 0, (2 * HEIGHT).into());
            crate::test_support::preview("TOMORI_CODE_WRAP_PREVIEW", &sheet);
            assert_ne!(before, after, "the whole lines must draw");
            let (after_ink, before_ink) = (ink(&after), ink(&before));
            assert!(
                after_ink > before_ink + 200,
                "what was cut is drawn: {after_ink} against {before_ink}"
            );
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }
}
