# `tomori-prometheus`

A generic Prometheus data source for [Tomori](https://github.com/base-rs/tomori) environments' server sides: a blocking client for the Prometheus HTTP API's instant and range queries, with an endpoint and a renewable bearer token that the application supplies and that never reach clients, bounds on time and size, typed results with explicit timestamps, and a label filter that keeps only the labels a server side may pass on to clients or the agent.

## Usage

```sh
cargo add tomori-prometheus
```

Build a `Client` in an environment's server side, which runs on a thread of its own and may block, with the endpoint and a token source from the application's own configuration, never from a client:

```rust,no_run
use std::time::Duration;
use tomori_prometheus::{Call, Client, Error, LabelFilter, Range, Timestamp, Token};

fn main() -> Result<(), Error> {
    // The application's own token source, such as a command that prints an identity token.
    let token = || {
        let output = std::process::Command::new("print-token")
            .output()
            .map_err(|_| "could not run print-token".to_owned())?;
        Token::new(&String::from_utf8_lossy(&output.stdout))
            .ok_or_else(|| "print-token printed no token".to_owned())
    };
    let client = Client::builder("https://prometheus.example")
        .token_source(token)
        .labels(LabelFilter::only(["namespace", "pod"]))
        .timeout(Duration::from_secs(15))
        .concurrency(4)
        .build()?;

    // Evaluate every query of one poll at the same instant, so that they agree.
    let at = Timestamp::now().floor(Duration::from_secs(5));
    let response = client.query("sum by (namespace, pod) (rate(http_requests_total[1m]))", at)?;
    for sample in response.data.into_vector().unwrap_or_default() {
        println!("{} {}: {}", sample.metric["namespace"], sample.metric["pod"], sample.point.value);
    }

    // A slow read of the whole fleet, with a longer time-out of its own, leaving a slot for quick reads on other threads.
    let fleet = Call::new().timeout(Duration::from_secs(60)).background();
    let range = Range::new(at - Duration::from_secs(3600), at, Duration::from_secs(60));
    for series in client.query_range_with("up", range, fleet)?.data {
        println!("{:?}: {} points", series.metric, series.points.len());
    }
    Ok(())
}
```

Another client of the application's, such as one for another API of the same gateway, sends the token the same source makes with `Token::expose`, the one way to read it, only in its `authorization` field, never logging or printing it. Calling `token()` may run a command each time, and that client gets none of this one's care, so it keeps the token for its lifetime and asks again only when it expires or is refused with HTTP 401:

```rust
use tomori_prometheus::{Token, TokenSource};

fn authorization(source: &dyn TokenSource) -> Result<String, String> {
    Ok(format!("Bearer {}", source.token()?.expose()))
}

assert_eq!(authorization(&Token::new("a-token").unwrap()).unwrap(), "Bearer a-token");
```

[Using the Prometheus Source](context/usage.md) describes the client's configuration, its errors and the PromQL helpers. Its requirements are in Tomori's [`design/data-sources.md`](https://github.com/base-rs/tomori/blob/main/design/data-sources.md). The interface is 0.x, and grows with Tomori's data sources ([#83](https://github.com/base-rs/tomori/issues/83)).

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-environment`](https://crates.io/crates/tomori-environment), the interface of the server sides that read data sources.
