// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spaces a server keeps across restarts, in its state directory. See `design/spaces.md`.

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The file the spaces are kept in, within the state directory.
pub const SPACES_FILE: &str = "spaces.json";

/// What is kept of one space: enough to list it again and bind it to its session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptSpace {
    pub id: String,
    pub title: String,
    pub backend: String,
    pub directory: String,
    /// The backend's identifier of the session the space is bound to.
    pub session: String,
    pub environment: String,
    pub attribution: bool,
    /// Why the space closed, if it has; a closed space is listed again closed, and not bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    /// When the space closed relative to the others, so that the least recently closed are still forgotten first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_order: Option<u64>,
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
        security::create_private_directory(directory)?;
        let path = directory.join(SPACES_FILE);
        let lock = File::create(path.with_extension("lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(KeepError::Busy),
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
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
        security::write_private(&self.path, text.as_bytes())?;
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
            backend: "fake".into(),
            directory: "/tmp".into(),
            session: "session-1".into(),
            environment: "sky".into(),
            attribution: false,
            closed: None,
            closed_order: None,
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
    fn one_server_keeps_a_directory_and_a_malformed_record_is_refused() {
        let directory = directory("refused");
        let store = SpaceStore::open(&directory).unwrap();
        assert!(matches!(SpaceStore::open(&directory), Err(KeepError::Busy)));
        drop(store);
        fs::write(directory.join(SPACES_FILE), "not json").unwrap();
        let error = SpaceStore::open(&directory).unwrap_err();
        assert!(matches!(error, KeepError::Format(_)));
        assert!(
            error
                .to_string()
                .starts_with("the record of spaces is malformed")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
