// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A raw HTTP/1.1 peer that sends a scripted response, for server behaviour the mini server does not produce.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc::{self, Receiver};

use tomori_protocol::{Frame, FrameKind, ServerMessage};

/// Encode `data` as one HTTP/1.1 chunk.
pub fn chunk(data: &[u8]) -> Vec<u8> {
    let mut output = format!("{:x}\r\n", data.len()).into_bytes();
    output.extend_from_slice(data);
    output.extend_from_slice(b"\r\n");
    output
}

pub fn message(message: &ServerMessage) -> Vec<u8> {
    chunk(&Frame::server(message).encode())
}

pub fn welcome() -> Vec<u8> {
    welcome_limiting(None)
}

/// A Welcome that limits utterances to `utterance_limit` seconds, if given.
pub fn welcome_limiting(utterance_limit: Option<u32>) -> Vec<u8> {
    message(&welcome_message(utterance_limit))
}

/// A Welcome's frame, outside any chunk.
pub fn welcome_frame() -> Vec<u8> {
    Frame::server(&welcome_message(None)).encode()
}

fn welcome_message(utterance_limit: Option<u32>) -> ServerMessage {
    ServerMessage::Welcome {
        protocol: tomori_protocol::PROTOCOL_VERSION,
        server: "mini".into(),
        source: 1,
        role: tomori_protocol::ConnectionRole::Owner,
        backends: vec![],
        roots: vec![],
        environments: vec![],
        utterance_limit,
        name: Some("Ada Lovelace".into()),
        limits: None,
    }
}

/// A control frame whose payload is `payload`, which need not be a valid message.
pub fn control(payload: &[u8]) -> Vec<u8> {
    chunk(
        &Frame {
            kind: FrameKind::Control,
            payload: payload.to_vec(),
        }
        .encode(),
    )
}

/// Accept connections forever. For each, read the request head and, if `body` is given, respond 200 with it as a chunked body; then hold the connection open until the client closes it.
pub fn start(body: Option<Vec<u8>>) -> SocketAddr {
    start_recording(body).0
}

/// Like [`start`], also sending the head of each request received.
pub fn start_recording(body: Option<Vec<u8>>) -> (SocketAddr, Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (heads, received) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let body = body.clone();
            let heads = heads.clone();
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let _ = heads.send(head);
                if let Some(body) = body {
                    let mut response =
                        b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
                    response.extend(body);
                    if stream.write_all(&response).is_err() {
                        return;
                    }
                }
                let mut sink = [0; 4096];
                while stream.read(&mut sink).unwrap_or(0) > 0 {}
            });
        }
    });
    (address, received)
}

/// Accept one connection, read its request head and respond 200 with `body` as a chunked body, then read nothing more. Once `reset` says so, close the connection with what the client sent unread, which resets it.
pub fn start_resetting(body: Vec<u8>, reset: Receiver<()>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let mut response = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
        response.extend(body);
        stream.write_all(&response).unwrap();
        let _ = reset.recv();
        drop(stream);
    });
    address
}

/// Accept connections forever. For each, read the request head and respond 200 with `body`, delimited by closing the connection, which follows at once.
pub fn start_closing(body: Vec<u8>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut head = Vec::new();
            let mut byte = [0; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let mut response = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
            response.extend(&body);
            stream.write_all(&response).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
        }
    });
    address
}
