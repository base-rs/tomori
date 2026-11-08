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
            bind_group_layouts: &[Some(&layout), Some(glyphs.layout())],
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
}
