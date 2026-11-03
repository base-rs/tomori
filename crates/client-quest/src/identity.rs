// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who this device is to the server, and which server it last joined.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::RandomState},
    fs,
    hash::{BuildHasher, Hasher},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// The server's limit on a display name, in bytes.
const NAME_BYTES: usize = 64;
/// Identities stored before protocol 2, in the file `participant`, were sent to other clients and may be recorded in agent sessions, so they are never reused; see `design/security.md`.
const PARTICIPANT_FILE: &str = "participant-2";
const SERVER_FILE: &str = "server";
/// The certificate and token kept for each server; see [`Credential`].
const CREDENTIALS_FILE: &str = "credentials.json";

/// Settings given when the app starts, which take precedence over the defaults.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// A server address to connect to straight away.
    pub server: Option<String>,
    pub name: Option<String>,
    /// A token for the server this device connects to next, such as one from `tomori-server auth mint`. See `design/security.md`.
    pub token: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    /// A random identity, so the server recognizes a reconnect, or a restart where it is stored, as the same participant.
    pub participant: String,
    /// The display name other participants see and the agent reads before each message.
    pub name: String,
    /// A server address to connect to straight away.
    pub server: Option<String>,
    /// A token given at startup, presented to the server this device connects to and then kept for it.
    pub token: Option<String>,
}

/// The participant identity and tokens are secrets, so debugging these shows only whether they are set.
fn secret(value: &Option<String>) -> Option<&'static str> {
    value.as_ref().map(|_| "[redacted]")
}

impl std::fmt::Debug for Overrides {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Overrides")
            .field("server", &self.server)
            .field("name", &self.name)
            .field("token", &secret(&self.token))
            .finish()
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Identity")
            .field("participant", &"[redacted]")
            .field("name", &self.name)
            .field("server", &self.server)
            .field("token", &secret(&self.token))
            .finish()
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credential")
            .field("certificate", &self.certificate)
            .field("token", &secret(&self.token))
            .finish()
    }
}

impl Identity {
    pub fn new(store: &Store, device: Option<&str>, overrides: Overrides) -> Self {
        let name = overrides
            .name
            .as_deref()
            .and_then(name)
            .or_else(|| device.and_then(name))
            .unwrap_or_else(|| "Tomori".into());
        Self {
            participant: store.participant(),
            name,
            server: overrides
                .server
                .map(|server| server.trim().to_owned())
                .filter(|server| !server.is_empty()),
            token: overrides
                .token
                .map(|token| token.trim().to_owned())
                .filter(|token| !token.is_empty()),
        }
    }
}

/// A display name the server accepts: not blank, not starting with `/`, and within its length limit.
pub fn name(candidate: &str) -> Option<String> {
    let name = candidate.trim();
    if name.is_empty() || name.starts_with('/') {
        return None;
    }
    let mut end = name.len().min(NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    Some(name[..end].trim_end().to_owned())
}

/// Small values kept across restarts, in the app's private storage. Without a directory nothing is kept, and each run is a new participant.
pub struct Store {
    directory: Option<PathBuf>,
    /// Kept in memory too, so that a device without storage still remembers them while it runs.
    credentials: std::sync::Mutex<Option<HashMap<String, Credential>>>,
}

/// What this device keeps for one server: the certificate it pinned and the token it presents. See `design/security.md`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// The fingerprint of the server's TLS certificate, pinned the first time this device connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl Store {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            credentials: std::sync::Mutex::new(None),
        }
    }

    /// What is kept for `server`, a discovered server's instance name or an address.
    pub fn credential(&self, server: &str) -> Credential {
        self.with_credentials(|credentials| credentials.get(server).cloned().unwrap_or_default())
    }

    /// Pin `fingerprint` as `server`'s certificate.
    pub fn remember_certificate(&self, server: &str, fingerprint: &str) {
        self.change_credential(server, |credential| {
            credential.certificate = Some(fingerprint.to_owned())
        });
    }

    pub fn remember_token(&self, server: &str, token: &str) {
        self.change_credential(server, |credential| {
            credential.token = Some(token.to_owned())
        });
    }

    /// Forget `server`'s token, such as when the server refused it.
    pub fn forget_token(&self, server: &str) {
        self.change_credential(server, |credential| credential.token = None);
    }

    fn with_credentials<T>(
        &self,
        use_them: impl FnOnce(&mut HashMap<String, Credential>) -> T,
    ) -> T {
        let mut credentials = self
            .credentials
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let credentials = credentials.get_or_insert_with(|| {
            self.read(CREDENTIALS_FILE)
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default()
        });
        use_them(credentials)
    }

    fn change_credential(&self, server: &str, change: impl FnOnce(&mut Credential)) {
        let text = self.with_credentials(|credentials| {
            let credential = credentials.entry(server.to_owned()).or_default();
            change(credential);
            if *credential == Credential::default() {
                credentials.remove(server);
            }
            serde_json::to_string_pretty(credentials).expect("credentials serialize")
        });
        self.write(CREDENTIALS_FILE, &text);
    }

    /// The stored participant identity, created on first use.
    pub fn participant(&self) -> String {
        if let Some(stored) = self
            .read(PARTICIPANT_FILE)
            .filter(|id| valid_participant(id))
        {
            return stored;
        }
        let participant = random_participant();
        self.write(PARTICIPANT_FILE, &participant);
        participant
    }

    /// The instance name of the server last connected to.
    pub fn server(&self) -> Option<String> {
        self.read(SERVER_FILE)
            .filter(|instance| !instance.is_empty())
    }

    pub fn remember_server(&self, instance: &str) {
        if self.server().as_deref() != Some(instance) {
            self.write(SERVER_FILE, instance);
        }
    }

    fn read(&self, file: &str) -> Option<String> {
        let path = self.directory.as_deref()?.join(file);
        Some(fs::read_to_string(path).ok()?.trim().to_owned())
    }

    fn write(&self, file: &str, value: &str) {
        let Some(directory) = self.directory.as_deref() else {
            return;
        };
        if let Err(error) = write(directory, file, value) {
            log::warn!("Could not store {file} in {}: {error}", directory.display());
        }
    }
}

/// Write through a temporary file readable only by this user, since the values include tokens, so a crash never leaves a partial value.
fn write(directory: &Path, file: &str, value: &str) -> std::io::Result<()> {
    use std::io::Write;
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!("{file}.tmp"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&temporary)?.write_all(value.as_bytes())?;
    fs::rename(temporary, directory.join(file))
}

fn valid_participant(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 128 random bits as hexadecimal. `RandomState` is seeded from the operating system's random source.
fn random_participant() -> String {
    let mut id = String::new();
    for round in 0..2_u8 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u8(round);
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        hasher.write_u32(std::process::id());
        id.push_str(&format!("{:016x}", hasher.finish()));
    }
    id
}

/// The name of this computer, for a default display name on the desktop.
#[cfg(not(target_os = "android"))]
pub fn host_name() -> Option<String> {
    let output = std::process::Command::new("hostname").output().ok()?;
    let host = String::from_utf8(output.stdout).ok()?;
    // `studio.local` reads better as `studio`.
    let host = host.trim().split('.').next()?.to_owned();
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "tomori-identity-{name}-{}-{}",
            std::process::id(),
            random_participant()
        ));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn names_must_not_be_blank_or_start_with_a_slash() {
        assert_eq!(name("  Ada "), Some("Ada".into()));
        assert_eq!(name("   "), None);
        assert_eq!(name(""), None);
        assert_eq!(name(" /clear"), None);
        assert_eq!(name("Ada/Bea"), Some("Ada/Bea".into()));
    }

    #[test]
    fn long_names_are_cut_at_a_character_boundary() {
        let long = "é".repeat(40);
        let cut = name(&long).unwrap();
        assert!(cut.len() <= NAME_BYTES);
        assert_eq!(cut, "é".repeat(32));
    }

    #[test]
    fn the_name_override_wins_over_the_device_name() {
        let store = Store::new(None);
        let overrides = Overrides {
            server: Some(" 10.0.0.1:8765 ".into()),
            name: Some("Ada".into()),
            token: Some(" tomori_abc ".into()),
        };
        let identity = Identity::new(&store, Some("Quest 3"), overrides);
        assert_eq!(identity.name, "Ada");
        assert_eq!(identity.server.as_deref(), Some("10.0.0.1:8765"));
        assert_eq!(identity.token.as_deref(), Some("tomori_abc"));
    }

    #[test]
    fn debugging_never_shows_secrets() {
        let store = Store::new(None);
        let overrides = Overrides {
            server: None,
            name: None,
            token: Some("tomori_secret".into()),
        };
        let identity = Identity::new(&store, None, overrides.clone());
        let credential = Credential {
            certificate: None,
            token: Some("tomori_secret".into()),
        };
        for debugged in [
            format!("{overrides:?}"),
            format!("{identity:?}"),
            format!("{credential:?}"),
        ] {
            assert!(!debugged.contains("tomori_secret"), "{debugged}");
            assert!(!debugged.contains(&identity.participant), "{debugged}");
        }
    }

    #[test]
    fn credentials_are_kept_for_each_server() {
        let directory = directory("credentials");
        let store = Store::new(Some(directory.clone()));
        assert_eq!(store.credential("Studio"), Credential::default());
        store.remember_certificate("Studio", "ab");
        store.remember_token("Studio", "tomori_abc");
        store.remember_token("127.0.0.1:8765", "tomori_usb");
        let expected = Credential {
            certificate: Some("ab".into()),
            token: Some("tomori_abc".into()),
        };
        assert_eq!(store.credential("Studio"), expected);
        // Kept across restarts, readable only by this user.
        let again = Store::new(Some(directory.clone()));
        assert_eq!(again.credential("Studio"), expected);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(directory.join(CREDENTIALS_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Forgetting the token keeps the pin; forgetting the last value forgets the server.
        again.forget_token("Studio");
        assert_eq!(again.credential("Studio").token, None);
        assert_eq!(
            again.credential("Studio").certificate.as_deref(),
            Some("ab")
        );
        again.forget_token("127.0.0.1:8765");
        let text = fs::read_to_string(directory.join(CREDENTIALS_FILE)).unwrap();
        assert!(!text.contains("127.0.0.1"));
        fs::remove_dir_all(directory).unwrap();

        // Without storage, they are kept while the app runs.
        let memory = Store::new(None);
        memory.remember_token("Studio", "tomori_abc");
        assert_eq!(
            memory.credential("Studio").token.as_deref(),
            Some("tomori_abc")
        );
    }

    #[test]
    fn an_invalid_override_falls_back_to_the_device_then_a_default() {
        let store = Store::new(None);
        let overrides = Overrides {
            server: Some("  ".into()),
            name: Some("/me".into()),
            token: Some(" ".into()),
        };
        let identity = Identity::new(&store, Some("Quest 3"), overrides.clone());
        assert_eq!(identity.name, "Quest 3");
        assert_eq!(identity.server, None);
        assert_eq!(identity.token, None);
        let identity = Identity::new(&store, Some(""), overrides);
        assert_eq!(identity.name, "Tomori");
    }

    #[test]
    fn the_participant_is_kept_where_there_is_storage() {
        let directory = directory("kept");
        let store = Store::new(Some(directory.clone()));
        let first = store.participant();
        assert!(valid_participant(&first), "{first}");
        assert_eq!(Store::new(Some(directory.clone())).participant(), first);
        fs::write(directory.join(PARTICIPANT_FILE), "not hex").unwrap();
        let replaced = store.participant();
        assert_ne!(replaced, first);
        assert!(valid_participant(&replaced));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn an_identity_from_before_protocol_2_is_not_reused() {
        let directory = directory("legacy");
        fs::create_dir_all(&directory).unwrap();
        let legacy = "0123456789abcdef0123456789abcdef";
        fs::write(directory.join("participant"), legacy).unwrap();
        let participant = Store::new(Some(directory.clone())).participant();
        assert_ne!(participant, legacy);
        assert!(valid_participant(&participant));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn without_storage_each_run_is_a_new_participant() {
        let store = Store::new(None);
        assert_ne!(store.participant(), store.participant());
        assert_eq!(store.server(), None);
        store.remember_server("studio");
        assert_eq!(store.server(), None);
    }

    #[test]
    fn remembers_the_last_server() {
        let directory = directory("server");
        let store = Store::new(Some(directory.clone()));
        assert_eq!(store.server(), None);
        store.remember_server("studio");
        assert_eq!(store.server().as_deref(), Some("studio"));
        store.remember_server("attic");
        assert_eq!(
            Store::new(Some(directory.clone())).server().as_deref(),
            Some("attic")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
