// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop window's calls into the window system: winit's event loop and window, and the window's Vulkan or Metal surface. It needs a display, so it is left out of the coverage gate; see the `[[package.metadata.bake.test.coverage.exclude]]` entries in `Cargo.toml`.
//!
//! Only those calls belong here. Each window event becomes a [`window::Input`] and each frame is drawn through a [`window::Surface`]; what the window does with them is decided by its [`window::Contents`], in covered modules: the server-less window, the participant's, or what [`show`] is given, such as the spectator's. See `design/desktop.md`.

use crate::{
    connection::Connection,
    conversation::Conversation,
    desktop::Demo,
    desktop_participant::Participant,
    window::{self, Contents, Flow, Gpu, Input, Settings, Wake},
};
use anyhow::{Context, Result};
use glam::Vec2;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::PhysicalKey,
    window::{Window, WindowId},
};

/// The server-less window, until it is closed; see [`Demo`].
pub fn run_demo() -> Result<()> {
    run(Demo::settings(), |gpu| Ok(Demo::new(gpu)))
}

/// The participant's window, connected as `conversation` and `connection` describe, until it is closed; see [`Participant`].
pub fn run_participant(conversation: Conversation, connection: Connection) -> Result<()> {
    run(Participant::settings(), |gpu| {
        Ok(Participant::new(gpu, conversation, connection))
    })
}

/// Open a window as `settings` describe and show what `open` makes of the window's device and `with`, until it is closed or fails, as the spectator's window does.
pub fn show<C: Contents, A>(
    settings: Settings,
    open: fn(Gpu, A) -> Result<C>,
    with: A,
) -> Result<()> {
    run(settings, move |gpu| open(gpu, with))
}

/// Open a window as `settings` describe and show what `start` makes once the window and its device exist, until it is closed or fails.
fn run<C: Contents>(settings: Settings, start: impl FnOnce(Gpu) -> Result<C>) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        settings,
        start: Some(start),
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

struct App<C, S> {
    settings: Settings,
    start: Option<S>,
    state: Option<(C, WindowSurface)>,
    error: Option<anyhow::Error>,
}

impl<C, S> App<C, S> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, result: Result<()>) {
        if let Err(error) = result {
            self.error = Some(error);
            event_loop.exit();
        }
    }
}

impl<C: Contents, S: FnOnce(Gpu) -> Result<C>> ApplicationHandler for App<C, S> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some(start) = self.start.take() else {
            return;
        };
        let result = (|| {
            let mut attributes = Window::default_attributes()
                .with_title(self.settings.title)
                .with_decorations(self.settings.decorations)
                .with_inner_size(self.settings.size);
            if let Some(size) = self.settings.min_size {
                attributes = attributes.with_min_inner_size(size);
            }
            let window = Arc::new(event_loop.create_window(attributes)?);
            let (surface, gpu) = pollster::block_on(WindowSurface::new(window))?;
            let contents = start(gpu)?;
            surface.window.request_redraw();
            self.state = Some((contents, surface));
            Ok(())
        })();
        self.fail(event_loop, result);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some((contents, surface)) = &mut self.state else {
            return;
        };
        if id != surface.window.id() {
            return;
        }
        let input = match event {
            WindowEvent::RedrawRequested => {
                let result = contents.redraw(surface);
                return self.fail(event_loop, result);
            }
            WindowEvent::CloseRequested => Input::Close,
            WindowEvent::Resized(size) => Input::Resized {
                width: size.width,
                height: size.height,
            },
            WindowEvent::Focused(focused) => Input::Focused(focused),
            WindowEvent::Occluded(occluded) => Input::Occluded(occluded),
            WindowEvent::CursorLeft { .. } => Input::CursorLeft,
            WindowEvent::CursorMoved { position, .. } => {
                Input::CursorMoved(Vec2::new(position.x as f32, position.y as f32))
            }
            WindowEvent::MouseInput { state, button, .. } => Input::Mouse {
                button,
                pressed: state == ElementState::Pressed,
            },
            WindowEvent::MouseWheel { delta, .. } => Input::Wheel(delta),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                Input::Key {
                    code,
                    pressed: event.state == ElementState::Pressed,
                    repeat: event.repeat,
                }
            }
            _ => return,
        };
        match contents.input(input, surface) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Exit) => event_loop.exit(),
            Err(error) => self.fail(event_loop, Err(error)),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some((contents, surface)) = &mut self.state else {
            return;
        };
        match contents.about_to_wait(surface) {
            Ok(Wake::Wait) => event_loop.set_control_flow(ControlFlow::Wait),
            Ok(Wake::At(time)) => event_loop.set_control_flow(ControlFlow::WaitUntil(time)),
            Err(error) => self.fail(event_loop, Err(error)),
        }
    }
}

/// A window's Vulkan or Metal surface, configured for sRGB output with vsync, and the frame acquired from it.
/// The backends a desktop window draws with: Vulkan, and Metal on macOS, which has no Vulkan driver of its own.
const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN.union(wgpu::Backends::METAL);

struct WindowSurface {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frame: Option<wgpu::SurfaceTexture>,
}

impl WindowSurface {
    async fn new(window: Arc<Window>) -> Result<(Self, Gpu)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let size = window.inner_size();
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("finding a desktop Vulkan or Metal adapter")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("desktop device"),
                ..Default::default()
            })
            .await?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface has no supported configuration")?;
        let formats = surface.get_capabilities(&adapter).formats;
        config.format = window::preferred_format(&formats).unwrap_or(config.format);
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let gpu = Gpu {
            adapter,
            device: device.clone(),
            queue: queue.clone(),
            format: config.format,
            width: config.width,
            height: config.height,
        };
        let surface = Self {
            window,
            surface,
            device,
            queue,
            config,
            frame: None,
        };
        Ok((surface, gpu))
    }
}

impl window::Surface for WindowSurface {
    fn acquire(&mut self) -> Result<Option<wgpu::TextureView>> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(None);
            }
            other => anyhow::bail!("Could not acquire desktop frame: {other:?}"),
        };
        let view = frame.texture.create_view(&Default::default());
        self.frame = Some(frame);
        Ok(Some(view))
    }

    fn present(&mut self) {
        if let Some(frame) = self.frame.take() {
            self.window.pre_present_notify();
            self.queue.present(frame);
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn set_title(&mut self, title: &str) {
        self.window.set_title(title);
    }

    fn request_redraw(&mut self) {
        self.window.request_redraw();
    }
}
