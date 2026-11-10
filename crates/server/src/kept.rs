// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spaces a server keeps across restarts, in its state directory. See `design/spaces.md`.
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub use crate::stage::Stage;

/// The file the spaces are kept in, within the state directory.
pub const SPACES_FILE: &str = "spaces.json";

/// What is kept of one space: enough to list it again and bind it to its session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeptSpace {
    pub id: String,
    pub title: String,
    /// Who gave the space its title in place of its session's, if anyone did; absent while the title follows its
    /// session, and for a space kept before titles followed sessions. See `design/spaces.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renamed_by: Option<tomori_protocol::Renamer>,
    pub backend: String,
    pub directory: String,
    /// The backend's identifier of the session the space is bound to.
    pub session: String,
    pub environment: String,
    pub attribution: bool,
    /// Whether a participant chose brief speech for the space. Speech is full otherwise, as for a space kept before the setting was stored, so that only a choice made since keeps a space brief. See `design/speech.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub brief: bool,
    /// How the agent moves about the space, if an owner chose other than the default. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<tomori_protocol::AgentMovement>,
    /// The space's own instructions to the agent, if an owner gave any. See `design/spaces.md`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    /// Why the space closed, if it has; a closed space is listed again closed, and not bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    /// When the space closed relative to the others, so that the least recently closed are still forgotten first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_order: Option<u64>,
    /// Where its participants put its cards, by card, in a space whose environment shows cards. See `design/github.md`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub placements: std::collections::BTreeMap<String, tomori_protocol::Pose>,
    /// Its projection walls and the panels on them or floating, in a format of their own with its version. See
    /// `design/projection.md`.
    #[serde(default, skip_serializing_if = "Stage::is_empty")]
    pub stage: Stage,
    /// Where its workers were placed, at the anchors of each environment, by environment. See
    /// `design/environments.md`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub places: crate::places::Kept,
    /// When the space was last active, in seconds since the Unix epoch, to the minute, if known. See
    /// `design/arrival.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<u64>,
    /// Its timers, in a format of their own with its version, [`crate::timers::Kept`], kept as JSON and read leniently,
    /// so that a record this server cannot read, as of a newer version or edited by hand, never stops the spaces from
    /// loading. See `design/timers.md`.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub timers: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    spaces: Vec<KeptSpace>,
}

/// Why the kept spaces could not be read.
#[derive(Debug)]
pub enum KeepError {
    Io(io::Error),
    /// The file is not a record of spaces.
    Format(serde_json::Error),
    /// Another server keeps its spaces in the same state directory.
    Busy,
}

impl fmt::Display for KeepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Format(error) => write!(formatter, "the record of spaces is malformed: {error}"),
            Self::Busy => write!(
                formatter,
                "another server keeps its spaces in this state directory"
            ),
        }
    }
}

impl std::error::Error for KeepError {}

impl From<io::Error> for KeepError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Another server holding the record is [`KeepError::Busy`].
impl From<fs::TryLockError> for KeepError {
    fn from(error: fs::TryLockError) -> Self {
        match error {
            fs::TryLockError::WouldBlock => Self::Busy,
            fs::TryLockError::Error(error) => Self::Io(error),
        }
    }
}

/// The record of a server's spaces, held locked for as long as the server runs.
pub struct SpaceStore {
    path: PathBuf,
    /// Held for the store's life, so that no other server writes the same record.
    _lock: File,
    /// What the file holds, so that it is written only when the spaces change. Held while a change is worked out and written, so that changes are written in order.
    written: Mutex<Vec<KeptSpace>>,
}

impl fmt::Debug for SpaceStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SpaceStore {
    /// Open the record in `directory`, creating the directory if needed. A record that cannot be read is an error, rather than no spaces, so that a server never starts having lost them.
    pub fn open(directory: &Path) -> Result<Self, KeepError> {
        tomori_security::create_private_directory(directory)?;
        let path = directory.join(SPACES_FILE);
        let lock = File::create(path.with_extension("lock"))?;
        lock.try_lock()?;
        let spaces = match fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str::<Stored>(&text)
                    .map_err(KeepError::Format)?
                    .spaces
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            _lock: lock,
            written: Mutex::new(spaces),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The spaces the record holds.
    pub fn spaces(&self) -> Vec<KeptSpace> {
        self.written.lock().expect("never left poisoned").clone()
    }

    /// Write the spaces `current` gives, if they differ from those written, and return whether they were written. `current` is asked while the record is held, so that a later change is never overwritten by an earlier one; it returns `None` to write nothing.
    pub fn keep(&self, current: impl FnOnce() -> Option<Vec<KeptSpace>>) -> io::Result<bool> {
        let mut written = self.written.lock().expect("never left poisoned");
        let Some(spaces) = current() else {
            return Ok(false);
        };
        if spaces == *written {
            return Ok(false);
        }
        let text = serde_json::to_string_pretty(&Stored {
            spaces: spaces.clone(),
        })
        .expect("spaces serialize");
        tomori_security::write_private(&self.path, text.as_bytes())?;
        *written = spaces;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "tomori-kept-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    fn space(id: &str) -> KeptSpace {
        KeptSpace {
            id: id.into(),
            title: "Studio".into(),
            renamed_by: None,
            backend: "fake".into(),
            directory: "/tmp".into(),
            session: "session-1".into(),
            environment: "sky".into(),
            attribution: false,
            brief: false,
            movement: None,
            instructions: String::new(),
            closed: None,
            closed_order: None,
            placements: Default::default(),
            stage: Stage::default(),
            places: Default::default(),
            active: None,
            timers: Default::default(),
        }
    }

    #[test]
    fn spaces_are_kept_and_read_again() {
        let directory = directory("kept");
        let store = SpaceStore::open(&directory).unwrap();
        assert!(store.spaces().is_empty());
        assert!(store.keep(|| Some(vec![space("space-a-1")])).unwrap());
        // The same spaces are not written again, and `None` writes nothing.
        assert!(!store.keep(|| Some(vec![space("space-a-1")])).unwrap());
        assert!(!store.keep(|| None).unwrap());
        let closed = KeptSpace {
            closed: Some("The agent session ended.".into()),
            closed_order: Some(4),
            ..space("space-a-2")
        };
        assert!(
            store
                .keep(|| Some(vec![space("space-a-1"), closed.clone()]))
                .unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(store);
        let store = SpaceStore::open(&directory).unwrap();
        assert_eq!(store.spaces(), [space("space-a-1"), closed]);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn timers_this_server_cannot_read_never_stop_the_spaces_loading() {
        let directory = directory("timers");
        tomori_security::create_private_directory(&directory).unwrap();
        let mut record = serde_json::to_value(space("space-t-1")).unwrap();
        record["timers"] = serde_json::json!({
            "version": 2,
            "timers": [{ "name": "x", "schedule": { "kind": "lunar" }, "cron": "0 9 L * *" }],
        });
        fs::write(
            directory.join(SPACES_FILE),
            serde_json::json!({ "spaces": [record] }).to_string(),
        )
        .unwrap();
        let store = SpaceStore::open(&directory).unwrap();
        let spaces = store.spaces();
        assert_eq!(spaces[0].id, "space-t-1");
        assert_eq!(spaces[0].timers["version"], 2);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn one_server_keeps_a_directory_and_a_malformed_record_is_refused() {
        let directory = directory("refused");
        let store = SpaceStore::open(&directory).unwrap();
        let Err(KeepError::Busy) = SpaceStore::open(&directory) else {
            unreachable!("expected Err(KeepError::Busy)");
        };
        drop(store);
        fs::write(directory.join(SPACES_FILE), "not json").unwrap();
        let error = SpaceStore::open(&directory).unwrap_err();
        let KeepError::Format(_) = error else {
            unreachable!("expected KeepError::Format(_)");
        };
        assert!(
            error
                .to_string()
                .starts_with("the record of spaces is malformed")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_record_that_cannot_be_locked_or_read_is_refused_and_errors_say_why() {
        let locked = directory("unlockable");
        fs::create_dir_all(locked.join(SPACES_FILE).with_extension("lock")).unwrap();
        let Err(KeepError::Io(_)) = SpaceStore::open(&locked) else {
            unreachable!("a lock that cannot be made is an error");
        };
        let unread = directory("unreadable");
        fs::create_dir_all(unread.join(SPACES_FILE)).unwrap();
        let Err(KeepError::Io(_)) = SpaceStore::open(&unread) else {
            unreachable!("a record that cannot be read is an error");
        };
        let store = SpaceStore::open(&directory("debug")).unwrap();
        assert!(format!("{store:?}").starts_with("SpaceStore { path: "));
        assert_eq!(
            KeepError::Busy.to_string(),
            "another server keeps its spaces in this state directory"
        );
        let KeepError::Io(_) = KeepError::from(fs::TryLockError::Error(io::Error::other("lock")))
        else {
            unreachable!("a lock that fails is an input error");
        };
        for directory in [locked, unread] {
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
