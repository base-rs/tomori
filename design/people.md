# People

This describes how the others in a space are drawn, as the first stage of `docs/proposals/drawing-other-people.md`, which the maintainer approved in November 2026: their rays, and what presence carries for them.

## Requirement

**Rays are sent while drawn in full.** A participant's presence carries `pointing`, each of their two rays drawn in full on their headset, left then right (`tomori_protocol::Ray`): its aim pose, pointing along the pose's −Z, how far it reaches, to what it meets or as far as an idle ray drawn in full reaches, and whether it ends on something it meets. A ray is drawn in full while it meets something, while its hand is busy, and for a second after (see `interaction.md`). A stub, a hidden ray, the hand menu's ray and a teleport's aim are never sent, since they are the participant's own: each is `null`. `pointing` is sent whenever one of the rays is shown, and left out while neither is, so that the server tells a current client, whose stubs point at nothing, from an older one, which sends none. The server sounds each ray as it sounds poses, a finite aim within reach with a rotation, made unit, and a finite length of more than nothing and at most `RAY_REACH`, 100 m, drops one that is not, and relays the rest to the others in the space with the poses, leaving `pointing` out where none is left. It adds about 130 bytes to a report for each ray drawn in full, and 24 while only stubs are shown.

**The agent is told what each ray points at from its aim.** The `presence` tool resolves what each of a participant's rays points at from the aim it was sent with (see `mcp.md`). From an older client, which sends no aims, each ray its client says is shown is resolved from its controller's pose, as before, which is the grip the headset draws the controller at rather than the aim its ray is drawn from.

## Rationale

On #417's review the maintainer noticed that nobody is drawn in a space, and asked for other people's heads, hands and rays to be drawn; the proposal's first stage draws heads and rays. Others' rays are drawn only while drawn in full on their own headset, as the maintainer agreed, so that a room of four is not a room of eight beams; idle stubs and teleport aims stay personal.

Presence sent the headset's controllers' grip poses, which it draws its controllers at, while its rays are drawn from the aim poses, which were never sent, so the server told the agent what a ray pointed at along the grip's −Z, which OpenXR tilts from the aim. Sending each ray's aim while it is drawn in full gives the others what to draw and the server what the participant sees their ray meet. A stub is sent as `null` rather than its aim, since what it points at is nothing the participant can act on, and sending `pointing` whenever a ray is shown costs a few bytes a frame rather than a capability in `Hello` that every connection would carry.

## Scope

`tomori_protocol::Ray` and presence's `pointing` (`crates/protocol`); the frame step's `Presence::pointing` and `pointing::Beam::sent` (`crates/scene`); `Connection::send_presence` (`crates/client`); and the server's relaying and resolving of it (`crates/server`'s `hub/act/presence.rs` and `presence.rs`).

## Validation

- `tomori-protocol`'s tests: presence round-trips with and without `pointing`, a stub as `null`, a ray meeting nothing without `meets`, and an older peer's presence reads as none; a ray is valid only aimed somewhere, with a rotation, finite and within reach.
- `tomori-scene`'s tests: a stub, the hand menu's ray and a teleport's aim are sent as `null`, a ray drawn in full from its aim as far as it reaches, and one on a worker meeting it.
- `tomori-server`'s tests: a ray drawn in full is relayed sounded and one that is no ray is not, stubs alone relay no `pointing`; the agent is told what a ray points at from its aim rather than its grip, a current client's stub points at nothing, and an older client's shown ray is resolved from its controller.
