// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The OpenCode backend for the Tomori server.
//!
//! One backend binds to one running `opencode serve` process. It owns a
//! dedicated thread that holds the scheduler, HTTP client, driver, and the
//! server's one global event stream; sessions are `'static` handles that talk
//! to that thread over channels. When the service restarts or its event
//! stream ends, the thread finds the service again and reconnects, and re-reads
//! each attached session. Permissions and questions are answered only
//! when a participant acts. See `design/agent-backends.md`.
use std::sync::Mutex;

use tomori_backend::{
    Backend, Capabilities, Error, ErrorKind, Result, SessionSpec, SessionSummary, ToolServer,
};

mod activity;
mod api;
mod connection;
mod endpoint;
mod session;
mod system;
mod wire;

pub use session::OpenCodeSession;

/// The backend name, as used in create-space requests.
pub const NAME: &str = "opencode";

/// How to reach the OpenCode server.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// The explicit URL, from `--opencode-url`. `None` falls back to the
    /// `OPENCODE_URL` environment variable, the XDG service file, then the
    /// well-known default.
    pub url: Option<String>,
}

/// One connection to the OpenCode server, shared by every session it hosts.
struct Connection {
    commands: connection::Commands,
    /// The thread, kept alive for the connection's lifetime.
    #[allow(dead_code)]
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The OpenCode backend.
pub struct OpenCode {
    config: Config,
    /// `None` until the first use; `Some(Ok(_))` once the thread is running;
    /// `Some(Err(_))` once the backend has been shut down.
    connection: Mutex<Option<std::result::Result<Connection, Error>>>,
}

/// Why a request fails once the backend has been shut down.
const SHUT_DOWN: &str = "the OpenCode backend is shutting down";

impl OpenCode {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            connection: Mutex::new(None),
        }
    }

    /// The shared connection's commands channel, starting the thread on first
    /// use. The sender is cloned out so no borrow of the locked `Connection`
    /// escapes the guard.
    fn connection(&self) -> Result<connection::Commands> {
        let mut guard = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_mut() {
            Some(Ok(connection)) => Ok(connection.commands.clone()),
            Some(Err(error)) => Err(error.clone()),
            None => {
                // Resolved again before every connection attempt: a restarted
                // service may record another port and password.
                let mut resolver = endpoint::Resolver::new(self.config.url.clone());
                let resolve: connection::Resolve = Box::new(move || resolver.next());
                let (commands, receiver) = connection::Commands::new();
                // Refused for this request only: the next starts it again.
                let thread = system::spawn("opencode-connection", move || {
                    connection::run_thread(resolve, receiver);
                })
                .map_err(|error| {
                    Error::new(
                        ErrorKind::Unavailable,
                        format!("could not start the connection thread: {error}"),
                    )
                })?;
                let value = Connection {
                    commands: commands.clone(),
                    thread: Some(thread),
                };
                *guard = Some(Ok(value));
                Ok(commands)
            }
        }
    }
}

impl Backend for OpenCode {
    type Session = OpenCodeSession;

    fn name(&self) -> &str {
        NAME
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            attach: true,
            interrupt: true,
            questions: true,
            workers: true,
            instructions: true,
            delete: true,
            tools: true,
        }
    }

    async fn sessions(&self, directory: &str) -> Result<Vec<SessionSummary>> {
        let commands = self.connection()?;
        let directory = directory.to_owned();
        commands
            .reply_with(move |reply| connection::Command::Sessions { directory, reply })
            .await
    }

    async fn create(&self, spec: SessionSpec) -> Result<OpenCodeSession> {
        let commands = self.connection()?;
        let (id, events) = commands
            .reply_with(move |reply| connection::Command::Create { spec, reply })
            .await?;
        Ok(OpenCodeSession::new(id, events, commands))
    }

    async fn forget_instructions(&self, id: &str, keys: Vec<String>) -> Result<()> {
        if let Some(key) = keys
            .iter()
            .find(|key| !tomori_backend::valid_instruction_key(key))
        {
            return Err(Error::new(
                ErrorKind::Invalid,
                format!("{key:?} cannot name an instruction entry"),
            ));
        }
        let commands = self.connection()?;
        let id = id.to_owned();
        commands
            .reply_with(move |reply| connection::Command::ForgetInstructions { id, keys, reply })
            .await
    }

    async fn delete_session(&self, id: &str) -> Result<()> {
        let commands = self.connection()?;
        let id = id.to_owned();
        commands
            .reply_with(move |reply| connection::Command::DeleteSession { id, reply })
            .await
    }

    async fn delete_worker(&self, session: &str, worker: &str, force: bool) -> Result<()> {
        let commands = self.connection()?;
        let session = session.to_owned();
        let worker = worker.to_owned();
        commands
            .reply_with(move |reply| connection::Command::DeleteWorker {
                session,
                worker,
                force,
                reply,
            })
            .await
    }

    async fn register_tools(&self, directory: &str, server: &ToolServer) -> Result<()> {
        let commands = self.connection()?;
        let directory = directory.to_owned();
        let server = server.clone();
        commands
            .reply_with(move |reply| connection::Command::RegisterTools {
                directory,
                server,
                reply,
            })
            .await
    }

    async fn unregister_tools(&self, directory: &str, name: &str) -> Result<()> {
        let commands = self.connection()?;
        let directory = directory.to_owned();
        let name = name.to_owned();
        commands
            .reply_with(move |reply| connection::Command::UnregisterTools {
                directory,
                name,
                reply,
            })
            .await
    }

    fn shutdown(&self) {
        let mut guard = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref() {
            Some(Ok(connection)) => connection.commands.shutdown(),
            // Never start a connection once shutting down.
            _ => *guard = Some(Err(Error::new(ErrorKind::Closed, SHUT_DOWN))),
        }
    }

    async fn attach(&self, id: &str, directory: &str) -> Result<OpenCodeSession> {
        let commands = self.connection()?;
        let id = id.to_owned();
        let directory = directory.to_owned();
        let attach_id = id.clone();
        let events = commands
            .reply_with(move |reply| connection::Command::Attach {
                id: attach_id,
                directory,
                reply,
            })
            .await?;
        Ok(OpenCodeSession::new(id, events, commands))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_thread_that_cannot_start_refuses_one_request_and_starts_for_the_next() {
        let backend = OpenCode::new(Config {
            url: Some("http://127.0.0.1:9".into()),
        });
        system::refuse("opencode-connection");
        let Err(error) = backend.connection() else {
            unreachable!("a connection started without its thread");
        };
        assert_eq!(error.kind, ErrorKind::Unavailable);
        assert_eq!(
            error.message,
            "could not start the connection thread: the system refused to start opencode-connection"
        );
        assert!(backend.connection().is_ok());
        backend.shutdown();
    }
}
