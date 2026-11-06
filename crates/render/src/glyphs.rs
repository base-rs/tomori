// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The glyph atlas on the GPU: a single-channel texture holding `tomori-text`'s atlas, which panels, labels and signs sample through one bind group. [`Glyphs::sync`] uploads the rows the atlas has gained since it last ran, from the shelf that was open then, and the whole atlas when it has grown, which also replaces the bind group. Glyphs never move in the atlas, so meshes built from it before stay valid.
use std::sync::{Mutex, PoisonError};
use tomori_text::atlas::{self, WIDTH};

/// How a thread that draws frames limits the time it spends drawing new glyphs into the atlas each frame, which [`Glyphs::sync`] ends.
pub use tomori_text::atlas::{Budget, budget};

/// How the texture is used: sampled, and written as the atlas gains glyphs; tests read it back too.
#[cfg(not(test))]
const USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST);
#[cfg(test)]
const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::COPY_DST)
    .union(wgpu::TextureUsages::COPY_SRC);

/// The atlas's texture and the bind group that draws from it.
pub struct Glyphs {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    state: Mutex<State>,
}

struct State {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    height: u32,
    /// The atlas's revision when last uploaded, and the top of its open shelf then, from which rows may have changed since.
    revision: u64,
    open: u32,
}

impl Glyphs {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph atlas layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Nothing is uploaded until the first sync, which uploads the whole atlas, as its revision differs.
        let (texture, binding) = Self::texture(device, &layout, &sampler, 1);
        Self {
            layout,
            sampler,
            state: Mutex::new(State {
                texture,
                binding,
                height: 1,
                revision: u64::MAX,
                open: 0,
            }),
        }
    }

    fn texture(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        height: u32,
    ) -> (wgpu::Texture, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: USAGE,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph atlas binding"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (texture, binding)
    }

    /// The layout of [`Glyphs::binding`], for the pipelines that draw text.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Upload what the process's atlas has gained since the last sync, before a frame that draws meshes built from it, and end this thread's frame of drawing glyphs into it, logging what that cost, so that the headset's log shows the time spent; see [`atlas::frame`].
    pub fn sync(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let frame = atlas::frame();
        if frame.drawn > 0 || frame.waiting > 0 {
            let level = if frame.waiting > 0 || frame.time.as_micros() >= 1000 {
                log::Level::Info
            } else {
                log::Level::Debug
            };
            log::log!(
                level,
                "Drew {} glyphs into the atlas in {:.2} ms this frame; {} wait for the next.",
                frame.drawn,
                frame.time.as_secs_f64() * 1000.0,
                frame.waiting
            );
        }
        // The texture's lock is always taken before the atlas's.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        atlas::with(|atlas| self.upload(device, queue, &mut state, atlas));
    }

    /// Upload what `atlas` has gained since `state` was last uploaded.
    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        state: &mut State,
        atlas: &atlas::Atlas,
    ) {
        if atlas.revision() == state.revision {
            return;
        }
        let from = if atlas.height() == state.height {
            state.open
        } else {
            let stats = atlas.stats();
            log::info!(
                "The glyph atlas holds {} glyphs in {WIDTH} × {} pixels ({} KiB), drawn in {:.1} ms in all.",
                stats.glyphs,
                stats.height,
                stats.bytes / 1024,
                stats.rasterising.as_secs_f64() * 1000.0
            );
            let (texture, binding) =
                Self::texture(device, &self.layout, &self.sampler, atlas.height());
            state.texture = texture;
            state.binding = binding;
            state.height = atlas.height();
            0
        };
        let rows = atlas.filled() - from;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &state.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: from,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels()[(from * WIDTH) as usize..(atlas.filled() * WIDTH) as usize],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        state.revision = atlas.revision();
        state.open = atlas.open();
    }

    /// The bind group that draws from the atlas as last synced.
    pub fn binding(&self) -> wgpu::BindGroup {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .binding
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_panel::{Canvas, Panel, Pipeline};
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Style, glyph, placed, shown};

    /// Screen pixels per font pixel: about what the headset shows of the hand menu and the wrist at arm's length, 2 to 3.
    const SCALE: usize = 3;
    /// Font pixels around the text.
    const PAD: usize = 4;

    /// Latin with accents, Japanese, and code with symbols, each with an empty row after it, so that no row's accents or descenders reach another's.
    const SAMPLES: [&str; 4] = [
        "José Łódź Dvořák Nguyễn Œuvre «ça» ½ — “ok”",
        "日本語のテキスト、こんにちは。カタカナ",
        "fn main() { let x = [1, 2]; } // ✓ → ↵ ●",
        "?",
    ];

    struct Drawn {
        /// Coverage, from 0 to 1, a value per screen pixel, row by row.
        coverage: Vec<f32>,
        width: usize,
        height: usize,
        /// Screen pixels per font pixel.
        scale: f32,
        canvas: Canvas,
    }

    impl Drawn {
        fn at(&self, x: usize, y: usize) -> f32 {
            self.coverage[y * self.width + x]
        }

        /// The ink in a rectangle of font pixels, in screen pixels, clamped to the image.
        fn ink(&self, x: f32, y: f32, width: f32, height: f32) -> f32 {
            let scale = self.scale;
            let left = ((x * scale).floor().max(0.0) as usize).min(self.width);
            let right = (((x + width) * scale).ceil().max(0.0) as usize).min(self.width);
            let top = ((y * scale).floor().max(0.0) as usize).min(self.height);
            let bottom = (((y + height) * scale).ceil().max(0.0) as usize).min(self.height);
            (top..bottom)
                .flat_map(|y| (left..right).map(move |x| (x, y)))
                .map(|(x, y)| self.at(x, y))
                .sum()
        }

        /// The ink in a rectangle of font pixels, each screen pixel weighted by how much of it the rectangle covers.
        fn ink_within(&self, x: f32, y: f32, width: f32, height: f32) -> f32 {
            let scale = self.scale;
            let (left, right) = (x * scale, (x + width) * scale);
            let (top, bottom) = (y * scale, (y + height) * scale);
            let overlap = |low: f32, high: f32, pixel: usize| {
                (high.min(pixel as f32 + 1.0) - low.max(pixel as f32)).max(0.0)
            };
            let columns = (left.floor().max(0.0) as usize)..(right.ceil() as usize).min(self.width);
            let rows = (top.floor().max(0.0) as usize)..(bottom.ceil() as usize).min(self.height);
            rows.flat_map(|y| columns.clone().map(move |x| (x, y)))
                .map(|(x, y)| self.at(x, y) * overlap(left, right, x) * overlap(top, bottom, y))
                .sum()
        }

        /// The quads of row `row`'s glyphs, in font pixels from the image's top-left, with their characters and the cells they take.
        fn glyphs(&self, row: usize) -> Vec<(char, usize, crate::ui_text::Quad)> {
            let cells = &self.canvas.rows[row].cells;
            placed(cells)
                .filter_map(|(column, ch, cells, style)| {
                    glyph(
                        ch,
                        cells,
                        (PAD + column * CELL_WIDTH) as f32,
                        (PAD + row * CELL_HEIGHT) as f32,
                        CELL_HEIGHT as f32,
                        style.bold,
                    )
                    .map(|quad| (ch, cells, quad))
                })
                .collect()
        }

        fn image(&self) -> image::RgbaImage {
            image::RgbaImage::from_fn(self.width as u32, self.height as u32, |x, y| {
                let value = (self.at(x as usize, y as usize) * 255.0).round() as u8;
                image::Rgba([value, value, value, 255])
            })
        }
    }

    /// `lines` drawn white on black by the panel pipeline, a font pixel to [`SCALE`] screen pixels, from a fresh texture of the shared atlas.
    fn draw(gpu: &crate::test_gpu::Gpu, lines: &[(&str, bool)]) -> Drawn {
        draw_at(gpu, lines, SCALE as f32, [0.0; 2])
    }

    /// `lines` drawn as [`draw`] draws them, a font pixel to `scale` screen pixels, moved right and down by `shift` screen pixels.
    fn draw_at(
        gpu: &crate::test_gpu::Gpu,
        lines: &[(&str, bool)],
        scale: f32,
        shift: [f32; 2],
    ) -> Drawn {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let columns = 46;
        let mut canvas = Canvas::new(columns, 1.0);
        canvas.background = [0.0; 4];
        canvas.padding = (PAD, PAD);
        for (line, bold) in lines {
            let style = if *bold {
                Style::bold([1.0; 4])
            } else {
                Style::plain([1.0; 4])
            };
            canvas.rows.push(Row::text(line, style, columns));
            canvas.rows.push(Row::default());
        }
        let (columns, rows) = canvas.size_in_pixels();
        let (width, height) = (
            (columns as f32 * scale).ceil() as usize,
            (rows as f32 * scale).ceil() as usize,
        );
        let glyphs = Glyphs::new(&gpu.device);
        let pipeline = Pipeline::new(&gpu.device, format, &glyphs, 1);
        let mut panel = Panel::new(&gpu.device, &pipeline, 24_000);
        panel.set(&gpu.queue, &canvas);
        // The canvas, in metres about its centre, with its top-left corner `shift` screen pixels from the target's.
        let (w, h) = canvas.size();
        let metres = w / columns as f32 / scale;
        let (left, top) = (-w / 2.0 - shift[0] * metres, h / 2.0 + shift[1] * metres);
        let transform = glam::Mat4::orthographic_rh(
            left,
            left + width as f32 * metres,
            top - height as f32 * metres,
            top,
            -1.0,
            1.0,
        );
        panel.prepare(&gpu.queue, 0, transform);
        glyphs.sync(&gpu.device, &gpu.queue);
        let target = |format, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("text test"),
                size: wgpu::Extent3d {
                    width: width as u32,
                    height: height as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let texture = target(
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = target(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let (view, depth) = (
            texture.create_view(&Default::default()),
            depth.create_view(&Default::default()),
        );
        let row_bytes = (width * 4).next_multiple_of(256);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text test readback"),
            size: (row_bytes * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pipeline.set(&mut pass, &glyphs.binding());
            panel.draw(&mut pass, 0);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes as u32),
                    rows_per_image: Some(height as u32),
                },
            },
            texture.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        let coverage = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| f32::from(data[y * row_bytes + x * 4]) / 255.0)
            .collect();
        Drawn {
            coverage,
            width,
            height,
            scale,
            canvas,
        }
    }

    /// The ink `quad` should draw, in screen pixels: how much of each of its tile's pixels is inside the edge, from its distance to it, scaled to the screen.
    fn expected(quad: &crate::ui_text::Quad) -> f32 {
        let [x, y, width, height] = quad.tile;
        let inside: f32 = atlas::with(|atlas| {
            (y as usize..(y + height) as usize)
                .flat_map(|row| {
                    let start = row * WIDTH as usize + x as usize;
                    atlas.pixels()[start..start + width as usize].to_vec()
                })
                .map(|value| {
                    let distance = (f32::from(value) / 255.0 - 0.5) * 2.0 * atlas::SPREAD;
                    (distance + 0.5).clamp(0.0, 1.0)
                })
                .sum()
        });
        let across = quad.width * SCALE as f32 / width;
        let down = quad.height * SCALE as f32 / height;
        inside * across * down
    }

    #[test]
    fn latin_japanese_and_code_draw_every_glyph_antialiased_and_in_place() {
        crate::test_gpu::with(|gpu| {
            let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let lines: Vec<_> = SAMPLES.iter().map(|line| (*line, false)).collect();
            let drawn = draw(&gpu, &lines);
            crate::test_support::preview("TOMORI_TEXT_PREVIEW", &drawn.image());
            let replacement = drawn.glyphs(6)[0].2;
            for (row, sample) in SAMPLES[..SAMPLES.len() - 1].iter().enumerate() {
                let glyphs = drawn.glyphs(row * 2);
                // Every character is drawn: none was replaced, and each draws as much ink as its tile holds, within a fifth.
                let shown: String = drawn.canvas.rows[row * 2].plain();
                assert_eq!(shown, *sample);
                for (ch, _, quad) in &glyphs {
                    assert_eq!(*ch, crate::ui_text::shown(*ch));
                    assert_ne!(quad.tile, replacement.tile, "{ch} is the replacement");
                    // Within the glyph's outline and half a font pixel of its edge, where a neighbour's ink never is.
                    let inset = quad.width * atlas::SPREAD / quad.tile[2] - 0.5;
                    let ink = drawn.ink(
                        quad.x + inset,
                        quad.y + inset,
                        quad.width - 2.0 * inset,
                        quad.height - 2.0 * inset,
                    );
                    let expected = expected(quad);
                    assert!(
                        (ink - expected).abs() <= expected * 0.2 + 4.0,
                        "{ch}: {ink} of {expected}"
                    );
                    assert!(ink > 4.0, "{ch} draws nothing");
                }
                // Nothing is drawn where there is no glyph: between the row and the next.
                let gap = drawn.ink(
                    0.0,
                    (PAD + (row * 2 + 1) * CELL_HEIGHT) as f32 + 3.0,
                    drawn.width as f32,
                    6.0,
                );
                assert_eq!(gap, 0.0, "row {row}");
            }
            // The edges are antialiased: many pixels are partly covered.
            let partial = drawn
                .coverage
                .iter()
                .filter(|value| (0.15..0.85).contains(*value))
                .count();
            let full = drawn
                .coverage
                .iter()
                .filter(|value| **value >= 0.85)
                .count();
            assert!(partial * 3 > full, "{partial} partly covered, {full} fully");
            // Latin letters without descenders sit on one baseline.
            let bottom = |quad: &crate::ui_text::Quad| {
                let x = (quad.x * SCALE as f32) as usize;
                let width = (quad.width * SCALE as f32) as usize;
                let top = (quad.y * SCALE as f32) as usize;
                let height = (quad.height * SCALE as f32) as usize;
                (top..top + height)
                    .rev()
                    .find(|y| (x..x + width).any(|x| drawn.at(x, *y) > 0.5))
                    .unwrap()
            };
            let baselines: Vec<usize> = drawn
                .glyphs(0)
                .iter()
                .filter(|(ch, _, _)| "osdváŁzoNuyễnŒve".contains(*ch) && *ch != 'y')
                .map(|(_, _, quad)| bottom(quad))
                .collect();
            let (low, high) = (
                baselines.iter().min().unwrap(),
                baselines.iter().max().unwrap(),
            );
            assert!(high - low <= 1, "{baselines:?}");
            // A descender reaches below it, and stays within its row.
            let y = drawn
                .glyphs(0)
                .into_iter()
                .find(|(ch, _, _)| *ch == 'y')
                .unwrap()
                .2;
            assert!(bottom(&y) > high + SCALE);
            assert!(bottom(&y) < (PAD + CELL_HEIGHT) * SCALE);
            // Japanese takes two cells, each character's ink centred across them, except punctuation, which sits where it is written, such as "、" low and to the left.
            let mut cell = PAD as f32 - 2.0 * CELL_WIDTH as f32;
            for (ch, cells, quad) in drawn.glyphs(2) {
                assert_eq!(cells, 2, "{ch}");
                cell += 2.0 * CELL_WIDTH as f32;
                if ('\u{3000}'..='\u{303f}').contains(&ch) {
                    continue;
                }
                let lit: Vec<usize> = ((quad.x * SCALE as f32) as usize
                    ..((quad.x + quad.width) * SCALE as f32) as usize)
                    .filter(|x| {
                        ((quad.y * SCALE as f32) as usize
                            ..((quad.y + quad.height) * SCALE as f32) as usize)
                            .any(|y| drawn.at(*x, y) > 0.5)
                    })
                    .collect();
                let centre = (lit[0] + lit[lit.len() - 1]) as f32 / 2.0 / SCALE as f32;
                let middle = cell + CELL_WIDTH as f32;
                assert!(
                    (centre - middle).abs() < 2.0,
                    "{ch}: ink about {centre}, cells about {middle}"
                );
            }
            // Code's symbols stay within their own cells, so that columns line up.
            for (ch, _, quad) in drawn.glyphs(4) {
                let column =
                    ((quad.x + quad.width / 2.0 - PAD as f32) / CELL_WIDTH as f32) as usize;
                let left = (PAD + column * CELL_WIDTH) as f32;
                let outside = drawn.ink(quad.x, quad.y, left - quad.x, quad.height)
                    + drawn.ink(
                        left + CELL_WIDTH as f32,
                        quad.y,
                        quad.x + quad.width - left - CELL_WIDTH as f32,
                        quad.height,
                    );
                assert!(outside < 2.0, "{ch}: {outside} outside its cell");
            }
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }

    /// How `text` draws as it moves across the screen by fractions of a pixel, a font pixel to `scale` screen pixels: the largest change in any glyph's ink, as a fraction of its mean; the mean of those changes; and its mean ink in all, in font pixels.
    fn shimmer(gpu: &crate::test_gpu::Gpu, text: &str, scale: f32) -> (f32, f32, f32) {
        let shifts: Vec<[f32; 2]> = (0..8)
            .map(|step| [step as f32 / 8.0, (step * 3 % 8) as f32 / 8.0])
            .collect();
        let draws: Vec<Drawn> = shifts
            .iter()
            .map(|shift| draw_at(gpu, &[(text, false)], scale, *shift))
            .collect();
        let cells = draws[0].canvas.rows[0].cells.clone();
        let inks: Vec<Vec<f32>> = placed(&cells)
            .map(|(column, _, cells, _)| {
                draws
                    .iter()
                    .zip(&shifts)
                    .map(|(drawn, shift)| {
                        drawn.ink_within(
                            (PAD + column * CELL_WIDTH) as f32 + shift[0] / scale,
                            PAD as f32 + shift[1] / scale,
                            (cells * CELL_WIDTH) as f32,
                            CELL_HEIGHT as f32,
                        ) / (scale * scale)
                    })
                    .collect()
            })
            .collect();
        let changes: Vec<f32> = inks
            .iter()
            .map(|inks| {
                let mean = inks.iter().sum::<f32>() / inks.len() as f32;
                let (low, high) = inks.iter().fold((f32::MAX, 0.0_f32), |(low, high), ink| {
                    (low.min(*ink), high.max(*ink))
                });
                (high - low) / mean
            })
            .collect();
        let worst = changes.iter().copied().fold(0.0, f32::max);
        let mean = changes.iter().sum::<f32>() / changes.len() as f32;
        let ink = inks.iter().flatten().sum::<f32>() / shifts.len() as f32;
        (worst, mean, ink)
    }

    #[test]
    fn distant_text_keeps_its_ink_as_it_moves_rather_than_shimmering() {
        crate::test_gpu::with(|gpu| {
            let text = "COMPLETED Readable task 完了しました";
            let (_, near, ink) = shimmer(&gpu, text, SCALE as f32);
            assert!(near < 0.1, "{near}");
            // About what a worker's sign shows from three to five metres away: a font pixel to a half or a third of a screen pixel, so that a pixel spans six to nine texels of the atlas. Sampled once, as it was, each glyph's ink changed by as much as it had as the head moved, and by twice as much at worst, so that "COMPLETED" read "COMBLETED"; averaged across the pixel, it holds, and the text draws about as much ink as it does up close.
            for scale in [0.5, 0.33] {
                let (worst, mean, far) = shimmer(&gpu, text, scale);
                assert!(worst < 0.5 && mean < 0.2, "{scale}: {worst}, {mean}");
                assert!((far - ink).abs() < ink * 0.3, "{scale}: {far} of {ink}");
            }
        });
    }

    #[test]
    fn bold_draws_more_ink_and_the_atlas_grows_on_the_gpu_too() {
        crate::test_gpu::with(|gpu| {
            let text = "Bold and plain 太字";
            let plain = draw(&gpu, &[(text, false)]);
            let bold = draw(&gpu, &[(text, true)]);
            let ink = |drawn: &Drawn| drawn.coverage.iter().sum::<f32>();
            assert!(ink(&bold) > ink(&plain) * 1.1);
            // Enough new glyphs to grow the atlas: the texture is made again at its new height and drawn from.
            let start = atlas::with(|atlas| atlas.height());
            let kanji: String = ('\u{4e00}'..'\u{9fa0}')
                .filter(|ch| shown(*ch) == *ch)
                .take(1200)
                .collect();
            let glyphs = Glyphs::new(&gpu.device);
            for chunk in kanji.chars().collect::<Vec<_>>().chunks(40) {
                for ch in chunk {
                    atlas::glyph(*ch);
                }
                glyphs.sync(&gpu.device, &gpu.queue);
            }
            let grown = atlas::with(|atlas| atlas.height());
            assert!(grown > start, "{grown}");
            assert_eq!(glyphs.state.lock().unwrap().height, grown);
            // A sync with nothing new does nothing.
            let revision = glyphs.state.lock().unwrap().revision;
            glyphs.sync(&gpu.device, &gpu.queue);
            assert_eq!(glyphs.state.lock().unwrap().revision, revision);
            let drawn = draw(&gpu, &[(&kanji[kanji.len() - 30..], false)]);
            assert!(ink(&drawn) > 100.0);
        });
    }

    /// The texture's first `rows` rows, read back from the GPU.
    fn read(gpu: &crate::test_gpu::Gpu, glyphs: &Glyphs, rows: u32) -> Vec<u8> {
        let state = glyphs.state.lock().unwrap();
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph atlas readback"),
            size: u64::from(WIDTH * rows),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            state.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH),
                    rows_per_image: Some(rows),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        buffer.slice(..).get_mapped_range().unwrap().to_vec()
    }

    #[test]
    fn a_sync_uploads_every_row_gained_since_the_last_even_on_a_shelf_that_has_closed() {
        crate::test_gpu::with(|gpu| {
            let glyphs = Glyphs::new(&gpu.device);
            let upload = |atlas: &atlas::Atlas| {
                let mut state = glyphs.state.lock().unwrap();
                glyphs.upload(&gpu.device, &gpu.queue, &mut state, atlas);
            };
            // An atlas of this test's own, which no other test draws into.
            let mut atlas = atlas::Atlas::new(atlas::START_HEIGHT);
            let mut kanji = ('\u{4e00}'..'\u{9fa0}').filter(|ch| shown(*ch) == *ch);
            for ch in kanji.by_ref().take(3) {
                atlas.glyph(ch);
            }
            upload(&atlas);
            let (height, open) = (atlas.height(), atlas.open());
            assert_eq!(
                read(&gpu, &glyphs, atlas.filled()),
                atlas.pixels()[..(atlas.filled() * WIDTH) as usize]
            );
            // More, until that shelf is closed and another opened below it, without the atlas growing, which would upload it all.
            while atlas.open() == open {
                atlas.glyph(kanji.next().unwrap());
            }
            atlas.glyph(kanji.next().unwrap());
            assert_eq!(atlas.height(), height);
            upload(&atlas);
            // The closed shelf's rows, filled since the first upload, are on the GPU, as well as the open shelf's.
            let rows = atlas.filled();
            assert!(rows > atlas.open());
            assert_eq!(
                read(&gpu, &glyphs, rows),
                atlas.pixels()[..(rows * WIDTH) as usize]
            );
        });
    }

    #[test]
    fn a_sync_ends_the_frame_of_drawing_glyphs() {
        crate::test_gpu::with(|gpu| {
            crate::test_support::log_everything();
            let glyphs = Glyphs::new(&gpu.device);
            // A frame that drew nothing, one that drew a little, and one that ran out of budget, each logged by how much it cost.
            glyphs.sync(&gpu.device, &gpu.queue);
            atlas::glyph('x');
            atlas::glyph('y');
            glyphs.sync(&gpu.device, &gpu.queue);
            assert_eq!(atlas::frame(), atlas::Frame::default());
            let kanji = crate::test_support::undrawn('\u{9d00}', 1);
            budget(Budget {
                time: std::time::Duration::MAX,
                glyphs: 0,
            });
            let waited = atlas::waited();
            atlas::glyph(kanji.chars().next().unwrap());
            assert_eq!(atlas::waited(), waited + 1);
            glyphs.sync(&gpu.device, &gpu.queue);
            budget(Budget::UNLIMITED);
            assert_eq!(atlas::frame(), atlas::Frame::default());
        });
    }
}
