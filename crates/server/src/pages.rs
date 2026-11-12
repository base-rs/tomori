// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The server's web pages: a sign-in page that takes a token, and a status page for owners. Plain HTML written here, without scripts or a web framework. See `design/security.md`.
use std::fmt::Write;

use tomori_protocol::{Activity, ConnectionRole};
use tomori_security::{Entry, Grant};

use crate::hub::{PairingState, PairingView, Status, spoken};

/// The paths of the pages and their actions.
pub const STATUS_PATH: &str = "/";
pub const SIGN_IN_PATH: &str = "/sign-in";
pub const SIGN_OUT_PATH: &str = "/sign-out";
pub const REVOKE_PATH: &str = "/tokens/revoke";
pub const CONFIRM_PAIRING_PATH: &str = "/pairing/confirm";
pub const DECLINE_PAIRING_PATH: &str = "/pairing/decline";
pub const ALLOW_PAIRING_PATH: &str = "/pairing/allow";
pub const OPEN_PAIRING_PATH: &str = "/pairing/open";
pub const CLOSE_PAIRING_PATH: &str = "/pairing/close";

/// The most bytes of a form body read.
pub const FORM_LIMIT: usize = 4096;

/// Who may see the pages, and that scripts, frames and other origins have no part in them.
pub const CONTENT_SECURITY_POLICY: &str =
    "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'";

/// Text escaped for HTML content and attribute values.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            character => escaped.push(character),
        }
    }
    escaped
}

/// The value of `name` in an `application/x-www-form-urlencoded` body, if present and well formed.
pub fn form_value(body: &[u8], name: &str) -> Option<String> {
    let body = std::str::from_utf8(body).ok()?;
    body.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(key)? == name).then(|| decode(value)).flatten()
    })
}

/// Percent-decode a form component, with `+` as a space, or `None` where an escape is not `%` and two hexadecimal
/// digits or the result is not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                // Exactly two digits, without the sign `u8::from_str_radix` would take.
                let digit = |at: usize| char::from(*bytes.get(at)?).to_digit(16);
                let high = digit(index + 1)?;
                let low = digit(index + 2)?;
                decoded.push((high * 16 + low) as u8);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).ok()
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{title} – Tomori</title>\
<style>body{{font-family:system-ui,sans-serif;max-width:48rem;margin:2rem auto;padding:0 1rem;line-height:1.4}}table{{border-collapse:collapse}}td,th{{padding:.2rem .6rem;text-align:left;border-bottom:1px solid #ddd}}form.inline{{display:inline}}.error{{color:#b00}}</style>\
</head><body><h1>{title}</h1>\n{body}</body></html>\n",
        title = escape(title),
    )
}

/// The sign-in page, with `error` shown if the last attempt failed.
pub fn sign_in(error: Option<&str>) -> String {
    let error = error
        .map(|error| format!("<p class=\"error\">{}</p>\n", escape(error)))
        .unwrap_or_default();
    page(
        "Sign in",
        &format!(
            "{error}<form method=\"post\" action=\"{SIGN_IN_PATH}\">\
<p><label>Token <input name=\"token\" type=\"password\" autocomplete=\"off\" required size=\"40\"></label></p>\
<p><button>Sign in</button></p></form>\n\
<p>Use a token from <code>tomori-server auth mint</code>.</p>\n"
        ),
    )
}

/// The page shown to someone signed in who is not an owner.
pub fn not_an_owner() -> String {
    page(
        "For owners only",
        &format!(
            "<p>The status page is for owners.</p>\n<form method=\"post\" action=\"{SIGN_OUT_PATH}\"><button>Sign out</button></form>\n"
        ),
    )
}

fn role_label(role: ConnectionRole) -> &'static str {
    match role {
        ConnectionRole::Owner => "owner",
        ConnectionRole::Collaborator => "collaborator",
        ConnectionRole::Visitor => "visitor",
        ConnectionRole::Pairing => "pairing",
        ConnectionRole::Unknown => "unknown",
    }
}

fn activity_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Idle => "idle",
        Activity::Running => "running",
        Activity::Waiting => "waiting",
        Activity::Closed => "closed",
        Activity::Unknown => "unknown",
    }
}

fn people(people: &[(String, ConnectionRole)]) -> String {
    if people.is_empty() {
        return "nobody".to_owned();
    }
    people
        .iter()
        .map(|(name, role)| format!("{} ({})", escape(name), role_label(*role)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether pairing is open, with a form to open it or close it. See `design/security.md`.
fn window(state: &PairingState) -> String {
    match state {
        PairingState::Closed { opens_for } => format!(
            "<p>Pairing is closed: a device that asks to pair is refused.</p>\n<form method=\"post\" action=\"{OPEN_PAIRING_PATH}\"><button>Open pairing for {}</button></form>\n",
            spoken(*opens_for)
        ),
        PairingState::Open { remaining, by } => {
            let seconds = remaining.as_secs();
            // Past its time, a pairing begun within it holds it until that pairing ends.
            let until = if seconds == 0 {
                "until the pairing waiting ends".to_owned()
            } else {
                format!("for {}:{:02} more", seconds / 60, seconds % 60)
            };
            format!(
                "<p>Pairing is open, {until}, as {} opened it {}: a device may ask to pair, and the first to pair closes it.</p>\n<form method=\"post\" action=\"{CLOSE_PAIRING_PATH}\"><button>Close pairing</button></form>\n",
                escape(&by.who),
                escape(by.place)
            )
        }
    }
}

/// The status page: the spaces and who is in them, whether pairing is open, the pairing waiting, the issued tokens, which an owner can revoke, and whether `signed_in` with a cookie, so that signing out is offered.
pub fn status(
    status: &Status,
    opening: Option<&PairingState>,
    pairing: Option<&PairingView>,
    paused: bool,
    tokens: Option<&[Entry]>,
    signed_in: bool,
) -> String {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "<p>{} visitors connected.</p>\n<h2>Spaces</h2>",
        status.visitors
    );
    if status.spaces.is_empty() {
        body.push_str("<p>No spaces.</p>\n");
    } else {
        body.push_str("<table><tr><th>Space</th><th>Activity</th><th>Participants</th><th>Visitors</th></tr>\n");
        for space in &status.spaces {
            let _ = writeln!(
                body,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&space.title),
                activity_label(space.activity),
                people(&space.participants),
                space.visitors
            );
        }
        body.push_str("</table>\n");
    }
    let _ = writeln!(body, "<h2>Lobby</h2>\n<p>{}</p>", people(&status.lobby));
    body.push_str("<h2>Pairing</h2>\n");
    if let Some(opening) = opening {
        body.push_str(&window(opening));
    }
    match pairing {
        Some(pairing) => {
            // Each form names the pairing shown, so that it acts on no other.
            let target = format!(
                "<input type=\"hidden\" name=\"pairing\" value=\"{}\">",
                pairing.serial
            );
            let decline = format!(
                "<form class=\"inline\" method=\"post\" action=\"{DECLINE_PAIRING_PATH}\">{target}<button>Decline</button></form>"
            );
            match &pairing.code {
                Some(code) => {
                    let _ = writeln!(
                        body,
                        "<p>Pairing request from {}, with code <strong>{}</strong>. Confirm only if the device you are pairing shows the same code.</p>\n\
<form class=\"inline\" method=\"post\" action=\"{CONFIRM_PAIRING_PATH}\">{target}<button>Confirm</button></form> {decline}",
                        escape(&pairing.device.to_string()),
                        escape(code)
                    );
                }
                None => {
                    let _ = writeln!(
                        body,
                        "<p>Pairing request from {}; its code is not shown yet.</p>\n{decline}",
                        escape(&pairing.device.to_string())
                    );
                }
            }
        }
        None => body.push_str("<p>No pairing is pending.</p>\n"),
    }
    if paused {
        let _ = writeln!(
            body,
            "<p>Pairing is paused, since too many pairings began recently.</p>\n<form method=\"post\" action=\"{ALLOW_PAIRING_PATH}\"><button>Allow pairing</button></form>"
        );
    }
    // What agents did through Tomori's tools, so that an owner sees each confirmation, decline and refusal.
    if !status.pairing_decisions.is_empty() {
        body.push_str("<h3>By agents</h3>\n<ul>\n");
        for decision in &status.pairing_decisions {
            let _ = writeln!(body, "<li>{}</li>", escape(decision));
        }
        body.push_str("</ul>\n");
    }
    if let Some(tokens) = tokens {
        body.push_str("<h2>Tokens</h2>\n");
        if tokens.is_empty() {
            body.push_str("<p>No tokens are issued.</p>\n");
        } else {
            body.push_str(
                "<table><tr><th>Identifier</th><th>Name</th><th>Role</th><th></th></tr>\n",
            );
            for entry in tokens {
                let role = match &entry.grant {
                    Grant::Owner => "owner".to_owned(),
                    Grant::Collaborator { space } => format!("collaborator in {}", escape(space)),
                };
                let _ = writeln!(
                    body,
                    "<tr><td><code>{id}</code></td><td>{name}</td><td>{role}</td><td><form class=\"inline\" method=\"post\" action=\"{REVOKE_PATH}\"><input type=\"hidden\" name=\"id\" value=\"{id}\"><button>Revoke</button></form></td></tr>",
                    id = escape(&entry.id),
                    name = escape(&entry.name),
                );
            }
            body.push_str("</table>\n");
        }
    }
    if signed_in {
        let _ = writeln!(
            body,
            "<form method=\"post\" action=\"{SIGN_OUT_PATH}\"><button>Sign out</button></form>"
        );
    }
    page("Status", &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::SpaceStatus;

    #[test]
    fn escapes_markup() {
        assert_eq!(
            escape("<a href=\"x\">Ada's & Bob's</a>"),
            "&lt;a href=&quot;x&quot;&gt;Ada&#39;s &amp; Bob&#39;s&lt;/a&gt;"
        );
    }

    #[test]
    fn reads_form_values() {
        let body = b"other=1&token=tomori_ab%2Bc+d&empty";
        assert_eq!(form_value(body, "token"), Some("tomori_ab+c d".to_owned()));
        assert_eq!(form_value(body, "empty"), Some(String::new()));
        assert_eq!(form_value(body, "missing"), None);
        assert_eq!(form_value(b"token=%zz", "token"), None);
        assert_eq!(form_value(b"token=%2", "token"), None);
        assert_eq!(form_value(b"token=%ff", "token"), None);
        assert_eq!(form_value(&[0xff], "token"), None);
    }

    #[test]
    fn a_percent_escape_is_two_hexadecimal_digits_without_a_sign() {
        assert_eq!(form_value(b"token=%+1", "token"), None);
        assert_eq!(form_value(b"%+1=1", "\u{1}"), None);
        assert_eq!(form_value(b"token=%4A", "token"), Some("J".to_owned()));
    }

    #[test]
    fn the_sign_in_page_escapes_its_error() {
        let page = sign_in(Some("<bad>"));
        assert!(page.contains("&lt;bad&gt;"));
        assert!(page.contains("action=\"/sign-in\""));
        assert!(!sign_in(None).contains("class=\"error\""));
        assert!(not_an_owner().contains("for owners"));
    }

    #[test]
    fn the_status_page_lists_spaces_people_and_tokens_without_revealing_them() {
        let status = Status {
            spaces: vec![SpaceStatus {
                title: "<Studio>".into(),
                activity: Activity::Running,
                participants: vec![
                    ("Ada".into(), ConnectionRole::Owner),
                    ("Bob".into(), ConnectionRole::Collaborator),
                ],
                visitors: 2,
            }],
            lobby: vec![],
            visitors: 3,
            pairing_decisions: vec!["The agent of space \"<Studio>\" declined pairing.".into()],
        };
        let page = status_page(&status);
        assert!(page.contains("<h3>By agents</h3>\n<ul>\n<li>The agent of space &quot;&lt;Studio&gt;&quot; declined pairing.</li>"), "{page}");
        assert!(page.contains("&lt;Studio&gt;"));
        assert!(page.contains("Ada (owner), Bob (collaborator)"));
        assert!(page.contains("<td>running</td>"));
        assert!(page.contains("3 visitors connected"));
        assert!(page.contains("Lobby</h2>\n<p>nobody"));
        assert!(page.contains("No tokens are issued"));
        assert!(page.contains("Sign out"));

        let empty = super::status(&Status::default(), None, None, false, None, false);
        assert!(!empty.contains("paused"));
        let paused = super::status(&Status::default(), None, None, true, None, false);
        assert!(paused.contains("Pairing is paused"));
        assert!(paused.contains("action=\"/pairing/allow\""));
        assert!(empty.contains("No pairing is pending"));
        let mut view = PairingView {
            serial: 7,
            device: tomori_security::pairing::ChosenName::new("<Ada's & Quest> 4 eight"),
            code: Some("042 917".into()),
        };
        let pairing = super::status(&Status::default(), None, Some(&view), false, None, false);
        assert!(pairing.contains(
            "Pairing request from a device calling itself &quot;Ada&#39;s &amp; Quest&quot;, with code <strong>042 917</strong>. Confirm only if the device you are pairing shows the same code."
        ));
        assert!(pairing.contains("action=\"/pairing/confirm\""));
        assert!(pairing.contains("action=\"/pairing/decline\""));
        assert!(pairing.contains("name=\"pairing\" value=\"7\""));
        view.code = None;
        let unrevealed = super::status(&Status::default(), None, Some(&view), false, None, false);
        assert!(unrevealed.contains("&amp; Quest&quot;; its code is not shown yet"));
        // A device whose name is left with nothing is shown as a device.
        view.device = tomori_security::pairing::ChosenName::new("482 913");
        let nameless = super::status(&Status::default(), None, Some(&view), false, None, false);
        assert!(nameless.contains("Pairing request from a device; its code is not shown yet"));
        assert!(!unrevealed.contains("/pairing/confirm"));
        assert!(unrevealed.contains("/pairing/decline"));
        assert!(empty.contains("No spaces."));
        assert!(!empty.contains("Tokens"));
        assert!(!empty.contains("Sign out"));
        for role in [
            ConnectionRole::Visitor,
            ConnectionRole::Pairing,
            ConnectionRole::Unknown,
        ] {
            assert!(!role_label(role).is_empty());
        }
        for activity in [
            Activity::Idle,
            Activity::Waiting,
            Activity::Closed,
            Activity::Unknown,
        ] {
            assert!(!activity_label(activity).is_empty());
        }
    }

    #[test]
    fn the_status_page_says_whether_pairing_is_open() {
        let page = |state: &PairingState| {
            super::status(&Status::default(), Some(state), None, false, None, false)
        };
        let closed = page(&PairingState::Closed {
            opens_for: std::time::Duration::from_secs(120),
        });
        assert!(
            closed.contains("<button>Open pairing for 2 minutes</button>"),
            "{closed}"
        );
        let by = crate::hub::Opener::status_page(Some("<Ada>"));
        let open = page(&PairingState::Open {
            remaining: std::time::Duration::from_secs(102),
            by: by.clone(),
        });
        assert!(
            open.contains(
                "Pairing is open, for 1:42 more, as &lt;Ada&gt; opened it on the status page"
            ),
            "{open}"
        );
        assert!(open.contains("action=\"/pairing/close\""));
        let held = page(&PairingState::Open {
            remaining: std::time::Duration::ZERO,
            by,
        });
        assert!(
            held.contains("Pairing is open, until the pairing waiting ends, as"),
            "{held}"
        );
    }

    fn status_page(status: &Status) -> String {
        super::status(status, None, None, false, Some(&[]), true)
    }

    #[test]
    fn a_form_that_is_not_utf_8_has_no_values() {
        assert_eq!(form_value(b"id=\xff", "id"), None);
        // Nor does a name that is not well formed.
        assert_eq!(form_value(b"%zz=1", "id"), None);
    }
}
