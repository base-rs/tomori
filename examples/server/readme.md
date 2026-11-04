# Example Server

An example of a server that another project builds on [`tomori-server`](../../crates/server/readme.md), without changing Tomori. It offers a placeholder backend, which repeats each prompt back, and a placeholder environment, which tells the agent it presents nothing yet, beside Tomori's own, registered through `tomori_server::Server::builder()`.

## Usage

Its executable takes the same command line as `tomori-server`:

```sh
cargo run -p tomori-example-server -- --backend placeholder --auth open --no-advertise
```

A client then offers the placeholder backend and, after Sky and Grid, the Placeholder environment, which clients show as the Minimal grid since none has its client half. Its tests run the executable and create a space with both.

See the Building a Tomori Server guide in [`crates/server/context/`](../../crates/server/context/getting-started.md).
