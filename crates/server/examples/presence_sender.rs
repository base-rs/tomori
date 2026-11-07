// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A scripted participant that creates a space and reports a moving presence
//! (head pose and one hand), so a spectator's live-follow path can be
//! exercised without an OpenXR runtime.
//!
//! Usage: `cargo run -p tomori-server --example presence_sender -- [ADDRESS]`
//! where `ADDRESS` defaults to `127.0.0.1:8766`. It creates a space, prints
//! its id, and reports presence for `PRESENCE_SECS` seconds (default 30).
//! Start the spectator in another shell with the printed space id:
//! `./target/debug/tomori-spectator --pipewire --server ADDRESS --space ID`.
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tomori_protocol::{
    ClientMessage, Frame, Hand, JOINT_COUNT, Joint, PROTOCOL_VERSION, ServerMessage,
};

/// A short description of a server frame, for debug logging.
fn kind_of(message: &ServerMessage) -> String {
    match message {
        ServerMessage::Welcome { .. } => "Welcome".into(),
        ServerMessage::Spaces { .. } => "Spaces".into(),
        ServerMessage::Space { state } => format!("Space {}", state.id),
        ServerMessage::Error { error, .. } => format!("Error {error}"),
        ServerMessage::Presence { .. } => "Presence".into(),
        ServerMessage::Ping { .. } => "Ping".into(),
        ServerMessage::Pong { .. } => "Pong".into(),
        _ => "other".into(),
    }
}

/// A raw protocol client: one TCP connection speaking the chunked frame
/// protocol, with a reader thread decoding server frames into a channel.
struct Participant {
    stream: TcpStream,
    messages: mpsc::Receiver<tomori_protocol::ServerMessage>,
}

impl Participant {
    /// Open `POST /connect` at `address` and wait for the welcome.
    fn connect(address: &str, participant: &str, name: &str) -> Self {
        let mut stream =
            TcpStream::connect(address).unwrap_or_else(|e| panic!("connect to {address}: {e}"));
        stream.set_nodelay(true).unwrap();
        write!(
            stream,
            "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\n\r\n",
            tomori_protocol::CONNECT_PATH,
            tomori_protocol::CONTENT_TYPE
        )
        .unwrap();

        let (tx, rx) = mpsc::channel::<tomori_protocol::ServerMessage>();
        let reader = stream.try_clone().unwrap();
        thread::spawn(move || {
            read_server_frames(reader, tx);
        });

        let mut client = Self {
            stream,
            messages: rx,
        };

        client.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: name.to_owned(),
            names: false,
        });
        client.wait_for(|message| match message {
            ServerMessage::Welcome { source, .. } => Some(*source),
            _ => None,
        });
        println!("hello: joined the lobby as {name}");
        client
    }

    fn send(&mut self, message: &ClientMessage) {
        self.write_chunk(&Frame::client(message).encode());
    }

    fn write_chunk(&mut self, data: &[u8]) {
        let mut chunk = format!("{:x}\r\n", data.len()).into_bytes();
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(b"\r\n");
        self.stream.write_all(&chunk).unwrap();
        self.stream.flush().unwrap();
    }

    /// Skip server frames until `select` yields a value, or the deadline.
    fn wait_for<T>(&self, mut select: impl FnMut(&ServerMessage) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if Instant::now() >= deadline {
                panic!("timed out waiting for a matching message");
            }
            match self.messages.recv_timeout(Duration::from_millis(50)) {
                Ok(message) => match select(&message) {
                    Some(value) => return value,
                    None => eprintln!("[reader] skipped: {}", kind_of(&message)),
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("the server closed the connection");
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

/// Read the server's response body as chunked frames, decoding each
/// control frame into a [`ServerMessage`].
fn read_server_frames(stream: TcpStream, tx: mpsc::Sender<ServerMessage>) {
    let mut reader = std::io::BufReader::new(stream);
    // Consume the response head: bytes up to and including the blank line
    // that ends the headers. Read raw so a binary body already in the buffer
    // can never be parsed as a header line.
    let mut head = Vec::new();
    let terminator = *b"\r\n\r\n";
    loop {
        let mut byte = [0u8; 1];
        if reader.read_exact(&mut byte).is_err() {
            return;
        }
        head.push(byte[0]);
        if head.len() >= terminator.len() && head.ends_with(&terminator) {
            break;
        }
    }
    if !head.starts_with(b"HTTP/1.1 200") {
        eprintln!(
            "[reader] unexpected status: {}",
            String::from_utf8_lossy(&head)
        );
    }

    let mut decoder = tomori_protocol::FrameDecoder::new();
    loop {
        let mut chunk = Vec::new();
        if read_chunk(&mut reader, &mut chunk) == 0 {
            break;
        }
        decoder.push(&chunk);
        while let Some(frame) = decoder.next_frame().unwrap() {
            if tomori_protocol::FrameKind::Control == frame.kind
                && tx.send(frame.to_server_message().unwrap()).is_err()
            {
                return;
            }
        }
    }
}

/// Read one HTTP chunk of the response body into `out`, returning its length.
///
/// The size line is read with `read_line`; any bytes it buffered past the
/// newline stay in the [`BufReader`], so the following `read_exact` serves
/// them first.
fn read_chunk(reader: &mut std::io::BufReader<TcpStream>, out: &mut Vec<u8>) -> usize {
    let mut size_line = String::new();
    if reader.read_line(&mut size_line).unwrap() == 0 {
        return 0;
    }
    let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
    if size == 0 {
        return 0;
    }
    out.resize(size, 0);
    reader.read_exact(out).unwrap();
    let mut crlf = [0u8; 2];
    reader.read_exact(&mut crlf).unwrap(); // trailing CRLF
    size
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args = std::env::args().skip(1);
    let address = args.next().unwrap_or_else(|| "127.0.0.1:8766".to_owned());
    let space = args.next();

    let mut client = Participant::connect(&address, "scripted-participant", "Scripted");

    // Create a fresh space, or join one that was named.
    let space = match space {
        Some(space) => {
            client.send(&ClientMessage::Join {
                space: space.clone(),
            });
            client.wait_for(|message| match message {
                ServerMessage::Space { state } if state.id == space => Some(state.id.clone()),
                _ => None,
            });
            space
        }
        None => {
            // The server only accepts spaces within its workspace roots, so the
            // directory defaults to the current directory; override with
            // SPACE_DIRECTORY when the server is rooted elsewhere.
            let directory = std::env::var("SPACE_DIRECTORY")
                .or_else(|_| std::env::current_dir().map(|d| d.to_string_lossy().into_owned()))
                .unwrap_or_else(|_| ".".to_owned());
            client.send(&ClientMessage::CreateSpace {
                title: "Scripted Space".to_owned(),
                backend: "fake".to_owned(),
                directory,
                session: None,
                environment: None,
                instructions: None,
            });
            client.wait_for(|message| match message {
                ServerMessage::Space { state } => Some(state.id.clone()),
                _ => None,
            })
        }
    };
    println!("SPACE_ID={space}");

    // Report a presence that orbits the center and waves one hand. The
    // spectator (running separately) follows this participant; the moving
    // head drives its camera, the hand drives its skeleton.
    let duration = std::env::var("PRESENCE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    println!(
        "Reporting presence for {duration} s; a spectator should follow \"Scripted\" in space {space}."
    );
    let start = Instant::now();
    let period = std::f64::consts::PI * 2.0 / 3.0; // one orbit in 3 s
    let mut ping_deadline = Instant::now() + Duration::from_secs(4);
    let tick = Duration::from_millis(16);
    while start.elapsed() < Duration::from_secs(duration) {
        let t = start.elapsed().as_secs_f64() * period;
        let head = tomori_protocol::Pose {
            position: [t.cos() as f32, 1.65, t.sin() as f32],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let wave = (0.5 + 0.5 * (t * 2.0).sin()) as f32;
        let wrist = [
            head.position[0] + 0.3,
            head.position[1] - 0.2 + 0.15 * wave,
            head.position[2] - 0.25,
        ];
        let finger = [wrist[0] + 0.05, wrist[1] + 0.05, wrist[2] + 0.08];
        let hand = Hand {
            joints: {
                let mut joints = [None; JOINT_COUNT];
                joints[1] = Some(Joint {
                    position: wrist,
                    radius: 0.020,
                });
                joints[2] = Some(Joint {
                    position: finger,
                    radius: 0.014,
                });
                joints
            },
        };
        client.send(&ClientMessage::Presence {
            head,
            left: None,
            right: None,
            hands: Some(Box::new([Some(hand), None])),
            rig: None,
        });

        // A periodic ping keeps the client side of the connection alive too.
        if Instant::now() >= ping_deadline {
            client.send(&ClientMessage::Ping { nonce: 0 });
            ping_deadline += Duration::from_secs(4);
        }
        thread::sleep(tick);
    }

    // End the upload cleanly.
    client.stream.write_all(b"0\r\n\r\n").unwrap();
    client.stream.flush().unwrap();
    println!("done: reported presence for {duration} s");
}
