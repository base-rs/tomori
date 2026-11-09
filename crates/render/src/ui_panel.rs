// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels of styled text in their own local space, drawn in the scene pass with depth testing and alpha blending, so that a hand or controller in front of one covers it, and then, where a panel's backing is opaque enough, its backing again into depth alone, so that whatever is behind it is hidden though drawn after it (see [`occlusion`](crate::occlusion)).
//!
//! A [`Canvas`] is what a panel shows: rows of [`ui_text`](crate::ui_text) cells on a translucent backing, with a colour per span, row backgrounds, speaker bars, meters and shapes. [`Canvas::mesh`] turns it into quads in the panel's plane, in metres, centred on the panel's origin, with +X right and +Y up, its glyphs drawn from the [`Glyphs`](crate::glyphs::Glyphs) atlas and its filled quads from the atlas's solid block; it needs no GPU, so that layouts are tested without one. A [`Panel`] holds one canvas on the GPU and rebuilds its mesh only when the canvas changes, so that moving it costs one matrix per eye per frame.
use glam::Mat4;

pub use tomori_scene::canvas::{Canvas, Vertex};

const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x2, 3 => Float32];

/// The pipelines every panel is drawn with, in the scene pass: depth tested against the scene without writing depth, and alpha blended, so that panels are translucent and a nearer hand covers them; and then its backing into depth alone.
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    occluder: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Pipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        glyphs: &crate::glyphs::Glyphs,
        samples: u32,
    ) -> Self {
        Self::with_layout(device, format, glyphs.layout(), samples)
    }

    /// As [`Pipeline::new`], drawing from the atlas a scenery is given in its
    /// [`Target`](crate::scenery::Target), so that a scenery draws panels and labels of its own in the eye's pass. See
    /// `design/text.md`.
    pub fn shared(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        glyphs: &crate::glyphs::SharedGlyphs,
        samples: u32,
    ) -> Self {
        Self::with_layout(device, format, glyphs.layout(), samples)
    }

    fn with_layout(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        glyphs: &wgpu::BindGroupLayout,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui_panel.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("panel transform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("panel pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(glyphs)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("panel pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(VERTICES)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: crate::glyphs::compilation(format),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let occluder = crate::occlusion::occluder(
            device,
            "panel backing occluder",
            &pipeline_layout,
            &shader,
            VERTICES,
            format,
            samples,
        );
        Self {
            pipeline,
            occluder,
            layout,
        }
    }

    /// The layout of a panel's transforms, by which an image over it is drawn too.
    pub(crate) fn transforms(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Draw panels from here on with this pipeline and the glyph atlas `glyphs`, from [`Glyphs::binding`](crate::glyphs::Glyphs::binding).
    pub fn set(&self, pass: &mut wgpu::RenderPass<'_>, glyphs: &wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, glyphs, &[]);
    }
}

const VERTICES: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: size_of::<Vertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &ATTRIBUTES,
};

/// One panel's canvas on the GPU, with a transform for each eye.
pub struct Panel {
    vertices: wgpu::Buffer,
    capacity: usize,
    count: u32,
    cameras: [wgpu::Buffer; 2],
    bindings: [wgpu::BindGroup; 2],
    canvas: Option<Canvas>,
    /// Whether the mesh has every glyph, rather than lacking some that waited for a later frame's budget.
    complete: bool,
    /// Whether the mesh's first quad is a backing opaque enough to hide what is behind it.
    occludes: bool,
    /// The image the canvas shows, by hash, which is drawn over it from [`Self::image_quad`] once its texture is ready.
    image: Option<String>,
    /// The image's quad, in the panel's plane, with where in the image each corner is.
    image_quad: wgpu::Buffer,
}

impl Panel {
    /// A panel holding at most `capacity` vertices; a larger mesh is cut, and is a layout bug the tests catch.
    pub fn new(device: &wgpu::Device, pipeline: &Pipeline, capacity: usize) -> Self {
        let cameras = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("panel eye transform"),
                size: 64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let bindings = std::array::from_fn(|eye| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("panel eye binding"),
                layout: &pipeline.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cameras[eye].as_entire_binding(),
                }],
            })
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panel quads"),
            size: (capacity * size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let image_quad = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panel image quad"),
            size: 6 * 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            vertices,
            capacity,
            count: 0,
            cameras,
            bindings,
            canvas: None,
            complete: false,
            occludes: false,
            image: None,
            image_quad,
        }
    }

    /// Show `canvas`, rebuilding the mesh only if it changed, or if glyphs it needs were waiting for the atlas when it was last built.
    pub fn set(&mut self, queue: &wgpu::Queue, canvas: &Canvas) {
        if self.complete && self.canvas.as_ref() == Some(canvas) {
            return;
        }
        let waited = tomori_text::atlas::waited();
        let mut mesh = canvas.mesh();
        self.complete = tomori_text::atlas::waited() == waited;
        if mesh.len() > self.capacity {
            log::debug!(
                "A panel's mesh of {} vertices was cut to {}",
                mesh.len(),
                self.capacity
            );
            mesh.truncate(self.capacity - self.capacity % 6);
        }
        self.count = mesh.len() as u32;
        // A canvas's mesh starts with its backing, unless the backing is wholly transparent.
        self.occludes = crate::occlusion::occludes(canvas.background[3]) && mesh.len() >= 6;
        if !mesh.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&mesh));
        }
        self.image = canvas
            .image_rect()
            .map(|(hash, [left, bottom, right, top])| {
                let quad: [[f32; 4]; 6] = [
                    [left, top, 0.0, 0.0],
                    [right, top, 1.0, 0.0],
                    [left, bottom, 0.0, 1.0],
                    [right, top, 1.0, 0.0],
                    [right, bottom, 1.0, 1.0],
                    [left, bottom, 0.0, 1.0],
                ];
                queue.write_buffer(&self.image_quad, 0, bytemuck::cast_slice(&quad));
                hash.to_owned()
            });
        self.canvas = Some(canvas.clone());
    }

    /// The image its canvas shows, by hash, if any.
    pub fn image(&self) -> Option<&str> {
        self.image.as_deref()
    }

    /// Draw its image's quad for `eye` from `binding`, its texture, with the image pipeline already set on `pass`.
    pub(crate) fn draw_image(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        binding: &wgpu::BindGroup,
        eye: usize,
    ) {
        pass.set_bind_group(0, &self.bindings[eye], &[]);
        pass.set_bind_group(1, binding, &[]);
        pass.set_vertex_buffer(0, self.image_quad.slice(..));
        pass.draw(0..6, 0..1);
    }

    /// Write `eye`'s transform, from the panel's plane to clip space, before the pass that draws it.
    pub fn prepare(&self, queue: &wgpu::Queue, eye: usize, transform: Mat4) {
        queue.write_buffer(
            &self.cameras[eye],
            0,
            bytemuck::cast_slice(&transform.to_cols_array()),
        );
    }

    /// Draw for `eye`, with [`Pipeline::set`] already called on `pass` with `pipeline`, which it is again once drawn: the
    /// panel, then its backing into depth alone, if it is opaque enough to hide what is behind it.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &Pipeline, eye: usize) {
        if self.count == 0 {
            return;
        }
        pass.set_bind_group(0, &self.bindings[eye], &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
        if self.occludes {
            pass.set_pipeline(&pipeline.occluder);
            pass.draw(0..6, 0..1);
            pass.set_pipeline(&pipeline.pipeline);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_text::{Row, Style};

    #[test]
    fn a_panel_built_while_glyphs_waited_is_built_again_until_it_has_them() {
        crate::test_gpu::with(|gpu| {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let glyphs = crate::glyphs::Glyphs::new(&gpu.device);
            let pipeline = Pipeline::new(&gpu.device, format, &glyphs, 1);
            let mut panel = Panel::new(&gpu.device, &pipeline, 1_000);
            let mut canvas = Canvas::new(8, 0.001);
            let kanji = crate::test_support::undrawn('\u{9f9f}', 2);
            canvas
                .rows
                .push(Row::text(&kanji, Style::plain([1.0; 4]), 8));
            // This frame's budget is spent: the panel is drawn without its glyphs, and drawn again next frame.
            let budget = crate::test_support::Budgeted::spent();
            panel.set(&gpu.queue, &canvas);
            assert!(!panel.complete);
            let bare = panel.count;
            tomori_text::atlas::frame();
            panel.set(&gpu.queue, &canvas);
            assert!(!panel.complete);
            assert_eq!(panel.count, bare);
            // With the budget to draw them, it has them, and is not built again.
            drop(budget);
            panel.set(&gpu.queue, &canvas);
            assert!(panel.complete);
            assert_eq!(panel.count, bare + 12);
            let waited = tomori_text::atlas::waited();
            panel.set(&gpu.queue, &canvas);
            assert_eq!(tomori_text::atlas::waited(), waited);
        });
    }

    #[test]
    fn a_panel_cuts_a_mesh_too_large_for_it_and_draws_nothing_when_empty() {
        crate::test_support::log_everything();
        crate::test_gpu::with(|gpu| {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let glyphs = crate::glyphs::Glyphs::new(&gpu.device);
            let pipeline = Pipeline::new(&gpu.device, format, &glyphs, 1);
            let mut panel = Panel::new(&gpu.device, &pipeline, 12);
            let mut canvas = Canvas::new(8, 0.001);
            let mut row = Row::new(vec![('#', Style::plain([1.0; 4])); 8]);
            row.background = Some([0.2, 0.2, 0.2, 1.0]);
            canvas.rows.push(row);
            assert!(canvas.mesh().len() > 12);
            panel.set(&gpu.queue, &canvas);
            assert_eq!(panel.count, 12);
            // The same canvas again changes nothing.
            panel.set(&gpu.queue, &canvas);
            let mut empty = Canvas::new(1, 1.0);
            empty.background = [0.0; 4];
            panel.set(&gpu.queue, &empty);
            assert_eq!(panel.count, 0);
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("panel test"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("panel test depth"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth = depth.create_view(&Default::default());
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("panel test"),
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
                glyphs.sync(&gpu.device, &gpu.queue);
                pipeline.set(&mut pass, &glyphs.binding());
                panel.draw(&mut pass, &pipeline, 0);
            }
            gpu.queue.submit([encoder.finish()]);
        });
    }

    /// `canvas` drawn face on, `scale` screen pixels to its font pixel and shifted `shift` of a pixel right and down, as
    /// a headset sees a panel at about a metre, its pixels nowhere on the screen's grid; with where its font pixel `(x,
    /// y)`, from its top-left corner, falls on the image.
    fn drawn_face_on(
        gpu: &crate::test_gpu::Gpu,
        canvas: &Canvas,
        scale: f32,
        shift: f32,
    ) -> (image::RgbaImage, impl Fn(f32, f32) -> (f32, f32)) {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let glyphs = crate::glyphs::Glyphs::new(&gpu.device);
        let pipeline = Pipeline::new(&gpu.device, format, &glyphs, 1);
        let mut panel = Panel::new(&gpu.device, &pipeline, 100_000);
        let (columns, rows) = canvas.size_in_pixels();
        // Rows read back are a multiple of 256 bytes.
        let (width, height) = (
            ((columns as f32 * scale) as u32 + 16).next_multiple_of(64),
            (rows as f32 * scale) as u32 + 16,
        );
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let target = |format, usage, label| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
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
            "face on",
        );
        let depth = target(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            "face on depth",
        );
        // From the panel's plane, in metres about its centre, to clip space: a font pixel `scale` screen pixels.
        let per_metre = scale / canvas.pixel;
        let shifted = 2.0 * shift / glam::Vec2::new(width as f32, -(height as f32));
        let transform = Mat4::from_translation(shifted.extend(0.0))
            * Mat4::from_scale(glam::Vec3::new(
                2.0 * per_metre / width as f32,
                2.0 * per_metre / height as f32,
                1.0,
            ));
        panel.set(&gpu.queue, canvas);
        panel.prepare(&gpu.queue, 0, transform);
        glyphs.sync(&gpu.device, &gpu.queue);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("face on"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture.create_view(&Default::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.create_view(&Default::default()),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pipeline.set(&mut pass, &glyphs.binding());
            panel.draw(&mut pass, &pipeline, 0);
        }
        gpu.queue.submit([encoder.finish()]);
        let image =
            crate::test_support::read_back(&gpu.device, &gpu.queue, &texture, (width, height));
        let place = move |x: f32, y: f32| {
            (
                width as f32 / 2.0 + (x - columns as f32 / 2.0) * scale + shift,
                height as f32 / 2.0 + (y - rows as f32 / 2.0) * scale + shift,
            )
        };
        (image, place)
    }

    /// How bright `image` is at `(x, y)`, the sum of its channels.
    fn brightness(image: &image::RgbaImage, x: u32, y: u32) -> u32 {
        image.get_pixel(x, y).0[..3]
            .iter()
            .map(|&c| u32::from(c))
            .sum()
    }

    /// How much a line adds to `image` through each pixel of `along`, summed over the pixels `across` it, above the
    /// darkest of them; and the most of those over the least, which a line as bright all along keeps near one.
    fn evenness(
        image: &image::RgbaImage,
        along: std::ops::Range<u32>,
        across: std::ops::Range<u32>,
        at: impl Fn(u32, u32) -> (u32, u32),
    ) -> (Vec<u32>, f32) {
        let added: Vec<u32> = along
            .map(|step| {
                let values: Vec<u32> = across
                    .clone()
                    .map(|side| {
                        let (x, y) = at(step, side);
                        brightness(image, x, y)
                    })
                    .collect();
                let least = values.iter().copied().min().unwrap_or_default();
                values.iter().map(|value| value - least).sum()
            })
            .collect();
        let most = added.iter().copied().max().unwrap_or_default();
        let least = added.iter().copied().min().unwrap_or_default().max(1);
        (added, most as f32 / least as f32)
    }

    /// Set `TOMORI_TABLE_RULE_PREVIEW` to a PNG path to save what this renders: a table on a panel in a wall's slot, face
    /// on, at about the headset's resolution, its pixels off the screen's grid.
    #[test]
    fn a_table_s_rule_and_the_lines_between_its_columns_are_as_bright_all_along() {
        crate::test_gpu::with(|gpu| {
            let body = "| Step | What happens |\n|---|---|\n| 1 | The agent reads the table, lays out its columns, wraps each cell within its own column, and draws the rule below the header and the lines between the columns, each as one line rather than many short pieces |\n| 2 | Striped |\n";
            let panel = tomori_protocol::Panel::new(
                "panel-1",
                "Steps",
                body,
                tomori_protocol::Place::Wall {
                    wall: tomori_protocol::TOMORI_WALL.into(),
                    slot: 0,
                },
            );
            let canvas = tomori_scene::projection::face(
                &panel,
                false,
                false,
                tomori_scene::projection::Fit::SLOT,
            );
            let (image, place) = drawn_face_on(&gpu, &canvas, 2.3, 0.37);
            crate::test_support::preview("TOMORI_TABLE_RULE_PREVIEW", &image);
            let (left, top) = (canvas.padding.0 as f32, canvas.padding.1 as f32);
            let cell = |column: usize, row: usize| {
                place(
                    left + (column * crate::ui_text::CELL_WIDTH) as f32,
                    top + (row * crate::ui_text::CELL_HEIGHT) as f32,
                )
            };
            // The rule below the header: the body's first row of nothing but a rule, below the panel's own.
            let rule = (3..canvas.rows.len())
                .find(|&row| {
                    let cells = &canvas.rows[row].cells;
                    !cells.is_empty() && cells.iter().all(|(ch, _)| *ch == '─')
                })
                .expect("a rule below the header");
            let length = canvas.rows[rule].cells.len();
            let (start, upper) = cell(0, rule);
            let (end, lower) = cell(length, rule + 1);
            let (_, ratio) = evenness(
                &image,
                start.ceil() as u32 + 3..end.floor() as u32 - 3,
                upper.ceil() as u32..lower.floor() as u32,
                |x, y| (x, y),
            );
            assert!(ratio < 1.1, "the rule is uneven along its length: {ratio}");
            // The line between the columns, down the first row's lines, which wrap.
            let lined = |row: usize| canvas.rows[row].cells.iter().position(|(ch, _)| *ch == '▏');
            let column = lined(rule + 1).expect("a line between the columns");
            let lines = (rule + 1..canvas.rows.len())
                .take_while(|&row| {
                    lined(row) == Some(column) && canvas.rows[row].cells[0].1.background.is_none()
                })
                .count();
            assert!(lines >= 4, "the first row wraps: {lines}");
            let (x, first) = cell(column, rule + 1);
            let (_, last) = cell(column, rule + 1 + lines);
            let span = (5.0 * 2.3) as u32;
            let (added, ratio) = evenness(
                &image,
                first.ceil() as u32 + 3..last.floor() as u32 - 3,
                x as u32 - span..x as u32 + span,
                |y, x| (x, y),
            );
            assert!(added.iter().all(|&added| added > 0), "the line has gaps");
            assert!(
                ratio < 1.1,
                "the line between the columns is uneven along its length: {ratio}"
            );
        });
    }
}
