// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_backend::{
    AnswerValue, Backend, Capabilities, Decision, Error, ErrorKind, Event, Prompt, Result, Role,
    Session, SessionSpec, SessionSummary, Snapshot,
};

/// The placeholder backend's name, which `--backend` and clients name it by.
pub const NAME: &str = "placeholder";

/// A backend with no agent behind it: each session replies to every prompt by repeating it, which is enough to see a space work end to end.
pub struct Placeholder;

/// A session of [`Placeholder`], whose events are its replies.
pub struct PlaceholderSession {
    id: String,
    sender: async_channel::Sender<Event>,
    receiver: async_channel::Receiver<Event>,
}

impl Backend for Placeholder {
    type Session = PlaceholderSession;

    fn name(&self) -> &str {
        NAME
    }

    /// It takes instructions, so that a space shows its environment's layer, and nothing else.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            instructions: true,
            ..Capabilities::default()
        }
    }

    async fn sessions(&self, _directory: &str) -> Result<Vec<SessionSummary>> {
        Ok(Vec::new())
    }

    async fn create(&self, spec: SessionSpec) -> Result<PlaceholderSession> {
        let (sender, receiver) = async_channel::unbounded();
        Ok(PlaceholderSession {
            id: format!("placeholder:{}", spec.title),
            sender,
            receiver,
        })
    }

    async fn attach(&self, _id: &str, _directory: &str) -> Result<PlaceholderSession> {
        Err(unsupported())
    }

    /// It keeps no instructions, so there are none to remove.
    async fn forget_instructions(&self, _id: &str, _keys: Vec<String>) -> Result<()> {
        Ok(())
    }
}

/// Refuse what the placeholder backend does not do.
fn unsupported() -> Error {
    Error::new(
        ErrorKind::Unsupported,
        "the placeholder backend does not do that",
    )
}

impl Session for PlaceholderSession {
    fn id(&self) -> String {
        self.id.clone()
    }

    async fn next_event(&self) -> Result<Event> {
        self.receiver
            .recv()
            .await
            .map_err(|_| Error::new(ErrorKind::Closed, "the placeholder session closed"))
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            busy: Some(false),
            ..Snapshot::default()
        })
    }

    async fn send(&self, prompt: Prompt) -> Result<()> {
        let message = format!("reply-{}", prompt.key);
        for event in [
            Event::Delivered { key: prompt.key },
            Event::Busy,
            Event::MessageStarted {
                message: message.clone(),
                role: Role::Assistant,
            },
            Event::TextDelta {
                message: message.clone(),
                text: format!("You said: {}", prompt.text),
            },
            Event::MessageCompleted { message },
            Event::Idle,
        ] {
            self.sender
                .try_send(event)
                .map_err(|_| Error::new(ErrorKind::Closed, "the placeholder session closed"))?;
        }
        Ok(())
    }

    async fn decide(&self, _permission: &str, _decision: Decision) -> Result<()> {
        Err(unsupported())
    }

    async fn answer(&self, _question: &str, _answers: Vec<(String, AnswerValue)>) -> Result<()> {
        Err(unsupported())
    }

    async fn interrupt(&self) -> Result<()> {
        Err(unsupported())
    }

    async fn set_mode(&self, _mode: &str) -> Result<()> {
        Err(unsupported())
    }

    /// The agent is told nothing, since there is none.
    async fn set_instruction(&self, _key: &str, _value: Option<&str>) -> Result<()> {
        Ok(())
    }

    /// Report [`Event::Closed`] once, then end.
    async fn close(&self) -> Result<()> {
        let _ = self.sender.try_send(Event::Closed { reason: None });
        self.sender.close();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;

    #[test]
    fn a_session_repeats_each_prompt_and_refuses_the_rest() {
        block_on(async {
            let backend = Placeholder;
            assert_eq!(backend.name(), NAME);
            assert!(backend.capabilities().instructions);
            assert!(!backend.capabilities().attach);
            assert_eq!(backend.sessions("/srv").await, Ok(Vec::new()));
            assert_eq!(
                backend.attach("placeholder:Notes", "/srv").await.err(),
                Some(unsupported())
            );
            assert_eq!(
                backend
                    .forget_instructions("placeholder:Notes", vec!["tomori".into()])
                    .await,
                Ok(())
            );

            let session = backend
                .create(SessionSpec {
                    directory: "/srv".into(),
                    title: "Notes".into(),
                })
                .await
                .unwrap();
            assert_eq!(session.id(), "placeholder:Notes");
            assert_eq!(session.snapshot().await.unwrap().busy, Some(false));
            assert_eq!(session.set_instruction("tomori", Some("Hi.")).await, Ok(()));
            session
                .send(Prompt {
                    key: "1".into(),
                    text: "Hello".into(),
                })
                .await
                .unwrap();
            let mut events = Vec::new();
            for _ in 0..6 {
                events.push(session.next_event().await.unwrap());
            }
            assert_eq!(events[0], Event::Delivered { key: "1".into() });
            assert_eq!(
                events[3],
                Event::TextDelta {
                    message: "reply-1".into(),
                    text: "You said: Hello".into()
                }
            );
            assert_eq!(events[5], Event::Idle);

            assert_eq!(
                session.decide("permission", Decision::Deny).await,
                Err(unsupported())
            );
            assert_eq!(
                session.answer("question", Vec::new()).await,
                Err(unsupported())
            );
            assert_eq!(session.interrupt().await, Err(unsupported()));
            assert_eq!(session.set_mode("plan").await, Err(unsupported()));

            // Closed once, then the session ends, and refuses prompts.
            session.close().await.unwrap();
            assert_eq!(
                session.next_event().await,
                Ok(Event::Closed { reason: None })
            );
            assert_eq!(
                session.next_event().await.unwrap_err().kind,
                ErrorKind::Closed
            );
            let prompt = Prompt {
                key: "2".into(),
                text: "Still there?".into(),
            };
            assert_eq!(
                session.send(prompt).await.unwrap_err().kind,
                ErrorKind::Closed
            );
        });
    }
}
