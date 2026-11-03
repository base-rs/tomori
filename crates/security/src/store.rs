// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::token::{self, TOKEN_PREFIX};
use crate::{Authenticator, Credentials, Principal, Verdict};

/// What a token makes its holder. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Grant {
    /// An owner on the whole server.
    Owner,
    /// A collaborator in one space, and a visitor everywhere else.
    Collaborator { space: String },
}

/// One issued token, as stored: never the token itself, only its hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// A short identifier for listing and revoking the token, which reveals nothing about it.
    pub id: String,
    pub name: String,
    pub grant: Grant,
    /// When it was issued, in seconds since the Unix epoch.
    pub issued: u64,
    hash: String,
}

impl Entry {
    /// The principal the token identifies.
    pub fn principal(&self) -> Principal {
        Principal {
            id: format!("token:{}", self.id),
            name: self.name.clone(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    tokens: Vec<Entry>,
}

/// Why the token store could not be read or written.
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    /// The file is not a token store.
    Format(serde_json::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Format(error) => write!(formatter, "the token store is malformed: {error}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

struct State {
    entries: Vec<Entry>,
}

/// The tokens a server has issued, kept in a file readable only by the server's user. The `tomori-server auth` commands and the running server share the file: each change is made under an exclusive lock on a lock file beside it, against the file as it is then, and [`reload`](Self::reload) picks up changes made by another process. See `design/security.md`.
pub struct TokenStore {
    path: PathBuf,
    state: RwLock<State>,
}

/// The token store's file in a state directory.
const TOKENS_FILE: &str = "tokens.json";

impl TokenStore {
    /// Open the store in `directory`, creating the directory, readable only by its user, if needed. A missing file is an empty store.
    pub fn open(directory: &Path) -> Result<Self, StoreError> {
        create_private_directory(directory)?;
        let path = directory.join(TOKENS_FILE);
        let entries = read(&path)?;
        Ok(Self {
            path,
            state: RwLock::new(State { entries }),
        })
    }

    /// Issue a token named `name` that grants `grant`. Returns its entry and the token, which is not kept and cannot be shown again.
    pub fn issue(&self, name: &str, grant: Grant) -> Result<(Entry, String), StoreError> {
        let token = token::generate();
        let entry = Entry {
            id: crate::hex::random(6),
            name: name.to_owned(),
            grant,
            issued: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
            hash: token::hash(&token),
        };
        let issued = entry.clone();
        self.change(move |entries| entries.push(entry))?;
        Ok((issued, token))
    }

    /// Revoke the token with identifier `id`. Returns false if there is none.
    pub fn revoke(&self, id: &str) -> Result<bool, StoreError> {
        let mut found = false;
        self.change(|entries| {
            let before = entries.len();
            entries.retain(|entry| entry.id != id);
            found = entries.len() != before;
        })?;
        Ok(found)
    }

    /// The issued tokens, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.read_state().entries.clone()
    }

    /// The entry for the principal `principal`, if its token is still issued.
    pub fn entry(&self, principal: &str) -> Option<Entry> {
        let id = principal.strip_prefix("token:")?;
        self.read_state()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    }

    /// Read the file again, for changes another process made. Returns whether the tokens changed. The file is read whole rather than trusting its modification time, which may not change between two quick writes.
    pub fn reload(&self) -> Result<bool, StoreError> {
        let entries = read(&self.path)?;
        let mut state = self.write_state();
        if entries == state.entries {
            return Ok(false);
        }
        state.entries = entries;
        Ok(true)
    }

    /// Apply `change` to the entries as they are in the file now, under the lock, and write them back.
    fn change(&self, change: impl FnOnce(&mut Vec<Entry>)) -> Result<(), StoreError> {
        let lock = File::create(self.path.with_extension("lock"))?;
        lock.lock()?;
        let mut entries = read(&self.path)?;
        change(&mut entries);
        write(&self.path, &entries)?;
        self.write_state().entries = entries;
        lock.unlock()?;
        Ok(())
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state
            .read()
            .expect("the token store is never left poisoned")
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state
            .write()
            .expect("the token store is never left poisoned")
    }
}

impl Authenticator for TokenStore {
    /// Tokens this server issued. A token with the prefix that is not issued, or no longer, passes to the next authenticator, and is refused if none accepts it.
    fn authenticate(&self, credentials: &Credentials) -> Verdict {
        let Some(token) = credentials.token.as_deref() else {
            return Verdict::Pass;
        };
        if !token.starts_with(TOKEN_PREFIX) {
            return Verdict::Pass;
        }
        let hash = token::hash(token);
        match self
            .read_state()
            .entries
            .iter()
            .find(|entry| entry.hash == hash)
        {
            Some(entry) => Verdict::Authenticated(entry.principal()),
            None => Verdict::Pass,
        }
    }
}

fn read(path: &Path) -> Result<Vec<Entry>, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let file: Stored = serde_json::from_str(&text).map_err(StoreError::Format)?;
    Ok(file.tokens)
}

/// Write through a temporary file readable only by its user, so that the store is never partly written or readable by others.
fn write(path: &Path, entries: &[Entry]) -> io::Result<()> {
    let text = serde_json::to_string_pretty(&Stored {
        tokens: entries.to_vec(),
    })
    .expect("entries serialize");
    let temporary = path.with_extension("tmp");
    let mut file = private(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)
}

/// Create (or truncate) a file readable and writable only by its user.
pub(crate) fn private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Create a directory, and its parents, readable only by its user.
pub(crate) fn create_private_directory(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("tomori-security-{name}-{}", crate::hex::random(4)));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    fn presented(token: &str) -> Credentials {
        Credentials {
            token: Some(token.to_owned()),
        }
    }

    #[test]
    fn issued_tokens_authenticate_and_are_stored_only_as_hashes() {
        let directory = directory("issue");
        let store = TokenStore::open(&directory).unwrap();
        let (entry, token) = store.issue("Ada's Quest", Grant::Owner).unwrap();
        assert_eq!(entry.name, "Ada's Quest");
        assert_eq!(
            store.authenticate(&presented(&token)),
            Verdict::Authenticated(Principal {
                id: format!("token:{}", entry.id),
                name: "Ada's Quest".into(),
            })
        );
        assert_eq!(store.entry(&entry.principal().id), Some(entry.clone()));
        assert_eq!(store.entry("token:missing"), None);
        assert_eq!(store.entry("other:principal"), None);

        let text = fs::read_to_string(directory.join(TOKENS_FILE)).unwrap();
        assert!(!text.contains(&token));
        assert!(text.contains(&token::hash(&token)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&directory.join(TOKENS_FILE)), 0o600);
            assert_eq!(mode(&directory), 0o700);
        }

        // The store survives reopening, as after a restart.
        let reopened = TokenStore::open(&directory).unwrap();
        assert_eq!(reopened.entries(), [entry]);
        assert!(matches!(
            reopened.authenticate(&presented(&token)),
            Verdict::Authenticated(_)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn other_tokens_pass_to_the_next_authenticator() {
        let directory = directory("pass");
        let store = TokenStore::open(&directory).unwrap();
        assert_eq!(store.authenticate(&Credentials::default()), Verdict::Pass);
        assert_eq!(
            store.authenticate(&presented("someone-elses")),
            Verdict::Pass
        );
        assert_eq!(
            store.authenticate(&presented(&token::generate())),
            Verdict::Pass
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn revoked_tokens_are_refused_and_changes_from_another_process_are_reloaded() {
        let directory = directory("revoke");
        let server = TokenStore::open(&directory).unwrap();
        let command = TokenStore::open(&directory).unwrap();
        let (entry, token) = command
            .issue(
                "Bob",
                Grant::Collaborator {
                    space: "space-1".into(),
                },
            )
            .unwrap();
        // The server does not see the command's token until it reloads.
        assert_eq!(server.authenticate(&presented(&token)), Verdict::Pass);
        assert!(server.reload().unwrap());
        assert!(!server.reload().unwrap());
        assert!(matches!(
            server.authenticate(&presented(&token)),
            Verdict::Authenticated(_)
        ));

        assert!(command.revoke(&entry.id).unwrap());
        assert!(!command.revoke(&entry.id).unwrap());
        assert!(server.reload().unwrap());
        assert_eq!(server.authenticate(&presented(&token)), Verdict::Pass);
        assert!(server.entries().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changes_apply_to_the_file_as_it_is_now() {
        let directory = directory("concurrent");
        let first = TokenStore::open(&directory).unwrap();
        let second = TokenStore::open(&directory).unwrap();
        first.issue("one", Grant::Owner).unwrap();
        // The second store has not reloaded, but its change keeps the first's token.
        second.issue("two", Grant::Owner).unwrap();
        let names: Vec<String> = second
            .entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["one", "two"]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_malformed_or_unreadable_store_is_an_error() {
        let directory = directory("malformed");
        create_private_directory(&directory).unwrap();
        fs::write(directory.join(TOKENS_FILE), "not json").unwrap();
        let error = TokenStore::open(&directory).err().unwrap();
        assert!(
            error
                .to_string()
                .starts_with("the token store is malformed"),
            "{error}"
        );

        fs::remove_file(directory.join(TOKENS_FILE)).unwrap();
        fs::create_dir(directory.join(TOKENS_FILE)).unwrap();
        let error = TokenStore::open(&directory).err().unwrap();
        assert!(matches!(error, StoreError::Io(_)), "{error}");
        assert!(!error.to_string().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }
}
