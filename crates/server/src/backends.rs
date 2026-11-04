// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The set of agent backends this server can bind spaces to.
//!
//! A backend is any [`Backend`], from this repository, such as OpenCode's, or from another crate: [`Backends::with`] holds each behind one type, [`AnyBackend`], whose sessions are [`AnySession`]s. Nothing else in the server names a concrete backend.

use std::future::Future;
use std::pin::Pin;

use tomori_backend::{
    AnswerValue, Backend, Capabilities, Decision, Event, Prompt, Result, Session, SessionSpec,
    SessionSummary, Snapshot,
};
use tomori_protocol::BackendInfo;

/// A backend's or session's future, boxed so that backends of different types can be held alike.
type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// [`Backend`], with its futures boxed and its sessions held as [`AnySession`], so that it can be a trait object.
trait DynamicBackend: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn sessions<'a>(&'a self, directory: &'a str) -> Boxed<'a, Vec<SessionSummary>>;
    fn create(&self, spec: SessionSpec) -> Boxed<'_, AnySession>;
    fn attach<'a>(&'a self, id: &'a str, directory: &'a str) -> Boxed<'a, AnySession>;
    fn forget_instructions<'a>(&'a self, id: &'a str, keys: Vec<String>) -> Boxed<'a, ()>;
    fn shutdown(&self);
}

impl<B: Backend> DynamicBackend for B {
    fn name(&self) -> &str {
        Backend::name(self)
    }

    fn capabilities(&self) -> Capabilities {
        Backend::capabilities(self)
    }

    fn sessions<'a>(&'a self, directory: &'a str) -> Boxed<'a, Vec<SessionSummary>> {
        Box::pin(Backend::sessions(self, directory))
    }

    fn create(&self, spec: SessionSpec) -> Boxed<'_, AnySession> {
        let created = Backend::create(self, spec);
        Box::pin(async move { created.await.map(AnySession::new) })
    }

    fn attach<'a>(&'a self, id: &'a str, directory: &'a str) -> Boxed<'a, AnySession> {
        let attached = Backend::attach(self, id, directory);
        Box::pin(async move { attached.await.map(AnySession::new) })
    }

    fn forget_instructions<'a>(&'a self, id: &'a str, keys: Vec<String>) -> Boxed<'a, ()> {
        Box::pin(Backend::forget_instructions(self, id, keys))
    }

    fn shutdown(&self) {
        Backend::shutdown(self)
    }
}

/// [`Session`], with its futures boxed, so that it can be a trait object.
trait DynamicSession: Send + Sync {
    fn id(&self) -> String;
    fn next_event(&self) -> Boxed<'_, Event>;
    fn snapshot(&self) -> Boxed<'_, Snapshot>;
    fn send(&self, prompt: Prompt) -> Boxed<'_, ()>;
    fn decide<'a>(&'a self, permission: &'a str, decision: Decision) -> Boxed<'a, ()>;
    fn answer<'a>(
        &'a self,
        question: &'a str,
        answers: Vec<(String, AnswerValue)>,
    ) -> Boxed<'a, ()>;
    fn set_mode<'a>(&'a self, mode: &'a str) -> Boxed<'a, ()>;
    fn set_instruction<'a>(&'a self, key: &'a str, value: Option<&'a str>) -> Boxed<'a, ()>;
    fn interrupt(&self) -> Boxed<'_, ()>;
    fn close(&self) -> Boxed<'_, ()>;
}

impl<S: Session> DynamicSession for S {
    fn id(&self) -> String {
        Session::id(self)
    }

    fn next_event(&self) -> Boxed<'_, Event> {
        Box::pin(Session::next_event(self))
    }

    fn snapshot(&self) -> Boxed<'_, Snapshot> {
        Box::pin(Session::snapshot(self))
    }

    fn send(&self, prompt: Prompt) -> Boxed<'_, ()> {
        Box::pin(Session::send(self, prompt))
    }

    fn decide<'a>(&'a self, permission: &'a str, decision: Decision) -> Boxed<'a, ()> {
        Box::pin(Session::decide(self, permission, decision))
    }

    fn answer<'a>(
        &'a self,
        question: &'a str,
        answers: Vec<(String, AnswerValue)>,
    ) -> Boxed<'a, ()> {
        Box::pin(Session::answer(self, question, answers))
    }

    fn set_mode<'a>(&'a self, mode: &'a str) -> Boxed<'a, ()> {
        Box::pin(Session::set_mode(self, mode))
    }

    fn set_instruction<'a>(&'a self, key: &'a str, value: Option<&'a str>) -> Boxed<'a, ()> {
        Box::pin(Session::set_instruction(self, key, value))
    }

    fn interrupt(&self) -> Boxed<'_, ()> {
        Box::pin(Session::interrupt(self))
    }

    fn close(&self) -> Boxed<'_, ()> {
        Box::pin(Session::close(self))
    }
}

/// One of the backends this server offers, of whatever type.
pub struct AnyBackend(Box<dyn DynamicBackend>);

/// A session of one of the backends this server offers.
pub struct AnySession(Box<dyn DynamicSession>);

impl AnyBackend {
    /// Hold `backend` as any other.
    pub(crate) fn new(backend: impl Backend + 'static) -> Self {
        Self(Box::new(backend))
    }
}

impl AnySession {
    fn new(session: impl Session) -> Self {
        Self(Box::new(session))
    }
}

impl Backend for AnyBackend {
    type Session = AnySession;

    fn name(&self) -> &str {
        self.0.name()
    }

    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }

    async fn sessions(&self, directory: &str) -> Result<Vec<SessionSummary>> {
        self.0.sessions(directory).await
    }

    async fn create(&self, spec: SessionSpec) -> Result<AnySession> {
        self.0.create(spec).await
    }

    async fn attach(&self, id: &str, directory: &str) -> Result<AnySession> {
        self.0.attach(id, directory).await
    }

    async fn forget_instructions(&self, id: &str, keys: Vec<String>) -> Result<()> {
        self.0.forget_instructions(id, keys).await
    }

    fn shutdown(&self) {
        self.0.shutdown()
    }
}

impl Session for AnySession {
    fn id(&self) -> String {
        self.0.id()
    }

    async fn next_event(&self) -> Result<Event> {
        self.0.next_event().await
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        self.0.snapshot().await
    }

    async fn send(&self, prompt: Prompt) -> Result<()> {
        self.0.send(prompt).await
    }

    async fn decide(&self, permission: &str, decision: Decision) -> Result<()> {
        self.0.decide(permission, decision).await
    }

    async fn answer(&self, question: &str, answers: Vec<(String, AnswerValue)>) -> Result<()> {
        self.0.answer(question, answers).await
    }

    async fn set_mode(&self, mode: &str) -> Result<()> {
        self.0.set_mode(mode).await
    }

    async fn set_instruction(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.0.set_instruction(key, value).await
    }

    async fn interrupt(&self) -> Result<()> {
        self.0.interrupt().await
    }

    async fn close(&self) -> Result<()> {
        self.0.close().await
    }
}

/// The backends a server offers, looked up by [`Backend::name`].
#[derive(Default)]
pub struct Backends {
    backends: Vec<AnyBackend>,
}

impl Backends {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `backend`, of any type. A later backend with the same name is never selected.
    pub fn with(self, backend: impl Backend + 'static) -> Self {
        self.with_any(AnyBackend::new(backend))
    }

    /// Add a backend already held as any other.
    pub(crate) fn with_any(mut self, backend: AnyBackend) -> Self {
        self.backends.push(backend);
        self
    }

    pub fn get(&self, name: &str) -> Option<&AnyBackend> {
        self.backends
            .iter()
            .find(|backend| Backend::name(*backend) == name)
    }

    /// Tell every backend that the server is shutting down; see [`Backend::shutdown`].
    pub fn shutdown(&self) {
        for backend in &self.backends {
            Backend::shutdown(backend);
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.backends.iter().map(Backend::name)
    }

    /// The backends, for a client to show: their name and whether existing sessions can be attached.
    pub fn info(&self) -> Vec<BackendInfo> {
        self.backends
            .iter()
            .map(|backend| BackendInfo {
                name: Backend::name(backend).to_owned(),
                attach: Backend::capabilities(backend).attach,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;
    use crate::fake::FakeBackend;

    #[test]
    fn a_backend_of_any_type_is_driven_through_its_sessions() {
        let fake = FakeBackend::new();
        let backends = Backends::new().with(fake.clone());
        let backend = backends.get("fake").expect("offered by its name");
        assert_eq!(backends.names().collect::<Vec<_>>(), ["fake"]);
        block_on(async {
            // Named in full, since every backend is also a trait object here.
            let spec = SessionSpec {
                directory: "/srv".into(),
                title: "Notes".into(),
            };
            let session = Backend::create(backend, spec).await.unwrap();
            Session::interrupt(&session).await.unwrap();
            assert_eq!(Session::next_event(&session).await.unwrap(), Event::Idle);
            assert_eq!(fake.started()[0].log().interrupts, 1);
        });
    }
}
