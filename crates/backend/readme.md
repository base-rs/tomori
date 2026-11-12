# `tomori-backend`

The backend-neutral interface between the [Tomori](https://github.com/base-rs/tomori) server and coding agents. A backend creates and attaches agent sessions, and reports what happens in them as structured `Event`s, such as messages, workers, permissions and questions. It never produces text for display or speech: the server owns that.

## Usage

```sh
cargo add tomori-backend
```

Implement `Backend`, whose sessions implement `Session`, to drive another agent from Tomori. [`tomori-backend-opencode`](https://crates.io/crates/tomori-backend-opencode) is the implementation for OpenCode. The interface's requirements are described in [`design/agent-backends.md`](https://github.com/base-rs/tomori/blob/main/design/agent-backends.md).

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-server`](https://crates.io/crates/tomori-server), which runs backends' sessions in its spaces.
