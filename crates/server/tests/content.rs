// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Content by digest over HTTP: an environment's asset read with `GET /content/DIGEST` by a client presenting its
//! credential and its session ticket, whole, in part, conditionally and by `HEAD`, with the fields that keep it
//! immutable, unsniffed and unshared; and every refusal: no ticket, another's, a credential refused, a digest not
//! offered or not written as one, a body, another method. See `design/content.md`.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;

use tomori_assets::{AssetRef, Digest, Embedded, Library};
use tomori_protocol::{ClientMessage, ContentKind, PROTOCOL_VERSION, ServerMessage};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber};
use tomori_server::{Environment, ServedAsset, Server};

/// A model of `length` bytes, unlike any other test's.
fn model(tag: &str, length: usize) -> &'static [u8] {
    let mut bytes = b"glTF".to_vec();
    bytes.extend_from_slice(tag.as_bytes());
    bytes.extend((0..length).map(|index| (index % 251) as u8));
    bytes.truncate(length);
    Box::leak(bytes.into_boxed_slice())
}

/// The garden, serving `bytes` as its lantern.
fn garden(bytes: &'static [u8]) -> Environment {
    Library::shared().add(Embedded::new().with(bytes));
    Environment::new("garden", "Garden").with_asset(
        ServedAsset::new(
            AssetRef::new("garden", 1, "lantern.glb", Digest::of(bytes)),
            ContentKind::Model,
        )
        .with_licence("by Ada, CC BY 4.0"),
    )
}

/// Say hello as a client that reads content, returning its ticket and the digests it is offered.
fn hello(client: &mut TestClient) -> (String, Vec<String>) {
    client.send(&ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        participant: "ada".to_owned(),
        name: "Ada".to_owned(),
        names: false,
        environments: None,
        environment_state: false,
        projection: false,
        pairing_window: false,
        speech_held: false,
        images: false,
        diagrams: false,
        world_edges: false,
        projection_patches: false,
        live_panels: false,
        spawns: false,
        limits: None,
        content: true,
    });
    client.wait_for(|message| match message {
        ServerMessage::Content { session, assets } => Some((
            session.clone(),
            assets.iter().map(|asset| asset.digest.clone()).collect(),
        )),
        _ => None,
    })
}

/// A response: its status, its fields, lower-cased, and its body.
struct Answer {
    status: u16,
    fields: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Send `method path` with `fields` and an optional body, and read the answer until the server closes.
fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    fields: &[(&str, &str)],
    body: &[u8],
) -> Answer {
    let mut stream = TcpStream::connect(address).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in fields {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(response[..split].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap()[9..12].parse().unwrap();
    let fields = lines
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_owned())
        })
        .collect();
    Answer {
        status,
        fields,
        body: response[split + 4..].to_vec(),
    }
}

#[test]
fn an_offered_asset_is_read_by_digest_whole_in_part_conditionally_and_by_head() {
    // Several pieces long.
    let lantern = model("an asset read over HTTP", 200_000);
    let digest = Digest::of(lantern).to_string();
    let server =
        TestServer::with_server(tomori_server::fake::FakeBackend::new(), move |backends| {
            Server::new(backends, Arc::new(TextTranscriber::default()))
                .with_environments([garden(lantern)])
        });
    let mut client = TestClient::open(server.address);
    let (ticket, offered) = hello(&mut client);
    assert_eq!(offered, std::slice::from_ref(&digest));
    let path = format!("/content/{digest}");
    let get = |fields: &[(&str, &str)]| {
        let mut with = vec![("Tomori-Session", ticket.as_str())];
        with.extend_from_slice(fields);
        request(server.address, "GET", &path, &with, b"")
    };
    let whole = get(&[]);
    assert_eq!(whole.status, 200);
    assert_eq!(whole.body, lantern);
    let etag = format!("\"{digest}\"");
    for (name, value) in [
        ("content-type", "model/gltf-binary"),
        ("content-length", "200000"),
        ("etag", etag.as_str()),
        ("cache-control", "private, max-age=31536000, immutable"),
        ("accept-ranges", "bytes"),
        ("x-content-type-options", "nosniff"),
        ("content-security-policy", "default-src 'none'; sandbox"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(whole.field(name), Some(value), "{name}");
    }
    // The rest of it, from where a transfer that stopped left off.
    let rest = get(&[("Range", "bytes=150000-")]);
    assert_eq!(rest.status, 206);
    assert_eq!(rest.body, &lantern[150_000..]);
    assert_eq!(
        rest.field("content-range"),
        Some("bytes 150000-199999/200000")
    );
    let past = get(&[("Range", "bytes=200000-")]);
    assert_eq!(
        (past.status, past.field("content-range")),
        (416, Some("bytes */200000"))
    );
    assert!(past.body.is_empty());
    // Kept already, it is not sent again.
    let kept = get(&[("If-None-Match", &format!("\"other\", {etag}"))]);
    assert_eq!(
        (kept.status, kept.field("etag")),
        (304, Some(etag.as_str()))
    );
    assert!(kept.body.is_empty());
    assert_eq!(get(&[("If-None-Match", "*")]).status, 304);
    let head = request(
        server.address,
        "HEAD",
        &path,
        &[("Tomori-Session", &ticket)],
        b"",
    );
    assert_eq!(
        (head.status, head.field("content-length")),
        (200, Some("200000"))
    );
    assert!(head.body.is_empty());
}

#[test]
fn a_content_request_is_refused_without_its_ticket_its_credential_or_an_offered_digest() {
    let lantern = model("an asset refused over HTTP", 100);
    let digest = Digest::of(lantern).to_string();
    let server = TestServer::with_tokens(false, move |server| {
        server.with_environments([garden(lantern)])
    });
    let (_, token) = server.issue("Ada", tomori_security::Grant::Owner);
    let bearer = format!("Bearer {token}");
    let mut client = TestClient::open_with(server.address, &[("Authorization", &bearer)]);
    let (ticket, _) = hello(&mut client);
    let path = format!("/content/{digest}");
    let status = |method: &str, path: &str, fields: &[(&str, &str)], body: &[u8]| {
        request(server.address, method, path, fields, body).status
    };
    let both = [
        ("Authorization", bearer.as_str()),
        ("Tomori-Session", ticket.as_str()),
    ];
    assert_eq!(status("GET", &path, &both, b""), 200);
    // Without the credential, or the ticket, or with another ticket, alike.
    assert_eq!(status("GET", &path, &both[1..], b""), 401);
    assert_eq!(status("GET", &path, &both[..1], b""), 401);
    let other = [
        ("Authorization", bearer.as_str()),
        ("Tomori-Session", "other"),
    ];
    assert_eq!(status("GET", &path, &other, b""), 401);
    let refused = request(server.address, "GET", &path, &other, b"");
    assert_eq!(refused.field("www-authenticate"), Some("Bearer"));
    // A digest not offered, written otherwise, or not a digest, is not found, alike.
    for missing in [
        format!("/content/{}", Digest::of(b"never kept")),
        format!("/content/{}", digest.to_uppercase()),
        "/content/lantern.glb".to_owned(),
        "/content/".to_owned(),
    ] {
        assert_eq!(status("GET", &missing, &both, b""), 404, "{missing}");
    }
    // A request with a body, or of another method, is refused.
    assert_eq!(status("GET", &path, &both, b"a body"), 400);
    assert_eq!(status("POST", &path, &both, b""), 405);
}
