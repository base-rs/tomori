// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A scripted in-memory backend for tests and for running the server without an agent.
//!
//! Each session keeps the family of workers it last reported with [`Event::WorkersChanged`] or [`Event::WorkersPartial`], as the agent's own record, which [`Backend::delete_worker`] checks and changes; [`FakeSession::set_family`] changes the record alone, as the agent changing behind what it reported would. Sessions offer two modes, `default` and `plan`, and confirm every switch to one of them. They take instruction entries, which tests can read with [`FakeSession::instructions`], unless the backend was told to take none with [`FakeBackend::take_no_instructions`]. In automatic mode, each prompt is answered immediately with a streamed echo. In manual mode, a turn stays busy until the test calls [`FakeSession::reply`] or scripts the reply's text deltas with [`FakeSession::stream`], so input queues up behind it. Tests can inject any [`Event`] and inspect what reached the session.
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use base_scheduler::Cancellation;
use tomori_backend::{
    AnswerValue, Backend, Capabilities, Decision, Error, ErrorKind, Event, Mode, Modes, Permission,
    Prompt, Question, Result, Role, Session, SessionSpec, SessionSummary, Snapshot, ToolServer,
    Worker, WorkerState,
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
    /// Closing a session fails, once it has closed; see [`FakeBackend::fail_closing`].
    closing_failure: Arc<AtomicBool>,
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
    /// The backend cannot attach existing sessions; see [`FakeBackend::attach_nothing`].
    no_attach: AtomicBool,
    /// Setting instructions fails as unavailable; see [`FakeBackend::fail_instructions`].
    instruction_failure: Arc<AtomicBool>,
    /// The sessions whose instruction entries were forgotten, with the keys, in order.
    forgotten: Mutex<Vec<(String, Vec<String>)>>,
    /// The backend cannot delete sessions; see [`FakeBackend::delete_nothing`].
    no_delete: AtomicBool,
    /// Deleting a session fails, as this; see [`FakeBackend::fail_deleting`].
    delete_failure: Mutex<Option<Error>>,
    /// The sessions the server deleted, in order.
    deleted: Mutex<Vec<String>>,
    /// Removing each of these workers fails, as its error; see [`FakeBackend::fail_removing`].
    worker_failures: Mutex<HashMap<String, Error>>,
    /// The workers the server removed, with their own workers, in order.
    removed_workers: Mutex<Vec<String>>,
    /// The running workers stopped to remove them, in order.
    stopped_workers: Mutex<Vec<String>>,
    /// The backend reports no workers; see [`FakeBackend::report_no_workers`].
    no_workers: AtomicBool,
    /// The backend takes no tools; see [`FakeBackend::take_no_tools`].
    no_tools: AtomicBool,
    /// Registering and unregistering tools fails; see [`FakeBackend::fail_tools`].
    tool_failure: AtomicBool,
    /// Each registration of tools, with its directory and server, and each removal, with its directory and `None`.
    tools: Mutex<Vec<(String, Option<ToolServer>)>>,
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

    /// Make closing sessions fail once they have closed, as an agent that does not answer would.
    pub fn fail_closing(&self) {
        self.shared.closing_failure.store(true, Ordering::SeqCst);
    }

    /// Take no instructions, as a backend without [`Capabilities::instructions`].
    pub fn take_no_instructions(&self) {
        self.shared.no_instructions.store(true, Ordering::SeqCst);
    }

    /// Attach no existing sessions, and so list none, as a backend without [`Capabilities::attach`].
    pub fn attach_nothing(&self) {
        self.shared.no_attach.store(true, Ordering::SeqCst);
    }

    /// Make setting instructions fail, as an agent whose endpoint for them changed would.
    pub fn fail_instructions(&self, fail: bool) {
        self.shared
            .instruction_failure
            .store(fail, Ordering::SeqCst);
    }

    /// Report no workers, as a backend without [`Capabilities::workers`].
    pub fn report_no_workers(&self) {
        self.shared.no_workers.store(true, Ordering::SeqCst);
    }

    /// Take no tools, as a backend without [`Capabilities::tools`].
    pub fn take_no_tools(&self) {
        self.shared.no_tools.store(true, Ordering::SeqCst);
    }

    /// Make registering and unregistering tools fail, as an agent whose endpoint for them changed would.
    pub fn fail_tools(&self, fail: bool) {
        self.shared.tool_failure.store(fail, Ordering::SeqCst);
    }

    /// Each registration of tools the server asked for, with its directory and server, and each removal, with its
    /// directory and `None`, in order, including those that failed.
    pub fn tools(&self) -> Vec<(String, Option<ToolServer>)> {
        self.shared.tools.lock().unwrap().clone()
    }

    /// Record a registration or removal of tools, failing it if told to, as an agent might, repeating what it was given.
    fn record_tools(&self, directory: &str, server: Option<ToolServer>) -> Result<()> {
        let given: Vec<String> = server
            .iter()
            .flat_map(|server| &server.headers)
            .map(|(name, value)| format!(", refusing {name}: {value}"))
            .collect();
        self.shared
            .tools
            .lock()
            .unwrap()
            .push((directory.to_owned(), server));
        if self.shared.tool_failure.load(Ordering::SeqCst) {
            return Err(Error::new(
                ErrorKind::Unavailable,
                format!("the tools went away{}", given.concat()),
            ));
        }
        Ok(())
    }

    /// The sessions whose instruction entries the server forgot, with the keys, in order.
    pub fn forgotten(&self) -> Vec<(String, Vec<String>)> {
        self.shared.forgotten.lock().unwrap().clone()
    }

    /// Delete no sessions, as a backend without [`Capabilities::delete`].
    pub fn delete_nothing(&self) {
        self.shared.no_delete.store(true, Ordering::SeqCst);
    }

    /// Make deleting sessions fail with `error`, as an agent that cannot be reached would, or succeed again.
    pub fn fail_deleting(&self, error: Option<Error>) {
        *self.shared.delete_failure.lock().unwrap() = error;
    }

    /// The sessions the server deleted, in order.
    pub fn deleted(&self) -> Vec<String> {
        self.shared.deleted.lock().unwrap().clone()
    }

    /// Make removing the worker `worker` fail with `error`, or succeed again with `None`.
    pub fn fail_removing(&self, worker: &str, error: Option<Error>) {
        let mut failures = self.shared.worker_failures.lock().unwrap();
        match error {
            Some(error) => failures.insert(worker.to_owned(), error),
            None => failures.remove(worker),
        };
    }

    /// The workers the server removed, with their own workers, in order.
    pub fn removed_workers(&self) -> Vec<String> {
        self.shared.removed_workers.lock().unwrap().clone()
    }

    /// The running workers stopped to remove them, in order.
    pub fn stopped_workers(&self) -> Vec<String> {
        self.shared.stopped_workers.lock().unwrap().clone()
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
        loop {
            // Listening before looking, so that a release between the two is not missed.
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
                id: Mutex::new(format!("fake-{number}")),
                title: Mutex::new(title.to_owned()),
                directory: directory.to_owned(),
                manual: self.shared.manual.load(Ordering::SeqCst),
                snapshot_failure: self.shared.snapshot_failure.clone(),
                closing_failure: self.shared.closing_failure.clone(),
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
                family: Mutex::new(Vec::new()),
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
            attach: !self.shared.no_attach.load(Ordering::SeqCst),
            interrupt: true,
            questions: true,
            workers: !self.shared.no_workers.load(Ordering::SeqCst),
            instructions: !self.shared.no_instructions.load(Ordering::SeqCst),
            delete: !self.shared.no_delete.load(Ordering::SeqCst),
            tools: !self.shared.no_tools.load(Ordering::SeqCst),
        }
    }

    /// Waits while commands are held, as an agent whose connection has dropped would; refuses a session bound again;
    /// and forgets the session, so that it can no longer be attached. One that does not exist is not an error.
    async fn delete_session(&self, id: &str) -> Result<()> {
        if self.shared.no_delete.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Unsupported, "no deleting"));
        }
        loop {
            let released = self.shared.commands_released.listen();
            if !self.shared.commands_held.load(Ordering::SeqCst) {
                break;
            }
            released.await;
        }
        if let Some(error) = self.shared.delete_failure.lock().unwrap().clone() {
            return Err(error);
        }
        let mut sessions = self.shared.sessions.lock().unwrap();
        if let Some(session) = sessions.iter().find(|session| session.id() == id)
            && session.inner.bound.lock().unwrap().is_some()
        {
            return Err(Error::new(
                ErrorKind::Invalid,
                "the fake session is bound again",
            ));
        }
        sessions.retain(|session| session.id() != id);
        self.shared.deleted.lock().unwrap().push(id.to_owned());
        Ok(())
    }

    /// Waits while commands are held; checks the worker's chain of parents in the family the session `session` last
    /// reported, as the agent's own record; refuses a session bound to a space, a worker of another session and,
    /// without `force`, a running worker or one with running workers of its own; and then removes it with its own
    /// workers, reporting the family left.
    async fn delete_worker(&self, session: &str, worker: &str, force: bool) -> Result<()> {
        if self.shared.no_delete.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Unsupported, "no deleting"));
        }
        loop {
            let released = self.shared.commands_released.listen();
            if !self.shared.commands_held.load(Ordering::SeqCst) {
                break;
            }
            released.await;
        }
        if let Some(error) = self.shared.worker_failures.lock().unwrap().get(worker) {
            return Err(error.clone());
        }
        let sessions = self.started();
        if sessions
            .iter()
            .any(|bound| bound.id() == worker && bound.inner.bound.lock().unwrap().is_some())
        {
            return Err(Error::new(
                ErrorKind::Invalid,
                "the fake session is bound to a space",
            ));
        }
        let root = sessions
            .iter()
            .find(|root| root.id() == session)
            .ok_or_else(|| Error::new(ErrorKind::Invalid, "no such fake session"))?;
        let family = root.inner.family.lock().unwrap().clone();
        if !family.iter().any(|known| known.id == worker) {
            // Another session's worker is refused; one that no longer exists anywhere is already gone.
            return if sessions.iter().any(|other| {
                other
                    .inner
                    .family
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|known| known.id == worker)
            }) {
                Err(Error::new(
                    ErrorKind::Invalid,
                    "the fake worker is not this session's",
                ))
            } else {
                Ok(())
            };
        }
        let gone = descendants(&family, worker);
        let running: Vec<&String> = gone
            .iter()
            .filter(|id| {
                family.iter().any(|known| {
                    &known.id == *id
                        && matches!(known.state, WorkerState::Running | WorkerState::Waiting)
                })
            })
            .collect();
        if !running.is_empty() && !force {
            return Err(Error::new(ErrorKind::Busy, "the fake worker is running"));
        }
        self.shared
            .stopped_workers
            .lock()
            .unwrap()
            .extend(running.into_iter().cloned());
        let left: Vec<Worker> = family
            .into_iter()
            .filter(|known| !gone.contains(&known.id))
            .collect();
        self.shared.removed_workers.lock().unwrap().extend(gone);
        root.emit(Event::WorkersChanged { workers: left });
        Ok(())
    }

    async fn register_tools(&self, directory: &str, server: &ToolServer) -> Result<()> {
        self.record_tools(directory, Some(server.clone()))
    }

    async fn unregister_tools(&self, directory: &str, _name: &str) -> Result<()> {
        self.record_tools(directory, None)
    }

    async fn forget_instructions(&self, id: &str, keys: Vec<String>) -> Result<()> {
        if self.shared.no_instructions.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Unsupported, "no instructions"));
        }
        if let Some(session) = self
            .started()
            .into_iter()
            .find(|session| session.id() == id)
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
            // The most recently started first, as the most recently active.
            .rev()
            .filter(|session| std::path::Path::new(&session.inner.directory).starts_with(directory))
            .map(|session| SessionSummary {
                id: session.id(),
                title: session.inner.title.lock().unwrap().clone(),
                directory: session.inner.directory.clone(),
                busy: session.is_busy(),
                // The fake does not keep when its sessions were last active.
                updated: None,
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
            .find(|session| session.id() == id)
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
    /// The agent may name its session anew; see [`FakeSession::rename`].
    id: Mutex<String>,
    closing_failure: Arc<AtomicBool>,
    /// The session's own title, which [`Event::TitleChanged`] changes as the agent's would.
    title: Mutex<String>,
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
    /// The family of workers the session last reported, as the agent's own record.
    family: Mutex<Vec<Worker>>,
    modes: Mutex<Modes>,
    log: Mutex<FakeLog>,
}

/// `worker` and its own workers in `family`, at any depth, `worker` first.
fn descendants(family: &[Worker], worker: &str) -> Vec<String> {
    let mut gone = vec![worker.to_owned()];
    let mut index = 0;
    while let Some(parent) = gone.get(index).cloned() {
        gone.extend(
            family
                .iter()
                .filter(|known| known.parent.as_deref() == Some(parent.as_str()))
                .map(|known| known.id.clone()),
        );
        index += 1;
    }
    gone
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
            Event::WorkersChanged { workers } | Event::WorkersPartial { workers, .. } => {
                self.inner.family.lock().unwrap().clone_from(workers);
            }
            Event::TitleChanged { title } => {
                title.clone_into(&mut self.inner.title.lock().unwrap())
            }
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
        let message = format!("{}-reply-{number}", self.id());
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

    /// Change the family of workers the agent records for this session, without reporting it, as the agent changing
    /// behind what the space was told would.
    pub fn set_family(&self, workers: Vec<Worker>) {
        *self.inner.family.lock().unwrap() = workers;
    }

    /// Title the session anew, as its user may in the agent's own interface, reporting it as the agent would.
    pub fn retitle(&self, title: &str) {
        self.emit(Event::TitleChanged {
            title: title.to_owned(),
        });
    }

    /// Name the session anew, as an agent may once it starts.
    pub fn rename(&self, id: &str) {
        *self.inner.id.lock().unwrap() = id.to_owned();
    }

    pub fn is_busy(&self) -> bool {
        self.inner.busy.load(Ordering::SeqCst)
    }
}

impl Session for FakeSession {
    fn id(&self) -> String {
        self.inner.id.lock().unwrap().clone()
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
                Event::Closed { reason: None }
            },
            async {
                self.handle
                    .receiver
                    .recv()
                    .await
                    .expect("the handle holds a sender, so its events never end")
            },
        )
        .await;
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
            title: Some(self.inner.title.lock().unwrap().clone()),
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
            // Listening before looking, so that a release between the two is not missed.
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
        // Whatever was emitted and not yet received waits for the next binding.
        outbound
            .backlog
            .extend(std::iter::from_fn(|| self.handle.receiver.try_recv().ok()));
        outbound.current = None;
        *bound = None;
        if self.inner.closing_failure.load(Ordering::SeqCst) {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "the fake agent did not answer",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::future::block_on;

    fn spec() -> SessionSpec {
        SessionSpec {
            directory: "/tmp".to_owned(),
            title: "Fake".to_owned(),
        }
    }

    fn permission() -> Permission {
        Permission {
            id: "request-1".to_owned(),
            tool: "Bash".to_owned(),
            input: serde_json::Value::Null,
            description: None,
            options: Vec::new(),
        }
    }

    fn question() -> Question {
        Question {
            id: "question-1".to_owned(),
            title: "Which?".to_owned(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn a_closed_handle_does_nothing_more() {
        let backend = FakeBackend::new();
        let session = block_on(backend.create(spec())).unwrap();
        block_on(session.close()).unwrap();
        let closed = |result: Result<()>| {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Closed);
        };
        let prompt = Prompt {
            key: "key".to_owned(),
            text: "Hello".to_owned(),
        };
        closed(block_on(session.send(prompt)));
        closed(block_on(session.decide("request-1", Decision::AllowOnce)));
        closed(block_on(session.answer("question-1", Vec::new())));
        closed(block_on(session.set_mode("plan")));
        closed(block_on(session.set_instruction("tomori.space", None)));
        closed(block_on(session.interrupt()));
        // Its waiter hears it close, then nothing more.
        assert_eq!(
            block_on(session.next_event()).unwrap(),
            Event::Closed { reason: None }
        );
        assert!(block_on(session.next_event()).is_err());
        // Closing again is harmless.
        block_on(session.close()).unwrap();
    }

    #[test]
    fn requests_and_prompts_are_taken_once() {
        let backend = FakeBackend::new();
        let session = block_on(backend.create(spec())).unwrap();
        for _ in 0..2 {
            session.emit(Event::PermissionRequested(permission()));
            session.emit(Event::QuestionAsked(question()));
        }
        let snapshot = block_on(session.snapshot()).unwrap();
        assert_eq!(snapshot.permissions.len(), 1);
        assert_eq!(snapshot.questions.len(), 1);
        let prompt = Prompt {
            key: "key".to_owned(),
            text: "Hello".to_owned(),
        };
        block_on(session.send(prompt.clone())).unwrap();
        block_on(session.send(prompt)).unwrap();
        assert_eq!(session.log().prompts.len(), 1);
        // An empty delta says nothing.
        let message = session.start_message();
        while session.handle.receiver.try_recv().is_ok() {}
        session.delta(&message, "");
        assert!(session.handle.receiver.try_recv().is_err());
    }

    #[test]
    fn a_worker_is_removed_only_from_its_own_session_s_family() {
        let backend = FakeBackend::new();
        let root = block_on(backend.create(spec())).unwrap();
        let other = block_on(backend.create(spec())).unwrap();
        let worker = |id: &str, parent: &str, state| Worker {
            id: id.into(),
            parent: Some(parent.into()),
            title: id.into(),
            state,
        };
        root.emit(Event::WorkersChanged {
            workers: vec![
                worker("w1", &root.id(), WorkerState::Completed),
                worker("w2", "w1", WorkerState::Waiting),
            ],
        });
        let refused = |session: &str, worker: &str| {
            block_on(backend.delete_worker(session, worker, true))
                .unwrap_err()
                .kind
        };
        // A session bound to a space, or a session the backend does not have, is refused.
        assert_eq!(refused(&root.id(), &other.id()), ErrorKind::Invalid);
        assert_eq!(refused("fake-99", "w1"), ErrorKind::Invalid);
        // A worker waiting on its own worker is running.
        assert_eq!(
            block_on(backend.delete_worker(&root.id(), "w1", false))
                .unwrap_err()
                .kind,
            ErrorKind::Busy
        );
        backend.delete_nothing();
        assert_eq!(refused(&root.id(), "w1"), ErrorKind::Unsupported);
        assert!(backend.removed_workers().is_empty());
    }

    #[test]
    fn an_interrupt_ends_the_turn() {
        let backend = FakeBackend::manual();
        let session = block_on(backend.create(spec())).unwrap();
        session.emit(Event::Busy);
        block_on(session.interrupt()).unwrap();
        assert!(!session.is_busy());
        assert_eq!(session.log().interrupts, 1);
    }

    #[test]
    fn modes_and_instruction_keys_it_does_not_know_are_refused() {
        let backend = FakeBackend::new();
        let session = block_on(backend.create(spec())).unwrap();
        assert_eq!(
            block_on(session.set_mode("yolo")).unwrap_err().kind,
            ErrorKind::Invalid
        );
        assert_eq!(
            block_on(session.set_instruction("not a key", Some("x")))
                .unwrap_err()
                .kind,
            ErrorKind::Invalid
        );
    }

    #[test]
    fn instructions_are_forgotten_only_where_there_are_some_and_the_session_is_not_bound_again() {
        let backend = FakeBackend::new();
        let session = block_on(backend.create(spec())).unwrap();
        let id = session.id();
        let keys = || vec!["tomori.space".to_owned()];
        // Still bound, its entries are the binding's.
        block_on(backend.forget_instructions(&id, keys())).unwrap();
        // Unknown sessions are forgotten without entries to remove.
        block_on(backend.forget_instructions("fake-99", keys())).unwrap();
        assert_eq!(backend.forgotten(), [("fake-99".to_owned(), keys())]);
        backend.take_no_instructions();
        assert_eq!(
            block_on(backend.forget_instructions(&id, keys()))
                .unwrap_err()
                .kind,
            ErrorKind::Unsupported
        );
    }
}
