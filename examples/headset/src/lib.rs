// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An example of a headset built on `tomori-client-quest` by another project: it draws a scenery of its own, a flat coloured floor, for the placeholder environment that `tomori-example-server` offers, registered through [`tomori::Builder`] beside Tomori's own, without changing Tomori. Its executable, `tomori-example-headset`, takes the same command line as `tomori`.
use glam::{Mat4, Vec3};
use tomori_render::scenery::{Scenery, Target};
use wgpu::util::DeviceExt;

/// The placeholder environment's identifier, as `tomori-example-server` offers it.
pub const ENVIRONMENT: &str = "placeholder";

/// The floor's colour, linear.
pub const COLOUR: [f32; 3] = [0.55, 0.25, 0.45];

/// Tomori's own headset, drawing the placeholder floor too.
pub fn builder() -> tomori::Builder {
    tomori::Builder::default().scenery(ENVIRONMENT, floor)
}

/// This build's headset on the Quest, in place of Tomori's own, which this crate turns off by depending on `tomori-client-quest` without its default `app` feature.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: tomori::AndroidApp) {
    builder().run_android(app)
}

/// The placeholder floor, for `target`.
pub fn floor(target: &Target) -> Box<dyn Scenery> {
    Box::new(Floor::new(target))
}

/// A flat, coloured floor, drawn before the scene in each eye.
pub struct Floor {
    pipeline: wgpu::RenderPipeline,
    eyes: [(wgpu::Buffer, wgpu::BindGroup); 2],
}

impl Floor {
    fn new(target: &Target) -> Self {
        let shader = target
            .device
            .create_shader_module(wgpu::include_wgsl!("floor.wgsl"));
        let layout = target
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("placeholder floor eye"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let pipeline_layout =
            target
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("placeholder floor"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                });
        let pipeline = target
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("placeholder floor"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(target.format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                // In the eye's pass, which has a depth target: the floor is behind everything else in the scene.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: target.samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            });
        let eyes = std::array::from_fn(|_| {
            let buffer = target
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("placeholder floor eye"),
                    contents: bytemuck::cast_slice(&Mat4::IDENTITY.to_cols_array()),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
            let group = target.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("placeholder floor eye"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            (buffer, group)
        });
        Self { pipeline, eyes }
    }
}

impl Scenery for Floor {
    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4) {
        queue.write_buffer(
            &self.eyes[eye].0,
            0,
            bytemuck::cast_slice(&scene_to_clip.to_cols_array()),
        );
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.eyes[eye].1, &[]);
        pass.draw(0..6, 0..1);
    }

    /// The floor is all or nothing: it shows once a space's scenery has grown, rather than within the growing sphere.
    fn set_reveal(&mut self, _reveal: Option<(Vec3, f32)>) {}
}
