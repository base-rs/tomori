# Builders

This describes how other projects build their own Tomori server and headset from Tomori's crates.

## Requirement

Another project builds a server of its own on the `tomori-server` library, without forking Tomori or changing its source: it registers backends and environments through the library's builder, `tomori_server::Server::builder()`, which starts with Tomori's own. What it can register is what Tomori supports today:

- **Backends:** any implementation of `tomori-backend`'s `Backend`, from any crate, registered under the name `--backend` and clients name it by, which must be its `Backend::name`. The server treats it as it treats OpenCode's; nothing in the server names a concrete backend. Registering a name already registered replaces that backend.
- **Environments:** an identifier, a label and the environment's layer of a space's instructions (see `spaces.md`), offered to clients after those already registered unless it replaces one with the same identifier. The first offered is the default, as `environments.md` requires.
- **Configuration:** hooks that change the configured server before it serves, through its public settings.

Environments' server halves (#75), data sources (#83) and tools (#82) will be registered through the same builder once Tomori has them; until then the builder offers nothing for them.

A server built this way takes the same command line as `tomori-server`, including `auth`, and serves the same way: admission, TLS, pairing, advertising, speech, kept spaces and shutdown follow `server.md`, `security.md` and `spaces.md` unchanged. `--help` lists the backends registered.

The `tomori-server` executable is the default build of that builder, with only Tomori's own backends, `fake` and `opencode`, and environments, Sky and the Minimal grid. Its command line, output and behaviour are those it had before the builder existed.

An example downstream crate, `tomori-example-server` in `examples/server`, depends on the published crates as another project would, registers a placeholder backend and environment through the builder, and is built and run by the workspace's tests.

Another project likewise builds a headset of its own on the `tomori-client-quest` library, whose library is `tomori`, through `tomori::Builder`, which starts with Tomori's own build. What it can register is what Tomori supports today:

- **Sceneries:** what the headset draws for a space bound to an environment, by the environment's identifier: an implementation of `tomori-render`'s `Scenery`, drawn natively in each eye's pass as Sky is, made for the renderer's targets by a function it registers. Registering an identifier already registered replaces that scenery. The Minimal grid is built in, and is what a build shows, naming the environment, for a space bound to one it has no scenery for. Tomori's own build registers Sky.

Environments' client halves will be registered through the same builder once #75 defines them; until then it offers nothing for them, and a scenery is all of an environment that a headset build adds.

`Builder::run_android` runs the headset on the Quest, from a build's own `android_main`; `run_xr` runs it on a desktop OpenXR runtime; and `tomori::main(builder)` is the `tomori` executable's command line, whose `--xr`, `--desktop` and server-less window draw the builder's sceneries, and whose desktop window's **E** cycles through them. The `tomori` executable and APK are the default build, `Builder::default()`; their command line and behaviour are those they had before the builder existed. The spectator still draws only Tomori's own sceneries. Packaging a build as its own APK is planned in #132.

An example downstream crate, `tomori-example-headset` in `examples/headset`, registers a placeholder floor for the example server's placeholder environment, and is built and tested by the workspace's tests.

## Rationale

Other projects, such as a private integration with environments of its own, need their own server and headset with their own environments, data sources and tools, without forking Tomori or adding their specifics to its source (#73, and the environment framework proposal in #90). Registering backends and environments is what the server can support now; the rest of the framework slots into the same builder as it lands, rather than ahead of its design. The live server and the people who run `tomori-server` depend on its command line, so the default build must not change. Planned in #73 in November 2026.

## Scope

The `tomori-server` library's builder and the registries it fills, the `tomori-server` executable, and the example in `examples/server`; the headset's builder, `tomori-render`'s sceneries, the `tomori` executable, and the example in `examples/headset`.

## Validation

- Test that a backend registered through the builder is offered by name, listed in `--help` after Tomori's, replaces one registered under the same name, and is refused when its own name differs from the one it was registered under or its constructor fails; and that environments registered are offered after Sky and the Minimal grid, or in place of one with the same identifier, with their instructions.
- Test against a fake backend that a registered environment's layer reaches the agent as its `tomori-environment` instruction entry, for a space created in it, changed to it or from it, and restored after a restart, including when a hook offers the environments after the kept spaces are listed again (see `environments.md`).
- Test that the example's executable lists its placeholder backend in `--help`, offers it and the placeholder environment in `Welcome`, and creates a space with both, whose state shows the environment's layer of the instructions.
- Compare `tomori-server --help`, an unknown argument and `auth --help` between the builder's revision and the one before it, and the startup output with the live server's flags, `--backend opencode --name NAME --speech-root PATH --speech-backend offline --workspace-root PATH` with token authentication and TLS, on a loopback port, without advertising and with an unreachable `--opencode-url`: they must be identical apart from ports, paths, fingerprints and times.
- Inspect the example: it depends only on published crates and uses only their public interfaces.
- Test that the headset builder's default draws the grid and Sky, that a registered scenery is listed after them or replaces one with the same identifier, and that a conversation and renderer given its sceneries draw a space bound to its environment, where Tomori's own build shows the grid and names it.
- Test that the headset example draws its floor for a space in the placeholder environment on the shared test device, and that its executable takes `tomori`'s command line.
- Run `cargo publish --workspace --dry-run --locked`.
