# Naming

**Tomori** (`ともり`, with `灯` as a light-associated name spelling) is the
name of both the system and its conversational coordinator. One cohesive name
connects the shared workspace to the presence you speak with inside it.

The lamplight association suggests a steady central presence: a light to gather
around. The intended spelling is **Tomori**, not "Tamori".

## Theme

A Japanese-inspired visual direction places a small shrine near the central
coordinator, with floating clouds, a night sky and a distant moon. The initial
**Sky** environment implements a procedural blockout of that direction with a
fixed moonlit sky. Detailed architecture and a cycle in a chosen timezone remain
future design options, not implemented features.

## crates.io

Read-only checks of the crates.io API on 2026-11-03, each a `GET` of `https://crates.io/api/v1/crates/NAME` with a `User-Agent`, returned HTTP 404 for every name Tomori uses or plans to use: no registered crate was found at the time of checking. A check of `serde` returned 200, confirming the API answered. A search for `tomori` found only `kotomori`, which is unrelated.

- To be published: `tomori-protocol`, `tomori-client`, `tomori-discovery`, `tomori-security`, `tomori-backend`, `tomori-backend-opencode`, `tomori-speech`, `tomori-server` and `tomori-environment-sky`, which a check on 2026-11-05 found still free (HTTP 404, with `serde` returning 200); and `tomori-scene`, `tomori-render`, `tomori-client-desktop`, `tomori-client-spectator` and `tomori-client-quest`, which a check later on 2026-11-05 found free too.
- `tomori-environment`, the interface of an environment's server side, which a check on 2026-11-07 found still free (HTTP 404), is to be published too.
- `tomori-prometheus`, a generic Prometheus data source for environments' server sides, which a check on 2026-11-09 found free (HTTP 404, with `serde` returning 200), is to be published too.
- Planned in [#97](https://github.com/ioquatix/tomori/issues/97): `tomori-physics`.
- The bare name `tomori`, which no package uses: the headset's library and executable are named `tomori`, but its package is `tomori-client-quest`.

None of these names has been reserved or published. Recheck them before the first publication; this is not a trademark or wider availability check. An earlier check, on 2026-10-28, found `tomori` free too.

## Scope

The public repository uses `ioquatix/tomori`. Technical names now match:

- Rust packages: `tomori-` followed by the crate's role, such as `tomori-protocol` and `tomori-server`, in folders under `crates/` without the prefix, such as `crates/protocol`. Rust paths use the crate names, such as `tomori_protocol::ClientMessage`.
- Headset and desktop application: package `tomori-client-quest` (in `crates/client-quest`); its library and executable are `tomori`.
- Server executable: `tomori-server`, from the package of the same name.
- Spectator executable: `tomori-spectator`, from the package `tomori-client-spectator` (in `crates/client-spectator`); `tomori --spectator…` runs it too.
- Desktop crate: `tomori-client-desktop` (in `crates/client-desktop`), which the `tomori` executable's desktop modes run.
- Android application ID: `com.codeotaku.tomori`; label: **Tomori**;
  APK: `tomori.apk`. A headset build of another project's has its own (see `crates/client-quest/context/android.md`); the example's is `com.example.tomori.headset`.
- Node bridge package: `tomori-opencode-bridge`.
- Environment variables: `TOMORI_*`.
- PipeWire video: `tomori-spectator` (**Tomori-Spectator**).
- PipeWire audio: `tomori-quest-microphone` and `tomori-assistant-voice`.

### Migrating to `com.codeotaku.tomori`

Builds before November 2026 used the Android ID `io.github.ioquatix.tomori`. The
new ID installs alongside that app rather than updating it, so the new app starts
without what the old one stored in its private storage: its participant identity,
the server it last joined, each server's pinned certificate and token, and its
feedback setting, as well as its granted permissions. Pair it again, with the
gold key beside the server and the 6-digit code it shows, or with
`TOMORI_TOKEN=… bash scripts/android.sh run`, and allow the microphone when
asked. The server sees it as a new participant. Remove the old app only when
ready:

```sh
adb uninstall io.github.ioquatix.tomori
```

### Migrating from the prototype name

Earlier builds used `base-agent-swarm` and Android ID `dev.base.agentswarm`.
The new Android ID installs alongside the old app rather than updating it;
app data and permissions are separate. Remove the old app only when ready.

Rename any local `BASE_SWARM_*` environment exports to `TOMORI_*` (including
those in `.android-env` or service definitions); the old prefix is no longer
read. Stop old services normally, then rebuild/restart with the new names.
Ports and HTTP routes are unchanged.

Reselect the renamed video/audio sources in OBS or other PipeWire consumers.
The bundled OBS compatibility patch now matches `tomori-spectator`; a plugin
using the earlier exact-name patch needs rebuilding, or use the documented
manual target/link method. The separate plugin fork's generic **Video/Source**
discovery option needs no name-specific change.

Name-reading reference: [Weblio's entry for 灯](https://ejje.weblio.jp/content/灯).
