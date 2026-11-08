// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What can go wrong, with what an error quotes scrubbed of the token and the endpoint.
use std::fmt;
use std::time::Duration;

/// The most of a source's or transport's own text, in bytes, that an error or warning quotes.
pub const QUOTED_BYTES: usize = 512;

/// What replaces the token in quoted text.
pub const REDACTED: &str = "[redacted]";

/// What replaces the endpoint's host, and its path, in quoted text.
pub const ENDPOINT: &str = "<endpoint>";

/// Why a call failed.
///
/// An error's own words never name the token or the endpoint. Where it quotes text from the transport or the source, such as a TLS failure, Prometheus's message or an excerpt of an error page, that text has the token replaced with [`REDACTED`] and the endpoint's host and path with [`ENDPOINT`], and is cut to [`QUOTED_BYTES`]. The host is replaced wherever it stands as a whole name, ignoring case, with any trailing dot and port, but not inside a longer name or word, such as `prometheus` in `prometheus_http_requests`; the path is replaced as a whole path, plain or percent-encoded. A host shorter than three characters is not replaced at all, since it would be found inside unrelated words, nor a path shorter than eight characters or made only of `api`, `v1`, `v2` and `v3` segments, since it would be found inside the API's own paths. The text may still hold anything else the source wrote, such as the query or label values, which no [`LabelFilter`](crate::LabelFilter) has filtered. A server side's error is shown to everyone in its space, visitors included, so it should say what failed in its own words rather than pass this text on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The client's configuration, or a call's arguments, cannot be used, such as an endpoint that is not `https://` or a range query whose step is zero.
    Invalid(String),
    /// The source could not be reached, or the exchange failed part way, such as on a connection the source had just closed.
    Transport(String),
    /// The call did not finish within the client's time-out.
    Timeout(Duration),
    /// The source answered with an HTTP status other than success, and not with Prometheus's own error, with an excerpt of its answer and the `retry-after` it gave, if any, as for a 429 or a 503.
    Status {
        status: u16,
        retry_after: Option<Duration>,
        detail: String,
    },
    /// The source refused the token, with HTTP 401 again after a new token, or 403, or the token source gave none.
    Auth(String),
    /// The answer could not be read, such as one that is not JSON.
    Parse(String),
    /// A bound was passed: a query too large to send, a range with too many points, or an answer too large to read.
    Limit(String),
    /// Prometheus refused the query with its own error: the HTTP status, such as 400, 422 or 503, any `retry-after`, as for a 429 or 503, its `errorType`, such as `bad_data`, `execution`, `timeout` or `unavailable`, and its message.
    Query {
        status: u16,
        retry_after: Option<Duration>,
        error_type: String,
        message: String,
    },
}

impl Error {
    /// The HTTP status the source answered with, for [`Error::Status`] and [`Error::Query`].
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } | Self::Query { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// How long the source asked to be left before it is asked again, for [`Error::Status`] and [`Error::Query`].
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Status { retry_after, .. } | Self::Query { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid Prometheus request: {message}"),
            Self::Transport(message) => write!(formatter, "could not reach Prometheus: {message}"),
            Self::Timeout(timeout) => write!(
                formatter,
                "Prometheus did not answer within {:.1} seconds",
                timeout.as_secs_f64()
            ),
            Self::Status { status, detail, .. } => {
                write!(formatter, "Prometheus answered HTTP {status}: {detail}")
            }
            Self::Auth(message) => write!(formatter, "Prometheus authentication failed: {message}"),
            Self::Parse(message) => {
                write!(formatter, "could not read Prometheus's answer: {message}")
            }
            Self::Limit(message) => write!(formatter, "Prometheus limit: {message}"),
            Self::Query {
                status,
                error_type,
                message,
                ..
            } => write!(
                formatter,
                "Prometheus refused the query (HTTP {status}, {error_type}): {message}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// What quoted text is scrubbed of: the token, if any, and the endpoint's host and path.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Scrub<'a> {
    pub token: Option<&'a str>,
    pub host: &'a str,
    /// The endpoint's path, empty for none.
    pub base: &'a str,
}

/// The shortest host that is scrubbed: anything shorter would be replaced inside unrelated words.
pub(crate) const SCRUBBED_LEAST: usize = 3;

/// The shortest path that is scrubbed: anything shorter, such as `/v1` or `/api/v1`, would be replaced inside the paths of Prometheus's own API.
pub(crate) const SCRUBBED_PATH_LEAST: usize = 8;

/// Whether `base` is a path worth scrubbing: long enough, and not made only of segments common to APIs, such as `/api/v1/v2`.
fn scrubbed_path(base: &str) -> bool {
    base.len() >= SCRUBBED_PATH_LEAST
        && !base
            .split('/')
            .filter(|segment| !segment.is_empty())
            .all(|segment| matches!(segment, "api" | "v1" | "v2" | "v3"))
}

/// Whether `character` may be part of a host name, so that a host found beside it is part of a longer name or word, such as `prometheus` in `prometheus_http_requests`.
fn in_host(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
}

/// Whether `character` may continue a path's last segment, so that a path found before it is only part of a longer one.
fn in_segment(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~')
}

/// `text` with every whole occurrence of `needle`, ignoring ASCII case, replaced with [`ENDPOINT`]: one with no `joined_before` character before it, and no `joined_after` character after it once `suffix` has taken what may follow it, such as a host's trailing dot and port.
fn replace(
    text: &str,
    needle: &str,
    suffix: fn(&str) -> usize,
    joined_before: fn(char) -> bool,
    joined_after: fn(char) -> bool,
) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut replaced = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = lower[from..].find(&needle) {
        let start = from + found;
        let end = start + needle.len();
        let end = end + suffix(&text[end..]);
        let before = text[..start].chars().next_back().is_some_and(joined_before)
            && !escaped(&text[..start]);
        let beyond = text[end..].chars().next().is_some_and(joined_after);
        if before || beyond {
            from = start + 1;
            continue;
        }
        replaced.push_str(&text[copied..start]);
        replaced.push_str(ENDPOINT);
        copied = end;
        from = end;
    }
    replaced.push_str(&text[copied..]);
    replaced
}

/// Whether `text` ends with a percent-encoded character, such as the `%2F` before a host in a URL that is itself encoded, which separates what follows from what precedes it.
fn escaped(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3
        && bytes[bytes.len() - 3] == b'%'
        && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_hexdigit)
}

/// What may follow a host and still be the endpoint: a trailing dot, unless it begins a longer name, and a port.
fn host_suffix(rest: &str) -> usize {
    let dot = usize::from(
        rest.starts_with('.') && !rest[1..].starts_with(|character: char| in_host(character)),
    );
    let digits = rest[dot..].strip_prefix(':').unwrap_or("");
    let port = digits.bytes().take_while(u8::is_ascii_digit).count();
    dot + if port > 0 { port + 1 } else { 0 }
}

/// Nothing before a path joins it to a word, since it begins with `/`.
fn never(_: char) -> bool {
    false
}

/// A path is taken as it is.
fn nothing(_: &str) -> usize {
    0
}

impl Scrub<'_> {
    /// `text`, safe to quote: without the token, the endpoint's path, in plain or percent-encoded form, or its host, each found only as a whole and ignoring case, on one line, and cut to [`QUOTED_BYTES`]. A host shorter than [`SCRUBBED_LEAST`] characters, or a path shorter than [`SCRUBBED_PATH_LEAST`] or made only of `api`, `v1`, `v2` and `v3` segments, is not scrubbed.
    pub(crate) fn quote(&self, text: &str) -> String {
        let mut text = text.to_owned();
        if let Some(token) = self.token {
            text = text.replace(token, REDACTED);
        }
        if scrubbed_path(self.base) {
            let encoded: String = form_urlencoded::byte_serialize(self.base.as_bytes()).collect();
            for base in [self.base, encoded.as_str()] {
                text = replace(&text, base, nothing, never, in_segment);
            }
        }
        if self.host.len() >= SCRUBBED_LEAST {
            text = replace(&text, self.host, host_suffix, in_host, in_host);
        }
        let mut quoted = String::new();
        for character in text.trim().chars() {
            if quoted.len() + character.len_utf8() > QUOTED_BYTES {
                break;
            }
            quoted.push(if character.is_control() {
                ' '
            } else {
                character
            });
        }
        quoted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_failed() {
        let cases = [
            (Error::Invalid("x".into()), "invalid Prometheus request: x"),
            (
                Error::Transport("x".into()),
                "could not reach Prometheus: x",
            ),
            (
                Error::Timeout(Duration::from_millis(1500)),
                "Prometheus did not answer within 1.5 seconds",
            ),
            (
                Error::Status {
                    status: 503,
                    retry_after: None,
                    detail: "x".into(),
                },
                "Prometheus answered HTTP 503: x",
            ),
            (
                Error::Auth("x".into()),
                "Prometheus authentication failed: x",
            ),
            (
                Error::Parse("x".into()),
                "could not read Prometheus's answer: x",
            ),
            (Error::Limit("x".into()), "Prometheus limit: x"),
            (
                Error::Query {
                    status: 422,
                    retry_after: None,
                    error_type: "execution".into(),
                    message: "x".into(),
                },
                "Prometheus refused the query (HTTP 422, execution): x",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn statuses_and_retry_afters_are_found_whatever_the_answer() {
        let wait = Some(Duration::from_secs(9));
        let status = Error::Status {
            status: 429,
            retry_after: wait,
            detail: String::new(),
        };
        let query = Error::Query {
            status: 503,
            retry_after: wait,
            error_type: "unavailable".into(),
            message: String::new(),
        };
        assert_eq!((status.status(), status.retry_after()), (Some(429), wait));
        assert_eq!((query.status(), query.retry_after()), (Some(503), wait));
        let timeout = Error::Timeout(Duration::from_secs(1));
        assert_eq!((timeout.status(), timeout.retry_after()), (None, None));
    }

    #[test]
    fn quoted_text_hides_the_token_and_endpoint_and_stays_short() {
        let scrub = Scrub {
            token: Some("s3cret"),
            host: "observe.example",
            base: "/gateway/prometheus",
        };
        assert_eq!(
            scrub.quote(" Bearer s3cret was\nrefused "),
            "Bearer [redacted] was refused"
        );
        // As rustls names the host when a certificate does not match it.
        assert_eq!(
            scrub.quote(
                "invalid peer certificate: certificate not valid for name \"observe.example\"; certificate is only valid for DnsName(\"*.other.example\")"
            ),
            "invalid peer certificate: certificate not valid for name \"<endpoint>\"; certificate is only valid for DnsName(\"*.other.example\")"
        );
        // As a gateway's error page names the URL it could not serve, twice, in another case, with its port and percent-encoded.
        assert_eq!(
            scrub.quote(
                "<p>https://Observe.Example:8443/gateway/prometheus/api/v1/query failed; see https://status.example/?url=https%3A%2F%2Fobserve.example.%2Fgateway%2Fprometheus</p>"
            ),
            "<p>https://<endpoint><endpoint>/api/v1/query failed; see https://status.example/?url=https%3A%2F%2F<endpoint><endpoint></p>"
        );
        // Only whole names and paths: not a longer host, a word or a longer path.
        assert_eq!(
            scrub.quote("api.observe.example observe.example-2 observe.examples /gateway/prometheus2 observe.example: observe.example."),
            "api.observe.example observe.example-2 observe.examples /gateway/prometheus2 <endpoint>: <endpoint>"
        );
        // Common API prefixes are left alone, so that Prometheus's own paths are not mangled.
        for base in ["/v1", "/api/v1", "/api/v1/v2"] {
            let prefix = Scrub {
                token: None,
                host: "observe.example",
                base,
            };
            assert_eq!(prefix.quote("POST /api/v1/query"), "POST /api/v1/query");
        }
        let unique = Scrub {
            token: None,
            host: "observe.example",
            base: "/observe-gateway",
        };
        assert_eq!(
            unique.quote("POST /observe-gateway/api/v1/query"),
            "POST <endpoint>/api/v1/query"
        );
        let short = Scrub {
            token: None,
            host: "prometheus",
            base: "/p",
        };
        assert_eq!(
            short.quote("prometheus_http_requests_total at prometheus:9090/p"),
            "prometheus_http_requests_total at <endpoint>/p"
        );
        let tiny = Scrub {
            token: None,
            host: "a",
            base: "",
        };
        assert_eq!(tiny.quote("a bad answer"), "a bad answer");
        let long = "é".repeat(QUOTED_BYTES);
        assert_eq!(tiny.quote(&long).len(), QUOTED_BYTES);
    }
}
