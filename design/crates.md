# Crates

This describes how Tomori's workspace crates are named, versioned and published.

## Requirement

Every workspace crate except the private `bake/` package is named `tomori-` followed by its role, such as `tomori-protocol`, and lives in a folder under `crates/` named without the prefix, such as `crates/protocol`. Clients follow the same rule, with the role `client-<target>`, as `clients.md` requires: `tomori-client-quest` lives in `crates/client-quest`. Examples of downstream crates are the exception: they build on the published crates as another project would (see `builders.md`), and each is named `tomori-example-` followed by what it builds, lives in `examples/` named without that prefix, such as `examples/server` for `tomori-example-server`, and is never published. Rust paths use the crate names, such as `tomori_protocol::ClientMessage`.

Names visible to people running Tomori do not change with package names: the server executable is `tomori-server`, and its command-line interface is unchanged by packaging; the headset's library and executable are `tomori`, its Android application ID is `com.codeotaku.tomori`, and its APK is `tomori.apk`. The application ID moved once, from `io.github.ioquatix.tomori`, in November 2026, at the maintainer's decision; the new app installs beside the old one and is paired again (see `docs/naming.md`).

The crates are released together: they share one version, `[workspace.package] version` in the root `Cargo.toml`, one `releases.md` and one release tag. Every dependency of one workspace crate on another names that version as well as its path.

These crates are published to crates.io, and every one of them can be packaged and verified with `cargo publish --dry-run`:

| Crate | Why it is published |
| --- | --- |
| `tomori-protocol` | Every client and server needs it, including implementations outside this repository. |
| `tomori-client` | The client core, for other clients and for headset builds outside this repository, with the connection that a client's session drives. |
| `tomori-discovery` | Needed by `tomori-client` and the server. |
| `tomori-security` | Needed by `tomori-client` and the server. |
| `tomori-backend` | The interface other projects implement to add an agent backend. |
| `tomori-backend-opencode` | The default backend, needed by the server's `opencode` feature. |
| `tomori-environment` | The interface of an environment's server side, which the server runs and another project's environment crate implements, without depending on the server. |
| `tomori-speech` | Needed by the server's default `speech` feature. |
| `tomori-server` | `cargo install tomori-server`, and the library that downstream servers build on through its builder; see `builders.md`. |
| `tomori-text` | The bundled fonts and the glyph atlas, which `tomori-scene` lays text out with and `tomori-render` draws. |
| `tomori-environment-sky` | The Sky environment's renderer, which `tomori-render` draws as Tomori's own scenery, so that the renderers and the clients can be published. Its interface changes when environments get a shared interface. |
| `tomori-scene` | The platform-neutral client, which every client crate and downstream headset builds on. |
| `tomori-render` | The renderers and the `Scenery` that a downstream headset implements to draw an environment of its own. |
| `tomori-client-desktop` | The desktop modes, which `tomori-client-quest`'s executable and builder run. |
| `tomori-client-spectator` | The spectator, `cargo install tomori-client-spectator` for its `tomori-spectator` executable, which `tomori-client-quest`'s executable runs too. |
| `tomori-client-quest` | The `tomori` library, with the headset's builder that downstream headsets build on (see `builders.md`), and the `tomori` executable. Tomori's own headset is also released as an APK. |

The client crates' interfaces are large and change with the environment framework (#75, #77); they stay 0.x, as every crate does, and their readmes say so.

These stay unpublished, with `publish = false`:

| Crate | Why it is not published |
| --- | --- |
| `tomori-example-server` | An example for downstream servers to follow; see `builders.md`. |
| `tomori-example-headset` | An example for downstream headsets to follow; see `builders.md`. |
| `tomori-bake` | Development tasks only. |

A published crate depends only on published crates and on crates.io packages, never on an unpublished crate, a path outside its folder or a Git repository. Each published crate has a description, license, repository, readme, keywords and categories, and ships `license.md`, linked from the repository root.

## Rationale

Other projects need to build their own server and headset with their own environments, data sources and tools, without forking Tomori. That needs Tomori's crates on crates.io under names that cannot collide with other packages in its flat namespace, released at one version so that a downstream project can depend on a matching set. The short folder names keep paths in the repository readable, as Base's own workspaces do. The executable, library and Android names are what people and scripts already use, so packaging must not break them. Proposed in [#73](https://github.com/ioquatix/tomori/issues/73) in November 2026; when to publish for the first time is still the maintainer's decision. The names were found free on crates.io on 2026-11-03; see `docs/naming.md`.

The headset's library has a builder that other projects can use (see `builders.md`), so it and the crates it was split into are published, at the maintainer's decision on #132 in November 2026, once the builder and `examples/headset` had proved their interfaces. A published crate's optional and host-only dependencies must be published too, so publishing `tomori-client-quest` publishes the desktop and spectator crates that its executable runs. `tomori-environment-sky` is published although its interface will change with the environment interface planned in [#97](https://github.com/ioquatix/tomori/issues/97), because `tomori-render` depends on it for Tomori's own scenery: the maintainer chose that over moving Tomori's Sky registration into an unpublished application package, in November 2026.

## Scope

Workspace members, their manifests, and the Rust paths, scripts, workflows and documents that name them.

## Validation

- `cargo bake cargo:packages` lists exactly the published crates above, each at the workspace version.
- `cargo publish --workspace --dry-run --locked` packages and builds every published crate from its packaged sources.
- A crate's first upload, `cargo bake cargo:bootstrap PACKAGE`, follows the order of the crates' dependencies, including dev-dependencies that name a version, as crates.io needs them to exist; `readme.md`'s Releasing section lists it.
- Inspect the manifests: each crate except `bake/` inherits `version.workspace = true`, each dependency on another workspace crate carries `version` and `path`, and each published crate has the metadata above.
- `target/debug/tomori-server` and `target/debug/apk/tomori.apk` are still what `cargo build -p tomori-server` and `bash scripts/android.sh build` produce.
