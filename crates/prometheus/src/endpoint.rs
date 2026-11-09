// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where a client sends its requests, read from the endpoint's URL. Errors never quote the URL, whose path may hold a secret of its own.
use std::net::IpAddr;

use crate::Error;

/// A parsed endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub https: bool,
    /// The host, without an IPv6 literal's brackets, which is resolved and named to TLS.
    pub host: String,
    pub port: u16,
    /// The host and port as given, for each request's authority.
    pub authority: String,
    /// The path before the API's prefix, without a trailing `/`; empty for none.
    pub base: String,
}

fn invalid(why: &str) -> Error {
    Error::Invalid(format!("the endpoint {why}"))
}

/// `url`, `https://HOST[:PORT][/PATH]`, or `http://` only to a loopback host.
pub(crate) fn parse(url: &str) -> Result<Endpoint, Error> {
    let (https, rest) = match url.split_once("://") {
        Some(("https", rest)) => (true, rest),
        Some(("http", rest)) => (false, rest),
        _ => return Err(invalid("is not an https:// URL")),
    };
    if rest.contains(['?', '#']) {
        return Err(invalid("has a query or fragment"));
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if path.contains(|character: char| !character.is_ascii_graphic()) {
        return Err(invalid("has an invalid path"));
    }
    if authority.contains('@') {
        return Err(invalid(
            "has user information; give credentials as a token source",
        ));
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(literal) => {
            let (host, after) = literal
                .split_once(']')
                .ok_or(invalid("has an unclosed IPv6 address"))?;
            let port = match after.strip_prefix(':') {
                Some(port) => Some(port),
                None if after.is_empty() => None,
                None => return Err(invalid("has text after its IPv6 address")),
            };
            (host, port)
        }
        None => match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(invalid("has an invalid port"))?,
        None => {
            if https {
                443
            } else {
                80
            }
        }
    };
    if host.is_empty() || host.contains(|character: char| !character.is_ascii_graphic()) {
        return Err(invalid("has no valid host"));
    }
    let loopback = host == "localhost"
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !https && !loopback {
        return Err(invalid(
            "uses http:// to a host that is not loopback; use https://",
        ));
    }
    Ok(Endpoint {
        https,
        host: host.to_owned(),
        port,
        authority: authority.to_owned(),
        base: format!("/{path}").trim_end_matches('/').to_owned(),
    })
}

/// `prefix`, an API's prefix such as `/api/v1`, without a trailing `/`, or why it cannot be used.
pub(crate) fn prefix(prefix: &str) -> Result<String, Error> {
    let valid = (prefix.is_empty() || prefix.starts_with('/'))
        && prefix
            .chars()
            .all(|character| character.is_ascii_graphic() && !matches!(character, '?' | '#'));
    if !valid {
        return Err(Error::Invalid(
            "the API prefix must be empty or a path beginning with /".into(),
        ));
    }
    Ok(prefix.trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_parsed() {
        let endpoint = parse("https://observe.example/prometheus/").unwrap();
        assert_eq!(
            endpoint,
            Endpoint {
                https: true,
                host: "observe.example".into(),
                port: 443,
                authority: "observe.example".into(),
                base: "/prometheus".into(),
            }
        );
        let endpoint = parse("http://127.0.0.1:9090").unwrap();
        assert_eq!(
            (endpoint.https, endpoint.port, endpoint.base.as_str()),
            (false, 9090, "")
        );
        let endpoint = parse("http://[::1]:9090/").unwrap();
        assert_eq!(
            (endpoint.host.as_str(), endpoint.authority.as_str()),
            ("::1", "[::1]:9090")
        );
        let endpoint = parse("http://localhost").unwrap();
        assert_eq!((endpoint.host.as_str(), endpoint.port), ("localhost", 80));
        assert_eq!(parse("https://[2001:db8::1]").unwrap().port, 443);
    }

    #[test]
    fn unsafe_or_unreadable_endpoints_are_refused_without_being_quoted() {
        for (url, why) in [
            ("ftp://x", "is not an https:// URL"),
            ("observe.example", "is not an https:// URL"),
            ("https://x/?key=secret", "has a query or fragment"),
            (
                "https://user:secret@x",
                "has user information; give credentials as a token source",
            ),
            ("https://[::1", "has an unclosed IPv6 address"),
            ("https://x:0", "has an invalid port"),
            ("https://x:http", "has an invalid port"),
            ("https://:443", "has no valid host"),
            ("https:///path", "has no valid host"),
            ("https://ex ample", "has no valid host"),
            ("http://[::1]evil", "has text after its IPv6 address"),
            ("http://::1:9090", "has an invalid port"),
            ("http://127.0.0.1:9090:1", "has an invalid port"),
            ("https://x/a b", "has an invalid path"),
            ("https://x/a\tb", "has an invalid path"),
            ("https://x/caf\u{e9}", "has an invalid path"),
            (
                "http://observe.example",
                "uses http:// to a host that is not loopback; use https://",
            ),
            (
                "http://10.0.0.1",
                "uses http:// to a host that is not loopback; use https://",
            ),
        ] {
            let error = parse(url).unwrap_err();
            assert_eq!(
                error,
                Error::Invalid(format!("the endpoint {why}")),
                "{url}"
            );
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn prefixes_are_paths() {
        assert_eq!(prefix("/api/v1/").unwrap(), "/api/v1");
        assert_eq!(prefix("").unwrap(), "");
        for bad in ["api/v1", "/api?x", "/api#x", "/a b"] {
            assert_eq!(
                prefix(bad),
                Err(Error::Invalid(
                    "the API prefix must be empty or a path beginning with /".into()
                ))
            );
        }
    }
}
