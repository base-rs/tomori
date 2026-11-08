# `tomori-security`

Authentication, tokens, pairing and certificate pinning for [Tomori](https://github.com/ioquatix/tomori), shared by the Tomori server and its clients.

## Usage

```sh
cargo add tomori-security
```

A client pins the server's self-signed certificate by its `Fingerprint`, and builds a `rustls` configuration that accepts only that certificate, or the first one seen when none is pinned yet, with `client_config(Pinned::new(pin))`. The `pairing` module derives the code both sides show while pairing a device with a server, and its `ChosenName` makes the name a pairing device chose for itself safe to show, carrying no numeral or number word.

The `issue` feature adds the server's side: the `TokenStore`, which issues, lists, revokes and authenticates tokens, and the server's `ServerCertificate`. Authenticators report who a connection is, as a `Principal`, never what it may do; the server keeps roles against principals.

Most clients use [`tomori-client`](https://crates.io/crates/tomori-client), which applies tokens and pinning itself. The model is described in [`design/security.md`](https://github.com/ioquatix/tomori/blob/main/design/security.md).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client) and [`tomori-server`](https://crates.io/crates/tomori-server), which use it.
