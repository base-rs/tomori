# Building a Tomori Server

`tomori-server` is the library that Tomori's own server, the `tomori-server` executable from `tomori-server-app`, is built from. Use this guide to build a server of your own that offers backends and environments Tomori doesn't have, without forking Tomori or changing its source. To run Tomori's own server, install `tomori-server-app` instead, as `tomori-server`'s readme explains.

## A server of your own

A downstream server is a binary crate that depends on `tomori-server` and calls `Server::builder`, which starts with Tomori's own backends and no environments, registers its own on it, and runs the process's command line:

```rust
use std::process::ExitCode;

use tomori_server::{Environment, Server};

fn main() -> ExitCode {
    Server::builder()
        .backend("placeholder", || Ok(my_backend::Placeholder))
        .environment(
            Environment::new("observe", "Observe")
                .with_instructions("The space shows the service's topology."),
        )
        .main()
}
```

The executable takes the same command line as `tomori-server`, including `auth` for tokens, and serves the same way: token authentication with TLS, pairing, Bonjour, speech, kept spaces and orderly shutdown. `--help` lists the registered backends. `Builder::run` takes a command line of your own instead of the process's, and `Builder::main` also sets up logging unless the program has already.

## Backends

`Builder::backend(name, create)` offers a backend under `name`, made by `create` when the command line names it with `--backend NAME`. Implement `Backend` and `Session` from `tomori-backend`; the server holds every backend behind one type, so yours is treated exactly like OpenCode's. The backend's `Backend::name` must be the name it is registered under, since clients name it by that. `create` reads any configuration of its own, such as environment variables, and returns an error message to stop the server from starting. Registering a name Tomori already uses, `fake` or `opencode`, replaces Tomori's backend.

## Environments

`Builder::environment` offers an environment for spaces: its identifier, which clients map to what they present, the label they show, its protocol version, 1 unless `Environment::with_version` gives another, and optionally its layer of the instructions a space's agent is given. Environments are offered in the order registered, and the first is the one a space whose creator names none is bound to; registering an identifier already offered replaces that environment in its place. The builder offers none until you register some, so a server that registers none refuses to create spaces. To offer Tomori's own, register them from their crates, as Tomori's own server, `tomori_server_app::builder()`, does: Sky by the identity in `tomori-environment-sky`, `Environment::new(tomori_environment_sky::ID, tomori_environment_sky::LABEL).with_version(tomori_environment_sky::VERSION)`, and the grid by `tomori_protocol::GRID` and `GRID_VERSION`. A client that lists the environments it presents in its `Hello`, as Tomori's own do, is offered only those it presents at the same version, and creates and changes spaces only in them; an owner's change still applies to everyone in the space. A client that joins a space in an environment it lacks, or has at another version, shows the Minimal grid and names the version it needs, so a headset needs the environment's client side, at the same version, to present it. Raise the version when a client side of the earlier version could not present the environment; a change it can ignore, such as a new optional field, keeps it.

An environment may also have a server side, a program in the server that runs while a space is bound to the environment, which `Environment::with_server_side` registers with it. Implement `ServerSide` from `tomori-environment`:

```rust
use tomori_environment::{Result, ServerSide, Space};
use tomori_server::Environment;

struct Observe;

impl ServerSide for Observe {
    fn run(&self, space: &Space) -> Result<()> {
        while !space.cancellation().is_cancelled() {
            // Read what the space presents, until it is bound to another environment or closes.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(())
    }
}

let observe = Environment::new("observe", "Observe").with_server_side(Observe);
```

The server runs it on a thread of its own, once each time a space is bound to the environment: when the space is created in it, an owner changes it to it, or it is bound to its session again after a restart. It is given the space's identifier, title and directory and a cancellation, which is cancelled when the space is bound to another environment, closes, or the server stops; the server waits only `Server::with_drain_timeout` for sides to stop when it stops itself. A side that returns an `Error` or panics is shown in the space, which keeps its environment and conversation, and is not run again for that binding. Environments have no state, data sources or tools on the server yet. Those are planned, and will be registered through the same builder. To make an environment of your own, with a common crate for its identity and server and client crates beside it, follow `tomori-environment`'s Making an Environment guide.

## Other settings

`Builder::configure` changes the server after the command line has configured it and before it serves, with `Server`'s `with_` methods, such as `with_max_visitors` or `with_participant_retention`. Hooks run in the order they were given.

`Builder::option` adds a command-line option of your own, a `CommandOption`, such as an environment's configuration: `--help` lists it after Tomori's own, the command line reads its value, and it changes the configured server before the hooks run. The GitHub environment prototype's `--github-repository` is one, which `tomori_environment_github_server::register` adds.

An environment whose spaces show cards that participants lay out, such as the GitHub prototype's issues and pull requests, implements `CardSource` and is offered with `Server::with_cards`, which reads the cards while someone watches them, within the source's rate limit.

## Without the command line

To embed a server without its command line, build a `Server` directly with `Server::new`, giving it `Backends` and a `Transcriber`, offer environments with `Server::with_environments`, before or after `Server::with_kept_spaces`, since a space's layer of the instructions is looked up in the environments offered whenever it is given, and serve a listener from `tomori_server::bind` with `Server::run` on a `base-scheduler` scheduler. Admission, TLS and kept spaces are then configured with `Server`'s `with_` methods, which is what the command line does for you.

The [example server](https://github.com/ioquatix/tomori/tree/main/examples/server) registers a placeholder backend and environment and tests its executable end to end. The requirements the server follows are in Tomori's [design documents](https://github.com/ioquatix/tomori/tree/main/design), such as `builders.md`, `server.md` and `environments.md`.
