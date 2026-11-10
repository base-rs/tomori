# Proposal: Channels, a Content Store and Live Streams

## Status

**Accepted,** 10 November 2026. The maintainer accepted the proposal as written ("That proposal sounds good to me"), so its recommendations stand, including the proposed answers to the [open questions](#open-questions-for-the-maintainer); questions without a proposed answer are left to the stage that needs them. Stage 0, moving to `base-http` 0.4, is [#285](https://github.com/ioquatix/tomori/pull/285); nothing else is built yet. As each stage is built, the parts it adopts become requirements in `design/server.md`, `projection.md`, `security.md`, `speech.md`, `live-audio.md` and `environments.md`, as listed under [Changes to the design documents](#changes-to-the-design-documents), and those documents then take precedence over this one. The rest of this document is the proposal as it was put to the maintainer, from [PR #283](https://github.com/ioquatix/tomori/pull/283).

It builds on these:

- the protocol scaling audit of `34d8ab56` (#275);
- projection revisions, patches and numeric `Limits` in `Hello` and `Welcome` (#281, #282, merging as this was written);
- the negotiated 32 MiB frame limit (in progress);
- image panels by content hash (#239, #240);
- environment text and live chart panels (#267, open).

## The request

The maintainer's direction, spoken from the headset, November 2026:

- **Voice goes over a separate connection.** A **command channel** carries control, state and patches, and a **data channel** carries audio and the like, so that big state never stalls speech.
- **Images and raw code, if big, go through a separate content-addressable store API.** Content is hashed and reachable at an HTTP endpoint. Synchronisation says only "this is the checksum of this content, go and load it". The endpoint returns the content type, which drives how the content is presented in the world.
- **That decoupling enables streaming and incremental loading.** Once state stops carrying every piece of content itself, a client can load a low-quality preview, then the high-quality version, and more.
- **The future** holds much more content (images), **real-time feeds** (video, and audio streams that aren't the chat), and **real-time diagrams generated on the server**, perhaps as Motion JPEG or similar. An open streaming mechanism enables all of that.

Constraints the maintainer added:

- **Everything starts on HTTP/1.1,** served by the maintainer's own `base-http` (`base-http-server`, `base-http-client`).
- **HTTP/2 and HTTP/3 are coming to `base-http` within weeks.** Channels must work over HTTP/1.1 now and upgrade transparently later, with no protocol change.
- **No other stack.** Tomori stays on `base-http`. This proposal states what Tomori needs from it, and the gaps.
- **A web client.** Since the content store and streams are plain HTTP, a browser client could reach them. The proposal should say how ready that is.

## Recommendation in brief

- **A channel is an HTTP exchange with a role, not a socket.** Tomori defines four kinds:
  - **control:** `POST /connect`, as today;
  - **media:** `POST /media`, long-lived, for speech and microphone audio;
  - **content:** `GET /content/{hash}`, plain, cacheable requests;
  - **streams:** `GET /stream/{id}`, long-lived responses.

  Each is framed exactly as `/connect` is, or is plain HTTP. On HTTP/1.1 each exchange gets its own TCP and TLS connection from the client's pool. On HTTP/2 each is a stream of one connection, and on HTTP/3 a QUIC stream. The bytes of each exchange are the same in all three, so upgrading is a matter of ALPN negotiation, not a protocol change.
- **Speech moves to the media channel together with everything that must stay in order with it.** That is audio frames in both directions, the `Speech` announcements, `Gate`, and `UtteranceBegin`, `UtteranceEnd` and `UtteranceCancel`. Everything else stays on control. A **session ticket** binds the extra channels to the control connection. It is minted when `/connect` is accepted and carried in a header or cookie, never in a URL.
- **One content store, addressed by SHA-256, readable only over `GET /content/{hash}`.** It generalises today's image store:
  - Only the server writes to it.
  - Responses are immutable, with `ETag` set to the hash and single byte ranges supported.
  - A request is allowed only for content the requester's current space references. Anything else is answered 404, the same as missing content.
  - The response's `Content-Type` decides how the content is decoded and presented. The reference in the projection carries only what layout needs before the content arrives.
  - Images move first, then code bodies (headsets finally get whole code panels, not only the short form), then long text bodies and environment assets (#194).
- **Variants are separate hashes.** A reference carries an inline placeholder of a few dozen bytes, an optional preview reference, and the full reference, so every hash still names exactly one byte string.
- **Live streams are picture streams first:** `multipart/x-mixed-replace` (Motion JPEG and the like, in a format browsers also render) over a long-lived `GET`.
  - The newest picture wins: at most one picture waits per subscriber.
  - A client subscribes while the stream's panel is in view.
  - **Native data beats pictures for #267's charts.** Its charts are sharper and far cheaper drawn natively from data with panel patches. A picture stream of a chart is still useful, though, for clients that cannot draw the chart spec: the web client, the spectator and older headsets.
  - Audio feeds and video (H.264/H.265 through MediaCodec on the Quest) come later.
- **Old clients keep everything on `/connect`.** New clients advertise `content` and `channels` in `Hello`. `PROTOCOL_VERSION` does not change.
- **Staged plan:**
  0. move to `base-http` 0.4;
  1. the content store for images and code;
  2. speech on the media channel;
  3. previews;
  4. picture streams;
  5. audio and video feeds;
  6. interest management.

  HTTP/2 and HTTP/3 are cross-cutting upgrades that Tomori switches on as `base-http` ships them.

## 1. The problem

Everything below is from `origin/main` at `d7eb3ef1`, with the audit's measurements.

### One connection carries everything

Each client keeps one `POST /connect`, HTTP/1.1 chunked in both directions. It carries every frame kind: JSON control, binary `Audio` frames (a 24-byte header and PCM) and `Image` chunks of up to 32 KiB. The server's outbox (`crates/server/src/outbox.rs`) sends in this order:

1. keepalive;
2. control;
3. agent speech (announcements and frames, in order);
4. participant audio;
5. image chunks.

The ordering applies only to what has not yet been written. **A frame that has started cannot be pre-empted**, and bytes already in the kernel's send buffer go first whatever comes next.

### Head-of-line blocking of speech

- **Speech has about 100 ms of slack.** Agent speech is sent at its playback rate, only `LEAD` = 100 ms ahead (`crates/server/src/retention.rs`). Any control frame that takes longer than about 100 ms to cross the link empties the client's speech buffer, which is heard as a gap.

| Frame | 50 Mbit/s | 100 Mbit/s | Heard? |
|---|---|---|---|
| typical projection, 16 KB | 2.6 ms | 1.3 ms | no |
| today's code-heavy maximum, 518 KiB | 85 ms | 42 ms | at the edge |
| a full 1 MiB frame | 168 ms | 84 ms | yes, at 50 Mbit/s |
| a 32 MiB frame (the negotiated limit in progress) | 5.4 s | 2.7 s | yes: seconds of silence |
| one image chunk, 32 KiB | 5 ms | 2.6 ms | no |

- **Patches help but don't fix it.**
  - #281/#282 make the common case small: about 400 bytes per change instead of 4 KB to 600 KB.
  - Joining, resynchronising and environment changes still send whole projections, as do older clients at every change.
  - Raising the frame limit to 32 MiB makes the worst case 32 times worse unless speech leaves the connection.
- **Image chunks ahead of speech.** Image chunks go after speech in the outbox. While nothing else waits, though, the connection writes chunk after chunk, and what the kernel has buffered but not sent can sit ahead of the next speech frame. This has not been measured. Separating the media channel removes the question, and on a socket that still mixes traffic, `TCP_NOTSENT_LOWAT` would bound it.
- **TCP itself.** One lost segment on Wi-Fi stalls everything behind it on that connection until it is retransmitted. A connection for media alone limits that to audio. Only QUIC removes it for audio (see [section 2](#transport-now-and-later)).

### Content inlined in state

- **Code panels are sent short:** 16 lines, 720 characters and 256 spans of a body of up to 200 lines. The whole body is about 50 KB, 80% of it spans, and 90 whole bodies would be 4.4 MiB, so it never reaches a headset.
- **`spaces.json` holds code spans pretty-printed:** 134 KB per full code panel on disk, up to about 16 MB per space.
- **#267's chart updates** re-send the whole projection to every headset, about 9 projections a second per client at today's caps. Patches reduce each update to one `panel` op of about 10 KB.
- **Images are already done right** (#239): by hash, fetched on request and cached across reconnects. They still share the one connection, and travel as a Tomori-specific frame kind that no other HTTP tool can fetch.

### No streaming

Nothing carries server-generated real-time content: a diagram that redraws as a build runs, a camera, a remote screen, or an audio feed other than the agent and the participants. The only paths are whole projections and patches, and the environment channel's 64 KiB states and 4 KiB events.

## 2. Channels

### What a channel is

A channel is **one HTTP exchange with a role.** Tomori's protocol defines what each role carries, and never which TCP connection it travels on. That is what makes HTTP/2 and HTTP/3 a transparent upgrade.

| Channel | Exchange | Body | Carries | Lifetime | `Priority` (RFC 9218) |
|---|---|---|---|---|---|
| control | `POST /connect` | Tomori frames, both ways | Hello and Welcome, space state, projection snapshots and patches, environment state and events, intents, presence, transcripts, errors, images for older clients | the session | `u=2` |
| media | `POST /media` | Tomori frames, both ways | audio frames both ways; `Speech`; `Gate`; `UtteranceBegin`, `UtteranceEnd`, `UtteranceCancel`; `MediaEpoch` (below) | the session; reconnects on its own | `u=0` |
| content | `GET /content/{hash}`, `HEAD` | the content's bytes | images, code bodies, long text, environment assets | one request | `u=4, i` |
| stream | `GET /stream/{id}` | `multipart/x-mixed-replace`, later other formats | live pictures, later audio and video feeds | while subscribed | `u=3` |

- **The `Priority` field from day one.** Tomori sends it on every request. HTTP/1.1 ignores it, and once HTTP/2 or HTTP/3 arrive the driver schedules streams by it with no protocol change.
- **Media, content and streams are in addition to `/connect`, never instead of it.** A client that opens none of them works exactly as today.

### What moves to the media channel, and why those messages

Some messages must stay in order with the audio they describe, so they travel on the same channel as it:

- **`Speech`** announces the segment whose frames follow (`ServerMessage::Speech`, "the agent speech audio frames that follow"). The outbox's agent lane already keeps announcements and frames in one queue (`outbox.rs`, `enum Agent`), so the lane moves whole.
- **`UtteranceBegin`, `UtteranceEnd` and `UtteranceCancel`** bracket the `agent` frames of an utterance. `speech.md` relies on "frames and control share one ordered stream": the end arrives with the last of the audio, and a gap fails the utterance visibly.
- **`Gate`** "applies to subsequent audio frames".

Everything else stays on control, including **presence**. It is small and coalesced, and moving it gains nothing until it can travel as unreliable datagrams (HTTP/3, stage 5).

**`MediaEpoch`.** Joining and leaving a space are control messages, but audio for the old space may still be in flight on media. So the server sends `MediaEpoch { space, epoch }` on media whenever the participant's space changes, and puts the same epoch in the space state on control. A client plays media frames only once the epochs of the two channels agree. That replaces the ordering one stream gave for free.

**Switching lanes at segment boundaries.** The server moves a session's speech lane from control to media only between segments, after a `Speech` segment's last frame, and moves it back the same way. Frames keep their source and sequence numbers, so the jitter buffer joins them across the switch. A client that said `channels` reads speech from either channel.

### Opening and binding

1. **`Hello`** gains `channels: ["media"]`, listing the channels the client opens.
2. **The session ticket.** When the server accepts `/connect`, it mints a **session ticket**: 256 random bits that name this control connection. It returns the ticket three ways:
   - in the response head, as `Tomori-Session: TICKET`;
   - as a cookie, `Set-Cookie: tomori-session=TICKET; Secure; HttpOnly; SameSite=Strict; Path=/`, for browsers (see [section 6](#6-web-client-readiness));
   - in `Welcome`, with the channels the server offers: `channels: { media: true }`, `content: true`, `streams: true`.
3. **The client opens `POST /media`** with the same credential it gave `/connect` (`Authorization: Bearer` or the token cookie) and the ticket, in `Tomori-Session` or the session cookie. The server accepts it only when all of these hold:
   - the ticket names a live control connection;
   - the request's principal is that connection's (a visitor's control connection has none, and the ticket alone binds);
   - the session has no media channel yet. A second one replaces the first, as a reconnect does.

   Anything else gets the same 401 reply and is logged, as `security.md` refuses credentials today.
4. **The ticket lives exactly as long as its control connection.** It is compared in constant time, never logged, never put in a URL, and never the same twice: a new control connection is a new ticket. It is a bearer secret given only over TLS, in a response to the device that made the connection.
5. **Content and stream requests** carry the ticket the same way, so the server knows whose space they ask about (see [section 3](#3-the-content-store)).

TLS channel binding (RFC 9266 `tls-exporter`) would tie the channels to one TLS session, but over HTTP/1.1 each channel is a separate TLS session. Under HTTP/2 and HTTP/3 all channels share one connection, and the ticket can then be checked against the connection too, as a defence in depth.

### Liveness and reconnecting

- **Each long-lived channel runs `connection-liveness.md` on its own:** `ping` and `pong`, the 5 s interval and the 15 s timeout. Any data received counts as liveness, so a steady audio stream needs no keepalive. Content and stream requests use the request timeout and stream deadlines instead.
- **Control is the session.** When control ends, the server ends that session's media channel and streams, and releases the participant as today. The client reconnects control with its usual backoff, then reopens the other channels with the new ticket.
- **Media can fail alone.** When media ends while control lives:
  1. the server moves the speech lane back to control at the next segment boundary, so the agent stays audible;
  2. an open push-to-talk utterance fails visibly, as a gap does today;
  3. the client reopens media with the same ticket and backoff, and the lane moves back.

  Whether the live microphone may resume on the reopened media channel or stops, as on a disconnection, is an [open question](#open-questions-for-the-maintainer). `live-audio.md` binds a capture to its connection, and the proposal is to bind it to the session instead, which the ticket names.

### Transport, now and later

| Transport | Channels as | Speech isolation | Status in `base-http` | Role in this proposal |
|---|---|---|---|---|
| HTTP/1.1, one TLS connection per exchange | separate TCP connections from the client's pool | full: media has its own TCP connection and congestion window | implemented; Tomori uses it today | **now** |
| HTTP/2 over TLS (ALPN `h2`) | streams of one TCP connection | at the application level only: a lost TCP segment still stalls all streams, and stream scheduling must favour media | planned, milestone M6, within weeks | **transparent upgrade** for control, content and streams; media too, if measured fine on Wi-Fi |
| HTTP/3 over QUIC (ALPN `h3`) | QUIC streams, plus HTTP datagrams (RFC 9297) for audio later | full, plus unreliable delivery for live audio | planned: native `base-quic` and `base-http3` | **the end state,** with TCP kept as a fallback where UDP is blocked |
| WebSocket | one message per Tomori frame | as HTTP/1.1 | upgrades exist; WebSocket framing is a follow-up | only for a browser client (section 6) |
| WebTransport | sessions over HTTP/3 | as HTTP/3 | not planned | perhaps, for browsers, after HTTP/3 |

**Recommendation.**

- **Now:** design and build the channels on HTTP/1.1, which `base-http` serves today.
- **When `base-http` ships HTTP/2:** Tomori turns on `h2` in ALPN on both ends and changes nothing else.
  - Control, content and streams then share one TLS connection: fewer handshakes, and one pinned certificate check.
  - **Whether media joins them is a client connection policy, not a protocol question.** The client's pool may keep media on a connection of its own, over HTTP/1.1 or a second HTTP/2 connection, until measurements on the Quest's Wi-Fi show that TCP head-of-line blocking under loss doesn't hurt speech.
- **When HTTP/3 arrives:** every channel moves to QUIC streams.
  - Later, live audio moves to HTTP datagrams, Opus-encoded with in-band forward error correction, as `server.md` already plans for UDP.
  - That is an additive capability (`Hello::datagrams`). TCP stays as the fallback.
  - Discovery adds the UDP port to the TXT record.

### Server resources

- **Participants open more connections.** Today a participant is one connection. With channels over HTTP/1.1, a participant is up to 2 long-lived connections, plus content requests (at most 4 at once, which the client's pool reuses with keep-alive) and streams (at most 4).
- **Channels count against their session, not against the server's slots as separate clients.** The server bounds the channels each session holds:
  - one media channel;
  - 4 concurrent content requests;
  - 4 streams.
- **Visitors count too.** The 64-visitor limit stays, and each visitor's channels count against its session. Requests beyond these bounds get 503 before they reach the hub.
- **The server's connection limit must grow to match,** or be applied per principal and session rather than per socket. Under HTTP/2 and HTTP/3 this mostly disappears.

## 3. The content store

### The endpoint

**`GET /content/{hash}`** (and `HEAD`). The hash is SHA-256, 64 lower-case hexadecimal digits, as `Panel::image` names images today.

| Response field | Value |
|---|---|
| status | 200, or 206 for a range; 304 for `If-None-Match` with the hash; 404 for anything not allowed or not kept, alike; 416 for an unsatisfiable range |
| `Content-Type` | the type the server stored with the content, from a fixed allow-list (below). **It decides how a client decodes and presents it.** |
| `Content-Length` | always known |
| `ETag` | `"HASH"`, a strong validator, since the hash is the content |
| `Cache-Control` | `private, max-age=31536000, immutable`: authorised content is never cached by a shared cache, and never revalidated by its owner |
| `Accept-Ranges` | `bytes`. One range per request; a request for several ranges is answered whole (200) |
| `X-Content-Type-Options` | `nosniff` |
| `Content-Security-Policy` | `default-src 'none'; sandbox` |
| `Cross-Origin-Resource-Policy` | `same-origin` |

**Requests carry** the credential and the session ticket, as media does. Under HTTP/1.1 the client keeps one or two keep-alive connections for content beside the long-lived channels, so content never shares a socket with speech.

### Authorisation

A request is allowed only when **the requester's current space references the hash**:

- **its projection,** any panel in the environment the session presents, including previews;
- **its environment's state.** The server doesn't read environment payloads, so a side declares the hashes its state uses: `Space::share` gains a list of content references, which the server checks and keeps with the state;
- **the assets of the environments the server offers that client** (#194), which every client of any role may fetch, since they are part of the environments themselves.

Anything else, whether another space's content, content from before a panel was dismissed, or content never kept, is **404, identical to missing content.** That gives no oracle for whether a hash exists. Without this check, someone could confirm a guess at small content: they would hash a candidate, such as a likely line of code, and see whether the server has it.

**Visitors read what participants read,** since they see the space's panels; `security.md`'s rule "a connection may fetch only an image a panel in its own space shows" carries over unchanged.

**Capability URLs** are links that grant access by themselves, such as signed and expiring ones. They are **not proposed.** Native clients set headers, and browsers send the session cookie on same-origin requests, including `<img>` and Motion JPEG. A capability in a URL would end up in logs, histories and screenshots, which is exactly what "tokens never in URLs" avoids. They stay an option for a case neither covers, listed under [open questions](#open-questions-for-the-maintainer).

### Writes: from the server only

Clients never upload. The server's own code puts content into the store:

```rust
impl ContentStore {
    /// Stores bytes the server made, of a type from the allow-list, and returns their reference.
    fn put(&self, kind: ContentKind, bytes: Vec<u8>) -> Result<ContentRef, StoreError>;
}
```

| Kind | `Content-Type` | Made by | Bound |
|---|---|---|---|
| image | `image/png` (8-bit RGB, no metadata) | `show_image`, as today | 8 MiB, 2,048 px |
| image preview | `image/jpeg` | the same image thread, from the cleaned image | 64 KiB, 256 px |
| code body | `application/vnd.tomori.code+json` (text and spans) | code panels, highlighted outside the hub lock as today | 200 lines, 8,000 characters and 2,048 spans, as `CODE_*` |
| long text | `text/markdown; charset=utf-8` | text panels whose body grows past the inline bound (later) | 64 KiB |
| environment asset | `model/gltf-binary`, `image/png` | an environment's server side, by `AssetRef` digest (#194) | `tomori-assets`' `Limits`: 16 MiB |

On disk the store is the state directory's `content/` folder, readable only by the server's user. Each file is named by its hash, with its type kept beside it. The existing `images/HASH.png` files are read as content of type `image/png`, so nothing is lost on upgrading. The latest 64 MB stay in memory, as images do now. Every file is checked against its name when it is read.

### Garbage collection by reference

As images are today, content is kept while anything references it:

- a kept panel, in any space or environment;
- an environment state that a side declared it in;
- an offered environment's asset.

It is removed once it is unreferenced and older than a minute, as the kept spaces are written. The minute keeps content just made, for a panel not yet projected, from being removed under it.

### Variants: a hash per variant

A reference names each variant by its own hash:

```rust
pub struct ContentRef {
    pub hash: Hash,            // the full content
    pub length: u32,
    pub kind: ContentKind,     // a layout hint only: image, code, text, asset
    pub width: Option<u16>,    // images: to lay the face out before anything arrives
    pub height: Option<u16>,
    pub preview: Option<Box<ContentRef>>,   // a smaller variant, its own hash
    pub placeholder: Option<String>,        // inline, ~30 bytes: a ThumbHash of an image
}
```

- **The ladder:**
  1. the inline placeholder: a blurred colour field drawn at once, at no request;
  2. the preview, at most 256 px and about 10–20 KB of JPEG;
  3. the full content.

  More levels, such as larger images or tiles, are more references, not a change of model.
- **Clients choose.** A headset loads previews for panels beyond reading distance, or when many images arrive at once (joining a full space), and the full content when a panel is near, floating or carried. It keeps the preview until the full content is decoded.
- **Why not `?variant=preview`.** It would make a hash name a family of byte strings rather than one. Each would then need its own `ETag`, the server would need a variant table, and caches would need to vary. A hash per variant keeps "hash = bytes" true everywhere, and costs one extra hash in the reference.
- **Code's short form is its preview.** The projection's inline short code, 16 lines, 720 characters and 256 spans, is exactly a preview. With the store, a code panel carries that inline as today, plus a reference to the whole body, which a client fetches when the panel is read up close or grown.

### What moves to the store

| Content | Today | With the store | Who gains |
|---|---|---|---|
| images | `FetchImage` and `Image` frames on `/connect` | `GET /content/{hash}`; frames remain for older clients | speech; caching by any HTTP client; previews |
| code bodies | short form inline, whole body never sent | short inline, whole by hash | headsets get whole code panels; projections stay small |
| long text bodies | 1,200 characters inline | inline up to a bound; beyond it by hash (later) | room for documents, logs and reports |
| environment assets | bundled in the build | by `AssetRef` digest, cached (#194) | headsets present environments they don't bundle |
| `spaces.json` | holds code spans, pretty-printed | holds references; bodies kept once in the store | persistence shrinks from up to ~16 MB per space to references |
| chart data (#267) | inline in the panel | **stays inline**, as patches | small, changes often, never worth a round trip |

**The threshold for inline content.** A body of a few hundred bytes costs more as a request than inline. Content goes to the store only above a bound per kind, such as 4 KiB of text or a code body beyond its short form.

### How a client uses the response's type

The maintainer's direction is that the endpoint's content type drives presentation. Clients do this:

- **The response's `Content-Type` chooses the decoder.** The reference's `kind`, width and height only lay the panel out before the content arrives.
- **The type is checked against the panel.** An image panel takes only `image/png` or `image/jpeg`, and a code panel only the code type. A response whose type doesn't belong where the reference was used is refused and logged, never decoded as something else.
- **The bytes are checked against the hash before decoding,** as images are today. The exception is a ranged read, which is checked once it is whole.

## 4. Live streams

### The model

A **stream** is a source the server owns, named by an identifier unique within the space, with a type, a size and a rate. A panel references it, as an image panel references an image:

```rust
pub struct StreamRef {
    pub id: String,
    pub kind: StreamKind,          // pictures; later audio, video
    pub width: u16, pub height: u16,
    pub rate: f32,                 // pictures per second, at most
    pub poster: Option<ContentRef>,// shown until the first picture, and when not subscribed
}
```

- **A client subscribes** with `GET /stream/{id}` while the panel is in view, and closes the request when it isn't. The subscription is the interest; the projection doesn't change.
- **Producers:**
  - an environment's server side, through `tomori-environment`'s `Space::stream(id, kind)`, which returns a sink it pushes encoded pictures to;
  - later, the agent, through a tool (`show_stream`) for something the server renders, such as a running build's graph, a terminal or a Blender viewport.
- **Authorisation** is as for content: the session's current space must reference the stream.

### The first format: pictures, as `multipart/x-mixed-replace`

```http
HTTP/1.1 200 OK
Content-Type: multipart/x-mixed-replace; boundary=tomori
Cache-Control: no-store
X-Content-Type-Options: nosniff

--tomori
Content-Type: image/jpeg
Content-Length: 41022
Tomori-Picture: 1287; t=1762790400.250

<JPEG bytes>
--tomori
...
```

- **One format serves native clients and browsers.** It is Motion JPEG as browsers render it in an `<img>`. Native clients parse it strictly, so every part must have a `Content-Length`, which is bounded by the stream's size, and the boundary is fixed.
- **Picture types:**
  - JPEG for photographic content, such as camera feeds and renders;
  - PNG for diagrams with thin lines and text, where JPEG rings;
  - WebP, which does both and is about a third smaller, only if the client's decoder list grows to include it.

  Every part's type must be one the client allows.
- **Each part carries its sequence number and time.** A client shows the newest picture and can measure lag.

### Rate control and back-pressure

- **Encode once.** The server encodes each picture once per stream and shares the bytes (`Arc<[u8]>`) with every subscriber.
- **The newest picture wins.** Each subscriber holds at most one picture waiting. A newer picture replaces it, as presence and partial transcripts already replace their predecessors in the outbox. A slow headset therefore sees fewer pictures, never older ones, and the server's memory per subscriber is one picture.
- **Rates are bounded by the source.** The rate it declares is its most. The client may ask for less, with `GET /stream/{id}?rate=2`. That is not a secret, so a query parameter is fine.
- **Server bounds:**
  - 4 streams per session;
  - 30 pictures a second;
  - a size per picture from the stream's declared dimensions;
  - total bytes a second per session.

  A stream request beyond these gets 503.
- **Liveness.** A subscriber whose write has made no progress for the keepalive timeout is closed, as `connection-liveness.md` already closes a stalled connection.

### On the headset

- **Decoding.** Pictures are decoded on a renderer worker thread, as image panels' PNGs are, never on the frame thread, with the image crate's pure Rust decoders.
- **One texture per stream, updated in place.**
  - It is allocated once and rewritten with each picture. Each update is a texture write, not a new texture with mipmaps; the mip chain is regenerated on the GPU, or a stream panel is drawn without mipmaps at a size close to its native resolution.
  - Updates are bounded per frame: at most one stream texture update a frame, round-robin. Even so, four streams at 10 pictures a second fit easily at 72 Hz.
  - A 1024×576 picture is 2.4 MB of RGBA per update, well within what the Quest uploads.
- **Video (stage 5)** is H.264 or H.265, decoded by Android's MediaCodec in hardware.
  - Pictures come out as `AHardwareBuffer`s, and the costly part is importing them into the Vulkan renderer without a copy: `VK_ANDROID_external_memory_android_hardware_buffer`, and `wgpu`'s lower-level `hal` interface, since `wgpu` has no external-memory import.
  - A copying fallback, MediaCodec into a CPU buffer and then a texture upload, works first, at a cost of a few milliseconds per picture.
  - The desktop decodes in software or with its platform's decoder. The choice is left to stage 5.

### #267's charts

#267 draws charts **natively** from a bounded chart spec, inside the panel's mesh, sharp at any distance and on curved and tilted walls. With #281's patches, an update is one `panel` op of about 10 KB, and a series append could be about 0.2 KB. A Motion JPEG of the same chart would cost about 30–60 KB per picture, and would blur at a distance or up close. **So charts should keep their data path on headsets.**

A picture stream of a chart is still the right first consumer of streams:

- **It is the server rendering what some clients cannot.** The web client, the spectator and older headsets that don't know the chart spec can show a chart as a stream.
- **It exercises the whole mechanism:**
  - a source in an environment's server side;
  - encoding once;
  - newest-wins back-pressure;
  - subscription while in view;
  - texture updates;
  - the browser's `<img>`.
- **It is easy to verify.** The same chart drawn natively and as a stream can be compared side by side.

Diagrams only the server can draw come next: a dependency graph laid out with Graphviz, a profiler's flame graph, a viewport render.

## 5. Security

- **TLS and pinning, unchanged.**
  - Every channel, content request and stream goes to the server the control connection reached, over TLS, through the same pinned certificate check (`security.md`).
  - References are hashes and identifiers, **never URLs.** A message can never point a client at another host, and the client never fetches content from anywhere but its server.
  - In `--auth open` mode, plain HTTP remains allowed, as for `/connect`.
- **Tokens never in URLs.**
  - Credentials travel in `Authorization` or the token cookie.
  - The session ticket travels in `Tomori-Session` or the session cookie.
  - Query parameters carry only what isn't secret, such as a stream's rate.
  - The status page and the server's log never show a ticket.
- **No leakage between spaces.**
  - Content and streams are allowed only by the current space's references, and everything else is an identical 404.
  - A ticket names one control connection, whose space the server knows. Leaving a space withdraws its content and streams at once: open streams are closed and further requests answered 404.
- **Visitors** read the content and streams of the space they watch, as they see its panels. They open media to hear speech, but the server neither relays nor transcribes audio from a visitor's media channel, exactly as on `/connect` today.
- **DoS limits.**
  - Per session: one media channel, 4 concurrent content requests and 4 streams.
  - Ranges are single.
  - `GET` requests have no body, and one with a body is refused.
  - The request timeout applies to content and stream requests until their response begins.
  - Picture sizes are bounded by their stream's declared size.
  - Requests beyond these limits are refused with 503 before they reach the hub. Channels count against their session and the visitor limit (see [section 2](#server-resources)).
- **Content types are the server's, from a fixed allow-list.**
  - The server stores a type with each piece of content it made, and never takes a type from a file's name or from a client.
  - It never serves `text/html` or `image/svg+xml` from the store. The owner's status-page cookie lives on the same origin, so content a browser renders as a document would be a cross-site scripting hole.
  - `nosniff`, a sandboxing `Content-Security-Policy` and `Cross-Origin-Resource-Policy: same-origin` go on every content and stream response.
- **Clients decode only allowed types,** as #239 re-encodes images today:
  - PNG and JPEG that the server made, from pictures it decoded and cleaned of metadata, so previews carry no EXIF either;
  - code JSON checked by `valid_spans` and the code bounds;
  - text as bounded UTF-8;
  - environment assets through `tomori-assets`' untrusted reader.

  Content is checked against its hash before it is decoded. Stream pictures have no hash: they are bounded by the stream's declared dimensions and bytes, and decoded by pure Rust decoders. Hardware video decoders are a larger attack surface. They are fed only streams the server encoded itself, and are stage 5's to review.
- **Environment states declare their content,** so a side cannot make the server serve something its space doesn't show.

## 6. Web client readiness

A web client is a browser user interface instead of VR. It could be a dashboard of a space's panels, a visitor's view, or a way to answer the agent from a laptop. Because content and streams are plain HTTP, it gets most of what it needs for free.

- **Same origin, no CORS.** The web client would be served by the Tomori server itself, such as at `/app/`. Every request is then same-origin, and the server sends no `Access-Control-Allow-*` fields at all, so other sites cannot read its responses. Any request that opens a channel with a cookie must also carry an `Origin` equal to the server's own, as the status page's forms already require. That defends against cross-site request forgery and against another site opening a channel with the user's cookie.
- **Cookies for authentication.**
  - The sign-in page already sets a `Secure`, `HttpOnly`, `SameSite=Strict` token cookie, and `/connect` already accepts it.
  - The session cookie from `/connect` then authorises `<img src="/content/HASH">` and `<img src="/stream/ID">` with no script involved.
  - Visitors need no token, only the session cookie.
- **The certificate.** Browsers warn about the self-signed certificate once, as they do for the status page today, and cannot pin it. The TXT record's fingerprint can be compared by hand.
- **What a browser renders directly:**

| Content | In a browser |
|---|---|
| image (`image/png`), preview (`image/jpeg`) | `<img>` |
| picture streams (`multipart/x-mixed-replace`) | `<img>`, natively |
| text panels (`text/markdown`) | rendered to HTML in the page by a sanitising Markdown renderer; the store itself never serves HTML |
| code (`application/vnd.tomori.code+json`) | `<pre>` with spans as classes |
| environment assets (`model/gltf-binary`) | a WebGL or WebGPU viewer, later |
| agent speech and participant audio | Web Audio: PCM frames through an `AudioWorklet`; Opus through WebCodecs once audio is Opus-encoded |
| video (stage 5) | WebCodecs or Media Source Extensions |

- **What a browser needs from the control channel:** Hello and Welcome, space state, projection snapshots and patches, transcripts, and the intents it offers, such as placing and dismissing panels, ticking items, deciding approvals and answering questions. Environment state matters only to a client that presents environments. `tomori-render` is on `wgpu`, which targets WebGPU, so presenting environments in a browser through WebAssembly is conceivable, but it is not part of this proposal.
- **Control is the one part a browser can't reach today.** Browsers can't stream a request body in both directions on one exchange: `fetch` streaming uploads are half-duplex and need HTTP/2. The options:
  - **(a) A downlink and uplink pair, available now.** `GET /connect` returns the server's frames as a streaming response, which `fetch` reads incrementally over HTTP/1.1. The client's messages go as small `POST /connect/send` requests, bound by the session cookie. That needs nothing new in `base-http` and suits a mostly read-only client. Microphone audio as many small requests is heavy on HTTP/1.1, though, so push-to-talk would send 100 ms batches.
  - **(b) WebSocket at `/connect`,** one binary message per Tomori frame, the same bytes as today. It is full duplex over HTTP/1.1 and suits full participation, but it needs WebSocket framing in `base-http`. Upgrades exist, and the framing is a listed follow-up.
  - **(c) HTTP/2 request streaming or WebTransport,** once those exist in `base-http` and browsers.

  **Recommendation:** start a web client on (a), which works with today's server, and move to (b) or (c) for live audio.

## 7. What Tomori needs from `base-http`

**Today, Tomori uses `base-http` 0.2.** `base-http`, `base-http1`, `base-http-protocol`, `base-http-server` and `base-http-client` are all at 0.2.0, with `base-scheduler` 0.3, `base-pool` 0.3 and `base-endpoint` 0.1. The current release is 0.4, which requires `base-scheduler` 0.5, `base-pool` 0.5 and `base-endpoint` 0.3. Moving to it is **stage 0**:

- it is breaking, since scheduler types are in the public API, but mechanical;
- Tomori runs on `base-scheduler-native`, which 0.5 keeps (it removed only the Direct backend);
- it brings fixes Tomori benefits from: handler errors answered with 400 or 500 rather than a silent close, idle pooled connections checked before reuse, and `Content-Length` on uploads of known length.

HTTP/2 and HTTP/3 will arrive on 0.4's interfaces, so this is the prerequisite for the transparent upgrade.

| Tomori needs | `base-http` 0.4 | Gap |
|---|---|---|
| Long-lived duplex exchanges: request and response bodies streaming at once | yes. Pull bodies with independent upload and download progress ("duplex"); `/connect` relies on it today | none |
| Long-lived chunked responses produced by the server (streams, the downlink of a browser's pair) | yes. Chunked framing; the driver pulls the body and flushes before awaiting more | none |
| Back-pressure | yes. The driver pulls only as fast as it writes, and bounded queues are the application's | `TCP_NOTSENT_LOWAT` isn't one of the scheduler's socket options (`TCP_NODELAY`, `SO_KEEPALIVE` and `SO_REUSEADDR` are); requested in base-rs/base-scheduler#132, and useful for any socket that still mixes traffic |
| TLS with Tomori's own certificate and a pinning verifier | yes. rustls through `base-endpoint` | none |
| ALPN and protocol selection | yes. `base-http-protocol` selects an engine, but offers only `http/1.1` | HTTP/2 (milestone M6) and HTTP/3 over native `base-quic` (in progress) |
| Several connections to one server from the client (HTTP/1.1 channels) | yes. The pool admits one exchange per HTTP/1 connection and opens more | Tomori must bound the pool to its channel budget |
| Range requests | not yet. "Field-specific date/range/authentication/negotiation… parsers remain follow-up work" | Tomori parses one `bytes=` range itself, or `base-http` adds `Range` and `Content-Range`, requested in base-rs/base-http#72 |
| `ETag`, `If-None-Match`, 304 | 304 framing is handled, but there are no conditional-request helpers | small; Tomori can do it, or `base-http` adds it, requested in base-rs/base-http#71 |
| Cookies | none (listed as a separate follow-up) | Tomori already parses its own token cookie |
| Stream scheduling by priority under HTTP/2 and HTTP/3 (RFC 9218 `Priority`) | not applicable yet | **a requirement to ask for in M6:** media must never wait behind content data, and flow-control windows must not let a big content response starve media |
| HTTP datagrams (RFC 9297) over HTTP/3, for unreliable audio | not on the roadmap. Extended `CONNECT` exists as protocol intent | **ask for it** with `base-http3`; it is what makes live audio loss-tolerant |
| UDP endpoints in `base-scheduler` | "packet endpoints, QUIC and HTTP/3 remain future work" | prerequisite of HTTP/3 |
| WebSocket | managed upgrades exist; framing is a follow-up | only for a browser client's full participation |
| Graceful drain (GOAWAY under HTTP/2) | HTTP/1 shutdown and drain are implemented | part of M6 |
| Serving with a cancellation per connection, a request-head idle timeout and a bounded drain | no. `serve` shares one cancellation among its connections and drains without a bound, so Tomori keeps its own accept loop, which must set `TCP_NODELAY` itself, as `serve` does, since `serve_connection` leaves a socket it is given as it is | requested in base-rs/base-http#73 |

## 8. Compatibility and migration

- **Capabilities, not a version.** The client says what it reads and opens, with `Hello::content: bool` and `Hello::channels: Vec<String>`. The server says what it serves, with the `Tomori-Session` field and `Welcome`'s `channels`, `content` and `streams`. `PROTOCOL_VERSION` stays at 2, as #239 and #281 kept it.
- **Old clients keep everything inline over `/connect`:**
  - images arrive through `FetchImage` and `Image` frames;
  - code arrives short;
  - speech travels on `/connect`;
  - a stream panel arrives as its poster image, or as a text panel saying "a live view: this headset needs an update to show it", as #239 describes images to older clients.
- **Old servers send no ticket.** A new client then stays on one connection and reads what it is sent, as today.
- **The server routes each connection's lanes** according to what that client said. Both models coexist in the same hub, per connection, as `projection_patches` and whole projections do after #281.
- **Frame limits.** The negotiated 32 MiB frame limit stays a ceiling, but nothing needs to approach it once code and images leave the projection. The server should keep a much smaller target for any one control frame, such as 256 KiB, splitting larger snapshots as the audit's stage 2 describes, so that even old clients, whose speech shares the connection, aren't stalled.
- **Persistence** is versioned, as kept spaces are. Code bodies written inline in `spaces.json` are moved into the store as the file is read, and written back as references.

## 9. Staged plan

Estimates are relative: sizes in reviewed pull requests, with risk, as the work of one agent with an independent review. They are not dates.

| Stage | What | Size | Risk | Depends on |
|---|---|---|---|---|
| 0 | `base-http` 0.4 on both ends (scheduler 0.5, pool 0.5, endpoint 0.3) | S: 1 PR | low: breaking but mechanical; native scheduler kept | — |
| 1 | **Content store:** `GET /content/{hash}`, the session ticket, `ContentRef`, images moved from `images/`, code bodies by hash, `Hello::content` | M: 3 PRs (server store and endpoint; protocol and projection references; clients fetching, caching and showing whole code) | low to medium: authorisation by reference must be exact; the rest generalises #239 | 0; #281 merged |
| 2 | **Speech on the media channel:** `POST /media`, the lane moved with `Speech`, `Gate` and the utterance messages, `MediaEpoch`, switching at segment boundaries, falling back to control | M to L: 3 PRs (server, client connection, headset and desktop) | **medium:** ordering across channels, catch-up and held speech, live microphone binding; needs headset listening tests | 1's ticket |
| 3 | **Previews:** the placeholder, preview and full ladder; JPEG previews; the client's distance policy | S to M: 2 PRs | low; adds JPEG decoding on clients (server-made only) | 1 |
| 4 | **Picture streams:** `GET /stream/{id}`, `multipart/x-mixed-replace`, newest-wins, `Space::stream`, in-place textures; first #267's chart rendered on the server for clients that can't draw it, then a server-rendered diagram | M: 3 PRs | medium: texture updates on the frame budget; renderer for the server-side chart | 1; #267 |
| 5 | **Audio and video feeds:** Opus audio feeds; H.264/H.265 video through MediaCodec, copying first; on HTTP/3, live audio as datagrams with Opus forward error correction | L: 5 or more PRs | **high:** hardware decoding into Vulkan, codecs on desktop, Opus everywhere, datagrams waiting on `base-http3` | 2, 4; HTTP/3 for datagrams |
| 6 | **Interest management:** subscriptions by wall and distance; summaries of far regions in projections; streams and content already pulled on interest | M to L: 3 PRs | medium: the projection's contract changes for clients that opt in | #281; 1 |
| ↔ | **HTTP/2:** `h2` in ALPN on both ends, `Priority` honoured, media's connection policy measured on the Quest | S on Tomori's side once M6 ships | low to medium: TCP head-of-line blocking of media under Wi-Fi loss, to be measured | `base-http` M6 |
| ↔ | **HTTP/3:** QUIC on the same port over UDP, its port in the TXT record, falling back to TCP | M on Tomori's side | medium: UDP on networks that block it; Android sockets | `base-quic`, `base-http3` |
| W | **A web client:** served at `/app/`, the downlink and uplink pair, read-mostly first | M to L | medium: a new client surface to secure | 1, 4 |

Stages 1 and 2 may be swapped. Stage 2 removes the audible failure, but stage 1 makes the speech problem much rarer by itself, since no big code bodies or images remain in control frames, and stage 1 is lower risk. **Recommended order: 0, 1, 2, then 3 and 4 in either order.**

## Risks

- **Ordering across channels.** Anything that relied on "one stream" must be found. The messages listed for the media channel are those found by reading `speech.md`, `live-audio.md` and the outbox; a review should look for more, such as `TranscriptProgress`'s "being transcribed" acknowledgement, which today follows the last audio on the same stream.
- **More connections over HTTP/1.1** cost handshakes on reconnect, and slots on the server. That falls away under HTTP/2 and HTTP/3. Until then, TLS session resumption would shorten reconnects if `base-endpoint` supports it.
- **HTTP/2 could reintroduce head-of-line blocking** for media under loss. That's why media's connection is client policy, to be measured, not a protocol rule.
- **The authorisation check runs per request.** It must be cheap: an index from each space to the hashes it references, kept as panels change, not a scan of the projection under the hub lock.
- **Video on the Quest** depends on importing hardware buffers into Vulkan, which `wgpu` doesn't expose. A copying path comes first.

## Open questions for the maintainer

1. **Media dropping alone.** When only the media channel reconnects, within one session, should the live microphone resume on its own, or stop as on a disconnection and need the grip again? (Proposed: resume, binding capture to the session.)
2. **Presence:** keep it on control until datagrams exist (proposed), or move it to media now?
3. **Capability URLs:** needed at all? Proposed: no. Headers and same-origin cookies cover native clients and the web client.
4. **A disk cache on the headset** for content, across app restarts: worth it, and how large? (Today's image cache is 64 MB in memory.)
5. **JPEG on clients** for previews and pictures, all made by the server: acceptable, or keep PNG only and accept larger previews?
6. **#267's charts:** agree that headsets keep native chart data with patches, and that the server-rendered chart stream is for clients that can't draw it, as the first stream?
7. **Participants' own uploads,** such as a photo shared from a phone or a file dropped from the desktop: out of scope here. Wanted later?
8. **The web client:** served by the Tomori server at `/app/`? Should text panels ever render as HTML, which would need a separate origin, such as another port?
9. **Hash naming:** bare hex (proposed, as images use now), or `sha256-HEX` to leave room for another algorithm?
10. **Video codecs:** H.264 only first? Desktop decoding through a platform decoder, or a bundled one, with its licensing?
11. **HTTP/2 and media:** willing to keep media on a connection of its own under HTTP/2 if measurements on the Quest's Wi-Fi say so?

## Changes to the design documents

If accepted, stage by stage:

- **`server.md`:**
  - "one long-lived connection" becomes "one control connection, with media, content and stream exchanges bound to it";
  - the channel table;
  - resource limits per session;
  - UDP and QUIC as the later transport, through `base-http`.
- **`security.md`:**
  - the session ticket;
  - authorisation of content and streams by reference;
  - the fixed content types and response fields;
  - `Origin` checks for channels opened with cookies.
- **`projection.md`:**
  - `ContentRef` with placeholder and preview;
  - code bodies by hash;
  - `StreamRef`;
  - the store replacing the image store, which keeps its rules.
- **`speech.md`, `live-audio.md`:** the media channel, the messages that travel on it, `MediaEpoch`, and what happens when media drops alone.
- **`connection-liveness.md`:** liveness per long-lived channel; control ends the session.
- **`environments.md`:** `Space::share` declaring content; `Space::stream`.
- **`assets.md`:** the server's `Source` that fetches by digest from `/content`.
