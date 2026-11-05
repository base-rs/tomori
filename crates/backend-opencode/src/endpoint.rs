// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Resolution of where the OpenCode server is and how to authenticate to it.
//!
//! The canonical order is: an explicit URL given to the backend (typically from
//! `--opencode-url`), the `OPENCODE_URL` environment variable, the `opencode
//! serve --service` registration file in the XDG state directory, and finally
//! the well-known default `http://127.0.0.1:4096`.

/// A resolved OpenCode service endpoint: its host and port, and optional HTTP
/// basic-auth credentials.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub host: String,
    pub port: u16,
    /// `(username, password)` when the server requires HTTP basic auth.
    pub auth: Option<(String, String)>,
}

impl Resolved {
    /// The value of the `Host` header, `host:port`.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The `Authorization` header value for basic auth, when present.
    pub fn authorization_header(&self) -> Option<String> {
        let (user, password) = self.auth.as_ref()?;
        Some(format!(
            "Basic {}",
            base64_basic(&format!("{user}:{password}"))
        ))
    }
}

/// The well-known default when no flag, env var or service file is present.
pub const DEFAULT_URL: &str = "http://127.0.0.1:4096";

/// The directory `opencode serve --service` writes its registration file to,
/// in the XDG state directory: `$XDG_STATE_HOME/opencode`, else
/// `$HOME/.local/state/opencode`.
pub fn xdg_state_dir() -> Option<std::path::PathBuf> {
    state_dir_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// [`xdg_state_dir`] from the values of `XDG_STATE_HOME` and `HOME`.
fn state_dir_from(
    state_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    let base = state_home
        .map(std::path::PathBuf::from)
        .or_else(|| home.map(|home| std::path::PathBuf::from(home).join(".local").join("state")))?;
    Some(base.join("opencode"))
}

/// Parse an `http://host:port` URL into a [`Resolved`]. `None` for a malformed
/// URL, an unsupported scheme, `localhost` without a port, or a port out of
/// range.
pub fn resolve_url(url: &str) -> Option<Resolved> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    // Splitting always yields the part before the first separator.
    let hostport = rest
        .split(['/', '?'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) => (host, port),
        // A bare hostname means port 443 for https and 80 for http, but OpenCode
        // always listens on an explicit port; refuse the ambiguity.
        _ => return None,
    };
    let host = host.to_owned();
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    Some(Resolved {
        host,
        port,
        auth: None,
    })
}

/// Read the `opencode serve --service` registration file at `path` (normally
/// `service.json` in [`xdg_state_dir`]), returning the recorded URL and
/// credentials. The file is a JSON object with fields `id`, `version`, `url`,
/// `pid`, `password`.
pub fn read_service_file_at(path: &std::path::Path) -> Option<Resolved> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let url = value.get("url")?.as_str()?;
    let mut resolved = resolve_url(url)?;
    if let Some(password) = value.get("password").and_then(|v| v.as_str())
        && !password.is_empty()
    {
        let username = value
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("opencode")
            .to_owned();
        resolved.auth = Some((username, password.to_owned()));
    }
    Some(resolved)
}

/// Where a resolved endpoint came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Explicit,
    Environment,
    ServiceFile,
    Default,
}

/// Resolve the OpenCode service in the canonical order: `explicit`, the
/// `OPENCODE_URL` environment variable, the XDG state file, then the default
/// URL.
#[cfg(test)]
pub fn resolve(explicit: Option<&str>) -> Resolved {
    Resolver::new(explicit.map(str::to_owned)).sources().0
}

/// Finds the service before every connection attempt, because a restarted
/// service may record another port and password. Once the service has been
/// found through the environment or its registration file, it is never
/// assumed to be at the default address instead: a registration file that is
/// missing or unreadable mid-run means the service has not recorded itself
/// yet, so the attempt waits for it rather than reaching whatever listens on
/// the default port.
pub struct Resolver {
    explicit: Option<String>,
    found: Option<Source>,
}

impl Resolver {
    pub fn new(explicit: Option<String>) -> Self {
        Self {
            explicit,
            found: None,
        }
    }

    fn sources(&self) -> (Resolved, Source) {
        let environment = std::env::var("OPENCODE_URL").ok();
        let service_file = xdg_state_dir().map(|directory| directory.join("service.json"));
        resolve_from(
            self.explicit.as_deref(),
            environment.as_deref(),
            service_file.as_deref(),
        )
    }

    /// The service's endpoint for the next attempt, or why it is not known.
    pub fn next(&mut self) -> Result<Resolved, String> {
        let resolution = self.sources();
        self.accept(resolution)
    }

    /// [`next`](Self::next) with its sources given.
    #[cfg(test)]
    pub fn next_from(
        &mut self,
        environment: Option<&str>,
        service_file: Option<&std::path::Path>,
    ) -> Result<Resolved, String> {
        let resolution = resolve_from(self.explicit.as_deref(), environment, service_file);
        self.accept(resolution)
    }

    fn accept(&mut self, (resolved, source): (Resolved, Source)) -> Result<Resolved, String> {
        match (source, self.found) {
            (Source::Default, Some(found)) if found != Source::Default => Err(format!(
                "the OpenCode service has not recorded its address yet (it was found through {})",
                match found {
                    Source::Environment => "OPENCODE_URL",
                    _ => "its service.json",
                }
            )),
            _ => {
                self.found = Some(source);
                Ok(resolved)
            }
        }
    }
}

/// [`Resolver`]'s order with its sources given: the explicit URL, the value
/// of `OPENCODE_URL`, and the path of the registration file.
pub fn resolve_from(
    explicit: Option<&str>,
    environment: Option<&str>,
    service_file: Option<&std::path::Path>,
) -> (Resolved, Source) {
    for (url, source) in [
        (explicit, Source::Explicit),
        (environment, Source::Environment),
    ] {
        if let Some(url) = url.map(str::trim).filter(|url| !url.is_empty())
            && let Some(resolved) = resolve_url(url)
        {
            return (resolved, source);
        }
    }
    if let Some(path) = service_file {
        // A file that exists but cannot be read may be mid-rewrite by a
        // restarting service; it is read once more before going on.
        for again in [false, true] {
            if let Some(resolved) = read_service_file_at(path) {
                return (resolved, Source::ServiceFile);
            }
            if again || !path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    (
        resolve_url(DEFAULT_URL).expect("the default URL is well-formed"),
        Source::Default,
    )
}

/// The standard base64 alphabet with padding, as HTTP basic auth requires:
/// opencode's auth middleware rejects an unpadded value.
fn base64_basic(bytes: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.as_bytes().chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_expected() {
        // The RFC 4628 examples.
        assert_eq!(base64_basic("opencode:secret"), "b3BlbmNvZGU6c2VjcmV0");
        assert_eq!(base64_basic("a"), "YQ==");
        assert_eq!(base64_basic("a b"), "YSBi");
        assert_eq!(base64_basic("ab"), "YWI=");
        assert_eq!(base64_basic(""), "");
    }

    #[test]
    fn the_state_directory_follows_xdg_then_home() {
        assert_eq!(
            state_dir_from(Some("/state".into()), Some("/home/ada".into())),
            Some(std::path::PathBuf::from("/state/opencode"))
        );
        assert_eq!(
            state_dir_from(None, Some("/home/ada".into())),
            Some(std::path::PathBuf::from("/home/ada/.local/state/opencode"))
        );
        assert_eq!(state_dir_from(None, None), None);
    }

    #[test]
    fn a_service_file_is_read_only_when_it_names_a_valid_url() {
        let directory = std::env::temp_dir().join(format!(
            "tomori-opencode-service-file-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("service.json");
        let read = |text: &str| {
            std::fs::write(&path, text).unwrap();
            read_service_file_at(&path).map(|resolved| (resolved.authority(), resolved.auth))
        };
        for text in ["not json", "{}", r#"{"url": 7}"#, r#"{"url": "localhost"}"#] {
            assert_eq!(read(text), None, "{text}");
        }
        assert_eq!(
            read(r#"{"url": "http://127.0.0.1:4096", "password": ""}"#),
            Some(("127.0.0.1:4096".to_owned(), None))
        );
        assert_eq!(
            read(r#"{"url": "http://127.0.0.1:4096", "password": "secret", "username": "ada"}"#),
            Some((
                "127.0.0.1:4096".to_owned(),
                Some(("ada".to_owned(), "secret".to_owned()))
            ))
        );
        // A file that exists but cannot be read is read once more, as one being replaced might be, then passed over.
        std::fs::write(&path, "{").unwrap();
        assert_eq!(resolve_from(None, None, Some(&path)).1, Source::Default);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn url_resolution_reads_host_and_port() {
        assert!(resolve_url("127.0.0.1:4096").is_none(), "no scheme");
        assert!(resolve_url("http://localhost").is_none(), "no port");
        assert!(resolve_url("ftp://127.0.0.1:4096").is_none());
        assert!(resolve_url("http://127.0.0.1:0").is_none());
        assert!(resolve_url("http://127.0.0.1:99999").is_none());
        let resolved = resolve_url(DEFAULT_URL).unwrap();
        assert_eq!(resolved.host, "127.0.0.1");
        assert_eq!(resolved.port, 4096);
        assert!(resolved.auth.is_none());
        assert_eq!(resolved.authority(), "127.0.0.1:4096");
        let resolved = resolve_url("http://example.com:8080/path").unwrap();
        assert_eq!(resolved.host, "example.com");
        assert_eq!(resolved.port, 8080);
    }

    #[test]
    fn a_changed_service_file_is_read_again_and_the_environment_overrides_it() {
        let directory = std::env::temp_dir().join(format!(
            "tomori-opencode-endpoint-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("service.json");
        std::fs::write(
            &path,
            r#"{"url":"http://127.0.0.1:4100","password":"first"}"#,
        )
        .unwrap();
        let first = resolve_from(None, None, Some(&path)).0;
        assert_eq!(first.port, 4100);
        assert_eq!(first.auth, Some(("opencode".into(), "first".into())));
        // A restarted service records another port and password.
        std::fs::write(
            &path,
            r#"{"url":"http://127.0.0.1:4200","password":"second"}"#,
        )
        .unwrap();
        let second = resolve_from(None, None, Some(&path)).0;
        assert_eq!(second.port, 4200);
        assert_eq!(second.auth, Some(("opencode".into(), "second".into())));
        let overridden = resolve_from(None, Some(" http://127.0.0.1:4300 "), Some(&path)).0;
        assert_eq!(overridden.port, 4300);
        assert!(overridden.auth.is_none());
        let explicit = resolve_from(
            Some("http://127.0.0.1:4400"),
            Some("http://127.0.0.1:4300"),
            Some(&path),
        )
        .0;
        assert_eq!(explicit.port, 4400);
        // Once the service was found through its file, a missing file means
        // it has not recorded itself yet, not that it moved to the default.
        let mut resolver = Resolver::new(None);
        assert_eq!(resolver.next_from(None, Some(&path)).unwrap().port, 4200);
        std::fs::remove_file(&path).unwrap();
        let waiting = resolver.next_from(None, Some(&path)).unwrap_err();
        assert!(waiting.contains("service.json"), "{waiting}");
        std::fs::write(
            &path,
            r#"{"url":"http://127.0.0.1:4500","password":"third"}"#,
        )
        .unwrap();
        assert_eq!(resolver.next_from(None, Some(&path)).unwrap().port, 4500);
        // Likewise for OPENCODE_URL, should it be unset.
        let mut resolver = Resolver::new(None);
        assert_eq!(
            resolver
                .next_from(Some("http://127.0.0.1:4300"), None)
                .unwrap()
                .port,
            4300
        );
        assert!(resolver.next_from(None, None).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
        // Without the file, and before anything else was found, the
        // well-known default applies.
        let (default, source) = resolve_from(None, Some(""), Some(&path));
        assert_eq!((default.port, source), (4096, Source::Default));
        assert_eq!(
            Resolver::new(None)
                .next_from(None, Some(&path))
                .unwrap()
                .port,
            4096
        );
    }

    #[test]
    fn the_authorization_header_uses_basic_auth() {
        let mut resolved = resolve_url(DEFAULT_URL).unwrap();
        resolved.auth = Some(("opencode".to_owned(), "secret".to_owned()));
        assert_eq!(
            resolved.authorization_header().as_deref(),
            Some("Basic b3BlbmNvZGU6c2VjcmV0")
        );
        // A credential whose base64 needs padding must still be padded;
        // opencode's auth middleware rejects unpadded values.
        let mut resolved = resolve_url(DEFAULT_URL).unwrap();
        resolved.auth = Some(("opencode".to_owned(), "abcdefghijklmnopqrstuv".to_owned()));
        assert_eq!(
            resolved.authorization_header().as_deref(),
            Some("Basic b3BlbmNvZGU6YWJjZGVmZ2hpamtsbW5vcHFyc3R1dg=="),
        );
    }
}
