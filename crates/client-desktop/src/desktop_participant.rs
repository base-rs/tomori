// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop window as a participant: it connects to a Tomori server and presents arrival, the space and the hand menu as the headset does, seen from a head at standing height that the mouse turns, with the keyboard for the controller's buttons and the mouse for its ray. See `design/desktop.md`.
use crate::{
    connection::Connection,
    conversation::Conversation,
    frame_timing::{self, FrameTimes, Segment, Timer},
    graphics::Renderer,
    log_cadence::{self, Cadence},
    scene::desktop::{Button, Desktop, Direction, Key, KeyResult, TITLE},
    scene_graphics::{self, Panels},
    spatial_audio,
    window::{Contents, Flow, Gpu, Input, Settings, Surface, Wake},
};
use anyhow::Result;
use glam::{Mat4, Vec2};
use std::time::{Duration, Instant};
use tomori_render::scenery::Sceneries;
use winit::{dpi::LogicalSize, event::MouseButton, keyboard::KeyCode};

/// What each key does, shown at startup and by `--help`; the keys stand for the headset's controls in `design/interaction.md`.
pub const CONTROLS: &str = "Mouse: point (as the right controller's ray); left button: choose what is pointed at, or attract the swarm; right drag: look around.\n\
Enter: A. Backspace: B (hold for a second to leave the space). Space (hold): X, push-to-talk. Tab: Y, which in a space pins the hand menu open at its Space section.\n\
Arrow keys: left stick, which in a space that allows travel teleports (hold Up, point at the floor, release), turns (Left and Right) and rises (hold Down). [ and ]: right stick left and right. Page Up and Page Down: right stick up and down.\n\
V: mute spoken replies (the hand menu's Spoken replies row). R: play the latest reply again, or stop it (right stick click).\n\
M: open or close the hand menu. T: show or hide the transcript on the right wrist; G: detach it into the world, or reattach it (the right grip). In a space, hold G to carry the panel pointed at: Page Up and Page Down push it further or pull it closer, [ and ] turn it, R squares it up, Backspace throws it away for everyone, and letting go of G puts it down. K: knock on the space you watch. + and -: admit or decline the first knock. P: pair this device as an owner, or cancel.\n\
Esc: close the hand menu, or quit.";

/// The key that `code` stands for, as `CONTROLS` describes.
fn key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Space => Key::Talk,
        KeyCode::Backspace => Key::Back,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::A,
        KeyCode::Tab => Key::Y,
        KeyCode::KeyV => Key::Mute,
        KeyCode::KeyR => Key::Replay,
        KeyCode::ArrowUp => Key::Left(Direction::Up),
        KeyCode::ArrowDown => Key::Left(Direction::Down),
        KeyCode::ArrowLeft => Key::Left(Direction::Left),
        KeyCode::ArrowRight => Key::Left(Direction::Right),
        KeyCode::PageUp => Key::Right(Direction::Up),
        KeyCode::PageDown => Key::Right(Direction::Down),
        KeyCode::BracketLeft => Key::Right(Direction::Left),
        KeyCode::BracketRight => Key::Right(Direction::Right),
        KeyCode::KeyM => Key::Menu,
        KeyCode::KeyT => Key::Transcript,
        KeyCode::KeyG => Key::Grip,
        KeyCode::KeyK => Key::Knock,
        KeyCode::KeyP => Key::Pair,
        KeyCode::Equal | KeyCode::NumpadAdd => Key::Admit,
        KeyCode::Minus | KeyCode::NumpadSubtract => Key::Decline,
        KeyCode::Escape => Key::Escape,
        _ => return None,
    })
}

/// The participant's window: the desktop's logic, [`Desktop`], driven by the window's input, drawn into it and keeping its connection going.
pub struct Participant {
    conversation: Conversation,
    connection: Connection,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    panels: Panels,
    width: u32,
    height: u32,
    occluded: bool,
    /// The desktop's logic, whose first frame steps from when the window was ready.
    desktop: Desktop,
    first_frame: bool,
    /// What each frame drawn costs the CPU, summarized once a minute with the frame rate; see `design/rendering.md`.
    frame_times: FrameTimes,
    frame_log: Cadence,
}

impl Participant {
    pub fn settings() -> Settings {
        Settings {
            title: TITLE,
            size: LogicalSize::new(1280.0, 800.0).into(),
            decorations: true,
            min_size: None,
        }
    }

    /// Take part as `conversation` and `connection` describe, drawn with `gpu`, with nothing but the grid.
    pub fn new(gpu: Gpu, conversation: Conversation, connection: Connection) -> Self {
        Self::with_sceneries(gpu, conversation, connection, &Sceneries::default())
    }

    /// As [`Participant::new`], drawing `sceneries`: the conversation draws their environments, and the grid for any other.
    pub fn with_sceneries(
        gpu: Gpu,
        mut conversation: Conversation,
        connection: Connection,
        sceneries: &Sceneries,
    ) -> Self {
        conversation.set_drawable(sceneries.environments());
        conversation.set_over_grid(sceneries.over_grid());
        conversation.set_locomotion(sceneries.locomotion());
        conversation.set_client_sides(sceneries.client_sides());
        conversation.set_ambiences(sceneries.ambiences());
        conversation.set_worker_placements(sceneries.worker_placements());
        conversation.set_avatars(sceneries.avatars());
        conversation.set_avatar_failures(sceneries.avatar_failures());
        let samples = gpu.samples(gpu.format);
        let renderer = Renderer::with_sceneries(
            &gpu.device,
            &gpu.queue,
            gpu.format,
            gpu.width,
            gpu.height,
            samples,
            sceneries,
        );
        let panels = Panels::new(&gpu.device, &renderer);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.width,
            gpu.height
        );
        log::info!("{CONTROLS}");
        let desktop = Desktop::new(&mut conversation, Instant::now());
        Self {
            conversation,
            connection,
            device: gpu.device,
            queue: gpu.queue,
            renderer,
            panels,
            width: gpu.width,
            height: gpu.height,
            occluded: false,
            desktop,
            first_frame: true,
            frame_times: FrameTimes::default(),
            frame_log: Cadence::new(log_cadence::FRAME_SUMMARY),
        }
    }

    /// Whether nothing drawn to the window would be seen.
    fn hidden(&self) -> bool {
        self.occluded || self.width == 0 || self.height == 0
    }

    fn resize(&mut self, width: u32, height: u32, surface: &mut dyn Surface) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            return;
        }
        surface.resize(width, height);
        self.renderer.resize(&self.device, width, height);
        surface.request_redraw();
    }

    /// One frame of the desktop's logic, returning the head's view and projection, and the transform from the scene to the participant's room, to draw it with, each part timed by `timer`.
    fn step(&mut self, surface: &mut dyn Surface, timer: &mut Timer) -> (Mat4, Mat4) {
        let now = Instant::now();
        let (conversation, connection) = (&mut self.conversation, &mut self.connection);
        timer.time(Segment::Poll, || {
            connection.poll(self.desktop.focused(), conversation)
        });
        let size = Vec2::new(self.width.max(1) as f32, self.height.max(1) as f32);
        let stepping = Instant::now();
        let drawn = self
            .desktop
            .frame(conversation, connection.feedback(), now, size);
        if let Some(title) = &drawn.title {
            surface.set_title(title);
        }
        timer.add(Segment::Step, stepping.elapsed());
        let uploads = timer.time(Segment::Draw, || {
            scene_graphics::draw(
                &mut self.renderer,
                &self.device,
                &self.queue,
                &mut self.panels,
                &drawn.frame,
            )
        });
        timer.meshes(uploads.meshes());
        // Every source is heard from where it is, relative to this participant's head; see `design/spatial-audio.md`.
        timer.time(Segment::Audio, || {
            spatial_audio::listen(
                connection,
                drawn.frame.listener,
                drawn.frame.agent,
                conversation,
            )
        });
        if let Some(presence) = &drawn.frame.presence {
            timer.time(Segment::Presence, || presence.send(connection));
        }
        (drawn.view_projection, drawn.scene_to_space)
    }

    /// One frame of the desktop's logic, drawn, and what it cost this thread kept, summarized with the frame rate at the first frame and then at most once a minute.
    fn frame(&mut self, surface: &mut dyn Surface) -> Result<()> {
        let mut timer = Timer::start();
        let started = Instant::now();
        let (view_projection, scene_to_space) = self.step(surface, &mut timer);
        self.draw(surface, view_projection, scene_to_space, &mut timer)?;
        timer.add(Segment::Frame, started.elapsed());
        self.frame_times.record(timer.finish());
        if let Some(due) = self.frame_log.record(Instant::now())
            && let Some(summary) = self.frame_times.summary()
        {
            log::info!(
                "Desktop frames, {}; {summary}",
                frame_timing::rate(due.rate())
            );
        }
        Ok(())
    }

    /// Draw what the last step decided into the window, from the participant's room: `view_projection` is the head's in it, and `scene_to_space` takes the space's scene, and the panels in it, from where the participant travelled to it. See `design/locomotion.md`.
    fn draw(
        &mut self,
        surface: &mut dyn Surface,
        view_projection: Mat4,
        scene_to_space: Mat4,
        timer: &mut Timer,
    ) -> Result<()> {
        let Some(target) = timer.time(Segment::ImageWait, || surface.acquire())? else {
            return Ok(());
        };
        timer.time(Segment::Render, || {
            self.renderer.render(
                &self.device,
                &self.queue,
                &[target],
                &[view_projection],
                scene_to_space,
                &self.panels.list(scene_to_space),
            )
        });
        surface.present();
        if self.first_frame {
            log::info!("Presented first desktop frame");
            self.first_frame = false;
        }
        Ok(())
    }
}

impl Contents for Participant {
    fn input(&mut self, input: Input, surface: &mut dyn Surface) -> Result<Flow> {
        let (desktop, conversation) = (&mut self.desktop, &mut self.conversation);
        match input {
            Input::Close => return Ok(Flow::Exit),
            Input::Resized { width, height } => self.resize(width, height, surface),
            Input::Focused(focused) => {
                if desktop.set_focused(focused, conversation) {
                    self.connection.request_microphone();
                }
            }
            Input::Occluded(occluded) => {
                self.occluded = occluded;
                desktop.set_visible(!occluded, conversation);
                if !occluded {
                    surface.request_redraw();
                }
            }
            Input::CursorLeft => desktop.cursor_left(),
            Input::CursorMoved(position) => desktop.cursor_moved(position),
            Input::Mouse { button, pressed } => {
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => return Ok(Flow::Continue),
                };
                desktop.mouse(button, pressed);
            }
            Input::Key {
                code,
                pressed,
                repeat,
            } => {
                if let Some(key) = key(code)
                    && desktop.key(key, pressed, repeat, conversation) == KeyResult::Quit
                {
                    return Ok(Flow::Exit);
                }
            }
            // The wheel scrolls the panel the mouse points at, a touchpad's pixels counted in notches.
            Input::Wheel(delta) => desktop.wheel(match delta {
                winit::event::MouseScrollDelta::LineDelta(_, lines) => lines,
                winit::event::MouseScrollDelta::PixelDelta(position) => {
                    position.y as f32 / tomori_scene::scene::desktop::WHEEL_PIXELS
                }
            }),
        }
        Ok(Flow::Continue)
    }

    fn redraw(&mut self, surface: &mut dyn Surface) -> Result<()> {
        if self.hidden() {
            return Ok(());
        }
        self.frame(surface)?;
        surface.request_redraw();
        Ok(())
    }

    fn about_to_wait(&mut self, surface: &mut dyn Surface) -> Result<Wake> {
        // A hidden window draws nothing, so it would never receive a redraw; it still keeps its connection and speech going.
        if !self.hidden() {
            return Ok(Wake::Wait);
        }
        self.step(surface, &mut Timer::start());
        Ok(Wake::At(Instant::now() + Duration::from_millis(20)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Server;
    use crate::{
        test_support::participant,
        window::test::{TestSurface, with_gpu},
    };
    use tomori_protocol::{ClientMessage, ServerMessage};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    /// Redraw until `done`, or fail after a while.
    fn until(
        window: &mut Participant,
        surface: &mut TestSurface,
        mut done: impl FnMut(&Participant) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(window) {
            assert!(Instant::now() < deadline, "timed out");
            window.redraw(surface).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn press(code: KeyCode, pressed: bool) -> Input {
        Input::Key {
            code,
            pressed,
            repeat: false,
        }
    }

    #[test]
    fn the_window_opens_at_the_participants_size_with_its_title() {
        let settings = Participant::settings();
        assert_eq!(settings.title, TITLE);
        assert!(settings.decorations);
        assert_eq!(settings.size, LogicalSize::new(1280.0, 800.0).into());
    }

    #[test]
    fn the_titles_hint_claims_only_what_the_controls_bind() {
        // Each control the title hints at, with what `CONTROLS` says of it and the key that binds it.
        for (hint, controls, code, bound) in [
            ("left choose", "left button: choose", None, None),
            ("right-drag look", "right drag: look around", None, None),
            (
                "M menu",
                "M: open or close the hand menu",
                Some(KeyCode::KeyM),
                Some(Key::Menu),
            ),
            (
                "Esc quit",
                "Esc: close the hand menu, or quit",
                Some(KeyCode::Escape),
                Some(Key::Escape),
            ),
        ] {
            assert!(TITLE.contains(hint), "{hint}");
            assert!(CONTROLS.contains(controls), "{controls}");
            assert_eq!(code.and_then(key), bound);
        }
    }

    #[test]
    fn the_window_draws_when_seen_and_keeps_its_connection_going_when_hidden() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let (conversation, connection) = participant("Ada", None);
            let mut window = Participant::new(gpu, conversation, connection);
            window.redraw(&mut surface).unwrap();
            window.redraw(&mut surface).unwrap();
            assert_eq!((surface.presented, surface.redraws), (2, 2));
            assert_eq!(window.about_to_wait(&mut surface).unwrap(), Wake::Wait);
            // Focus asks for the microphone, once.
            for focused in [true, false, true] {
                window.input(Input::Focused(focused), &mut surface).unwrap();
            }
            assert!(window.desktop.focused());
            // Hidden, it steps without drawing, and wakes itself to keep going.
            window.input(Input::Occluded(true), &mut surface).unwrap();
            window.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 2);
            assert_ne!(window.about_to_wait(&mut surface).unwrap(), Wake::Wait);
            assert_eq!(surface.presented, 2);
            window.input(Input::Occluded(false), &mut surface).unwrap();
            assert_eq!(surface.redraws, 3);
            // Minimised, too; a new size is configured and drawn at.
            for (width, height) in [(0, 0), (64, 48)] {
                window
                    .input(Input::Resized { width, height }, &mut surface)
                    .unwrap();
            }
            assert_eq!(surface.sizes, [(64, 48)]);
            // Nothing to draw into this time: nothing is presented.
            surface.available = false;
            window.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 2);
            surface.fail = true;
            assert!(window.redraw(&mut surface).is_err());
        });
    }

    #[test]
    fn the_mouse_and_keys_reach_the_desktops_logic() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let (conversation, connection) = participant("Ada", None);
            let mut window = Participant::new(gpu, conversation, connection);
            let mut input = |input| window.input(input, &mut surface).unwrap();
            input(Input::Focused(true));
            input(Input::CursorMoved(Vec2::new(10.0, 10.0)));
            for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
                for pressed in [true, false] {
                    assert_eq!(input(Input::Mouse { button, pressed }), Flow::Continue);
                }
            }
            input(Input::CursorLeft);
            assert_eq!(
                input(Input::Wheel(winit::event::MouseScrollDelta::LineDelta(
                    0.0, 1.0
                ))),
                Flow::Continue
            );
            // A touchpad's scroll, in pixels, scrolls as the wheel's notches.
            assert_eq!(
                input(Input::Wheel(winit::event::MouseScrollDelta::PixelDelta(
                    winit::dpi::PhysicalPosition::new(0.0, -80.0)
                ))),
                Flow::Continue
            );
            // M opens the hand menu, which Escape closes before it quits.
            assert_eq!(input(press(KeyCode::KeyM, true)), Flow::Continue);
            assert_eq!(input(press(KeyCode::KeyQ, true)), Flow::Continue);
            assert_eq!(input(press(KeyCode::Escape, true)), Flow::Continue);
            assert_eq!(input(press(KeyCode::Escape, false)), Flow::Continue);
            assert_eq!(input(press(KeyCode::Escape, true)), Flow::Exit);
            assert_eq!(input(Input::Close), Flow::Exit);
        });
    }

    #[test]
    fn in_a_space_its_presence_is_sent_and_a_knock_is_named_in_the_title() {
        with_gpu(FORMAT, |gpu| {
            let server = Server::start();
            let bea = server.remote("Bea", Some(server.owner("Bea")));
            let space = bea.create_space("Studio", || {}).id;
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let (mut conversation, connection) =
                server.participant("Ada", Some(server.owner("Ada")));
            conversation.join_space(space.clone());
            let mut window = Participant::new(gpu, conversation, connection);
            // The others see where this participant's head is.
            bea.wait_for(
                || window.redraw(&mut surface).unwrap(),
                |message| matches!(message, ServerMessage::Presence { .. }).then_some(()),
            );
            // Space held is push-to-talk, which opens the microphone.
            window.input(Input::Focused(true), &mut surface).unwrap();
            window
                .input(press(KeyCode::Space, true), &mut surface)
                .unwrap();
            until(&mut window, &mut surface, |window| {
                window.conversation.recording().is_some()
            });
            window
                .input(press(KeyCode::Space, false), &mut surface)
                .unwrap();
            let vic = server.remote("Vic", None);
            vic.send(ClientMessage::Join {
                space: space.clone(),
            });
            vic.wait_for(
                || {},
                |message| matches!(message, ServerMessage::Space { .. }).then_some(()),
            );
            vic.send(ClientMessage::Knock);
            until(&mut window, &mut surface, |window| {
                !window.conversation.knocks().is_empty()
            });
            assert!(surface.titles.last().is_some_and(|title| {
                title.starts_with("Tomori — Vic (visitor) is knocking")
                    && title.ends_with(
                        "| mouse: point · left choose · right-drag look · M menu · Esc quit",
                    )
            }));
            // + admits the first knock, and the title says no one is knocking.
            window
                .input(press(KeyCode::Equal, true), &mut surface)
                .unwrap();
            until(&mut window, &mut surface, |window| {
                window.conversation.knocks().is_empty()
            });
            assert_eq!(surface.titles.last().map(String::as_str), Some(TITLE));
        });
    }

    /// A worker placement that stands every worker at its origin.
    struct AtOrigin;

    impl tomori_scene::anchors::WorkerPlacement for AtOrigin {
        fn place(
            &self,
            origin: tomori_scene::anchors::Anchor,
            _worker: &tomori_scene::anchors::Placing<'_>,
        ) -> glam::Vec3 {
            origin.position
        }
    }

    #[test]
    fn after_travelling_the_window_draws_the_space_and_its_panels_from_where_the_participant_is() {
        use glam::{Quat, Vec3};
        use tomori_scene::locomotion::{Locomotion, Surface};
        with_gpu(FORMAT, |gpu| {
            let (device, queue) = (gpu.device.clone(), gpu.queue.clone());
            let server = Server::start();
            let bea = server.remote("Bea", Some(server.owner("Bea")));
            let space = bea.create_space("Studio", || {});
            assert_eq!(space.environment, "sky");
            let (mut conversation, connection) =
                server.participant("Ada", Some(server.owner("Ada")));
            conversation.join_space(space.id);
            // A scenery for the space's environment, as an application registers one.
            let mut sceneries = tomori_render::scenery::test::Counted::default().sceneries("sky");
            sceneries.set_locomotion(
                "sky",
                Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
            );
            sceneries.set_worker_placement("sky", AtOrigin);
            let mut window = Participant::with_sceneries(gpu, conversation, connection, &sceneries);
            // The build's worker placement reaches the conversation, as its client sides do.
            let placements = window.conversation.worker_placements();
            assert_eq!(placements.len(), 1);
            assert_eq!(
                placements[0].1.place(
                    tomori_scene::anchors::Anchor::AGENT,
                    &tomori_scene::anchors::Placing::new("ses_1", 0)
                ),
                tomori_scene::swarm::CENTER
            );
            let mut surface = TestSurface::new(&device, FORMAT);
            let input = |window: &mut Participant, surface: &mut TestSurface, input| {
                window.input(input, surface).unwrap();
                window.redraw(surface).unwrap();
            };
            input(&mut window, &mut surface, Input::Focused(true));
            // Low in the window, the mouse points at the floor a little ahead.
            input(
                &mut window,
                &mut surface,
                Input::CursorMoved(Vec2::new(32.0, 46.0)),
            );
            // Once the space's scenery has grown in, ↑ held and released lands there, and ← turns.
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                assert!(Instant::now() < deadline, "timed out travelling");
                for (code, pressed) in [
                    (KeyCode::ArrowUp, true),
                    (KeyCode::ArrowUp, false),
                    (KeyCode::ArrowLeft, true),
                    (KeyCode::ArrowLeft, false),
                ] {
                    input(&mut window, &mut surface, press(code, pressed));
                }
                let (_, scene_to_space) = window.step(&mut surface, &mut Timer::start());
                let (_, turned, moved) = scene_to_space.to_scale_rotation_translation();
                if turned.angle_between(Quat::IDENTITY) > 0.1 && moved.length() > 0.1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // The presence the window sends has its travel applied, so Bea receives the head where the participant now is in the shared scene, which their room takes back to where they stand at home, and hears them there.
            let (_, scene_to_space) = window.step(&mut surface, &mut Timer::start());
            let home = tomori_scene::arrival::HOME;
            let head = bea.wait_for(
                || window.redraw(&mut surface).unwrap(),
                |message| match message {
                    ServerMessage::Presence { head, .. }
                        if scene_to_space
                            .transform_point3(Vec3::from_array(head.position))
                            .distance(home)
                            < 1e-4 =>
                    {
                        Some(*head)
                    }
                    _ => None,
                },
            );
            assert!(Vec3::from_array(head.position).distance(home) > 0.1);
            // The transcript shows on the right wrist, a panel in the scene, where the participant now is.
            input(&mut window, &mut surface, press(KeyCode::KeyT, true));
            input(&mut window, &mut surface, press(KeyCode::KeyT, false));
            // The transcript is drawn last, after the pill, which stands in the eyes' space; the space's projection
            // wall, also in the scene, is drawn first, so it is the last panel that says the transcript shows.
            let last = |window: &Participant, scene_to_space: Mat4| -> Option<Mat4> {
                let list = window.panels.list(scene_to_space);
                list.last().map(|(_, model)| *model)
            };
            let (view_projection, scene_to_space) = loop {
                assert!(
                    Instant::now() < deadline,
                    "timed out showing the transcript"
                );
                let (view_projection, scene_to_space) =
                    window.step(&mut surface, &mut Timer::start());
                if last(&window, scene_to_space) != last(&window, Mat4::IDENTITY) {
                    break (view_projection, scene_to_space);
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            window
                .draw(
                    &mut surface,
                    view_projection,
                    scene_to_space,
                    &mut Timer::start(),
                )
                .unwrap();
            let drawn = surface.pixels(&device, &queue);
            // What the window draws is the space, and the panels in it, both taken to the participant's room; drawing either from home instead would differ.
            let render = |world: Mat4, panels: Mat4| {
                let target = surface.texture.create_view(&Default::default());
                window.renderer.render(
                    &device,
                    &queue,
                    &[target],
                    &[view_projection],
                    world,
                    &window.panels.list(panels),
                );
                surface.pixels(&device, &queue)
            };
            assert!(drawn == render(scene_to_space, scene_to_space));
            assert!(
                drawn != render(Mat4::IDENTITY, scene_to_space),
                "the space from home"
            );
            assert!(
                drawn != render(scene_to_space, Mat4::IDENTITY),
                "the panels from home"
            );
        });
    }

    #[test]
    fn every_control_has_a_key_and_other_keys_do_nothing() {
        let bound = [
            (KeyCode::Space, Key::Talk),
            (KeyCode::Backspace, Key::Back),
            (KeyCode::Enter, Key::A),
            (KeyCode::NumpadEnter, Key::A),
            (KeyCode::Tab, Key::Y),
            (KeyCode::KeyV, Key::Mute),
            (KeyCode::KeyR, Key::Replay),
            (KeyCode::ArrowUp, Key::Left(Direction::Up)),
            (KeyCode::ArrowDown, Key::Left(Direction::Down)),
            (KeyCode::ArrowLeft, Key::Left(Direction::Left)),
            (KeyCode::ArrowRight, Key::Left(Direction::Right)),
            (KeyCode::PageUp, Key::Right(Direction::Up)),
            (KeyCode::PageDown, Key::Right(Direction::Down)),
            (KeyCode::BracketLeft, Key::Right(Direction::Left)),
            (KeyCode::BracketRight, Key::Right(Direction::Right)),
            (KeyCode::KeyM, Key::Menu),
            (KeyCode::KeyT, Key::Transcript),
            (KeyCode::KeyG, Key::Grip),
            (KeyCode::KeyK, Key::Knock),
            (KeyCode::KeyP, Key::Pair),
            (KeyCode::Equal, Key::Admit),
            (KeyCode::NumpadAdd, Key::Admit),
            (KeyCode::Minus, Key::Decline),
            (KeyCode::NumpadSubtract, Key::Decline),
            (KeyCode::Escape, Key::Escape),
        ];
        for (code, expected) in bound {
            assert_eq!(key(code), Some(expected));
        }
        assert_eq!(key(KeyCode::KeyQ), None);
    }
}
