// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Experimental CPU-readback video output. The GStreamer child provides the
//! PipeWire node; neither the render loop nor the GPU waits for a consumer.
use anyhow::{Context, Result, bail};
use std::{
    io::Write,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread::{self, JoinHandle},
};

pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
pub const FPS: u32 = 30;
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const ROW_BYTES: u32 = WIDTH * 4;
const PADDED_ROW_BYTES: u32 =
    ROW_BYTES.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
const FRAME_BYTES: usize = (ROW_BYTES * HEIGHT) as usize;

pub struct Video {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    blitter: Option<wgpu::util::TextureBlitter>,
    buffer: wgpu::Buffer,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    producer: Producer,
    failed: bool,
}

impl Video {
    /// Frames for `producer` to publish, drawn into [`Self::view`], with a preview blitter for a window's `surface_format` if there is one.
    pub fn new(
        device: &wgpu::Device,
        surface_format: Option<wgpu::TextureFormat>,
        producer: Producer,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PipeWire spectator 1080p"),
            size: extent(),
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
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PipeWire spectator readback"),
            size: u64::from(PADDED_ROW_BYTES * HEIGHT),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("PipeWire prototype: tomori-spectator, RGBA {WIDTH}x{HEIGHT} up to {FPS} fps");
        Self {
            texture,
            view,
            blitter: surface_format.map(|format| wgpu::util::TextureBlitter::new(device, format)),
            buffer,
            pending: None,
            producer,
            failed: false,
        }
    }

    /// Copy the latest frame into a window's `target`, scaled to fit, as its preview.
    ///
    /// # Panics
    ///
    /// If the video was made without a window's surface format.
    pub fn preview(&self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("spectator preview"),
        });
        self.blitter
            .as_ref()
            .expect("a preview blitter, for a window's surface format")
            .copy(device, &mut encoder, &self.view, target);
        queue.submit([encoder.finish()]);
    }

    /// Poll the previous readback and enqueue the latest texture if possible.
    /// At most one readback and one queued CPU frame exist at any time.
    pub fn capture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<()> {
        if self.failed {
            return Ok(());
        }
        if let Err(error) = self.try_capture(device, queue) {
            self.failed = true;
            self.producer.stop();
            return Err(error);
        }
        Ok(())
    }

    fn try_capture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<()> {
        if let Some(stopped) = stopped(self.producer.child.try_wait()) {
            bail!(stopped);
        }
        // Polling without waiting reports no error: there is no submission to wait for, nor a timeout.
        let _ = device.poll(wgpu::PollType::Poll);
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok(Ok(())) => {
                    let mapped = self
                        .buffer
                        .slice(..)
                        .get_mapped_range()
                        .expect("the readback buffer is mapped once its callback succeeds");
                    let frame = pack_rows(
                        &mapped,
                        ROW_BYTES as usize,
                        PADDED_ROW_BYTES as usize,
                        HEIGHT as usize,
                    );
                    drop(mapped);
                    self.buffer.unmap();
                    self.pending = None;
                    self.producer.send(frame)?;
                }
                Ok(Err(error)) => bail!("GPU readback failed: {error}"),
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    unreachable!("wgpu calls every mapping callback once")
                }
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("PipeWire readback"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(PADDED_ROW_BYTES),
                    rows_per_image: Some(HEIGHT),
                },
            },
            extent(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.pending = Some(rx);
        Ok(())
    }
}

/// Why GStreamer has stopped, from what waiting for it without blocking says, or `None` while it runs.
fn stopped(status: std::io::Result<Option<std::process::ExitStatus>>) -> Option<String> {
    match status {
        Ok(None) => None,
        Ok(Some(status)) => Some(format!("GStreamer exited with {status}")),
        Err(error) => Some(format!("Cannot check GStreamer: {error}")),
    }
}

fn extent() -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    }
}

fn pack_rows(bytes: &[u8], row: usize, stride: usize, height: usize) -> Vec<u8> {
    assert!(stride >= row && bytes.len() >= stride * height);
    if stride == row {
        return bytes[..row * height].to_vec();
    }
    let mut packed = Vec::with_capacity(row * height);
    for line in bytes.chunks_exact(stride).take(height) {
        packed.extend_from_slice(&line[..row]);
    }
    packed
}

/// The GStreamer process that publishes the frames as a PipeWire node, and the thread that writes them to it.
pub struct Producer {
    child: Child,
    sender: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<()>>,
}

impl Producer {
    /// Start `gst-launch-1.0` with PipeWire's sink.
    pub fn start() -> Result<Self> {
        let mut command = gstreamer_command();
        // A killed spectator cannot run Drop, so the PipeWire child must also end with it, including when a consumer is absent.
        crate::child_process::end_with_parent(&mut command);
        Self::spawn(command)
    }

    /// Start `command`, which reads the frames from its standard input, as [`gstreamer_command`] does.
    pub fn spawn(mut command: Command) -> Result<Self> {
        let child = command
            .spawn()
            .context("starting gst-launch-1.0 (install GStreamer and its PipeWire plugin)")?;
        Self::from_child(child)
    }

    fn from_child(mut child: Child) -> Result<Self> {
        let mut stdin = child.stdin.take().context("opening GStreamer stdin")?;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(1);
        let writer = thread::spawn(move || {
            for frame in rx {
                if let Err(error) = stdin.write_all(&frame) {
                    log::warn!("PipeWire video writer stopped: {error}");
                    break;
                }
            }
        });
        Ok(Self {
            child,
            sender: Some(tx),
            writer: Some(writer),
        })
    }

    fn send(&mut self, frame: Vec<u8>) -> Result<()> {
        if frame.len() != FRAME_BYTES {
            bail!("invalid video frame size: {}", frame.len());
        }
        match self
            .sender
            .as_ref()
            .context("producer already stopped")?
            .try_send(frame)
        {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()), // Drop rather than block rendering.
            Err(TrySendError::Disconnected(_)) => bail!(self.disconnected()),
        }
    }

    /// Why the writer could not write: GStreamer stopped reading, or is exiting, which closes its input before it can be waited for. Killing it ends one that only stopped reading, and is ignored by one already exiting, so that waiting gives its own status.
    fn disconnected(&mut self) -> String {
        use std::os::unix::process::ExitStatusExt;
        const SIGKILL: i32 = 9;
        let _ = self.child.kill();
        match self.child.wait() {
            Ok(status) if status.signal() == Some(SIGKILL) => {
                "GStreamer video writer disconnected".to_owned()
            }
            status => stopped(status.map(Some)).expect("a waited-for process has stopped"),
        }
    }

    fn stop(&mut self) {
        // Kill first: a writer blocked in write_all must be released before join.
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn gstreamer_command() -> Command {
    let mut command = Command::new("gst-launch-1.0");
    command
        .args([
            "-q", "fdsrc", "fd=0", "blocksize=65536", "!", "rawvideoparse", "format=rgba",
        ])
        .arg(format!("width={WIDTH}"))
        .arg(format!("height={HEIGHT}"))
        .arg(format!("framerate={FPS}/1"))
        .args([
            "!",
            "pipewiresink", "mode=provide", "use-bufferpool=false", "sync=false",
            "stream-properties=props,media.class=Video/Source,media.type=Video,media.role=Production,node.name=tomori-spectator,node.description=Tomori-Spectator",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    command
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_environment_sky::test_gpu;

    #[test]
    fn frame_layout_is_tightly_packed_rgba_1080p() {
        assert_eq!((WIDTH, HEIGHT), (1920, 1080));
        assert_eq!(ROW_BYTES, 7_680);
        assert_eq!(PADDED_ROW_BYTES, ROW_BYTES);
        assert_eq!(FRAME_BYTES, 8_294_400);
        assert_eq!(FORMAT, wgpu::TextureFormat::Rgba8UnormSrgb);
    }

    fn producer(program: &str, arguments: &[&str]) -> Producer {
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::null());
        Producer::spawn(command).unwrap()
    }

    /// Capture `count` times, a little apart so that each readback completes, returning the first failure.
    fn capture(video: &mut Video, gpu: &test_gpu::Gpu, count: usize) -> Result<()> {
        for _ in 0..count {
            video.capture(&gpu.device, &gpu.queue)?;
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(())
    }

    #[test]
    fn frames_are_read_back_and_published() {
        test_gpu::with(|gpu| {
            let mut video = Video::new(&gpu.device, None, producer("cat", &[]));
            capture(&mut video, &gpu, 4).unwrap();
            assert!(!video.failed);
        });
    }

    #[test]
    fn publishing_stops_when_gstreamer_does() {
        test_gpu::with(|gpu| {
            let mut video = Video::new(&gpu.device, None, producer("true", &[]));
            std::thread::sleep(std::time::Duration::from_millis(100));
            let error = capture(&mut video, &gpu, 1).unwrap_err();
            assert!(error.to_string().starts_with("GStreamer exited with"));
            // Stopped, it publishes nothing more and reports nothing more.
            capture(&mut video, &gpu, 1).unwrap();
            assert!(video.failed);
        });
    }

    #[test]
    fn publishing_stops_when_gstreamer_stops_reading() {
        test_gpu::with(|gpu| {
            // GStreamer that closes its input but keeps running.
            let closes = producer("sh", &["-c", "exec 0<&-; exec sleep 10"]);
            let mut video = Video::new(&gpu.device, None, closes);
            let error = capture(&mut video, &gpu, 8).unwrap_err();
            assert_eq!(error.to_string(), "GStreamer video writer disconnected");
        });
    }

    #[test]
    fn a_writer_disconnected_by_gstreamer_exiting_says_how_it_exited() {
        let mut producer = producer("true", &[]);
        producer.child.wait().unwrap();
        // The writer stops at the first frame it cannot write.
        producer.send(vec![0; FRAME_BYTES]).unwrap();
        producer.writer.take().unwrap().join().unwrap();
        let error = producer.send(vec![0; FRAME_BYTES]).unwrap_err();
        assert_eq!(error.to_string(), "GStreamer exited with exit status: 0");
    }

    #[test]
    fn a_readback_still_on_its_way_is_waited_for_and_a_failed_one_stops_publishing() {
        test_gpu::with(|gpu| {
            let mut video = Video::new(&gpu.device, None, producer("cat", &[]));
            let (sender, pending) = mpsc::channel();
            video.pending = Some(pending);
            video.capture(&gpu.device, &gpu.queue).unwrap();
            sender.send(Err(wgpu::BufferAsyncError)).unwrap();
            let error = video.capture(&gpu.device, &gpu.queue).unwrap_err();
            assert!(error.to_string().starts_with("GPU readback failed"));
        });
    }

    #[test]
    fn gstreamer_that_cannot_be_started_or_written_to_is_reported() {
        let error = Producer::spawn(Command::new("/nonexistent/gst-launch-1.0"))
            .map(drop)
            .expect_err("nothing to start");
        assert!(error.to_string().starts_with("starting gst-launch-1.0"));
        let error = Producer::spawn(Command::new("true"))
            .map(drop)
            .expect_err("no input to write to");
        assert_eq!(error.to_string(), "opening GStreamer stdin");
    }

    #[cfg(unix)]
    #[test]
    fn gstreamer_is_stopped_when_it_exits_or_cannot_be_checked() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(stopped(Ok(None)), None);
        assert_eq!(
            stopped(Ok(Some(std::process::ExitStatus::from_raw(256)))).as_deref(),
            Some("GStreamer exited with exit status: 1")
        );
        assert_eq!(
            stopped(Err(std::io::Error::other("no child"))).as_deref(),
            Some("Cannot check GStreamer: no child")
        );
    }

    #[test]
    fn gstreamer_format_matches_the_render_target() {
        let command = gstreamer_command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(args.contains(&"format=rgba"));
        assert!(args.contains(&"width=1920"));
        assert!(args.contains(&"height=1080"));
        assert!(args.contains(&"framerate=30/1"));
    }

    #[test]
    fn removes_gpu_row_padding_without_reordering_channels() {
        assert_eq!(
            pack_rows(&[1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 99, 99], 4, 6, 2),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn bounded_channel_drops_when_full() {
        let (tx, _rx) = mpsc::sync_channel(1);
        assert!(tx.try_send(vec![1]).is_ok());
        assert_eq!(tx.try_send(vec![2]), Err(TrySendError::Full(vec![2])));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn stop_reaps_child_even_when_writer_cannot_finish_a_frame() {
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut producer = Producer::from_child(child).unwrap();
        assert!(producer.send(vec![0; 1]).is_err());
        producer.send(vec![0; FRAME_BYTES]).unwrap();
        producer.stop();
        assert!(producer.child.try_wait().unwrap().is_some());
        assert!(producer.sender.is_none());
        assert!(producer.writer.is_none());
        assert!(producer.send(vec![0; FRAME_BYTES]).is_err());
    }
}
