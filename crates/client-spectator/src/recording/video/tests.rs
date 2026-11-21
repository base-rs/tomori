// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// A frame so large that the queue holds the fewest.
const LARGE: usize = QUEUED_BYTES;
const QUEUED: usize = MINIMUM_QUEUED;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, channel};

fn millis(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// A pipe that keeps what is written to it.
#[derive(Clone, Default)]
struct Kept(Arc<Mutex<Vec<u8>>>);

impl Write for Kept {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An encoder that has stopped reading: a write says it has begun, and waits until the test lets it go.
struct Stuck(Receiver<()>, mpsc::Sender<()>);

impl Write for Stuck {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _ = self.1.send(());
        let _ = self.0.recv();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Broken;

impl Write for Broken {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_ending_is_said_once_for_the_writers_to_see() {
    let ending = Ending::default();
    assert_eq!(ending.at(), None);
    ending.end(millis(250));
    assert_eq!(ending.at(), Some(millis(250)));
}

#[test]
fn frames_are_handed_over_without_waiting_and_dropped_when_the_writer_is_far_behind() {
    let start = Instant::now();
    let (sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    for _ in 0..QUEUED {
        assert!(sender.send(Instant::now(), vec![1]));
    }
    assert_eq!(sender.lost(), 0);
    // The writer is not taking them: the next is dropped, not waited for.
    assert!(!sender.send(Instant::now(), vec![2]));
    sender.lose();
    assert_eq!(sender.lost(), 2);
    let (at, frame) = frames.try_recv().unwrap();
    assert_eq!(*frame, [1]);
    assert!(at <= start.elapsed());
    // A frame drawn before the recording started is at its start.
    assert!(sender.send(start - millis(5), vec![3]));
    drop(frames);
    assert!(!sender.send(Instant::now(), vec![4]));
    assert_eq!(sender.lost(), 3);
}

#[test]
fn a_recording_has_a_frame_for_every_slot_of_its_length() {
    let start = Instant::now();
    let (sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    let (kept, ending, stop) = (
        Kept::default(),
        Arc::new(Ending::default()),
        Stop::default(),
    );
    let clock = FrameClock::new(10);
    let writer = write(
        kept.clone(),
        frames,
        Origin::at(start),
        clock,
        ending.clone(),
        stop.clone(),
        4,
    )
    .unwrap();
    for value in 1..=3 {
        assert!(sender.send(Instant::now(), vec![value; 4]));
        thread::sleep(millis(100));
    }
    // 0.7 seconds at ten a second: seven frames, however many were drawn.
    ending.end(millis(700));
    let totals = writer.join().unwrap();
    assert_eq!(totals.counts.written, 7);
    assert_eq!(kept.0.lock().unwrap().len(), 7 * 4);
    assert_eq!(totals.error, None);
    assert!(!stop.requested());
    // The first frame drawn is the first in the file, and the last repeats to the end.
    let written = kept.0.lock().unwrap().clone();
    assert_eq!(&written[..4], [1; 4]);
    assert_eq!(&written[24..], [3; 4]);
}

#[test]
fn a_recording_stopped_before_anything_was_drawn_is_black_for_its_length() {
    let start = Instant::now();
    let (_sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    let (kept, ending) = (Kept::default(), Arc::new(Ending::default()));
    let clock = FrameClock::new(10);
    let writer = write(
        kept.clone(),
        frames,
        Origin::at(start),
        clock,
        ending.clone(),
        Stop::default(),
        4,
    )
    .unwrap();
    ending.end(millis(200));
    let totals = writer.join().unwrap();
    assert_eq!(totals.counts.written, 2);
    assert_eq!(*kept.0.lock().unwrap(), [0; 8]);
}

#[test]
fn a_writer_that_has_fallen_behind_catches_up_by_repeating_the_frame_and_says_so() {
    log::set_max_level(log::LevelFilter::Trace);
    // A recording that began two seconds ago, drawn once.
    let start = Instant::now() - Duration::from_secs(2);
    let (sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    sender.send(start, vec![9; 4]);
    let (kept, ending) = (Kept::default(), Arc::new(Ending::default()));
    // It ends as it starts, with all two seconds of slots to write at once.
    ending.end(Duration::from_secs(2));
    let writer = write(
        kept.clone(),
        frames,
        Origin::at(start),
        FrameClock::new(20),
        ending,
        Stop::default(),
        4,
    )
    .unwrap();
    let totals = writer.join().unwrap();
    assert_eq!(totals.counts.written, 40);
    assert_eq!(totals.counts.duplicated, 39);
    assert!(
        totals.behind > Duration::from_secs(1),
        "{:?}",
        totals.behind
    );
}

#[test]
fn a_video_that_cannot_be_written_stops_the_recording() {
    let start = Instant::now() - Duration::from_secs(1);
    let (sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    sender.send(start, vec![9; 4]);
    let (ending, stop) = (Arc::new(Ending::default()), Stop::default());
    let writer = write(
        Broken,
        frames,
        Origin::at(start),
        FrameClock::new(10),
        ending,
        stop.clone(),
        4,
    )
    .unwrap();
    let totals = writer.join().unwrap();
    assert_eq!(
        totals.error.as_deref(),
        Some("cannot write the video to ffmpeg: broken pipe")
    );
    assert_eq!(totals.counts.written, 0);
    assert_eq!(stop.reason().as_deref(), Some("the encoder stopped"));
}

#[test]
fn an_encoder_that_stops_reading_never_holds_up_the_render_loop() {
    let start = Instant::now() - Duration::from_secs(1);
    let (sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    let (release, stuck) = channel();
    let (entered, writing) = channel();
    let ending = Arc::new(Ending::default());
    sender.send(start, vec![9; 4]);
    let writer = write(
        Stuck(stuck, entered),
        frames,
        Origin::at(start),
        FrameClock::new(10),
        ending.clone(),
        Stop::default(),
        4,
    )
    .unwrap();
    // The writer is stuck in its first write. A render loop at 60 frames a second for three seconds is never held up: frames over the queue are dropped.
    writing.recv().unwrap();
    let mut sent = 0;
    for _ in 0..180 {
        sent += u64::from(sender.send(Instant::now(), vec![1; 4]));
    }
    assert_eq!(sent, QUEUED as u64);
    assert_eq!(sender.lost(), 180 - QUEUED as u64);
    // The encoder reads again, and the recording ends.
    drop(release);
    ending.end(millis(1_500));
    let totals = writer.join().unwrap();
    assert_eq!(totals.counts.written, 15);
}

#[test]
fn a_writer_goes_on_after_every_sender_has_gone_and_ends_with_the_recording() {
    let start = Instant::now();
    let (sender, frames) = FrameSender::new(Origin::at(start), LARGE);
    sender.send(start, vec![5; 4]);
    drop(sender);
    let (kept, ending) = (Kept::default(), Arc::new(Ending::default()));
    let writer = write(
        kept.clone(),
        frames,
        Origin::at(start),
        FrameClock::new(10),
        ending.clone(),
        Stop::default(),
        4,
    )
    .unwrap();
    thread::sleep(millis(80));
    ending.end(millis(300));
    let totals = writer.join().unwrap();
    assert_eq!(totals.counts.written, 3);
    assert_eq!(*kept.0.lock().unwrap(), [5; 12]);
}

#[test]
fn a_writer_that_cannot_be_started_is_reported() {
    let (_sender, frames) = FrameSender::new(Origin::at(Instant::now()), LARGE);
    system::refuse("tomori-record-video");
    let error = write(
        Kept::default(),
        frames,
        Origin::at(Instant::now()),
        FrameClock::new(30),
        Arc::default(),
        Stop::default(),
        4,
    )
    .map(drop)
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the system refused to start tomori-record-video"
    );
}

#[test]
fn the_queue_holds_what_128_mib_of_frames_hold_and_never_fewer_than_four() {
    let held = |size: (usize, usize)| {
        let (sender, _frames) = FrameSender::new(Origin::at(Instant::now()), size.0 * size.1 * 4);
        (0..64)
            .take_while(|_| sender.send(Instant::now(), vec![0]))
            .count()
    };
    // 640x360 is small enough to hold the most; 1080p holds sixteen; 4K, which is four times the pixels, four.
    assert_eq!(held((640, 360)), MAXIMUM_QUEUED);
    assert_eq!(held((1920, 1080)), 16);
    assert_eq!(held((3840, 2160)), MINIMUM_QUEUED);
    assert_eq!(held((1, 1)), MAXIMUM_QUEUED);
}

#[test]
fn nothing_is_timed_before_the_recording_begins() {
    let origin = Origin::default();
    assert!(!origin.begun());
    assert_eq!(origin.elapsed(), Duration::ZERO);
    assert_eq!(origin.since(Instant::now()), Duration::ZERO);
    let start = Instant::now();
    origin.begin(start);
    // The first to begin it keeps it.
    origin.begin(start + Duration::from_secs(5));
    assert!(origin.begun());
    assert_eq!(origin.since(start + millis(40)), millis(40));
    assert_eq!(origin.since(start - millis(40)), Duration::ZERO);
    assert!(origin.elapsed() < Duration::from_secs(5));
}
