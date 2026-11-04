# Environments

This describes how a space's environment relates to the space. The first part is a current requirement; the rest is long-term direction, and the initial design must not prevent it.

## Requirement

Each space is bound to exactly one environment at a time. The environment is chosen when the space is created, is part of the space's shared state, and is the same for everyone in it, participants and visitors alike. A participant does not switch a space's environment on their own device; the per-device environment choice, such as squeezing the grip to switch between Sky and Minimal, does not apply inside a space.

**Changing it.** An owner may change a space's environment after it is created, to another the server offers, from the hand menu's Space section (see `hand-menu.md`), with `set_environment`. The change applies to the whole space at once: the server checks that the requester is an owner and that it offers the environment, keeps the new environment in the space's record, so that a restart keeps it (see `spaces.md`), and sends the space's state to everyone in it, participants and visitors alike, who switch to it, the new scenery growing out of the space as it does on joining. Someone joining later arrives in it, and the lobby lists the space in it. A collaborator or visitor is refused, and their menu does not offer the change; an environment the server does not offer is refused, naming those it does; and a closed space keeps the environment it had. The conversation, queue, workers and settings are unaffected, except that the agent is given its instructions again, since the environment's layer changes with it. A server says in the space state that it changes environments, and a client offers the change only to such a server, so older servers and clients work as before; an older client sees the new environment only when it next joins. The server offers the environments a space can use, as it offers backends and directories; today Sky (`crates/environment-sky`), first, so that it is the default for a space whose creator names none, and the Minimal grid, labelled Grid. A client that doesn't know a space's environment shows the Minimal grid and says which environment it lacks, rather than substituting another environment silently.

Arrival, outside any space, uses the Minimal grid as its waiting area (see `arrival.md`).

An environment may contribute a layer of the instructions Tomori gives a space's agent, about what it presents and which tools and data sources to use (see `spaces.md`). Sky contributes none yet.

Long-term direction, not yet requirements:

- An environment is much more than scenery. It can be dynamic and data-backed, with its own interaction, such as Fantail Observe, a separate project that shows a running system's topology and telemetry spatially. Environments are expected to be built on the server-owned scene graph and modules in `scene-modules.md`.
- The space's agent is aware of its environment: the information and data it presents, and what each participant is pointing at or has selected in it. Pointing and selection identify concrete resources, with their identity and context such as the time window, not only positions or pixels.
- A participant can refer to what they are pointing at in speech, such as "Tomori, can you investigate why this server is overloaded?". The agent receives the identified resource with the request, and uses the tools available in that space, such as its MCP servers, to investigate and answer.

## Rationale

The user intends to build specific environments for different kinds of work, so a space represents much more than a cosmetic place: it pulls in the information and interaction that work needs. Tying the environment to the space keeps every participant looking at the same thing, which shared pointing and the agent's awareness of the environment depend on. Stated by the user, speaking from the headset, in November 2026, with Fantail Observe as the example to connect eventually.

In November 2026 the user decided, from the headset, that a space's environment can be changed after it is created, from the Space section of the hand menu: by owners only, for everyone in the space, and kept in the space's record (issue #74). Since the space moves as a whole, everyone keeps looking at the same thing. When environments agree versions with clients (issue #75), a change is also to be refused, saying who cannot follow, when someone in the space cannot present the new environment.

## Scope

The space's shared state and its creation and change requests in `crates/protocol` and the server, the clients' environment selection and the hand menu's change of it, and, long term, the scene module host and how participant pointing reaches the agent.

## Validation

- Create a space and confirm that its state names its environment, that a second participant sees the same environment, and that squeezing the grip inside the space doesn't change it.
- Test, with a server offering two environments, that an owner's change reaches everyone in the space, participants and visitors, the lobby listing and someone joining later; that the agent is given its instructions again; that a collaborator's or visitor's change, an unknown environment and a change to a closed space are refused without effect; and that the changed environment survives a restart. Test that the hand menu offers the server's environments only to an owner on a server that changes them, marking the current one, while anyone else reads the environment; and that the headset's scenery, the desktop window's and the spectator's follow the space's state when it changes.
- Test that the server offers Sky first and the Minimal grid, and binds a space whose creator names no environment to Sky.
- Confirm that the creation choices offer only the environments the server lists.
- Join a space whose environment the client doesn't know, for example with a test server that names an unknown one, and confirm that the client shows the grid and names the missing environment.

Open questions, not yet decided:

- Whether an environment is a scene module, a set of modules, or something that hosts them.
- How participant pointing and selection are delivered to the agent, and how that relaxes `scene-modules.md`'s current separation of the scene graph from agent interactions.
- How an environment's data sources, such as a telemetry service, relate to the tools available to the space's agent.
