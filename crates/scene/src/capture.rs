// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform microphone, opened for live audio or push-to-talk. See `design/live-audio.md`.
//!
//! Capture hands mono samples to a callback on its own thread; [`crate::connection::Connection`] passes them to the client's [`tomori_client::Microphone`], which frames and sends them.
//!
//! # Device adapters
//!
//! Each client's microphone is a [`Capture`], which its own crate implements over the platform's audio interface. Two helpers keep those adapters thin, so that their logic is tested here:
//!
//! - a device that pushes interleaved frames to a callback, such as cpal's, passes them through [`downmix`];
//! - a device that is read from, such as AAudio's, implements [`InputStream`] and runs [`run_capture`] on its capture thread, which decides what is delivered at the push-to-talk boundary and when a lost connection stops live capture.

/// Receives captured mono samples in the range -1 to 1, on the capture thread.
pub type Samples = Box<dyn FnMut(&[f32]) + Send>;

/// Told, from any thread, why capture stopped early.
pub type Failed = Box<dyn Fn(String) + Send + Sync>;

/// Whether the connection on which live capture started is still usable.
pub type Active = Box<dyn Fn() -> bool + Send>;

/// Capture in progress, stopped when dropped. Samples already delivered are not lost.
pub struct Recording {
    pub sample_rate: u32,
    _stream: Box<dyn std::any::Any>,
}

impl Recording {
    pub fn new(sample_rate: u32, stream: impl std::any::Any) -> Self {
        Self {
            sample_rate,
            _stream: Box::new(stream),
        }
    }
}

/// A source of microphone audio.
pub trait Capture {
    /// Whether the app may record, asking the participant if it has not yet. A request is answered later, so `Ok(false)` means ask again after they respond.
    fn permit(&mut self) -> Result<bool, String> {
        Ok(true)
    }

    /// Ask for permission ahead of the first recording, without using up the
    /// request [`Self::permit`] makes, so that a dismissed request is asked
    /// again when the participant first uses the microphone.
    fn request(&mut self) -> Result<bool, String> {
        Ok(true)
    }

    /// Start capturing at a rate between 8 and 48 kHz, which the protocol carries.
    fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String>;

    /// Capture for an open microphone, stopping when its connection is lost.
    /// Platforms must opt in with a capture implementation that can stop on
    /// the capture thread, even if the application thread is stalled.
    fn open_live(
        &mut self,
        _samples: Samples,
        _failed: Failed,
        _active: Active,
    ) -> Result<Recording, String> {
        Err("Live microphone is not available on this platform.".into())
    }

    /// The audio session the open live microphone captures in, where the platform allocates one, so that the speaker can play in the same session while it is open. See `design/spatial-audio.md`.
    fn live_session(&self) -> Option<i32> {
        None
    }
}

/// Average interleaved channels into mono, for a device adapter whose device delivers `channels` samples per frame. `output` is cleared first, and receives one sample per whole frame; a trailing partial frame is dropped.
pub fn downmix(samples: &[f32], channels: usize, output: &mut Vec<f32>) {
    output.clear();
    if channels <= 1 {
        output.extend_from_slice(samples);
        return;
    }
    output.extend(
        samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32),
    );
}

/// A pull-based input stream, such as AAudio's, which a device adapter implements for [`run_capture`]. Separated from the device so
/// that the capture boundary can be tested deterministically.
///
/// [`run_capture`] calls [`Self::start`] once, then reads until capture ends, then [`Self::stop`] once, whatever the result. An error from any of them ends capture with that error.
pub trait InputStream {
    /// Start the device capturing. Nothing is read before this succeeds.
    fn start(&mut self) -> Result<(), String>;
    /// Read up to `buffer.len()` mono samples, in the range -1 to 1 at the
    /// [`Recording`]'s rate, into the start of `buffer`, and return how many,
    /// waiting at most `timeout_ns`; zero returns only what the device has
    /// already captured, without waiting, and returns 0 once that is all read.
    fn read(&mut self, buffer: &mut [f32], timeout_ns: i64) -> Result<usize, String>;
    /// Stop the device. Called once, after the last read.
    fn stop(&mut self);
    /// How many times the device has discarded audio because it was not read
    /// in time, a count that only grows while started: any increase between
    /// reads is taken as lost audio. A device that cannot tell reports a
    /// constant, such as zero or an error code, which is never taken as a loss.
    fn overruns(&self) -> i32;
}

const CONNECTION_LOST: &str = "Connection lost; live microphone stopped.";

/// How long one read may wait for audio, so that stop and disconnection are
/// noticed promptly.
const READ_TIMEOUT_NS: i64 = 100_000_000;

/// The most non-blocking reads made to collect what the device had already
/// captured when capture was stopped; a well-behaved device runs dry first.
const MAX_DRAIN_READS: usize = 64;

/// The shortest interval between warnings that the live microphone lost audio.
const OVERRUN_LOG: std::time::Duration = std::time::Duration::from_secs(30);

/// Capture from the stream `open` makes until `stop` is set or, for live
/// audio, `active` reports that the connection ended, delivering what is read
/// to `samples`, through `buffer`. A device adapter runs this on its capture
/// thread; `active` is `None` for push-to-talk.
///
/// The capture boundary is the moment `stop` is observed. Every sample the
/// device captured before it is delivered: a read in flight when stop is set,
/// then what the device already holds, read without waiting, including a final
/// partial frame. If that cannot be read intact, including when the device
/// reports an overrun during push-to-talk, capture fails, so the utterance is
/// cancelled visibly instead of being sent with a gap. Live audio tolerates an
/// overrun, which only glitches what the other participants hear.
///
/// A connection that ended before or while the device opened never starts
/// the stream, and no sample is delivered once `active` reports false. A
/// disconnection racing the start itself stops the stream before any read.
pub fn run_capture<S: InputStream>(
    open: impl FnOnce() -> Result<S, String>,
    samples: &mut dyn FnMut(&[f32]),
    stop: &std::sync::atomic::AtomicBool,
    active: Option<&Active>,
    buffer: &mut [f32],
) -> Result<(), String> {
    use crate::log_cadence::Cadence;
    use std::sync::atomic::Ordering;
    let current = || active.is_none_or(|active| active());
    if !current() {
        return Err(CONNECTION_LOST.into());
    }
    let mut stream = open()?;
    // Opening the device can take a while; the connection may have ended.
    if !current() {
        return Err(CONNECTION_LOST.into());
    }
    if stop.load(Ordering::Acquire) {
        // Released before capture began: nothing was captured.
        return Ok(());
    }
    stream.start()?;
    let overruns = std::cell::Cell::new(stream.overruns());
    // Overruns that keep recurring would otherwise warn on every read; see `design/client-logging.md`.
    let unreported = std::cell::RefCell::new((Cadence::new(OVERRUN_LOG), 0_u64));
    // Push-to-talk (no `active`) must reach recognition intact.
    let intact = |stream: &S| -> Result<(), String> {
        let count = stream.overruns();
        let lost = count.saturating_sub(overruns.get());
        if lost > 0 {
            if active.is_none() {
                return Err("Microphone lost audio; not sent.".into());
            }
            overruns.set(count);
            let (cadence, pending) = &mut *unreported.borrow_mut();
            *pending += u64::from(lost.unsigned_abs());
            if cadence.record(std::time::Instant::now()).is_some() {
                warn_overruns(std::mem::take(pending));
            }
        }
        Ok(())
    };
    let result = loop {
        if !current() {
            break Err(CONNECTION_LOST.into());
        }
        if stop.load(Ordering::Acquire) {
            break drain(&mut stream, samples, &current, &intact, buffer);
        }
        match stream.read(buffer, READ_TIMEOUT_NS) {
            // A successful read was captured before stop was observed, so it
            // is delivered even if stop was set while it was in flight.
            Ok(count) => {
                if let Err(error) = intact(&stream) {
                    break Err(error);
                }
                if current() {
                    samples(&buffer[..count]);
                }
            }
            Err(error) => break Err(error),
        }
    };
    stream.stop();
    let (_, pending) = unreported.into_inner();
    if pending > 0 {
        warn_overruns(pending);
    }
    result
}

fn warn_overruns(lost: u64) {
    log::warn!("The live microphone lost audio ({lost} overruns).");
}

/// Deliver what the device captured before the boundary, without waiting.
fn drain<S: InputStream>(
    stream: &mut S,
    samples: &mut dyn FnMut(&[f32]),
    current: &dyn Fn() -> bool,
    intact: &dyn Fn(&S) -> Result<(), String>,
    buffer: &mut [f32],
) -> Result<(), String> {
    for _ in 0..MAX_DRAIN_READS {
        let count = stream.read(buffer, 0)?;
        intact(stream)?;
        if count == 0 {
            return Ok(());
        }
        if !current() {
            return Err(CONNECTION_LOST.into());
        }
        samples(&buffer[..count]);
    }
    Err("The microphone kept delivering audio after it was stopped.".into())
}

/// The protocol's range of sample rates.
#[cfg(not(target_os = "android"))]
const RATES: std::ops::RangeInclusive<u32> = 8_000..=48_000;

/// The nearest rate to a device's `rate` that the protocol carries.
#[cfg(not(target_os = "android"))]
pub fn protocol_rate(rate: u32) -> u32 {
    rate.clamp(*RATES.start(), *RATES.end())
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "android"))]
    use super::protocol_rate;
    use super::{
        Active, CONNECTION_LOST, Capture, InputStream, MAX_DRAIN_READS, downmix, run_capture,
    };
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// One scripted device step, run by a read.
    enum Step {
        /// A read that returns these samples, after running a side effect
        /// that happens while the read is in flight.
        Read(Vec<f32>, Box<dyn FnOnce()>),
        Fail,
    }

    #[derive(Default)]
    struct Log {
        started: bool,
        stopped: bool,
        blocking_reads: usize,
        drain_reads: usize,
        overruns: i32,
    }

    struct Fake {
        /// Whether starting the stream fails.
        fail_start: bool,
        /// Whether the device reports an overrun when what it holds is read.
        overrun_on_drain: bool,
        steps: VecDeque<Step>,
        /// What the device already holds when read without waiting.
        buffered: VecDeque<Result<Vec<f32>, ()>>,
        log: Rc<RefCell<Log>>,
    }

    impl InputStream for Fake {
        fn start(&mut self) -> Result<(), String> {
            if self.fail_start {
                return Err("cannot start".into());
            }
            self.log.borrow_mut().started = true;
            Ok(())
        }

        fn read(&mut self, buffer: &mut [f32], timeout_ns: i64) -> Result<usize, String> {
            let next = if timeout_ns == 0 {
                let mut log = self.log.borrow_mut();
                log.drain_reads += 1;
                log.overruns += i32::from(self.overrun_on_drain);
                drop(log);
                match self.buffered.pop_front() {
                    None => return Ok(0),
                    Some(Err(())) => return Err("drain failed".into()),
                    Some(Ok(samples)) => samples,
                }
            } else {
                self.log.borrow_mut().blocking_reads += 1;
                match self.steps.pop_front().expect("unexpected read") {
                    Step::Read(samples, during) => {
                        during();
                        samples
                    }
                    Step::Fail => return Err("read failed".into()),
                }
            };
            buffer[..next.len()].copy_from_slice(&next);
            Ok(next.len())
        }

        fn stop(&mut self) {
            self.log.borrow_mut().stopped = true;
        }

        fn overruns(&self) -> i32 {
            self.log.borrow().overruns
        }
    }

    fn fake(steps: Vec<Step>, buffered: Vec<Result<Vec<f32>, ()>>) -> (Fake, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let fake = Fake {
            fail_start: false,
            overrun_on_drain: false,
            steps: steps.into(),
            buffered: buffered.into(),
            log: log.clone(),
        };
        (fake, log)
    }

    fn run(
        fake: Result<Fake, String>,
        stop: &Arc<AtomicBool>,
        active: Option<&Active>,
        during_open: impl FnOnce(),
    ) -> (Result<(), String>, Vec<f32>) {
        let mut delivered = Vec::new();
        let mut buffer = [0.0_f32; 8];
        let result = run_capture(
            || {
                during_open();
                fake
            },
            &mut |samples: &[f32]| delivered.extend_from_slice(samples),
            stop,
            active,
            &mut buffer,
        );
        (result, delivered)
    }

    #[test]
    fn a_disconnect_while_the_device_opens_never_starts_the_stream() {
        let connected = Arc::new(AtomicBool::new(true));
        let current = connected.clone();
        let active: Active = Box::new(move || current.load(Ordering::SeqCst));
        let (device, log) = fake(vec![], vec![]);
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(Ok(device), &stop, Some(&active), || {
            connected.store(false, Ordering::SeqCst)
        });
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert!(delivered.is_empty());
        assert!(!log.borrow().started);
    }

    #[test]
    fn a_disconnect_before_opening_never_opens_the_device() {
        let active: Active = Box::new(|| false);
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(
            Err("never opened".into()),
            &stop,
            Some(&active),
            std::thread::yield_now,
        );
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert!(delivered.is_empty());
    }

    #[test]
    fn a_device_that_cannot_open_or_start_says_why() {
        let stop = Arc::new(AtomicBool::new(false));
        let (result, _) = run(Err("busy".into()), &stop, None, || {});
        assert_eq!(result.unwrap_err(), "busy");
        let (mut device, log) = fake(vec![], vec![]);
        device.fail_start = true;
        let (result, _) = run(Ok(device), &stop, None, || {});
        assert_eq!(result.unwrap_err(), "cannot start");
        assert!(!log.borrow().stopped);
    }

    #[test]
    fn recurring_live_overruns_are_reported_together() {
        let active: Active = Box::new(|| true);
        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let (device, log) = fake(vec![], vec![]);
        let overrun = |log: &Rc<RefCell<Log>>| {
            let log = log.clone();
            Box::new(move || log.borrow_mut().overruns += 1) as Box<dyn FnOnce()>
        };
        let mut device = device;
        device.steps = vec![
            Step::Read(vec![0.1; 2], overrun(&log)),
            Step::Read(vec![0.1; 2], overrun(&log)),
            Step::Read(
                vec![0.1; 2],
                Box::new(move || {
                    release.store(true, Ordering::Release);
                }),
            ),
        ]
        .into();
        let (result, delivered) = run(Ok(device), &stop, Some(&active), || {});
        assert!(result.is_ok());
        assert_eq!(delivered.len(), 6);
        assert_eq!(log.borrow().overruns, 2);
    }

    #[test]
    fn a_drain_that_fails_or_outlives_its_connection_says_so() {
        let released = || {
            let stop = Arc::new(AtomicBool::new(false));
            let releasing = stop.clone();
            let step = Step::Read(
                vec![0.1; 2],
                Box::new(move || releasing.store(true, Ordering::Release)),
            );
            (stop, step)
        };
        let (stop, step) = released();
        let (device, _) = fake(vec![step], vec![Err(())]);
        let (result, _) = run(Ok(device), &stop, None, || {});
        assert_eq!(result.unwrap_err(), "drain failed");
        // The connection ends while the drain reads what the device held: the sixth time it is asked.
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counting = asked.clone();
        let active: Active = Box::new(move || counting.fetch_add(1, Ordering::SeqCst) < 5);
        let (stop, step) = released();
        let (device, _) = fake(vec![step], vec![Ok(vec![0.2; 2])]);
        let (result, delivered) = run(Ok(device), &stop, Some(&active), || {});
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert_eq!(delivered, [0.1; 2]);
        // Push-to-talk loses what the device overran while it drained.
        let (stop, step) = released();
        let (mut device, _) = fake(vec![step], vec![Ok(vec![0.2; 2])]);
        device.overrun_on_drain = true;
        let (result, _) = run(Ok(device), &stop, None, || {});
        assert_eq!(result.unwrap_err(), "Microphone lost audio; not sent.");
    }

    #[test]
    fn a_disconnect_during_a_read_drops_that_read_and_stops() {
        let connected = Arc::new(AtomicBool::new(true));
        let current = connected.clone();
        let active: Active = Box::new(move || current.load(Ordering::SeqCst));
        let lost = connected.clone();
        let (device, log) = fake(
            vec![
                Step::Read(vec![0.1; 4], Box::new(|| {})),
                Step::Read(
                    vec![0.2; 4],
                    Box::new(move || lost.store(false, Ordering::SeqCst)),
                ),
            ],
            vec![Ok(vec![0.3; 4])],
        );
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(Ok(device), &stop, Some(&active), || {});
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert_eq!(delivered, vec![0.1; 4]);
        let log = log.borrow();
        assert!(log.stopped);
        assert_eq!(log.drain_reads, 0);
    }

    #[test]
    fn release_during_a_read_delivers_it_and_what_the_device_already_holds() {
        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let (device, log) = fake(
            vec![
                Step::Read(vec![0.1; 8], Box::new(|| {})),
                // Push-to-talk is released while this read is in flight.
                Step::Read(
                    vec![0.2; 8],
                    Box::new(move || release.store(true, Ordering::Release)),
                ),
            ],
            // The device held one full and one partial frame at release.
            vec![Ok(vec![0.3; 8]), Ok(vec![0.4; 3])],
        );
        let (result, delivered) = run(Ok(device), &stop, None, || {});
        result.unwrap();
        let mut expected = vec![0.1; 8];
        expected.extend([0.2; 8]);
        expected.extend([0.3; 8]);
        expected.extend([0.4; 3]);
        assert_eq!(delivered, expected);
        let log = log.borrow();
        assert!(log.stopped);
        assert_eq!(log.blocking_reads, 2);
        assert_eq!(log.drain_reads, 3);
    }

    #[test]
    fn a_failed_final_read_fails_capture_instead_of_ending_short() {
        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let (device, _log) = fake(
            vec![Step::Read(
                vec![0.1; 8],
                Box::new(move || release.store(true, Ordering::Release)),
            )],
            vec![Ok(vec![0.2; 2]), Err(())],
        );
        let (result, delivered) = run(Ok(device), &stop, None, || {});
        assert_eq!(result.unwrap_err(), "drain failed");
        // What was delivered is still pre-boundary audio; the caller cancels
        // the utterance on failure rather than ending it.
        assert_eq!(delivered.len(), 10);

        let (device, log) = fake(vec![Step::Fail], vec![]);
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(Ok(device), &stop, None, || {});
        assert_eq!(result.unwrap_err(), "read failed");
        assert!(delivered.is_empty());
        assert!(log.borrow().stopped);
    }

    #[test]
    fn an_overrun_fails_push_to_talk_but_only_glitches_live_audio() {
        let overrun = |log: &Rc<RefCell<Log>>| {
            let log = log.clone();
            Box::new(move || log.borrow_mut().overruns += 1) as Box<dyn FnOnce()>
        };
        let log = Rc::new(RefCell::new(Log::default()));
        let device = Fake {
            fail_start: false,
            overrun_on_drain: false,
            steps: vec![
                Step::Read(vec![0.1; 4], Box::new(|| {})),
                Step::Read(vec![0.2; 4], overrun(&log)),
            ]
            .into(),
            buffered: VecDeque::new(),
            log: log.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(Ok(device), &stop, None, || {});
        assert!(result.unwrap_err().contains("lost audio"));
        // The samples after the gap are not delivered.
        assert_eq!(delivered, vec![0.1; 4]);

        let log = Rc::new(RefCell::new(Log::default()));
        let release = stop.clone();
        stop.store(false, Ordering::Release);
        let device = Fake {
            fail_start: false,
            overrun_on_drain: false,
            steps: vec![
                Step::Read(vec![0.2; 4], overrun(&log)),
                Step::Read(
                    vec![0.3; 4],
                    Box::new(move || release.store(true, Ordering::Release)),
                ),
            ]
            .into(),
            buffered: VecDeque::new(),
            log,
        };
        let active: Active = Box::new(|| true);
        let (result, delivered) = run(Ok(device), &stop, Some(&active), || {});
        result.unwrap();
        assert_eq!(delivered.len(), 8);
    }

    #[test]
    fn a_device_that_never_runs_dry_fails_capture() {
        let stop = Arc::new(AtomicBool::new(true));
        let endless = (0..=MAX_DRAIN_READS).map(|_| Ok(vec![0.0; 1])).collect();
        let (device, _log) = fake(vec![], endless);
        // Stop before the first read still starts nothing.
        let (result, delivered) = run(Ok(device), &stop, None, || {});
        result.unwrap();
        assert!(delivered.is_empty());

        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let endless = (0..=MAX_DRAIN_READS).map(|_| Ok(vec![0.0; 1])).collect();
        let (device, _log) = fake(
            vec![Step::Read(
                vec![],
                Box::new(move || release.store(true, Ordering::Release)),
            )],
            endless,
        );
        let (result, delivered) = run(Ok(device), &stop, None, || {});
        assert!(result.unwrap_err().contains("kept delivering"));
        assert_eq!(delivered.len(), MAX_DRAIN_READS);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn desktop_rates_are_brought_within_the_protocol() {
        assert_eq!(protocol_rate(4_000), 8_000);
        assert_eq!(protocol_rate(16_000), 16_000);
        assert_eq!(protocol_rate(96_000), 48_000);
    }

    #[test]
    fn downmixes_interleaved_channels() {
        let mut mono = Vec::new();
        downmix(&[1.0, 0.0, 0.5, 0.5, -1.0, -0.5], 2, &mut mono);
        assert_eq!(mono, [0.5, 0.5, -0.75]);
        downmix(&[0.25, 0.75], 1, &mut mono);
        assert_eq!(mono, [0.25, 0.75]);
    }

    #[test]
    fn platforms_without_independently_stoppable_live_capture_refuse_it() {
        // A capture that opens only for push-to-talk, as the test microphone does.
        let samples = |_: &[f32]| {};
        let failed = |_: String| {};
        let active = || true;
        // What a capture that opened would call.
        samples(&[]);
        failed(String::new());
        assert!(active());
        let error = crate::test_support::Silent
            .open_live(Box::new(samples), Box::new(failed), Box::new(active))
            .map(drop)
            .unwrap_err();
        assert!(error.contains("not available"));
    }
}
