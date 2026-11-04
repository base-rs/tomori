// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! `tomori-server [--bind ADDRESS] [--backend NAME]... [--speech-root PATH] [--speech-provider NAME] [--name NAME] [--no-advertise] [--no-advertise-attention] [--keepalive-interval SECONDS] [--keepalive-timeout SECONDS] [--request-timeout SECONDS] [--workspace-root PATH]... [--max-spaces COUNT]`
//!
//! Serves Tomori clients on the local network, advertised with Bonjour as `_tomori._tcp`. Holders of the server's tokens are owners or collaborators; anyone else may watch as a visitor, unless `--no-visitors`. `tomori-server auth` manages the tokens. See `design/server.md` and `design/security.md`.
//!
//! This is the default build of the library's [`tomori_server::Builder`], with Tomori's own backends and environments.

use std::process::ExitCode;

fn main() -> ExitCode {
    tomori_server::Server::builder().main()
}
