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
}

impl Server {
    pub fn start() -> Self {
        let certificate = tomori_security::ServerCertificate::generate();
        let pin = certificate.fingerprint();
        let config = certificate
            .server_config()
            .expect("the certificate is usable");
        let server = tomori_server::test_support::TestServer::with_tokens(true, move |server| {
            server.with_tls(config)
        });
        Self { server, pin }
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
        let store = Store::new(None);
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
            Box::new(Silent),
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

    /// The first message `select` chooses, waiting for it, while `meanwhile` runs between events, such as the frames of the client tested.
    pub fn wait_for<T>(
        &self,
        mut meanwhile: impl FnMut(),
        mut select: impl FnMut(&tomori_protocol::ServerMessage) -> Option<T>,
    ) -> T {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "timed out");
            while let Some(event) = self.client.try_event() {
                if let tomori_client::Event::Message(message) = event
                    && let Some(selected) = select(&message)
                {
                    return selected;
                }
            }
            meanwhile();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
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
        self.wait_for(meanwhile, |message| match message {
            tomori_protocol::ServerMessage::Space { state } if state.title == title => {
                Some((**state).clone())
            }
            _ => None,
        })
    }
}
