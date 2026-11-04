// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The TXT record and the translation of resolved services into [`Event`]s, kept free of sockets so that it can be unit-tested.

use std::net::{IpAddr, SocketAddr, SocketAddrV6};

use tomori_protocol::PROTOCOL_VERSION;

use crate::{
    FINGERPRINT_KEY, NAME_KEY, PARTICIPANTS_KEY, PROBLEMS_KEY, PROTOCOL_KEY, REPLIES_KEY,
    SERVICE_TYPE, SPACES_KEY, WAITING_KEY,
};

/// The longest DNS label, which bounds the instance name.
pub(crate) const INSTANCE_LIMIT: usize = 63;

/// The longest TXT string is 255 bytes, including `name=`.
const NAME_LIMIT: usize = 255 - NAME_KEY.len() - 1;

/// A server found by browsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    /// The DNS-SD instance name, unique on the network; it identifies the server in [`Event::Lost`].
    pub instance: String,
    /// The display name from the TXT record, or the instance name if the record has none.
    pub name: String,
    /// The multicast DNS host name, such as `tomori-studio.local.`.
    pub host: String,
    /// The server's addresses with its port, most preferred first: routable IPv4, routable IPv6, then link-local addresses. IPv6 link-local addresses carry the scope (interface index) they were found on.
    pub addresses: Vec<SocketAddr>,
    pub port: u16,
    /// The advertised protocol version, or `None` if the TXT record has no valid one.
    pub protocol: Option<u32>,
    /// The advertised number of spaces, or `None` if the TXT record has no valid one, as from a server that predates it.
    pub spaces: Option<u32>,
    /// The advertised number of connected participants, or `None` if the TXT record has no valid one.
    pub participants: Option<u32>,
    /// The advertised number of spaces waiting for an approval or an answer, or `None` if the TXT record has no valid
    /// one, as from a server that predates it or whose operator turned the attention counts off.
    pub waiting: Option<u32>,
    /// The advertised number of spaces with a problem nobody has seen, or `None` if unknown.
    pub problems: Option<u32>,
    /// The advertised number of spaces with a reply nobody has heard, or `None` if unknown.
    pub replies: Option<u32>,
    /// The fingerprint of the server's TLS certificate, as 64 lowercase hexadecimal digits, or `None` if it advertises none, as a server in `--auth open` mode does. Discovery is unauthenticated, so a client that has already pinned a certificate for the server keeps its pin.
    pub fingerprint: Option<String>,
}

/// What a server advertises about its use: see the crate documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// The spaces the server lists, closed ones included.
    pub spaces: u32,
    /// The participants connected, whether in a space or in the lobby.
    pub participants: u32,
    /// How many spaces need attention, at each level, or `None` to leave them out of the record.
    pub attention: Option<AttentionCounts>,
}

/// How many spaces need attention, by the level each needs most; see `design/spaces.md`. A space is counted once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttentionCounts {
    /// Spaces waiting for an approval or an answer.
    pub waiting: u32,
    /// Spaces with a problem that happened while nobody was in them.
    pub problems: u32,
    /// Spaces with a reply that finished while nobody was in them.
    pub replies: u32,
}

impl Server {
    /// The most preferred address as `host:port`, suitable for `tomori_client::Options::address`.
    pub fn address(&self) -> Option<String> {
        self.addresses.first().map(SocketAddr::to_string)
    }

    /// Whether this build of the client can talk to the server.
    pub fn is_compatible(&self) -> bool {
        self.protocol == Some(PROTOCOL_VERSION)
    }
}

/// A change in the set of servers on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A compatible server appeared, or its addresses or name changed. Repeated for the same instance as records update; treat it as an insert or replace.
    Found(Server),
    /// A server appeared whose protocol version differs from [`PROTOCOL_VERSION`] or is missing. It must not be offered for connection, but may be shown as unavailable.
    Incompatible(Server),
    /// The server with this instance name stopped advertising or expired.
    Lost { instance: String },
}

/// The TXT properties a server advertises.
pub(crate) fn properties(
    name: &str,
    protocol: u32,
    counts: Counts,
    fingerprint: Option<&str>,
) -> Vec<(String, String)> {
    let mut properties = vec![
        (PROTOCOL_KEY.to_owned(), protocol.to_string()),
        (NAME_KEY.to_owned(), truncate(name, NAME_LIMIT).to_owned()),
        (SPACES_KEY.to_owned(), counts.spaces.to_string()),
        (PARTICIPANTS_KEY.to_owned(), counts.participants.to_string()),
    ];
    if let Some(attention) = counts.attention {
        properties.extend([
            (WAITING_KEY.to_owned(), attention.waiting.to_string()),
            (PROBLEMS_KEY.to_owned(), attention.problems.to_string()),
            (REPLIES_KEY.to_owned(), attention.replies.to_string()),
        ]);
    }
    if let Some(fingerprint) = fingerprint {
        properties.push((FINGERPRINT_KEY.to_owned(), fingerprint.to_owned()));
    }
    properties
}

/// A fingerprint TXT value: 64 hexadecimal digits, in lowercase. Anything else is none.
fn fingerprint(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

/// A decimal `u32` TXT value; anything else, such as a sign, a fraction or an empty value, is unknown rather than a guess.
fn number(value: Option<&str>) -> Option<u32> {
    value.and_then(|value| value.trim().parse().ok())
}

/// `name` cut to at most `limit` bytes on a character boundary.
pub(crate) fn truncate(name: &str, limit: usize) -> &str {
    if name.len() <= limit {
        return name;
    }
    let mut end = limit;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// The instance name from a full service name such as `Studio\.lan._tomori._tcp.local.`, with DNS-SD escapes removed.
pub(crate) fn instance(fullname: &str) -> String {
    let escaped = fullname
        .strip_suffix(SERVICE_TYPE)
        .and_then(|name| name.strip_suffix('.'))
        .unwrap_or(fullname);
    let mut result = String::with_capacity(escaped.len());
    let mut characters = escaped.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                result.push(next);
            }
        } else {
            result.push(character);
        }
    }
    result
}

/// Most preferred first; see [`Server::addresses`].
fn rank(address: &SocketAddr) -> u8 {
    match address.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => 5,
        IpAddr::V4(ip) if ip.is_link_local() => 2,
        IpAddr::V4(_) => 0,
        IpAddr::V6(ip) if ip.is_loopback() => 5,
        IpAddr::V6(ip) if ip.is_unicast_link_local() => 3,
        IpAddr::V6(_) => 1,
    }
}

/// Socket addresses for `port`, ordered by preference. An IPv6 address is paired with the interface index it was found on, which link-local addresses need in order to connect.
pub(crate) fn addresses(
    found: impl IntoIterator<Item = (IpAddr, u32)>,
    port: u16,
) -> Vec<SocketAddr> {
    let mut addresses: Vec<SocketAddr> = found
        .into_iter()
        .map(|(ip, scope)| match ip {
            IpAddr::V4(ip) => SocketAddr::from((ip, port)),
            IpAddr::V6(ip) => {
                let scope = if ip.is_unicast_link_local() { scope } else { 0 };
                SocketAddr::V6(SocketAddrV6::new(ip, port, 0, scope))
            }
        })
        .collect();
    addresses.sort_by_key(|address| (rank(address), *address));
    addresses.dedup();
    addresses
}

/// The event for a resolved service, given a TXT lookup.
pub(crate) fn resolved<'a>(
    fullname: &str,
    host: &str,
    addresses: Vec<SocketAddr>,
    port: u16,
    property: impl Fn(&str) -> Option<&'a str>,
) -> Event {
    let instance = instance(fullname);
    let name = property(NAME_KEY)
        .filter(|name| !name.is_empty())
        .map_or_else(|| instance.clone(), str::to_owned);
    let server = Server {
        instance,
        name,
        host: host.to_owned(),
        addresses,
        port,
        protocol: number(property(PROTOCOL_KEY)),
        spaces: number(property(SPACES_KEY)),
        participants: number(property(PARTICIPANTS_KEY)),
        waiting: number(property(WAITING_KEY)),
        problems: number(property(PROBLEMS_KEY)),
        replies: number(property(REPLIES_KEY)),
        fingerprint: fingerprint(property(FINGERPRINT_KEY)),
    };
    if server.is_compatible() {
        Event::Found(server)
    } else {
        Event::Incompatible(server)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn event(txt: &HashMap<String, String>) -> Event {
        resolved(
            "Studio._tomori._tcp.local.",
            "tomori-studio.local.",
            vec![SocketAddr::from(([192, 168, 1, 10], 8765))],
            8765,
            |key| txt.get(key).map(String::as_str),
        )
    }

    /// The server a service resolved with the TXT properties `txt` is reported as, and whether it is compatible.
    fn reported(txt: &HashMap<String, String>) -> (bool, Server) {
        match event(txt) {
            Event::Found(server) => (true, server),
            Event::Incompatible(server) => (false, server),
            Event::Lost { .. } => unreachable!("a resolved service is found, compatible or not"),
        }
    }

    /// The server `txt` is reported as, which must be compatible.
    fn found(txt: &HashMap<String, String>) -> Server {
        let (compatible, server) = reported(txt);
        assert!(compatible, "{server:?}");
        server
    }

    /// The server `txt` is reported as, which must be incompatible.
    fn incompatible(txt: &HashMap<String, String>) -> Server {
        let (compatible, server) = reported(txt);
        assert!(!compatible, "{server:?}");
        server
    }

    #[test]
    fn encodes_the_protocol_version_name_and_counts() {
        let counts = Counts {
            spaces: 2,
            participants: 1,
            attention: None,
        };
        assert_eq!(
            properties("Studio", 7, counts, None),
            vec![
                ("protocol".to_owned(), "7".to_owned()),
                ("name".to_owned(), "Studio".to_owned()),
                ("spaces".to_owned(), "2".to_owned()),
                ("participants".to_owned(), "1".to_owned()),
            ]
        );
        let fingerprint = "ab".repeat(32);
        assert_eq!(
            properties("Studio", 7, counts, Some(&fingerprint)).last(),
            Some(&("fingerprint".to_owned(), fingerprint))
        );
    }

    #[test]
    fn decodes_a_valid_fingerprint_only() {
        let protocol = PROTOCOL_VERSION.to_string();
        let fingerprint = |value: &str| {
            let server = found(&lookup(&[("protocol", &protocol), ("fingerprint", value)]));
            server.fingerprint
        };
        assert_eq!(fingerprint(&"AB".repeat(32)), Some("ab".repeat(32)));
        assert_eq!(fingerprint(&"ab".repeat(31)), None);
        assert_eq!(fingerprint(&"zz".repeat(32)), None);
        let server = found(&lookup(&[("protocol", &protocol)]));
        assert_eq!(server.fingerprint, None);
    }

    #[test]
    fn attention_counts_are_encoded_only_when_given() {
        let counts = Counts {
            spaces: 4,
            participants: 1,
            attention: Some(AttentionCounts {
                waiting: 1,
                problems: 0,
                replies: 2,
            }),
        };
        let encoded = properties("Studio", 7, counts, None);
        assert_eq!(
            encoded[4..],
            [
                ("waiting".to_owned(), "1".to_owned()),
                ("problems".to_owned(), "0".to_owned()),
                ("replies".to_owned(), "2".to_owned()),
            ]
        );
        let txt: HashMap<String, String> = properties("Studio", PROTOCOL_VERSION, counts, None)
            .into_iter()
            .collect();
        let server = found(&txt);
        assert_eq!(
            (server.waiting, server.problems, server.replies),
            (Some(1), Some(0), Some(2))
        );

        // Turned off by the operator, the keys are left out rather than advertised as zero, so browsers read them as
        // unknown rather than as nothing needing attention.
        let hidden = Counts {
            attention: None,
            ..counts
        };
        let encoded = properties("Studio", PROTOCOL_VERSION, hidden, None);
        assert!(
            encoded
                .iter()
                .all(|(key, _)| !["waiting", "problems", "replies"].contains(&key.as_str()))
        );
        let server = found(&encoded.into_iter().collect());
        assert_eq!(
            (server.waiting, server.problems, server.replies),
            (None, None, None)
        );
        assert_eq!(server.spaces, Some(4));

        // Each is read on its own, with the same rule as the other counts.
        let protocol = PROTOCOL_VERSION.to_string();
        let server = found(&lookup(&[
            ("protocol", &protocol),
            ("waiting", "-1"),
            ("problems", " 3 "),
            ("replies", "many"),
        ]));
        assert_eq!(
            (server.waiting, server.problems, server.replies),
            (None, Some(3), None)
        );
    }

    #[test]
    fn decodes_the_counts_it_encodes() {
        let counts = Counts {
            spaces: 3,
            participants: u32::MAX,
            attention: None,
        };
        let txt: HashMap<String, String> = properties("Studio", PROTOCOL_VERSION, counts, None)
            .into_iter()
            .collect();
        let server = found(&txt);
        assert_eq!(server.spaces, Some(3));
        assert_eq!(server.participants, Some(u32::MAX));
    }

    #[test]
    fn missing_or_malformed_counts_are_unknown() {
        let protocol = PROTOCOL_VERSION.to_string();
        // A server from before the counts were advertised is still found.
        let server = found(&lookup(&[("protocol", &protocol)]));
        assert_eq!((server.spaces, server.participants), (None, None));
        for value in ["", "two", "-1", "1.5", "4294967296", "0x2"] {
            let server = found(&lookup(&[
                ("protocol", &protocol),
                ("spaces", value),
                ("participants", value),
            ]));
            assert_eq!(
                (server.spaces, server.participants),
                (None, None),
                "{value:?}"
            );
        }
        // Each count is read on its own.
        let server = found(&lookup(&[
            ("protocol", &protocol),
            ("spaces", " 0 "),
            ("participants", "many"),
        ]));
        assert_eq!((server.spaces, server.participants), (Some(0), None));
    }

    #[test]
    fn truncates_long_names_on_a_character_boundary() {
        let name = "é".repeat(200);
        let encoded = &properties(&name, 1, Counts::default(), None)[1].1;
        assert!(encoded.len() + "name=".len() <= 255);
        assert!(encoded.chars().all(|character| character == 'é'));
        assert_eq!(truncate("añb", 2), "a");
    }

    #[test]
    fn reports_a_matching_protocol_as_found() {
        let server = found(&lookup(&[
            ("protocol", &PROTOCOL_VERSION.to_string()),
            ("name", "Samuel's Studio"),
        ]));
        assert_eq!(server.instance, "Studio");
        assert_eq!(server.name, "Samuel's Studio");
        assert_eq!(server.protocol, Some(PROTOCOL_VERSION));
        assert_eq!(server.address().as_deref(), Some("192.168.1.10:8765"));
    }

    #[test]
    fn reports_another_protocol_as_incompatible() {
        let other = (PROTOCOL_VERSION + 1).to_string();
        let server = incompatible(&lookup(&[("protocol", &other)]));
        assert_eq!(server.protocol, Some(PROTOCOL_VERSION + 1));
        assert!(!server.is_compatible());
    }

    #[test]
    fn reports_a_missing_or_invalid_protocol_as_incompatible() {
        for txt in [lookup(&[]), lookup(&[("protocol", "one")])] {
            let server = incompatible(&txt);
            assert_eq!(server.protocol, None);
        }
    }

    #[test]
    fn falls_back_to_the_instance_name() {
        let protocol = PROTOCOL_VERSION.to_string();
        for txt in [
            lookup(&[("protocol", &protocol)]),
            lookup(&[("protocol", &protocol), ("name", "")]),
        ] {
            let server = found(&txt);
            assert_eq!(server.name, "Studio");
        }
    }

    #[test]
    fn unescapes_instance_names() {
        assert_eq!(
            instance(r"Studio\.lan\\b._tomori._tcp.local."),
            r"Studio.lan\b"
        );
        assert_eq!(instance("Samuel's Mac._tomori._tcp.local."), "Samuel's Mac");
        // An escape with nothing after it is dropped.
        assert_eq!(instance(r"Studio\"), "Studio");
    }

    #[test]
    fn orders_addresses_by_preference_with_link_local_scopes() {
        let link_local: Ipv6Addr = "fe80::1".parse().unwrap();
        let global: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let ordered = addresses(
            [
                (IpAddr::V6(link_local), 4),
                (IpAddr::V4(Ipv4Addr::new(169, 254, 0, 2)), 4),
                (IpAddr::V4(Ipv4Addr::LOCALHOST), 1),
                (IpAddr::V6(Ipv6Addr::LOCALHOST), 1),
                (IpAddr::V6(global), 4),
                (IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 4),
                (IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 5),
            ],
            8765,
        );
        let printed: Vec<String> = ordered.iter().map(ToString::to_string).collect();
        assert_eq!(
            printed,
            [
                "192.168.1.10:8765",
                "[2001:db8::1]:8765",
                "169.254.0.2:8765",
                "[fe80::1%4]:8765",
                "127.0.0.1:8765",
                "[::1]:8765",
            ]
        );
        // The client parses the printed form back, scope included.
        assert_eq!(
            "[fe80::1%4]:8765".parse::<SocketAddr>().unwrap(),
            ordered[3]
        );
    }
}
