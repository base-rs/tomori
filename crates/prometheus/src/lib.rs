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
//! A client built with a [`Cache`] shares one request among identical calls in flight together, keeps answers while they are fresh, rounding instant queries' times to a bucket ([`Call::bucket`]) so that consumers agree, and keeps range queries' points so that a moving window asks only for the steps it lacks; a partial answer ([`Response::partial`]) is never kept.
//!
//! A server side that lets the space's agent inspect a metric it presents describes it with a [`Metric`], which has no field for the endpoint or the token: the query, when it was evaluated, its labels, its latest value, its unit and a link.
//!
//! The crate logs nothing. The token is sent only in the `authorization` field of the client's own requests, and is in no error, `Debug` output or log; errors and warnings quote the source's own text only scrubbed of the token and the endpoint (see [`Error`]).
use std::fmt;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use base_scheduler::Cancellation;
use base_scheduler_native::{NativeOptions, NativeScheduler};

mod cache;
mod data;
mod endpoint;
mod error;
mod labels;
mod metric;
pub mod promql;
mod slots;
mod system;
mod time;
mod token;
mod worker;

#[cfg(test)]
mod tests;

pub use cache::{Cache, CacheStats, DEFAULT_CACHE_BYTES, DEFAULT_CACHE_ENTRIES, DEFAULT_OVERLAP};
pub use data::{Data, Labels, Point, Response, Sample, Series};
pub use error::{ENDPOINT, Error, QUOTED_BYTES, REDACTED};
pub use labels::LabelFilter;
pub use metric::{Metric, Window};
pub use time::{Range, Timestamp};
pub use token::{Token, TokenSource};

use cache::{Begin, Key, Store};
use data::Answer;
use error::Scrub;
use slots::Slots;
use worker::{Job, Outgoing, Reply, Settings};

/// The README's examples, checked as documentation tests.
#[doc = include_str!("../readme.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

/// How long a request may take unless the application or the call sets another time-out.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How many requests a client sends at once unless the application sets another concurrency.
pub const DEFAULT_CONCURRENCY: usize = 4;

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

/// How one call is sent, for [`Client::query_with`] and [`Client::query_range_with`]: its own time-out, whether it is a background request, the labels kept of its answer, and, with a client's [`Cache`], how long its answer is fresh and the bucket an instant query's time is rounded down to.
///
/// ```
/// use std::time::Duration;
/// use tomori_prometheus::{Call, LabelFilter};
///
/// let fleet = Call::new()
///     .timeout(Duration::from_secs(60))
///     .background()
///     .bucket(Duration::from_secs(60));
/// let chart = Call::new().labels(LabelFilter::only(["location"]));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Call {
    timeout: Option<Duration>,
    background: bool,
    fresh: Option<Duration>,
    bucket: Option<Duration>,
    labels: Option<LabelFilter>,
}

impl Call {
    /// A call with the client's time-out, not in the background.
    pub fn new() -> Self {
        Self::default()
    }

    /// This call's time-out in place of the client's, at most [`MAX_TIMEOUT`], which a longer one is taken as, as [`Builder::timeout`] describes.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Send this call as a background request, such as a slow read of a whole fleet: a background request never takes a slot while another request waits for one, and background requests together hold at most all but one of the client's slots, so that one is always left for other requests. With a concurrency of 1, a background request takes the only slot when no other request is waiting, and a request that comes after waits for it to finish: give a client that sends background requests a concurrency of at least 2 for them to run alongside others.
    pub fn background(mut self) -> Self {
        self.background = true;
        self
    }

    /// How long this call's answer is fresh, in place of the cache's [`Cache::fresh`] or the call's bucket: with a client's cache, a call for the same answer within this time is answered from it without a request. A client without a cache ignores it.
    pub fn fresh(mut self, fresh: Duration) -> Self {
        self.fresh = Some(fresh);
        self
    }

    /// Evaluate an instant query at the start of the `bucket`-long slot, counted from the epoch, that holds the time it is given ([`Timestamp::floor`]), so that every call within the slot asks the same, and with a client's cache, its answer fresh for the bucket unless [`Call::fresh`] says otherwise, is read once a slot by them all. A range query ignores it: give it a range whose start and end are steps from the epoch ([`Range::aligned`]) for a query cache in front of the source to help.
    pub fn bucket(mut self, bucket: Duration) -> Self {
        self.bucket = Some(bucket);
        self
    }

    /// Keep the labels `filter` keeps of this call's answer, in place of the client's [`Builder::labels`], so that one client can serve consumers that need different labels, such as a chart grouped by a label the client's own filter leaves out. The filter replaces the client's rather than adding to it: list every label the call needs.
    ///
    /// **It can widen what the client keeps.** A call's filter may keep labels the client's filter removes, and [`LabelFilter::all`] keeps every label, which loosens the protection the client's filter gives: labels can carry URLs, user identifiers or secrets, so give a call only the labels it needs and may pass on, never a filter built from what a client or the agent asked for unchecked. With a client's cache, an answer is kept under the filter it was read with, so calls with different filters never share an answer, nor a request in flight, and each filter's answers count towards the cache's bounds.
    pub fn labels(mut self, filter: LabelFilter) -> Self {
        self.labels = Some(filter);
        self
    }
}

/// The configuration of a [`Client`], from [`Client::builder`].
pub struct Builder {
    endpoint: String,
    tokens: Option<Arc<dyn TokenSource>>,
    timeout: Duration,
    concurrency: usize,
    token_lifetime: Duration,
    max_response_bytes: usize,
    max_query_bytes: usize,
    api_prefix: String,
    method: QueryMethod,
    labels: LabelFilter,
    user_agent: String,
    cache: Option<Cache>,
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

    /// How long each request may wait for a slot, and then, separately, take from when it has one until its answer is read; [`DEFAULT_TIMEOUT`] otherwise, and at most [`MAX_TIMEOUT`], which a longer one is taken as. A call may set its own with [`Call::timeout`]. A request that finds no slot within it fails as [`Error::Queued`], and one the source does not answer within it as [`Error::Timeout`]. It is sent as Prometheus's evaluation `timeout` too. A call whose first request is refused with HTTP 401 asks the token source again and sends a second request with time-outs of its own, so a call can take four times the time-out, and the token source's time, which is not bounded here, besides.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// How many requests the client and its clones send at once, each over a connection of its own; [`DEFAULT_CONCURRENCY`] otherwise. A request waits for a slot while this many are in flight. Background requests ([`Call::background`]) hold at most all but one of them, and let waiting requests go first.
    ///
    /// The address the client keeps has at most twice this many connections, counting those still closing; when the client moves to another address, the old one's connections close as their requests end, so for a moment there may be more. Each request in flight may hold its answer, up to [`Builder::max_response_bytes`], so the answers may take about this many times that much memory at once.
    pub fn concurrency(mut self, requests: usize) -> Self {
        self.concurrency = requests;
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

    /// The labels kept of every answer, unless a call gives its own ([`Call::labels`]); every label otherwise.
    pub fn labels(mut self, filter: LabelFilter) -> Self {
        self.labels = filter;
        self
    }

    /// The `user-agent` field; `tomori-prometheus/VERSION` otherwise.
    pub fn user_agent(mut self, user_agent: &str) -> Self {
        self.user_agent = user_agent.to_owned();
        self
    }

    /// Keep answers in `cache`, shared by the client's clones: identical calls in flight together send one request, answers are kept while fresh, and range queries ask only for the steps they lack (see [`Cache`]). Without one, every call sends its own request.
    ///
    /// A call waits for an identical call's request in flight only if that request has the same priority ([`Call::background`]) and a time-out at least as long as its own; otherwise it sends its own, so that a call is never held to a background request's turn for a slot, nor fails sooner than its own time-out. It waits at most twice its own time-out, as its own request could, failing as [`Error::Timeout`] after.
    pub fn cache(mut self, cache: Cache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// The client, with a thread of its own, or why it cannot be made: an endpoint, prefix or limit that cannot be used, or a system out of threads.
    pub fn build(self) -> Result<Client, Error> {
        let endpoint = endpoint::parse(&self.endpoint)?;
        let prefix = endpoint::prefix(&self.api_prefix)?;
        if self.timeout.is_zero()
            || self.concurrency == 0
            || self.max_response_bytes == 0
            || self.max_query_bytes == 0
        {
            return Err(Error::Invalid(
                "the time-out, concurrency and limits must be more than zero".into(),
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
            concurrency: self.concurrency,
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
                    capacity: self.concurrency.saturating_mul(2).max(16),
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
                renewed: Condvar::new(),
                token_lifetime: self.token_lifetime,
                timeout: self.timeout.min(MAX_TIMEOUT),
                slots: Slots::new(self.concurrency),
                max_query_bytes: self.max_query_bytes,
                path,
                method: self.method,
                labels: self.labels,
                host,
                base,
                cache: self.cache.map(Store::new),
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
/// It sends up to [`Builder::concurrency`] requests at once, from any of its callers' threads, on a thread and scheduler of its own, each over a connection of the HTTP client's pool, which keeps connections for the requests after and retires one after a failure on it, leaving the others. It tries each of the host's addresses in turn until one connects, and keeps to that address until connecting to it fails. A request waits for a free slot within its time-out, which fails as [`Error::Queued`], distinct from the source not answering, [`Error::Timeout`], and then has its time-out again for its exchange; a caller can give each call its own time-out, and mark slow reads as background requests, which leave a slot for others, with [`Call`]. Clones share the thread, the connections, the slots and the token. Dropping the last stops the thread and waits for it, for at most [`DROP_WAIT`], since a lookup stuck in the system's resolver cannot be interrupted; the thread then stops on its own once the lookup ends.
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
    /// Notified as a renewal ends.
    renewed: Condvar,
    token_lifetime: Duration,
    timeout: Duration,
    /// The slots each request takes before it is sent.
    slots: Arc<Slots>,
    max_query_bytes: usize,
    /// The endpoint's path and the API's prefix, before each call.
    path: String,
    method: QueryMethod,
    labels: LabelFilter,
    /// The endpoint's host and path, which quoted text is scrubbed of.
    host: String,
    base: String,
    cache: Option<Store>,
}

/// The token last given, when, and how many have been given, so that a caller whose request was refused renews only the token it sent.
#[derive(Default)]
struct Kept {
    token: Option<(Token, Instant)>,
    generation: u64,
    /// Whether a caller is asking the source for a token, which others needing one wait for.
    renewing: bool,
    /// Callers waiting for a renewal, in tests.
    #[cfg(test)]
    waiting: usize,
}

/// A caller's turn to ask the token source, ended as it is dropped, a panicking source's included.
struct Renewing<'a> {
    shared: &'a Shared,
}

impl Drop for Renewing<'_> {
    fn drop(&mut self) {
        self.shared.kept().renewing = false;
        self.shared.renewed.notify_all();
    }
}

impl Shared {
    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.token.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
            .field("concurrency", &shared.slots.limit())
            .field("method", &shared.method)
            .field("labels", &shared.labels)
            .field("cache", &shared.cache.as_ref().map(Store::config))
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
            concurrency: DEFAULT_CONCURRENCY,
            token_lifetime: DEFAULT_TOKEN_LIFETIME,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_query_bytes: DEFAULT_MAX_QUERY_BYTES,
            api_prefix: DEFAULT_API_PREFIX.to_owned(),
            method: QueryMethod::Post,
            labels: LabelFilter::all(),
            user_agent: concat!("tomori-prometheus/", env!("CARGO_PKG_VERSION")).to_owned(),
            cache: None,
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
        self.query_with(expression, at, Call::new())
    }

    /// Evaluate `expression` at `at`, as [`Client::query`] does, sent as `call` says: at the start of its bucket, if it has one, and with the client's cache, answered from it while fresh.
    pub fn query_with(
        &self,
        expression: &str,
        at: Timestamp,
        call: Call,
    ) -> Result<Response<Data>, Error> {
        let at = call.bucket.map_or(at, |bucket| at.floor(bucket));
        let fetch = || self.call("query", expression, &[("time", at.to_string())], &call);
        let Some(cache) = &self.shared.cache else {
            return fetch();
        };
        let fresh = call.fresh.or(call.bucket).unwrap_or(cache.config_fresh());
        let key = Key::Instant {
            expression: expression.to_owned(),
            at: at.millis(),
            labels: self.keyed_labels(&call),
        };
        match cache.begin_instant(key, fresh, self.sending(&call)) {
            Begin::Hit(response) => Ok(response),
            Begin::Follow(flight) => flight.wait(self.follow_timeout(&call)),
            Begin::Lead(landing, ()) => {
                let outcome = fetch();
                cache.finish_instant(landing, fresh, &outcome);
                outcome
            }
        }
    }

    /// What the client's cache has done and holds; nothing without one.
    pub fn cache_stats(&self) -> CacheStats {
        self.shared
            .cache
            .as_ref()
            .map(Store::stats)
            .unwrap_or_default()
    }

    /// How `call`'s request is sent, which decides whose identical request in flight it may wait for.
    fn sending(&self, call: &Call) -> cache::Sending {
        cache::Sending {
            background: call.background,
            timeout: self.follow_timeout(call),
        }
    }

    /// A call's time-out, which bounds its wait for an identical call's request in flight as it would its own request.
    fn follow_timeout(&self, call: &Call) -> Duration {
        call.timeout.unwrap_or(self.shared.timeout).min(MAX_TIMEOUT)
    }

    /// The labels kept of `call`'s answer: its own filter's, or the client's.
    fn labels<'a>(&'a self, call: &'a Call) -> &'a LabelFilter {
        call.labels.as_ref().unwrap_or(&self.shared.labels)
    }

    /// What `call`'s answers are kept under in the cache besides their expression: its own filter where it differs from the client's, `None` for the client's, so that a call without a filter of its own copies none.
    fn keyed_labels(&self, call: &Call) -> Option<LabelFilter> {
        call.labels
            .as_ref()
            .filter(|labels| **labels != self.shared.labels)
            .cloned()
    }

    /// Evaluate `expression` at every step of `range`, a range query, whose series have at most [`MAX_POINTS`] points.
    pub fn query_range(
        &self,
        expression: &str,
        range: Range,
    ) -> Result<Response<Vec<Series>>, Error> {
        self.query_range_with(expression, range, Call::new())
    }

    /// Evaluate `expression` at every step of `range`, as [`Client::query_range`] does, sent as `call` says. With the client's cache, the points kept of the same expression at the same step, whose steps coincide with `range`'s, are used, and only the steps they lack asked for, with the cache's overlap; the window is answered wholly from them while they are fresh and have every step of it.
    pub fn query_range_with(
        &self,
        expression: &str,
        range: Range,
        call: Call,
    ) -> Result<Response<Vec<Series>>, Error> {
        Self::check(&range)?;
        let Some(cache) = &self.shared.cache else {
            return self.fetch_range(expression, range, &call);
        };
        let fresh = call.fresh.unwrap_or(cache.config_fresh());
        match cache.begin_range(
            Key::range(expression, &range, self.keyed_labels(&call)),
            &range,
            fresh,
            self.sending(&call),
        ) {
            Begin::Hit(response) => Ok(response),
            Begin::Follow(flight) => {
                flight
                    .wait(self.follow_timeout(&call))
                    .map(|response| match response.data {
                        Data::Matrix(series) => Response {
                            data: series,
                            warnings: response.warnings,
                            partial: response.partial,
                        },
                        _ => unreachable!("a range query's leader shares its series as a matrix"),
                    })
            }
            Begin::Lead(landing, plan) => {
                let tail = Range::new(Timestamp::from_millis(plan.from), range.end, range.step);
                let fetched = self.fetch_range(expression, tail, &call);
                cache.finish_range(landing, &range, plan, fetched)
            }
        }
    }

    /// Keep `series`, the answer to a range query of `expression` over `range` that the application kept itself, such as across a restart, in the client's cache as though read, though never fresh: the next range of the same expression and step whose steps coincide with it, and whose start it holds, asks only for the steps after its last, with the cache's overlap. Its points outside `range` are left out, and any the cache has read itself are kept in place of those remembered for the same steps, being newer. Each series' points must be in order and on `range`'s steps, else it is refused as [`Error::Invalid`], keeping nothing. An expression evaluated at its range's start or end is never kept. Give only what this client answered, whole and not partial, so that it has the client's labels; a client without a cache keeps nothing.
    pub fn remember(&self, expression: &str, range: Range, series: &[Series]) -> Result<(), Error> {
        self.remember_with(expression, range, series, Call::new())
    }

    /// Keep `series` in the client's cache as [`Client::remember`] does, for the range queries made with `call`'s label filter ([`Call::labels`]): give only what a call with the same filter answered. The call's other settings are not used.
    pub fn remember_with(
        &self,
        expression: &str,
        range: Range,
        series: &[Series],
        call: Call,
    ) -> Result<(), Error> {
        Self::check(&range)?;
        if let Some(cache) = &self.shared.cache {
            let key = Key::range(expression, &range, self.keyed_labels(&call));
            cache.remember(key, &range, series)?;
        }
        Ok(())
    }

    /// Whether `range` can be asked for.
    fn check(range: &Range) -> Result<(), Error> {
        let points = range.points().ok_or(Error::Invalid(
            "a range must not end before it starts, and its step must be at least a millisecond"
                .into(),
        ))?;
        if points > MAX_POINTS {
            return Err(Error::Limit(format!(
                "a range of {points} points is more than {MAX_POINTS}"
            )));
        }
        Ok(())
    }

    /// Send a range query over `range` as `call` says.
    fn fetch_range(
        &self,
        expression: &str,
        range: Range,
        call: &Call,
    ) -> Result<Response<Vec<Series>>, Error> {
        let response = self.call(
            "query_range",
            expression,
            &[
                ("start", range.start.to_string()),
                ("end", range.end.to_string()),
                ("step", time::seconds(range.step)),
            ],
            call,
        )?;
        match response.data {
            Data::Matrix(series) => Ok(Response {
                data: series,
                warnings: response.warnings,
                partial: response.partial,
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
        options: &Call,
    ) -> Result<Response<Data>, Error> {
        let shared = &self.shared;
        let timeout = options.timeout.unwrap_or(shared.timeout).min(MAX_TIMEOUT);
        if timeout.is_zero() {
            return Err(Error::Invalid("a time-out must be more than zero".into()));
        }
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
        form.append_pair("timeout", &time::seconds(timeout));
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
            let request = Outgoing {
                post,
                target: target.clone(),
                body: body.clone(),
                authorization: token
                    .as_ref()
                    .map(|(token, _)| format!("Bearer {}", token.expose())),
            };
            let reply = self.send(request, timeout, options.background)?;
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
                token: token.as_ref().map(|(token, _)| token.expose()),
                ..self.scrub()
            };
            return data::read(answer, self.labels(options), &scrub);
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
        let mut kept = shared.kept();
        loop {
            let fresh = match &kept.token {
                Some((token, since)) => (since.elapsed() < shared.token_lifetime
                    && refused != Some(kept.generation))
                .then(|| (token.clone(), kept.generation)),
                None => None,
            };
            if let Some(fresh) = fresh {
                return Ok(Some(fresh));
            }
            if !kept.renewing {
                break;
            }
            // Another caller is asking the source: take what it is given, rather than ask again.
            #[cfg(test)]
            {
                kept.waiting += 1;
            }
            kept = shared
                .renewed
                .wait(kept)
                .unwrap_or_else(PoisonError::into_inner);
            #[cfg(test)]
            {
                kept.waiting -= 1;
            }
        }
        // The source is asked without the lock, so that callers whose token is still good never wait for it.
        kept.renewing = true;
        drop(kept);
        let renewing = Renewing { shared };
        let token = source
            .token()
            .map_err(|message| Error::Auth(self.scrub().quote(&message)))?;
        let mut kept = shared.kept();
        kept.token = Some((token.clone(), Instant::now()));
        kept.generation += 1;
        let generation = kept.generation;
        drop(kept);
        drop(renewing);
        Ok(Some((token, generation)))
    }

    /// Take a slot for `request`, within `timeout`, then send it on the client's thread and wait for its reply, within `timeout` again.
    fn send(&self, request: Outgoing, timeout: Duration, background: bool) -> Result<Reply, Error> {
        let slot = self
            .shared
            .slots
            .take(background, timeout)
            .ok_or(Error::Queued(timeout))?;
        let (reply, answer) = sync_channel(1);
        let cancellation = Cancellation::new();
        let job = Job {
            request,
            slot,
            cancellation: cancellation.clone(),
            deadline: Instant::now() + timeout,
            reply,
        };
        if self.shared.jobs.send_blocking(job).is_err() {
            return Err(Error::Transport("the client's thread has stopped".into()));
        }
        let answer = answer.recv_timeout(timeout);
        // The thread stops working on a request nobody waits for any longer, retiring its connection and giving back its slot. One answered needs nothing stopped, and cancelling it could retire its connection before the HTTP client has returned it to its pool.
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
