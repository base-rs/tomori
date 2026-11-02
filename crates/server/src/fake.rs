// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A scripted in-memory backend for tests and for running the server without an agent.
//!
//! Sessions offer two modes, `default` and `plan`, and confirm every switch to one of them. In automatic mode, each prompt is answered immediately with a streamed echo. In manual mode, a turn stays busy until the test calls [`FakeSession::reply`] or scripts the reply's text deltas with [`FakeSession::stream`], so input queues up behind it. Tests can inject any [`Event`] and inspect what reached the session.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use backend::{
    AnswerValue, Backend, Capabilities, Decision, Error, ErrorKind, Event, Mode, Modes, Prompt,
    Result, Role, Session, SessionSpec, SessionSummary, Snapshot,
};

/// The backend name used in `CreateSpace` requests.
pub const FAKE_BACKEND_NAME: &str = "fake";

/// Everything the server asked a [`FakeSession`] to do, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FakeLog {
    pub prompts: Vec<Prompt>,
    pub decisions: Vec<(String, Decision)>,
    pub answers: Vec<(String, Vec<(String, AnswerValue)>)>,
    pub interrupts: usize,
    /// The modes the server asked for, in order.
    pub modes: Vec<String>,
    /// Whether the server closed the session.
    pub closed: bool,
}

#[derive(Clone, Default)]
pub struct FakeBackend {
    shared: Arc<FakeShared>,
}

#[derive(Default)]
struct FakeShared {
    manual: AtomicBool,
    snapshot_failure: Arc<AtomicBool>,
    /// While set, session creation waits; see [`FakeBackend::hold_creation`].
    held: AtomicBool,
    released: event_listener::Event,
    creating: AtomicU64,
    next: AtomicU64,
    sessions: Mutex<Vec<FakeSession>>,
}

impl FakeBackend {
    /// A backend whose sessions answer every prompt immediately.
    pub fn new() -> Self {
        Self::default()
    }

    /// A backend whose turns stay busy until [`FakeSession::reply`] is called.
    pub fn manual() -> Self {
        let backend = Self::default();
        backend.shared.manual.store(true, Ordering::SeqCst);
        backend
    }

    /// Simulate an unavailable initial session snapshot.
    pub fn fail_snapshots(&self, fail: bool) {
        self.shared.snapshot_failure.store(fail, Ordering::SeqCst);
    }

    /// Make session creation wait until [`release_creation`](Self::release_creation), as a slow agent start would.
    pub fn hold_creation(&self) {
        self.shared.held.store(true, Ordering::SeqCst);
    }

    pub fn release_creation(&self) {
        self.shared.held.store(false, Ordering::SeqCst);
        self.shared.released.notify(usize::MAX);
    }

    /// Sessions whose creation is waiting.
    pub fn creating(&self) -> u64 {
        self.shared.creating.load(Ordering::SeqCst)
    }

    /// Every session created or attached so far, oldest first.
    pub fn started(&self) -> Vec<FakeSession> {
        self.shared.sessions.lock().unwrap().clone()
    }

    fn start(&self, title: &str, directory: &str) -> FakeSession {
        let number = self.shared.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (sender, receiver) = async_channel::unbounded();
        let session = FakeSession {
            generation: 1,
            inner: Arc::new(FakeSessionInner {
                id: format!("fake-{number}"),
                title: title.to_owned(),
                directory: directory.to_owned(),
                manual: self.shared.manual.load(Ordering::SeqCst),
                snapshot_failure: self.shared.snapshot_failure.clone(),
                sender,
                receiver,
                busy: AtomicBool::new(false),
                closed: AtomicBool::new(false),
                bound: Mutex::new(Some(1)),
                next_generation: AtomicU64::new(1),
                next_message: AtomicU64::new(0),
                delivered: Mutex::new(HashSet::new()),
                modes: Mutex::new(Modes {
                    offered: ["default", "plan"]
                        .map(|id| Mode {
                            id: id.to_owned(),
                            name: id.to_owned(),
                            description: None,
                        })
                        .into(),
                    current: Some("default".to_owned()),
                }),
                log: Mutex::new(FakeLog::default()),
            }),
        };
        self.shared.sessions.lock().unwrap().push(session.clone());
        session
    }
}

impl Backend for FakeBackend {
    type Session = FakeSession;

    fn name(&self) -> &str {
        FAKE_BACKEND_NAME
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            attach: true,
            interrupt: true,
            questions: true,
            workers: true,
        }
    }

    async fn sessions(&self, directory: &str) -> Result<Vec<SessionSummary>> {
        Ok(self
            .started()
            .iter()
            .filter(|session| session.inner.directory == directory)
            .map(|session| SessionSummary {
                id: session.inner.id.clone(),
                title: session.inner.title.clone(),
                directory: session.inner.directory.clone(),
            })
            .collect())
    }

    async fn create(&self, spec: SessionSpec) -> Result<FakeSession> {
        self.shared.creating.fetch_add(1, Ordering::SeqCst);
        while self.shared.held.load(Ordering::SeqCst) {
            let listener = self.shared.released.listen();
            if !self.shared.held.load(Ordering::SeqCst) {
                break;
            }
            listener.await;
        }
        self.shared.creating.fetch_sub(1, Ordering::SeqCst);
        Ok(self.start(&spec.title, &spec.directory))
    }

    async fn attach(&self, id: &str, _directory: &str) -> Result<FakeSession> {
        let session = self
            .started()
            .into_iter()
            .find(|session| session.inner.id == id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no fake session {id}")))?;
        let mut bound = session.inner.bound.lock().unwrap();
        if bound.is_some() {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "the fake session is already attached",
            ));
        }
        // A previous handle's closed notification must not end the new one.
        while session.inner.receiver.try_recv().is_ok() {}
        session.inner.closed.store(false, Ordering::SeqCst);
        let generation = session.inner.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        *bound = Some(generation);
        let inner = session.inner.clone();
        drop(bound);
        Ok(FakeSession { inner, generation })
    }
}

/// A handle to one fake session. Clones share the session.
#[derive(Clone)]
pub struct FakeSession {
    inner: Arc<FakeSessionInner>,
    generation: u64,
}

struct FakeSessionInner {
    id: String,
    title: String,
    directory: String,
    manual: bool,
    snapshot_failure: Arc<AtomicBool>,
    sender: async_channel::Sender<Event>,
    receiver: async_channel::Receiver<Event>,
    busy: AtomicBool,
    closed: AtomicBool,
    bound: Mutex<Option<u64>>,
    next_generation: AtomicU64,
    next_message: AtomicU64,
    delivered: Mutex<HashSet<String>>,
    modes: Mutex<Modes>,
    log: Mutex<FakeLog>,
}

impl FakeSession {
    fn ensure_current(&self) -> Result<()> {
        if *self.inner.bound.lock().unwrap() == Some(self.generation) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Closed,
                "the fake session handle has closed",
            ))
        }
    }

    /// Report an event as if the agent produced it.
    pub fn emit(&self, event: Event) {
        match event {
            Event::Busy => self.inner.busy.store(true, Ordering::SeqCst),
            Event::Idle => self.inner.busy.store(false, Ordering::SeqCst),
            _ => {}
        }
        // The receiver lives as long as the session, so sending cannot fail.
        let _ = self.inner.sender.try_send(event);
    }

    /// Stream an assistant reply in two deltas, complete it, and end the turn.
    pub fn reply(&self, text: &str) {
        let middle = text.char_indices().nth(text.chars().count() / 2);
        let (first, second) = text.split_at(middle.map_or(text.len(), |(index, _)| index));
        self.stream(&[first, second]);
    }

    /// Stream an assistant reply as exactly these text deltas, complete it, and end the turn. Returns the message identifier.
    pub fn stream(&self, deltas: &[&str]) -> String {
        let message = self.start_message();
        for delta in deltas {
            self.delta(&message, delta);
        }
        self.emit(Event::MessageCompleted {
            message: message.clone(),
        });
        self.emit(Event::Idle);
        message
    }

    /// Start a new assistant message and return its identifier. Follow with [`delta`](Self::delta) and a [`Event::MessageCompleted`].
    pub fn start_message(&self) -> String {
        let number = self.inner.next_message.fetch_add(1, Ordering::SeqCst) + 1;
        let message = format!("{}-reply-{number}", self.inner.id);
        self.emit(Event::MessageStarted {
            message: message.clone(),
            role: Role::Assistant,
        });
        message
    }

    /// Stream one text delta of an assistant message. Empty deltas are skipped.
    pub fn delta(&self, message: &str, text: &str) {
        if !text.is_empty() {
            self.emit(Event::TextDelta {
                message: message.to_owned(),
                text: text.to_owned(),
            });
        }
    }

    pub fn log(&self) -> FakeLog {
        self.inner.log.lock().unwrap().clone()
    }

    pub fn is_busy(&self) -> bool {
        self.inner.busy.load(Ordering::SeqCst)
    }
}

impl Session for FakeSession {
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    async fn next_event(&self) -> Result<Event> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Closed, "the fake session closed"));
        }
        if self
            .inner
            .bound
            .lock()
            .unwrap()
            .is_some_and(|generation| generation != self.generation)
        {
            return Err(Error::new(
                ErrorKind::Closed,
                "the fake session handle has closed",
            ));
        }
        let event = self
            .inner
            .receiver
            .recv()
            .await
            .map_err(|_| Error::new(ErrorKind::Closed, "the fake session closed"))?;
        if matches!(event, Event::Closed { .. }) {
            self.inner.closed.store(true, Ordering::SeqCst);
        }
        Ok(event)
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        self.ensure_current()?;
        if self.inner.snapshot_failure.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Unavailable, "snapshot unavailable"));
        }
        Ok(Snapshot {
            busy: Some(self.is_busy()),
            modes: self.inner.modes.lock().unwrap().clone(),
            ..Snapshot::default()
        })
    }

    async fn send(&self, prompt: Prompt) -> Result<()> {
        self.ensure_current()?;
        if !self
            .inner
            .delivered
            .lock()
            .unwrap()
            .insert(prompt.key.clone())
        {
            return Ok(());
        }
        self.inner.log.lock().unwrap().prompts.push(prompt.clone());
        self.emit(Event::Delivered {
            key: prompt.key.clone(),
        });
        self.emit(Event::Busy);
        if !self.inner.manual {
            self.reply(&format!("You said: {}", prompt.text));
        }
        Ok(())
    }

    async fn decide(&self, permission: &str, decision: Decision) -> Result<()> {
        self.ensure_current()?;
        self.inner
            .log
            .lock()
            .unwrap()
            .decisions
            .push((permission.to_owned(), decision));
        self.emit(Event::PermissionResolved {
            id: permission.to_owned(),
        });
        Ok(())
    }

    async fn answer(&self, question: &str, answers: Vec<(String, AnswerValue)>) -> Result<()> {
        self.ensure_current()?;
        self.inner
            .log
            .lock()
            .unwrap()
            .answers
            .push((question.to_owned(), answers));
        self.emit(Event::QuestionResolved {
            id: question.to_owned(),
        });
        Ok(())
    }

    async fn set_mode(&self, mode: &str) -> Result<()> {
        self.ensure_current()?;
        self.inner.log.lock().unwrap().modes.push(mode.to_owned());
        let modes = {
            let mut modes = self.inner.modes.lock().unwrap();
            if !modes.offered.iter().any(|offered| offered.id == mode) {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    format!("the fake session has no {mode} mode"),
                ));
            }
            modes.current = Some(mode.to_owned());
            modes.clone()
        };
        self.emit(Event::ModesChanged(modes));
        Ok(())
    }

    async fn interrupt(&self) -> Result<()> {
        self.ensure_current()?;
        self.inner.log.lock().unwrap().interrupts += 1;
        self.emit(Event::Idle);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let mut bound = self.inner.bound.lock().unwrap();
        if *bound != Some(self.generation) {
            return Ok(());
        }
        self.inner.log.lock().unwrap().closed = true;
        if !self.inner.closed.load(Ordering::SeqCst) {
            self.emit(Event::Closed { reason: None });
        }
        *bound = None;
        Ok(())
    }
}
