// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A client's own thread, which runs its requests one at a time on a scheduler of its own, over one connection that it keeps between them and replaces after any failure.
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::pin::pin;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_channel::Receiver;
use base_http::{
    Field, Method, Path, Request,
    body::{BodyFrame, BufferedBody, ReadableBody},
};
use base_http_client::{Client as Http, Endpoint as Address, Options};
use base_scheduler::{
    AddressHints, AddressInfoCollection, AddressInfoView, AddressResolution, AddressService,
    Cancellation, Clock,
};
use base_scheduler_native::NativeScheduler;
use futures_lite::FutureExt;
use futures_lite::future::{block_on, or, zip};

use crate::Error;
use crate::endpoint::Endpoint;
use crate::error::Scrub;

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

/// A request, its cancellation, which its caller cancels once it no longer waits, and where its reply goes.
pub(crate) struct Job {
    pub request: Outgoing,
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
    /// Whether a connection was made, which a caller's time-out while connecting leaves unmade.
    connected: bool,
}

/// Run `jobs` until every sender has gone, then stop the scheduler.
pub(crate) fn run(scheduler: NativeScheduler, jobs: Receiver<Job>, settings: Settings) {
    block_on(serve(&scheduler, &jobs, &settings));
    block_on(scheduler.shutdown());
}

async fn serve(scheduler: &NativeScheduler, jobs: &Receiver<Job>, settings: &Settings) {
    // The address that last connected, tried first until an attempt fails.
    let mut preferred = None;
    while let Ok(mut job) = jobs.recv().await {
        // A caller that gave up while its request waited needs nothing done.
        if job.cancellation.is_cancelled() {
            continue;
        }
        let mut addresses = match resolve(scheduler, settings, &job.cancellation).await {
            Ok(addresses) => addresses,
            Err(error) => {
                let _ = job.reply.send(Err(error));
                continue;
            }
        };
        addresses.sort_by_key(|address| Some(*address) != preferred);
        // Each address in turn, until one connects; it is kept for as long as its connection is.
        let count = addresses.len();
        for (index, address) in addresses.into_iter().enumerate() {
            let attempt = connect(scheduler, jobs, settings, address, job, count - index);
            // A panic, which would be a mistake of this crate's, fails the request it happened in, whose reply is dropped as it unwinds, rather than the thread.
            let Ok(outcome) = AssertUnwindSafe(attempt).catch_unwind().await else {
                preferred = None;
                break;
            };
            preferred = outcome.connected.then_some(address);
            match outcome.again {
                Some(again) => job = again,
                None => break,
            }
        }
    }
}

/// How a connection to one address went.
struct Outcome {
    /// The request, unanswered, if the connection could not be made and there is another address to try.
    again: Option<Job>,
    /// Whether the connection was made.
    connected: bool,
}

/// Serve `job`, then the jobs after it, over a connection to `address`, until an exchange fails or every sender has gone. `left` counts the addresses left to try, this one included: unless it is the last, connecting may take only its share of what is left of the request's time.
async fn connect(
    scheduler: &NativeScheduler,
    jobs: &Receiver<Job>,
    settings: &Settings,
    address: SocketAddr,
    job: Job,
    left: usize,
) -> Outcome {
    #[cfg(test)]
    if let Some(attempts) = &settings.attempts {
        attempts.lock().unwrap().push(address);
    }
    let mut address = Address::new(address);
    address.authority = settings.endpoint.authority.clone().into();
    if let Some(tls) = &settings.tls {
        address = address.with_tls(tls.clone(), settings.endpoint.host.clone());
    }
    let (http, driver) =
        Http::new(scheduler, address, Options::default()).expect("the default options are valid");
    let lifetime = Cancellation::new();
    let limit = (left > 1).then(|| {
        let share = job.deadline.saturating_duration_since(Instant::now()) / left as u32;
        share.clamp(CONNECT_LEAST, CONNECT_MOST)
    });
    let (_, outcome) = zip(driver.run(&lifetime), async {
        let mut job = job;
        let reply = exchange(scheduler, &http, &job, settings, limit).await;
        let outcome = if let Err(failure) = &reply
            && failure.connecting
        {
            if limit.is_some() {
                Outcome {
                    again: Some(job),
                    connected: false,
                }
            } else {
                let _ = job.reply.send(reply.map_err(|failure| failure.error));
                Outcome {
                    again: None,
                    connected: false,
                }
            }
        } else {
            // A caller's time-out while still connecting to the last address made no connection, so it is not preferred.
            let connected = match &reply {
                Ok(_) => true,
                Err(failure) => failure.connected,
            };
            let mut reply = reply;
            loop {
                let failed = reply.is_err();
                let _ = job.reply.send(reply.map_err(|failure| failure.error));
                // A failure may leave the connection part way through an exchange, so the next request starts afresh, resolving the host again.
                if failed {
                    break;
                }
                match next(jobs).await {
                    Some(next) => job = next,
                    None => break,
                }
                reply = exchange(scheduler, &http, &job, settings, None).await;
            }
            Outcome {
                again: None,
                connected,
            }
        };
        lifetime.cancel();
        outcome
    })
    .await;
    outcome
}

/// The next job whose caller still waits, or `None` once every sender has gone.
async fn next(jobs: &Receiver<Job>) -> Option<Job> {
    while let Ok(job) = jobs.recv().await {
        if !job.cancellation.is_cancelled() {
            return Some(job);
        }
    }
    None
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
fn transport(error: base_http_client::Error, settings: &Settings, connected: bool) -> Failure {
    let scrub = settings.scrub();
    match error {
        base_http_client::Error::Connect { source, .. } => Failure {
            error: Error::Transport(format!(
                "could not connect: {}",
                scrub.quote(&source.to_string())
            )),
            connecting: true,
            connected: false,
        },
        error => Failure {
            error: Error::Transport(scrub.quote(&error.to_string())),
            connecting: false,
            connected,
        },
    }
}

/// Send `job`'s request and read its answer. With a `limit`, a connection that is not made within it is a failure of connecting, so that another address can be tried; a connection made within it but slow to answer is not.
async fn exchange(
    scheduler: &NativeScheduler,
    http: &Http<'_>,
    job: &Job,
    settings: &Settings,
    limit: Option<Duration>,
) -> Result<Reply, Failure> {
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
    let mut request = Request::new(
        method,
        Path::from(outgoing.target.clone()),
        BufferedBody::from(outgoing.body.clone()),
    );
    for (name, value) in [
        ("user-agent", Some(settings.user_agent.as_str())),
        ("accept", Some("application/json")),
        ("content-type", content_type),
        ("authorization", outgoing.authorization.as_deref()),
    ] {
        if let Some(value) = value {
            let field = Field::new(name, value)
                .expect("the user agent and token are checked to be visible ASCII");
            request
                .fields
                .append(field)
                .expect("a request has room for these few fields");
        }
    }
    let mut call = pin!(http.call(request, &job.cancellation));
    let response = match limit {
        None => call.await,
        Some(limit) => {
            let waited = or(async { Some((&mut call).await) }, async {
                let _ = scheduler.sleep(limit, &Cancellation::new()).await;
                None
            })
            .await;
            match waited {
                Some(response) => response,
                None if http.pool_stats().live > 0 => call.await,
                None => {
                    return Err(Failure {
                        error: Error::Transport(format!(
                            "could not connect within {} ms",
                            limit.as_millis()
                        )),
                        connecting: true,
                        connected: false,
                    });
                }
            }
        }
    }
    .map_err(|error| {
        let stats = http.pool_stats();
        transport(error, settings, stats.live + stats.retiring > 0)
    })?;
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
        let frame = body
            .read(&job.cancellation)
            .await
            .map_err(|error| Failure {
                error: Error::Transport(format!(
                    "could not read the answer: {}",
                    settings.scrub().quote(&error.to_string())
                )),
                connecting: false,
                connected: true,
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
                    connected: true,
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
