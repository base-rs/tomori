// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a desktop window gives the contents it shows, and what they ask of it, so that the contents are tested without a display.
//!
//! The window itself, its winit event loop and its Vulkan surface, is `desktop_window.rs`, which holds only those calls: it turns each window event into an [`Input`], and draws through a [`Surface`]. Everything the window does with them is decided here and in the contents: the server-less window ([`crate::desktop`]), the participant's ([`crate::desktop_participant`]), and the spectator's, in `tomori-client-spectator`. See `design/desktop.md`.

use anyhow::Result;
use glam::Vec2;
use std::time::Instant;
use winit::{
    dpi::Size,
    event::{MouseButton, MouseScrollDelta},
    keyboard::KeyCode,
};

/// How a window opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub title: &'static str,
    pub size: Size,
    pub decorations: bool,
    pub min_size: Option<Size>,
}

/// A window event, as the window's contents see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// The window was asked to close.
    Close,
    /// The window's inner size, in physical pixels; either may be zero, as when the window is minimised.
    Resized {
        width: u32,
        height: u32,
    },
    Focused(bool),
    /// Whether the window is hidden, so that nothing it draws is seen.
    Occluded(bool),
    CursorLeft,
    /// The cursor's position, in physical pixels from the window's top left.
    CursorMoved(Vec2),
    Mouse {
        button: MouseButton,
        pressed: bool,
    },
    Wheel(MouseScrollDelta),
    /// A key, by its place on the keyboard; `repeat` when the key is held.
    Key {
        code: KeyCode,
        pressed: bool,
        repeat: bool,
    },
}

/// Whether the window stays open after an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// When the window next wakes, without an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// Only for the next event.
    Wait,
    /// At this time, or the next event if sooner.
    At(Instant),
}

/// The device a window's contents draw with, and the size and format of the images they draw into.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
}

impl Gpu {
    /// The samples per pixel to draw `format` with: 4, for multisample anti-aliasing, where the adapter supports it, or else 1.
    pub fn samples(&self, format: wgpu::TextureFormat) -> u32 {
        crate::graphics::supported_samples(&self.adapter, format, crate::graphics::SAMPLES)
    }
}

/// The format a window's surface draws in, of those it offers: the first sRGB format, so that colours are encoded as the renderer expects, or `None` to keep the surface's default.
pub fn preferred_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats.iter().copied().find(|format| format.is_srgb())
}

/// The window, as its contents draw into it.
pub trait Surface {
    /// The next image to draw into, or `None` when there is nothing to draw into this time, such as when the surface had to be configured again.
    fn acquire(&mut self) -> Result<Option<wgpu::TextureView>>;
    /// Show the image acquired last.
    fn present(&mut self);
    /// Configure the surface for a new size, neither of which is zero.
    fn resize(&mut self, width: u32, height: u32);
    fn set_title(&mut self, title: &str);
    /// Ask for [`Contents::redraw`] when the window is next ready to draw.
    fn request_redraw(&mut self);
}

/// What opens a window and shows contents in it: [`crate::desktop_window::Desktop`], the window system, or, in tests, `test::Headless`, the shared test device.
pub trait Shell {
    /// Open a window as `settings` describe, and show what `open` makes once the window has a device, until it is closed or fails.
    fn show<C: Contents>(
        &self,
        settings: Settings,
        open: impl FnOnce(Gpu) -> Result<C>,
    ) -> Result<()>;
}

/// What a window shows, driven by its events.
pub trait Contents {
    /// Handle one input.
    fn input(&mut self, input: Input, surface: &mut dyn Surface) -> Result<Flow>;
    /// Draw, when the window is ready to.
    fn redraw(&mut self, surface: &mut dyn Surface) -> Result<()>;
    /// Called when the window has handled every waiting event; say when to wake next.
    fn about_to_wait(&mut self, surface: &mut dyn Surface) -> Result<Wake>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_draws_in_srgb_where_its_surface_offers_it() {
        use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8UnormSrgb};
        assert_eq!(
            preferred_format(&[Bgra8Unorm, Bgra8UnormSrgb, Rgba8UnormSrgb]),
            Some(Bgra8UnormSrgb)
        );
        assert_eq!(preferred_format(&[Bgra8Unorm]), None);
    }
}

/// A window to test contents with: it draws into an offscreen texture and records what was asked of it.
/// A surface and a device for testing contents without a display, shared with the spectator's tests through the `testing` feature.
#[cfg(any(test, feature = "testing"))]
pub mod test {
    use super::*;

    pub struct TestSurface {
        pub texture: wgpu::Texture,
        /// Whether [`Surface::acquire`] has an image to give.
        pub available: bool,
        pub fail: bool,
        pub acquired: usize,
        pub presented: usize,
        pub sizes: Vec<(u32, u32)>,
        pub titles: Vec<String>,
        pub redraws: usize,
    }

    impl TestSurface {
        pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test surface"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 48,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            Self {
                texture,
                available: true,
                fail: false,
                acquired: 0,
                presented: 0,
                sizes: Vec::new(),
                titles: Vec::new(),
                redraws: 0,
            }
        }
    }

    impl TestSurface {
        /// What was drawn into the surface last, as rows of RGBA bytes from the top.
        pub fn pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
            let size = self.texture.size();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("test surface readback"),
                size: u64::from(size.width * size.height * 4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                self.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        // 64 pixels of 4 bytes, as wgpu requires a multiple of 256.
                        bytes_per_row: Some(size.width * 4),
                        rows_per_image: Some(size.height),
                    },
                },
                size,
            );
            queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("the device finishes the copy");
            buffer
                .slice(..)
                .get_mapped_range()
                .expect("the copy is mapped")
                .to_vec()
        }
    }

    impl Surface for TestSurface {
        fn acquire(&mut self) -> Result<Option<wgpu::TextureView>> {
            anyhow::ensure!(!self.fail, "the test surface is lost");
            if !self.available {
                return Ok(None);
            }
            self.acquired += 1;
            Ok(Some(self.texture.create_view(&Default::default())))
        }

        fn present(&mut self) {
            self.presented += 1;
        }

        fn resize(&mut self, width: u32, height: u32) {
            self.sizes.push((width, height));
        }

        fn set_title(&mut self, title: &str) {
            self.titles.push(title.to_owned());
        }

        fn request_redraw(&mut self) {
            self.redraws += 1;
        }
    }

    /// A [`Shell`] that shows contents on the shared test device, through a [`TestSurface`], so that what a runner shows is tested without a display: it draws them once, then gives them each of `inputs` and draws them again after each, then draws them every few milliseconds until `until` holds of the frame drawn last. Where there is no GPU, nothing is shown.
    ///
    /// # Panics
    ///
    /// If the contents fail to open, handle an input or draw, or if `until` does not hold within 10 seconds.
    pub struct Headless {
        pub inputs: Vec<Input>,
        pub until: Until,
    }

    /// Whether a test has seen what it waits for, given the frame drawn last, as [`TestSurface::pixels`] reads it.
    pub type Until = Box<dyn Fn(&[u8]) -> bool>;

    /// The keys' places, for [`press`].
    pub use winit::keyboard::KeyCode;

    /// Pressing the key at `code`'s place, once.
    pub fn press(code: KeyCode) -> Input {
        Input::Key {
            code,
            pressed: true,
            repeat: false,
        }
    }

    impl Headless {
        /// Give the contents `inputs`, then draw them until `until` holds of the frame drawn last.
        pub fn new(inputs: Vec<Input>, until: impl Fn(&[u8]) -> bool + 'static) -> Self {
            Self {
                inputs,
                until: Box::new(until),
            }
        }
    }

    impl Shell for Headless {
        fn show<C: Contents>(
            &self,
            _settings: Settings,
            open: impl FnOnce(Gpu) -> Result<C>,
        ) -> Result<()> {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            with_gpu(format, |gpu| {
                let mut surface = TestSurface::new(&gpu.device, format);
                let (device, queue) = (gpu.device.clone(), gpu.queue.clone());
                let mut contents = open(gpu).expect("the contents open");
                contents.redraw(&mut surface).expect("the contents draw");
                for input in &self.inputs {
                    contents
                        .input(*input, &mut surface)
                        .expect("the contents handle the input");
                    contents.redraw(&mut surface).expect("the contents draw");
                }
                let deadline = Instant::now() + std::time::Duration::from_secs(10);
                while !(self.until)(&surface.pixels(&device, &queue)) {
                    assert!(
                        Instant::now() < deadline,
                        "what the test waits for is not shown within 10 seconds"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    contents.redraw(&mut surface).expect("the contents draw");
                }
            });
            Ok(())
        }
    }

    /// Run `test` with the shared test device, as a window would give it, at the test surface's size; skipped where there is no GPU.
    pub fn with_gpu(format: wgpu::TextureFormat, test: impl FnOnce(Gpu)) {
        // The contents log what they set up, such as the adapter, which tests exercise.
        tomori_scene::test_support::log_everything();
        tomori_render::test_gpu::with(|gpu| {
            test(Gpu {
                adapter: gpu.adapter,
                device: gpu.device,
                queue: gpu.queue,
                format,
                width: 64,
                height: 48,
            })
        });
    }
}
