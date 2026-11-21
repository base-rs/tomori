// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The recording's render target and its GPU readback: the spectator renders into a texture of the recording's size, which is copied to a buffer, mapped without waiting, and handed to the video writer with the time it was drawn at. See `design/spectator.md`.
//!
//! Several buffers are in flight at once, so that the GPU's copy of one frame overlaps the drawing of the next. If every buffer is still being read when a frame is drawn, that frame is drawn for the preview and not recorded, and counted; the file repeats the frame before.
use super::{FORMAT, Recording};
use crate::letterbox::Letterbox;
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

/// The longest finishing waits for the frames in flight to be read back.
const FINISH_WAIT: Duration = Duration::from_secs(5);

/// How many readbacks may be in flight at once.
const IN_FLIGHT: usize = 3;

struct Readback {
    buffer: wgpu::Buffer,
    /// The channel the buffer's mapping reports on, and when the frame in it was drawn, while it is read back.
    pending: Option<(Receiver<Result<(), wgpu::BufferAsyncError>>, Instant)>,
}

pub struct Capture {
    texture: wgpu::Texture,
    /// What the spectator draws into: the recording's size, in [`FORMAT`].
    pub view: wgpu::TextureView,
    letterbox: Option<Letterbox>,
    device: wgpu::Device,
    readbacks: Vec<Readback>,
    recording: Recording,
    row: usize,
    padded_row: usize,
}

impl Capture {
    /// A target for `recording`, with a letterboxed preview for a window's `surface_format` if there is one.
    pub fn new(
        device: &wgpu::Device,
        surface_format: Option<wgpu::TextureFormat>,
        recording: Recording,
    ) -> Self {
        let (width, height) = recording.size;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spectator recording"),
            size: extent(recording.size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let row = width as usize * 4;
        let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
        let readbacks = (0..IN_FLIGHT)
            .map(|_| Readback {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("spectator recording readback"),
                    size: (padded_row * height as usize) as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                pending: None,
            })
            .collect();
        log::info!("Recording: RGBA {width}x{height} at {} fps", recording.fps);
        Self {
            texture,
            view,
            letterbox: surface_format.map(|format| Letterbox::new(device, format)),
            device: device.clone(),
            readbacks,
            recording,
            row,
            padded_row,
        }
    }

    /// The size of the recording, the aspect of what the spectator draws.
    pub fn size(&self) -> (u32, u32) {
        self.recording.size
    }

    /// Copy the latest frame into a window's `target`, as its preview: whole, as large as it fits and centred, with black bars beside or above and below it, as the PipeWire source's preview shows its output.
    ///
    /// # Panics
    ///
    /// If the capture was made without a window's surface format.
    pub fn preview(&self, queue: &wgpu::Queue, target: &wgpu::TextureView) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("spectator recording preview"),
            });
        self.letterbox
            .as_ref()
            .expect("a preview, for a window's surface format")
            .draw(&self.device, &mut encoder, &self.view, target);
        queue.submit([encoder.finish()]);
    }

    /// Read back the frame just drawn into [`Self::view`], at `drawn`, and hand on those whose readback has finished. Does not wait.
    pub fn capture(&mut self, queue: &wgpu::Queue, drawn: Instant) {
        // The first frame drawn begins the recording.
        self.recording.origin.begin(drawn);
        // Polling without waiting reports no error: there is no submission to wait for, nor a timeout.
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.collect();
        let Some(readback) = self
            .readbacks
            .iter_mut()
            .find(|readback| readback.pending.is_none())
        else {
            self.recording.frames.lose();
            return;
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("spectator recording readback"),
            });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row as u32),
                    rows_per_image: Some(self.recording.size.1),
                },
            },
            extent(self.recording.size),
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        readback
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        readback.pending = Some((receiver, drawn));
    }

    /// Hand on every readback that has finished, oldest frame first.
    fn collect(&mut self) {
        let mut finished = Vec::new();
        for (index, readback) in self.readbacks.iter().enumerate() {
            let Some((receiver, drawn)) = &readback.pending else {
                continue;
            };
            match receiver.try_recv() {
                Ok(result) => finished.push((*drawn, index, result)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    unreachable!("wgpu calls every mapping callback once")
                }
            }
        }
        finished.sort_by_key(|(drawn, ..)| *drawn);
        for (drawn, index, result) in finished {
            let readback = &mut self.readbacks[index];
            readback.pending = None;
            if let Err(error) = result {
                self.recording
                    .stop
                    .request(format!("the GPU readback failed: {error}"));
                continue;
            }
            let frame = {
                let mapped = readback
                    .buffer
                    .slice(..)
                    .get_mapped_range()
                    .expect("the readback buffer is mapped once its callback succeeds");
                let mut frame = self.recording.frames.buffer();
                crate::pipewire_video::pack_rows_into(
                    &mut frame,
                    &mapped,
                    self.row,
                    self.padded_row,
                    self.recording.size.1 as usize,
                );
                frame
            };
            readback.buffer.unmap();
            self.recording.frames.send(drawn, frame);
        }
    }

    /// Wait for the frames still being read back and hand them on: the recording is ending. It waits for the GPU no longer than [`FINISH_WAIT`], so that one that has hung cannot keep the spectator from ending.
    pub fn finish(&mut self) {
        self.finish_within(FINISH_WAIT);
    }

    fn finish_within(&mut self, wait: Duration) {
        let deadline = Instant::now() + wait;
        loop {
            let _ = self.device.poll(wgpu::PollType::Poll);
            self.collect();
            if self
                .readbacks
                .iter()
                .all(|readback| readback.pending.is_none())
            {
                return;
            }
            if Instant::now() >= deadline {
                log::warn!(
                    "Recording: the GPU did not finish reading back the last frames in {} s; they are left out",
                    wait.as_secs_f32()
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.finish();
        let lost = self.recording.frames.lost();
        if lost > 0 {
            log::warn!(
                "Recording: {lost} frames were drawn but not recorded, because the GPU or the encoder was too slow; the video repeats the frame before"
            );
        }
    }
}

fn extent((width, height): (u32, u32)) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{Origin, Stop, video::FrameSender};
    use std::time::Duration;
    use tomori_render::test_gpu;

    fn recording(
        size: (u32, u32),
    ) -> (
        Recording,
        mpsc::Receiver<(Duration, super::super::video::Frame)>,
    ) {
        let origin = Origin::default();
        let (frames, drawn) = FrameSender::new(origin.clone(), 70 * 64 * 4);
        let recording = Recording {
            size,
            fps: 30,
            origin,
            frames,
            stop: Stop::default(),
        };
        (recording, drawn)
    }

    /// Clear `view` to `colour`.
    fn clear(gpu: &test_gpu::Gpu, view: &wgpu::TextureView, colour: wgpu::Color) {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(colour),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        gpu.queue.submit([encoder.finish()]);
    }

    #[test]
    fn frames_are_read_back_tightly_packed_in_the_order_they_were_drawn() {
        test_gpu::with(|gpu| {
            // 70 pixels of 4 bytes is not a multiple of the 256 bytes a row is copied in.
            let (recording, drawn) = recording((70, 64));
            let start = Instant::now();
            let mut capture = Capture::new(&gpu.device, None, recording);
            assert_eq!(capture.size(), (70, 64));
            for (index, colour) in [wgpu::Color::RED, wgpu::Color::GREEN, wgpu::Color::BLUE]
                .into_iter()
                .enumerate()
            {
                clear(&gpu, &capture.view, colour);
                capture.capture(&gpu.queue, start + Duration::from_millis(10 * index as u64));
            }
            // The first frame drawn begins the recording.
            assert!(capture.recording.origin.begun());
            capture.finish();
            let frames: Vec<_> = drawn.try_iter().collect();
            assert_eq!(frames.len(), 3);
            for ((at, frame), (index, pixel)) in frames.iter().zip(
                [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
                    .into_iter()
                    .enumerate(),
            ) {
                assert_eq!(*at, Duration::from_millis(10 * index as u64));
                assert_eq!(frame.len(), 70 * 64 * 4);
                assert!(
                    frame.as_chunks::<4>().0.iter().all(|each| *each == pixel),
                    "{index}"
                );
            }
            assert_eq!(capture.recording.frames.lost(), 0);
            assert!(!capture.recording.stop.requested());
        });
    }

    #[test]
    fn a_frame_drawn_while_every_buffer_is_in_flight_is_counted_and_not_recorded() {
        test_gpu::with(|gpu| {
            log::set_max_level(log::LevelFilter::Trace);
            let (recording, drawn) = recording((64, 64));
            let mut capture = Capture::new(&gpu.device, None, recording);
            // Readbacks that never finish.
            let held: Vec<_> = capture
                .readbacks
                .iter_mut()
                .map(|readback| {
                    let (sender, receiver) = mpsc::channel();
                    readback.pending = Some((receiver, Instant::now()));
                    sender
                })
                .collect();
            capture.capture(&gpu.queue, Instant::now());
            assert_eq!(capture.recording.frames.lost(), 1);
            assert!(drawn.try_recv().is_err());
            // A readback that fails ends the recording, and sends nothing.
            // Two together are taken oldest first, and each is told of.
            held[1].send(Err(wgpu::BufferAsyncError)).unwrap();
            held[0].send(Err(wgpu::BufferAsyncError)).unwrap();
            capture.collect();
            let reason = capture.recording.stop.reason().unwrap();
            assert!(reason.starts_with("the GPU readback failed"), "{reason}");
            assert!(drawn.try_recv().is_err());
            assert!(capture.readbacks[0].pending.is_none());
            // Finishing does not wait for ever for readbacks that never come.
            let started = Instant::now();
            capture.finish_within(Duration::from_millis(30));
            assert!(started.elapsed() >= Duration::from_millis(30));
            assert!(
                capture
                    .readbacks
                    .iter()
                    .any(|readback| readback.pending.is_some())
            );
            for readback in &mut capture.readbacks {
                readback.pending = None;
            }
        });
    }

    #[test]
    fn the_preview_shows_the_recording_whole_in_a_window_of_another_shape() {
        test_gpu::with(|gpu| {
            let (recording, _drawn) = recording((64, 64));
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let capture = Capture::new(&gpu.device, Some(format), recording);
            clear(&gpu, &capture.view, wgpu::Color::WHITE);
            let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 128,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 128 * 64 * 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            capture.preview(&gpu.queue, &target.create_view(&Default::default()));
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(128 * 4),
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
            let red = |x: usize| pixels[(32 * 128 + x) * 4];
            // The square, in the middle of the wide window, between black bars.
            assert_eq!((red(0), red(31)), (0, 0));
            assert_eq!((red(32), red(95)), (255, 255));
            assert_eq!((red(96), red(127)), (0, 0));
        });
    }

    #[test]
    #[should_panic(expected = "a preview, for a window's surface format")]
    fn a_capture_without_a_window_has_no_preview() {
        test_gpu::with(|gpu| {
            let (recording, _drawn) = recording((64, 64));
            let capture = Capture::new(&gpu.device, None, recording);
            let view = capture.view.clone();
            capture.preview(&gpu.queue, &view);
        });
    }
}
