// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Devices and connections for tests: a microphone and a speaker without hardware.

use crate::{
    capture::{Capture, Failed, Recording, Samples},
    connection::Connection,
    conversation::Conversation,
    identity::{Identity, Overrides, Store},
    playback::{self, Output, Playback, Route, Shared},
};

/// Evaluate every log line's arguments, from now on in this test binary, so that what each would say is exercised; nothing is written, as no logger is installed.
pub fn log_everything() {
    log::set_max_level(log::LevelFilter::Trace);
}

/// A microphone that opens at 16 kHz and hears nothing.
pub struct Silent;

impl Capture for Silent {
    fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
        Ok(Recording::new(16_000, ()))
    }
}

/// A speaker that opens and plays nowhere.
pub struct Nowhere;

impl Playback for Nowhere {
    fn open(
        &self,
        _player: Shared,
        _failed: playback::Failed,
        _route: Route,
    ) -> Result<Output, String> {
        Ok(Output::new(()))
    }
}

/// A participant named `name`, keeping nothing, connected to `server` if one is given, with [`Silent`] and [`Nowhere`] for its devices.
pub fn participant(name: &str, server: Option<String>) -> (Conversation, Connection) {
    let store = Store::new(None);
    let identity = Identity::new(
        &store,
        None,
        Overrides {
            server,
            name: Some(name.to_owned()),
            token: None,
        },
    );
    let mut conversation = Conversation::new(&identity.name);
    let connection = Connection::new(
        identity,
        store,
        Box::new(()),
        Box::new(Silent),
        Box::new(Nowhere),
        &mut conversation,
    );
    (conversation, connection)
}

/// A server on a loopback port that serves only TLS and admits by tokens, and anyone else as a visitor, with scripted agent sessions.
pub struct Server {
    pub server: tomori_server::test_support::TestServer,
    pub pin: tomori_security::Fingerprint,
    /// Where the operator confirms a device's pairing code, on a server that pairs.
    pub pairing: Option<tomori_server::Pairing>,
}

impl Server {
    pub fn start() -> Self {
        Self::with(true, false)
    }

    /// A server that admits visitors if `visitors`, and pairs devices as owners if `pairing`.
    pub fn with(visitors: bool, pairing: bool) -> Self {
        let certificate = tomori_security::ServerCertificate::generate();
        let pin = certificate.fingerprint();
        let config = certificate
            .server_config()
            .expect("the certificate is usable");
        let (sender, handle) = std::sync::mpsc::channel();
        let server =
            tomori_server::test_support::TestServer::with_tokens(visitors, move |server| {
                let server = server.with_tls(config);
                let server = if pairing {
                    server.with_pairing(pin)
                } else {
                    server
                };
                let _ = sender.send(pairing.then(|| server.pairing()));
                server
            });
        let pairing = handle.recv().expect("the server starts");
        Self {
            server,
            pin,
            pairing,
        }
    }

    pub fn address(&self) -> String {
        self.server.address.to_string()
    }

    /// A token naming `name` as an owner.
    pub fn owner(&self, name: &str) -> String {
        self.server.issue(name, tomori_security::Grant::Owner).1
    }

    /// The headset's participant named `name`, keeping nothing, connected to this server with `token`, or as a visitor without one.
    pub fn participant(&self, name: &str, token: Option<String>) -> (Conversation, Connection) {
        self.participant_with(name, token, Box::new(Silent))
    }

    /// [`Self::participant`], with `capture` for its microphone.
    pub fn participant_with(
        &self,
        name: &str,
        token: Option<String>,
        capture: Box<dyn Capture>,
    ) -> (Conversation, Connection) {
        self.participant_kept(name, token, capture, Store::new(None))
    }

    /// [`Self::participant_with`], keeping what it keeps in `store`, which may already hold something for this server.
    pub fn participant_kept(
        &self,
        name: &str,
        token: Option<String>,
        capture: Box<dyn Capture>,
        store: Store,
    ) -> (Conversation, Connection) {
        let identity = Identity::new(
            &store,
            None,
            Overrides {
                server: Some(self.address()),
                name: Some(name.to_owned()),
                token,
            },
        );
        let mut conversation = Conversation::new(&identity.name);
        let connection = Connection::new(
            identity,
            store,
            Box::new(()),
            capture,
            Box::new(Nowhere),
            &mut conversation,
        );
        (conversation, connection)
    }

    /// Another client in this server's spaces, named `name`, with `token`, or as a visitor without one.
    pub fn remote(&self, name: &str, token: Option<String>) -> Remote {
        let mut options =
            tomori_client::Options::new(self.address(), name.to_lowercase(), name.to_owned());
        options.token = token;
        options.transport = tomori_client::Transport::Tls {
            pin: Some(self.pin),
        };
        Remote {
            client: tomori_client::Client::connect(options).expect("the client starts"),
        }
    }
}

/// A client of the [`Server`], other than the one tested.
pub struct Remote {
    pub client: tomori_client::Client,
}

impl Remote {
    pub fn send(&self, message: tomori_protocol::ClientMessage) {
        self.client.send(message).expect("the message is queued");
    }

    /// The first message `select` chooses, waiting for it, while `meanwhile` runs before each look at the events, such as the frames of the client tested. It runs at least once, however soon the message arrives.
    pub fn wait_for<T>(
        &self,
        mut meanwhile: impl FnMut(),
        mut select: impl FnMut(&tomori_protocol::ServerMessage) -> Option<T>,
    ) -> T {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "timed out");
            meanwhile();
            while let Some(event) = self.client.try_event() {
                if let tomori_client::Event::Message(message) = event
                    && let Some(selected) = select(&message)
                {
                    return selected;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// The first snapshot of a space that `select` chooses, waiting for it while `meanwhile` runs.
    pub fn space<T>(
        &self,
        meanwhile: impl FnMut(),
        mut select: impl FnMut(&tomori_protocol::SpaceState) -> Option<T>,
    ) -> T {
        self.wait_for(meanwhile, |message| {
            let tomori_protocol::ServerMessage::Space { state } = message else {
                return None;
            };
            select(state)
        })
    }

    /// Create a space with the scripted backend, and wait for its first snapshot.
    pub fn create_space(
        &self,
        title: &str,
        meanwhile: impl FnMut(),
    ) -> tomori_protocol::SpaceState {
        self.send(tomori_protocol::ClientMessage::CreateSpace {
            title: title.to_owned(),
            backend: tomori_server::fake::FAKE_BACKEND_NAME.to_owned(),
            directory: "/tmp".to_owned(),
            session: None,
            environment: None,
            instructions: None,
        });
        self.space(meanwhile, |state| {
            (state.title == title).then(|| state.clone())
        })
    }
}

/// A microphone that hears `samples`, once, each time it opens, then nothing until it is closed; live or for push-to-talk alike.
pub struct Talker {
    pub samples: Vec<i16>,
}

/// A capture thread, stopped and joined when dropped.
struct Talking {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Talking {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.thread.take().map(std::thread::JoinHandle::join);
    }
}

impl Capture for Talker {
    fn open(&mut self, mut samples: Samples, _failed: Failed) -> Result<Recording, String> {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Halfway between samples, so that converting back to 16 bits gives each sample exactly.
        let speech: Vec<f32> = self
            .samples
            .iter()
            .map(|sample| (f32::from(*sample) + 0.5) / f32::from(i16::MAX))
            .collect();
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            for chunk in speech.chunks(320) {
                samples(chunk);
            }
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        Ok(Recording::new(
            16_000,
            Talking {
                stop,
                thread: Some(thread),
            },
        ))
    }

    fn open_live(
        &mut self,
        samples: Samples,
        failed: Failed,
        _active: crate::capture::Active,
    ) -> Result<Recording, String> {
        self.open(samples, failed)
    }
}
