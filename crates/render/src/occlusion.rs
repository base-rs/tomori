// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What is nearer the eye is drawn in front, whatever order it is drawn in: see `design/rendering.md`.
//!
//! Panels, labels and signs are translucent, so they are drawn with the depth test and without writing depth, each blended
//! over what is behind it. On its own, that leaves which of two of them is in front to the order they are drawn in. So
//! once a surface is drawn, the backing behind its text, where it is opaque enough to hide what is behind it, is drawn
//! again into depth alone, with an [`occluder`] pipeline: whatever is drawn after it and behind it is hidden, as the
//! scene's opaque bodies hide it, and whatever is drawn after it and in front of it still covers it. Its depth is written
//! a little behind it, by [`TIE`], so that between surfaces about as far away as each other, such as a panel carried over
//! a wall's slot and the panel in that slot, the order the frame step draws them in still decides. A surface too faint
//! to hide what is behind it, such as a wall's translucent board, a slot's outline or a fading label, writes no depth, so
//! that nothing vanishes behind what can barely be seen. Arrival's labels, the hand menu's in a space, are drawn last,
//! so nothing after them needs their depth, and they write none.

/// How opaque a backing must be, from 0 to 1, to hide what is behind it: one that shows less than two fifths of what is
/// behind it.
pub const OCCLUDING: f32 = 0.6;

/// How far behind a backing its depth is written, in the depth buffer's units, from 0 at the near plane to 1 at the far.
/// With every client's near plane at 5 cm and far plane at 50 m, a surface drawn later and less than about
/// `TIE * distance² / 0.05` metres behind it, about 2% of its distance for each metre of it, is still drawn over it: 4 mm
/// at arm's length, 2 cm at 1 m, and 11 cm at 2.3 m, as far as the Tomori wall, enough for a panel carried in one of its
/// slots and the hint below it, which faces the eyes and so leans back under the panel seen from above. Further behind,
/// it is hidden.
pub const TIE: f32 = 1e-3;

/// Whether a backing of `alpha` hides what is behind it.
pub fn occludes(alpha: f32) -> bool {
    alpha >= OCCLUDING
}

/// A pipeline that draws `vertex`'s triangles into depth alone, [`TIE`] behind where they are, depth tested, with no
/// colour, within a pass of `format` and `samples`: the backings of surfaces already drawn. `shader` has the vertex entry
/// point `vs_occlude`, which moves its position back by its override `TIE`, and the fragment entry point `fs_occlude`,
/// whose colour is not written.
pub fn occluder(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex: wgpu::VertexBufferLayout<'_>,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_occlude"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[("TIE", TIE as f64)],
                ..Default::default()
            },
            buffers: &[Some(vertex)],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_occlude"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backings_hide_what_is_behind_them_only_when_opaque_enough() {
        // A panel's face, the label on the ray, the hint, the pill and a worker's sign hide what is behind them.
        for alpha in [1.0, 0.95, 0.88, 0.82, OCCLUDING] {
            assert!(occludes(alpha), "{alpha}");
        }
        // A wall's board, a curved wall's slots, a slot's outline and a label fading out do not.
        for alpha in [0.5, 0.45, 0.16, 0.0] {
            assert!(!occludes(alpha), "{alpha}");
        }
    }
}
