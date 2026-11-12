// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Captures, speakers and a client for the connection's tests, and its parts' tests.
use crate::{
    Client, Options,
    capture::{Active, Capture, Failed, Recording, Samples},
    playback::Layout,
    speaker::{Output, Playback, Route, Shared},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

/// A client that never connects, which is enough to queue messages.
pub(super) fn client() -> Client {
    Client::connect(Options::new("127.0.0.1:9", "ada", "Ada")).unwrap()
}

/// A capture that permits and opens as it is told, keeping the callbacks it is given.
#[derive(Default)]
pub(super) struct Scripted {
    /// Why permission is refused, if it is.
    pub(super) refuse: Option<String>,
    /// Why opening fails, if it does, after reporting it through the failure callback; from the opening numbered `fail_from` on.
    pub(super) fail: Option<String>,
    pub(super) fail_from: usize,
    pub(super) opened: usize,
    pub(super) samples: Arc<Mutex<Vec<Samples>>>,
    /// The failure callback of each capture opened.
    pub(super) failed: Arc<Mutex<Vec<Failed>>>,
    /// What live capture was told about its connection when it opened.
    pub(super) current: Arc<Mutex<Vec<bool>>>,
    /// Whether asking ahead of time is granted at once.
    pub(super) granted: bool,
    /// How many times it was asked ahead of time.
    pub(super) asked: Arc<AtomicU32>,
    /// Why capture fails as it stops, if it does.
    pub(super) fail_on_stop: Option<String>,
    /// How many captures it opened have stopped.
    pub(super) stopped: Arc<AtomicU32>,
    /// The audio session live capture plays in.
    pub(super) session: Option<i32>,
}

/// Reports a failure when its recording is dropped, as a device whose last read fails, and counts it as stopped.
struct FailsOnStop(Option<(Failed, String)>, Arc<AtomicU32>);

impl Drop for FailsOnStop {
    fn drop(&mut self) {
        self.1.fetch_add(1, Ordering::Relaxed);
        if let Some((failed, error)) = self.0.take() {
            failed(error);
        }
    }
}

impl Capture for Scripted {
    fn permit(&mut self) -> Result<bool, String> {
        self.refuse.clone().map_or(Ok(true), Err)
    }

    fn request(&mut self) -> Result<bool, String> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.refuse.clone().map_or(Ok(self.granted), Err)
    }

    fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String> {
        self.opened += 1;
        if let Some(error) = self.fail.clone().filter(|_| self.opened > self.fail_from) {
            failed(error.clone());
            return Err(error);
        }
        self.samples.lock().unwrap().push(samples);
        let stopping = match self.fail_on_stop.clone() {
            Some(error) => FailsOnStop(Some((failed, error)), self.stopped.clone()),
            None => {
                self.failed.lock().unwrap().push(failed);
                FailsOnStop(None, self.stopped.clone())
            }
        };
        Ok(Recording::new(16_000, stopping))
    }

    fn live_session(&self) -> Option<i32> {
        self.session
    }

    fn open_live(
        &mut self,
        samples: Samples,
        failed: Failed,
        active: Active,
    ) -> Result<Recording, String> {
        self.current.lock().unwrap().push(active());
        self.open(samples, failed)
    }
}

/// Opens outputs that render the player once, or fails to open while `refuse` is set. Keeps each open output's failure callback.
#[derive(Clone, Default)]
pub(super) struct FakeSpeaker {
    pub(super) opened: Arc<AtomicU32>,
    pub(super) closed: Arc<AtomicU32>,
    pub(super) refuse: Arc<AtomicBool>,
    pub(super) failed: Arc<Mutex<Option<crate::speaker::Failed>>>,
    /// Whether an open output is opened again on gaining focus.
    pub(super) reopens: bool,
}

/// Closes by counting, so that a test can see the speaker thread close its output.
struct FakeOutput(Arc<AtomicU32>);

impl Drop for FakeOutput {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

impl Playback for FakeSpeaker {
    fn open(
        &self,
        player: Shared,
        failed: crate::speaker::Failed,
        _route: Route,
    ) -> Result<Output, String> {
        if self.refuse.load(Ordering::Relaxed) {
            return Err("No speaker is available.".into());
        }
        let mut output = [1; 48];
        player
            .lock()
            .unwrap()
            .render(48_000, Layout::Mono, &mut output);
        assert_eq!(output, [0; 48]);
        *self.failed.lock().unwrap() = Some(failed);
        self.opened.fetch_add(1, Ordering::Relaxed);
        Ok(Output::new(FakeOutput(self.closed.clone())))
    }

    fn reopens_on_focus(&self) -> bool {
        self.reopens
    }
}

/// Wait for another thread, such as the speaker's.
pub(super) fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}
