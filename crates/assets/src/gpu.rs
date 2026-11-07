// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Drawing models with wgpu: a [`Renderer`] uploads each model once, as a [`GpuModel`], and draws a [`Batch`] of its
//! instances in an eye's pass, one draw for every instance of each primitive, skinned on the GPU from each instance's
//! joint matrices, which a storage buffer holds, or placed by its node's matrix, lit by the scenery's [`Lighting`].
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::{INFLUENCES, Instance, Model, Texture};

/// The most lamps that light a scenery's models.
pub const LAMPS: usize = 4;

/// A lamp: where it is, how far its light reaches, falling to nothing there, and its colour, as bright as at its
/// centre, linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Lamp {
    pub position: Vec3,
    pub reach: f32,
    pub color: Vec3,
}

impl Lamp {
    pub fn new(position: Vec3, reach: f32, color: Vec3) -> Self {
        Self {
            position,
            reach,
            color,
        }
    }
}

/// How a scenery lights its models: an ambient light from the sky above and the ground below, the sun or moon, up to
/// [`LAMPS`] lamps, and haze that fades distant things into its colour. Colours are linear RGB.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Lighting {
    /// The direction towards the sun or moon.
    pub sun_direction: Vec3,
    pub sun: Vec3,
    pub sky: Vec3,
    pub ground: Vec3,
    pub haze: Vec3,
    /// How thick the haze is: what is `d` metres away fades by `1 - exp(-density d²)`.
    pub haze_density: f32,
    pub lamps: Vec<Lamp>,
}

impl Default for Lighting {
    /// A plain, even light from above, without haze or lamps.
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(0.3, 0.8, 0.5).normalize(),
            sun: Vec3::splat(0.6),
            sky: Vec3::splat(0.4),
            ground: Vec3::splat(0.2),
            haze: Vec3::ZERO,
            haze_density: 0.0,
            lamps: Vec::new(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [f32; 16],
    eye: [f32; 4],
    reveal: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LampUniform {
    position: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightingUniform {
    sun_direction: [f32; 4],
    sun: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    haze: [f32; 4],
    lamps: [LampUniform; LAMPS],
    count: [u32; 4],
}

impl From<&Lighting> for LightingUniform {
    fn from(lighting: &Lighting) -> Self {
        let mut lamps = [LampUniform::zeroed(); LAMPS];
        for (slot, lamp) in lamps.iter_mut().zip(&lighting.lamps) {
            *slot = LampUniform {
                position: lamp.position.extend(lamp.reach).to_array(),
                color: lamp.color.extend(1.0).to_array(),
            };
        }
        Self {
            sun_direction: lighting
                .sun_direction
                .normalize_or_zero()
                .extend(0.0)
                .to_array(),
            sun: lighting.sun.extend(0.0).to_array(),
            sky: lighting.sky.extend(0.0).to_array(),
            ground: lighting.ground.extend(0.0).to_array(),
            haze: lighting.haze.extend(lighting.haze_density).to_array(),
            lamps,
            count: [lighting.lamps.len().min(LAMPS) as u32, 0, 0, 0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    emissive: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Placed {
    world: [f32; 16],
    joints: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    joints: [u16; INFLUENCES],
    weights: [f32; INFLUENCES],
}

/// A primitive on the GPU.
struct GpuPrimitive {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    material: Option<usize>,
}

/// A model on the GPU: its primitives, by mesh, and its materials with their textures.
pub struct GpuModel {
    primitives: Vec<GpuPrimitive>,
    meshes: Vec<Range<usize>>,
    materials: Vec<wgpu::BindGroup>,
    plain: wgpu::BindGroup,
}

/// What a batch draws: each primitive with its range of placed instances.
struct Call {
    primitive: usize,
    instances: Range<u32>,
}

/// The instances of one model to draw this frame: where each primitive of each is placed, and its joint matrices.
pub struct Batch {
    placed: wgpu::Buffer,
    joints: wgpu::Buffer,
    binding: wgpu::BindGroup,
    calls: Vec<Call>,
}

/// Draws models in an eye's pass, into targets of the format and sample count it was made for, with a 32-bit float
/// depth buffer.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    revealed: wgpu::RenderPipeline,
    material_layout: wgpu::BindGroupLayout,
    placed_layout: wgpu::BindGroupLayout,
    cameras: [wgpu::Buffer; 2],
    lighting: wgpu::Buffer,
    frames: [wgpu::BindGroup; 2],
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    reveal: Option<(Vec3, f32)>,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// `texture`'s mipmaps, from its own size down to one texel, each averaging up to four texels of the last in linear
/// light, so that a distant model does not shimmer.
pub(crate) fn mipmaps(texture: &Texture) -> Vec<(u32, u32, Vec<u8>)> {
    let to_linear = |value: u8| (f32::from(value) / 255.0).powf(2.2);
    let mut levels = vec![(texture.width, texture.height, texture.pixels.clone())];
    let mut level: Vec<f32> = texture
        .pixels
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index % 4 == 3 {
                f32::from(*value) / 255.0
            } else {
                to_linear(*value)
            }
        })
        .collect();
    let (mut width, mut height) = (texture.width as usize, texture.height as usize);
    while width > 1 || height > 1 {
        let (next_width, next_height) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = vec![0.0; next_width * next_height * 4];
        for y in 0..next_height {
            for x in 0..next_width {
                for channel in 0..4 {
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(width - 1);
                        let sy = (y * 2 + dy).min(height - 1);
                        sum += level[(sy * width + sx) * 4 + channel];
                    }
                    next[(y * next_width + x) * 4 + channel] = sum / 4.0;
                }
            }
        }
        let bytes = next
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let value = if index % 4 == 3 {
                    *value
                } else {
                    value.powf(1.0 / 2.2)
                };
                (value * 255.0).round() as u8
            })
            .collect();
        levels.push((next_width as u32, next_height as u32, bytes));
        level = next;
        (width, height) = (next_width, next_height);
    }
    levels
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &Texture,
) -> wgpu::TextureView {
    let levels = mipmaps(texture);
    let gpu = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Model texture"),
        size: wgpu::Extent3d {
            width: texture.width,
            height: texture.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (width, height, pixels)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(*height),
            },
            wgpu::Extent3d {
                width: *width,
                height: *height,
                depth_or_array_layers: 1,
            },
        );
    }
    gpu.create_view(&Default::default())
}

impl Renderer {
    /// A renderer for targets of `format` with `samples` samples a pixel.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Model frame layout"),
            entries: &[
                uniform_entry(0, both),
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Model material layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let placed_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Model placement layout"),
            entries: &[storage_entry(0), storage_entry(1)],
        });
        let cameras = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Model eye camera"),
                size: size_of::<CameraUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let lighting = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Model lighting"),
            contents: bytemuck::bytes_of(&LightingUniform::from(&Lighting::default())),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frames = std::array::from_fn(|eye: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Model frame"),
                layout: &frame_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: cameras[eye].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: lighting.as_entire_binding(),
                    },
                ],
            })
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("model.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Model pipeline layout"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&material_layout),
                Some(&placed_layout),
            ],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4,
            4 => Uint16x4, 5 => Uint16x4, 6 => Float32x4, 7 => Float32x4
        ];
        let pipeline = |fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Model pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("model_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let white = upload_texture(
            device,
            queue,
            &Texture {
                width: 1,
                height: 1,
                pixels: vec![255; 4],
            },
        );
        Self {
            pipeline: pipeline("model_fragment"),
            revealed: pipeline("revealed_model_fragment"),
            material_layout,
            placed_layout,
            cameras,
            lighting,
            frames,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Model sampler"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            white,
            reveal: None,
        }
    }

    fn material(
        &self,
        device: &wgpu::Device,
        uniform: MaterialUniform,
        texture: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Model material"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model material"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Upload `model`, once, to draw any number of its instances.
    pub fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue, model: &Model) -> GpuModel {
        let textures: Vec<_> = model
            .textures
            .iter()
            .map(|texture| upload_texture(device, queue, texture))
            .collect();
        let materials = model
            .materials
            .iter()
            .map(|material| {
                let uniform = MaterialUniform {
                    base_color: material.base_color.to_array(),
                    emissive: material
                        .emissive
                        .extend(f32::from(u8::from(material.masked)))
                        .to_array(),
                };
                let texture = material
                    .texture
                    .map_or(&self.white, |texture| &textures[texture]);
                self.material(device, uniform, texture)
            })
            .collect();
        let mut primitives = Vec::new();
        let meshes = model
            .meshes
            .iter()
            .map(|mesh| {
                let start = primitives.len();
                for primitive in &mesh.primitives {
                    let vertices: Vec<Vertex> = (0..primitive.positions.len())
                        .map(|index| Vertex {
                            position: primitive.positions[index].to_array(),
                            normal: primitive
                                .normals
                                .get(index)
                                .copied()
                                .unwrap_or(Vec3::ZERO)
                                .to_array(),
                            uv: primitive
                                .uvs
                                .get(index)
                                .copied()
                                .unwrap_or_default()
                                .to_array(),
                            color: primitive
                                .colors
                                .get(index)
                                .copied()
                                .unwrap_or(Vec4::ONE)
                                .to_array(),
                            joints: primitive.joints.get(index).copied().unwrap_or_default(),
                            weights: primitive.weights.get(index).copied().unwrap_or_default(),
                        })
                        .collect();
                    primitives.push(GpuPrimitive {
                        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Model vertices"),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Model indices"),
                            contents: bytemuck::cast_slice(&primitive.indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                        count: primitive.indices.len() as u32,
                        material: primitive.material,
                    });
                }
                start..primitives.len()
            })
            .collect();
        let plain = MaterialUniform {
            base_color: [1.0; 4],
            emissive: [0.0; 4],
        };
        GpuModel {
            primitives,
            meshes,
            materials,
            plain: self.material(device, plain, &self.white),
        }
    }

    /// A batch to draw instances with.
    pub fn batch(&self, device: &wgpu::Device) -> Batch {
        self.sized_batch(device, 1, 1)
    }

    fn sized_batch(&self, device: &wgpu::Device, placed: usize, joints: usize) -> Batch {
        let buffer = |label, bytes: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let placed = buffer("Model placements", placed * size_of::<Placed>());
        let joints = buffer("Model joints", joints * size_of::<[f32; 16]>());
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model placements"),
            layout: &self.placed_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: placed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: joints.as_entire_binding(),
                },
            ],
        });
        Batch {
            placed,
            joints,
            binding,
            calls: Vec::new(),
        }
    }

    /// Place `instances`, each of `model`, in `batch`, to be drawn as they are now, growing its buffers if need be.
    pub fn place(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &GpuModel,
        batch: &mut Batch,
        instances: &[Instance],
    ) {
        let mut placed: Vec<Vec<Placed>> = vec![Vec::new(); model.primitives.len()];
        let mut joints: Vec<[f32; 16]> = Vec::new();
        for instance in instances {
            for draw in instance.draws() {
                let first = joints.len() as u32;
                let skinned = draw.joints.as_ref().map_or(0, |matrices| {
                    joints.extend(matrices.iter().map(Mat4::to_cols_array));
                    1
                });
                for primitive in model.meshes[draw.mesh].clone() {
                    placed[primitive].push(Placed {
                        world: draw.world.to_cols_array(),
                        joints: [first, skinned, 0, 0],
                    });
                }
            }
        }
        let mut calls = Vec::new();
        let mut flat = Vec::new();
        for (primitive, placed) in placed.into_iter().enumerate() {
            if placed.is_empty() {
                continue;
            }
            let start = flat.len() as u32;
            flat.extend(placed);
            calls.push(Call {
                primitive,
                instances: start..flat.len() as u32,
            });
        }
        let placed_bytes = (flat.len().max(1) * size_of::<Placed>()) as u64;
        let joint_bytes = (joints.len().max(1) * size_of::<[f32; 16]>()) as u64;
        if placed_bytes > batch.placed.size() || joint_bytes > batch.joints.size() {
            *batch = self.sized_batch(
                device,
                flat.len().next_power_of_two(),
                joints.len().max(1).next_power_of_two(),
            );
        }
        queue.write_buffer(&batch.placed, 0, bytemuck::cast_slice(&flat));
        queue.write_buffer(&batch.joints, 0, bytemuck::cast_slice(&joints));
        batch.calls = calls;
    }

    /// Light every model as `lighting` says, from the next pass.
    pub fn set_lighting(&self, queue: &wgpu::Queue, lighting: &Lighting) {
        queue.write_buffer(
            &self.lighting,
            0,
            bytemuck::bytes_of(&LightingUniform::from(lighting)),
        );
    }

    /// Draw models only inside a sphere, `(centre, radius)` in the scene, as a scenery grows out of a point, or
    /// everywhere, with `None`; applied by the next [`Self::prepare`].
    pub fn set_reveal(&mut self, reveal: Option<(Vec3, f32)>) {
        self.reveal = reveal.filter(|(centre, radius)| centre.is_finite() && radius.is_finite());
    }

    /// Before eye `eye`'s pass: its view and projection, and where the eye is.
    pub fn prepare(&self, queue: &wgpu::Queue, eye: usize, view_projection: Mat4, position: Vec3) {
        let camera = CameraUniform {
            view_projection: view_projection.to_cols_array(),
            eye: position.extend(1.0).to_array(),
            reveal: match self.reveal {
                Some((centre, radius)) => centre.extend(radius.max(0.0)).to_array(),
                None => [0.0, 0.0, 0.0, -1.0],
            },
        };
        queue.write_buffer(&self.cameras[eye], 0, bytemuck::bytes_of(&camera));
    }

    /// In eye `eye`'s pass: draw `batch`'s instances of `model`.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        eye: usize,
        model: &GpuModel,
        batch: &Batch,
    ) {
        if batch.calls.is_empty() {
            return;
        }
        pass.set_pipeline(if self.reveal.is_some() {
            &self.revealed
        } else {
            &self.pipeline
        });
        pass.set_bind_group(0, &self.frames[eye], &[]);
        pass.set_bind_group(2, &batch.binding, &[]);
        for call in &batch.calls {
            let primitive = &model.primitives[call.primitive];
            let material = primitive
                .material
                .map_or(&model.plain, |material| &model.materials[material]);
            pass.set_bind_group(1, material, &[]);
            pass.set_vertex_buffer(0, primitive.vertices.slice(..));
            pass.set_index_buffer(primitive.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..primitive.count, 0, call.instances.clone());
        }
    }
}

#[cfg(test)]
mod tests;
