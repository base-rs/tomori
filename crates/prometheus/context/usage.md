# Using the Prometheus Source

`tomori-prometheus` reads a Prometheus-compatible endpoint from an environment's server side. Everything it is configured with, the endpoint, the token and the queries, comes from the application, Tomori's own or another project's, and stays in the server: clients and the agent are sent only what the server side chooses to send, with the labels its filter keeps.

## The client

`Client::builder(endpoint)` takes `https://HOST[:PORT][/PATH]`, or `http://` only to a loopback host, such as a test's fake. Before `build()`:

- `token_source(source)` authenticates with bearer tokens. A `Token` is a fixed source, and any `Fn() -> Result<Token, String>` is one, such as a closure that runs a command. The client keeps a token for `token_lifetime` (40 minutes by default), and asks again after it has passed, or once after the endpoint refuses a request with HTTP 401, which it then retries once. Another client of the application's can send the same token: call the source's `token()` and send `Token::expose()` as `Bearer TOKEN` in its `authorization` field, replacing it in its own errors, never logging, printing or storing it; a `Token`'s `Debug` hides it, and it has no `Display`.
- `timeout` (30 seconds, at most a day) bounds each request, from when it is queued until its answer is read, and is sent as Prometheus's evaluation `timeout`. A call refused with HTTP 401 sends a second request, with a time-out of its own, after asking the token source, whose time is not bounded here.
- `max_response_bytes` (8 MiB) and `max_query_bytes` (16 KiB) bound answers and expressions.
- `api_prefix` (`/api/v1`) is where the API is beneath the endpoint, for a gateway that serves it elsewhere.
- `method` sends queries form-encoded in a `POST` (the default) or in the URL of a `GET`.
- `labels(LabelFilter::only([...]))` keeps only the labels listed, removing the rest from every answer; without it, every label is kept.
- `user_agent` replaces `tomori-prometheus/VERSION`.

The client runs its requests one at a time, on a thread of its own, over one connection that it keeps for the next request and replaces after any failure, trying each of the host's addresses in turn, the one that last connected first, each but the last within its share of the time-out; clones share it. Redirects are not followed. Dropping the last clone stops the thread, waiting for it for at most `DROP_WAIT`, a second, since a lookup stuck in the system's resolver cannot be interrupted.

## Queries

- `query(expression, at)` is an instant query at a `Timestamp`. Evaluate every query of one poll at the same `at`, such as `Timestamp::now().floor(step)`, so that they agree.
- `query_range(expression, Range::new(start, end, step))` is a range query, of at most 11,000 points per series.

Answers are `Response`s, the data with Prometheus's warnings. An instant query's `Data` is a `Vector` of `Sample`s (`into_vector()`), a `Matrix` of `Series`, a `Scalar` or a `String`; a range query's is its series. Each `Point` has its own timestamp, to the millisecond, and its value, `NaN` and infinities included. Samples of native histograms are left out, keeping float samples, with a warning saying how many: wrap them in `histogram_count()`, `histogram_sum()` or `histogram_quantile()` to read them.

## Building queries

`promql::quote` makes a string literal of a value read from elsewhere, such as a pod's name, so that it cannot change the query's meaning; `promql::escape_regex` escapes it for a regular expression matcher, inside `quote`. `promql::batch` joins expressions with `or`, each on a line of its own so that a `# comment` cannot swallow the next, into as few queries as fit a bound, such as `max_query_bytes`. Join only instant vectors, since `or` refuses a scalar (wrap one in `vector()`), and give each expression's series a label of its own, with `label_replace`, since `or` keeps only the left of two series with the same labels.

## Describing a metric to the agent

When the space's agent inspects something a server side presents from Prometheus, such as a chart, through Tomori's `inspect` tool, describe it with `Metric`: `Metric::range(datasource, query, range)` or `Metric::instant(datasource, query, at)`, with `with_labels`, `with_latest`, `with_unit` and `with_link`, and answer with `to_json()`. `datasource` is a name of the application's choosing, such as `production`: `Metric` has no field for the endpoint or the token, so neither can reach the agent through it. Give it only the labels the client's label filter keeps; a link with user information, or that is not `http://` or `https://`, is refused. Times are written in UTC as RFC 3339, with `Timestamp::rfc3339`, and `NaN` and infinities as Prometheus writes them.

## Errors

Every failure is an `Error`: `Invalid` configuration or arguments, `Transport`, `Timeout`, `Status` (with an excerpt of the answer and any `retry-after`), `Auth`, `Parse`, `Limit`, or `Query`, Prometheus's own refusal with its HTTP status, any `retry-after` and its `errorType`; `status()` and `retry_after()` read them from either. An error's own words never name the token or the endpoint; text it quotes from the transport or the source has the token replaced with `[redacted]` and the endpoint's host and path, wherever each stands whole, with `<endpoint>` (a host shorter than three characters, and a path shorter than eight or made only of `api`, `v1`, `v2` and `v3` segments, such as `/api/v1`, are left alone), but may hold anything else the source wrote, unfiltered by the label filter. A server side's error is shown to everyone in its space, visitors included, so say what failed in your own words rather than passing an `Error`'s text on. The client never retries a 429, a 5xx or a request on a connection the source has just closed: poll at your own cadence, waiting at least the `retry-after` the source gave.
