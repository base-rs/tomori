// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop window as a participant: it connects to a Tomori server and presents arrival, the space and the hand menu as the headset does, seen from a head at standing height that the mouse turns, with the keyboard for the controller's buttons and the mouse for its ray. See `design/desktop.md`.

use crate::{
    connection::Connection,
    conversation::Conversation,
    desktop::{WindowGpu, acquire},
    graphics::Renderer,
    scene::desktop::{Button, Desktop, Direction, Key, KeyResult, TITLE},
    scene_graphics::{self, Panels},
};
use anyhow::Result;
use glam::{Mat4, Vec2};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

/// What each key does, shown at startup and by `--help`; the keys stand for the headset's controls in `design/interaction.md`.
pub const CONTROLS: &str = "Mouse: point (as the right controller's ray); left button: choose what is pointed at, or attract the swarm; right drag: look around.\n\
Enter: A. Backspace: B (hold for a second to leave the space). Space (hold): X, push-to-talk. Tab: Y, which in a space pins the hand menu open at its Space section.\n\
Arrow keys: left stick. [ and ]: right stick left and right. Page Up and Page Down: right stick up and down.\n\
V: mute spoken replies (left stick click). R: play the latest reply again, or stop it (right stick click).\n\
M: open or close the hand menu. T: show or hide the transcript on the right wrist; G: detach it into the world, or reattach it (the right grip). K: knock on the space you watch. + and -: admit or decline the first knock. P: pair this device as an owner, or cancel.\n\
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

/// Open a desktop window connected as `conversation` and `connection` describe, until it is closed.
pub fn run(conversation: Conversation, connection: Connection) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        participant: Some(Participant::new(conversation, connection)),
        state: None,
        error: None,
    };
    log::info!("{CONTROLS}");
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct App {
    /// Taken into `state` once the window exists.
    participant: Option<Participant>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let result = (|| {
            let attributes = Window::default_attributes()
                .with_title(TITLE)
                .with_inner_size(LogicalSize::new(1280.0, 800.0));
            let window = Arc::new(event_loop.create_window(attributes)?);
            let participant = self.participant.take().expect("the participant, once");
            pollster::block_on(State::new(window, participant))
        })();
        match result {
            Ok(state) => {
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else {
            return;
        };
        if id != state.window.id() {
            return;
        }
        let (p, desktop) = (&mut state.participant, &mut state.desktop);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::Focused(focused) => {
                if desktop.set_focused(focused, &mut p.conversation) {
                    p.connection.request_microphone();
                }
            }
            WindowEvent::Occluded(occluded) => {
                state.occluded = occluded;
                desktop.set_visible(!occluded, &mut p.conversation);
                if !occluded {
                    state.window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => desktop.cursor_left(),
            WindowEvent::CursorMoved { position, .. } => {
                desktop.cursor_moved(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } => {
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => return,
                };
                desktop.mouse(button, pressed == ElementState::Pressed);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let Some(key) = key(code) else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                if desktop.key(key, pressed, event.repeat, &mut p.conversation) == KeyResult::Quit {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                if state.occluded || state.size.width == 0 || state.size.height == 0 {
                    return;
                }
                if let Err(error) = state.frame(true) {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A hidden window draws nothing, so it would never receive a redraw; it still keeps its connection and speech going.
        let Some(state) = &mut self.state else {
            return;
        };
        if state.occluded || state.size.width == 0 || state.size.height == 0 {
            if let Err(error) = state.frame(false) {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(20),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// The conversation with the server, until the window exists.
struct Participant {
    conversation: Conversation,
    connection: Connection,
}

impl Participant {
    fn new(conversation: Conversation, connection: Connection) -> Self {
        Self {
            conversation,
            connection,
        }
    }
}

struct State {
    window: Arc<Window>,
    gpu: WindowGpu,
    renderer: Renderer,
    panels: Panels,
    size: PhysicalSize<u32>,
    occluded: bool,
    participant: Participant,
    /// The desktop's logic, whose first frame steps from when the window was ready.
    desktop: Desktop,
    first_frame: bool,
}

impl State {
    async fn new(window: Arc<Window>, mut participant: Participant) -> Result<Self> {
        let gpu = WindowGpu::new(window.clone()).await?;
        let size = window.inner_size();
        let samples = gpu.samples(gpu.config.format);
        let renderer = Renderer::with_samples(
            &gpu.device,
            &gpu.queue,
            gpu.config.format,
            gpu.config.width,
            gpu.config.height,
            samples,
        );
        let panels = Panels::new(&gpu.device, &renderer);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.config.width,
            gpu.config.height
        );
        let desktop = Desktop::new(&mut participant.conversation, Instant::now());
        Ok(Self {
            window,
            gpu,
            renderer,
            panels,
            size,
            occluded: false,
            participant,
            desktop,
            first_frame: true,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.gpu.config.width = size.width;
        self.gpu.config.height = size.height;
        self.gpu
            .surface
            .configure(&self.gpu.device, &self.gpu.config);
        self.renderer
            .resize(&self.gpu.device, size.width, size.height);
        self.window.request_redraw();
    }

    /// One frame of the desktop's logic, drawing it if `render`.
    fn frame(&mut self, render: bool) -> Result<()> {
        let now = Instant::now();
        let (p, desktop) = (&mut self.participant, &mut self.desktop);
        p.connection.poll(desktop.focused(), &mut p.conversation);
        let size = Vec2::new(
            self.size.width.max(1) as f32,
            self.size.height.max(1) as f32,
        );
        let drawn = desktop.frame(&mut p.conversation, p.connection.feedback(), now, size);
        if let Some(title) = &drawn.title {
            self.window.set_title(title);
        }
        scene_graphics::draw(
            &mut self.renderer,
            &self.gpu.device,
            &self.gpu.queue,
            &mut self.panels,
            &drawn.frame,
        );
        // Every source is heard from where it is, relative to this participant's head; see `design/spatial-audio.md`.
        p.connection
            .listen(drawn.frame.listener, drawn.frame.agent, &p.conversation);
        if let Some(presence) = drawn.frame.presence {
            p.connection.send_presence(
                presence.head,
                presence.left,
                presence.right,
                presence.hands,
            );
        }
        let view_projection = drawn.view_projection;
        if !render {
            return Ok(());
        }
        let Some(frame) = acquire(&self.gpu.surface, &self.gpu.device, &self.gpu.config)? else {
            return Ok(());
        };
        let target = frame.texture.create_view(&Default::default());
        self.renderer.render(
            &self.gpu.device,
            &self.gpu.queue,
            &[target],
            &[view_projection],
            Mat4::IDENTITY,
            &self.panels.list(Mat4::IDENTITY),
        );
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        if self.first_frame {
            log::info!("Presented first desktop frame");
            self.first_frame = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(key(code), Some(expected), "{code:?}");
        }
        assert_eq!(key(KeyCode::KeyQ), None);
    }
}
