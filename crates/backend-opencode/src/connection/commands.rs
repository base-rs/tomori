// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The commands the backend and its sessions send the connection's thread, and the channel they travel on.

use super::*;

/// One command for the thread, carrying the channel to answer it on.
///
/// Every reply channel carries a `tomori_backend::Error`, not an `api::Failure`: the
/// thread maps failures to the neutral error before answering.
pub enum Command {
    Sessions {
        directory: String,
        reply: async_channel::Sender<tomori_backend::Result<Vec<SessionSummary>>>,
    },
    Create {
        spec: SessionSpec,
        reply: async_channel::Sender<tomori_backend::Result<(String, RegistrationEvents)>>,
    },
    Attach {
        id: String,
        directory: String,
        reply: async_channel::Sender<tomori_backend::Result<RegistrationEvents>>,
    },
    Snapshot {
        id: String,
        identity: Arc<AtomicU64>,
        reply: async_channel::Sender<tomori_backend::Result<SnapshotReply>>,
    },
    Prompt {
        id: String,
        identity: Arc<AtomicU64>,
        key: String,
        text: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    Interrupt {
        id: String,
        identity: Arc<AtomicU64>,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    Decide {
        id: String,
        identity: Arc<AtomicU64>,
        request: String,
        decision: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    Answer {
        id: String,
        identity: Arc<AtomicU64>,
        form: String,
        answer: Value,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Choose the agent for `id`'s later prompts.
    SetMode {
        id: String,
        identity: Arc<AtomicU64>,
        mode: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Set `id`'s instruction entry `key` to `value`, or remove it when `value` is `None`.
    Instruction {
        id: String,
        identity: Arc<AtomicU64>,
        key: String,
        value: Option<String>,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Remove the instruction entries `keys` from the session `id`, unless it is attached when this is performed:
    /// then a space was bound to it again, and the entries are that space's.
    ForgetInstructions {
        id: String,
        keys: Vec<String>,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Stop any turn the session `id` is running and delete it, with its child sessions, unless it is attached when
    /// this is performed: then a space was bound to it again, and it is that space's.
    DeleteSession {
        id: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Delete the worker session `worker` of the session `session`, with its own workers, once OpenCode's records say
    /// that it descends from `session` through no session a space is bound to, and, unless `force`, that neither it nor
    /// any of its own workers known is running; with `force`, those are stopped first.
    DeleteWorker {
        session: String,
        worker: String,
        force: bool,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Register Tomori's MCP server for `directory`, replacing one of its name.
    RegisterTools {
        directory: String,
        server: tomori_backend::ToolServer,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// The directory the session `id` is in now, as its record says.
    Locate {
        id: String,
        reply: async_channel::Sender<tomori_backend::Result<Option<String>>>,
    },
    /// Remove the MCP server `name` from `directory`; one OpenCode does not have is already removed.
    UnregisterTools {
        directory: String,
        name: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Close `id`'s registration without deleting its OpenCode session.
    Close {
        id: String,
        identity: Arc<AtomicU64>,
    },
    /// The backend is shutting down: fail every waiting and later request,
    /// and close every registration, without waiting for OpenCode.
    Shutdown,
}

/// A snapshot's worker epoch becomes authoritative only when its recipient
/// receives it; an abandoned reply must not invalidate queued worker events.
pub(crate) struct SnapshotReply {
    pub(super) snapshot: Snapshot,
    pub(super) epoch: u64,
    pub(super) watermark: Arc<AtomicU64>,
    pub(super) worker_incomplete: Arc<AtomicBool>,
    pub(super) ordinary_incomplete: Arc<AtomicBool>,
}

impl SnapshotReply {
    pub(crate) fn accept(self) -> tomori_backend::Result<Snapshot> {
        if self.ordinary_incomplete.load(Ordering::Acquire) {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "Session events were lost while reading the initial state.",
            ));
        }
        if self.worker_incomplete.load(Ordering::Acquire) {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "Worker observation exceeds the supported limit.",
            ));
        }
        let previous = self.watermark.fetch_max(self.epoch, Ordering::AcqRel);
        if self.epoch < previous {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "A newer session snapshot was already accepted.",
            ));
        }
        Ok(self.snapshot)
    }
}

/// The connection thread's end of the commands channel.
pub struct CommandReceiver {
    pub(super) rx: async_channel::Receiver<Command>,
    stopping: Arc<AtomicBool>,
    /// Never carries a message; closed when the backend shuts down.
    stop: async_channel::Receiver<()>,
}

impl CommandReceiver {
    /// The next command. Once the backend is shutting down this is
    /// [`Command::Shutdown`], even if a full channel kept that command out.
    pub(super) async fn recv(&self) -> std::result::Result<Command, async_channel::RecvError> {
        if self.stopping.load(Ordering::Acquire) {
            return Ok(Command::Shutdown);
        }
        self.rx.recv().await
    }

    pub(super) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    /// Finishes once the backend is shutting down, so that a request in
    /// flight does not hold up shutdown.
    pub(super) async fn stopped(&self) {
        let _ = self.stop.recv().await;
    }
}

/// The commands channel, shared by the backend and every session handle.
#[derive(Clone)]
pub struct Commands {
    tx: async_channel::Sender<Command>,
    /// Set once the backend is shutting down; later requests fail at once.
    stopping: Arc<AtomicBool>,
    /// Closed once the backend is shutting down.
    stop: async_channel::Sender<()>,
}

impl Commands {
    pub fn new() -> (Self, CommandReceiver) {
        let (tx, rx) = async_channel::bounded(256);
        let stopping = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = async_channel::bounded(1);
        (
            Self {
                tx,
                stopping: stopping.clone(),
                stop,
            },
            CommandReceiver {
                rx,
                stopping,
                stop: stopped,
            },
        )
    }

    /// Stop waiting for OpenCode: requests waiting for a connection, or for
    /// one in flight, fail at once, and so do later ones. Never blocks.
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        self.stop.close();
        let _ = self.tx.try_send(Command::Shutdown);
    }

    /// The raw sender, including nonblocking registration cleanup.
    pub fn tx(&self) -> &async_channel::Sender<Command> {
        &self.tx
    }

    /// Send a command built against a fresh reply channel, and wait for it.
    /// The reply channel carries the neutral `Result`, so this returns the
    /// command's own outcome, not a double wrap.
    pub async fn reply_with<T: Send + 'static>(
        &self,
        build: impl FnOnce(
            async_channel::Sender<std::result::Result<T, tomori_backend::Error>>,
        ) -> Command,
    ) -> std::result::Result<T, tomori_backend::Error> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                SHUTTING_DOWN,
            ));
        }
        let (reply, waiting) = async_channel::bounded(1);
        let command = build(reply);
        self.tx.send(command).await.map_err(|_| {
            tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "the connection has closed",
            )
        })?;
        waiting.recv().await.map_err(|_| {
            tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "the connection has closed",
            )
        })?
    }
}
