# `tomori-environment-github`

The identity of [Tomori](https://github.com/ioquatix/tomori)'s GitHub environment, a prototype whose spaces show one repository's open issues and pull requests as cards that participants lay out: its identifier, `ID`, protocol version, `VERSION`, and label, `LABEL`. It is the environment's common crate, which its server and client halves both use, so that the two cannot disagree; it has no dependencies.

- [`tomori-environment-github-server`](https://crates.io/crates/tomori-environment-github-server) is its server half: the GitHub client that reads the cards with a token only the server holds, the agent's instructions, and the `--github-repository` option it registers on a server's builder.
- [`tomori-environment-github-client`](https://crates.io/crates/tomori-environment-github-client) is its client half, which presents it over the grid, where the shared scene draws its cards.

Its interface is 0.x, and the prototype may be reshaped with the environment framework; see Tomori's [`design/github.md`](https://github.com/ioquatix/tomori/blob/main/design/github.md).
