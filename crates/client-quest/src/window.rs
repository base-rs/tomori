// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a desktop window gives the contents it shows, and what they ask of it, so that the contents are tested without a display.
//!
//! The window itself, its winit event loop and its Vulkan surface, is `desktop_window.rs`, which holds only those calls: it turns each window event into an [`Input`], and draws through a [`Surface`]. Everything the window does with them is decided here and in the contents: the server-less window and the spectator's ([`crate::desktop`]), and the participant's ([`crate::desktop_participant`]). See `design/desktop.md`.

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
#[cfg(test)]
pub(crate) mod test {
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

    /// Run `test` with the shared test device, as a window would give it, at the test surface's size; skipped where there is no GPU.
    pub fn with_gpu(format: wgpu::TextureFormat, test: impl FnOnce(Gpu)) {
        // The contents log what they set up, such as the adapter, which tests exercise.
        crate::test_support::log_everything();
        tomori_environment_sky::test_gpu::with(|gpu| {
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
