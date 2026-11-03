// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels of styled text in their own local space, drawn in the scene pass with depth testing and alpha blending, so that a hand or controller in front of one covers it.
//!
//! A [`Canvas`] is what a panel shows: rows of [`ui_text`](crate::ui_text) cells on a translucent backing, with a colour per span, row backgrounds, speaker bars, meters and shapes. [`Canvas::mesh`] turns it into quads in the panel's plane, in metres, centred on the panel's origin, with +X right and +Y up; it is pure, so that layouts are tested without a GPU. A [`Panel`] holds one canvas on the GPU and rebuilds its mesh only when the canvas changes, so that moving it costs one matrix per eye per frame.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Rgba, Row, glyph};

const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// A rectangle in font pixels from the canvas's top-left corner, inside its padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: Rgba,
}

/// What a panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    /// The width of the text, in cells; rows longer than this are cut.
    pub columns: usize,
    pub rows: Vec<Row>,
    /// The size of a font pixel, in metres.
    pub pixel: f32,
    /// Behind everything, usually translucent.
    pub background: Rgba,
    /// Font pixels around the text, horizontally and vertically.
    pub padding: (usize, usize),
    /// A frame around the panel, a font pixel wide.
    pub border: Option<Rgba>,
    /// Shapes drawn over the rows' backgrounds and under the text.
    pub shapes: Vec<Rect>,
}

impl Canvas {
    pub fn new(columns: usize, pixel: f32) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            pixel,
            background: [0.008, 0.018, 0.035, 0.85],
            padding: (6, 4),
            border: None,
            shapes: Vec::new(),
        }
    }

    /// The panel's size in font pixels.
    pub fn size_in_pixels(&self) -> (usize, usize) {
        (
            self.columns * CELL_WIDTH + 2 * self.padding.0,
            self.rows.len() * CELL_HEIGHT + 2 * self.padding.1,
        )
    }

    /// The panel's width and height in metres.
    #[cfg(test)]
    pub fn size(&self) -> (f32, f32) {
        let (width, height) = self.size_in_pixels();
        (width as f32 * self.pixel, height as f32 * self.pixel)
    }

    /// The text, a row per line, as the console prints it.
    #[cfg(test)]
    pub fn plain(&self) -> String {
        self.rows
            .iter()
            .map(Row::plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The quads that draw the canvas, centred on its origin.
    pub fn mesh(&self) -> Vec<Vertex> {
        let mut mesh = Mesh::new(self);
        let (width, height) = self.size_in_pixels();
        let (width, height) = (width as f32, height as f32);
        mesh.quad(0.0, 0.0, width, height, self.background);
        if let Some(border) = self.border {
            mesh.quad(0.0, 0.0, width, 1.0, border);
            mesh.quad(0.0, height - 1.0, width, 1.0, border);
            mesh.quad(0.0, 0.0, 1.0, height, border);
            mesh.quad(width - 1.0, 0.0, 1.0, height, border);
        }
        let (left, top) = (self.padding.0 as f32, self.padding.1 as f32);
        let text_width = (self.columns * CELL_WIDTH) as f32;
        for (index, row) in self.rows.iter().enumerate() {
            let y = top + (index * CELL_HEIGHT) as f32;
            if let Some(background) = row.background {
                mesh.quad(
                    left - 2.0,
                    y,
                    text_width + 4.0,
                    CELL_HEIGHT as f32,
                    background,
                );
            }
            // Runs of cells with the same background are one quad.
            let mut column = 0;
            let cells = &row.cells[..row.cells.len().min(self.columns)];
            while column < cells.len() {
                let background = cells[column].1.background;
                let start = column;
                while column < cells.len() && cells[column].1.background == background {
                    column += 1;
                }
                if let Some(background) = background {
                    mesh.quad(
                        left + (start * CELL_WIDTH) as f32,
                        y,
                        ((column - start) * CELL_WIDTH) as f32,
                        CELL_HEIGHT as f32,
                        background,
                    );
                }
            }
            if let Some(bar) = row.bar {
                mesh.quad(left + 1.0, y, 3.0, CELL_HEIGHT as f32, bar);
            }
            if let Some((fraction, color)) = row.meter {
                let fraction = fraction.clamp(0.0, 1.0);
                mesh.quad(
                    left,
                    y + CELL_HEIGHT as f32 - 1.5,
                    text_width * fraction,
                    1.5,
                    color,
                );
            }
        }
        for shape in &self.shapes {
            mesh.quad(
                left + shape.x,
                top + shape.y,
                shape.width,
                shape.height,
                shape.color,
            );
        }
        for (index, row) in self.rows.iter().enumerate() {
            let y = top + (index * CELL_HEIGHT) as f32 + 2.0;
            for (column, &(ch, style)) in row.cells.iter().take(self.columns).enumerate() {
                if ch == ' ' {
                    continue;
                }
                let x = left + (column * CELL_WIDTH) as f32;
                for (line, bits) in glyph(ch).iter().enumerate() {
                    let mut bits = u16::from(*bits);
                    if style.bold {
                        bits |= bits << 1;
                    }
                    // Lit pixels in a line are one quad.
                    let mut pixel = 0;
                    while pixel < 9 {
                        if bits & (1 << pixel) == 0 {
                            pixel += 1;
                            continue;
                        }
                        let start = pixel;
                        while pixel < 9 && bits & (1 << pixel) != 0 {
                            pixel += 1;
                        }
                        mesh.quad(
                            x + start as f32,
                            y + line as f32,
                            (pixel - start) as f32,
                            1.0,
                            style.color,
                        );
                    }
                }
            }
        }
        mesh.vertices
    }
}

/// Builds quads in font pixels from the top-left corner, placed in metres about the centre.
struct Mesh {
    vertices: Vec<Vertex>,
    pixel: f32,
    origin: (f32, f32),
}

impl Mesh {
    fn new(canvas: &Canvas) -> Self {
        let (width, height) = canvas.size_in_pixels();
        Self {
            vertices: Vec::new(),
            pixel: canvas.pixel,
            origin: (width as f32 / 2.0, height as f32 / 2.0),
        }
    }

    fn quad(&mut self, x: f32, y: f32, width: f32, height: f32, color: Rgba) {
        if width <= 0.0 || height <= 0.0 || color[3] <= 0.0 {
            return;
        }
        let place = |x: f32, y: f32| {
            [
                (x - self.origin.0) * self.pixel,
                (self.origin.1 - y) * self.pixel,
            ]
        };
        let (left, top) = (x, y);
        let (right, bottom) = (x + width, y + height);
        for position in [
            place(left, top),
            place(right, top),
            place(left, bottom),
            place(right, top),
            place(right, bottom),
            place(left, bottom),
        ] {
            self.vertices.push(Vertex { position, color });
        }
    }
}

/// The pipeline every panel is drawn with, in the scene pass: depth tested against the scene without writing depth, and alpha blended, so that panels are translucent and a nearer hand covers them.
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
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
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("panel pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &ATTRIBUTES,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
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
        Self { pipeline, layout }
    }

    pub fn set(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
    }
}

/// One panel's canvas on the GPU, with a transform for each eye.
pub struct Panel {
    vertices: wgpu::Buffer,
    capacity: usize,
    count: u32,
    cameras: [wgpu::Buffer; 2],
    bindings: [wgpu::BindGroup; 2],
    canvas: Option<Canvas>,
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
        Self {
            vertices,
            capacity,
            count: 0,
            cameras,
            bindings,
            canvas: None,
        }
    }

    /// Show `canvas`, rebuilding the mesh only if it changed.
    pub fn set(&mut self, queue: &wgpu::Queue, canvas: &Canvas) {
        if self.canvas.as_ref() == Some(canvas) {
            return;
        }
        let mut mesh = canvas.mesh();
        if mesh.len() > self.capacity {
            log::debug!(
                "A panel's mesh of {} vertices was cut to {}",
                mesh.len(),
                self.capacity
            );
            mesh.truncate(self.capacity - self.capacity % 6);
        }
        self.count = mesh.len() as u32;
        if !mesh.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&mesh));
        }
        self.canvas = Some(canvas.clone());
    }

    /// Write `eye`'s transform, from the panel's plane to clip space, before the pass that draws it.
    pub fn prepare(&self, queue: &wgpu::Queue, eye: usize, transform: Mat4) {
        queue.write_buffer(
            &self.cameras[eye],
            0,
            bytemuck::cast_slice(&transform.to_cols_array()),
        );
    }

    /// Draw for `eye`, with [`Pipeline::set`] already called on `pass`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        if self.count == 0 {
            return;
        }
        pass.set_bind_group(0, &self.bindings[eye], &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_text::{Span, Style, Wrap, wrap};

    #[test]
    fn the_mesh_stays_within_the_panel_and_is_centred() {
        let mut canvas = Canvas::new(10, 0.001);
        for line in wrap(
            &[Span::new(
                "Every row of this text fits within ten columns, wrapped.",
                Style::bold([1.0; 4]).on([0.2, 0.2, 0.2, 1.0]),
            )],
            10,
            Wrap::default(),
        ) {
            canvas
                .rows
                .push(Row::new(line).with_bar(Some([0.0, 1.0, 1.0, 1.0])));
        }
        canvas.rows[0].meter = Some((0.5, [1.0; 4]));
        canvas.border = Some([1.0, 0.6, 0.2, 1.0]);
        let (width, height) = canvas.size();
        let mesh = canvas.mesh();
        assert_eq!(mesh.len() % 6, 0);
        assert!(mesh.iter().all(|vertex| {
            vertex.position[0].abs() <= width / 2.0 + 1e-5
                && vertex.position[1].abs() <= height / 2.0 + 1e-5
        }));
        // The backing is the first quad, from corner to corner.
        assert_eq!(mesh[0].position, [-width / 2.0, height / 2.0]);
        assert_eq!(mesh[4].position, [width / 2.0, -height / 2.0]);
    }

    #[test]
    fn lit_pixels_in_a_line_are_merged_and_bold_is_wider() {
        let mut canvas = Canvas::new(1, 1.0);
        canvas.background = [0.0; 4];
        canvas
            .rows
            .push(Row::new(vec![('_', Style::plain([1.0; 4]))]));
        // The underscore is one line of lit pixels: one quad.
        assert_eq!(canvas.mesh().len(), 6);
        let quad = canvas.mesh();
        let plain = quad[1].position[0] - quad[0].position[0];
        canvas.rows[0].cells[0].1.bold = true;
        let quad = canvas.mesh();
        let bold = quad[1].position[0] - quad[0].position[0];
        assert_eq!(bold, plain + 1.0);
    }

    #[test]
    fn transparent_and_empty_parts_draw_nothing() {
        let mut canvas = Canvas::new(4, 1.0);
        canvas.background = [0.0; 4];
        canvas
            .rows
            .push(Row::new(vec![(' ', Style::plain([1.0; 4]))]));
        assert!(canvas.mesh().is_empty());
        assert_eq!(canvas.plain(), " ");
    }
}
