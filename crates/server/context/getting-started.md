# Building a Tomori Server

`tomori-server` is both the `tomori-server` executable and the library it is built from. Use this guide to build a server of your own that offers backends and environments Tomori doesn't have, without forking Tomori or changing its source. To run Tomori's own server, install `tomori-server` instead, as its readme explains.

## A server of your own

A downstream server is a binary crate that depends on `tomori-server` and calls `Server::builder`, which starts with Tomori's own backends and environments, registers its own on it, and runs the process's command line:

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

`Builder::environment` offers an environment for spaces: its identifier, which clients map to what they present, the label they show, and optionally its layer of the instructions a space's agent is given. Environments are offered in order, after Tomori's own Sky and Grid, and the first is the one a space whose creator names none is bound to; registering an identifier already offered replaces that environment in its place. A client that doesn't know an environment's identifier shows the Minimal grid and names it, so a headset needs the environment's client half to present it.

Environments have no server halves, data sources or tools yet. Those are planned, and will be registered through the same builder.

## Other settings

`Builder::configure` changes the server after the command line has configured it and before it serves, with `Server`'s `with_` methods, such as `with_max_visitors` or `with_participant_retention`. Hooks run in the order they were given.

## Without the command line

To embed a server without its command line, build a `Server` directly with `Server::new`, giving it `Backends` and a `Transcriber`, offer environments with `Server::with_environments`, before or after `Server::with_kept_spaces`, since a space's layer of the instructions is looked up in the environments offered whenever it is given, and serve a listener from `tomori_server::bind` with `Server::run` on a `base-scheduler` scheduler. Admission, TLS and kept spaces are then configured with `Server`'s `with_` methods, which is what the command line does for you.

The [example server](https://github.com/ioquatix/tomori/tree/main/examples/server) registers a placeholder backend and environment and tests its executable end to end. The requirements the server follows are in Tomori's [design documents](https://github.com/ioquatix/tomori/tree/main/design), such as `builders.md`, `server.md` and `environments.md`.
