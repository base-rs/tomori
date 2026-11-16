// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop window's calls into the window system: winit's event loop and window, and the window's Vulkan or Metal surface. It needs a display, so it is left out of the coverage gate; see the `[[package.metadata.bake.test.coverage.exclude]]` entries in `Cargo.toml`.
//!
//! Only those calls belong here. Each window event becomes a [`window::Input`] and each frame is drawn through a [`window::Surface`]; what the window does with them is decided by its [`window::Contents`], in covered modules: the server-less window, the participant's, or what [`show`] is given, such as the spectator's. The desktop's modes open their windows through [`Desktop`], so that their tests can show the same contents through `window::test::Headless` instead. See `design/desktop.md`.
use crate::window::{self, Contents, Flow, Gpu, Input, Settings, Shell, Wake};
use anyhow::{Context, Result};
use glam::Vec2;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::PhysicalKey,
    window::{Window, WindowId},
};

/// The window system's windows, as the desktop's modes show them.
pub struct Desktop;

impl Shell for Desktop {
    fn show<C: Contents>(
        &self,
        settings: Settings,
        open: impl FnOnce(Gpu) -> Result<C>,
    ) -> Result<()> {
        run(settings, open)
    }
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
                let result = redraw(event_loop, contents, surface);
                return self.fail(event_loop, result);
            }
            WindowEvent::CloseRequested => Input::Close,
            WindowEvent::Resized(size) => match surface.give_size(size) {
                Some(input) => input,
                None => return,
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
        let result = give(event_loop, contents, surface, input);
        self.fail(event_loop, result);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some((contents, surface)) = &mut self.state else {
            return;
        };
        #[cfg(target_os = "macos")]
        if fit_content_to_full_screen(&surface.window) {
            // The window system reports the new size only with the next event, which an idle window may wait long for.
            let result = match surface.give_size(surface.window.inner_size()) {
                Some(input) => give(event_loop, contents, surface, input),
                None => Ok(()),
            };
            surface.window.request_redraw();
            if result.is_err() {
                return self.fail(event_loop, result);
            }
        }
        match contents.about_to_wait(surface) {
            Ok(Wake::Wait) => event_loop.set_control_flow(ControlFlow::Wait),
            Ok(Wake::At(time)) => event_loop.set_control_flow(ControlFlow::WaitUntil(time)),
            Err(error) => self.fail(event_loop, Err(error)),
        }
    }
}

/// Give `input` to `contents`, ending the event loop when they ask.
fn give<C: Contents>(
    event_loop: &ActiveEventLoop,
    contents: &mut C,
    surface: &mut WindowSurface,
    input: Input,
) -> Result<()> {
    if contents.input(input, surface)? == Flow::Exit {
        event_loop.exit();
    }
    Ok(())
}

/// Draw `contents`, at the window's size now: a size the window system has not reported yet, as after the window's style changed, is given to them first, so that no frame is drawn at a size the window no longer has.
fn redraw<C: Contents>(
    event_loop: &ActiveEventLoop,
    contents: &mut C,
    surface: &mut WindowSurface,
) -> Result<()> {
    if let Some(input) = surface.give_size(surface.window.inner_size()) {
        give(event_loop, contents, surface, input)?;
    }
    contents.redraw(surface)
}

/// Extend `window`'s content under its title bar while it is full screen, and only then, as [`window::full_size_content`] decides, so that the menu bar and title bar sliding down over it do not resize it in every frame of the slide, returning whether it changed. Checked each time the window has handled its events, since winit reports no change of the platform's full screen. Its view is made the first responder again after each change, entering full screen and leaving it, as winit does whenever it changes a window's style, since AppKit otherwise stops sending it keys until the window is clicked.
#[cfg(target_os = "macos")]
fn fit_content_to_full_screen(window: &Window) -> bool {
    use objc2_app_kit::{NSView, NSWindowStyleMask};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // SAFETY: winit's AppKit handle is its window's content view, which lives as long as the window, and this runs on the main thread, in winit's event loop.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(ns_window) = view.window() else {
        return false;
    };
    let mask = ns_window.styleMask();
    let Some(full_size) = window::full_size_content(
        mask.contains(NSWindowStyleMask::FullScreen),
        mask.contains(NSWindowStyleMask::FullSizeContentView),
    ) else {
        return false;
    };
    ns_window.setStyleMask(if full_size {
        mask | NSWindowStyleMask::FullSizeContentView
    } else {
        mask & !NSWindowStyleMask::FullSizeContentView
    });
    let _ = ns_window.makeFirstResponder(Some(&**view));
    true
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
    /// The size last given to the window's contents, in physical pixels.
    given: (u32, u32),
}

impl WindowSurface {
    /// The resize to give the contents, now that the window's size is `size`, if they do not have it yet; see [`window::resize`].
    fn give_size(&mut self, size: PhysicalSize<u32>) -> Option<Input> {
        let size = (size.width, size.height);
        let input = window::resize(self.given, size);
        self.given = size;
        input
    }
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
                // The GPU's timestamps, where it has them, for the frame log, and ASTC textures where it samples them;
                // see `design/rendering.md`.
                required_features: adapter.features()
                    & (tomori_render::profiler::FEATURES | tomori_render::textures::FEATURES),
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
            given: (config.width, config.height),
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
