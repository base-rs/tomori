// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection driver's own steps, on a scheduler of their own: reaching the service, waiting, refusing, and applying what reads return.

use super::*;

/// A scheduler for a test of its own.
fn test_scheduler() -> NativeScheduler {
    NativeScheduler::with_options(NativeOptions {
        workers: 1,
        threads: 2,
        capacity: 256,
        ..Default::default()
    })
    .unwrap()
}

/// The timers of `scheduler`, except that its waits and sleeps after the
/// first `honoured` end at once, failing if `fail` says so.
struct Hurried<'a> {
    scheduler: &'a NativeScheduler,
    honoured: std::sync::atomic::AtomicUsize,
    fail: bool,
}

impl<'a> Hurried<'a> {
    fn new(scheduler: &'a NativeScheduler, honoured: usize, fail: bool) -> Self {
        Self {
            scheduler,
            honoured: std::sync::atomic::AtomicUsize::new(honoured),
            fail,
        }
    }

    /// How the next wait ends if it is not honoured, or `None` if it is.
    fn ended(&self) -> Option<base_scheduler::OperationResult<()>> {
        // The count only falls, and saturates at none.
        let honoured = self.honoured.load(Ordering::Acquire);
        self.honoured
            .store(honoured.saturating_sub(1), Ordering::Release);
        match (honoured > 0, self.fail) {
            (true, _) => None,
            (false, true) => Some(Err(base_scheduler::OperationError::Io(
                std::io::Error::other("the timer broke"),
            ))),
            (false, false) => Some(Ok(())),
        }
    }
}

impl base_scheduler::Clock for Hurried<'_> {
    fn now(&self) -> Instant {
        self.scheduler.now()
    }

    fn sleep_until(
        &self,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> impl Future<Output = base_scheduler::OperationResult<()>> + Send {
        let ended = self.ended();
        async move {
            match ended {
                Some(ended) => ended,
                None => self.scheduler.sleep_until(deadline, cancellation).await,
            }
        }
    }
}

impl Timers for Hurried<'_> {
    type Timer = <NativeScheduler as Timers>::Timer;

    fn timer(&self) -> Self::Timer {
        self.scheduler.timer()
    }

    fn timer_wait_until(
        &self,
        timer: &mut Self::Timer,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> impl Future<Output = base_scheduler::OperationResult<()>> + Send {
        let ended = self.ended();
        async move {
            match ended {
                Some(ended) => ended,
                None => {
                    self.scheduler
                        .timer_wait_until(timer, deadline, cancellation)
                        .await
                }
            }
        }
    }
}

/// A resolver that finds nothing, or fails.
struct Unresolved {
    fail: bool,
}

impl AddressResolution for Unresolved {
    type Addresses = Vec<base_scheduler::AddressInfo>;

    fn address_resolve(
        &self,
        _name: Option<&str>,
        _service: AddressService<'_>,
        _hints: AddressHints,
        _cancellation: &Cancellation,
    ) -> impl Future<Output = base_scheduler::OperationResult<Self::Addresses>> + Send {
        std::future::ready(match self.fail {
            true => Err(base_scheduler::OperationError::Io(std::io::Error::other(
                "no such host",
            ))),
            false => Ok(Vec::new()),
        })
    }
}

fn local(port: u16) -> Resolved {
    Resolved {
        host: "[::1]".into(),
        port,
        auth: None,
    }
}

#[test]
fn an_address_that_resolves_to_nothing_or_fails_is_unreachable() {
    let cancellation = Cancellation::new();
    let empty = block_on(lookup(
        &Unresolved { fail: false },
        &local(1),
        &cancellation,
    ));
    assert_eq!(
        empty.unwrap_err(),
        "the server's address did not resolve to any endpoint"
    );
    let failed = block_on(lookup(&Unresolved { fail: true }, &local(1), &cancellation));
    assert_eq!(failed.unwrap_err(), "resolving [::1]:1: no such host");
}

#[test]
fn a_lost_stream_says_why_the_http_driver_ended() {
    let Outcome::Lost(reason) = with_driver(
        Outcome::Lost("ended".into()),
        Err(base_http_client::Error::Closed),
    ) else {
        unreachable!("expected a lost connection");
    };
    assert!(
        reason.starts_with("ended (the HTTP driver ended: "),
        "{reason}"
    );
    let Outcome::Shutdown = with_driver(Outcome::Shutdown, Err(base_http_client::Error::Closed))
    else {
        unreachable!("expected a shutdown");
    };
}

#[test]
fn the_backoff_resets_only_after_a_connection_that_lasted() {
    let delay = Duration::from_secs(4);
    assert_eq!(after_loss(delay, Duration::from_secs(1)), delay);
    assert_eq!(after_loss(delay, RECONNECT_MAX), RECONNECT_INITIAL);
}

#[test]
fn a_step_that_takes_too_long_or_whose_timer_fails_makes_the_attempt_unreachable() {
    let scheduler = test_scheduler();
    let (_commands, receiver) = Commands::new();
    let mut sessions = Sessions::default();
    for (fail, expected) in [
        (false, "no answer within 5s"),
        (true, "the connection timer failed: the timer broke"),
    ] {
        let timers = Hurried::new(&scheduler, 0, fail);
        let outcome = block_on(holding(
            std::future::pending::<std::result::Result<(), String>>(),
            &timers,
            &receiver,
            &mut sessions,
            CONNECT_TIMEOUT,
        ));
        let Err(Outcome::Unreachable(reason)) = outcome else {
            unreachable!("expected an unreachable attempt");
        };
        assert_eq!(reason, expected);
    }
    // Without a timer, the backoff ends the connection, as shutting down
    // does while it waits.
    let timers = Hurried::new(&scheduler, 0, true);
    let now = base_scheduler::Clock::now(&timers);
    let later = now + Duration::from_secs(3600);
    assert!(!block_on(wait_until_attempt(
        &timers,
        &receiver,
        &mut sessions,
        later,
        now
    )));
    _commands.shutdown();
    assert!(!block_on(wait_until_attempt(
        &scheduler,
        &receiver,
        &mut sessions,
        later,
        now
    )));
    block_on(scheduler.shutdown());
}

#[test]
fn waiting_for_a_connection_holds_commands_until_they_end_or_shut_down() {
    let scheduler = test_scheduler();
    // Neither the step nor its timer ends.
    let timers = Hurried::new(&scheduler, usize::MAX, false);
    for shut_down in [false, true] {
        let (commands, receiver) = Commands::new();
        let (reply, _replies) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Sessions {
                directory: "/tmp".into(),
                reply,
            })
            .unwrap();
        match shut_down {
            true => commands.shutdown(),
            false => drop(commands),
        }
        let mut sessions = Sessions::default();
        let outcome = block_on(holding(
            std::future::pending::<std::result::Result<(), String>>(),
            &timers,
            &receiver,
            &mut sessions,
            CONNECT_TIMEOUT,
        ));
        let Err(Outcome::Shutdown) = outcome else {
            unreachable!("expected the attempt to shut down");
        };
        // A shutdown is taken before what waits.
        assert_eq!(sessions.held.len(), usize::from(!shut_down));
    }
    // Waiting for the next attempt ends as the commands do.
    let (commands, receiver) = Commands::new();
    drop(commands);
    let now = base_scheduler::Clock::now(&scheduler);
    assert!(!block_on(wait_until_attempt(
        &timers,
        &receiver,
        &mut Sessions::default(),
        now + Duration::from_secs(3600),
        now
    )));
    block_on(scheduler.shutdown());
}

#[test]
fn a_connection_ends_when_its_reconciliation_timer_fails_or_its_stream_is_refused() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let other = FakeService::start(shared.clone(), "other");
    // The wait for the stream to open is honoured; the next, for the first
    // reconciliation, fails.
    for (port, expected) in [
        (
            service.address.port(),
            "stopped: the state reconciliation timer failed: the timer broke",
        ),
        (
            other.address.port(),
            "the event stream was refused with status 401",
        ),
    ] {
        let outcome = crate::api::tests::with_client(
            port,
            async |client, resolved, cancellation, scheduler| {
                let timers = Hurried::new(scheduler, 1, true);
                let (_commands, receiver) = Commands::new();
                let mut sessions = Sessions::default();
                connected(
                    client,
                    resolved,
                    &receiver,
                    &mut sessions,
                    cancellation,
                    &timers,
                    true,
                )
                .await
            },
        );
        let (Outcome::Lost(reason) | Outcome::Unreachable(reason)) = outcome else {
            unreachable!("expected the connection to end");
        };
        assert_eq!(reason, expected);
    }
}

#[test]
fn a_service_that_cannot_be_found_refuses_what_waits_for_it() {
    // An empty host is refused by the resolver itself.
    let resolvers: [(Resolve, &str); 2] = [
        (
            Box::new(|| Err("no service".to_owned())),
            "OpenCode is not reachable: no service",
        ),
        (
            Box::new(|| {
                Ok(Resolved {
                    host: String::new(),
                    port: 1,
                    auth: None,
                })
            }),
            "OpenCode is not reachable at :1: resolving :1: ",
        ),
    ];
    for (resolve, expected) in resolvers {
        let (commands, receiver) = Commands::new();
        let connection = std::thread::spawn(move || run_thread(resolve, receiver));
        let refused = block_on(commands.reply_with(|reply| Command::Sessions {
            directory: "/tmp".into(),
            reply,
        }))
        .unwrap_err();
        assert!(refused.message.starts_with(expected), "{}", refused.message);
        drop(commands);
        join_within(connection, Duration::from_secs(5));
    }
}

#[test]
fn commands_after_a_shutdown_or_once_the_connection_ended_are_answered_at_once() {
    let (commands, receiver) = Commands::new();
    commands.shutdown();
    let Ok(Command::Shutdown) = block_on(receiver.recv()) else {
        unreachable!("expected Ok(Command::Shutdown)");
    };
    let (commands, receiver) = Commands::new();
    drop(receiver);
    let refused = block_on(commands.reply_with(|reply| Command::Sessions {
        directory: "/tmp".into(),
        reply,
    }))
    .unwrap_err();
    assert_eq!(refused.message, "the connection has closed");
}

#[test]
fn failures_are_classified_by_status() {
    for (status, kind) in [
        (Some(404), tomori_backend::ErrorKind::NotFound),
        (Some(400), tomori_backend::ErrorKind::Invalid),
        (Some(500), tomori_backend::ErrorKind::Unavailable),
        (None, tomori_backend::ErrorKind::Unavailable),
    ] {
        let error = map_failure(Failure {
            status,
            message: "failed".into(),
        });
        assert_eq!((error.kind, error.message.as_str()), (kind, "failed"));
    }
}

/// The message of an error a command was answered with.
type Answer = Box<dyn FnOnce() -> String>;

/// One of each command that is answered, with how to read its answer.
fn answered_commands() -> Vec<(Command, Answer)> {
    fn answer<T: 'static>(
        build: impl FnOnce(async_channel::Sender<tomori_backend::Result<T>>) -> Command,
    ) -> (Command, Answer) {
        let (reply, replies) = async_channel::bounded::<tomori_backend::Result<T>>(1);
        let answer = move || {
            let Ok(Err(error)) = replies.try_recv() else {
                unreachable!("expected the command to be refused");
            };
            error.message
        };
        (build(reply), Box::new(answer))
    }
    let identity = Arc::new(AtomicU64::new(0));
    let id = || "s".to_owned();
    vec![
        answer(|reply| Command::Sessions {
            directory: "/tmp".into(),
            reply,
        }),
        answer(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Title".into(),
            },
            reply,
        }),
        answer(|reply| Command::Attach {
            id: id(),
            directory: "/tmp".into(),
            reply,
        }),
        answer(|reply| Command::Snapshot {
            id: id(),
            identity: identity.clone(),
            reply,
        }),
        answer(|reply| Command::Prompt {
            id: id(),
            identity: identity.clone(),
            key: "k".into(),
            text: "Hello.".into(),
            reply,
        }),
        answer(|reply| Command::Interrupt {
            id: id(),
            identity: identity.clone(),
            reply,
        }),
        answer(|reply| Command::Decide {
            id: id(),
            identity: identity.clone(),
            request: "p".into(),
            decision: "once".into(),
            reply,
        }),
        answer(|reply| Command::Answer {
            id: id(),
            identity: identity.clone(),
            form: "f".into(),
            answer: Value::Null,
            reply,
        }),
        answer(|reply| Command::SetMode {
            id: id(),
            identity: identity.clone(),
            mode: "plan".into(),
            reply,
        }),
        answer(|reply| Command::Instruction {
            id: id(),
            identity: identity.clone(),
            key: "k".into(),
            value: None,
            reply,
        }),
        answer(|reply| Command::ForgetInstructions {
            id: id(),
            keys: vec!["k".into()],
            reply,
        }),
        answer(|reply| Command::DeleteSession { id: id(), reply }),
        answer(|reply| Command::DeleteWorker {
            session: id(),
            worker: "w".into(),
            force: false,
            reply,
        }),
        answer(|reply| Command::RegisterTools {
            directory: "/tmp".into(),
            server: tomori_backend::ToolServer::new("tomori", "http://127.0.0.1:9/mcp"),
            reply,
        }),
        answer(|reply| Command::UnregisterTools {
            directory: "/tmp".into(),
            name: "tomori".into(),
            reply,
        }),
        answer(|reply| Command::Locate { id: id(), reply }),
    ]
}

#[test]
fn every_command_is_refused_with_the_error_and_known_abandoned() {
    for (command, answer) in answered_commands() {
        refuse(
            command,
            tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, "refused"),
        );
        assert_eq!(answer(), "refused");
    }
    // Closing and shutting down have nobody to answer.
    refuse(
        Command::Close {
            id: "s".into(),
            identity: Arc::new(AtomicU64::new(0)),
        },
        tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, "refused"),
    );
    refuse(
        Command::Shutdown,
        tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, "refused"),
    );
    for (command, answer) in answered_commands() {
        assert!(!abandoned(&command));
        drop(answer);
        assert!(abandoned(&command));
    }
}

fn test_worker(id: &str) -> Worker {
    Worker {
        id: id.into(),
        parent: Some("parent".into()),
        title: id.into(),
        state: WorkerState::Unknown,
    }
}

#[test]
fn a_registration_waiting_for_events_ends_when_they_are_lost() {
    let (events, receiver) = Events::new();
    let mut waiting = Box::pin(receiver.recv());
    assert!(block_on(futures_lite::future::poll_once(waiting.as_mut())).is_none());
    events.mark_ordinary_incomplete();
    assert!(block_on(waiting).is_err());
    // Nor are workers published to it.
    assert!(!events.try_deliver_workers(Vec::new(), None));
}

#[test]
fn only_a_known_running_state_is_an_event() {
    assert!(RunningState::Unknown.event().is_none());
    let Some(Event::Busy) = RunningState::Running.event() else {
        unreachable!("expected Some(Event::Busy)");
    };
}

#[test]
fn workers_read_again_are_merged_only_while_observation_is_complete() {
    let (mut registration, _events) = Registration::new();
    let mut oversized = test_worker("x");
    oversized.id = "x".repeat(MAX_WORKER_ID_BYTES + 1);
    assert!(
        registration
            .merge_workers(vec![oversized], &HashMap::new())
            .is_err()
    );
    for index in 0..api::MAX_TRACKED_WORKERS {
        registration.insert_child(test_worker(&format!("w{index}")));
    }
    registration
        .merge_workers(vec![test_worker("more")], &HashMap::new())
        .unwrap();
    assert!(registration.workers_incomplete);
    assert!(!registration.children.contains_key("more"));
    // Once incomplete, a listing changes nothing, and it stays incomplete.
    registration
        .merge_workers(vec![test_worker("later")], &HashMap::new())
        .unwrap();
    registration.mark_workers_incomplete();
    assert!(!registration.children.contains_key("later"));
    assert!(registration.workers_incomplete);
}

#[test]
fn deletions_beyond_the_limit_during_a_read_are_noted_as_too_many() {
    let (mut registration, _events) = Registration::new();
    registration.snapshot_in_flight = Some(Cancellation::new());
    registration.snapshot_deletions = (0..api::MAX_TRACKED_WORKERS)
        .map(|index| format!("w{index}"))
        .collect();
    registration.remove_worker("w0");
    assert!(!registration.snapshot_deletions_overflow);
    registration.remove_worker("another");
    assert!(registration.snapshot_deletions_overflow);
}

#[test]
fn a_snapshot_is_refused_once_workers_are_incomplete_or_too_many() {
    let (mut registration, _events) = Registration::new();
    for index in 0..api::MAX_TRACKED_WORKERS {
        registration.insert_child(test_worker(&format!("w{index}")));
    }
    assert!(
        registration
            .install_workers(&mut vec![test_worker("more")])
            .is_err()
    );
    registration.workers_incomplete = true;
    assert!(registration.install_workers(&mut Vec::new()).is_err());
    assert!(registration.install(&mut Snapshot::default()).is_err());
}

#[test]
fn a_state_seen_while_workers_are_discovered_applies_without_a_title() {
    let (mut registration, _events) = Registration::new();
    registration
        .initial_changes
        .insert("w".into(), (Some(WorkerState::Running), None));
    let mut workers = vec![test_worker("w")];
    registration.install_workers(&mut workers).unwrap();
    assert_eq!(
        (workers[0].state, workers[0].title.as_str()),
        (WorkerState::Running, "w")
    );
}

/// A registration whose events were lost, being read again.
fn resyncing() -> (
    HashMap<String, Registration>,
    RegistrationEvents,
    Arc<AtomicU64>,
) {
    let mut registrations = HashMap::new();
    let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    let registration = registrations.get_mut("s").unwrap();
    registration.install_workers(&mut Vec::new()).unwrap();
    registration.resync = Some(Resync::default());
    registration.snapshot_in_flight = Some(Cancellation::new());
    let identity = registration.events.worker_epoch.clone();
    (registrations, events, identity)
}

fn read_again(
    identity: &Arc<AtomicU64>,
    result: tomori_backend::Result<Snapshot>,
    modes: Option<Modes>,
) -> ResyncResult {
    ResyncResult {
        id: "s".into(),
        registration: identity.clone(),
        result,
        rest: None,
        missing: Vec::new(),
        modes,
    }
}

#[test]
fn a_read_again_fails_when_too_many_workers_changed_or_one_is_oversized() {
    let (mut registrations, _events, identity) = resyncing();
    registrations
        .get_mut("s")
        .unwrap()
        .snapshot_deletions_overflow = true;
    finish_resync(
        &mut registrations,
        read_again(&identity, Ok(Snapshot::default()), None),
        Instant::now(),
    );
    assert_eq!(registrations["s"].resync.as_ref().unwrap().failures, 1);
    let registration = registrations.get_mut("s").unwrap();
    registration.snapshot_in_flight = Some(Cancellation::new());
    let mut oversized = test_worker("x");
    oversized.id = "x".repeat(MAX_WORKER_ID_BYTES + 1);
    let snapshot = Snapshot {
        workers: vec![oversized],
        ..Snapshot::default()
    };
    finish_resync(
        &mut registrations,
        read_again(&identity, Ok(snapshot), None),
        Instant::now(),
    );
    assert_eq!(registrations["s"].resync.as_ref().unwrap().failures, 2);
}

#[test]
fn a_read_again_takes_the_listed_agent_and_the_running_state_held_since() {
    let (mut registrations, events, identity) = resyncing();
    let registration = registrations.get_mut("s").unwrap();
    registration.resync.as_mut().unwrap().hold(1, Event::Idle);
    let snapshot = Snapshot {
        busy: Some(true),
        ..Snapshot::default()
    };
    let modes = Modes {
        offered: vec![tomori_backend::Mode {
            id: "plan".into(),
            name: "plan".into(),
            description: None,
        }],
        current: Some("plan".into()),
    };
    finish_resync(
        &mut registrations,
        read_again(&identity, Ok(snapshot), Some(modes)),
        Instant::now(),
    );
    let registration = &registrations["s"];
    assert_eq!(registration.modes.current.as_deref(), Some("plan"));
    let RunningState::Idle = registration.running else {
        unreachable!("expected RunningState::Idle");
    };
    let received: Vec<Event> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(QueuedEvent::into_current)
        .collect();
    let [
        Event::WorkersChanged { .. },
        Event::Resynchronized { snapshot },
        Event::Idle,
    ] = received.as_slice()
    else {
        unreachable!("expected the workers, the state read again and the idle event");
    };
    assert_eq!(snapshot.modes.current.as_deref(), Some("plan"));
}

#[test]
fn a_restore_for_a_closed_registration_is_dropped_and_one_that_overflows_closes_it() {
    let (mut registrations, events, identity) = resyncing();
    let restored = |registration: &Arc<AtomicU64>| RestoreResult {
        id: "s".into(),
        registration: registration.clone(),
        messages: vec![tomori_backend::SnapshotMessage {
            id: "m".into(),
            role: tomori_backend::Role::Assistant,
            parts: Vec::new(),
            completed: true,
        }],
        unrecorded: Vec::new(),
    };
    finish_restore(&mut registrations, restored(&Arc::new(AtomicU64::new(0))));
    assert!(registrations.contains_key("s"));
    let resync = registrations.get_mut("s").unwrap().resync.as_mut().unwrap();
    for index in 0..MAX_RESYNC_EVENTS {
        assert!(resync.hold(index as u64, Event::Busy));
    }
    finish_restore(&mut registrations, restored(&identity));
    assert!(!registrations.contains_key("s"));
    let Some(Event::Closed { reason: Some(_) }) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Closed {{ .. }})");
    };
}

#[test]
fn events_that_are_not_for_a_session_or_change_nothing_are_passed_over() {
    assert!(sse_payload(": keepalive").is_none());
    let mut registrations = HashMap::new();
    let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    let registration = registrations.get_mut("s").unwrap();
    registration.install_workers(&mut Vec::new()).unwrap();
    registration.modes.current = Some("plan".into());
    registration.running = RunningState::Running;
    dispatch_events("not json", &mut registrations);
    // An agent switch to the current agent and an event for no part of the
    // session change nothing; an idle session is said to be.
    for event in [
        json!({"type":"session.agent.switched","data":{"sessionID":"s","agent":"plan"}}),
        json!({"type":"session.unknown","data":{"sessionID":"s"}}),
        json!({"type":"session.idle","data":{"sessionID":"s"}}),
    ] {
        dispatch_events(&event.to_string(), &mut registrations);
    }
    let Some(Event::Idle) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Idle)");
    };
    assert!(events.try_recv().is_err());
}

#[test]
fn worker_events_without_what_they_are_about_or_changing_nothing_are_passed_over() {
    let mut registrations = registrations();
    let parent = registrations.get_mut("parent").unwrap();
    parent.install_workers(&mut Vec::new()).unwrap();
    parent
        .events
        .snapshot_reply(Snapshot::default())
        .accept()
        .unwrap();
    parent.insert_child(test_worker("child"));
    let before = registrations["parent"].children.clone();
    for event in [
        json!({"data":{"sessionID":"child"}}),
        json!({"type":"session.renamed"}),
        json!({"type":"session.renamed","data":{}}),
        json!({"type":"session.created","data":{"sessionID":"parent","parentID":"parent"}}),
        json!({"type":"session.created","data":{"sessionID":"child","parentID":"child"}}),
        json!({"type":"session.created","data":{"sessionID":"child","parentID":"parent"}}),
        json!({"type":"session.renamed","data":{"sessionID":"child","title":"child"}}),
        json!({"type":"session.execution.interrupted","data":{"sessionID":"child"}}),
    ] {
        apply_worker_event(&event, &mut registrations);
    }
    let mut stopped = before;
    stopped.get_mut("child").unwrap().state = WorkerState::Stopped;
    assert_eq!(registrations["parent"].children, stopped);
    apply_worker_event(
        &json!({"type":"session.execution.interrupted","data":{"sessionID":"child"}}),
        &mut registrations,
    );
    assert_eq!(registrations["parent"].children, stopped);
}

#[test]
fn worker_changes_during_a_read_again_are_kept_up_to_the_limit() {
    let (mut registrations, _events, _identity) = resyncing();
    for (kind, state) in [
        ("session.execution.failed", WorkerState::Error),
        ("session.execution.interrupted", WorkerState::Stopped),
    ] {
        apply_worker_event(
            &json!({"type":kind,"data":{"sessionID":"w"}}),
            &mut registrations,
        );
        assert_eq!(
            registrations["s"].resync.as_ref().unwrap().changes["w"],
            (Some(state), None)
        );
    }
    let resync = registrations.get_mut("s").unwrap().resync.as_mut().unwrap();
    resync.changes = (0..api::MAX_TRACKED_WORKERS)
        .map(|index| (format!("w{index}"), (None, None)))
        .collect();
    apply_worker_event(
        &json!({"type":"session.execution.failed","data":{"sessionID":"another"}}),
        &mut registrations,
    );
    assert!(registrations["s"].resync.as_ref().unwrap().changes_overflow);
}

#[test]
fn workers_not_delivered_are_delivered_again_and_an_unknown_registration_ends_quietly() {
    let (mut registration, events) = Registration::new();
    registration.workers_ready = true;
    registration.workers_dirty = true;
    let mut registrations = HashMap::from([("s".to_owned(), registration)]);
    retry_worker_delivery(&mut registrations);
    assert!(!registrations["s"].workers_dirty);
    let Some(Event::WorkersChanged { .. }) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::WorkersChanged {{ .. }})");
    };
    end_registration(&mut registrations, "unknown", None);
    assert!(registrations.contains_key("s"));
    assert!(session_summaries(&[json!({"id": 1})], None).is_empty());
}

#[test]
fn a_session_opencode_is_running_is_busy_and_a_message_without_a_type_is_none() {
    let parts = api::SnapshotParts {
        messages: json!({"data": [{"id": "m", "type": 1}, {"id": "n"}]}),
        permissions: Vec::new(),
        forms: Vec::new(),
        children: Vec::new(),
        busy: Some(false),
        has_parent: false,
        title: None,
        directory: None,
    };
    let snapshot = snapshot(
        parts,
        "s",
        Modes::default(),
        Some(&HashSet::from(["s".to_owned()])),
    );
    assert_eq!(snapshot.busy, Some(true));
    assert!(snapshot.messages.is_empty());
}

/// Perform `command`, as one that waited for a connection, against the
/// service on `port`, and apply what it completes.
fn perform(port: u16, registrations: &mut HashMap<String, Registration>, command: Command) {
    crate::api::tests::with_client(port, async |client, resolved, cancellation, scheduler| {
        let mut snapshots = FuturesUnordered::new();
        let sequence = Arc::new(AtomicU64::new(0));
        let request = prepare_command(
            command,
            true,
            registrations,
            &mut snapshots,
            &sequence,
            client,
            resolved,
            cancellation,
            scheduler,
        );
        assert!(snapshots.is_empty());
        if let Some(request) = request
            && let Some(completion) = request.await
        {
            complete(completion, registrations);
        }
    });
}

/// The error a command is answered with, once performed against the
/// service on `port`.
fn refused_by<T: 'static>(
    port: u16,
    registrations: &mut HashMap<String, Registration>,
    build: impl FnOnce(async_channel::Sender<tomori_backend::Result<T>>) -> Command,
) -> tomori_backend::Error {
    let (reply, replies) = async_channel::bounded(1);
    perform(port, registrations, build(reply));
    let Ok(Err(error)) = replies.try_recv() else {
        unreachable!("expected the command to be refused");
    };
    error
}

fn anonymous(method: &str, path: &str) -> (&'static str, String) {
    match (method, path) {
        ("POST", "/api/session") => ("200 OK", r#"{"data":{"title":"No id"}}"#.to_owned()),
        _ => (
            "200 OK",
            r#"{"data":{"id":"s","location":{"directory":"/elsewhere"}}}"#.to_owned(),
        ),
    }
}

#[test]
fn commands_opencode_fails_or_answers_without_what_they_need_are_refused() {
    let mut registrations = HashMap::new();
    let unreachable = crate::api::tests::unreachable_port();
    let anonymous = crate::api::tests::service(anonymous);
    for port in [unreachable, anonymous] {
        let error = refused_by(port, &mut registrations, |reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Title".into(),
            },
            reply,
        });
        assert_eq!(error.kind, tomori_backend::ErrorKind::Unavailable);
    }
    let error = refused_by(anonymous, &mut registrations, |reply| Command::Attach {
        id: "s".into(),
        directory: "/tmp".into(),
        reply,
    });
    assert_eq!(error.kind, tomori_backend::ErrorKind::NotFound);
    // Removing entries stops at the first that fails.
    let error = refused_by(unreachable, &mut registrations, |reply| {
        Command::ForgetInstructions {
            id: "s".into(),
            keys: vec!["first".into(), "second".into()],
            reply,
        }
    });
    assert_eq!(error.kind, tomori_backend::ErrorKind::Unavailable);
    // Deleting stops at a stop that fails, and at a deletion that fails.
    let error = refused_by(unreachable, &mut registrations, |reply| {
        Command::DeleteSession {
            id: "s".into(),
            reply,
        }
    });
    assert_eq!(error.kind, tomori_backend::ErrorKind::Unavailable);
    // An entry for a registration that has closed is not set.
    let error = refused_by(unreachable, &mut registrations, |reply| {
        Command::Instruction {
            id: "s".into(),
            identity: Arc::new(AtomicU64::new(0)),
            key: "k".into(),
            value: Some("v".into()),
            reply,
        }
    });
    assert_eq!(error.kind, tomori_backend::ErrorKind::Closed);
    assert!(registrations.is_empty());
}

fn agents(method: &str, path: &str) -> (&'static str, String) {
    match (method, path.split('?').next().unwrap_or_default()) {
        ("GET", "/api/agent") => ("200 OK", r#"{"data":[{"id":"plan"}]}"#.to_owned()),
        _ => ("200 OK", "{}".to_owned()),
    }
}

fn no_agents(_method: &str, _path: &str) -> (&'static str, String) {
    ("200 OK", r#"{"data":[]}"#.to_owned())
}

/// A service that creates a session using the plan agent, but fails to list its agents.
fn agents_unlisted(method: &str, path: &str) -> (&'static str, String) {
    match (method, path.split('?').next().unwrap_or_default()) {
        ("GET", "/api/agent") => ("500 Internal Server Error", "{}".to_owned()),
        _ => (
            "200 OK",
            r#"{"data":{"id":"s","agent":"plan","location":{"directory":"/tmp"}}}"#.to_owned(),
        ),
    }
}

#[test]
fn a_session_whose_agents_cannot_be_listed_opens_without_offered_modes() {
    let mut registrations = HashMap::new();
    let (reply, replies) = async_channel::bounded(1);
    perform(
        crate::api::tests::service(agents_unlisted),
        &mut registrations,
        Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Title".into(),
            },
            reply,
        },
    );
    let Ok(Ok((id, _events))) = replies.try_recv() else {
        unreachable!("expected the session to open");
    };
    assert_eq!(id, "s");
    // Reconciliation lists them again.
    assert!(registrations["s"].modes.offered.is_empty());
}

#[test]
fn a_session_offering_no_agents_lists_them_again_to_switch() {
    let mut registrations = HashMap::new();
    let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    let identity = events.identity.clone();
    let registration = registrations.get_mut("s").unwrap();
    registration.install_workers(&mut Vec::new()).unwrap();
    registration
        .events
        .snapshot_reply(Snapshot::default())
        .accept()
        .unwrap();
    registration.modes.current = Some("plan".into());
    // Still none: the agent is not offered.
    let error = refused_by(
        crate::api::tests::service(no_agents),
        &mut registrations,
        |reply| Command::SetMode {
            id: "s".into(),
            identity: identity.clone(),
            mode: "plan".into(),
            reply,
        },
    );
    assert_eq!(error.kind, tomori_backend::ErrorKind::Invalid);
    assert!(registrations["s"].modes.offered.is_empty());
    let (reply, replies) = async_channel::bounded(1);
    perform(
        crate::api::tests::service(agents),
        &mut registrations,
        Command::SetMode {
            id: "s".into(),
            identity,
            mode: "plan".into(),
            reply,
        },
    );
    block_on(replies.recv()).unwrap().unwrap();
    assert_eq!(registrations["s"].modes.offered.len(), 1);
    let Some(Event::ModesChanged(modes)) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::ModesChanged(..))");
    };
    assert_eq!(modes.current.as_deref(), Some("plan"));
}

#[test]
fn a_session_found_attached_once_its_request_finished_is_refused() {
    let mut registrations = HashMap::new();
    let _events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    let (created, creations) = async_channel::bounded(1);
    complete(
        Completion::Created {
            id: "s".into(),
            directory: "/tmp".into(),
            modes: Modes::default(),
            reply: created,
        },
        &mut registrations,
    );
    assert!(block_on(creations.recv()).unwrap().is_err());
    let (attached, attachments) = async_channel::bounded(1);
    complete(
        Completion::Attached {
            id: "s".into(),
            directory: "/tmp".into(),
            modes: Modes::default(),
            reply: attached,
        },
        &mut registrations,
    );
    assert!(block_on(attachments.recv()).unwrap().is_err());
    // Agents listed for a session that has closed are dropped.
    complete(
        Completion::Listed {
            id: "gone".into(),
            identity: Arc::new(AtomicU64::new(0)),
            modes: Modes::default(),
        },
        &mut registrations,
    );
    assert!(!registrations.contains_key("gone"));
}

#[test]
fn a_snapshot_waits_for_a_read_again_or_another_snapshot() {
    let (mut registrations, _events, identity) = resyncing();
    let message = |registrations: &mut HashMap<String, Registration>| {
        refused_by(
            crate::api::tests::unreachable_port(),
            registrations,
            |reply| Command::Snapshot {
                id: "s".into(),
                identity: identity.clone(),
                reply,
            },
        )
        .message
    };
    assert!(message(&mut registrations).contains("read again"));
    registrations.get_mut("s").unwrap().resync = None;
    assert!(message(&mut registrations).contains("already running"));
}

#[test]
fn reads_again_and_restores_start_only_as_many_as_are_allowed() {
    let mut registrations = HashMap::new();
    let mut receivers = Vec::new();
    for index in 0..=MAX_RESYNCS {
        let id = format!("s{index}");
        receivers.push(insert_registration(
            &mut registrations,
            id.clone(),
            "/tmp",
            Modes::default(),
        ));
        let registration = registrations.get_mut(&id).unwrap();
        registration.resync = Some(Resync::default());
        registration.restore_due.push("m".into());
    }
    let port = crate::api::tests::unreachable_port();
    crate::api::tests::with_client(port, async |client, resolved, cancellation, scheduler| {
        let sequence = Arc::new(AtomicU64::new(0));
        let mut resyncs = FuturesUnordered::new();
        start_resyncs(
            &mut registrations,
            &mut resyncs,
            &sequence,
            client,
            resolved,
            cancellation,
            scheduler,
        );
        assert_eq!(resyncs.len(), MAX_RESYNCS);
        let mut restores = FuturesUnordered::new();
        start_restores(
            &mut registrations,
            &mut restores,
            client,
            resolved,
            cancellation,
            scheduler,
        );
        assert_eq!(restores.len(), MAX_RESYNCS);
    });
}

#[test]
fn modes_read_again_are_listed_or_none_when_the_record_cannot_be_read_in_time() {
    let listed = crate::api::tests::service(agents);
    let modes = crate::api::tests::with_client(
        listed,
        async |client, resolved, cancellation, scheduler| {
            bounded_modes(client, resolved, "s", "/tmp", cancellation, scheduler).await
        },
    );
    let offered: Vec<_> = modes
        .unwrap()
        .offered
        .into_iter()
        .map(|mode| mode.id)
        .collect();
    assert_eq!(offered, ["plan"]);
    // A record that cannot be read.
    let refused = crate::api::tests::unreachable_port();
    let modes = crate::api::tests::with_client(
        refused,
        async |client, resolved, cancellation, scheduler| {
            bounded_modes(client, resolved, "s", "/tmp", cancellation, scheduler).await
        },
    );
    assert!(modes.is_none());
    // A service that never answers.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let modes =
        crate::api::tests::with_client(port, async |client, resolved, cancellation, scheduler| {
            let timers = Hurried::new(scheduler, 0, false);
            bounded_modes(client, resolved, "s", "/tmp", cancellation, &timers).await
        });
    assert!(modes.is_none());
}

fn messages(_method: &str, _path: &str) -> (&'static str, String) {
    (
        "200 OK",
        json!({"data": [
            {"id": "written", "type": "assistant", "content": [{"type": "text", "text": "Hello."}]},
            {"id": "empty", "type": "assistant", "content": [{"type": "text", "text": ""}]},
            {"id": "tools", "type": "assistant", "content": [{"type": "tool", "id": "t", "name": "bash", "state": {}}]},
        ], "cursor": {"next": null}})
        .to_string(),
    )
}

/// Restore `wanted` messages from the service on `port`, with `timers`
/// made from its scheduler.
fn restore(
    port: u16,
    wanted: &[&str],
    timers: impl for<'a> FnOnce(&'a NativeScheduler) -> Hurried<'a>,
) -> RestoreResult {
    let mut registrations = HashMap::new();
    let _events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    registrations.get_mut("s").unwrap().restore_due =
        wanted.iter().map(|id| (*id).to_owned()).collect();
    crate::api::tests::with_client(port, async |client, resolved, cancellation, scheduler| {
        let timers = timers(scheduler);
        let mut restores = FuturesUnordered::new();
        start_restores(
            &mut registrations,
            &mut restores,
            client,
            resolved,
            cancellation,
            &timers,
        );
        restores.next().await.unwrap()
    })
}

#[test]
fn a_restore_takes_messages_with_text_and_none_once_the_read_takes_too_long() {
    let listed = crate::api::tests::service(messages);
    let restored = restore(listed, &["written", "empty", "tools"], |scheduler| {
        Hurried::new(scheduler, usize::MAX, true)
    });
    let ids: Vec<_> = restored
        .messages
        .iter()
        .map(|message| message.id.as_str())
        .collect();
    assert_eq!(ids, ["written"]);
    assert_eq!(restored.unrecorded, ["empty", "tools"]);
    // A service that never answers.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let restored = restore(port, &["written"], |scheduler| {
        Hurried::new(scheduler, 0, false)
    });
    assert!(restored.messages.is_empty());
    assert_eq!(restored.unrecorded, ["written"]);
}

#[test]
fn reconciliation_lists_agents_for_a_session_offering_none_and_reports_its_running_state() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let address = Arc::new(Mutex::new(service.address));
    let (commands, connection) = connect_to(&address);
    let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
        spec: SessionSpec {
            directory: "/tmp".into(),
            title: "Reconciled".into(),
        },
        reply,
    }))
    .unwrap();
    block_on(commands.reply_with(|reply| Command::Snapshot {
        id: id.clone(),
        identity: events.identity.clone(),
        reply,
    }))
    .and_then(SnapshotReply::accept)
    .unwrap();
    let watched = watch(&events);
    // A keepalive is passed over, and an agent switch is the session's
    // agent though none is offered yet.
    service.send_raw(b": keepalive\n\n");
    service.send(json!({"type":"session.agent.switched","data":{"sessionID":id,"agent":"plan"}}));
    let switched = Event::ModesChanged(Modes {
        offered: Vec::new(),
        current: Some("plan".into()),
    });
    until(&watched, "the agent switch", |event| *event == switched);
    // The next reconciliation lists the agents, and finds it running.
    shared.update(|state| {
        state.agents = Some(json!([{"id": "plan"}]));
        state.active.insert(id.clone());
    });
    let listed = Event::ModesChanged(Modes {
        offered: vec![tomori_backend::Mode {
            id: "plan".into(),
            name: "plan".into(),
            description: None,
        }],
        current: Some("plan".into()),
    });
    until(&watched, "the listed agents", |event| *event == listed);
    until(&watched, "the running state", |event| *event == Event::Busy);
    // Once agents are offered, a reconciliation only checks the running state.
    shared.update(|state| {
        state.active.clear();
    });
    until(&watched, "the idle state", |event| *event == Event::Idle);
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

#[test]
fn the_simulated_service_does_not_find_what_it_does_not_simulate() {
    let service = FakeService::start(Arc::new(FakeShared::default()), "");
    let mut socket = std::net::TcpStream::connect(service.address).unwrap();
    write!(socket, "GET /unknown HTTP/1.1\r\nMalformed\r\n\r\n").unwrap();
    let mut response = String::new();
    BufReader::new(socket).read_line(&mut response).unwrap();
    assert_eq!(response, "HTTP/1.1 404 Not Found\r\n");
}

#[test]
fn a_backend_that_cannot_reach_opencode_does_not_create_a_session() {
    use tomori_backend::Backend;

    let port = crate::api::tests::unreachable_port();
    let backend = crate::OpenCode::new(crate::Config {
        url: Some(format!("http://127.0.0.1:{port}")),
    });
    let refused = block_on(backend.create(SessionSpec {
        directory: "/tmp".into(),
        title: "Unreachable".into(),
    }))
    .map(drop)
    .unwrap_err();
    assert_eq!(refused.kind, tomori_backend::ErrorKind::Unavailable);
    backend.shutdown();
}

#[test]
fn a_session_closed_while_it_waits_for_an_event_says_so() {
    use tomori_backend::Session;

    let (events, receiver) = Events::new();
    let (commands, _commands) = Commands::new();
    let session = crate::session::OpenCodeSession::new("s".into(), receiver, commands);
    let mut next = Box::pin(session.next_event());
    assert!(block_on(futures_lite::future::poll_once(next.as_mut())).is_none());
    block_on(session.close()).unwrap();
    drop(events);
    let Err(error) = block_on(next) else {
        unreachable!("expected the session to have closed");
    };
    assert_eq!(error.kind, tomori_backend::ErrorKind::Closed);
}
