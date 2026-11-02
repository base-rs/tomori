# Security

This describes who may connect to the server and what each connection may do: roles, authentication, encryption, pairing and admission. It replaces the deferral of authentication in `server.md`. Parts of it are long-term; the open questions at the end are not yet decided.

## Requirement

**Roles.** Every connection has exactly one role, which decides what it may do:

- **Owner**: everything a collaborator can do, and also creating spaces and listing an agent's existing sessions.
- **Collaborator**: joining and leaving spaces, push-to-talk to the agent, live voice, presence, deciding approvals, answering questions, cancelling queued input, changing the session's mode (see `agent-backends.md`) and setting attribution.
- **Visitor**: listing spaces, joining a space to watch it, leaving it, and knocking. A visitor receives what a participant receives in that space: its state, presence, agent speech and participants' audio. A visitor sends no audio, and appears neither in a space's participants nor in a space's or the server's participant count. No reduced view is required for visitors yet.

Owners and collaborators are a space's participants (see `spaces.md`); visitors are not. The server checks every client message against the connection's role in one place before acting on it, and refuses a message the role does not permit with a short error that has no effect. It never branches on hardware (see `clients.md`). `Welcome` names the connection's role. Only an owner's `Welcome` lists the workspace roots and backends, since only owners create spaces. A client hides the actions its role does not permit, and reads a role it does not know as unknown, permitting nothing beyond a visitor's actions. The spectator client is a client like any other: it watches as a visitor, or connects with a token where visitors are refused.

**Modes.** What a connection without credentials becomes is the server's mode:

- By default it is a visitor. Credentials are needed only to do more than watch.
- With `tomori-server --no-visitors` it is refused.
- With `tomori-server --auth open` it is an owner, as before authentication existed. The server then accepts no credentials, and prints and logs at startup that anyone who can reach it can create spaces and approve tool use. `--auth open` cannot be combined with `--no-visitors`.

At most 64 visitors are connected at once by default, configurable with `tomori-server --max-visitors COUNT`, so that visitors cannot take every one of the server's connection slots from participants. A visitor beyond the limit is refused with HTTP 503 before the connection reaches the hub.

**Authentication at the door.** Credentials are carried on the HTTP request that opens a connection (`POST /connect`), as an `Authorization: Bearer` header or a cookie, and are checked before the connection reaches the hub. No protocol message carries credentials; the reserved `credentials` field of Hello is removed. A connection that presents a credential the server does not accept, whether unknown, revoked or malformed, is refused with HTTP 401 and the same short reply in every case, never silently downgraded to a visitor; the reason is logged on the server. The client then forgets that credential, connects again without it, and shows that it is no longer admitted.

**The security crate.** Authenticators, the token store, pairing and the server's certificate live in their own workspace crate, `crates/security`. It depends on neither the server nor any client crate, so the server and clients share it, as clients derive the pairing code too. The server's hub and connection handling stay in the server crate.

**Pluggable authenticators.** Authentication is performed by a chain of authenticators in the security crate. Each inspects the request head and reports a principal, an opaque identifier namespaced by the authenticator that verified it, or passes to the next, or refuses. Authenticators report who a connection is, never what it may do: the server keeps roles against principal identifiers, so adding an authenticator changes no role check. The first authenticator is `token`, below.

**Tokens.** The `token` authenticator accepts random bearer tokens of at least 256 bits that the server issued. The server stores only a hash of each token, with the principal and role it was issued for, in a file readable only by the server's user; tokens survive a server restart. `tomori-server auth list` lists the issued tokens without revealing them, and `tomori-server auth revoke` revokes one. A revoked token is refused from then on, and every connection that presented it, or was admitted with it, is ended.

**Encryption.** Whenever it accepts credentials, the server serves only TLS, and a TLS handshake that has not completed counts against the request timeout (`server.md`). It uses a self-signed certificate that persists across restarts, and advertises its SHA-256 fingerprint in its Bonjour TXT record. A client remembers a server's certificate the first time it connects, or when it pairs. It never sends a credential to that server over plain HTTP or over a connection presenting a different certificate; it connects as a visitor instead, and shows that the server's identity has changed. In `--auth open` mode, plain HTTP remains allowed.

**Participant identity.** The identity a client says in Hello is a secret of that device. The server never sends a client's identity to another client: shared state, including space participants and queue entries, names participants by a public handle the server assigns, which stays the same for as long as the server keeps that participant (see `spaces.md`). A client learns its own handle in `Welcome`. The server keeps an authenticated participant under its principal and identity together, so the same identity on a connection without that principal is a different participant and cannot take over the first one's connection, space or audio source.

**Pairing.** A device becomes an owner by pairing. Pairing runs over TLS and derives a 6-digit code from three things: the server certificate's fingerprint, as each side sees it, a random value from the device and a random value from the server. The device first sends only a commitment to its value, a hash of it. The server then sends its value, and only then does the device reveal its own. So neither side, nor anyone relaying between them, can choose a value after seeing the other's, and a relay presenting a different certificate shows the two ends different codes, except by a one-in-a-million chance. The device shows the code in the world, anchored to the server's object (see `arrival.md`), and the server shows it on its console. The operator confirms only if the two codes match, and the pairing completes only once they confirm. The device then receives an owner token, stores it for that server with the server's certificate, and connects with it silently from then on. At most one pairing is pending at a time; another request while one is pending is refused.

**Knocking.** A visitor watching a space can knock to ask to collaborate. The knock, with the visitor's display name, is shown to the space's participants, and the first participant to admit or decline it resolves it, as for approvals (`spaces.md`). An admitted visitor receives a collaborator token over its connection, which becomes a collaborator connection at once without reconnecting; the client stores the token for that server as for pairing. Knocks are bounded: a visitor has at most one pending knock, a knock unanswered for 2 minutes expires, a space has at most 8 pending knocks, and a declined or expired knock cannot be repeated on that space for 1 minute. Visitors cannot admit or decline knocks.

**Protocol version.** These changes raise the protocol version. A participant's handle replaces their identity in shared state, and Hello and `Welcome` change; an older client could not recognize its own queue entries.

## Rationale

`server.md` deferred authentication until Tomori is used on untrusted networks, because space participants can approve agent tool use. The user wants people without credentials to be able to watch a space, hearing the speech and seeing the geometry, while collaborating requires admission, and only owners create spaces. Authentication should be unobtrusive: a device authenticates once and then connects silently, and admitting someone happens in the scene rather than through login forms, in keeping with `arrival.md`. It should also be pluggable, so that identity providers can be added without changing what each role may do.

Checking credentials on the HTTP request means a refused or unauthenticated client never receives a participant record with privileges, or the workspace roots, and works with reverse proxies that set headers or cookies. Refusing an invalid credential rather than downgrading it lets a device learn that it was revoked. Bearer tokens are only safe encrypted, so accepting them requires TLS. Pairing by comparing a code shown on both ends needs no keyboard on the headset and also proves to the device that it reached the intended server; the commitment, as in Bluetooth numeric comparison, stops a relay from searching for values that make the codes match. The participant identity was sent to every client in a space, which let anyone in it reconnect as another participant and take over their connection; with visitors receiving the same state without credentials, that had to stop. The limits on visitors and knocks keep the server's resources bounded, as `server.md` requires of everything a client can cause. Agreed with the user in the November 2026 authentication discussion; the default limits were chosen in drafting and may be revised.

## Scope

The server's HTTP handling of `/connect`, its role checks, token store and command-line options; `crates/security`; the protocol's Hello, `Welcome`, knock and participant handle; the Bonjour TXT record; and how clients store credentials, pin certificates, pair and knock.

## Validation

- Test each role against every client message: each permitted message takes effect and every other message is refused with no effect. A visitor's audio frames are neither relayed nor transcribed.
- Test that a visitor joining a space receives its state, presence, agent speech and participants' audio, that it appears neither in the space's participants nor in any participant count, and that its `Welcome` names its role. Test that only an owner's `Welcome` lists roots and backends.
- Test the modes: without credentials a connection is a visitor by default, is refused with `--no-visitors` and is an owner with `--auth open`, which prints its warning. Test that `--auth open --no-visitors` is refused, and that visitors beyond `--max-visitors` are refused with 503 without reaching the hub.
- Test that an unknown, revoked and malformed credential each receive the same 401 reply without reaching the hub, and that a client then forgets the credential and reconnects as a visitor.
- Inspect the security crate's dependencies: it depends on neither the server nor any client crate. Check that role decisions are made only in the server against principal identifiers. Inspect the protocol: no message carries credentials.
- Test that the token store holds no token in the clear and survives a restart, and that revoking a token refuses it and ends both a connection that presented it and one admitted with it.
- Test that a server accepting credentials serves only TLS, that a stalled handshake is closed after the request timeout, and that its fingerprint is in its TXT record. Test that a client sends no credential to a remembered server over plain HTTP or over a different certificate.
- Test that no client's identity appears in any message sent to another client, that a participant's handle survives their reconnecting, and that a Hello repeating an authenticated participant's identity without their credential neither supersedes their connection nor joins their space.
- Test pairing: the codes match through a direct connection, and with a relay holding a different certificate they differ, whatever values the relay chooses. Nothing is issued until the operator confirms. A second request while one is pending is refused. Afterwards the device connects with its token without further steps.
- Test knocking: the first decision resolves a knock; an admitted visitor becomes a collaborator on the same connection; the pending, expiry, per-space and repeat limits hold; a visitor cannot decide a knock.

Long-term direction, not yet requirements:

- An owner can promote an existing participant or visitor to collaborator, and demote them. There is no interface for it yet.
- Invitations: an owner creates a link or code that grants a role, with a limited number of uses and an expiry.
- Further authenticators, such as one that trusts an identity header from a reverse proxy or Tailscale at configured addresses, and OpenID Connect for browser visitors.
- A reduced view for visitors, if visitors arrive from outside a trusted network.

Open questions, not yet decided:

- Whether and how visitors are shown to participants: not at all, as faint markers, or as a count. Until it is decided, a visitor sends no presence and the server advertises no visitor count.
- Whether admitting a knock makes the visitor a collaborator in that space only or on the whole server, and so what the issued token grants.
- Whether an owner already in a space can confirm a pairing in the world, as well as the operator on the console.
