// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The profiler's summary, from frames timed on the shared test device and from frames that could not be.
use super::*;

#[test]
fn a_profiler_without_timestamps_keeps_only_what_was_drawn() {
    let mut profiler = Profiler::untimed();
    assert!(!profiler.times());
    assert_eq!(profiler.summary(), None);
    assert_eq!(profiler.begin(2), None);
    assert!(profiler.pass(None, 0).is_none());
    for drawn in [10, 20] {
        profiler.tally(Tally {
            drawn,
            culled: 5,
            draws: 3,
            triangles: drawn * 12,
        });
    }
    assert_eq!(
        profiler.summary().unwrap(),
        "GPU over 2 frames, no timestamps; an eye: drawn 15.0/20/20, culled 5.0/5/5, draws 3.0/3/3, triangles 180.0/240/240"
    );
    assert_eq!(profiler.summary(), None);
}

/// One frame of `eyes` empty passes into a small target, timed by `profiler` if it has a readback free, marking each
/// layer of each pass; then submitted, and waited for.
fn frame(gpu: &crate::test_gpu::Gpu, profiler: &mut Profiler, eyes: usize) -> Option<Frame> {
    let target = gpu
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Profiler test target"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let frame = profiler.begin(eyes);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for eye in 0..eyes {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Profiler test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: profiler.pass(frame, eye),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for layer in 0..LAYERS.len() - 1 {
            profiler.mark(&mut pass, frame, eye, layer);
        }
    }
    profiler.resolve(&mut encoder, frame);
    gpu.queue.submit([encoder.finish()]);
    profiler.submitted(frame);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    frame
}

#[test]
fn a_profiler_times_each_eye_and_each_layer_where_the_device_has_timestamps() {
    crate::test_gpu::with(|gpu| {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        for within in [true, false] {
            let mut profiler = Profiler::new(&gpu.device, &gpu.queue, within);
            if !profiler.times() {
                // A device without timestamps keeps only what was drawn.
                assert!(
                    !gpu.device
                        .features()
                        .contains(wgpu::Features::TIMESTAMP_QUERY)
                );
                return;
            }
            // Unmarked, as on the headset, each eye writes only its pass's start and end, and only those are resolved:
            // Vulkan waits for every query a resolve copies, and one never written would never be ready.
            let marked = within
                && gpu
                    .device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
            let queries = profiler.queries.as_ref().unwrap();
            assert_eq!(queries.marks, if marked { MARKS } else { 2 });
            assert_eq!(queries.set.count(), IN_FLIGHT as u32 * 2 * queries.marks);
            for _ in 0..3 {
                assert!(frame(&gpu, &mut profiler, 2).is_some());
                profiler.tally(Tally::default());
            }
            // Read back as the next frame begins.
            let next = profiler.begin(2);
            assert!(next.is_some());
            let summary = profiler.summary().unwrap();
            assert!(
                summary.starts_with("GPU over 3 frames, 3 timed, ms mean/p95/max: eyes "),
                "{summary}"
            );
            assert!(summary.contains(", left ") && summary.contains(", right "));
            assert_eq!(
                summary.contains(", scenery ") && summary.contains(", labels "),
                within
                    && gpu
                        .device
                        .features()
                        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
            );
            assert!(summary.ends_with(
                "; an eye: drawn 0.0/0/0, culled 0.0/0/0, draws 0.0/0/0, triangles 0.0/0/0"
            ));
            // A frame not timed writes no timestamps, and a frame that finds every readback still on its way is not
            // timed.
            assert!(profiler.pass(None, 0).is_none());
            let busy: Vec<Option<Frame>> = (0..IN_FLIGHT).map(|_| profiler.begin(1)).collect();
            assert_eq!(busy.iter().flatten().count(), IN_FLIGHT - 1);
            assert_eq!(busy[IN_FLIGHT - 1], None);
            assert!(frame(&gpu, &mut profiler, 1).is_none());
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
