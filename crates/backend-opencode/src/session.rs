// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! One live OpenCode session: a `'static` handle over the connection thread.
//!
//! The handle owns one end of the session's event channel and shares the
//! connection's commands channel. [`next_event`] reads the channel; the other
//! methods send a command and wait for its reply.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tomori_backend::{AnswerValue, Decision, ErrorKind, Event, Prompt, Result, Session, Snapshot};

use crate::connection::{Command, Commands, RegistrationEvents};
use crate::wire;

/// One session bound to the connection thread.
pub struct OpenCodeSession {
    inner: Arc<Inner>,
}

struct Inner {
    id: String,
    events: RegistrationEvents,
    commands: Commands,
    closed: AtomicBool,
}

impl OpenCodeSession {
    /// Build the handle from the thread's reply to a create or attach command.
    pub(crate) fn new(id: String, events: RegistrationEvents, commands: Commands) -> Self {
        Self {
            inner: Arc::new(Inner {
                id,
                events,
                commands,
                closed: AtomicBool::new(false),
            }),
        }
    }
}

impl Session for OpenCodeSession {
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    fn next_event(&self) -> impl std::future::Future<Output = Result<Event>> + Send {
        let inner = Arc::clone(&self.inner);
        async move {
            loop {
                if inner.closed.load(Ordering::Acquire) {
                    return Err(tomori_backend::Error::new(
                        ErrorKind::Closed,
                        "the session has closed",
                    ));
                }
                let queued = inner.events.recv().await.map_err(|error| {
                    let kind = if inner.closed.load(Ordering::Acquire) {
                        ErrorKind::Closed
                    } else {
                        ErrorKind::Unavailable
                    };
                    tomori_backend::Error::new(kind, error.to_string())
                })?;
                let Some(event) = queued.into_current() else {
                    continue;
                };
                if matches!(event, Event::Closed { .. }) {
                    inner.closed.store(true, Ordering::Release);
                }
                return Ok(event);
            }
        }
    }

    fn snapshot(&self) -> impl std::future::Future<Output = Result<Snapshot>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        async move {
            commands
                .reply_with(move |reply| Command::Snapshot {
                    id,
                    identity,
                    reply,
                })
                .await
                .and_then(|snapshot| snapshot.accept())
        }
    }

    fn send(&self, prompt: Prompt) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        async move {
            commands
                .reply_with(move |reply| Command::Prompt {
                    id,
                    identity,
                    key: prompt.key,
                    text: prompt.text,
                    reply,
                })
                .await
        }
    }

    fn decide(
        &self,
        permission: &str,
        decision: Decision,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        async move {
            // OpenCode's reply is a flat `once` / `always` / `reject`; the
            // session's single standing grant is what `always` applies.
            let decision = match decision {
                Decision::AllowOnce => "once",
                Decision::Grant { .. } => "always",
                Decision::Deny => "reject",
            };
            let decision = decision.to_owned();
            commands
                .reply_with(move |reply| Command::Decide {
                    id,
                    identity,
                    request: permission.to_owned(),
                    decision,
                    reply,
                })
                .await
        }
    }

    fn answer(
        &self,
        question: &str,
        answers: Vec<(String, AnswerValue)>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        async move {
            let answer = wire::form_answer(&answers);
            commands
                .reply_with(move |reply| Command::Answer {
                    id,
                    identity,
                    form: question.to_owned(),
                    answer,
                    reply,
                })
                .await
        }
    }

    fn set_mode(&self, mode: &str) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        let mode = mode.to_owned();
        async move {
            commands
                .reply_with(move |reply| Command::SetMode {
                    id,
                    identity,
                    mode,
                    reply,
                })
                .await
        }
    }

    fn set_instruction(
        &self,
        key: &str,
        value: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        let key = key.to_owned();
        let value = value.map(str::to_owned);
        async move {
            if !tomori_backend::valid_instruction_key(&key) {
                return Err(tomori_backend::Error::new(
                    ErrorKind::Invalid,
                    format!("{key:?} cannot name an instruction entry"),
                ));
            }
            commands
                .reply_with(move |reply| Command::Instruction {
                    id,
                    identity,
                    key,
                    value,
                    reply,
                })
                .await
        }
    }

    fn interrupt(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        async move {
            commands
                .reply_with(move |reply| Command::Interrupt {
                    id,
                    identity,
                    reply,
                })
                .await
        }
    }

    fn close(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        let id = self.inner.id.clone();
        let identity = self.inner.events.identity.clone();
        let commands = self.inner.commands.clone();
        let inner = Arc::clone(&self.inner);
        async move {
            inner.closed.store(true, Ordering::Release);
            // The registration is removed in command order. Never let a
            // stalled shared dispatcher hold this space's cleanup forever.
            commands
                .tx()
                .try_send(Command::Close { id, identity })
                .map_err(|_| {
                    tomori_backend::Error::new(
                        ErrorKind::Unavailable,
                        "the connection could not close the registration",
                    )
                })
        }
    }
}
