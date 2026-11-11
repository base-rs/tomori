// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The server sides of environments, run for the spaces bound to them. See `design/environments.md`.
//!
//! [`ServerSides::reconcile`] is told which spaces are open and the environment each is bound to, and starts and stops sides to match, so that a space created in an environment, changed to it or bound again in it after a restart is run for the same way, whichever order the environments were offered and the kept spaces listed in. Each side runs on a thread of its own, as `crates/server/src/github.rs` reads GitHub, so that nothing it does can block the hub, another space or a conversation.
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base_scheduler::Cancellation;
use event_listener::{Event, EventListener};
use tomori_environment::{Inbox, Inspector, Intent, Outlet, ServerSide, Space};

use crate::environments::Environments;
use crate::limits;

/// An open space a side may run for: its identity, and the environment it is bound to.
pub(crate) struct Wanted {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub environment: String,
}

/// A side that failed, which the space shows.
pub(crate) struct Failure {
    /// The space the side ran for.
    pub space: String,
    /// Which binding of the space to its environment the side ran for, so that a failure after the space moved on is ignored.
    pub binding: u64,
    /// What to show in the space, or `None` for a side that finished, returning `Ok` before its space was done with it.
    pub message: Option<String>,
}

/// What is told of a side that fails after it started.
type Report = Box<dyn Fn(Failure) + Send + Sync>;

/// Makes the outlet a side's state, events and descriptions go to, for a space, a binding and the environment.
type Outlets =
    Box<dyn Fn(&str, u64, &crate::environments::Environment) -> Arc<dyn Outlet> + Send + Sync>;

/// The sides running or finished for the open spaces, by space.
#[derive(Default)]
pub(crate) struct ServerSides {
    running: Mutex<HashMap<String, Running>>,
    binding: AtomicU64,
    /// Told when a side fails after it started, set once by the hub.
    report: Arc<OnceLock<Report>>,
    /// Makes each side's outlet, set once by the hub; without it, as in tests, a side's state goes nowhere.
    outlets: OnceLock<Outlets>,
    /// The sides whose threads have not ended, and what is notified as each ends.
    alive: Arc<Alive>,
}

/// A side for one binding of a space to its environment, which stays until the space is bound elsewhere or closes, so that it is not run again.
struct Running {
    environment: String,
    /// The side it runs, so that an environment offered again with another side, or no longer offered, stops it.
    side: Arc<dyn ServerSide>,
    binding: u64,
    cancellation: Cancellation,
    /// Where intents for the side are handed on, and its inspector found.
    inbox: Inbox,
    /// How many of the side's inspections are under way, those whose waiting has ended but whose inspector has not
    /// returned included.
    inspecting: Arc<AtomicUsize>,
    /// How many actions handed to the side wait for its answer.
    acting: Arc<AtomicUsize>,
}

/// The most inspections of one binding's side under way at once, so that a side that never answers holds at most this
/// many threads.
pub(crate) const INSPECTIONS: usize = 4;

/// One inspection under way, counted against [`INSPECTIONS`] until it is dropped.
pub(crate) struct Inspecting(Arc<AtomicUsize>);

impl Drop for Inspecting {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Why a side cannot be asked to inspect something.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Uninspectable {
    /// No side runs for the space, or it gave no inspector.
    Silent,
    /// [`INSPECTIONS`] are under way already.
    Busy,
}

/// The most actions of one binding's side waiting for its answer at once, as for inspections. See
/// `design/environment-actions.md`.
pub(crate) const ACTIONS: usize = 4;

/// An action handed to a side: where to wait for its answer, its ticket, and its count against [`ACTIONS`], until it is
/// dropped.
pub(crate) struct Handed {
    pub inbox: Inbox,
    pub ticket: u64,
    _acting: Inspecting,
}

/// Why an action cannot be handed to a side.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unactable {
    /// No side runs for the space's binding.
    Silent,
    /// [`ACTIONS`] wait for its answer already.
    Busy,
    /// Its inbox is full, as it is not keeping up.
    Full,
}

#[derive(Default)]
struct Alive {
    count: AtomicUsize,
    ended: Event,
}

/// Counts a side's thread as alive until it ends, however it ends.
struct Living(Arc<Alive>);

impl Living {
    fn new(alive: &Arc<Alive>) -> Self {
        alive.count.fetch_add(1, Ordering::SeqCst);
        Self(alive.clone())
    }
}

impl Drop for Living {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::SeqCst);
        self.0.ended.notify(usize::MAX);
    }
}

/// ServerSides still running when their hub is dropped, as one that never served is, are cancelled with it.
impl Drop for ServerSides {
    fn drop(&mut self) {
        for running in self
            .running
            .get_mut()
            .expect("never left poisoned")
            .values()
        {
            running.cancellation.cancel();
        }
    }
}

impl ServerSides {
    /// Tell `report` of each side that fails after it started. Set once; the hub does so when it is made.
    pub(crate) fn on_failure(&self, report: impl Fn(Failure) + Send + Sync + 'static) {
        let _ = self.report.set(Box::new(report));
    }

    /// Send each side's state, events and descriptions to the outlet `outlets` makes for its space, binding and environment. Set once; the hub does so when it is made.
    pub(crate) fn on_outlet(
        &self,
        outlets: impl Fn(&str, u64, &crate::environments::Environment) -> Arc<dyn Outlet>
        + Send
        + Sync
        + 'static,
    ) {
        let _ = self.outlets.set(Box::new(outlets));
    }

    /// Hand `intent` on to the side running for `space`, if there is one; says whether it was handed on.
    pub(crate) fn deliver(&self, space: &str, intent: Intent) -> bool {
        self.running
            .lock()
            .expect("never left poisoned")
            .get(space)
            .is_some_and(|running| running.inbox.push(intent))
    }

    /// The inspector the side running for `space` gave, with the inspection counted until it is dropped, or why there is
    /// none to ask.
    pub(crate) fn inspector(
        &self,
        space: &str,
    ) -> Result<(Arc<dyn Inspector>, Inspecting), Uninspectable> {
        let running = self.running.lock().expect("never left poisoned");
        let running = running.get(space).ok_or(Uninspectable::Silent)?;
        let inspector = running.inbox.inspector().ok_or(Uninspectable::Silent)?;
        // Counted first, so that it is uncounted as it is dropped, whether or not it is under way.
        let inspecting = Inspecting(running.inspecting.clone());
        if running.inspecting.fetch_add(1, Ordering::SeqCst) >= INSPECTIONS {
            return Err(Uninspectable::Busy);
        }
        Ok((inspector, inspecting))
    }

    /// Hand the action `action` on the object of `reference`, with `parameters`, to the side running for `binding` of
    /// `space`, on the word of `participant`, or why it cannot be.
    pub(crate) fn act(
        &self,
        space: &str,
        binding: u64,
        participant: u32,
        reference: &str,
        action: &str,
        parameters: &str,
    ) -> Result<Handed, Unactable> {
        let running = self.running.lock().expect("never left poisoned");
        let running = running
            .get(space)
            .filter(|running| running.binding == binding)
            .ok_or(Unactable::Silent)?;
        let acting = Inspecting(running.acting.clone());
        if running.acting.fetch_add(1, Ordering::SeqCst) >= ACTIONS {
            return Err(Unactable::Busy);
        }
        let ticket = running
            .inbox
            .act(participant, reference, action, parameters)
            .ok_or(Unactable::Full)?;
        Ok(Handed {
            inbox: running.inbox.clone(),
            ticket,
            _acting: acting,
        })
    }

    /// The binding the side of `space` runs for, if one does.
    pub(crate) fn binding(&self, space: &str) -> Option<u64> {
        self.running
            .lock()
            .expect("never left poisoned")
            .get(space)
            .map(|running| running.binding)
    }

    /// Start the sides the open spaces `wanted` need, stop those whose spaces are no longer there or are bound to another environment, and return the sides that could not be started.
    ///
    /// A side is run once for each binding, so one that has finished or failed stays until its space leaves its environment.
    pub(crate) fn reconcile(&self, environments: &Environments, wanted: &[Wanted]) -> Vec<Failure> {
        let mut running = self.running.lock().expect("never left poisoned");
        running.retain(|id, running| {
            // Bound to the same environment, which still offers the same side.
            let kept = wanted
                .iter()
                .any(|space| space.id == *id && space.environment == running.environment)
                && environments
                    .get(&running.environment)
                    .and_then(|environment| environment.server_side())
                    .is_some_and(|offered| Arc::ptr_eq(offered, &running.side));
            if !kept {
                running.cancellation.cancel();
            }
            kept
        });
        let mut failures = Vec::new();
        for space in wanted {
            if running.contains_key(&space.id) {
                continue;
            }
            let Some(environment) = environments.get(&space.environment) else {
                continue;
            };
            let Some(side) = environment.server_side().cloned() else {
                continue;
            };
            let binding = self.binding.fetch_add(1, Ordering::SeqCst);
            let cancellation = Cancellation::new();
            let label = environment.label().to_owned();
            let offered = side.clone();
            let mut started = Space::with_cancellation(
                space.id.clone(),
                space.title.clone(),
                space.directory.clone(),
                cancellation.clone(),
            );
            if let Some(outlets) = self.outlets.get() {
                started = started.with_outlet(outlets(&space.id, binding, environment));
            }
            let inbox = started.inbox();
            let living = Living::new(&self.alive);
            let report = self.report.clone();
            let id = space.id.clone();
            let thread = crate::system::spawn("tomori-environment", move || {
                let _living = living;
                let space = started;
                let outcome = catch_unwind(AssertUnwindSafe(|| side.run(&space)));
                let message = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(format!(
                        "The {label} environment failed: {}",
                        limits::truncate(error.message(), limits::ERROR_BYTES)
                    )),
                    Err(panic) => {
                        let reason = panic
                            .downcast_ref::<&str>()
                            .map(|reason| (*reason).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "without a message".to_owned());
                        log::error!(
                            "The {label} environment's server side panicked in space {id}: {reason}"
                        );
                        Some(format!("The {label} environment stopped unexpectedly."))
                    }
                };
                // Stopping is not a failure: the space has moved on, or the server is stopping. Finishing before then is
                // told, with no message, so that what the side published rests.
                if !space.cancellation().is_cancelled()
                    && let Some(report) = report.get()
                {
                    report(Failure {
                        space: id,
                        binding,
                        message,
                    });
                }
            });
            if let Err(error) = thread {
                log::error!(
                    "Could not start the {} environment's server side for space {}: {error}",
                    environment.label(),
                    space.id
                );
                failures.push(Failure {
                    space: space.id.clone(),
                    binding,
                    message: Some(format!(
                        "The {} environment could not be started.",
                        environment.label()
                    )),
                });
            }
            running.insert(
                space.id.clone(),
                Running {
                    environment: space.environment.clone(),
                    side: offered,
                    binding,
                    cancellation,
                    inbox,
                    inspecting: Arc::default(),
                    acting: Arc::default(),
                },
            );
        }
        failures
    }

    /// Whether `binding` is the one the space's side is running for, as when it fails.
    pub(crate) fn is_current(&self, space: &str, binding: u64) -> bool {
        self.running
            .lock()
            .expect("never left poisoned")
            .get(space)
            .is_some_and(|side| side.binding == binding)
    }

    /// Wait until every side's thread has ended, as it does once cancelled, or `expiry` is ready, and so abandon the rest. Returns how many were abandoned.
    pub(crate) async fn stopped(&self, expiry: impl std::future::Future<Output = ()>) -> usize {
        let mut expiry = std::pin::pin!(expiry);
        loop {
            let ended: EventListener = self.alive.ended.listen();
            let alive = self.alive.count.load(Ordering::SeqCst);
            if alive == 0 {
                return 0;
            }
            match futures_util::future::select(ended, expiry.as_mut()).await {
                futures_util::future::Either::Left(_) => {}
                futures_util::future::Either::Right(_) => {
                    return self.alive.count.load(Ordering::SeqCst);
                }
            }
        }
    }

    /// How many sides have threads that have not ended.
    #[cfg(test)]
    pub(crate) fn alive(&self) -> usize {
        self.alive.count.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Sender, channel};
    use std::time::Duration;

    use futures_lite::future::block_on;
    use tomori_environment::ServerSide;

    use super::*;
    use crate::environments::Environment;

    /// A side that tells `started` the space it runs for, and then ends as `ending` says.
    struct Test {
        started: Mutex<Sender<String>>,
        ending: fn(&Space) -> tomori_environment::Result<()>,
    }

    impl ServerSide for Test {
        fn run(&self, space: &Space) -> tomori_environment::Result<()> {
            self.started
                .lock()
                .unwrap()
                .send(space.id().to_owned())
                .unwrap();
            (self.ending)(space)
        }
    }

    fn until_cancelled(space: &Space) -> tomori_environment::Result<()> {
        while !space.cancellation().is_cancelled() {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    /// Environments offering `observe`, running a side that ends as `ending` says, and `plain`, which has none.
    fn offered(
        ending: fn(&Space) -> tomori_environment::Result<()>,
    ) -> (Environments, std::sync::mpsc::Receiver<String>) {
        let (sender, receiver) = channel();
        let observe = Environment::new("observe", "Observe").with_server_side(Test {
            started: Mutex::new(sender),
            ending,
        });
        (
            Environments::new([Environment::new("plain", "Plain"), observe]),
            receiver,
        )
    }

    fn wanted(id: &str, environment: &str) -> Wanted {
        Wanted {
            id: id.to_owned(),
            title: "Fleet".to_owned(),
            directory: "/tmp".to_owned(),
            environment: environment.to_owned(),
        }
    }

    /// A `ServerSides` that sends what is reported to the returned receiver.
    fn reporting() -> (ServerSides, std::sync::mpsc::Receiver<Failure>) {
        let sides = ServerSides::default();
        let (sender, receiver) = channel();
        let sender = Mutex::new(sender);
        sides.on_failure(move |failure| sender.lock().unwrap().send(failure).unwrap());
        (sides, receiver)
    }

    fn eventually(what: &str, ready: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_side_is_started_once_for_each_binding_and_cancelled_when_it_ends() {
        let sides = ServerSides::default();
        let (environments, started) = offered(until_cancelled);
        let open = [wanted("space-1", "observe")];
        assert!(sides.reconcile(&environments, &open).is_empty());
        assert_eq!(
            started.recv_timeout(Duration::from_secs(5)).unwrap(),
            "space-1"
        );
        // Reconciling again, as every change to the hub does, starts nothing more.
        assert!(sides.reconcile(&environments, &open).is_empty());
        assert!(sides.reconcile(&environments, &open).is_empty());
        assert!(started.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(sides.alive(), 1);

        // A space that is no longer open, or is bound elsewhere, has its side cancelled; the thread ends.
        sides.reconcile(&environments, &[wanted("space-1", "plain")]);
        eventually("the side never ended", || sides.alive() == 0);
        assert!(started.try_recv().is_err());

        // Bound again, it is a new binding, and runs again.
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        assert_eq!(
            started.recv_timeout(Duration::from_secs(5)).unwrap(),
            "space-1"
        );
        sides.reconcile(&environments, &[]);
        eventually("the side never ended", || sides.alive() == 0);
    }

    #[test]
    fn an_environment_offered_again_with_another_side_or_none_stops_the_side_it_ran() {
        let sides = ServerSides::default();
        let (environments, started) = offered(until_cancelled);
        let open = [wanted("space-1", "observe")];
        sides.reconcile(&environments, &open);
        started.recv_timeout(Duration::from_secs(5)).unwrap();

        // The same environment with another side: the first stops, and the other runs for the space.
        let (replaced, started_again) = offered(until_cancelled);
        sides.reconcile(&replaced, &open);
        assert_eq!(
            started_again.recv_timeout(Duration::from_secs(5)).unwrap(),
            "space-1"
        );
        eventually("the first side never ended", || sides.alive() == 1);

        // The environment without a side, or no longer offered, leaves it running none.
        let plain = Environments::new([Environment::new("observe", "Observe")]);
        sides.reconcile(&plain, &open);
        eventually("the side never ended", || sides.alive() == 0);
        sides.reconcile(&Environments::new([]), &open);
        assert_eq!(sides.alive(), 0);
    }

    #[test]
    fn dropped_sides_are_cancelled() {
        let sides = ServerSides::default();
        let alive = sides.alive.clone();
        let (environments, started) = offered(until_cancelled);
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(alive.count.load(Ordering::SeqCst), 1);
        drop(sides);
        eventually("the side never ended", || {
            alive.count.load(Ordering::SeqCst) == 0
        });
    }

    #[test]
    fn an_environment_that_is_unknown_or_has_no_side_starts_nothing() {
        let sides = ServerSides::default();
        let (environments, started) = offered(until_cancelled);
        let open = [wanted("a", "plain"), wanted("b", "gone")];
        assert!(sides.reconcile(&environments, &open).is_empty());
        assert!(started.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(sides.alive(), 0);
        assert!(!sides.is_current("a", 0));
    }

    #[test]
    fn a_failure_is_reported_for_its_binding_and_the_side_is_not_run_again() {
        for (ending, message) in [
            (
                (|_: &Space| Err("no route".into()))
                    as fn(&Space) -> tomori_environment::Result<()>,
                "The Observe environment failed: no route",
            ),
            (
                |_| panic!("no route"),
                "The Observe environment stopped unexpectedly.",
            ),
            (
                |_| panic!("{}", "no route".to_owned()),
                "The Observe environment stopped unexpectedly.",
            ),
            (
                |_| std::panic::panic_any(7),
                "The Observe environment stopped unexpectedly.",
            ),
        ] {
            let (sides, reported) = reporting();
            let (environments, started) = offered(ending);
            let open = [wanted("space-1", "observe")];
            sides.reconcile(&environments, &open);
            started.recv_timeout(Duration::from_secs(5)).unwrap();
            let failure = reported.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(failure.space, "space-1");
            assert_eq!(failure.message.as_deref(), Some(message));
            assert!(sides.is_current(&failure.space, failure.binding));
            assert!(!sides.is_current(&failure.space, failure.binding + 1));
            // Not started again for the same binding.
            sides.reconcile(&environments, &open);
            assert!(started.recv_timeout(Duration::from_millis(50)).is_err());
        }
    }

    #[test]
    fn an_error_is_cut_and_stopping_is_not_a_failure() {
        let (sides, reported) = reporting();
        let (environments, started) = offered(|space| {
            until_cancelled(space).expect("it ends once cancelled");
            Err("late".repeat(1000).into())
        });
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        sides.reconcile(&environments, &[]);
        eventually("the side never ended", || sides.alive() == 0);
        assert!(reported.try_recv().is_err());

        // An error that is not from stopping is cut to the limit.
        let (environments, started) = offered(|_| Err("long".repeat(1000).into()));
        sides.reconcile(&environments, &[wanted("space-2", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let failure = reported.recv_timeout(Duration::from_secs(5)).unwrap();
        let message = failure.message.unwrap();
        assert!(message.len() < 2 * limits::ERROR_BYTES);
        assert!(message.contains(limits::TRUNCATED));
    }

    #[test]
    fn a_side_that_finishes_before_its_space_is_done_is_told_without_a_message() {
        let (sides, reported) = reporting();
        let (environments, started) = offered(|_| Ok(()));
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let finished = reported.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            (finished.space.as_str(), finished.message),
            ("space-1", None)
        );
    }

    #[test]
    fn a_failure_with_nobody_to_tell_is_only_logged() {
        let sides = ServerSides::default();
        let (environments, started) = offered(|_| Err("no route".into()));
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        eventually("the side never ended", || sides.alive() == 0);
    }

    #[test]
    fn a_side_that_cannot_be_started_is_a_failure_that_is_not_retried() {
        let sides = ServerSides::default();
        let (environments, started) = offered(until_cancelled);
        let open = [wanted("space-1", "observe")];
        crate::system::refuse("tomori-environment");
        let failures = sides.reconcile(&environments, &open);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].space, "space-1");
        assert_eq!(
            failures[0].message.as_deref(),
            Some("The Observe environment could not be started.")
        );
        assert!(sides.is_current("space-1", failures[0].binding));
        assert_eq!(sides.alive(), 0);
        // Not retried for the same binding.
        assert!(sides.reconcile(&environments, &open).is_empty());
        assert!(started.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn a_side_s_inspector_is_asked_at_most_a_few_times_at_once() {
        let sides = ServerSides::default();
        let (environments, started) = offered(|space| {
            space.inspect_with(
                |reference: &str| -> tomori_environment::Result<Option<String>> {
                    Ok(Some(format!("\"{reference}\"")))
                },
            );
            until_cancelled(space)
        });
        assert_eq!(
            sides.inspector("space-1").err(),
            Some(Uninspectable::Silent)
        );
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        eventually("the side never gave its inspector", || {
            sides.inspector("space-1").is_ok()
        });
        let (inspector, _) = sides.inspector("space-1").unwrap();
        assert_eq!(
            inspector.inspect("hen/pip"),
            Ok(Some("\"hen/pip\"".to_owned()))
        );
        let under_way: Vec<_> = (0..INSPECTIONS)
            .map(|_| sides.inspector("space-1").unwrap())
            .collect();
        assert_eq!(sides.inspector("space-1").err(), Some(Uninspectable::Busy));
        drop(under_way);
        assert!(sides.inspector("space-1").is_ok());
        // A side that gives none has nothing to ask.
        let (environments, started) = offered(until_cancelled);
        sides.reconcile(&environments, &[wanted("space-2", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            sides.inspector("space-2").err(),
            Some(Uninspectable::Silent)
        );
        sides.reconcile(&environments, &[]);
        eventually("the sides never ended", || sides.alive() == 0);
    }

    #[test]
    fn an_action_is_handed_only_to_its_binding_s_side_a_few_at_once() {
        let sides = ServerSides::default();
        let (environments, started) = offered(until_cancelled);
        assert_eq!(
            sides.act("space-1", 0, 1, "hen/pip", "peck", "{}").err(),
            Some(Unactable::Silent)
        );
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let binding = sides.binding("space-1").unwrap();
        // Another binding's side is never handed it.
        assert_eq!(
            sides
                .act("space-1", binding + 1, 1, "hen/pip", "peck", "{}")
                .err(),
            Some(Unactable::Silent)
        );
        let waiting: Vec<Handed> = (0..ACTIONS)
            .map(|_| {
                sides
                    .act("space-1", binding, 1, "hen/pip", "peck", "{}")
                    .unwrap()
            })
            .collect();
        assert_eq!(waiting[0].ticket, 1);
        assert_eq!(
            sides
                .act("space-1", binding, 1, "hen/pip", "peck", "{}")
                .err(),
            Some(Unactable::Busy)
        );
        drop(waiting);
        // A side not keeping up, its inbox full, is handed nothing more.
        for _ in 0..tomori_environment::WAITING_INTENT_LIMIT - ACTIONS {
            assert!(sides.deliver("space-1", Intent::new(1, "{}")));
        }
        assert_eq!(
            sides
                .act("space-1", binding, 1, "hen/pip", "peck", "{}")
                .err(),
            Some(Unactable::Full)
        );
        sides.reconcile(&environments, &[]);
        eventually("the sides never ended", || sides.alive() == 0);
    }

    #[test]
    fn stopping_waits_for_the_sides_to_end_and_abandons_those_that_do_not() {
        let sides = ServerSides::default();
        assert_eq!(block_on(sides.stopped(std::future::pending())), 0);

        // Cancelled, the side ends while the wait is under way.
        let (environments, started) = offered(until_cancelled);
        sides.reconcile(&environments, &[wanted("space-1", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        sides.reconcile(&environments, &[]);
        assert_eq!(block_on(sides.stopped(std::future::pending())), 0);

        // One that ignores its cancellation is abandoned when the wait expires.
        let (environments, started) = offered(|_| {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        });
        sides.reconcile(&environments, &[wanted("space-2", "observe")]);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        sides.reconcile(&environments, &[]);
        assert_eq!(block_on(sides.stopped(std::future::ready(()))), 1);
    }
}
