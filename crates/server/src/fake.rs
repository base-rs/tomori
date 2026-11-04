// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A scripted in-memory backend for tests and for running the server without an agent.
//!
//! Sessions offer two modes, `default` and `plan`, and confirm every switch to one of them. They take instruction entries, which tests can read with [`FakeSession::instructions`], unless the backend was told to take none with [`FakeBackend::take_no_instructions`]. In automatic mode, each prompt is answered immediately with a streamed echo. In manual mode, a turn stays busy until the test calls [`FakeSession::reply`] or scripts the reply's text deltas with [`FakeSession::stream`], so input queues up behind it. Tests can inject any [`Event`] and inspect what reached the session.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use base_scheduler::Cancellation;
use tomori_backend::{
    AnswerValue, Backend, Capabilities, Decision, Error, ErrorKind, Event, Mode, Modes, Permission,
    Prompt, Question, Result, Role, Session, SessionSpec, SessionSummary, Snapshot,
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
    /// The instruction entries the server set, or removed with `None`, in order, including those refused.
    pub instructions: Vec<(String, Option<String>)>,
    /// How many of [`Self::instructions`] the server had given when each prompt was delivered.
    pub instructed_at_prompts: Vec<usize>,
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
    /// While set, decisions wait; see [`FakeBackend::hold_commands`].
    commands_held: Arc<AtomicBool>,
    commands_released: Arc<event_listener::Event>,
    next: AtomicU64,
    sessions: Mutex<Vec<FakeSession>>,
    /// The backend takes no instructions; see [`FakeBackend::take_no_instructions`].
    no_instructions: AtomicBool,
    /// Setting instructions fails as unavailable; see [`FakeBackend::fail_instructions`].
    instruction_failure: Arc<AtomicBool>,
    /// The sessions whose instruction entries were forgotten, with the keys, in order.
    forgotten: Mutex<Vec<(String, Vec<String>)>>,
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

    /// Take no instructions, as a backend without [`Capabilities::instructions`].
    pub fn take_no_instructions(&self) {
        self.shared.no_instructions.store(true, Ordering::SeqCst);
    }

    /// Make setting instructions fail, as an agent whose endpoint for them changed would.
    pub fn fail_instructions(&self, fail: bool) {
        self.shared
            .instruction_failure
            .store(fail, Ordering::SeqCst);
    }

    /// The sessions whose instruction entries the server forgot, with the keys, in order.
    pub fn forgotten(&self) -> Vec<(String, Vec<String>)> {
        self.shared.forgotten.lock().unwrap().clone()
    }

    /// Make creating and attaching sessions wait until [`release_creation`](Self::release_creation), as a slow agent start would.
    pub fn hold_creation(&self) {
        self.shared.held.store(true, Ordering::SeqCst);
    }

    pub fn release_creation(&self) {
        self.shared.held.store(false, Ordering::SeqCst);
        self.shared.released.notify(usize::MAX);
    }

    /// Make decisions wait until [`release_commands`](Self::release_commands), as an agent whose connection has
    /// dropped would.
    pub fn hold_commands(&self) {
        self.shared.commands_held.store(true, Ordering::SeqCst);
    }

    pub fn release_commands(&self) {
        self.shared.commands_held.store(false, Ordering::SeqCst);
        self.shared.commands_released.notify(usize::MAX);
    }

    /// Sessions whose creation is waiting.
    pub fn creating(&self) -> u64 {
        self.shared.creating.load(Ordering::SeqCst)
    }

    /// Wait while creating and attaching are held.
    async fn released(&self) {
        while self.shared.held.load(Ordering::SeqCst) {
            let listener = self.shared.released.listen();
            if !self.shared.held.load(Ordering::SeqCst) {
                break;
            }
            listener.await;
        }
    }

    /// Every session created or attached so far, oldest first.
    pub fn started(&self) -> Vec<FakeSession> {
        self.shared.sessions.lock().unwrap().clone()
    }

    fn start(&self, title: &str, directory: &str) -> FakeSession {
        let number = self.shared.next.fetch_add(1, Ordering::SeqCst) + 1;
        let handle = Arc::new(FakeHandle::new(1));
        let session = FakeSession {
            handle: handle.clone(),
            inner: Arc::new(FakeSessionInner {
                id: format!("fake-{number}"),
                title: title.to_owned(),
                directory: directory.to_owned(),
                manual: self.shared.manual.load(Ordering::SeqCst),
                snapshot_failure: self.shared.snapshot_failure.clone(),
                commands_held: self.shared.commands_held.clone(),
                commands_released: self.shared.commands_released.clone(),
                instruction_failure: self.shared.instruction_failure.clone(),
                instructions: Mutex::new(BTreeMap::new()),
                outbound: Mutex::new(FakeOutbound {
                    current: Some(handle),
                    backlog: VecDeque::new(),
                }),
                busy: AtomicBool::new(false),
                bound: Mutex::new(Some(1)),
                next_generation: AtomicU64::new(1),
                permissions: Mutex::new(Vec::new()),
                questions: Mutex::new(Vec::new()),
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
            instructions: !self.shared.no_instructions.load(Ordering::SeqCst),
        }
    }

    async fn forget_instructions(&self, id: &str, keys: Vec<String>) -> Result<()> {
        if self.shared.no_instructions.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Unsupported, "no instructions"));
        }
        if let Some(session) = self
            .started()
            .into_iter()
            .find(|session| session.inner.id == id)
        {
            // Bound again: its entries are the new binding's.
            if session.inner.bound.lock().unwrap().is_some() {
                return Ok(());
            }
            let mut entries = session.inner.instructions.lock().unwrap();
            for key in &keys {
                entries.remove(key);
            }
        }
        self.shared
            .forgotten
            .lock()
            .unwrap()
            .push((id.to_owned(), keys));
        Ok(())
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
        self.released().await;
        self.shared.creating.fetch_sub(1, Ordering::SeqCst);
        Ok(self.start(&spec.title, &spec.directory))
    }

    async fn attach(&self, id: &str, _directory: &str) -> Result<FakeSession> {
        self.released().await;
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
        let generation = session.inner.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let handle = Arc::new(FakeHandle::new(generation));
        let mut outbound = session.inner.outbound.lock().unwrap();
        while let Some(event) = outbound.backlog.pop_front() {
            let _ = handle.sender.try_send(event);
        }
        outbound.current = Some(handle.clone());
        *bound = Some(generation);
        let inner = session.inner.clone();
        drop(outbound);
        drop(bound);
        Ok(FakeSession { inner, handle })
    }
}

/// A handle to one fake session. Clones share the session.
#[derive(Clone)]
pub struct FakeSession {
    inner: Arc<FakeSessionInner>,
    handle: Arc<FakeHandle>,
}

struct FakeHandle {
    generation: u64,
    closing: Cancellation,
    closed: AtomicBool,
    sender: async_channel::Sender<Event>,
    receiver: async_channel::Receiver<Event>,
}

impl FakeHandle {
    fn new(generation: u64) -> Self {
        let (sender, receiver) = async_channel::unbounded();
        Self {
            generation,
            closing: Cancellation::new(),
            closed: AtomicBool::new(false),
            sender,
            receiver,
        }
    }
}

struct FakeOutbound {
    current: Option<Arc<FakeHandle>>,
    backlog: VecDeque<Event>,
}

struct FakeSessionInner {
    id: String,
    title: String,
    directory: String,
    manual: bool,
    snapshot_failure: Arc<AtomicBool>,
    commands_held: Arc<AtomicBool>,
    commands_released: Arc<event_listener::Event>,
    instruction_failure: Arc<AtomicBool>,
    /// The session's instruction entries, as the agent would keep them.
    instructions: Mutex<BTreeMap<String, String>>,
    outbound: Mutex<FakeOutbound>,
    busy: AtomicBool,
    bound: Mutex<Option<u64>>,
    next_generation: AtomicU64,
    permissions: Mutex<Vec<Permission>>,
    questions: Mutex<Vec<Question>>,
    next_message: AtomicU64,
    delivered: Mutex<HashSet<String>>,
    modes: Mutex<Modes>,
    log: Mutex<FakeLog>,
}

impl FakeSession {
    fn current_binding(&self) -> Result<MutexGuard<'_, Option<u64>>> {
        let binding = self.inner.bound.lock().unwrap();
        if *binding == Some(self.handle.generation) {
            Ok(binding)
        } else {
            Err(Error::new(
                ErrorKind::Closed,
                "the fake session handle has closed",
            ))
        }
    }

    /// Report an event as if the agent produced it.
    pub fn emit(&self, event: Event) {
        match &event {
            Event::Busy => self.inner.busy.store(true, Ordering::SeqCst),
            Event::Idle => self.inner.busy.store(false, Ordering::SeqCst),
            Event::PermissionRequested(permission) => {
                let mut pending = self.inner.permissions.lock().unwrap();
                if !pending.iter().any(|entry| entry.id == permission.id) {
                    pending.push(permission.clone());
                }
            }
            Event::PermissionResolved { id } => self
                .inner
                .permissions
                .lock()
                .unwrap()
                .retain(|entry| entry.id != *id),
            Event::QuestionAsked(question) => {
                let mut pending = self.inner.questions.lock().unwrap();
                if !pending.iter().any(|entry| entry.id == question.id) {
                    pending.push(question.clone());
                }
            }
            Event::QuestionResolved { id } => self
                .inner
                .questions
                .lock()
                .unwrap()
                .retain(|entry| entry.id != *id),
            _ => {}
        }
        let mut outbound = self.inner.outbound.lock().unwrap();
        if let Some(handle) = &outbound.current {
            let _ = handle.sender.try_send(event);
        } else {
            outbound.backlog.push_back(event);
        }
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

    /// The session's instruction entries now.
    pub fn instructions(&self) -> BTreeMap<String, String> {
        self.inner.instructions.lock().unwrap().clone()
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
        if self.handle.closed.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Closed, "the fake session closed"));
        }
        // Prefer cancellation if an old handle closes while a new one has
        // already received events; the old waiter must not steal them.
        let event = futures_lite::future::or(
            async {
                self.handle.closing.cancelled().await;
                Ok(Event::Closed { reason: None })
            },
            async {
                self.handle
                    .receiver
                    .recv()
                    .await
                    .map_err(|_| Error::new(ErrorKind::Closed, "the fake session closed"))
            },
        )
        .await?;
        if matches!(event, Event::Closed { .. }) {
            self.handle.closed.store(true, Ordering::SeqCst);
        }
        Ok(event)
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        let _binding = self.current_binding()?;
        if self.inner.snapshot_failure.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Unavailable, "snapshot unavailable"));
        }
        Ok(Snapshot {
            busy: Some(self.is_busy()),
            permissions: self.inner.permissions.lock().unwrap().clone(),
            questions: self.inner.questions.lock().unwrap().clone(),
            modes: self.inner.modes.lock().unwrap().clone(),
            ..Snapshot::default()
        })
    }

    async fn send(&self, prompt: Prompt) -> Result<()> {
        let _binding = self.current_binding()?;
        if !self
            .inner
            .delivered
            .lock()
            .unwrap()
            .insert(prompt.key.clone())
        {
            return Ok(());
        }
        {
            let mut log = self.inner.log.lock().unwrap();
            log.prompts.push(prompt.clone());
            let given = log.instructions.len();
            log.instructed_at_prompts.push(given);
        }
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
        loop {
            if !self.inner.commands_held.load(Ordering::SeqCst) {
                break;
            }
            let released = self.inner.commands_released.listen();
            if !self.inner.commands_held.load(Ordering::SeqCst) {
                break;
            }
            released.await;
        }
        let _binding = self.current_binding()?;
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
        let _binding = self.current_binding()?;
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
        let _binding = self.current_binding()?;
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

    async fn set_instruction(&self, key: &str, value: Option<&str>) -> Result<()> {
        let _binding = self.current_binding()?;
        if !tomori_backend::valid_instruction_key(key) {
            return Err(Error::new(ErrorKind::Invalid, "not an instruction key"));
        }
        self.inner
            .log
            .lock()
            .unwrap()
            .instructions
            .push((key.to_owned(), value.map(str::to_owned)));
        if self.inner.instruction_failure.load(Ordering::SeqCst) {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "the fake agent refused the instructions",
            ));
        }
        let mut entries = self.inner.instructions.lock().unwrap();
        match value {
            Some(value) => entries.insert(key.to_owned(), value.to_owned()),
            None => entries.remove(key),
        };
        Ok(())
    }

    async fn interrupt(&self) -> Result<()> {
        let _binding = self.current_binding()?;
        self.inner.log.lock().unwrap().interrupts += 1;
        self.emit(Event::Idle);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let mut bound = self.inner.bound.lock().unwrap();
        if *bound != Some(self.handle.generation) {
            return Ok(());
        }
        self.inner.log.lock().unwrap().closed = true;
        self.handle.closing.cancel();
        let mut outbound = self.inner.outbound.lock().unwrap();
        while let Ok(event) = self.handle.receiver.try_recv() {
            outbound.backlog.push_back(event);
        }
        outbound.current = None;
        *bound = None;
        Ok(())
    }
}
