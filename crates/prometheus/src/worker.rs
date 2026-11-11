// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A client's own thread, which runs as many requests at once as their callers' slots allow, on a scheduler of its own, over the connections that the HTTP client's pool keeps to the address that last connected.
use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_channel::Receiver;
use base_http::{
    Field, Method, Path, RequestMetadata,
    body::{BodyFrame, ReadableBody},
};
use base_http_client::{Client as Http, Driver, Endpoint as Address, Options, WritableRequest};
use base_scheduler::{
    AddressHints, AddressInfoCollection, AddressInfoView, AddressResolution, AddressService,
    Cancellation, Clock,
};
use base_scheduler_native::NativeScheduler;
use futures_lite::FutureExt;
use futures_lite::future::{block_on, or};
use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::Error;
use crate::endpoint::Endpoint;
use crate::error::Scrub;
use crate::slots::Slot;

/// Resolves the endpoint's host in place of the system, in tests, so that none depends on a resolver or the network.
#[cfg(test)]
pub(crate) type Resolver = Arc<dyn Fn() -> Vec<SocketAddr> + Send + Sync>;

/// What the thread needs to reach the endpoint.
pub(crate) struct Settings {
    pub endpoint: Endpoint,
    /// Trusted roots, for an `https://` endpoint.
    pub tls: Option<Arc<rustls::ClientConfig>>,
    pub user_agent: String,
    pub max_response_bytes: usize,
    /// How many requests may be in flight at once.
    pub concurrency: usize,
    #[cfg(test)]
    pub resolver: Option<Resolver>,
    /// Records each address the thread tries to connect to, in tests.
    #[cfg(test)]
    pub attempts: Option<Arc<std::sync::Mutex<Vec<SocketAddr>>>>,
}

/// The least time connecting to one of several addresses may take.
const CONNECT_LEAST: Duration = Duration::from_millis(250);

/// The most time connecting to one of several addresses may take.
const CONNECT_MOST: Duration = Duration::from_secs(5);

impl Settings {
    /// What the transport's errors are scrubbed of.
    fn scrub(&self) -> Scrub<'_> {
        Scrub {
            token: None,
            host: &self.endpoint.host,
            base: &self.endpoint.base,
        }
    }
}

/// A request, the slot its caller took for it, its cancellation, which its caller cancels once it no longer waits, and where its reply goes.
pub(crate) struct Job {
    pub request: Outgoing,
    pub slot: Slot,
    pub cancellation: Cancellation,
    /// When its caller stops waiting.
    pub deadline: Instant,
    pub reply: SyncSender<Result<Reply, Error>>,
}

/// A request to send.
pub(crate) struct Outgoing {
    /// A form-encoded `POST`, or else a `GET`.
    pub post: bool,
    /// The path, with its query for a `GET`.
    pub target: String,
    pub body: Vec<u8>,
    pub authorization: Option<String>,
}

/// What the endpoint answered.
pub(crate) struct Reply {
    pub status: u16,
    /// The `retry-after` field, in seconds from now.
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

/// Why an exchange failed, and whether it failed before anything was sent, connecting, so that another address may be tried.
struct Failure {
    error: Error,
    connecting: bool,
}

/// One address's HTTP client, whose pool keeps connections to it for the requests after.
struct Route<'a> {
    address: SocketAddr,
    /// How many addresses the host resolved to when it was found.
    addresses: usize,
    http: Http<'a>,
}

/// What a client's thread shares between the requests it is handling.
struct State<'a> {
    scheduler: &'a NativeScheduler,
    settings: &'a Settings,
    /// The route that last connected, which requests use until a new connection to its address fails to connect.
    current: RefCell<Option<Rc<Route<'a>>>>,
    /// The address that last connected, tried first until an attempt fails.
    preferred: Cell<Option<SocketAddr>>,
    /// The turn to find a route, held by one request at a time.
    turn: (async_channel::Sender<()>, async_channel::Receiver<()>),
    /// The drivers of routes found since the thread last took them, to run until their routes have gone.
    drivers: RefCell<Vec<Driver<'a>>>,
}

/// Run `jobs`, each as it comes and as many at once as their callers' slots allow, until every sender has gone, then stop the scheduler.
pub(crate) fn run(scheduler: NativeScheduler, jobs: Receiver<Job>, settings: Settings) {
    block_on(serve(&scheduler, jobs, &settings));
    block_on(scheduler.shutdown());
}

async fn serve(scheduler: &NativeScheduler, jobs: Receiver<Job>, settings: &Settings) {
    let mut jobs = pin!(jobs);
    let state = State {
        scheduler,
        settings,
        current: RefCell::new(None),
        preferred: Cell::new(None),
        turn: async_channel::bounded(1),
        drivers: RefCell::default(),
    };
    let _ = state.turn.0.try_send(());
    let lifetime = Cancellation::new();
    let mut drivers = FuturesUnordered::new();
    let mut requests = FuturesUnordered::new();
    loop {
        let next = poll_fn(|context| {
            while let Poll::Ready(Some(())) = requests.poll_next_unpin(context) {}
            // Only requests find routes, so every driver is taken before the thread waits again or stops.
            for driver in state.drivers.take() {
                drivers.push(drive(driver, &lifetime));
            }
            while let Poll::Ready(Some(())) = drivers.poll_next_unpin(context) {}
            jobs.poll_next_unpin(context)
        })
        .await;
        let Some(job) = next else {
            break;
        };
        // A caller that gave up while its request waited needs nothing done.
        if !job.cancellation.is_cancelled() {
            requests.push(handle(&state, job));
        }
    }
    // No caller is left waiting: stop what is in flight, close every route and wait for their drivers to retire their connections.
    drop(requests);
    state.current.take();
    lifetime.cancel();
    while drivers.next().await.is_some() {}
}

/// Run a route's driver until its route has gone and its connections have retired, or the thread stops.
async fn drive<'a>(driver: Driver<'a>, lifetime: &'a Cancellation) {
    let _ = driver.run(lifetime).await;
}

/// Answer `job`, giving back its slot before replying.
async fn handle(state: &State<'_>, job: Job) {
    // A panic, which would be a mistake of this crate's, fails the request it happened in, whose reply is dropped as it unwinds, rather than the thread.
    let Ok(reply) = AssertUnwindSafe(send(state, &job)).catch_unwind().await else {
        return;
    };
    let Job {
        reply: sender,
        slot,
        ..
    } = job;
    drop(slot);
    let _ = sender.send(reply);
}

/// Send `job`'s request over the kept route, or else find one that connects, and read its answer.
async fn send(state: &State<'_>, job: &Job) -> Result<Reply, Error> {
    let kept = state.current.borrow().clone();
    let opened = match kept {
        Some(route) => {
            // Making a new connection to it may take only the share an attempt would, if the host has other addresses.
            let limit = (route.addresses > 1).then(|| share(job, route.addresses));
            match open(state.scheduler, &route.http, job, state.settings, limit).await {
                Ok(opened) => opened,
                // Its address failed to connect, so nothing was sent: forget it, and find another, trying this address last.
                Err(failure) if failure.connecting => {
                    forget(state, &route);
                    opened_in_turn(state, job, Some(route.address)).await?
                }
                // Any other failure, such as its caller giving up while a new connection was being made, ends only that connection, leaving the route and the others in its pool.
                Err(failure) => return Err(failure.error),
            }
        }
        None => opened_in_turn(state, job, None).await?,
    };
    // Any failure from here on retires only the connection it happened on, in the HTTP client's pool.
    read(opened, job, state.settings)
        .await
        .map_err(|failure| failure.error)
}

/// What is left of `job`'s time, shared among `left` addresses, within the bounds of an attempt to connect.
fn share(job: &Job, left: usize) -> Duration {
    let share = job.deadline.saturating_duration_since(Instant::now()) / left as u32;
    share.clamp(CONNECT_LEAST, CONNECT_MOST)
}

/// Forget `route` and its address, unless another request has already forgotten or replaced it.
fn forget<'a>(state: &State<'a>, route: &Rc<Route<'a>>) {
    let mut current = state.current.borrow_mut();
    if current.as_ref().is_some_and(|kept| Rc::ptr_eq(kept, route)) {
        *current = None;
        state.preferred.set(None);
    }
}

/// `job`'s request, opened over a route found in turn, trying `last` after the host's other addresses.
async fn opened_in_turn<'a>(
    state: &State<'a>,
    job: &Job,
    last: Option<SocketAddr>,
) -> Result<WritableRequest<'a>, Error> {
    match turn(state, job, last).await? {
        Found::Kept(route) => open(state.scheduler, &route.http, job, state.settings, None)
            .await
            .map_err(|failure| failure.error),
        Found::Opened(opened) => Ok(opened),
    }
}

/// What a request found in its turn.
enum Found<'a> {
    /// The route another request found meanwhile.
    Kept(Rc<Route<'a>>),
    /// The request, opened over a route it found.
    Opened(WritableRequest<'a>),
}

/// Wait for the turn to find a route, which one request at a time takes until it has connected, so that requests sent together share one route rather than each finding its own.
async fn turn<'a>(
    state: &State<'a>,
    job: &Job,
    last: Option<SocketAddr>,
) -> Result<Found<'a>, Error> {
    let (release, take) = &state.turn;
    let taken = or(async { take.recv().await.is_ok() }, async {
        job.cancellation.cancelled().await;
        false
    })
    .await;
    if !taken {
        return Err(Error::Transport("the request was abandoned".into()));
    }
    // Given back however finding a route ends, a panic included.
    let _turn = Turn(release);
    let kept = state.current.borrow().clone();
    match kept {
        Some(route) => Ok(Found::Kept(route)),
        None => establish(state, job, last).await.map(Found::Opened),
    }
}

/// The turn to find a route, given back as it is dropped.
struct Turn<'a>(&'a async_channel::Sender<()>);

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

/// Resolve the host and try each address in turn, the preferred first and `avoided` last, until one connects, keeping its route for the requests after, and open `job`'s request over it. Each but the last may take only its share of what is left of the request's time to connect.
async fn establish<'a>(
    state: &State<'a>,
    job: &Job,
    avoided: Option<SocketAddr>,
) -> Result<WritableRequest<'a>, Error> {
    let mut addresses = resolve(state.scheduler, state.settings, &job.cancellation).await?;
    let preferred = state.preferred.get();
    addresses.sort_by_key(|address| (Some(*address) == avoided, Some(*address) != preferred));
    let count = addresses.len();
    let (last, rest) = addresses.split_last().expect("resolving found an address");
    for (index, address) in rest.iter().enumerate() {
        let limit = share(job, count - index);
        match attempt(state, job, *address, count, Some(limit)).await {
            Err(failure) if failure.connecting => {}
            opened => return opened.map_err(|failure| failure.error),
        }
    }
    attempt(state, job, *last, count, None)
        .await
        .map_err(|failure| failure.error)
}

/// Open `job`'s request over a new route to `address`, which is kept, and its address preferred, once it has connected.
async fn attempt<'a>(
    state: &State<'a>,
    job: &Job,
    address: SocketAddr,
    addresses: usize,
    limit: Option<Duration>,
) -> Result<WritableRequest<'a>, Failure> {
    let route = Rc::new(route(state, address, addresses));
    let opened = open(state.scheduler, &route.http, job, state.settings, limit).await;
    if opened.is_ok() {
        state.preferred.set(Some(address));
        *state.current.borrow_mut() = Some(route);
    } else {
        // Whether it refused, did not connect within its share, or its caller gave up while it was still connecting.
        state.preferred.set(None);
    }
    opened
}

/// A route to `address`, whose pool may keep twice the client's concurrency of connections, since one retiring after a failure still counts against it until it has closed, and may connect as many at once as the client sends requests.
fn route<'a>(state: &State<'a>, address: SocketAddr, addresses: usize) -> Route<'a> {
    let settings = state.settings;
    #[cfg(test)]
    if let Some(attempts) = &settings.attempts {
        attempts.lock().unwrap().push(address);
    }
    let mut endpoint = Address::new(address);
    endpoint.authority = settings.endpoint.authority.clone().into();
    if let Some(tls) = &settings.tls {
        endpoint = endpoint.with_tls(tls.clone(), settings.endpoint.host.clone());
    }
    let mut options = Options::default();
    options.pool.max_resources = settings.concurrency.saturating_mul(2);
    options.pool.max_constructing = settings.concurrency;
    let (http, driver) =
        Http::new(state.scheduler, endpoint, options).expect("the options are valid");
    state.drivers.borrow_mut().push(driver);
    Route {
        address,
        addresses,
        http,
    }
}

async fn resolve(
    scheduler: &NativeScheduler,
    settings: &Settings,
    cancellation: &Cancellation,
) -> Result<Vec<SocketAddr>, Error> {
    #[cfg(test)]
    if let Some(resolver) = &settings.resolver {
        return found(resolver());
    }
    let endpoint = &settings.endpoint;
    let addresses = scheduler
        .address_resolve(
            Some(&endpoint.host),
            AddressService::Port(endpoint.port),
            AddressHints::default(),
            cancellation,
        )
        .await
        .map(|addresses| addresses.entries().map(|entry| entry.address()).collect())
        .unwrap_or_default();
    found(addresses)
}

fn found(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, Error> {
    if addresses.is_empty() {
        return Err(Error::Transport(
            "could not resolve the endpoint's host".into(),
        ));
    }
    Ok(addresses)
}

/// What a failure of the HTTP client says, without the address it connected to, scrubbed of the endpoint, such as a TLS failure naming its host.
fn transport(error: base_http_client::Error, settings: &Settings) -> Failure {
    let scrub = settings.scrub();
    match error {
        base_http_client::Error::Connect { source, .. } => Failure {
            error: Error::Transport(format!(
                "could not connect: {}",
                scrub.quote(&source.to_string())
            )),
            connecting: true,
        },
        error => Failure {
            error: Error::Transport(scrub.quote(&error.to_string())),
            connecting: false,
        },
    }
}

/// Take a connection of `http`'s pool, making one if none is free, and open `job`'s request on it. With a `limit`, a connection that is not made within it is a failure of connecting, so that another address can be tried.
async fn open<'a>(
    scheduler: &NativeScheduler,
    http: &Http<'a>,
    job: &Job,
    settings: &Settings,
    limit: Option<Duration>,
) -> Result<WritableRequest<'a>, Failure> {
    let outgoing = &job.request;
    #[cfg(test)]
    if outgoing.target.contains("/panic/") {
        panic!("a test's panic in the client's thread");
    }
    let (method, content_type) = if outgoing.post {
        (Method::POST, Some("application/x-www-form-urlencoded"))
    } else {
        (Method::GET, None)
    };
    let mut metadata = RequestMetadata::new(method);
    metadata.path = Some(Path::from(outgoing.target.clone()));
    metadata.content_length = Some(outgoing.body.len() as u64);
    for (name, value) in [
        ("user-agent", Some(settings.user_agent.as_str())),
        ("accept", Some("application/json")),
        ("content-type", content_type),
        ("authorization", outgoing.authorization.as_deref()),
    ] {
        if let Some(value) = value {
            let field = Field::new(name, value)
                .expect("the user agent and token are checked to be visible ASCII");
            metadata
                .fields
                .append(field)
                .expect("a request has room for these few fields");
        }
    }
    let opening = http.open_request(metadata, &job.cancellation);
    let opened = match limit {
        None => opening.await,
        Some(limit) => or(async { Some(opening.await) }, async {
            let _ = scheduler.sleep(limit, &Cancellation::new()).await;
            None
        })
        .await
        .ok_or_else(|| Failure {
            error: Error::Transport(format!("could not connect within {} ms", limit.as_millis())),
            connecting: true,
        })?,
    };
    opened.map_err(|error| transport(error, settings))
}

/// Send the body of a request `opened`, and read its answer.
async fn read(
    mut opened: WritableRequest<'_>,
    job: &Job,
    settings: &Settings,
) -> Result<Reply, Failure> {
    let cancellation = &job.cancellation;
    let written = if job.request.body.is_empty() {
        Ok(())
    } else {
        opened.write(job.request.body.clone(), cancellation).await
    };
    // A request whose body could not be written has been abandoned, so finishing it fails too, and the first failure is the one reported.
    let response = opened.finish(cancellation).await;
    let response = written
        .and(response)
        .map_err(|error| transport(error, settings))?;
    let retry_after = response
        .fields
        .values("retry-after")
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| retry_after(value, unix_now()));
    let status = response.status.as_u16();
    let mut body = response.body;
    let mut bytes = Vec::new();
    loop {
        let frame = body.read(cancellation).await.map_err(|error| Failure {
            error: Error::Transport(format!(
                "could not read the answer: {}",
                settings.scrub().quote(&error.to_string())
            )),
            connecting: false,
        })?;
        let end = frame.is_end();
        if let BodyFrame::Data { data, .. } = frame {
            if bytes.len() + data.len() > settings.max_response_bytes {
                return Err(Failure {
                    error: Error::Limit(format!(
                        "the answer is larger than {} bytes",
                        settings.max_response_bytes
                    )),
                    connecting: false,
                });
            }
            bytes.extend_from_slice(&data);
        }
        if end {
            break;
        }
    }
    Ok(Reply {
        status,
        retry_after,
        body: bytes,
    })
}

/// Seconds since the epoch now.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The longest wait a `retry-after` field is taken to ask for: a day.
pub(crate) const MOST_RETRY_AFTER: u64 = 86_400;

/// A `retry-after` field's wait, in seconds from `now`, at most [`MOST_RETRY_AFTER`]: delay seconds, or an HTTP date (`Sun, 06 Nov 1994 08:49:37 GMT`) from 1970 to 9999, a date already past being no wait. `None` for anything else, such as a date out of range or HTTP's obsolete date forms. Every field is checked before any arithmetic, so no value the source sends can overflow it.
pub(crate) fn retry_after(text: &str, now: u64) -> Option<u64> {
    let text = text.trim();
    if let Ok(seconds) = text.parse::<u64>() {
        return Some(seconds.min(MOST_RETRY_AFTER));
    }
    let [_, day, month, year, time, "GMT"] = text.split(' ').collect::<Vec<_>>()[..] else {
        return None;
    };
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|name| *name == month)? as i64
        + 1;
    let year = year
        .parse::<i64>()
        .ok()
        .filter(|year| (1970..=9999).contains(year))?;
    let day = day
        .parse::<i64>()
        .ok()
        .filter(|day| (1..=month_days(year, month)).contains(day))?;
    let [hour, minute, second] = time
        .split(':')
        .map(|part| part.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?[..]
    else {
        return None;
    };
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return None;
    }
    // Within these ranges, the date is at most about 2.5e11 seconds from the epoch, far within `i64`.
    let at = days(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some((at as u64).saturating_sub(now).min(MOST_RETRY_AFTER))
}

/// The days in `month` of `year`, of the Gregorian calendar.
fn month_days(year: i64, month: i64) -> i64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from the epoch to a date of the proleptic Gregorian calendar (Howard Hinnant's `days_from_civil`).
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_reads_seconds_and_http_dates() {
        let now = 784_111_777; // Sun, 06 Nov 1994 08:49:37 GMT
        assert_eq!(retry_after(" 120 ", now), Some(120));
        assert_eq!(
            retry_after("18446744073709551615", now),
            Some(MOST_RETRY_AFTER)
        );
        assert_eq!(retry_after("Sun, 06 Nov 1994 08:51:37 GMT", now), Some(120));
        assert_eq!(retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now), Some(0));
        assert_eq!(retry_after("Sat, 05 Nov 1994 08:49:37 GMT", now), Some(0));
        assert_eq!(
            retry_after("Sun, 06 Nov 1994 23:59:59 GMT", now),
            Some(54_622)
        );
        assert_eq!(
            retry_after("Fri, 31 Dec 9999 23:59:59 GMT", now),
            Some(MOST_RETRY_AFTER)
        );
        assert_eq!(
            retry_after("Thu, 29 Feb 2024 00:00:00 GMT", 1_709_164_700),
            Some(100)
        );
        assert_eq!(
            retry_after("Tue, 29 Feb 2000 00:00:00 GMT", 951_782_400),
            Some(0)
        );
        assert_eq!(retry_after("Thu, 01 Jan 1970 00:00:00 GMT", 0), Some(0));
        assert_eq!(
            retry_after("Sun, 30 Apr 1995 00:00:00 GMT", 799_200_000),
            Some(0)
        );
        for text in [
            "soon",
            "-5",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49 GMT",
            "Sun, 06 Nov 1994 08:xx:37 GMT",
            "Sun, xx Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov xxxx 08:49:37 GMT",
            // Out of range, or large enough to overflow arithmetic that did not check them first.
            "Sun, 06 Nov 1994 9223372036854775807:00:00 GMT",
            "Sun, 06 Nov 1994 -1:00:00 GMT",
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "Sun, 06 Nov 1994 08:60:00 GMT",
            "Sun, 06 Nov 1994 08:00:60 GMT",
            "Sun, 9223372036854775807 Nov 1994 08:49:37 GMT",
            "Sun, 00 Nov 1994 08:49:37 GMT",
            "Sun, 31 Nov 1994 08:49:37 GMT",
            "Thu, 29 Feb 2023 00:00:00 GMT",
            "Mon, 29 Feb 2100 00:00:00 GMT",
            "Sun, 06 Nov 1969 08:49:37 GMT",
            "Sun, 06 Nov 10000 08:49:37 GMT",
            "Sun, 06 Nov -9223372036854775808 08:49:37 GMT",
        ] {
            assert_eq!(retry_after(text, now), None);
        }
        assert_eq!(days(1969, 12, 31), -1);
    }
}
