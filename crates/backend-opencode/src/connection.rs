// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection to the OpenCode server, on its own thread.
//!
//! [`base_http_client::Client`] and its [`Driver`] borrow from a scheduler and
//! the driver must be polled alongside every call, so the whole connection
//! lives on one dedicated thread that owns its scheduler. The [`Backend`] and
//! its sessions are `'static` handles that talk to this thread over channels:
//! every session owns an event channel the thread feeds, and commands (prompts,
//! replies, snapshots) travel one way and are answered on a per-command
//! channel. The thread reads the server's one global event stream and fans its
//! events out to the sessions they name.
//!
//! Registrations outlive any one connection. When the stream ends or fails,
//! as when the service restarts, the thread resolves the service again and
//! reconnects with a bounded backoff; commands meanwhile wait for the next
//! attempt. The stream does not replay what was missed, so each session whose
//! state had been read is read again once the stream reopens and presented
//! with [`Event::Resynchronized`]; one that cannot be is closed with the reason.
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use base_http::body::ReadableBody;
use base_http_client::{Client, Endpoint};
use base_scheduler::{
    AddressHints, AddressInfoCollection, AddressInfoView, AddressResolution, AddressService,
    Cancellation, SocketClose, SocketConnect, SocketOpen, SocketSetOption, SocketStreamReading,
    SocketStreamWriting, TcpNoDelay, Timers,
};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::StreamExt;
use futures_lite::future::block_on;
use futures_util::stream::FuturesUnordered;
use serde_json::Value;
use tomori_backend::{Event, Modes, SessionSpec, SessionSummary, Snapshot, Worker, WorkerState};

use crate::api::{self, Failure};
use crate::endpoint::Resolved;
use crate::wire;

mod commands;
mod dispatch;
mod reads;
mod registration;
mod requests;

use commands::*;
use dispatch::*;
use reads::*;
use registration::*;
use requests::*;

pub(crate) use commands::{Command, Commands};
pub(crate) use registration::RegistrationEvents;

/// How often the running state of each session is re-checked against the
/// server's record. The fast path is the event stream's idle events; this is
/// the backstop that clears a busy state when one of those is missed, so a
/// queue gated on the agent being idle cannot be left stuck. It is a no-op
/// while a turn is genuinely running, which is why the cadence can be loose:
/// it only matters once a turn has ended and the stream has not reported it.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);
/// A snapshot is isolated from the event dispatcher and bounded in time and
/// concurrency, including every response body and page.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SNAPSHOTS: usize = 16;
/// Sessions whose workers are discovered in the background at once, after a
/// read ran out of time for them; each lists at most eight parents' children
/// at once, and gives up on a listing OpenCode does not answer within
/// [`api::FamilyWalk::LISTING_TIMEOUT`].
const MAX_DISCOVERIES: usize = 4;
/// Why a session's workers may be incomplete while they are discovered.
const DISCOVERING: &str = "Still finding this session's workers, so some may be missing.";
/// How long a background discovery that failed, because OpenCode answered a
/// listing with an error or not in time, waits before each attempt to go on
/// from where it stopped. Once they are used up, the workers stay reported as
/// incomplete, with the reason, until the session is read again.
const DISCOVERY_RETRIES: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(120),
];
const MAX_RECONCILIATIONS: usize = 16;

/// How many reconciliations list a session's agents again while OpenCode offers none.
const LISTINGS: u8 = 30;

/// The delay before reconnecting after the event stream is lost or an attempt
/// fails. It doubles after each failed attempt, up to [`RECONNECT_MAX`], and
/// resets after a connection that stayed open at least that long.
const RECONNECT_INITIAL: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(10);
/// Resolving the service's address and opening its event stream are each
/// bounded, so an attempt against a service that accepts but never answers
/// still ends.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Commands waiting for a connection while the service is unreachable.
const MAX_HELD: usize = 64;
/// Sessions read again at once after reconnecting.
const MAX_RESYNCS: usize = 16;
/// Attempts to read a session again after reconnecting before its space is
/// closed.
const RESYNC_ATTEMPTS: u8 = 3;
/// Connections lost before a session could be read again, after the first
/// loss, before its space is closed.
const RESYNC_INTERRUPTIONS: u8 = 5;
/// Events held for a session while it is read again; more close it, as an
/// overflowing event channel does. Well below the channel's capacity, so the
/// held events and the state they follow can be delivered together.
const MAX_RESYNC_EVENTS: usize = 512;
/// Why a request fails when too many already wait for a connection.
const TOO_MANY_HELD: &str = "OpenCode is not reachable and too many requests are waiting for it";
/// Requests to OpenCode in flight at once on a connection, alongside its
/// event stream.
const MAX_REQUESTS: usize = 16;
/// Commands waiting to start on an open connection, behind a request in
/// flight for their session or the limit on requests; more are refused.
const MAX_WAITING: usize = 256;
/// Why a request fails when too many already wait to start.
const TOO_MANY_WAITING: &str = "too many requests are waiting for OpenCode";
/// How long the requests in flight when the event stream is lost may take to
/// be answered before the backend reconnects without them.
const IN_FLIGHT_GRACE: Duration = CONNECT_TIMEOUT;
/// Why a request fails once the backend is shutting down.
const SHUTTING_DOWN: &str = "the OpenCode backend is shutting down";
/// What a session is told when the event stream is lost.
const DISCONNECTED: &str = "Lost the connection to OpenCode; reconnecting.";

struct ReconcileResult {
    id: String,
    registration: Arc<AtomicU64>,
    revision: u64,
    modes: Option<Modes>,
    running: Option<bool>,
}

/// How the backend finds the OpenCode service. It is called before every
/// connection attempt, so a service that restarts on another port or with
/// another password is found again.
/// It fails when the service's address is not known yet, as when its
/// registration file is missing mid-run, and the attempt is retried later.
pub type Resolve = Box<dyn FnMut() -> std::result::Result<Resolved, String> + Send>;

/// Run the connection until its command channel closes, reconnecting whenever
/// the service cannot be reached or its event stream ends. Blocking: called on
/// a dedicated thread.
pub fn run_thread(resolve: Resolve, commands: CommandReceiver) {
    block_on(drive(resolve, commands));
}

async fn drive(resolve: Resolve, commands: CommandReceiver) {
    // With these options, starting fails only if the system refuses its
    // threads or event queue, as spawning any thread can.
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 1,
        threads: 2,
        capacity: 256,
        ..Default::default()
    })
    .expect("start the connection scheduler");
    run(&scheduler, resolve, commands).await;
    block_on(scheduler.shutdown());
}

/// What outlives one connection to the service.
#[derive(Default)]
struct Sessions {
    registrations: HashMap<String, Registration>,
    /// How many events the event streams have dispatched, across connections.
    sequence: Arc<AtomicU64>,
    /// Commands received while no connection is open, performed in order once
    /// one opens, or refused if the attempt fails.
    held: VecDeque<Command>,
}

/// Why a connection to the service ended.
enum Outcome {
    /// The command channel closed: the backend and every session were dropped.
    Shutdown,
    /// No connection could be opened.
    Unreachable(String),
    /// The event stream was open and then ended or failed.
    Lost(String),
}

/// Connect, serve until the connection is lost, and connect again, with a
/// bounded backoff, until the command channel closes.
async fn run<'scope, S: Scheduler<'scope> + Send + Sync + 'static>(
    scheduler: &S,
    mut resolve: Resolve,
    commands: CommandReceiver,
) where
    <S as Scheduler<'scope>>::Socket: 'static,
{
    let mut sessions = Sessions::default();
    let mut delay = RECONNECT_INITIAL;
    let mut attempt_at = scheduler.now();
    let mut earliest = attempt_at;
    let mut reported: Option<String> = None;
    let mut announce = false;
    loop {
        if !wait_until_attempt(scheduler, &commands, &mut sessions, attempt_at, earliest).await {
            break;
        }
        let started = scheduler.now();
        // A command waiting for a connection brings the next attempt forward,
        // but never to less than the initial delay after this one.
        earliest = started + RECONNECT_INITIAL;
        let outcome = match resolve() {
            Ok(resolved) => {
                let outcome =
                    connect(scheduler, &resolved, &commands, &mut sessions, announce).await;
                match outcome {
                    Outcome::Unreachable(reason) => Outcome::Unreachable(format!(
                        "OpenCode is not reachable at {}: {reason}",
                        resolved.authority()
                    )),
                    outcome => outcome,
                }
            }
            Err(reason) => Outcome::Unreachable(format!("OpenCode is not reachable: {reason}")),
        };
        match outcome {
            Outcome::Shutdown => break,
            Outcome::Unreachable(message) => {
                // Report each distinct failure once, not every retry.
                if reported.as_deref() != Some(message.as_str()) {
                    eprintln!("{message}; retrying");
                    reported = Some(message.clone());
                }
                for command in sessions.held.drain(..) {
                    refuse(
                        command,
                        tomori_backend::Error::new(
                            tomori_backend::ErrorKind::Unavailable,
                            message.clone(),
                        ),
                    );
                }
                attempt_at = started + delay;
                delay = (delay * 2).min(RECONNECT_MAX);
                announce = true;
            }
            Outcome::Lost(reason) => {
                // The sessions were told when the stream was lost.
                eprintln!("the opencode event stream {reason}; reconnecting");
                reported = None;
                let now = scheduler.now();
                let wait = after_loss(delay, now.saturating_duration_since(started));
                attempt_at = now + wait;
                delay = (wait * 2).min(RECONNECT_MAX);
                announce = true;
            }
        }
    }
    // Deliver Closed to every session still bound, so no session waits forever
    // for its event channel to fill, and fail every waiting request.
    for (_, registration) in sessions.registrations.drain() {
        registration
            .events
            .close_with(Some("the connection ended".to_owned()));
    }
    let shutting_down =
        || tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, SHUTTING_DOWN);
    for command in sessions.held.drain(..) {
        refuse(command, shutting_down());
    }
    // Sessions may still send commands, such as closing, until they are
    // dropped; answer them at once rather than leave them waiting.
    while let Ok(command) = commands.rx.recv().await {
        refuse(command, shutting_down());
    }
}

/// The delay before reconnecting after an event stream that was open for
/// `lasted` was lost, when the next delay was `delay`. A connection that is
/// lost soon after it opens counts as a failed attempt, so a service that keeps
/// dropping the stream is not reconnected to, and every session read again, in
/// a tight loop; one that lasted resets the backoff.
fn after_loss(delay: Duration, lasted: Duration) -> Duration {
    if lasted >= RECONNECT_MAX {
        RECONNECT_INITIAL
    } else {
        delay
    }
}

/// Answer `command` with `error` without performing it.
fn refuse(command: Command, error: tomori_backend::Error) {
    match command {
        Command::Sessions { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Create { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Attach { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Snapshot { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Locate { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. }
        | Command::Instruction { reply, .. }
        | Command::ForgetInstructions { reply, .. }
        | Command::DeleteSession { reply, .. }
        | Command::DeleteWorker { reply, .. }
        | Command::RegisterTools { reply, .. }
        | Command::UnregisterTools { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        // Closing needs no connection; see `hold`.
        Command::Close { .. } | Command::Shutdown => {}
    }
}

/// Whether nobody waits for `command`'s reply any more, for a command that
/// waited to be started.
fn abandoned(command: &Command) -> bool {
    match command {
        Command::Sessions { reply, .. } => reply.is_closed(),
        Command::Create { reply, .. } => reply.is_closed(),
        Command::Attach { reply, .. } => reply.is_closed(),
        Command::Snapshot { reply, .. } => reply.is_closed(),
        Command::Locate { reply, .. } => reply.is_closed(),
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. }
        | Command::Instruction { reply, .. }
        | Command::ForgetInstructions { reply, .. }
        | Command::DeleteSession { reply, .. }
        | Command::DeleteWorker { reply, .. }
        | Command::RegisterTools { reply, .. }
        | Command::UnregisterTools { reply, .. } => reply.is_closed(),
        Command::Close { .. } | Command::Shutdown => {
            unreachable!("a close or shutdown is never held or waiting")
        }
    }
}

/// Take a command while no connection is open: a close is performed at once,
/// and anything else waits for the next attempt.
/// Returns false when the backend is shutting down.
fn hold(sessions: &mut Sessions, command: Command) -> bool {
    match command {
        Command::Shutdown => return false,
        Command::Close { id, identity } => {
            close_registration(&mut sessions.registrations, &id, &identity);
            withdraw(
                &mut sessions.held,
                &identity,
                |command| command,
                |command| command,
            );
        }
        command if sessions.held.len() < MAX_HELD => sessions.held.push_back(command),
        command => refuse(
            command,
            tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_HELD),
        ),
    }
    true
}

/// Run `future`, a step of a connection attempt, for at most `timeout` while
/// no connection is open, taking commands meanwhile as [`hold`] does. The
/// attempt ends unreachable if the step fails or takes too long, and shuts
/// down once the command channel has closed.
async fn holding<T, S: Timers>(
    future: impl Future<Output = std::result::Result<T, String>>,
    timers: &S,
    commands: &CommandReceiver,
    sessions: &mut Sessions,
    timeout: Duration,
) -> std::result::Result<T, Outcome> {
    let cancellation = Cancellation::new();
    let deadline = timers.now() + timeout;
    let mut timer = timers.timer();
    let mut future = Fused::new(future);
    loop {
        futures_util::select! {
            result = &mut future => return result.map_err(Outcome::Unreachable),
            command = Fused::new(commands.recv()) => match command {
                Ok(command) => if !hold(sessions, command) {
                    return Err(Outcome::Shutdown);
                },
                Err(_) => return Err(Outcome::Shutdown),
            },
            due = Fused::new(timers.timer_wait_until(&mut timer, deadline, &cancellation)) => {
                return Err(Outcome::Unreachable(match due {
                    Ok(()) => format!("no answer within {timeout:?}"),
                    Err(error) => format!("the connection timer failed: {error}"),
                }));
            }
        }
    }
}

/// Wait until `deadline`, or `earliest` while a command waits for a
/// connection, taking commands meanwhile as [`hold`] does. Returns false once
/// the command channel has closed.
async fn wait_until_attempt<S: Timers>(
    timers: &S,
    commands: &CommandReceiver,
    sessions: &mut Sessions,
    deadline: Instant,
    earliest: Instant,
) -> bool {
    let cancellation = Cancellation::new();
    let mut timer = timers.timer();
    loop {
        let deadline = if sessions.held.is_empty() {
            deadline
        } else {
            deadline.min(earliest)
        };
        if timers.now() >= deadline {
            return true;
        }
        futures_util::select! {
            command = Fused::new(commands.recv()) => match command {
                Ok(command) => if !hold(sessions, command) {
                    return false;
                },
                Err(_) => return false,
            },
            due = Fused::new(timers.timer_wait_until(&mut timer, deadline, &cancellation)) => {
                if let Err(error) = due {
                    // Without a timer the backoff cannot be bounded; stop
                    // rather than retry in a tight loop.
                    eprintln!("the opencode reconnection timer failed: {error}");
                    return false;
                }
            }
        }
    }
}

/// Resolve the service's address, open its event stream and serve until it
/// ends. Every step before the stream is open is bounded by
/// [`CONNECT_TIMEOUT`] and keeps taking commands.
async fn connect<'scope, S: Scheduler<'scope> + Send + Sync + 'static>(
    scheduler: &S,
    resolved: &Resolved,
    commands: &CommandReceiver,
    sessions: &mut Sessions,
    announce: bool,
) -> Outcome
where
    <S as Scheduler<'scope>>::Socket: 'static,
{
    let cancellation = Cancellation::new();
    let lookup = lookup(scheduler, resolved, &cancellation);
    let address = match holding(lookup, scheduler, commands, sessions, CONNECT_TIMEOUT).await {
        Ok(address) => address,
        Err(outcome) => return outcome,
    };
    // Only options that are not valid are refused, and the defaults are.
    let (client, driver) = Client::new(
        scheduler,
        Endpoint::new(address),
        base_http_client::Options::default(),
    )
    .expect("the default client options are valid");

    // The driver must be polled for as long as the client is used, so it runs
    // alongside the connection loop in the same future; finishing either ends
    // both. The cancellation is shared so closing the connection unwinds both.
    let (driver_result, outcome) = futures_lite::future::zip(
        async {
            let result = driver.run(&cancellation).await;
            cancellation.cancel();
            result
        },
        async {
            let outcome = connected(
                &client,
                resolved,
                commands,
                sessions,
                &cancellation,
                scheduler,
                announce,
            )
            .await;
            cancellation.cancel();
            outcome
        },
    )
    .await;
    with_driver(outcome, driver_result)
}

/// The service's first address, resolved by `resolver`.
async fn lookup<R: AddressResolution>(
    resolver: &R,
    resolved: &Resolved,
    cancellation: &Cancellation,
) -> std::result::Result<base_scheduler::SocketAddress, String> {
    resolver
        .address_resolve(
            Some(resolved.host.trim_matches(['[', ']'])),
            AddressService::Port(resolved.port),
            AddressHints::default(),
            cancellation,
        )
        .await
        .map_err(|error| format!("resolving {}: {error}", resolved.authority()))?
        .entries()
        .next()
        .map(|entry| entry.address())
        .ok_or_else(|| "the server's address did not resolve to any endpoint".to_owned())
}

/// How a connection ended, with why its HTTP driver ended if it failed, when
/// that may be why the event stream was lost.
fn with_driver(
    outcome: Outcome,
    driver: std::result::Result<(), base_http_client::Error>,
) -> Outcome {
    match (outcome, driver) {
        (Outcome::Lost(reason), Err(error)) => {
            Outcome::Lost(format!("{reason} (the HTTP driver ended: {error})"))
        }
        (outcome, _) => outcome,
    }
}

/// The event stream was lost, so events are missing until the next connection.
/// Every session whose state has been read is read again then, holding its
/// events meanwhile, and is told about the gap now. A session not yet read is
/// closed: the read it is waiting for was abandoned with the connection, and
/// reading it later would present events from before the gap as current.
fn connection_lost(registrations: &mut HashMap<String, Registration>) {
    let mut unread = Vec::new();
    let mut unsettled = Vec::new();
    for (id, registration) in registrations.iter_mut() {
        // Their futures were dropped with the connection.
        registration.snapshot_in_flight = None;
        registration.reconcile_in_flight = None;
        registration.abandon_discovery();
        // A restore being read is read again on the next connection.
        registration.restore_in_flight = false;
        // What the workers do is unknown until their records are read again on the next connection: what happened
        // meanwhile, such as a run ending, is in no event that will arrive.
        registration.activity_in_flight = false;
        registration.activity_due = registration.activity.unsync();
        let reading = std::mem::take(&mut registration.restore_reading);
        registration.restore_due.extend(reading);
        registration.snapshot_deletions.clear();
        registration.snapshot_deletions_overflow = false;
        if registration.workers_ready {
            // Held events are kept: text deltas are in no record until their
            // message ends. Those the next read reflects are dropped then.
            //
            // Failures are not forgotten, and a connection lost again before
            // the session was read counts too, so a service that keeps
            // dropping its stream cannot leave a space reconnecting forever.
            let again = registration.resync.is_some();
            let resync = registration.resync.get_or_insert_with(Resync::default);
            resync.retry_at = None;
            if again {
                resync.interruptions = resync.interruptions.saturating_add(1);
                if resync.interruptions >= RESYNC_INTERRUPTIONS {
                    unsettled.push(id.clone());
                    continue;
                }
            }
            registration.events.deliver(Event::Reconnecting {
                reason: DISCONNECTED.to_owned(),
            });
        } else {
            unread.push(id.clone());
        }
    }
    for id in unsettled {
        end_registration(
            registrations,
            &id,
            Some(
                "the connection to OpenCode kept dropping before the session could be read again"
                    .to_owned(),
            ),
        );
    }
    for id in unread {
        end_registration(
            registrations,
            &id,
            Some("the connection to OpenCode was lost before the session was read".to_owned()),
        );
    }
}

/// The scheduler capabilities the connection needs.
trait Scheduler<'scope>:
    AddressResolution
    + SocketOpen
    + SocketConnect
    + SocketClose
    + SocketSetOption<TcpNoDelay>
    + SocketStreamReading
    + SocketStreamWriting
    + Timers
    + 'scope
{
    type Socket: 'static;
}

impl<'scope, S> Scheduler<'scope> for S
where
    S: AddressResolution
        + SocketOpen
        + SocketConnect
        + SocketClose
        + SocketSetOption<TcpNoDelay>
        + SocketStreamReading
        + SocketStreamWriting
        + Timers
        + 'scope,
    S::Socket: 'static,
{
    type Socket = S::Socket;
}

/// Wraps a future in a `FusedFuture`, pinning its inner future, since
/// `select!` requires every branch to implement it and the channel and body
/// futures do not.
struct Fused<F: Future> {
    inner: Pin<Box<F>>,
    done: bool,
}

impl<F: Future> Fused<F> {
    fn new(inner: F) -> Self {
        Self {
            inner: Box::pin(inner),
            done: false,
        }
    }
}

impl<F: Future> Future for Fused<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.done {
            unreachable!("a fused future was polled after finishing");
        }
        let output = self.inner.as_mut().poll(cx);
        if output.is_ready() {
            self.done = true;
        }
        output
    }
}

impl<F: Future> futures_core::FusedFuture for Fused<F> {
    fn is_terminated(&self) -> bool {
        self.done
    }
}

type SnapshotFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = SnapshotResult> + 'a>>>;
type ResyncFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = ResyncResult> + 'a>>>;
type RestoreFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = RestoreResult> + 'a>>>;
type ActivityFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = ActivityResult> + 'a>>>;
type DiscoveryFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = DiscoveryResult> + 'a>>>;

/// The service a connection's requests and reads go to: its HTTP client,
/// where it is and with what credentials, the connection's cancellation, which
/// ends them all with it, and its timers.
#[derive(Clone, Copy)]
struct Service<'a, 'c, T> {
    client: &'a Client<'c>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: T,
}

/// Open the server's global event stream, then serve it and the commands,
/// polling the stream, the command channel, the requests in flight, reads and
/// a periodic reconciliation of each session's running state together, until
/// the backend closes or the stream ends. Commands held while no connection
/// was open are started first, in order, and sessions whose events were lost
/// are read again.
async fn connected<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    commands: &CommandReceiver,
    sessions: &mut Sessions,
    cancellation: &Cancellation,
    timers: &S,
    announce: bool,
) -> Outcome {
    let open = async {
        let mut request = base_http::Request::get(base_http::Path::from("/api/event".to_owned()));
        if let Some(authorization) = resolved.authorization_header()
            && let Ok(field) = base_http::Field::new("authorization", authorization)
        {
            let _ = request.fields.append(field);
        }
        let response = client
            .call(request, cancellation)
            .await
            .map_err(|failure| format!("opening the event stream: {failure}"))?;
        let status = response.status.as_u16();
        if !(200..300).contains(&status) {
            return Err(format!("the event stream was refused with status {status}"));
        }
        Ok(response.body)
    };
    let mut stream = match holding(open, timers, commands, sessions, CONNECT_TIMEOUT).await {
        Ok(stream) => stream,
        Err(outcome) => return outcome,
    };
    if announce {
        eprintln!("connected to OpenCode at {}", resolved.authority());
    }
    let Sessions {
        registrations,
        held,
        sequence,
    } = sessions;
    let service = Service {
        client,
        resolved,
        cancellation,
        timers,
    };

    let mut groups = EventGroups::default();
    let mut timer = timers.timer();
    let mut next_reconcile = timers.now() + RECONCILE_INTERVAL;
    let mut reconcile_offset = 0usize;
    let mut snapshots: SnapshotFutures<'_> = FuturesUnordered::new();
    let mut resyncs: ResyncFutures<'_> = FuturesUnordered::new();
    let mut restores: RestoreFutures<'_> = FuturesUnordered::new();
    let mut activities: ActivityFutures<'_> = FuturesUnordered::new();
    let mut discoveries: DiscoveryFutures<'_> = FuturesUnordered::new();
    let mut reconciliations: FuturesUnordered<Pin<Box<dyn Future<Output = ReconcileResult> + '_>>> =
        FuturesUnordered::new();
    // The stream is open, so whatever happens from now on is observed: read
    // again every session whose events were lost.
    start_resyncs(registrations, &mut resyncs, sequence, service);

    // Commands not yet started, in the order they arrived, each with whether
    // it waited for this connection; the requests in flight; and the sessions
    // with one in flight, whose later commands wait for it.
    let mut waiting: VecDeque<(Command, bool)> =
        held.drain(..).map(|command| (command, true)).collect();
    let mut requests: RequestFutures<'_> = FuturesUnordered::new();
    let mut busy: HashSet<String> = HashSet::new();
    // Why the event stream was lost, while the requests in flight are
    // answered, and until when they may be.
    let mut lost: Option<String> = None;
    let mut grace: Option<Instant> = None;
    let mut grace_timer = timers.timer();

    let outcome = loop {
        retry_worker_delivery(registrations);
        // A request in flight is not cut short: the loss is noticed once each
        // has been answered.
        if let Some(reason) = &lost
            && requests.is_empty()
        {
            break Outcome::Lost(reason.clone());
        }
        // Commands that waited for this connection are started first, in
        // order, then the others as they arrive. Once shutting down, none is
        // started, held or not, and the shutdown is seen below as the command
        // channel is.
        if lost.is_none() & !commands.is_stopping() {
            start_commands(
                &mut waiting,
                &mut busy,
                &mut requests,
                registrations,
                &mut snapshots,
                sequence,
                service,
            );
        }
        let reading = lost.is_none();
        let until = grace;
        futures_util::select! {
            () = Fused::new(commands.stopped()) => break Outcome::Shutdown,
            () = Fused::new(async {
                match until {
                    Some(until) => {
                        let _ = timers.timer_wait_until(&mut grace_timer, until, cancellation).await;
                    }
                    None => std::future::pending().await,
                }
            }) => {
                // Those still unanswered are abandoned, so that one OpenCode
                // never answers cannot keep the backend from reconnecting.
                break Outcome::Lost(lost.clone().expect("a grace period follows a loss"));
            }
            command = Fused::new(commands.recv()) => match command {
                Ok(Command::Shutdown) | Err(_) => break Outcome::Shutdown,
                // Once the stream is lost, commands wait for the next
                // connection, as while none is open.
                Ok(Command::Close { id, identity }) if !reading => {
                    close_registration(registrations, &id, &identity);
                    withdraw(held, &identity, |command| command, |command| command);
                }
                Ok(command) if !reading => {
                    if held.len() < MAX_HELD {
                        held.push_back(command);
                    } else {
                        refuse(command, tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_HELD));
                    }
                }
                Ok(Command::Close { id, identity }) => {
                    close_session(registrations, &mut waiting, &id, &identity);
                }
                Ok(command @ Command::Snapshot { .. }) => {
                    prepare_command(command, false, registrations, &mut snapshots, sequence, service);
                }
                Ok(command) if waiting.len() >= MAX_WAITING => {
                    refuse(command, tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_WAITING));
                }
                Ok(command) => waiting.push_back((command, false)),
            },
            completed = Fused::new(async {
                if requests.is_empty() {
                    std::future::pending::<Option<(Option<String>, Option<Completion>)>>().await
                } else {
                    requests.next().await
                }
            }) => {
                let (session, completion) = completed.expect("a request was in flight");
                if let Some(session) = session {
                    busy.remove(&session);
                }
                if let Some(completion) = completion {
                    complete(completion, registrations);
                }
            }
            completed = Fused::new(async {
                if snapshots.is_empty() {
                    std::future::pending::<Option<SnapshotResult>>().await
                } else {
                    snapshots.next().await
                }
            }) => {
                finish_snapshot(registrations, completed.expect("a snapshot was in flight"));
                start_discoveries(registrations, &mut discoveries, service);
                continue;
            }
            completed = Fused::new(async {
                if discoveries.is_empty() {
                    std::future::pending::<Option<DiscoveryResult>>().await
                } else {
                    discoveries.next().await
                }
            }) => {
                let completed = completed.expect("a discovery was in flight");
                finish_discovery(registrations, completed, timers.now());
                start_discoveries(registrations, &mut discoveries, service);
                continue;
            }
            completed = Fused::new(async {
                if resyncs.is_empty() {
                    std::future::pending::<Option<ResyncResult>>().await
                } else {
                    resyncs.next().await
                }
            }) => {
                finish_resync(registrations, completed.expect("a read was in flight"), timers.now());
                // A slot is free: start the next waiting session at once.
                start_resyncs(registrations, &mut resyncs, sequence, service);
                start_discoveries(registrations, &mut discoveries, service);
                start_restores(registrations, &mut restores, service);
                start_activity_syncs(registrations, &mut activities, service);
                continue;
            }
            completed = Fused::new(async {
                if activities.is_empty() {
                    std::future::pending::<Option<ActivityResult>>().await
                } else {
                    activities.next().await
                }
            }) => {
                finish_activity_sync(registrations, completed.expect("a read was in flight"));
                start_activity_syncs(registrations, &mut activities, service);
                continue;
            }
            completed = Fused::new(async {
                if restores.is_empty() {
                    std::future::pending::<Option<RestoreResult>>().await
                } else {
                    restores.next().await
                }
            }) => {
                finish_restore(registrations, completed.expect("a restore was in flight"));
                start_restores(registrations, &mut restores, service);
                continue;
            }
            completed = Fused::new(async {
                if reconciliations.is_empty() {
                    std::future::pending::<Option<ReconcileResult>>().await
                } else {
                    reconciliations.next().await
                }
            }) => {
                let completed = completed.expect("a reconciliation was in flight");
                if let Some(registration) = registrations.get_mut(&completed.id)
                    && Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
                {
                    registration.reconcile_in_flight = None;
                    if registration.event_revision == completed.revision && registration.resync.is_none() {
                        if let Some(modes) = completed.modes
                            && !modes.offered.is_empty()
                            && registration.modes.offered.is_empty()
                        {
                            registration.modes = modes;
                            registration.publish_modes();
                        }
                        if let Some(running) = completed.running {
                            let running = if running { RunningState::Running } else { RunningState::Idle };
                            registration.reconcile_running(running);
                        }
                    }
                }
                continue;
            }
            result = Fused::new(async {
                if reading {
                    stream.read(cancellation).await
                } else {
                    std::future::pending().await
                }
            }) => {
                let loss = match result {
                    Err(error) => Some(format!("failed: {error}")),
                    Ok(frame) => {
                        let end = frame.is_end();
                        let mut oversized = false;
                        if let base_http::body::BodyFrame::Data { data, .. } = frame {
                            let complete = groups.push(&data);
                            for group in &complete {
                                if let Some(payload) = sse_payload(group) {
                                    let number = sequence.fetch_add(1, Ordering::AcqRel) + 1;
                                    dispatch_events_at(&payload, registrations, number);
                                }
                            }
                            start_restores(registrations, &mut restores, service);
                            oversized = groups.oversized();
                        }
                        if oversized {
                            // Dropping part of it could lose an event, so the
                            // stream is treated as lost, and every session is
                            // read again on the next one.
                            Some(format!("sent an event larger than {MAX_EVENT_BYTES} bytes"))
                        } else {
                            end.then(|| "ended".to_owned())
                        }
                    }
                };
                if let Some(reason) = loss {
                    lose(&mut lost, &mut waiting, held, registrations, reason);
                    // The reads in flight are abandoned with the stream; each
                    // session is read again on the next connection.
                    snapshots.clear();
                    resyncs.clear();
                    restores.clear();
                    discoveries.clear();
                    reconciliations.clear();
                    grace = Some(timers.now() + IN_FLIGHT_GRACE);
                }
                continue;
            }
            // Re-check each session's running state against the server's record,
            // so a missed idle event cannot leave a busy state — and the queue it
            // gates — stuck. A no-op while no session is registered.
            due = Fused::new(async {
                if reading {
                    timers.timer_wait_until(&mut timer, next_reconcile, cancellation).await
                } else {
                    std::future::pending().await
                }
            }) => {
                if let Err(error) = due {
                    break Outcome::Lost(format!("stopped: the state reconciliation timer failed: {error}"));
                }
                let mut ids: Vec<_> = registrations.keys().cloned().collect();
                for id in &ids {
                    close_abandoned_registration(registrations, id);
                }
                // A session read again after reconnecting reports its
                // running state from that read.
                // One not yet read reports it with its initial state.
                ids.retain(|id| registrations.get(id).is_some_and(|registration| registration.resync.is_none() && registration.workers_ready));
                ids.sort();
                if !ids.is_empty() {
                    let start = reconcile_offset % ids.len();
                    ids.rotate_left(start);
                    reconcile_offset = (start + MAX_RECONCILIATIONS) % ids.len();
                }
                for id in ids {
                    let registration = registrations.get_mut(&id).expect("registered at the start of this tick");
                    if registration.reconcile_in_flight.is_some() || reconciliations.len() >= MAX_RECONCILIATIONS {
                        continue;
                    }
                    let local_cancel = cancellation.child();
                    registration.reconcile_in_flight = Some(local_cancel.clone());
                    let identity = registration.events.worker_epoch.clone();
                    let revision = registration.event_revision;
                    let directory = registration.directory.clone();
                    let current = registration.modes.current.clone();
                    let list = registration.modes.offered.is_empty() && registration.listings > 0;
                    if list { registration.listings -= 1; }
                    reconciliations.push(Box::pin(async move {
                        let (modes, running) = futures_lite::future::race(
                            async {
                                let modes = if list {
                                    let info = current.map_or(Value::Null, |agent| serde_json::json!({"agent":agent}));
                                    Some(modes(client, resolved, &directory, &info, &local_cancel).await)
                                } else { None };
                                let running = api::session_running(client, resolved, &id, &local_cancel, timers).await.ok().flatten();
                                (modes, running)
                            },
                            async {
                                let _ = timers.sleep(SNAPSHOT_TIMEOUT, cancellation).await;
                                (None, None)
                            },
                        ).await;
                        local_cancel.cancel();
                        ReconcileResult { id, registration: identity, revision, modes, running }
                    }));
                }
                // Retry a restore whose record had no text yet.
                for registration in registrations.values_mut() {
                    let retry = std::mem::take(&mut registration.restore_retry);
                    registration.restore_due.extend(retry);
                }
                start_restores(registrations, &mut restores, service);
                start_activity_syncs(registrations, &mut activities, service);
                // Retry a session whose read again failed.
                start_resyncs(registrations, &mut resyncs, sequence, service);
                // Go on with a discovery whose retry is due.
                start_discoveries(registrations, &mut discoveries, service);
                retry_worker_delivery(registrations);
                next_reconcile = timers.now() + RECONCILE_INTERVAL;
                continue;
            }
        }
    };
    // Shutdown abandons the requests in flight rather than wait for OpenCode,
    // and the commands still waiting are refused or held with the others.
    drop(requests);
    for (command, _) in waiting.into_iter().rev() {
        held.push_front(command);
    }
    if matches!(outcome, Outcome::Lost(_)) && lost.is_none() {
        connection_lost(registrations);
    }
    cancellation.cancel();
    outcome
}

/// The event stream was lost: every session is told at once, the requests in
/// flight may still be answered, and the commands not started yet wait for the
/// next connection, ahead of those held later, however many there are.
fn lose(
    lost: &mut Option<String>,
    waiting: &mut VecDeque<(Command, bool)>,
    held: &mut VecDeque<Command>,
    registrations: &mut HashMap<String, Registration>,
    reason: String,
) {
    *lost = Some(reason);
    for (command, _) in waiting.drain(..).rev() {
        held.push_front(command);
    }
    connection_lost(registrations);
}

/// Fail at once the commands in `queue` made through the registration
/// `identity`, which has closed, rather than when they would have started.
fn withdraw<T>(
    queue: &mut VecDeque<T>,
    identity: &Arc<AtomicU64>,
    command: impl Fn(&T) -> &Command,
    into: impl Fn(T) -> Command,
) {
    let mut kept = VecDeque::with_capacity(queue.len());
    for entry in queue.drain(..) {
        if command_registration(command(&entry)).is_some_and(|other| Arc::ptr_eq(other, identity)) {
            refuse(
                into(entry),
                tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Closed,
                    "The session registration has closed.",
                ),
            );
        } else {
            kept.push_back(entry);
        }
    }
    *queue = kept;
}

/// The session a command is performed for, whose commands are performed one
/// at a time, in the order they arrived; `None` for one that may run beside
/// any other.
fn command_session(command: &Command) -> Option<&str> {
    match command {
        Command::Attach { id, .. }
        | Command::Prompt { id, .. }
        | Command::Interrupt { id, .. }
        | Command::Decide { id, .. }
        | Command::Answer { id, .. }
        | Command::SetMode { id, .. }
        | Command::Instruction { id, .. }
        | Command::ForgetInstructions { id, .. }
        | Command::DeleteSession { id, .. }
        | Command::DeleteWorker { worker: id, .. } => Some(id),
        Command::Sessions { .. }
        | Command::Create { .. }
        | Command::Snapshot { .. }
        | Command::Locate { .. }
        | Command::RegisterTools { .. }
        | Command::UnregisterTools { .. }
        | Command::Close { .. }
        | Command::Shutdown => None,
    }
}

/// The registration a command was made through, if it names one.
fn command_registration(command: &Command) -> Option<&Arc<AtomicU64>> {
    match command {
        Command::Prompt { identity, .. }
        | Command::Interrupt { identity, .. }
        | Command::Decide { identity, .. }
        | Command::Answer { identity, .. }
        | Command::SetMode { identity, .. }
        | Command::Instruction { identity, .. } => Some(identity),
        _ => None,
    }
}

/// Close `id`'s registration, failing at once the commands made through it
/// that have not started, rather than when its request in flight is answered.
fn close_session(
    registrations: &mut HashMap<String, Registration>,
    waiting: &mut VecDeque<(Command, bool)>,
    id: &str,
    identity: &Arc<AtomicU64>,
) {
    close_registration(registrations, id, identity);
    withdraw(
        waiting,
        identity,
        |(command, _)| command,
        |(command, _)| command,
    );
}

/// Start the waiting commands that can start, in order: up to
/// [`MAX_REQUESTS`] requests are in flight, and a session's next command waits
/// for the one in flight for it. Each request runs beside the event stream,
/// reads and every other request, so a stalled one holds up only its own
/// session's later commands.
fn start_commands<'a, S: Timers>(
    waiting: &mut VecDeque<(Command, bool)>,
    busy: &mut HashSet<String>,
    requests: &mut RequestFutures<'a>,
    registrations: &mut HashMap<String, Registration>,
    snapshots: &mut SnapshotFutures<'a>,
    sequence: &Arc<AtomicU64>,
    service: Service<'a, '_, &'a S>,
) {
    let mut kept = VecDeque::new();
    while let Some((command, held)) = waiting.pop_front() {
        let session = command_session(&command).map(str::to_owned);
        if requests.len() >= MAX_REQUESTS
            || session
                .as_ref()
                .is_some_and(|session| busy.contains(session))
        {
            kept.push_back((command, held));
            continue;
        }
        if let Some(request) =
            prepare_command(command, held, registrations, snapshots, sequence, service)
        {
            if let Some(session) = &session {
                busy.insert(session.clone());
            }
            requests.push(Box::pin(async move { (session, request.await) }));
        }
    }
    *waiting = kept;
}

#[cfg(test)]
mod tests;
