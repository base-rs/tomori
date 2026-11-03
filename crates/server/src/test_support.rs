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
use protocol::{
    AudioFrame, ClientMessage, Frame, FrameDecoder, FrameKind, Gate, PROTOCOL_VERSION,
    ServerMessage, SpaceState,
};

use crate::fake::{FAKE_BACKEND_NAME, FakeBackend};
use crate::{Backends, Server, Transcriber, TranscriptSink, UtteranceId, WorkspaceRoots};

/// How long test helpers wait for an expected message.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// A transcriber that reads each sample as a Unicode scalar value, so tests can "speak" exact text. See [`speech`].
#[derive(Default)]
pub struct TextTranscriber {
    utterances: Mutex<HashMap<UtteranceId, (TranscriptSink, String)>>,
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
}

/// A server running on its own thread, listening on an ephemeral loopback port. Dropping it shuts the server down.
pub struct TestServer {
    pub address: SocketAddr,
    pub backend: FakeBackend,
    /// The token store, for a server that admits by tokens; see [`TestServer::with_tokens`].
    pub tokens: Option<Arc<security::TokenStore>>,
    /// The token store's temporary directory, removed when the server is dropped.
    state: Option<std::path::PathBuf>,
    cancellation: Cancellation,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// A server whose fake sessions answer every prompt immediately.
    pub fn start() -> Self {
        Self::with_backend(FakeBackend::new())
    }

    /// A server that admits holders of its tokens, issued with [`TestServer::issue`], and anyone else as a visitor if `visitors`. `configure` may change the server further, such as its visitor limit.
    pub fn with_tokens(
        visitors: bool,
        configure: impl FnOnce(Server) -> Server + Send + 'static,
    ) -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let state = std::env::temp_dir().join(format!(
            "tomori-test-tokens-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&state);
        let store = Arc::new(security::TokenStore::open(&state).expect("the token store opens"));
        let tokens = store.clone();
        let mut server = Self::with_server(FakeBackend::new(), move |backends| {
            configure(
                Server::new(backends, Arc::new(TextTranscriber::default()))
                    .with_tokens(tokens, visitors),
            )
        });
        server.tokens = Some(store);
        server.state = Some(state);
        server
    }

    /// Issue a token granting `grant`, as `tomori-server auth mint` does.
    pub fn issue(&self, name: &str, grant: security::Grant) -> (security::Entry, String) {
        self.tokens
            .as_ref()
            .expect("the server admits by tokens")
            .issue(name, grant)
            .expect("the token is issued")
    }

    /// Revoke a token as `tomori-server auth revoke` does, from another process's view of the token store, which the server notices when it next reads the store.
    pub fn revoke_elsewhere(&self, id: &str) -> bool {
        let state = self.state.as_ref().expect("the server admits by tokens");
        security::TokenStore::open(state)
            .expect("the token store opens")
            .revoke(id)
            .expect("the token store is written")
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
            protocol::CONNECT_PATH,
            protocol::CONTENT_TYPE
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

    /// A server built by `build` from the backends, such as with another transcriber or a synthesizer.
    pub fn with_server(
        backend: FakeBackend,
        build: impl FnOnce(Backends) -> Server + Send + 'static,
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
                let server =
                    build(Backends::new().with(backend)).with_default_workspace_roots(roots);
                block_on(async {
                    let address = "127.0.0.1:0".parse().unwrap();
                    let listener = crate::bind(&scheduler, address, &cancellation)
                        .await
                        .expect("the listener binds");
                    sender
                        .send(scheduler.socket_local_address(&listener).unwrap())
                        .unwrap();
                    if let Err(error) = server.run(&scheduler, listener, &cancellation).await {
                        panic!("the server failed: {error}");
                    }
                });
                block_on(scheduler.shutdown());
            })
        };
        let address = receiver.recv().expect("the server starts");
        Self {
            address,
            backend,
            tokens: None,
            state: None,
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
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !std::thread::panicking() {
                result.expect("the server thread exits cleanly");
            }
        }
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
    /// The audio source number assigned in the welcome.
    pub source: u32,
    /// The role named in the welcome.
    pub role: protocol::ConnectionRole,
    sequence: u32,
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
            protocol::CONNECT_PATH,
            protocol::CONTENT_TYPE
        )
        .unwrap();
        let (message_sender, messages) = channel();
        let (audio_sender, audio) = channel();
        let reader = stream.try_clone().unwrap();
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
                        FrameKind::Control => message_sender
                            .send(frame.to_server_message().unwrap())
                            .is_ok(),
                        FrameKind::Audio => audio_sender.send(frame.to_audio().unwrap()).is_ok(),
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
            source: 0,
            role: protocol::ConnectionRole::Unknown,
            sequence: 0,
        }
    }

    pub fn hello(&mut self, participant: &str, name: &str) {
        self.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: name.to_owned(),
        });
        (self.source, self.role) = self.wait_for(|message| match message {
            ServerMessage::Welcome { source, role, .. } => Some((*source, *role)),
            _ => None,
        });
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
        self.send(&ClientMessage::UtteranceBegin {
            utterance: utterance.to_owned(),
            answer: None,
        });
        self.send_audio(Gate::Agent, speech(text));
        self.send(&ClientMessage::UtteranceEnd {
            utterance: utterance.to_owned(),
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
            environment: Some(crate::environments::SKY.to_owned()),
            instructions: None,
        });
        let title = title.to_owned();
        self.space(|state| state.title == title)
    }

    pub fn next_audio(&self, timeout: Duration) -> Option<AudioFrame> {
        self.audio.recv_timeout(timeout).ok()
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
    let size = line.trim_end().split(';').next()?;
    let size = usize::from_str_radix(size, 16).ok()?;
    if size == 0 {
        return None;
    }
    let mut chunk = vec![0; size + 2];
    reader.read_exact(&mut chunk).ok()?;
    chunk.truncate(size);
    Some(chunk)
}
