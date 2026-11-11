// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's scenery, as Tomori's clients draw it in each eye's pass: the evening sky behind everything, then the
//! yard and the coop, models drawn with `tomori-assets`, lit by the moon and the lanterns' warm light; beyond them the
//! backdrop, its foothills and its bush, models too, its river and waterfall, drawn with a shader of its own that moves
//! them, and the ranges at the sky's depth; all of it growing out of the space through the reveal sphere as the space's
//! environment does.
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use tomori_assets::gpu::{Batch, GpuModel, Lamp, Lighting, Renderer};
use tomori_assets::{Instance, Library, Model};
use tomori_render::scenery::{Scenery, Target};
use wgpu::util::DeviceExt;

use tomori_environment_farmyard::backdrop::{self, Kind};
use tomori_environment_farmyard::coop::{self, Coop};
use tomori_environment_farmyard::layout;
use tomori_environment_farmyard::props::{self, evening, palette};

/// What the sky's, the ranges' and the water's shaders are told for each eye.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyCamera {
    inverse_view_projection: [f32; 16],
    reveal: [f32; 4],
    view_projection: [f32; 16],
    eye: [f32; 4],
    haze: [f32; 4],
    time: [f32; 4],
}

/// A point of the ranges, as their shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FarVertex {
    direction: [f32; 3],
    color: [f32; 4],
}

/// A point of the water, as its shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterVertex {
    position: [f32; 3],
    color: [f32; 4],
    flow: [f32; 2],
}

/// What moves the water: the time since the scenery was made, wrapping every quarter of an hour, or a moment held
/// still, for pictures that must be the same each time they are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clock {
    Running,
    Held(f32),
}

impl Clock {
    /// The time it says, in seconds, for a scenery made at `made`.
    pub fn seconds(self, made: Instant) -> f32 {
        match self {
            Clock::Running => made.elapsed().as_secs_f32() % 900.0,
            Clock::Held(seconds) => seconds,
        }
    }
}

/// How the farmyard is lit: a cool moon low in the east, the sky's blue overhead and the earth's brown below, the
/// lanterns' warm light by the coop's door, on the gate and over the workbench, and a dusky haze over the far meadow.
pub fn lighting() -> Lighting {
    Lighting::new(
        evening::MOON.normalize(),
        evening::MOONLIGHT,
        evening::SKY,
        evening::GROUND,
    )
    .with_haze(Vec3::new(0.03, 0.033, 0.06), 0.0008)
    .with_lamp(Lamp::new(layout::LANTERN, 6.0, palette::GLOW * 1.4))
    .with_lamp(Lamp::new(layout::GATE_LANTERN, 4.5, palette::GLOW * 1.1))
    .with_lamp(Lamp::new(
        layout::workbench_lantern(),
        3.5,
        palette::GLOW * 1.2,
    ))
}

/// Where the eye is, from its view and projection: the point every ray of the view begins from, as every scenery is
/// told it.
pub use tomori_render::scenery::eye;

/// One model the farmyard draws, uploaded once, with its instances placed.
struct Drawn {
    model: GpuModel,
    batch: Batch,
}

impl Drawn {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        model: &Arc<Model>,
        placement: Mat4,
    ) -> Self {
        Self::placed(device, queue, renderer, model, &[placement])
    }

    /// `model`, uploaded once, with an instance at each of `placements`, drawn together.
    fn placed(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        model: &Arc<Model>,
        placements: &[Mat4],
    ) -> Self {
        let gpu = renderer.upload(device, queue, model);
        let mut batch = renderer.batch(device);
        let instances: Vec<Instance> = placements
            .iter()
            .map(|placement| {
                let mut instance = Instance::new(model.clone());
                instance.transform = *placement;
                instance
            })
            .collect();
        renderer.place(device, queue, &gpu, &mut batch, &instances);
        Self { model: gpu, batch }
    }
}

/// A pipeline of the farmyard's own, drawing triangles of `T` from `buffers`' layout with `shader`'s `vertex` and
/// `fragment` entry points, in the eyes' pass, writing depth or not.
#[allow(clippy::too_many_arguments)]
fn pipeline(
    target: &Target,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    (vertex, fragment): (&str, &str),
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    (write, compare): (bool, wgpu::CompareFunction),
    label: &str,
) -> wgpu::RenderPipeline {
    target
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: target.samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
}

/// Triangles of the farmyard's own, in a buffer, drawn with their pipeline.
struct Triangles {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    count: u32,
}

impl Triangles {
    fn new<T: Pod>(device: &wgpu::Device, pipeline: wgpu::RenderPipeline, vertices: &[T]) -> Self {
        Self {
            pipeline,
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Farmyard backdrop"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            count: vertices.len() as u32,
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

/// The backdrop beyond the fence: its foothills, each kind of plant with all its instances, the water and the ranges.
struct Backdrop {
    foothills: Drawn,
    plants: Vec<Drawn>,
    water: Triangles,
    ranges: Triangles,
}

impl Backdrop {
    fn new(
        target: &Target,
        models: &Renderer,
        layout: &wgpu::PipelineLayout,
        sky: &wgpu::ShaderModule,
    ) -> Self {
        let (device, queue) = (target.device, target.queue);
        let foothills = Drawn::new(
            device,
            queue,
            models,
            &Arc::new(backdrop::model()),
            Mat4::IDENTITY,
        );
        let placed = backdrop::plants();
        let plants = Kind::ALL
            .iter()
            .map(|kind| {
                let placements: Vec<Mat4> = placed
                    .iter()
                    .filter(|plant| plant.kind == *kind)
                    .map(|plant| plant.transform())
                    .collect();
                Drawn::placed(device, queue, models, &Arc::new(kind.model()), &placements)
            })
            .collect();
        let shader = device.create_shader_module(wgpu::include_wgsl!("water.wgsl"));
        let water: Vec<WaterVertex> = backdrop::water()
            .into_iter()
            .map(|point| WaterVertex {
                position: point.position.to_array(),
                color: point.color.extend(point.kind).to_array(),
                flow: point.flow.to_array(),
            })
            .collect();
        let water = Triangles::new(
            device,
            pipeline(
                target,
                layout,
                &shader,
                ("water_vertex", "water_fragment"),
                &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<WaterVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32x2],
                })],
                (true, wgpu::CompareFunction::Less),
                "Farmyard water",
            ),
            &water,
        );
        let ranges: Vec<FarVertex> = backdrop::ranges()
            .into_iter()
            .map(|far| FarVertex {
                direction: far.direction.to_array(),
                color: far.color.extend(far.fade).to_array(),
            })
            .collect();
        // At the sky's depth, where nothing nearer has been drawn, each range over the one beyond it.
        let ranges = Triangles::new(
            device,
            pipeline(
                target,
                layout,
                sky,
                ("range_vertex", "range_fragment"),
                &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<FarVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
                (false, wgpu::CompareFunction::LessEqual),
                "Farmyard ranges",
            ),
            &ranges,
        );
        Self {
            foothills,
            plants,
            water,
            ranges,
        }
    }
}

/// The farmyard's scenery.
pub struct Farmyard {
    sky: wgpu::RenderPipeline,
    cameras: [wgpu::Buffer; 2],
    bindings: [wgpu::BindGroup; 2],
    models: Renderer,
    yard: Drawn,
    coop: Drawn,
    backdrop: Backdrop,
    /// The haze's colour and density, which the water is hazed by as the models are.
    haze: [f32; 4],
    clock: Clock,
    made: Instant,
    /// The chickens, uploaded once their client side first shows them, placed as it last did.
    chickens: Option<Drawn>,
    reveal: Option<(Vec3, f32)>,
}

impl Farmyard {
    /// The farmyard for `target`, with `coop`, whose asset, if it has one, comes from `library`, its water moving with
    /// the time.
    pub fn new(target: &Target, coop: &Coop, library: &Library) -> Self {
        Self::with_clock(target, coop, library, Clock::Running)
    }

    /// As [`Self::new`], its water moved by `clock`.
    pub fn with_clock(target: &Target, coop: &Coop, library: &Library, clock: Clock) -> Self {
        let device = target.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Farmyard sky layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let cameras: [wgpu::Buffer; 2] = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Farmyard sky camera"),
                size: size_of::<SkyCamera>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let bindings = std::array::from_fn(|eye| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Farmyard sky camera"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cameras[eye].as_entire_binding(),
                }],
            })
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("sky.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Farmyard sky"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sky = pipeline(
            target,
            &pipeline_layout,
            &shader,
            ("sky_vertex", "sky_fragment"),
            &[],
            (false, wgpu::CompareFunction::LessEqual),
            "Farmyard sky",
        );
        let models = Renderer::new(device, target.queue, target.format, target.samples);
        let lighting = lighting();
        models.set_lighting(target.queue, &lighting);
        let yard = Drawn::new(
            device,
            target.queue,
            &models,
            &Arc::new(props::model()),
            Mat4::IDENTITY,
        );
        let coop = Drawn::new(
            device,
            target.queue,
            &models,
            &coop.model(library),
            coop::placement(),
        );
        let backdrop = Backdrop::new(target, &models, &pipeline_layout, &shader);
        Self {
            sky,
            cameras,
            bindings,
            models,
            yard,
            coop,
            backdrop,
            haze: lighting.haze.extend(lighting.haze_density).to_array(),
            clock,
            made: Instant::now(),
            chickens: None,
            reveal: None,
        }
    }
}

impl Scenery for Farmyard {
    fn animate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        drawn: &(dyn std::any::Any + Send + Sync),
    ) {
        let Some(drawn) = drawn.downcast_ref::<crate::client::Drawn>() else {
            return;
        };
        let Some(first) = drawn.chickens.first() else {
            return;
        };
        let models = &self.models;
        let chickens = self.chickens.get_or_insert_with(|| Drawn {
            model: models.upload(device, queue, first.model()),
            batch: models.batch(device),
        });
        models.place(
            device,
            queue,
            &chickens.model,
            &mut chickens.batch,
            &drawn.chickens,
        );
    }

    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4) {
        self.models
            .prepare(queue, eye, scene_to_clip, self::eye(scene_to_clip));
        let inverse = scene_to_clip.inverse();
        let camera = SkyCamera {
            inverse_view_projection: if inverse.is_finite() {
                inverse
            } else {
                Mat4::IDENTITY
            }
            .to_cols_array(),
            reveal: match self.reveal {
                Some((centre, radius)) => centre.extend(radius.max(0.0)).to_array(),
                None => [0.0, 0.0, 0.0, -1.0],
            },
            view_projection: scene_to_clip.to_cols_array(),
            eye: self::eye(scene_to_clip).extend(1.0).to_array(),
            haze: self.haze,
            time: [self.clock.seconds(self.made), 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.cameras[eye], 0, bytemuck::bytes_of(&camera));
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        pass.set_pipeline(&self.sky);
        pass.set_bind_group(0, &self.bindings[eye], &[]);
        pass.draw(0..3, 0..1);
        // Nearest first, so that what they hide is not shaded: the yard, the coop and the chickens, then the foothills and
        // the bush beyond, the water, and the ranges at the sky's depth, where nothing else has been drawn.
        let backdrop = &self.backdrop;
        for drawn in [Some(&self.yard), Some(&self.coop), self.chickens.as_ref()]
            .into_iter()
            .flatten()
            .chain([&backdrop.foothills])
            .chain(&backdrop.plants)
        {
            self.models.draw(pass, eye, &drawn.model, &drawn.batch);
        }
        pass.set_bind_group(0, &self.bindings[eye], &[]);
        backdrop.water.draw(pass);
        backdrop.ranges.draw(pass);
    }

    fn set_reveal(&mut self, reveal: Option<(Vec3, f32)>) {
        self.reveal = reveal.filter(|(centre, radius)| centre.is_finite() && radius.is_finite());
        self.models.set_reveal(self.reveal);
    }
}

#[cfg(test)]
mod tests;
