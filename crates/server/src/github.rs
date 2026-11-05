// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The GitHub environment's data source, a prototype (issue #148): one repository's open issues and pull requests,
//! read with one GraphQL query through a token that only the server holds. Read-only. See `design/github.md`.
//!
//! The token is never sent to clients, put in a space's state, told to the agent or logged: [`Token`]'s `Debug` hides
//! it, it has no `Display`, and it is read only to make a request's `authorization` field.

use std::ffi::OsString;
use std::fmt;
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base_http::{
    Field, Method, Path, Request,
    body::{BodyFrame, BufferedBody, ReadableBody},
};
use base_http_client::{Client, Endpoint, Options};
use base_scheduler::{
    AddressHints, AddressInfoCollection, AddressInfoView, AddressResolution, AddressService,
    Cancellation, Clock,
};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::{block_on, or, zip};
use serde_json::Value;
use tomori_protocol::{Card, CardKind, Checks, Label, MAX_CARDS};

use crate::cards::{CardSource, Failure, Fetched, FetchedCard, Rate};

/// The environment's identifier, which clients map to its presentation.
pub const ENVIRONMENT: &str = "github";

/// GitHub's GraphQL endpoint.
pub const API: &str = "https://api.github.com/graphql";

/// How long one request may take, from resolving GitHub's address to reading the last of its answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The largest answer read, far above what [`QUERY`] returns.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// The open issues and pull requests, most recently updated first, with the remaining rate limit. GitHub counts it as
/// one point of the 5,000 an hour a token has.
const QUERY: &str = "query($owner: String!, $name: String!) {
  rateLimit { remaining resetAt }
  repository(owner: $owner, name: $name) {
    issues(first: 100, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes { number title url createdAt updatedAt author { login } labels(first: 10) { nodes { name color } } }
    }
    pullRequests(first: 100, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title url isDraft createdAt updatedAt author { login } labels(first: 10) { nodes { name color } }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      }
    }
  }
}";

/// The environment's layer of a space's instructions to the agent, about the cards of `repository`. See
/// `design/spaces.md`.
pub(crate) fn instructions(repository: &str) -> String {
    format!(
        "This space is in Tomori's GitHub environment, a prototype. Around the participants are cards for the open \
issues and pull requests of the GitHub repository {repository}: each shows its number, title and kind (issue, pull \
request or draft), and, for a pull request, whether its checks pass. The cards start in an inbox box, and \
participants pick them up and put them down to organise them; where they put them is shared with everyone in the \
space and kept with it, but nothing is changed on GitHub.

When a participant points at a card while speaking, their words are followed by a line naming it, such as \
\"[Pointing at the card for issue {repository}#97 ...]\": \"this one\" or \"this\" then means that issue or pull \
request. To find out more about one, read it with your own tools, such as `gh issue view 97 --repo {repository}` or \
`gh pr view 97 --repo {repository}`. Tomori's server reads GitHub with a token of its own, which you do not have and \
must not ask for. Change nothing on GitHub, such as labels, comments or state, unless a participant asks you to in \
words."
    )
}

/// A repository, `OWNER/NAME`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    owner: String,
    name: String,
}

impl FromStr for Repository {
    type Err = String;

    /// `OWNER/NAME`, each of letters, digits, `-`, `_` and `.`, as GitHub allows.
    fn from_str(text: &str) -> Result<Self, String> {
        let valid = |part: &str| {
            !part.is_empty()
                && part.len() <= 100
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        match text.split_once('/') {
            Some((owner, name)) if valid(owner) && valid(name) => Ok(Self {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            _ => Err(format!(
                "--github-repository needs OWNER/NAME, such as ioquatix/tomori, not {text:?}"
            )),
        }
    }
}

impl fmt::Display for Repository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.owner, self.name)
    }
}

/// A GitHub token. Its `Debug` hides it and it has no `Display`, so that it cannot reach a log by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token(hidden)")
    }
}

impl Token {
    /// A token, from text that is visible ASCII once trimmed, as GitHub's tokens are, so that it is always a valid
    /// field value.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty() && text.chars().all(|c| c.is_ascii_graphic()))
            .then(|| Self(text.to_owned()))
    }

    /// The token in `variable`, the value of `GITHUB_TOKEN`, or else what `command`, `gh auth token`, prints. Failures
    /// say where the token was looked for, never what was found.
    pub fn find(variable: Option<OsString>, command: &mut Command) -> Result<Self, String> {
        if let Some(token) = variable.and_then(|value| Self::new(&value.to_string_lossy())) {
            return Ok(token);
        }
        let missing =
            "--github-repository needs a token: set GITHUB_TOKEN, or sign in with `gh auth login`";
        let output = command
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("{missing} (running gh failed: {error})"))?;
        if !output.status.success() {
            return Err(format!("{missing} (`gh auth token` failed)"));
        }
        Self::new(&String::from_utf8_lossy(&output.stdout))
            .ok_or_else(|| format!("{missing} (`gh auth token` printed no token)"))
    }

    /// The token in `GITHUB_TOKEN`, or else what `gh auth token` prints, as [`Self::find`] finds it.
    pub fn from_environment() -> Result<Self, String> {
        Self::find(
            std::env::var_os("GITHUB_TOKEN"),
            Command::new("gh").args(["auth", "token"]),
        )
    }

    fn authorization(&self) -> String {
        format!("bearer {}", self.0)
    }
}

/// Where the GraphQL endpoint is, parsed from a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Api {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

impl Api {
    /// `https://HOST[:PORT]/PATH`, or `http://` for a test's service.
    fn parse(url: &str) -> Option<Self> {
        let (https, rest) = match url.split_once("://")? {
            ("https", rest) => (true, rest),
            ("http", rest) => (false, rest),
            _ => return None,
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok().filter(|port| *port != 0)?),
            None => (authority, if https { 443 } else { 80 }),
        };
        (!host.is_empty()).then(|| Self {
            https,
            host: host.to_owned(),
            port,
            path: format!("/{path}"),
        })
    }
}

/// One repository on GitHub, read with a token.
pub struct GitHub {
    repository: Repository,
    token: Token,
    api: Api,
    /// Trusted roots for TLS, made once.
    tls: Arc<rustls::ClientConfig>,
    /// How long a request may take; see [`REQUEST_TIMEOUT`].
    timeout: Duration,
}

impl fmt::Debug for GitHub {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitHub")
            .field("repository", &self.repository)
            .field("api", &self.api)
            .finish_non_exhaustive()
    }
}

impl GitHub {
    /// `repository`, read with `token` from [`API`].
    pub fn new(repository: Repository, token: Token) -> Self {
        Self::with_api(repository, token, API).expect("GitHub's own endpoint parses")
    }

    /// `repository`, read with `token` from the GraphQL endpoint at `url`, such as a test's. `None` for a URL that is
    /// not `http://` or `https://` with a host.
    pub fn with_api(repository: Repository, token: Token, url: &str) -> Option<Self> {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        Some(Self {
            repository,
            token,
            api: Api::parse(url)?,
            tls: Arc::new(tls),
            timeout: REQUEST_TIMEOUT,
        })
    }

    /// The query's request body.
    fn body(&self) -> Vec<u8> {
        serde_json::json!({
            "query": QUERY,
            "variables": {"owner": self.repository.owner, "name": self.repository.name},
        })
        .to_string()
        .into_bytes()
    }

    /// Ask GitHub, on a scheduler of this thread's own, and read its answer.
    fn exchange(&self) -> Result<Answer, String> {
        let scheduler = crate::system::refused("github scheduler")
            .and_then(|()| {
                NativeScheduler::with_options(NativeOptions {
                    workers: 1,
                    threads: 2,
                    capacity: 64,
                    ..Default::default()
                })
            })
            .map_err(|error| format!("could not start a scheduler: {error}"))?;
        let cancellation = Cancellation::new();
        let answer = block_on(async {
            let request = async {
                let answer = self.request(&scheduler, &cancellation).await;
                cancellation.cancel();
                answer
            };
            let timeout = async {
                let _ = scheduler.sleep(self.timeout, &cancellation).await;
                Err(format!(
                    "GitHub did not answer within {} seconds",
                    self.timeout.as_secs()
                ))
            };
            or(request, timeout).await
        });
        cancellation.cancel();
        block_on(scheduler.shutdown());
        answer
    }

    async fn request(
        &self,
        scheduler: &NativeScheduler,
        cancellation: &Cancellation,
    ) -> Result<Answer, String> {
        let address = scheduler
            .address_resolve(
                Some(&self.api.host),
                AddressService::Port(self.api.port),
                AddressHints::default(),
                cancellation,
            )
            .await
            .ok()
            .and_then(|addresses| addresses.entries().next().map(|entry| entry.address()))
            .ok_or_else(|| format!("could not resolve {}", self.api.host))?;
        let mut endpoint = Endpoint::new(address);
        endpoint.authority = self.api.host.clone().into();
        if self.api.https {
            endpoint = endpoint.with_tls(self.tls.clone(), self.api.host.clone());
        }
        let (client, driver) = Client::new(scheduler, endpoint, Options::default())
            .expect("the default options are valid");
        let local = cancellation.child();
        let (_, answer) = zip(driver.run(&local), async {
            let answer = self.call(&client, &local).await;
            local.cancel();
            answer
        })
        .await;
        answer
    }

    async fn call(
        &self,
        client: &Client<'_>,
        cancellation: &Cancellation,
    ) -> Result<Answer, String> {
        let mut request = Request::new(
            Method::POST,
            Path::from(self.api.path.clone()),
            BufferedBody::from(self.body()),
        );
        let authorization = self.token.authorization();
        for (name, value) in [
            ("authorization", authorization.as_str()),
            ("user-agent", "tomori-server"),
            ("content-type", "application/json"),
            ("accept", "application/json"),
            ("connection", "close"),
        ] {
            let field =
                Field::new(name, value).expect("a token is visible ASCII, a valid field value");
            request
                .fields
                .append(field)
                .expect("a request has room for these few fields");
        }
        let response = client
            .call(request, cancellation)
            .await
            .map_err(|error| format!("could not reach GitHub: {error}"))?;
        let header = |name: &str| {
            response
                .fields
                .values(name)
                .next()
                .and_then(|value| std::str::from_utf8(value).ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
        };
        let status = response.status.as_u16();
        let retry_after = header("retry-after");
        let reset = header("x-ratelimit-reset");
        let mut body = response.body;
        let mut bytes = Vec::new();
        loop {
            let frame = body
                .read(cancellation)
                .await
                .map_err(|error| format!("could not read GitHub's answer: {error}"))?;
            let end = frame.is_end();
            if let BodyFrame::Data { data, .. } = frame {
                if bytes.len() + data.len() > MAX_RESPONSE_BYTES {
                    return Err("GitHub's answer is too large".to_owned());
                }
                bytes.extend_from_slice(&data);
            }
            if end {
                break;
            }
        }
        Ok(Answer {
            status,
            retry_after,
            reset,
            body: bytes,
        })
    }
}

/// What GitHub answered.
struct Answer {
    status: u16,
    /// The `retry-after` field, in seconds.
    retry_after: Option<u64>,
    /// The `x-ratelimit-reset` field, in seconds since the epoch.
    reset: Option<u64>,
    body: Vec<u8>,
}

/// Seconds since the epoch now.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl CardSource for GitHub {
    fn source(&self) -> String {
        self.repository.to_string()
    }

    fn fetch(&self) -> Result<Fetched, Failure> {
        let answer = self.exchange().map_err(Failure::new)?;
        interpret(&self.repository, answer, unix_now())
    }
}

/// The cards in GitHub's `answer` for `repository`, at `now` in seconds since the epoch, or why there are none.
fn interpret(repository: &Repository, answer: Answer, now: u64) -> Result<Fetched, Failure> {
    let wait = |seconds: u64| Duration::from_secs(seconds);
    match answer.status {
        200 => {}
        401 => return Err(Failure::new("GitHub refused the server's token")),
        403 | 429 => {
            // A secondary rate limit says how long to wait; the primary one, when it resets.
            let retry = answer
                .retry_after
                .or_else(|| answer.reset.map(|reset| reset.saturating_sub(now)))
                .map(wait);
            return Err(Failure {
                message: format!("GitHub is limiting requests ({})", answer.status),
                retry_after: retry,
            });
        }
        status => return Err(Failure::new(format!("GitHub answered {status}"))),
    }
    let value: Value = serde_json::from_slice(&answer.body)
        .map_err(|_| Failure::new("GitHub's answer is not JSON"))?;
    let data = value.get("data").filter(|data| !data.is_null());
    let Some(repo) = data
        .and_then(|data| data.get("repository"))
        .filter(|repo| !repo.is_null())
    else {
        let message = value
            .pointer("/errors/0/message")
            .and_then(Value::as_str)
            .map_or_else(
                || "GitHub returned no repository".to_owned(),
                |message| format!("GitHub: {}", crate::space::one_line(message, 200)),
            );
        return Err(Failure::new(message));
    };
    let rate = data
        .and_then(|data| data.get("rateLimit"))
        .and_then(|rate| {
            Some(Rate {
                remaining: rate.get("remaining").and_then(Value::as_u64)?,
                reset: rate
                    .get("resetAt")
                    .and_then(Value::as_str)
                    .and_then(timestamp)
                    .map_or(Duration::ZERO, |reset| wait(reset.saturating_sub(now))),
            })
        });
    let nodes = |key: &str| {
        repo.pointer(&format!("/{key}/nodes"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut cards: Vec<(u64, FetchedCard)> = nodes("issues")
        .iter()
        .map(|node| (node, false))
        .chain(nodes("pullRequests").iter().map(|node| (node, true)))
        .filter_map(|(node, pull)| card(repository, node, pull, now))
        .collect();
    // Most recently updated first, then by number, so that the order is stable.
    cards.sort_by(|(a, first), (b, second)| {
        b.cmp(a)
            .then_with(|| second.card.number.cmp(&first.card.number))
    });
    cards.truncate(MAX_CARDS);
    Ok(Fetched {
        cards: cards.into_iter().map(|(_, card)| card).collect(),
        rate,
    })
}

/// The card for one issue or pull request `node`, with when it was last updated, in seconds since the epoch. `None`
/// for a node without a number or title.
fn card(repository: &Repository, node: &Value, pull: bool, now: u64) -> Option<(u64, FetchedCard)> {
    let number = node.get("number").and_then(Value::as_u64)?;
    let title = node.get("title").and_then(Value::as_str)?;
    let time = |key: &str| node.get(key).and_then(Value::as_str).and_then(timestamp);
    let opened = time("createdAt").unwrap_or(now);
    let updated = time("updatedAt").unwrap_or(opened);
    let kind = match (pull, node.get("isDraft").and_then(Value::as_bool)) {
        (false, _) => CardKind::Issue,
        (true, Some(true)) => CardKind::Draft,
        (true, _) => CardKind::PullRequest,
    };
    let checks = match node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(Value::as_str)
    {
        None => Checks::None,
        Some("SUCCESS") => Checks::Passing,
        Some("FAILURE" | "ERROR") => Checks::Failing,
        Some("PENDING" | "EXPECTED") => Checks::Pending,
        Some(_) => Checks::Unknown,
    };
    let labels = node
        .pointer("/labels/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|label| {
            Some(Label {
                name: crate::space::one_line(label.get("name").and_then(Value::as_str)?, 50),
                color: label
                    .get("color")
                    .and_then(Value::as_str)
                    .filter(|color| {
                        color.len() == 6 && color.chars().all(|c| c.is_ascii_hexdigit())
                    })
                    .unwrap_or("808080")
                    .to_owned(),
            })
        })
        .collect();
    let segment = if pull { "pull" } else { "issues" };
    let author = node
        .pointer("/author/login")
        .and_then(Value::as_str)
        .unwrap_or("ghost");
    Some((
        updated,
        FetchedCard {
            card: Card {
                id: format!("github://{repository}/{segment}/{number}"),
                number,
                kind,
                title: crate::space::one_line(title, 200),
                author: crate::space::one_line(author, 40),
                labels,
                checks,
                age: 0,
                placement: None,
            },
            opened,
            url: format!("https://github.com/{repository}/{segment}/{number}"),
        },
    ))
}

/// Seconds since the epoch of an ISO 8601 time in UTC, such as `2026-11-05T09:30:00Z`, as GitHub writes them.
fn timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let separators = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'Z'),
    ];
    // Digits everywhere but the separators.
    let shaped = bytes.len() == 20
        && bytes.iter().enumerate().all(|(at, byte)| {
            match separators.iter().find(|(position, _)| *position == at) {
                Some((_, separator)) => byte == separator,
                None => byte.is_ascii_digit(),
            }
        });
    if !shaped {
        return None;
    }
    let number = |range: std::ops::Range<usize>| {
        bytes[range]
            .iter()
            .fold(0, |value, digit| value * 10 + u64::from(digit - b'0'))
    };
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    let (hour, minute, second) = (number(11..13), number(14..16), number(17..19));
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days since the epoch, from Howard Hinnant's days_from_civil.
    let (year, month) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = year / 400;
    let of_era = year - era * 400;
    let of_year = (153 * month + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
