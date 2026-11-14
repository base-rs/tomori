// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The PipeWire source's preview: its fixed 1920×1080 output drawn into a window of any size, whole, as large as it fits and centred, with black bars beside or above and below it rather than stretched. See `design/spectator.md`.

const SHADER: &str = "
struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varyings {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Varyings;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_main(in: Varyings) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
}
";

/// The rectangle, `[x, y, width, height]` in pixels, that shows a `source`-sized image whole in a `target`-sized one, as large as it fits, centred.
pub(crate) fn fit(source: (u32, u32), target: (u32, u32)) -> [f32; 4] {
    let (source_width, source_height) = (source.0 as f32, source.1 as f32);
    let (target_width, target_height) = (target.0 as f32, target.1 as f32);
    let scale = (target_width / source_width).min(target_height / source_height);
    let (width, height) = (source_width * scale, source_height * scale);
    [
        (target_width - width) / 2.0,
        (target_height - height) / 2.0,
        width,
        height,
    ]
}

/// Draws a texture into a window's surface, letterboxed.
pub(crate) struct Letterbox {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Letterbox {
    /// For a window's surface in `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectator preview"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectator preview"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spectator preview"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spectator preview"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spectator preview"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    /// Draw `source` into `target`, letterboxed with black bars.
    pub fn draw(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spectator preview"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let (source_size, target_size) = (source.texture().size(), target.texture().size());
        let [x, y, width, height] = fit(
            (source_size.width, source_size.height),
            (target_size.width, target_size.height),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spectator preview"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(x, y, width, height, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_fits_whole_and_centred_with_bars() {
        // The same aspect fills the window.
        assert_eq!(fit((1920, 1080), (960, 540)), [0.0, 0.0, 960.0, 540.0]);
        // A taller window has bars above and below.
        assert_eq!(fit((1920, 1080), (64, 48)), [0.0, 6.0, 64.0, 36.0]);
        // A wider one, beside it.
        assert_eq!(fit((1920, 1080), (320, 90)), [80.0, 0.0, 160.0, 90.0]);
    }

    #[test]
    fn the_preview_draws_the_output_between_black_bars() {
        tomori_render::test_gpu::with(|gpu| {
            let texture = |width, height, usage| {
                gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage,
                    view_formats: &[],
                })
            };
            // A white 16:9 source, into a square target of 64×64 pixels.
            let source = texture(
                16,
                9,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            gpu.queue.write_texture(
                source.as_image_copy(),
                &[255; 16 * 9 * 4],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(16 * 4),
                    rows_per_image: None,
                },
                source.size(),
            );
            let target = texture(
                64,
                64,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 64 * 64 * 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let letterbox = Letterbox::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            letterbox.draw(
                &gpu.device,
                &mut encoder,
                &source.create_view(&Default::default()),
                &target.create_view(&Default::default()),
            );
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(64 * 4),
                        rows_per_image: None,
                    },
                },
                target.size(),
            );
            gpu.queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let pixels = buffer.slice(..).get_mapped_range().unwrap().to_vec();
            let red = |x: usize, y: usize| pixels[(y * 64 + x) * 4];
            // The image is 36 pixels high, from row 14 to row 50, with black bars above and below it.
            assert_eq!(fit((16, 9), (64, 64)), [0.0, 14.0, 64.0, 36.0]);
            for x in [0, 32, 63] {
                assert_eq!((red(x, 0), red(x, 13)), (0, 0), "bar above, column {x}");
                assert_eq!((red(x, 14), red(x, 49)), (255, 255), "image, column {x}");
                assert_eq!((red(x, 50), red(x, 63)), (0, 0), "bar below, column {x}");
            }
        });
    }
}
