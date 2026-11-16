// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A minimal Tomori server on `base-http-server`: it answers each `/connect` request body with a response body that streams concurrently, frame by frame.
use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use base_endpoint::{AddressEndpoint, BindEndpoint, BindOptions};
use base_http::body::{BodyError, BodyFrame, ReadableBody};
use base_http::{InterimHandler, InterimResponses, Request, Response, Status};
use base_http_server::Handler;
use base_scheduler::{Cancellation, SocketLocalAddress};
use base_scheduler_native::NativeScheduler;
use futures_lite::future::block_on;
use tomori_protocol::{
    AudioFrame, CONNECT_PATH, ClientMessage, Frame, FrameDecoder, FrameKind, PROTOCOL_VERSION,
    ServerMessage, SpaceState,
};

/// The source the mini server assigns to echoed audio.
pub const ECHO_SOURCE: u32 = 99;

/// The image the mini server sends for any image asked for, larger than one chunk, and its hash; a request for
/// [`BROKEN_IMAGE`] is sent the same bytes under that hash, which they are not.
pub fn image() -> (String, Vec<u8>) {
    let bytes: Vec<u8> = (0..tomori_protocol::IMAGE_CHUNK_BYTES + 100)
        .map(|index| (index % 253) as u8)
        .collect();
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(&bytes).into();
    (tomori_protocol::hash_text(&hash), bytes)
}

pub const BROKEN_IMAGE: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The ticket the mini server gives a client that reads content, and requires of each content request.
pub const TICKET: &str = "the-ticket";

/// How the mini server answers a request for one of the assets it offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// As it should: whole, or from a range asked for.
    Whole,
    /// With another `Content-Type`.
    Html,
    /// With more bytes than offered.
    Longer,
    /// Not found.
    Missing,
    /// With bytes that are not what its digest says.
    Other,
    /// Always as a range from its fifth byte, whatever was asked.
    WrongRange,
    /// Its head saying more bytes than its body has, so that its body ends early.
    Cut,
}

/// An asset the mini server offers: its name, its bytes, and how it answers for it.
pub struct Offered {
    pub name: &'static str,
    pub bytes: Vec<u8>,
    pub answer: Answer,
}

impl Offered {
    pub fn digest(&self) -> tomori_assets::Digest {
        tomori_assets::Digest::of(&self.bytes)
    }
}

/// The assets the mini server offers, in order, each as a model of its own: those answered as they should, one to
/// resume and one whose kept part is not its own among them, then each way of answering wrongly, and last a sentinel,
/// fetched once everything before it is done.
pub fn offered() -> Vec<Offered> {
    let model = |tag: &str| {
        let mut bytes = b"glTF".to_vec();
        bytes.extend(tag.bytes().cycle().take(3000));
        bytes
    };
    [
        ("good.glb", Answer::Whole),
        ("resumed.glb", Answer::Whole),
        ("reprefixed.glb", Answer::Whole),
        ("html.glb", Answer::Html),
        ("longer.glb", Answer::Longer),
        ("missing.glb", Answer::Missing),
        ("other.glb", Answer::Other),
        ("ranged.glb", Answer::WrongRange),
        ("cut.glb", Answer::Cut),
        ("sentinel.glb", Answer::Whole),
    ]
    .into_iter()
    .map(|(name, answer)| Offered {
        name,
        bytes: model(name),
        answer,
    })
    .collect()
}

/// What the mini server offers by digest, as it offers it: each of [`offered`], then one whose digest is not written as
/// one, and one larger than a model may be.
fn offer() -> ServerMessage {
    let asset = |name: &str, digest: String, length| tomori_protocol::OfferedAsset {
        environment: "mini".into(),
        version: 1,
        name: name.into(),
        digest,
        length,
        kind: tomori_protocol::ContentKind::Model,
        licence: None,
    };
    let mut assets: Vec<_> = offered()
        .iter()
        .map(|offered| {
            asset(
                offered.name,
                offered.digest().to_string(),
                offered.bytes.len() as u64,
            )
        })
        .collect();
    assets.insert(0, asset("unwritten.glb", "not a digest".into(), 10));
    assets.insert(1, asset("huge.glb", "0".repeat(64), 1 << 30));
    ServerMessage::Content {
        session: TICKET.into(),
        assets,
    }
}

/// What the client sent, by connection number (counting from one).
pub type Log = Arc<Mutex<Vec<(u32, Received)>>>;

#[derive(Clone, Debug, PartialEq)]
pub enum Received {
    Message(ClientMessage),
    Audio(u32),
    /// A request for content: the asset's name, and the range it asked for, if any.
    Fetch(String, Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Welcome with an incompatible protocol version.
    WrongProtocol,
    /// Respond 503 to every connection.
    Unavailable,
    /// Stop reading the request body after Hello, so the client's writes back up.
    Stall,
    /// Echo each audio frame back attributed to the client itself, as well as to [`ECHO_SOURCE`].
    EchoToSender,
}

struct Mini {
    mode: Mode,
    connections: AtomicU32,
    log: Log,
}

impl Handler for Mini {
    type Error = Infallible;
    type Body<B: ReadableBody> = Session<B>;

    async fn call<B: ReadableBody, I: InterimHandler>(
        &self,
        request: Request<B>,
        _: &mut InterimResponses<I>,
        _: &Cancellation,
    ) -> Result<Response<Session<B>>, Infallible> {
        if let Some(digest) = request
            .path
            .as_deref()
            .and_then(|path| path.strip_prefix(tomori_protocol::CONTENT_PATH))
            .map(str::to_owned)
        {
            let fields = request.fields.clone();
            return Ok(self.content(request.body, digest, &fields));
        }
        let path_matches = request.path.as_deref() == Some(CONNECT_PATH);
        let status = if self.mode == Mode::Unavailable || !path_matches {
            Status::SERVICE_UNAVAILABLE
        } else {
            Status::OK
        };
        let connection = self.connections.fetch_add(1, Ordering::SeqCst) + 1;
        let ended = status != Status::OK;
        Ok(Response::new(
            status,
            Session {
                body: request.body,
                decoder: FrameDecoder::new(),
                output: VecDeque::new(),
                ended,
                greeted: false,
                connection,
                mode: self.mode,
                log: self.log.clone(),
            },
        ))
    }
}

impl Mini {
    /// The answer to a request for the content `digest`, with `fields`, as [`offered`] says.
    fn content<B: ReadableBody>(
        &self,
        body: B,
        digest: String,
        fields: &base_http::Fields,
    ) -> Response<Session<B>> {
        let field = |name| {
            fields
                .values(name)
                .next()
                .map(|value| String::from_utf8(value.to_vec()).unwrap())
        };
        let offered = offered()
            .into_iter()
            .find(|offered| offered.digest().to_string() == digest)
            .unwrap();
        self.log
            .lock()
            .unwrap()
            .push((0, Received::Fetch(offered.name.to_owned(), field("range"))));
        let length = offered.bytes.len();
        let from = field("range")
            .and_then(|range| {
                range
                    .strip_prefix("bytes=")?
                    .strip_suffix('-')?
                    .parse()
                    .ok()
            })
            .unwrap_or(0);
        let ticketed = field(tomori_protocol::SESSION_FIELD).as_deref() == Some(TICKET);
        let (status, bytes, content_range) = match offered.answer {
            _ if !ticketed => (Status::new(401).unwrap(), Vec::new(), None),
            Answer::Missing => (Status::NOT_FOUND, Vec::new(), None),
            Answer::Longer => (
                Status::OK,
                [offered.bytes.clone(), vec![0; 10]].concat(),
                None,
            ),
            Answer::Other => (Status::OK, vec![b'x'; length], None),
            Answer::WrongRange => (
                Status::new(206).unwrap(),
                offered.bytes[5..].to_vec(),
                Some(format!("bytes 5-{}/{length}", length - 1)),
            ),
            _ if from > 0 => (
                Status::new(206).unwrap(),
                offered.bytes[from..].to_vec(),
                Some(format!("bytes {from}-{}/{length}", length - 1)),
            ),
            _ => (Status::OK, offered.bytes.clone(), None),
        };
        let content_type = if offered.answer == Answer::Html {
            "text/html"
        } else {
            "model/gltf-binary"
        };
        let cut = offered.answer == Answer::Cut;
        let declared = bytes.len() as u64 + if cut { 100 } else { 0 };
        let mut response = Response::new(
            status,
            Session {
                body,
                decoder: FrameDecoder::new(),
                output: VecDeque::from([bytes]),
                ended: true,
                greeted: false,
                connection: 0,
                mode: self.mode,
                log: self.log.clone(),
            },
        );
        response.metadata.content_length = Some(declared);
        response.fields.add("content-type", content_type).unwrap();
        if let Some(range) = content_range {
            response.fields.add("content-range", range).unwrap();
        }
        response
    }
}

/// Transforms the request body into the response body: each client frame produces zero or more server frames.
pub struct Session<B> {
    body: B,
    decoder: FrameDecoder,
    output: VecDeque<Vec<u8>>,
    ended: bool,
    greeted: bool,
    connection: u32,
    mode: Mode,
    log: Log,
}

impl<B: ReadableBody> Session<B> {
    fn record(&self, received: Received) {
        self.log.lock().unwrap().push((self.connection, received));
    }

    fn reply(&mut self, message: ServerMessage) {
        self.output.push_back(Frame::server(&message).encode());
    }

    fn handle(&mut self, frame: Frame) {
        match frame.kind {
            FrameKind::Audio => {
                let mut audio: AudioFrame = frame.to_audio().unwrap();
                self.record(Received::Audio(audio.sequence));
                if self.mode == Mode::EchoToSender {
                    // The client's source is its connection number; see the Welcome below.
                    audio.source = self.connection;
                    self.output.push_back(Frame::audio(&audio).encode());
                }
                audio.source = ECHO_SOURCE;
                self.output.push_back(Frame::audio(&audio).encode());
            }
            FrameKind::Image => unreachable!("a client sends no images"),
            FrameKind::Control => {
                let message = frame.to_client_message().unwrap();
                self.record(Received::Message(message.clone()));
                match message {
                    ClientMessage::Hello { content, .. } => {
                        self.greeted = true;
                        let protocol = if self.mode == Mode::WrongProtocol {
                            PROTOCOL_VERSION + 1
                        } else {
                            PROTOCOL_VERSION
                        };
                        self.reply(ServerMessage::Welcome {
                            protocol,
                            server: "mini".into(),
                            source: self.connection,
                            role: tomori_protocol::ConnectionRole::Owner,
                            backends: vec![],
                            roots: vec![],
                            environments: vec![],
                            utterance_limit: None,
                            name: Some("Ada Lovelace".into()),
                            limits: None,
                        });
                        if content {
                            self.reply(offer());
                        }
                    }
                    ClientMessage::Join { space } => self.reply(ServerMessage::Space {
                        state: Box::new(SpaceState {
                            id: space,
                            ..Default::default()
                        }),
                    }),
                    ClientMessage::Leave => self.reply(ServerMessage::LEFT),
                    // Renamed, as a visitor is, without keeping the name.
                    ClientMessage::SetName { name } => self.reply(ServerMessage::Name { name }),
                    ClientMessage::ListSpaces => {
                        self.reply(ServerMessage::Spaces { spaces: Vec::new() })
                    }
                    ClientMessage::Ping { nonce } => self.reply(ServerMessage::Pong { nonce }),
                    ClientMessage::FetchImage { image: asked } => {
                        let (hash, bytes) = image();
                        let hash = if asked == BROKEN_IMAGE { asked } else { hash };
                        let hash = tomori_protocol::hash_bytes(&hash).unwrap();
                        for chunk in tomori_protocol::ImageChunk::split(hash, &bytes) {
                            self.output.push_back(Frame::image(&chunk).encode());
                        }
                    }
                    // A test hook: end the response, which disconnects the client.
                    ClientMessage::Answer { text, .. } if text == "kick" => self.ended = true,
                    // A test hook: send an image frame too short to hold its header.
                    ClientMessage::Answer { text, .. } if text == "bad image" => {
                        self.output.push_back(
                            Frame {
                                kind: FrameKind::Image,
                                payload: vec![0; 3],
                            }
                            .encode(),
                        );
                    }
                    // A test hook: send an image that is not what its hash says, then name it.
                    ClientMessage::Answer { text, .. } if text == "image" => {
                        let chunk = tomori_protocol::ImageChunk::new([7; 32], 0, 3, vec![1, 2, 3]);
                        self.output.push_back(Frame::image(&chunk).encode());
                        self.reply(ServerMessage::Name {
                            name: "After the image".into(),
                        });
                    }
                    // A test hook: ping the client, which answers with a Pong.
                    ClientMessage::Answer { text, .. } if text == "ping" => {
                        self.reply(ServerMessage::Ping { nonce: 7 })
                    }
                    _ => {}
                }
            }
        }
    }
}

impl<B: ReadableBody> ReadableBody for Session<B> {
    type Data = Vec<u8>;
    type Error = BodyError;

    fn close(&mut self, _: Option<BodyError>) {
        self.body.close(None);
    }

    async fn read(&mut self, cancellation: &Cancellation) -> Result<BodyFrame<Vec<u8>>, BodyError> {
        loop {
            if self.output.is_empty() && self.ended {
                return Ok(BodyFrame::End);
            }
            if !self.output.is_empty() {
                let mut data = Vec::new();
                while let Some(bytes) = self.output.pop_front() {
                    data.extend(bytes);
                }
                return Ok(BodyFrame::Data { data, end: false });
            }
            if self.mode == Mode::Stall && self.greeted {
                cancellation.cancelled().await;
                return Err(BodyError::new(std::io::Error::other("stalled")));
            }
            match self.body.read(cancellation).await.map_err(BodyError::new)? {
                BodyFrame::Data { data, end } => {
                    self.decoder.push(data.as_ref());
                    while let Some(frame) = self.decoder.next_frame().unwrap() {
                        self.handle(frame);
                    }
                    self.ended |= end;
                }
                BodyFrame::Headers { end, .. } => self.ended |= end,
                BodyFrame::End => self.ended = true,
            }
        }
    }
}

pub struct MiniServer {
    pub address: SocketAddr,
    pub log: Log,
    stop: Cancellation,
    thread: Option<JoinHandle<()>>,
}

impl MiniServer {
    pub fn start(address: SocketAddr, mode: Mode) -> Self {
        let log = Log::default();
        let stop = Cancellation::new();
        let (bound, receive) = mpsc::channel();
        let handler = Mini {
            mode,
            connections: AtomicU32::new(0),
            log: log.clone(),
        };
        let lifetime = stop.clone();
        let thread = std::thread::spawn(move || {
            let scheduler = NativeScheduler::new().unwrap();
            block_on(async {
                let listener = AddressEndpoint::new(address)
                    .bind(&scheduler, BindOptions::default(), &lifetime)
                    .await
                    .unwrap()
                    .remove(0);
                bound
                    .send(scheduler.socket_local_address(&listener).unwrap())
                    .unwrap();
                let _ = base_http_server::serve(
                    &scheduler,
                    listener,
                    &handler,
                    &Default::default(),
                    &lifetime,
                )
                .await;
            });
            block_on(scheduler.shutdown());
        });
        Self {
            address: receive.recv().unwrap(),
            log,
            stop,
            thread: Some(thread),
        }
    }

    /// What connection `connection` received, in order.
    pub fn received(&self, connection: u32) -> Vec<Received> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(number, _)| *number == connection)
            .map(|(_, received)| received.clone())
            .collect()
    }
}

impl Drop for MiniServer {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
