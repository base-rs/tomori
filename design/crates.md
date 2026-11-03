# Crates

This describes how Tomori's workspace crates are named, versioned and published.

## Requirement

Every workspace crate except the private `bake/` package is named `tomori-` followed by its role, such as `tomori-protocol`, and lives in a folder under `crates/` named without the prefix, such as `crates/protocol`. Rust paths use the crate names, such as `tomori_protocol::ClientMessage`.

Names visible to people running Tomori do not change with package names: the server executable is `tomori-server`, and its command-line interface is unchanged by packaging; the headset's library and executable are `tomori`, its Android application ID is `io.github.ioquatix.tomori`, and its APK is `tomori.apk`.

The crates are released together: they share one version, `[workspace.package] version` in the root `Cargo.toml`, one `releases.md` and one release tag. Every dependency of one workspace crate on another names that version as well as its path.

These crates are published to crates.io, and every one of them can be packaged and verified with `cargo publish --dry-run`:

| Crate | Why it is published |
| --- | --- |
| `tomori-protocol` | Every client and server needs it, including implementations outside this repository. |
| `tomori-client` | The client core, for other clients and for headset builds outside this repository. |
| `tomori-discovery` | Needed by `tomori-client` and the server. |
| `tomori-security` | Needed by `tomori-client` and the server. |
| `tomori-backend` | The interface other projects implement to add an agent backend. |
| `tomori-backend-opencode` | The default backend, needed by the server's `opencode` feature. |
| `tomori-speech` | Needed by the server's default `speech` feature. |
| `tomori-server` | `cargo install tomori-server`, and the library that downstream servers will build on. |

These stay unpublished, with `publish = false`:

| Crate | Why it is not published |
| --- | --- |
| `tomori-quest` | The headset and desktop application, released as an APK. Downstream headset builds need it as a library first. |
| `tomori-environment-sky` | Only `tomori-quest` uses it, and its interface will change when environments get a shared interface. |
| `tomori-bake` | Development tasks only. |

A published crate depends only on published crates and on crates.io packages, never on an unpublished crate, a path outside its folder or a Git repository. Each published crate has a description, license, repository, readme, keywords and categories.

## Rationale

Other projects need to build their own server and headset with their own environments, data sources and tools, without forking Tomori. That needs Tomori's crates on crates.io under names that cannot collide with other packages in its flat namespace, released at one version so that a downstream project can depend on a matching set. The short folder names keep paths in the repository readable, as Base's own workspaces do. The executable, library and Android names are what people and scripts already use, so packaging must not break them. Proposed in [#73](https://github.com/ioquatix/tomori/issues/73) in November 2026; when to publish for the first time is still the maintainer's decision. The names were found free on crates.io on 2026-11-03; see `docs/naming.md`.

The headset stays unpublished until its library has a builder that other projects can use, which is also planned in #73; until then only the APK is useful. `tomori-environment-sky` waits for the environment interface planned in [#97](https://github.com/ioquatix/tomori/issues/97).

## Scope

Workspace members, their manifests, and the Rust paths, scripts, workflows and documents that name them.

## Validation

- `cargo bake cargo:packages` lists exactly the published crates above, each at the workspace version.
- `cargo publish --workspace --dry-run --locked` packages and builds every published crate from its packaged sources.
- Inspect the manifests: each crate inherits `version.workspace = true`, each dependency on another workspace crate carries `version` and `path`, and each published crate has the metadata above.
- `target/debug/tomori-server` and `target/debug/apk/tomori.apk` are still what `cargo build -p tomori-server` and `bash scripts/android.sh build` produce.
