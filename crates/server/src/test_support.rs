// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Helpers for end-to-end tests: a server on an ephemeral port with the fake backend, and a blocking client speaking raw HTTP/1.1 chunked framing over TCP.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base_scheduler::{Cancellation, SocketLocalAddress};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::block_on;
use tomori_protocol::{
    AudioFrame, ClientMessage, Frame, FrameDecoder, FrameKind, Gate, PROTOCOL_VERSION, Resource,
    ServerMessage, SpaceState,
};

use crate::fake::{FAKE_BACKEND_NAME, FakeBackend};
use crate::{Backends, Server, Transcriber, TranscriptSink, UtteranceId, WorkspaceRoots};

/// How long test helpers wait for an expected message.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Call `check` until it returns a value, for state that the server updates after telling clients, such as what a space's driver has sent its session. Panics after [`TIMEOUT`].
pub fn eventually<T>(mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a condition"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A transcriber that reads each sample as a Unicode scalar value, so tests can "speak" exact text. See [`tomori_speech`].
#[derive(Default)]
pub struct TextTranscriber {
    utterances: Mutex<HashMap<UtteranceId, (TranscriptSink, String)>>,
    /// The longest utterance it takes, if it limits them.
    limit: Option<Duration>,
}

impl TextTranscriber {
    /// A transcriber that takes utterances of up to `limit`.
    pub fn limited(limit: Duration) -> Self {
        Self {
            limit: Some(limit),
            ..Self::default()
        }
    }
}

/// Samples that [`TextTranscriber`] transcribes as `text`.
pub fn speech(text: &str) -> Vec<i16> {
    text.chars()
        .map(|character| i16::try_from(u32::from(character)).unwrap_or(0x3f))
        .collect()
}

impl Transcriber for TextTranscriber {
    fn begin(&self, utterance: UtteranceId, sink: TranscriptSink) {
        self.utterances
            .lock()
            .unwrap()
            .insert(utterance, (sink, String::new()));
    }

    fn push(&self, utterance: UtteranceId, frame: &AudioFrame) {
        let partial = {
            let mut utterances = self.utterances.lock().unwrap();
            let Some((sink, text)) = utterances.get_mut(&utterance) else {
                return;
            };
            text.extend(
                frame
                    .samples
                    .iter()
                    .filter_map(|&sample| char::from_u32(u32::from(sample as u16))),
            );
            (sink.clone(), text.clone())
        };
        partial.0.partial(&partial.1);
    }

    fn end(&self, utterance: UtteranceId) {
        let finished = self.utterances.lock().unwrap().remove(&utterance);
        if let Some((sink, text)) = finished {
            sink.finish(&text);
        }
    }

    fn cancel(&self, utterance: UtteranceId) {
        self.utterances.lock().unwrap().remove(&utterance);
    }

    fn utterance_limit(&self) -> Option<Duration> {
        self.limit
    }
}

/// The identifier of the first environment a test server offers: an environment standing in for one an application registers, as Tomori's own server program registers Sky. Nothing outside tests names it.
pub const SKY: &str = "sky";

/// The environments a test server offers unless it is given its own: [`SKY`], first, so that a space whose creator names none is bound to it, and the grid, as Tomori's own server program offers Sky and the grid. A server offers none until it is given some; see `design/environments.md`.
pub fn environments() -> [crate::Environment; 2] {
    [
        crate::Environment::new(SKY, "Sky"),
        crate::Environment::new(tomori_protocol::GRID, "Grid"),
    ]
}

/// A server running on its own thread, listening on an ephemeral loopback port. Dropping it shuts the server down, and it offers [`environments`] unless it is given its own.
pub struct TestServer {
    pub address: SocketAddr,
    pub backend: FakeBackend,
    /// The token store, for a server that admits by tokens; see [`TestServer::with_tokens`].
    pub tokens: Option<Arc<tomori_security::TokenStore>>,
    /// The token store's temporary directory, removed when the server is dropped.
    state: Option<std::path::PathBuf>,
    /// Where Tomori's MCP endpoint listens, for a server that serves it; see [`TestServer::with_tools`].
    pub tools: Option<SocketAddr>,
    hub: Arc<crate::hub::Hub>,
    cancellation: Cancellation,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// A server whose fake sessions answer every prompt immediately.
    pub fn start() -> Self {
        Self::with_backend(FakeBackend::new())
    }

    /// A server that admits holders of its tokens, issued with [`TestServer::issue`], and anyone else as a visitor if `visitors`, offering [`environments`]. `configure` may change the server further, such as its visitor limit or the environments it offers.
    pub fn with_tokens(
        visitors: bool,
        configure: impl FnOnce(Server) -> Server + Send + 'static,
    ) -> Self {
        Self::admitting(visitors, configure, false)
    }

    /// A server as [`TestServer::with_tokens`] makes, also serving Tomori's MCP endpoint, as [`TestServer::with_tools`] does.
    pub fn with_tokens_and_tools(
        visitors: bool,
        configure: impl FnOnce(Server) -> Server + Send + 'static,
    ) -> Self {
        Self::admitting(visitors, configure, true)
    }

    fn admitting(
        visitors: bool,
        configure: impl FnOnce(Server) -> Server + Send + 'static,
        tools: bool,
    ) -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let state = std::env::temp_dir().join(format!(
            "tomori-test-tokens-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&state);
        let store =
            Arc::new(tomori_security::TokenStore::open(&state).expect("the token store opens"));
        let tokens = store.clone();
        let build = move |backends| {
            configure(
                Server::new(backends, Arc::new(TextTranscriber::default()))
                    .with_environments(environments())
                    .with_tokens(tokens, visitors),
            )
        };
        let mut server = Self::launch(FakeBackend::new(), build, tools);
        server.tokens = Some(store);
        server.state = Some(state);
        server
    }

    /// Issue a token granting `grant`, as `tomori-server auth mint` does.
    pub fn issue(
        &self,
        name: &str,
        grant: tomori_security::Grant,
    ) -> (tomori_security::Entry, String) {
        self.tokens
            .as_ref()
            .expect("the server admits by tokens")
            .issue(name, grant)
            .expect("the token is issued")
    }

    /// Revoke a token as `tomori-server auth revoke` does, from another process's view of the token store, which the server notices when it next reads the store.
    pub fn revoke_elsewhere(&self, id: &str) -> bool {
        let state = self.state.as_ref().expect("the server admits by tokens");
        tomori_security::TokenStore::open(state)
            .expect("the token store opens")
            .revoke(id)
            .expect("the token store is written")
    }

    /// Rename a token as `tomori-server auth rename` does, from another process's view of the token store, which the server notices when it next reads the store.
    pub fn rename_elsewhere(&self, id: &str, name: &str) -> bool {
        let state = self.state.as_ref().expect("the server admits by tokens");
        tomori_security::TokenStore::open(state)
            .expect("the token store opens")
            .rename(id, name, |_| false)
            .expect("the token store is written")
            == tomori_security::Renamed::Renamed
    }

    /// Open a connection presenting `token`, say hello and wait for the welcome.
    pub fn connect_with_token(&self, participant: &str, name: &str, token: &str) -> TestClient {
        let mut client = TestClient::open_with(
            self.address,
            &[("Authorization", &format!("Bearer {token}"))],
        );
        client.hello(participant, name);
        client
    }

    /// Open `POST /connect` with the `fields` given, and return the response's status and body, for a connection that is refused.
    pub fn refused(&self, fields: &[(&str, &str)]) -> (u16, String) {
        let mut stream = TcpStream::connect(self.address).unwrap();
        let fields: String = fields
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect();
        write!(
            stream,
            "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {}\r\nContent-Length: 0\r\nConnection: close\r\n{fields}\r\n",
            tomori_protocol::CONNECT_PATH,
            tomori_protocol::CONTENT_TYPE
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default();
        (status, body)
    }

    pub fn with_backend(backend: FakeBackend) -> Self {
        Self::with_server(backend, |backends| {
            Server::new(backends, Arc::new(TextTranscriber::default()))
        })
    }

    /// A server built by `build` from the backends, such as with another transcriber or a synthesizer, offering [`environments`] unless `build` offers its own.
    pub fn with_server(
        backend: FakeBackend,
        build: impl FnOnce(Backends) -> Server + Send + 'static,
    ) -> Self {
        Self::launch(backend, build, false)
    }

    /// A server as [`TestServer::with_server`] makes, also serving Tomori's MCP endpoint on another ephemeral loopback port, at [`TestServer::tools`]. See `design/mcp.md`.
    pub fn with_tools(
        backend: FakeBackend,
        build: impl FnOnce(Backends) -> Server + Send + 'static,
    ) -> Self {
        Self::launch(backend, build, true)
    }

    /// The bearer token of the MCP endpoint of a server made by [`TestServer::with_tools`], once it serves it, as its registration with backends carries it, so that a test can call the endpoint before anything is registered.
    pub fn tools_token(&self) -> Option<String> {
        self.hub.served_tools().and_then(|server| {
            server
                .headers
                .iter()
                .find(|(name, _)| name == "Authorization")
                .and_then(|(_, value)| value.strip_prefix("Bearer "))
                .map(str::to_owned)
        })
    }

    fn launch(
        backend: FakeBackend,
        build: impl FnOnce(Backends) -> Server + Send + 'static,
        tools: bool,
    ) -> Self {
        let cancellation = Cancellation::new();
        let (sender, receiver) = channel();
        let thread = {
            let backend = backend.clone();
            let cancellation = cancellation.clone();
            std::thread::spawn(move || {
                let scheduler = NativeScheduler::with_options(NativeOptions {
                    workers: 2,
                    threads: 2,
                    capacity: 256,
                    ..Default::default()
                })
                .expect("the scheduler starts");
                // Tests name `/tmp` or the temporary directory unless they give their own roots.
                let roots = WorkspaceRoots::new(["/tmp".into(), std::env::temp_dir()])
                    .expect("the temporary directories exist");
                let server = build(Backends::new().with(backend))
                    .with_default_workspace_roots(roots)
                    .with_default_environments(environments());
                let hub = server.hub.clone();
                block_on(async {
                    let address = "127.0.0.1:0".parse().unwrap();
                    let listener = crate::bind(&scheduler, address, &cancellation)
                        .await
                        .expect("the listener binds");
                    let local = scheduler.socket_local_address(&listener).unwrap();
                    let served = if tools {
                        let tools = crate::bind(&scheduler, address, &cancellation)
                            .await
                            .expect("the tools' listener binds");
                        let at = scheduler.socket_local_address(&tools).unwrap();
                        sender.send((local, Some(at), hub)).unwrap();
                        server
                            .run_with_tools(&scheduler, listener, tools, &cancellation)
                            .await
                    } else {
                        sender.send((local, None, hub)).unwrap();
                        server.run(&scheduler, listener, &cancellation).await
                    };
                    served.expect("the server runs until it is cancelled");
                });
                block_on(scheduler.shutdown());
            })
        };
        let (address, tools, hub) = receiver.recv().expect("the server starts");
        Self {
            address,
            backend,
            tokens: None,
            state: None,
            tools,
            hub,
            cancellation,
            thread: Some(thread),
        }
    }

    /// Open a connection, say hello and wait for the welcome.
    pub fn connect(&self, participant: &str, name: &str) -> TestClient {
        let mut client = TestClient::open(self.address);
        client.hello(participant, name);
        client
    }

    /// Make one plain request and return the status code and body.
    pub fn get(&self, path: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(self.address).unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default();
        (status, body)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let thread = self.thread.take().expect("joined only here");
        // Not panicking again when a failing test drops the server.
        let joined = thread.join().is_ok() | std::thread::panicking();
        assert!(joined, "the server thread exits cleanly");
        if let Some(state) = self.state.take() {
            let _ = std::fs::remove_dir_all(state);
        }
    }
}

/// A blocking client connection. Received control messages and audio frames are queued separately.
pub struct TestClient {
    stream: TcpStream,
    messages: Receiver<ServerMessage>,
    audio: Receiver<AudioFrame>,
    images: Receiver<tomori_protocol::ImageChunk>,
    /// The audio source number assigned in the welcome.
    pub source: u32,
    /// The role named in the welcome.
    pub role: tomori_protocol::ConnectionRole,
    sequence: u32,
    /// Every control message received, as JSON, in order, whether or not a test has read it.
    received: Arc<Mutex<Vec<String>>>,
}

impl TestClient {
    /// Open `POST /connect` without saying hello.
    pub fn open(address: SocketAddr) -> Self {
        Self::open_with(address, &[])
    }

    /// Open `POST /connect` with the extra request `fields` given, such as credentials, without saying hello.
    pub fn open_with(address: SocketAddr, fields: &[(&str, &str)]) -> Self {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.set_nodelay(true).unwrap();
        let fields: String = fields
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect();
        write!(
            stream,
            "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\n{fields}\r\n",
            tomori_protocol::CONNECT_PATH,
            tomori_protocol::CONTENT_TYPE
        )
        .unwrap();
        let (message_sender, messages) = channel();
        let (audio_sender, audio) = channel();
        let (image_sender, images) = channel();
        let reader = stream.try_clone().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let record = received.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(
                line.starts_with("HTTP/1.1 200"),
                "unexpected status {line:?}"
            );
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            let mut decoder = FrameDecoder::new();
            while let Some(chunk) = read_chunk(&mut reader) {
                decoder.push(&chunk);
                while let Some(frame) = decoder.next_frame().unwrap() {
                    let delivered = match frame.kind {
                        FrameKind::Control => {
                            let message = frame.to_server_message().unwrap();
                            record
                                .lock()
                                .unwrap()
                                .push(serde_json::to_string(&message).unwrap());
                            message_sender.send(message).is_ok()
                        }
                        FrameKind::Audio => audio_sender.send(frame.to_audio().unwrap()).is_ok(),
                        FrameKind::Image => image_sender.send(frame.to_image().unwrap()).is_ok(),
                    };
                    if !delivered {
                        return;
                    }
                }
            }
        });
        Self {
            stream,
            messages,
            audio,
            images,
            source: 0,
            role: tomori_protocol::ConnectionRole::Unknown,
            sequence: 0,
            received,
        }
    }

    /// Every control message received so far, as JSON, whether or not the test has read it.
    pub fn received(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }

    pub fn hello(&mut self, participant: &str, name: &str) {
        self.hello_presenting(participant, name, None);
    }

    /// Say hello as [`Self::hello`] does, as a client that reads what is projected in its space. See `design/projection.md`.
    pub fn hello_projected(&mut self, participant: &str, name: &str) {
        self.greet(participant, name, None, (true, false, false));
    }

    /// Say hello as [`Self::hello`] does, as a client that reads whether pairing is open, as an owner's is told. See `design/security.md`.
    pub fn hello_reading_pairing(&mut self, participant: &str, name: &str) {
        self.greet(participant, name, None, (false, true, false));
    }

    /// Say hello as [`Self::hello_projected`] does, as a client that also shows image panels, fetching their images.
    pub fn hello_showing_images(&mut self, participant: &str, name: &str) {
        self.greet(participant, name, None, (true, false, true));
    }

    /// Say hello as [`Self::hello`] does, listing the environments this client `presents`, and return those the welcome offers it. See `design/environments.md`.
    pub fn hello_presenting(
        &mut self,
        participant: &str,
        name: &str,
        presents: Option<Vec<tomori_protocol::EnvironmentVersion>>,
    ) -> Vec<tomori_protocol::EnvironmentInfo> {
        self.greet(participant, name, presents, (false, false, false))
    }

    fn greet(
        &mut self,
        participant: &str,
        name: &str,
        presents: Option<Vec<tomori_protocol::EnvironmentVersion>>,
        (projection, pairing_window, images): (bool, bool, bool),
    ) -> Vec<tomori_protocol::EnvironmentInfo> {
        self.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: name.to_owned(),
            names: false,
            environments: presents,
            environment_state: false,
            projection,
            pairing_window,
            images,
        });
        // The welcome is the first reply to hello.
        let environments;
        (self.source, self.role, environments) = self.wait_for(|message| {
            let ServerMessage::Welcome {
                source,
                role,
                environments,
                ..
            } = message
            else {
                unreachable!("the welcome comes first, not {message:?}");
            };
            Some((*source, *role, environments.clone()))
        });
        environments
    }

    fn write_chunk(&mut self, data: &[u8]) {
        let mut chunk = format!("{:x}\r\n", data.len()).into_bytes();
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(b"\r\n");
        self.stream.write_all(&chunk).unwrap();
    }

    /// Write raw request body bytes, which may split chunks and frames anywhere.
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
    }

    pub fn send(&mut self, message: &ClientMessage) {
        self.write_chunk(&Frame::client(message).encode());
    }

    /// Send `message` as a control message, as a client that says what [`ClientMessage`] does not might.
    pub fn send_json(&mut self, message: &serde_json::Value) {
        self.write_chunk(&Frame::control(message).encode());
    }

    /// Send one audio frame with the next sequence number.
    pub fn send_audio(&mut self, gate: Gate, samples: Vec<i16>) {
        let frame = AudioFrame {
            gate,
            source: 0,
            sequence: self.sequence,
            timestamp: u64::from(self.sequence) * 20_000,
            sample_rate: 16_000,
            samples,
        };
        self.sequence += 1;
        self.write_chunk(&Frame::audio(&frame).encode());
    }

    /// Skip sequence numbers, as if frames were lost.
    pub fn skip_sequence(&mut self, count: u32) {
        self.sequence += count;
    }

    /// Speak `text` to the agent as one push-to-talk utterance.
    pub fn say(&mut self, utterance: &str, text: &str) {
        self.say_attending(utterance, text, Vec::new(), None);
    }

    /// Speak `text` to the agent as [`Self::say`] does, attending to `attention` as the utterance begins, and naming
    /// `pointed` as it ends, as clients from before attention did.
    pub fn say_attending(
        &mut self,
        utterance: &str,
        text: &str,
        attention: Vec<Resource>,
        pointed: Option<Resource>,
    ) {
        self.send(&ClientMessage::UtteranceBegin {
            utterance: utterance.to_owned(),
            answer: None,
            progress: false,
            naming: false,
            titling: false,
            attention,
        });
        self.send_audio(Gate::Agent, speech(text));
        self.send(&ClientMessage::UtteranceEnd {
            utterance: utterance.to_owned(),
            pointed,
        });
    }

    /// End the upload; the server ends the response after sending what is queued.
    pub fn finish(&mut self) {
        self.stream.write_all(b"0\r\n\r\n").unwrap();
    }

    /// The next control message, or `None` after `timeout`.
    pub fn next_message(&self, timeout: Duration) -> Option<ServerMessage> {
        self.messages.recv_timeout(timeout).ok()
    }

    /// Skip messages until `select` returns a value. Panics after [`TIMEOUT`].
    pub fn wait_for<T>(&self, mut select: impl FnMut(&ServerMessage) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self
                .next_message(remaining)
                .expect("timed out waiting for a matching message");
            if let Some(value) = select(&message) {
                return value;
            }
        }
    }

    /// Wait for a space snapshot satisfying `predicate`.
    pub fn space(&self, mut predicate: impl FnMut(&SpaceState) -> bool) -> SpaceState {
        self.wait_for(|message| match message {
            ServerMessage::Space { state } if predicate(state) => Some((**state).clone()),
            _ => None,
        })
    }

    /// Wait for an error message and return its text.
    pub fn error(&self) -> String {
        self.wait_for(|message| match message {
            ServerMessage::Error { error, .. } => Some(error.clone()),
            _ => None,
        })
    }

    /// Create a fake space; the creator joins it. Returns its first snapshot.
    pub fn create_space(&mut self, title: &str) -> SpaceState {
        self.send(&ClientMessage::CreateSpace {
            title: title.to_owned(),
            backend: FAKE_BACKEND_NAME.to_owned(),
            directory: "/tmp".to_owned(),
            session: None,
            environment: Some(SKY.to_owned()),
            instructions: None,
        });
        let title = title.to_owned();
        self.space(|state| state.title == title)
    }

    pub fn next_audio(&self, timeout: Duration) -> Option<AudioFrame> {
        self.audio.recv_timeout(timeout).ok()
    }

    /// The next part of an image the server sends, within `timeout`.
    pub fn next_image_chunk(&self, timeout: Duration) -> Option<tomori_protocol::ImageChunk> {
        self.images.recv_timeout(timeout).ok()
    }

    /// Whether the server has ended the response.
    pub fn is_closed(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(_) => continue,
                Err(RecvTimeoutError::Disconnected) => return true,
                Err(RecvTimeoutError::Timeout) => return false,
            }
        }
    }
}

/// Read one chunk of a chunked body, or `None` at the end of the body or connection.
fn read_chunk(reader: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let size = line
        .trim_end()
        .split(';')
        .next()
        .expect("a split yields at least one piece");
    let size = usize::from_str_radix(size, 16).ok()?;
    if size == 0 {
        return None;
    }
    let mut chunk = vec![0; size + 2];
    reader.read_exact(&mut chunk).ok()?;
    chunk.truncate(size);
    Some(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Called only from the integration tests otherwise, where it is inlined and its coverage is not counted as its own.
    #[test]
    fn skipping_sequence_numbers_moves_the_next_one_on() {
        let server = TestServer::start();
        let mut client = server.connect("ada", "Ada");
        let sequence = client.sequence;
        client.skip_sequence(3);
        assert_eq!(client.sequence, sequence + 3);
    }

    #[test]
    fn eventually_checks_again_until_there_is_a_value() {
        let mut checks = 0;
        let value = eventually(|| {
            checks += 1;
            (checks == 2).then_some(checks)
        });
        assert_eq!(value, 2);
    }

    #[test]
    fn the_text_transcriber_ignores_utterances_it_did_not_begin() {
        let transcriber = TextTranscriber::default();
        let frame = AudioFrame {
            gate: tomori_protocol::Gate::Agent,
            source: 0,
            sequence: 0,
            timestamp: 0,
            sample_rate: 16_000,
            samples: speech("hi"),
        };
        transcriber.push(UtteranceId(1), &frame);
        transcriber.end(UtteranceId(1));
        assert!(transcriber.utterances.lock().unwrap().is_empty());
    }

    /// A connection that fails as it is read.
    struct Reset;

    impl std::io::Read for Reset {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::ConnectionReset.into())
        }
    }

    #[test]
    fn a_chunk_ends_with_its_body_or_its_connection() {
        assert_eq!(
            read_chunk(&mut "2;x=y\r\nab\r\n".as_bytes()),
            Some(b"ab".to_vec())
        );
        assert_eq!(read_chunk(&mut "0\r\n\r\n".as_bytes()), None);
        assert_eq!(read_chunk(&mut "".as_bytes()), None);
        assert_eq!(read_chunk(&mut "z\r\n".as_bytes()), None);
        // Cut off, or reset.
        assert_eq!(read_chunk(&mut "5\r\nab".as_bytes()), None);
        assert_eq!(read_chunk(&mut std::io::BufReader::new(Reset)), None);
    }
}
