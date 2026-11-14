// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The log records of this crate's tests, for a test to check what was said, whichever process it runs in and whatever ran before it.
//!
//! Tests run in parallel in one process under Cargo's harness, so a test watches for text only its own records hold, such as its own path or identifier. Only records that a live [`Watch`] matches are kept, so the recorder holds no more than what tests are looking for.
use std::sync::{Arc, Mutex, Weak};

/// The records a test is looking for: those that contain `about`.
struct Watched {
    about: String,
    records: Mutex<Vec<(log::Level, String)>>,
}

/// The watches of this process's tests.
struct Recorder(Mutex<Vec<Weak<Watched>>>);

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let watches: Vec<Arc<Watched>> = {
            let mut watches = self.0.lock().unwrap();
            watches.retain(|watch| watch.strong_count() > 0);
            watches.iter().filter_map(Weak::upgrade).collect()
        };
        // Written out only when some test is watching.
        let line = std::cell::OnceCell::new();
        let text = || line.get_or_init(|| record.args().to_string());
        for watch in watches.iter().filter(|watch| text().contains(&watch.about)) {
            watch
                .records
                .lock()
                .unwrap()
                .push((record.level(), text().clone()));
        }
    }

    fn flush(&self) {}
}

static LOG: Recorder = Recorder(Mutex::new(Vec::new()));

/// Log at every level from now on in this process, so that each log line's arguments are evaluated.
pub fn record() {
    static RECORDING: std::sync::Once = std::sync::Once::new();
    RECORDING.call_once(|| {
        log::set_logger(&LOG).expect("only these tests log to a recorder");
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// Records that contain the text a test watches for, from when it began watching until it is dropped.
pub struct Watch(Arc<Watched>);

/// Watch for records that contain `about`, logging at every level as [`record`] does.
pub fn watch(about: impl Into<String>) -> Watch {
    record();
    let watched = Arc::new(Watched {
        about: about.into(),
        records: Mutex::new(Vec::new()),
    });
    LOG.0.lock().unwrap().push(Arc::downgrade(&watched));
    Watch(watched)
}

impl Watch {
    /// The records seen so far, with their levels.
    pub fn records(&self) -> Vec<(log::Level, String)> {
        log::Log::flush(&LOG);
        assert!(log::Log::enabled(&LOG, &log::Metadata::builder().build()));
        self.0.records.lock().unwrap().clone()
    }
}
