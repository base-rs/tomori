// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A recording in progress: the encoder child, the threads that write the video and the sound to it, and finishing the file. See `design/spectator.md`.
//!
//! The recording ends the same way whichever way it was asked to, by the window closing, a signal, the server going away or the encoder failing: the render loop stops, [`Session::finish`] says when the recording ended, the writers write the slots and the silence up to that time and close their pipes, and ffmpeg, seeing the end of its inputs, writes the file's index and exits. Only then is the file reported. A spectator that is killed outright closes the pipes too, and ffmpeg finishes the file by itself.
use super::{
    Origin, Recording, Stop,
    audio::{self, AudioTap, Tee},
    clock::FrameClock,
    ffmpeg::{self, Ffmpeg, Platform},
    options::RecordOptions,
    process::{self, Said},
    signals,
    video::{self, Ending, FrameSender},
};
use anyhow::{Context, Result, bail};
use std::{
    fmt,
    path::PathBuf,
    process::Child,
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tomori_client::speaker::Playback;

/// How long finishing waits.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// For the writers to write the last of the video and the sound to an encoder that is behind, before it is ended.
    pub drain: Duration,
    /// For the encoder to write the file's index once it has everything, which for a long recording copies the file.
    pub finalise: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            drain: Duration::from_secs(15),
            finalise: Duration::from_secs(120),
        }
    }
}

struct Parts {
    child: Child,
    video: JoinHandle<video::Totals>,
    audio: JoinHandle<audio::Totals>,
    said: Said,
    reader: JoinHandle<()>,
}

/// How long the encoder's last messages are waited for once it has ended: a process it left behind may hold them open.
const MESSAGES: Duration = Duration::from_secs(2);

/// Puts the signals back as they were when it goes.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        signals::release();
    }
}

/// Whether the encoder finished the file at `path`, with `status` and having said `said`. A zero status does. So does 255 with nothing said at the level it is run at, errors, and a file: that is how ffmpeg ends when it is sent SIGTERM, as by a supervisor that ends the whole group, after it has written the file's index.
fn finished(status: std::process::ExitStatus, said: &str, path: &std::path::Path) -> bool {
    status.success()
        || (status.code() == Some(255)
            && said.is_empty()
            && std::fs::metadata(path).is_ok_and(|file| file.len() > 0))
}

/// End an encoder of a recording that could not start.
fn abandon(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A recording that is running.
pub struct Session {
    options: RecordOptions,
    clock: FrameClock,
    origin: Origin,
    stop: Stop,
    ending: Arc<Ending>,
    frames: FrameSender,
    tap: AudioTap,
    limits: Limits,
    parts: Option<Parts>,
}

/// What a finished recording holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub path: PathBuf,
    /// The length of the video, and so of the file.
    pub duration: Duration,
    pub fps: u32,
    /// Frames in the file.
    pub frames: u64,
    /// Of those, frames that repeat the one before because none newer was drawn in time.
    pub duplicated: u64,
    /// Frames drawn that were not used: superseded within their slot, not read back from the GPU, or dropped because the encoder was too far behind.
    pub dropped: u64,
    /// Seconds of the sound that are silence put in where the player rendered none, apart from the start before it first rendered.
    pub silence: Duration,
    /// Blocks of the sound dropped because the writer was too far behind.
    pub lost_sound: u64,
    /// The size of the file, once written.
    pub bytes: u64,
    /// Why the recording ended.
    pub reason: Option<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = format!(
            "Recorded {}: {:.2} s, {} frames at {} fps ({} repeated, {} dropped), {:.1} MB",
            self.path.display(),
            self.duration.as_secs_f64(),
            self.frames,
            self.fps,
            self.duplicated,
            self.dropped,
            self.bytes as f64 / 1_000_000.0
        );
        if self.lost_sound > 0 {
            text.push_str(&format!(", {} blocks of sound dropped", self.lost_sound));
        }
        if let Some(reason) = &self.reason {
            text.push_str(&format!("; stopped: {reason}"));
        }
        formatter.write_str(&text)
    }
}

impl Session {
    /// Refuse what cannot be recorded, find ffmpeg and an encoder, and start recording to `options.path`. The signals that end a program end the recording instead, finishing its file.
    pub fn start(options: &RecordOptions) -> Result<Self> {
        let container = options.check()?;
        options.check_destination()?;
        let ffmpeg = Ffmpeg::detect("ffmpeg", options.codec, Platform::current())?;
        let command = ffmpeg::command(&ffmpeg, options, container);
        signals::catch();
        Self::launch(options, command, Stop::on_signals(), Limits::default())
    }

    /// Start recording with `command`, which reads the video from its standard input and the sound from descriptor 3, as ffmpeg's does.
    pub fn launch(
        options: &RecordOptions,
        command: std::process::Command,
        stop: Stop,
        limits: Limits,
    ) -> Result<Self> {
        Self::launch_at(Origin::default(), options, command, stop, limits)
    }

    /// [`launch`](Self::launch), for a recording whose time is `origin`'s, which a test sets by hand.
    fn launch_at(
        origin: Origin,
        options: &RecordOptions,
        command: std::process::Command,
        stop: Stop,
        limits: Limits,
    ) -> Result<Self> {
        let process::Launched {
            child,
            video,
            audio,
            said,
            reader,
        } = process::launch(command)?;
        let clock = FrameClock::new(options.fps);
        let ending = Arc::new(Ending::default());
        let frame_bytes = options.size.0 as usize * options.size.1 as usize * 4;
        let (frames, drawn) = FrameSender::new(origin.clone(), frame_bytes);
        let (tap, blocks) = AudioTap::new(origin.clone());
        let video = match video::write(
            video,
            drawn,
            origin.clone(),
            clock,
            ending.clone(),
            stop.clone(),
            frame_bytes,
        ) {
            Ok(video) => video,
            Err(error) => {
                abandon(child);
                return Err(error).context("starting the thread that writes the video");
            }
        };
        let audio = match audio::write(
            audio,
            blocks,
            origin.clone(),
            clock,
            ending.clone(),
            stop.clone(),
        ) {
            Ok(audio) => audio,
            Err(error) => {
                // The writer that started ends, at once, with nothing written.
                ending.end(Duration::ZERO);
                let _ = video.join();
                abandon(child);
                return Err(error).context("starting the thread that writes the sound");
            }
        };
        Ok(Self {
            options: options.clone(),
            clock,
            origin,
            stop,
            ending,
            frames,
            tap,
            limits,
            parts: Some(Parts {
                child,
                video,
                audio,
                said,
                reader,
            }),
        })
    }

    /// What the render loop needs of this recording.
    pub fn recording(&self) -> Recording {
        Recording {
            size: self.options.size,
            fps: self.options.fps,
            origin: self.origin.clone(),
            frames: self.frames.clone(),
            stop: self.stop.clone(),
        }
    }

    /// The playback for the spectator's player: `speaker` as well, unless the recording is not to play to it, with the recording tapped onto the player.
    pub fn playback(&self, speaker: Box<dyn Playback>) -> Box<dyn Playback> {
        let speaker = self.options.playback.then_some(speaker);
        Box::new(Tee::new(speaker, self.tap.clone()))
    }

    /// Whether the recording is to end.
    pub fn stop(&self) -> &Stop {
        &self.stop
    }

    /// End the recording now: write what is left of the video and the sound, close the encoder's inputs, wait for it to write the file's index, and report the file. The file is written whether the recording was asked to end or ends here.
    pub fn finish(mut self) -> Result<Report> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<Report> {
        let Some(Parts {
            mut child,
            video,
            audio,
            said,
            reader,
        }) = self.parts.take()
        else {
            bail!("the recording has finished already");
        };
        // A second Ctrl+C while the file is finished ends the program at once, the encoder, in a group of its own, going on to write the file; the signals are as they were once it is.
        let _restore = Restore;
        let end = self.origin.elapsed();
        self.tap.close();
        self.ending.end(end);
        let deadline = Instant::now() + self.limits.drain;
        while !(video.is_finished() && audio.is_finished()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if !(video.is_finished() && audio.is_finished()) {
            log::warn!(
                "Recording: the encoder did not take the last of the recording in time; ending it"
            );
            let _ = child.kill();
        }
        let video = video.join().expect("the video writer does not panic");
        let audio = audio.join().expect("the sound writer does not panic");
        // The writers have closed the encoder's inputs: it writes the file's index and exits.
        let deadline = Instant::now() + self.limits.finalise;
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .expect("waiting for a child this process started")
            {
                break status;
            }
            if Instant::now() >= deadline {
                // Its messages are not waited for: they end with it, however long that takes.
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "ffmpeg did not finish writing {} in {} s, so the file may not play",
                    self.options.path.display(),
                    self.limits.finalise.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        process::join_within(reader, MESSAGES);
        if !finished(status, &said.summary(), &self.options.path) {
            bail!(
                "ffmpeg failed with {status} and {} may not play: {}",
                self.options.path.display(),
                said.summary()
            );
        }
        if let Some(error) = video.error.or(audio.error) {
            bail!("{error}: {}", said.summary());
        }
        let counts = video.counts;
        let report = Report {
            path: self.options.path.clone(),
            duration: self.clock.timestamp(counts.written),
            fps: self.options.fps,
            frames: counts.written,
            duplicated: counts.duplicated,
            dropped: counts.superseded + self.frames.lost(),
            silence: Duration::from_secs_f64(audio.padded as f64 / f64::from(super::SAMPLE_RATE)),
            lost_sound: self.tap.lost(),
            bytes: std::fs::metadata(&self.options.path).map_or(0, |metadata| metadata.len()),
            reason: self.stop.reason(),
        };
        log::info!("{report}");
        Ok(report)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.parts.is_some()
            && let Err(error) = self.shutdown()
        {
            log::error!("Recording: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests;
