# Content

This describes the content store: immutable content the server serves by its digest beside each client's connection, which clients fetch, check and keep. Today it holds environments' assets; images, code bodies and baked light are to follow. It is stage 1 of `docs/proposals/channels-and-content.md`, which this takes precedence over where they differ.

## Requirement

**By digest, immutable.** A piece of content is named by the SHA-256 of its bytes, written as 64 lower-case hexadecimal digits, and is exactly those bytes: it never changes, and different bytes are different content. Only the server puts content in the store, from code of its own; clients never upload.

**Kinds from an allow-list.** Every piece of content is of a kind from `tomori_protocol::ContentKind`, declared by whoever puts it in the store, never taken from a file's name or a client, each with its `Content-Type`, its bound and the signature its bytes must start with:

| Kind | `Content-Type` | Bound | Starts with |
| --- | --- | --- | --- |
| model | `model/gltf-binary` | 16 MiB, as `tomori-assets` reads one | `glTF` |
| image | `image/png` | 8 MiB | PNG's signature |
| sound | `audio/ogg` | 1 MiB, as an ambience plays | `OggS` |
| texture | `image/ktx2` | 16 MiB, as `tomori-render` loads one | KTX2's identifier |

Nothing executable or that a browser renders as a document, such as `text/html` or `image/svg+xml`, is ever a kind. A kind is added with the content that needs it: baked light comes with the renderer's later stages (see `docs/proposals/rendering-architecture.md`).

**A texture in several forms.** A texture a device samples in one compressed form or another is served in each, each its own content, named for its form, `NAME.FORM.ktx2`, such as `textures/earth-colour.astc.ktx2` and `textures/earth-colour.bc7.ktx2`, and a client that says the form its GPU samples (`tomori_client::Options::textures`, `Connection::sample_textures`), `astc` on the headset and `bc7` on a desktop, fetches each such texture only in that one, so that neither downloads nor keeps what it cannot draw; one that says none fetches every form.

**Environments' assets.** An environment declares the assets it serves as it is registered, `Environment::with_asset(ServedAsset::new(asset, kind))`, each an `AssetRef` as its code names it (see `assets.md`), with a short licence notice of who made it and under what licence where it has one, at most 512 bytes. The server reads each from the program's asset library, `tomori_assets::Library::shared()`, to which the environment's crate adds its embedded assets and a program may add a folder, checked against its digest, its kind's bound and signature, and keeps it in memory, at most 512 MiB together. One that cannot be read so, or whose name is empty or longer than 256 bytes, is not served, and the log says why. Registering the environments again reads only what is new, and lets go of what no environment declares any more.

**The offer and the ticket.** A client that reads content says so in Hello (`content`). After its `Welcome`, the server sends it `ServerMessage::Content`: a session ticket, 256 random bits that name this connection and last exactly as long as it, never twice the same, and the assets of the environments offered to it, those its Hello presents at the same version (see `environments.md`), at most 256, each with its environment, version, name, digest, length, kind and licence. A connection admitted only to pair is sent neither. Older clients, which do not say `content`, are never sent the message; an older server never sends it, and a client then fetches nothing and uses what it bundles.

**The endpoint.** A client reads content with `GET /content/DIGEST`, or `HEAD`, beside its connection, on a connection of its own, so that content never waits ahead of speech, presenting its credential as `/connect` does and its ticket in the `Tomori-Session` field:

- a credential the door refuses, or a ticket missing or naming no connection of the same principal (none, for a visitor's), is answered 401, as at `/connect`;
- content that is not kept, not offered to the ticket's connection, or a digest not written in lower case, is answered 404, all alike, so that nothing says whether content exists;
- a request with a body is answered 400, and a method other than `GET` or `HEAD` 405;
- a connection has at most 4 content requests open at once, and another is answered 503.

A response carries the content's `Content-Type`, `Content-Length`, `ETag` of the quoted digest, `Cache-Control: private, max-age=31536000, immutable`, `Accept-Ranges: bytes`, `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox` and `Cross-Origin-Resource-Policy: same-origin`. One range of bytes, `bytes=START-`, `START-END` or `-SUFFIX`, is answered 206 with its `Content-Range`, so that a transfer that stopped resumes where it left off; a range past the end 416; anything else, such as several ranges, whole. `If-None-Match` with the digest's tag, or `*`, is answered 304. The body is written in pieces of 64 KiB as the client reads them, and a response whose client has taken nothing of it for the keepalive timeout, 15 s, has its connection ended by the keepalive sweep, as a connection whose writes stall is (see `connection-liveness.md`), so that a client that stops reading holds none of the server's connections.

**The first assets served.** Tomori's own server serves the farmyard's chicken this way (see `farmyard.md`), which headsets still bundle, as the fallback for a server that does not serve it, and the farmyard's surface textures, earth, stone and bark, each a base colour and a normal map in ASTC and in BC7, generated in code the first time its store reads them (see `rendering.md`), which no client bundles: one that has not fetched them draws those surfaces by their own colours.

**The client's side** is described in `clients.md`: a client fetches each asset it is offered that its cache does not keep, beside its connection, resuming one that stopped, checks it against its digest, and keeps it across sessions in a cache bounded in size, whose least recently used asset is let go first, which its asset library reads before what the app bundles.

## Rationale

The maintainer accepted the channels and content proposal in November 2026 and chose serving environments' assets by digest as the first of the renderer's steps, so that a headset can present an environment its build does not bundle, and so that textures made on the server, and later its baked light, reach headsets the same way. Content addressing makes what a piece of content is independent of where it came from, so that a client keeps it once whichever server or environment names it, and fetches a changed asset once and an unchanged one never again. The store follows the proposal's stage 1 with three differences, each to be taken up by the stage that needs it: the ticket is sent in a message of its own after `Welcome` rather than in `Welcome`, and not yet in a response field or a cookie, which only a browser needs; a connection reads only environments' assets, offered by the environments it presents, which every role may read as they are part of the environments themselves, while images and code bodies still travel on `/connect`; and content is kept in memory, read again from the program's library at each start, rather than in the state directory, since environments' assets are part of the program.

## Scope

`crates/protocol/src/content.rs`; `crates/server/src/content.rs`, the endpoint in `application.rs`, and the offer and tickets in `hub.rs`; and the client's side, `crates/client/src/content.rs` and `crates/assets/src/cache.rs`.

## Validation

- Tests keep and offer the assets environments declare, once each, only to a connection presenting their environment at its version, and refuse one missing from the library, not its digest, larger than its kind's bound, not starting as its kind, with a name or licence too long, kept as another kind, or past the store's bound, each saying why.
- Tests end a response whose client has taken nothing of it for the keepalive timeout, and no other.
- Tests send a ticket and the offer only after the `Welcome` of a client that said `content`, never to one that did not or that only pairs; read only with a ticket of the same principal, a visitor's with a visitor's; refuse a fifth request open at once; and end the ticket with its connection.
- Tests keep and offer a texture, and refuse one that does not start as KTX2 or is larger than 16 MiB; a client fetches a texture named for a form only in the form it says it samples, and every form if it says none; and a client sampling ASTC fetches the farmyard's ASTC textures from Tomori's own server and none in BC7.
- Tests read an asset over HTTP whole, in part, past its end, conditionally and by `HEAD`, with every field above, and refuse a request without its credential or ticket, with another's ticket, for a digest not offered or not written as one, with a body, or of another method.
