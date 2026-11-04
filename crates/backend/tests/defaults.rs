// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a backend gets from the trait's default methods when it implements only the required ones, as a backend outside this repository might.

use std::future::{Future, Ready, ready};
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use tomori_backend::{
    AnswerValue, Backend, Capabilities, Decision, Error, ErrorKind, Event, Prompt, Result, Session,
    SessionSpec, SessionSummary, Snapshot,
};

/// The result of a future that is ready at once, as every future here is.
fn now<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => output,
        Poll::Pending => unreachable!("every future here is ready at once"),
    }
}

/// A backend that implements only the required methods, refusing each.
struct Minimal;

fn refused<T>() -> Ready<Result<T>> {
    ready(Err(Error::new(ErrorKind::Unsupported, "minimal")))
}

impl Backend for Minimal {
    type Session = Minimal;

    fn name(&self) -> &str {
        "minimal"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn sessions(&self, _: &str) -> impl Future<Output = Result<Vec<SessionSummary>>> + Send {
        refused()
    }

    fn create(&self, _: SessionSpec) -> impl Future<Output = Result<Minimal>> + Send {
        refused()
    }

    fn attach(&self, _: &str, _: &str) -> impl Future<Output = Result<Minimal>> + Send {
        refused()
    }
}

impl Session for Minimal {
    fn id(&self) -> String {
        "minimal".to_owned()
    }

    fn next_event(&self) -> impl Future<Output = Result<Event>> + Send {
        refused()
    }

    fn snapshot(&self) -> impl Future<Output = Result<Snapshot>> + Send {
        refused()
    }

    fn send(&self, _: Prompt) -> impl Future<Output = Result<()>> + Send {
        refused()
    }

    fn decide(&self, _: &str, _: Decision) -> impl Future<Output = Result<()>> + Send {
        refused()
    }

    fn answer(
        &self,
        _: &str,
        _: Vec<(String, AnswerValue)>,
    ) -> impl Future<Output = Result<()>> + Send {
        refused()
    }

    fn interrupt(&self) -> impl Future<Output = Result<()>> + Send {
        refused()
    }

    fn set_mode(&self, _: &str) -> impl Future<Output = Result<()>> + Send {
        refused()
    }

    fn close(&self) -> impl Future<Output = Result<()>> + Send {
        refused()
    }
}

#[test]
fn a_backend_without_instructions_refuses_them_as_unsupported() {
    let backend = Minimal;
    assert!(!backend.capabilities().instructions);
    let unsupported = Error::new(
        ErrorKind::Unsupported,
        "the backend does not take instructions",
    );
    assert_eq!(
        now(backend.forget_instructions("session", vec!["tomori".into()])),
        Err(unsupported.clone())
    );
    assert_eq!(
        now(Minimal.set_instruction("tomori", Some("Speak briefly."))),
        Err(unsupported.clone())
    );
    assert_eq!(
        now(Minimal.set_instruction("tomori", None)),
        Err(unsupported.clone())
    );
    assert_eq!(
        unsupported.to_string(),
        "Unsupported: the backend does not take instructions"
    );
    // Shutting down needs nothing from a backend that holds no connection.
    backend.shutdown();
}

#[test]
fn the_minimal_backend_refuses_everything_it_implements() {
    let backend = Minimal;
    assert_eq!(backend.name(), "minimal");
    assert_eq!(backend.id(), "minimal");
    let spec = SessionSpec {
        directory: "/tmp".into(),
        title: "Tomori".into(),
    };
    let prompt = Prompt {
        key: "1".into(),
        text: "Hello.".into(),
    };
    let kinds = [
        now(backend.sessions("/tmp")).err(),
        now(backend.create(spec)).err(),
        now(backend.attach("session", "/tmp")).err(),
        now(backend.next_event()).err(),
        now(backend.snapshot()).err(),
        now(backend.send(prompt)).err(),
        now(backend.decide("permission", Decision::Deny)).err(),
        now(backend.answer("question", Vec::new())).err(),
        now(backend.interrupt()).err(),
        now(backend.set_mode("plan")).err(),
        now(backend.close()).err(),
    ]
    .map(|error| error.map(|error| error.kind));
    assert_eq!(kinds, [Some(ErrorKind::Unsupported); 11]);
}
