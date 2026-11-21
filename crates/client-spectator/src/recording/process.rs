// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Starting the encoder with its pipes: video on its standard input, audio on a descriptor of its own, and what it says on standard error kept for the report if it fails.
use super::{ffmpeg::AUDIO_DESCRIPTOR, system};
use anyhow::{Context, Result};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, PipeWriter},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, PoisonError},
    thread::{self, JoinHandle},
};

/// How many of the encoder's last lines are kept.
const KEPT: usize = 12;

/// What the encoder said last, to say why it failed.
#[derive(Clone, Default)]
pub struct Said(Arc<Mutex<VecDeque<String>>>);

impl Said {
    fn keep(&self, line: String) {
        let mut lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() == KEPT {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// The last lines, on one.
    pub fn summary(&self) -> String {
        let lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Wait up to `within` for the thread that reads the encoder's messages, and leave it if it has not ended, as when a process the encoder left behind holds them open.
pub fn join_within(reader: JoinHandle<()>, within: std::time::Duration) {
    let deadline = std::time::Instant::now() + within;
    while !reader.is_finished() && std::time::Instant::now() < deadline {
        thread::sleep(std::time::Duration::from_millis(5));
    }
    if reader.is_finished() {
        let _ = reader.join();
    }
}

/// The encoder, started, with the ends of its pipes this process writes to.
pub struct Launched {
    pub child: Child,
    pub video: ChildStdin,
    pub audio: PipeWriter,
    pub said: Said,
    /// The thread that reads its standard error, which ends when the encoder does.
    pub reader: JoinHandle<()>,
}

/// Start `command`, which reads video from its standard input and audio from the descriptor [`AUDIO_DESCRIPTOR`].
///
/// It runs in a process group of its own, so that Ctrl+C in the terminal reaches this process alone, which finishes the file, rather than ending the encoder before the last frames are written. It ends when its inputs do, finishing the file, even if this process is killed.
pub fn launch(mut command: Command) -> Result<Launched> {
    let (audio_reader, audio) = system::pipe().context("making the audio pipe")?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0);
    crate::child_process::pass_descriptor(&mut command, audio_reader.as_raw_fd(), AUDIO_DESCRIPTOR);
    crate::child_process::end_with_parent(&mut command);
    let mut child = command.spawn().context("starting ffmpeg")?;
    // Only the encoder reads the audio, so that it sees the end of it when this process closes its end.
    drop(audio_reader);
    let video = child.stdin.take().expect("the input was piped");
    let stderr = child.stderr.take().expect("the messages were piped");
    let said = Said::default();
    let kept = said.clone();
    let reader = match system::spawn("tomori-record-ffmpeg", move || {
        let mut messages = BufReader::new(stderr);
        let mut line = Vec::new();
        // Whatever bytes it says them in: a line that is not UTF-8 is read lossily, and the next is read.
        while messages
            .read_until(b'\n', &mut line)
            .is_ok_and(|read| read > 0)
        {
            let text = String::from_utf8_lossy(&line).trim_end().to_owned();
            log::warn!("ffmpeg: {text}");
            kept.keep(text);
            line.clear();
        }
    }) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("starting the thread that reads ffmpeg's messages");
        }
    };
    Ok(Launched {
        child,
        video,
        audio,
        said,
        reader,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn the_child_reads_video_from_standard_input_and_audio_from_descriptor_3() {
        let folder = std::env::temp_dir().join(format!("tomori-process-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (video, audio) = (folder.join("video"), folder.join("audio"));
        let script = format!(
            "cat <&3 > {} & cat > {}; wait",
            audio.display(),
            video.display()
        );
        let mut launched = launch(shell(&script)).unwrap();
        launched.video.write_all(b"frames").unwrap();
        launched.audio.write_all(b"samples").unwrap();
        // Closing both ends is what ends it.
        drop((launched.video, launched.audio));
        assert!(launched.child.wait().unwrap().success());
        launched.reader.join().unwrap();
        assert_eq!(std::fs::read(&video).unwrap(), b"frames");
        assert_eq!(std::fs::read(&audio).unwrap(), b"samples");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn what_the_child_says_is_kept_for_the_last_lines() {
        log::set_max_level(log::LevelFilter::Trace);
        let launched = launch(shell(
            "for n in 1 2 3 4 5 6 7 8 9 10 11 12 13 14; do echo line $n >&2; done; exit 4",
        ))
        .unwrap();
        let mut child = launched.child;
        assert_eq!(child.wait().unwrap().code(), Some(4));
        launched.reader.join().unwrap();
        let summary = launched.said.summary();
        assert!(summary.starts_with("line 3; line 4"), "{summary}");
        assert!(summary.ends_with("line 13; line 14"), "{summary}");
        assert_eq!(Said::default().summary(), "");
    }

    #[test]
    fn what_the_system_refuses_is_reported_and_leaves_no_child_behind() {
        system::refuse(system::PIPE);
        let error = launch(shell("exit 0")).map(drop).unwrap_err();
        assert_eq!(error.to_string(), "making the audio pipe");
        // The child that started is ended, not left running with nobody to read its messages.
        system::refuse("tomori-record-ffmpeg");
        let folder =
            std::env::temp_dir().join(format!("tomori-process-refused-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let marker = folder.join("ended");
        let error = launch(shell(&format!(
            "trap 'touch {}' TERM; sleep 30",
            marker.display()
        )))
        .map(drop)
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "starting the thread that reads ffmpeg's messages"
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn what_the_child_says_is_read_on_past_bytes_that_are_not_text() {
        let launched = launch(shell(
            "printf 'caf\\377 bad\\n' >&2; echo later >&2; exit 4",
        ))
        .unwrap();
        let mut child = launched.child;
        child.wait().unwrap();
        launched.reader.join().unwrap();
        assert_eq!(launched.said.summary(), "caf\u{fffd} bad; later");
    }

    #[test]
    fn the_messages_are_waited_for_only_so_long() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let reader = thread::spawn(move || {
            let _ = held.recv();
        });
        let started = std::time::Instant::now();
        join_within(reader, std::time::Duration::from_millis(30));
        assert!(started.elapsed() >= std::time::Duration::from_millis(30));
        // One that has ended is joined.
        let ended = thread::spawn(|| {});
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !ended.is_finished() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(1));
        }
        join_within(ended, std::time::Duration::from_secs(60));
        drop(release);
    }

    #[test]
    fn a_child_that_cannot_be_started_is_reported() {
        let error = launch(Command::new("/nonexistent/ffmpeg"))
            .map(drop)
            .unwrap_err();
        assert_eq!(error.to_string(), "starting ffmpeg");
    }

    #[test]
    fn the_child_is_in_a_process_group_of_its_own_so_that_ctrl_c_does_not_reach_it() {
        let launched = launch(shell("ps -o pgid= -p $$ >&2; cat >/dev/null")).unwrap();
        let (mut child, said) = (launched.child, launched.said);
        drop((launched.video, launched.audio));
        child.wait().unwrap();
        launched.reader.join().unwrap();
        let group: i32 = said.summary().trim().parse().unwrap();
        // SAFETY: getpgrp has no preconditions.
        assert_ne!(group, unsafe { libc::getpgrp() });
        assert_eq!(group, child.id() as i32);
    }
}
