# `tomori-environment-github-server`

The server half of [Tomori](https://github.com/ioquatix/tomori)'s GitHub environment prototype, whose spaces show one repository's open issues and pull requests as cards that participants lay out: the GitHub client that reads them, read-only, with one GraphQL query through a token only the server holds; the environment's layer of the agent's instructions; and `register`, which adds `--github-repository OWNER/NAME` to a server's builder, as Tomori's own server does:

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    tomori_environment_github_server::register(tomori_server::Server::builder()).main()
}
```

Given a repository, the server offers the environment, at the identity in [`tomori-environment-github`](https://crates.io/crates/tomori-environment-github), after the environments registered, with the token in `GITHUB_TOKEN`, or else from `gh auth token`, and does not start without one. `GitHub::new(repository, token).offer(server)` offers it to a server built without the command line. The token lives in this crate alone: it is never sent to a client, put in a space's state, told to the agent or logged. A client presents the environment with [`tomori-environment-github-client`](https://crates.io/crates/tomori-environment-github-client).

Its interface is 0.x, and the prototype may be reshaped with the environment framework; see Tomori's [`design/github.md`](https://github.com/ioquatix/tomori/blob/main/design/github.md).
