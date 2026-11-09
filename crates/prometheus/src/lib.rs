// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A generic Prometheus data source for Tomori environments' server sides: a blocking [`Client`] for the Prometheus HTTP API's instant and range queries, with an endpoint and a renewable bearer token that the application supplies and that never reach clients, bounds on time and size, typed results with explicit timestamps, and a [`LabelFilter`] that keeps only the labels a server side may pass on. See Tomori's `design/data-sources.md`.
//!
//! ```no_run
//! use std::time::Duration;
//! use tomori_prometheus::{Client, Data, LabelFilter, Range, Timestamp, Token};
//!
//! let client = Client::builder("https://prometheus.example")
//!     .token_source(Token::new("a-token").unwrap())
//!     .labels(LabelFilter::only(["namespace", "pod"]))
//!     .build()?;
//! // Evaluate every query of one poll at the same instant, so that they agree.
//! let at = Timestamp::now().floor(Duration::from_secs(5));
//! if let Data::Vector(samples) = client.query("sum by (pod) (rate(http_requests_total[1m]))", at)?.data {
//!     for sample in samples {
//!         println!("{:?} {} at {}", sample.metric, sample.point.value, sample.point.at);
//!     }
//! }
//! let range = Range::new(at - Duration::from_secs(3600), at, Duration::from_secs(60));
//! for series in client.query_range("up", range)?.data {
//!     println!("{:?}: {} points", series.metric, series.points.len());
//! }
//! # Ok::<(), tomori_prometheus::Error>(())
//! ```
//!
//! The crate logs nothing. The token is sent only in the `authorization` field of the client's own requests, and is in no error, `Debug` output or log; errors and warnings quote the source's own text only scrubbed of the token and the endpoint (see [`Error`]).
use std::fmt;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use base_scheduler::Cancellation;
use base_scheduler_native::{NativeOptions, NativeScheduler};

mod data;
mod endpoint;
mod error;
mod labels;
pub mod promql;
mod system;
mod time;
mod token;
mod worker;

#[cfg(test)]
mod tests;

pub use data::{Data, Labels, Point, Response, Sample, Series};
pub use error::{ENDPOINT, Error, QUOTED_BYTES, REDACTED};
pub use labels::LabelFilter;
pub use time::{Range, Timestamp};
pub use token::{Token, TokenSource};

use data::Answer;
use error::Scrub;
use worker::{Job, Outgoing, Reply, Settings};

/// The README's examples, checked as documentation tests.
#[doc = include_str!("../readme.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

/// How long a call may take unless the application sets another time-out.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a client keeps a token before asking its source for another, unless the application sets another lifetime: 40 minutes, within the hour an identity token usually lasts.
pub const DEFAULT_TOKEN_LIFETIME: Duration = Duration::from_secs(40 * 60);

/// The largest answer a client reads unless the application sets another limit: 8 MiB.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// The longest expression a client sends unless the application sets another bound: 16 KiB, as gateways commonly allow.
pub const DEFAULT_MAX_QUERY_BYTES: usize = 16 * 1024;

/// Where the Prometheus HTTP API is beneath an endpoint unless the application names another prefix.
pub const DEFAULT_API_PREFIX: &str = "/api/v1";

/// The most points a range query's series may have, as Prometheus allows.
pub const MAX_POINTS: u64 = 11_000;

/// The longest time-out a client takes: a longer one is taken as this, a day.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// How long dropping a client's last clone waits for its thread to stop before leaving it to stop on its own, as it must when the system's resolver is stuck on a lookup, which nothing can interrupt.
pub const DROP_WAIT: Duration = Duration::from_secs(1);

/// How a client sends a query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryMethod {
    /// Form-encoded in the body of a `POST`, which no limit on a URL's length constrains. The default.
    #[default]
    Post,
    /// In the URL of a `GET`, for a source that does not take `POST`.
    Get,
}

/// The configuration of a [`Client`], from [`Client::builder`].
pub struct Builder {
    endpoint: String,
    tokens: Option<Arc<dyn TokenSource>>,
    timeout: Duration,
    token_lifetime: Duration,
    max_response_bytes: usize,
    max_query_bytes: usize,
    api_prefix: String,
    method: QueryMethod,
    labels: LabelFilter,
    user_agent: String,
    /// [`DROP_WAIT`], or a test's.
    drop_wait: Duration,
    #[cfg(test)]
    resolver: Option<worker::Resolver>,
    #[cfg(test)]
    attempts: Option<Arc<Mutex<Vec<std::net::SocketAddr>>>>,
    /// Trusted roots of a test's own, in place of the web's.
    #[cfg(test)]
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl Builder {
    /// Authenticate with bearer tokens from `source`. Without a source, the client sends no `authorization` field.
    pub fn token_source(mut self, source: impl TokenSource + 'static) -> Self {
        self.tokens = Some(Arc::new(source));
        self
    }

    /// How long each request may take, from when it is queued, waiting for other callers' requests included, until its answer is read; [`DEFAULT_TIMEOUT`] otherwise, and at most [`MAX_TIMEOUT`], which a longer one is taken as. It is sent as Prometheus's evaluation `timeout` too. A call whose first request is refused with HTTP 401 asks the token source again and sends a second request with a time-out of its own, so a call can take twice the time-out, and the token source's time, which is not bounded here, besides.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// How long a token is kept before the source is asked for another; [`DEFAULT_TOKEN_LIFETIME`] otherwise.
    pub fn token_lifetime(mut self, lifetime: Duration) -> Self {
        self.token_lifetime = lifetime;
        self
    }

    /// The largest answer read, in bytes; [`DEFAULT_MAX_RESPONSE_BYTES`] otherwise.
    pub fn max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }

    /// The longest expression sent, in bytes; [`DEFAULT_MAX_QUERY_BYTES`] otherwise.
    pub fn max_query_bytes(mut self, bytes: usize) -> Self {
        self.max_query_bytes = bytes;
        self
    }

    /// Where the API is beneath the endpoint, such as `/api/v1`, the default, or a gateway's own; empty for the endpoint itself.
    pub fn api_prefix(mut self, prefix: &str) -> Self {
        self.api_prefix = prefix.to_owned();
        self
    }

    /// How queries are sent; [`QueryMethod::Post`] otherwise.
    pub fn method(mut self, method: QueryMethod) -> Self {
        self.method = method;
        self
    }

    /// The labels kept of every answer; every label otherwise.
    pub fn labels(mut self, filter: LabelFilter) -> Self {
        self.labels = filter;
        self
    }

    /// The `user-agent` field; `tomori-prometheus/VERSION` otherwise.
    pub fn user_agent(mut self, user_agent: &str) -> Self {
        self.user_agent = user_agent.to_owned();
        self
    }

    /// The client, with a thread of its own, or why it cannot be made: an endpoint, prefix or limit that cannot be used, or a system out of threads.
    pub fn build(self) -> Result<Client, Error> {
        let endpoint = endpoint::parse(&self.endpoint)?;
        let prefix = endpoint::prefix(&self.api_prefix)?;
        if self.timeout.is_zero() || self.max_response_bytes == 0 || self.max_query_bytes == 0 {
            return Err(Error::Invalid(
                "the time-out and limits must be more than zero".into(),
            ));
        }
        if self.user_agent.is_empty()
            || !self
                .user_agent
                .bytes()
                .all(|byte| byte == b' ' || byte.is_ascii_graphic())
        {
            return Err(Error::Invalid(
                "the user agent must be printable ASCII".into(),
            ));
        }
        let path = format!("{}{prefix}", endpoint.base);
        let (host, base) = (endpoint.host.clone(), endpoint.base.clone());
        #[cfg(test)]
        let tls = {
            let roots = self.tls;
            move || roots.unwrap_or_else(tls)
        };
        let settings = Settings {
            tls: endpoint.https.then(tls),
            endpoint,
            user_agent: self.user_agent,
            max_response_bytes: self.max_response_bytes,
            #[cfg(test)]
            resolver: self.resolver,
            #[cfg(test)]
            attempts: self.attempts,
        };
        let scheduler = system::refused("prometheus scheduler")
            .and_then(|()| {
                NativeScheduler::with_options(NativeOptions {
                    workers: 1,
                    threads: 2,
                    capacity: 16,
                    ..Default::default()
                })
            })
            .map_err(|error| Error::Transport(format!("could not start a scheduler: {error}")))?;
        let (jobs, receiver) = async_channel::unbounded();
        // Dropped as the thread ends, which is how dropping the client learns that it has.
        let (finished, done) = sync_channel::<()>(0);
        let thread = system::refused("prometheus thread")
            .and_then(|()| {
                thread::Builder::new()
                    .name("tomori-prometheus".into())
                    .spawn(move || {
                        let _finished = finished;
                        worker::run(scheduler, receiver, settings);
                    })
            })
            .map_err(|error| Error::Transport(format!("could not start a thread: {error}")))?;
        Ok(Client {
            shared: Arc::new(Shared {
                jobs,
                thread: Some(thread),
                done: Mutex::new(done),
                drop_wait: self.drop_wait,
                tokens: self.tokens,
                token: Mutex::new(Kept::default()),
                token_lifetime: self.token_lifetime,
                timeout: self.timeout.min(MAX_TIMEOUT),
                max_query_bytes: self.max_query_bytes,
                path,
                method: self.method,
                labels: self.labels,
                host,
                base,
            }),
        })
    }
}

/// The web's public roots, which an `https://` endpoint's certificate is checked against.
fn tls() -> Arc<rustls::ClientConfig> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring supports the default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    Arc::new(config)
}

/// A blocking Prometheus client, for a server side's own thread.
///
/// It sends one request at a time, on a thread and scheduler of its own, over one connection that it keeps for the next request and replaces after any failure, trying each of the host's addresses in turn until one connects; a caller on another thread waits its turn, within its time-out. Clones share the thread, the connection and the token. Dropping the last stops the thread and waits for it, for at most [`DROP_WAIT`], since a lookup stuck in the system's resolver cannot be interrupted; the thread then stops on its own once the lookup ends.
///
/// It sends a request again only when the endpoint refused it with HTTP 401, once, with a new token from its source, unless another caller has renewed the token meanwhile. Nothing else is retried: a 429 or 5xx is reported as [`Error::Status`], or [`Error::Query`] when Prometheus explains it, with the `retry-after` the source gave, for the caller to wait as it polls at its own cadence; a request that meets a connection the source has just closed fails as [`Error::Transport`], and the next call opens a new one. Redirects are not followed, so a token is never sent to another place.
#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
}

struct Shared {
    jobs: async_channel::Sender<Job>,
    /// The client's thread, which is joined once every clone has gone.
    thread: Option<thread::JoinHandle<()>>,
    /// Dropped as the client's thread ends.
    done: Mutex<Receiver<()>>,
    drop_wait: Duration,
    tokens: Option<Arc<dyn TokenSource>>,
    token: Mutex<Kept>,
    token_lifetime: Duration,
    timeout: Duration,
    max_query_bytes: usize,
    /// The endpoint's path and the API's prefix, before each call.
    path: String,
    method: QueryMethod,
    labels: LabelFilter,
    /// The endpoint's host and path, which quoted text is scrubbed of.
    host: String,
    base: String,
}

/// The token last given, when, and how many have been given, so that a caller whose request was refused renews only the token it sent.
#[derive(Default)]
struct Kept {
    token: Option<(Token, Instant)>,
    generation: u64,
}

impl Drop for Shared {
    /// Stop the client's thread, and wait for it for at most [`DROP_WAIT`]: no caller waits for what it is doing then, so it has only to drop it and stop its scheduler, unless its scheduler waits for a lookup stuck in the system's resolver, when the thread is left to stop on its own.
    fn drop(&mut self) {
        self.jobs.close();
        let done = self.done.get_mut().unwrap_or_else(PoisonError::into_inner);
        if done.recv_timeout(self.drop_wait) == Err(RecvTimeoutError::Timeout) {
            return;
        }
        let _ = self.thread.take().map(thread::JoinHandle::join);
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shared = &self.shared;
        formatter
            .debug_struct("Client")
            .field("authenticated", &shared.tokens.is_some())
            .field("timeout", &shared.timeout)
            .field("method", &shared.method)
            .field("labels", &shared.labels)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for the Prometheus HTTP API at `endpoint`, `https://HOST[:PORT][/PATH]`, or `http://` only to a loopback host, configured before [`Builder::build`] makes it.
    pub fn builder(endpoint: &str) -> Builder {
        Builder {
            endpoint: endpoint.to_owned(),
            tokens: None,
            timeout: DEFAULT_TIMEOUT,
            token_lifetime: DEFAULT_TOKEN_LIFETIME,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_query_bytes: DEFAULT_MAX_QUERY_BYTES,
            api_prefix: DEFAULT_API_PREFIX.to_owned(),
            method: QueryMethod::Post,
            labels: LabelFilter::all(),
            user_agent: concat!("tomori-prometheus/", env!("CARGO_PKG_VERSION")).to_owned(),
            drop_wait: DROP_WAIT,
            #[cfg(test)]
            resolver: None,
            #[cfg(test)]
            attempts: None,
            #[cfg(test)]
            tls: None,
        }
    }

    /// Evaluate `expression` at `at`, an instant query. Evaluating several queries at one `at` makes them agree.
    pub fn query(&self, expression: &str, at: Timestamp) -> Result<Response<Data>, Error> {
        self.call("query", expression, &[("time", at.to_string())])
    }

    /// Evaluate `expression` at every step of `range`, a range query, whose series have at most [`MAX_POINTS`] points.
    pub fn query_range(
        &self,
        expression: &str,
        range: Range,
    ) -> Result<Response<Vec<Series>>, Error> {
        let points = range.points().ok_or(Error::Invalid(
            "a range must not end before it starts, and its step must be at least a millisecond"
                .into(),
        ))?;
        if points > MAX_POINTS {
            return Err(Error::Limit(format!(
                "a range of {points} points is more than {MAX_POINTS}"
            )));
        }
        let response = self.call(
            "query_range",
            expression,
            &[
                ("start", range.start.to_string()),
                ("end", range.end.to_string()),
                ("step", time::seconds(range.step)),
            ],
        )?;
        match response.data {
            Data::Matrix(series) => Ok(Response {
                data: series,
                warnings: response.warnings,
            }),
            _ => Err(Error::Parse(
                "a range query's answer is not a matrix".into(),
            )),
        }
    }

    fn call(
        &self,
        call: &str,
        expression: &str,
        parameters: &[(&str, String)],
    ) -> Result<Response<Data>, Error> {
        let shared = &self.shared;
        if expression.len() > shared.max_query_bytes {
            return Err(Error::Limit(format!(
                "a query of {} bytes is more than {}",
                expression.len(),
                shared.max_query_bytes
            )));
        }
        let mut form = form_urlencoded::Serializer::new(String::new());
        form.append_pair("query", expression);
        for (name, value) in parameters {
            form.append_pair(name, value);
        }
        form.append_pair("timeout", &time::seconds(shared.timeout));
        let form = form.finish();
        let path = format!("{}/{call}", shared.path);
        let post = shared.method == QueryMethod::Post;
        let (target, body) = if post {
            (path, form.into_bytes())
        } else {
            (format!("{path}?{form}"), Vec::new())
        };
        let mut refused = None;
        loop {
            let token = self.token(refused)?;
            let reply = self.send(Outgoing {
                post,
                target: target.clone(),
                body: body.clone(),
                authorization: token
                    .as_ref()
                    .map(|(token, _)| format!("Bearer {}", token.secret())),
            })?;
            if reply.status == 401
                && let Some((_, generation)) = &token
                && refused.is_none()
            {
                refused = Some(*generation);
                continue;
            }
            if matches!(reply.status, 401 | 403) {
                let without = if token.is_none() {
                    ", and the client has no token source"
                } else {
                    ""
                };
                return Err(Error::Auth(format!(
                    "Prometheus refused the request with HTTP {}{without}",
                    reply.status
                )));
            }
            let answer = Answer {
                status: reply.status,
                retry_after: reply.retry_after,
                body: &reply.body,
            };
            let scrub = Scrub {
                token: token.as_ref().map(|(token, _)| token.secret()),
                ..self.scrub()
            };
            return data::read(answer, &shared.labels, &scrub);
        }
    }

    /// What quoted text is scrubbed of, the token aside.
    fn scrub(&self) -> Scrub<'_> {
        Scrub {
            token: None,
            host: &self.shared.host,
            base: &self.shared.base,
        }
    }

    /// The token to send, and its generation, `None` without a source. The token kept is sent unless it has outlived its lifetime, or it is the one of the generation `refused`, which the endpoint refused; one refused but since renewed by another caller is not renewed again.
    fn token(&self, refused: Option<u64>) -> Result<Option<(Token, u64)>, Error> {
        let shared = &self.shared;
        let Some(source) = &shared.tokens else {
            return Ok(None);
        };
        let mut kept = shared.token.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = match &kept.token {
            Some((_, since)) => {
                since.elapsed() < shared.token_lifetime && refused != Some(kept.generation)
            }
            None => false,
        };
        if !fresh {
            let token = source
                .token()
                .map_err(|message| Error::Auth(self.scrub().quote(&message)))?;
            kept.token = Some((token, Instant::now()));
            kept.generation += 1;
        }
        let (token, _) = kept.token.as_ref().expect("a token was just kept");
        Ok(Some((token.clone(), kept.generation)))
    }

    /// Send `request` on the client's thread and wait for its reply, within the time-out.
    fn send(&self, request: Outgoing) -> Result<Reply, Error> {
        let (reply, answer) = sync_channel(1);
        let cancellation = Cancellation::new();
        let timeout = self.shared.timeout;
        let job = Job {
            request,
            cancellation: cancellation.clone(),
            deadline: Instant::now() + timeout,
            reply,
        };
        if self.shared.jobs.send_blocking(job).is_err() {
            return Err(Error::Transport("the client's thread has stopped".into()));
        }
        let answer = answer.recv_timeout(timeout);
        // The thread stops working on a request nobody waits for any longer. One answered needs nothing stopped, and cancelling it could retire the kept connection before the HTTP client has returned it to its pool.
        if answer.is_err() {
            cancellation.cancel();
        }
        match answer {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => Err(Error::Timeout(timeout)),
            // The thread drops a reply unanswered only when handling its request panicked, which would be a mistake of this crate's.
            Err(RecvTimeoutError::Disconnected) => Err(Error::Transport(
                "the client's thread failed while handling the request".into(),
            )),
        }
    }
}
