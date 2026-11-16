// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Someone asking a space's agent for their own colour, which the server keeps with their token, through Tomori's
//! tools. See `design/people.md` and `design/mcp.md`.

use serde_json::json;
use tomori_protocol::ClientMessage;
use tomori_server::test_support::{TestServer, eventually};

pub mod mcp_support;
use mcp_support::{call, failure, say};

#[test]
fn whoever_asks_is_drawn_in_the_colour_they_asked_for_and_keeps_it() {
    let server = TestServer::with_tokens_and_tools(true, |server| {
        server.with_tokens_over_plain_http_for_tests()
    });
    let (entry, owner) = server.issue("Samuel", tomori_security::Grant::Owner);
    let mut quest = server.connect_with_token("quest", "Quest 3", &owner);
    let space = quest.create_space("Kitchen").id;
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let (_, bob) = server.issue("Bob", tomori_security::Grant::Owner);
    let mut bob = server.connect_with_token("bob", "Bob", &bob);
    bob.send(&ClientMessage::Join {
        space: space.clone(),
    });
    bob.space(|state| state.participants.len() == 2 && state.colours);
    let colour = |arguments| call(&server, &token, &session, "set_my_colour", arguments);

    say(&mut quest, "teal", "Make me teal.");
    // Not anyone else's, not without who asked, and not a colour that is no person's.
    assert!(
        failure(&colour(json!({ "colour": "teal", "requested_by": "Bob" }))).starts_with(
            "The words you were given last in this space are not from the participant you named"
        )
    );
    assert!(
        failure(&colour(json!({ "colour": "teal" }))).starts_with("This tool needs requested_by")
    );
    assert_eq!(
        failure(&colour(json!({ "requested_by": "Samuel" }))),
        "This tool needs colour, as text."
    );
    assert_eq!(
        failure(&colour(json!({ "colour": "teal", "requested_by": 5 }))),
        "requested_by is a participant's name, as text."
    );
    assert!(
        failure(&colour(
            json!({ "colour": "purple", "requested_by": "Samuel" })
        ))
        .starts_with("Colour not changed: A colour is one of coral,")
    );
    assert!(
        failure(&colour(
            json!({ "colour": "240", "requested_by": "Samuel" })
        ))
        .starts_with("Colour not changed: Hue 240 is kept for the agent")
    );
    assert_eq!(
        failure(&colour(
            json!({ "colour": "a".repeat(33), "requested_by": "Samuel" })
        )),
        "The colour is too long; the limit is 32 bytes."
    );

    let id = format!("Samuel#{}", entry.tag.unwrap());
    let teal = colour(json!({ "colour": "Teal", "requested_by": "Samuel" }));
    assert_eq!(
        teal["structuredContent"],
        json!({
            "space": space,
            "id": id,
            "colour": "teal",
            "hue": 165,
            "changed": true,
            "next": "Tell them briefly that they are now teal.",
        })
    );
    // Everyone in the space sees them in it, and it is kept with their token.
    bob.space(|state| {
        state
            .participants
            .iter()
            .any(|participant| participant.name == "Samuel" && participant.colour == Some(165))
    });
    let kept = |server: &TestServer| {
        server
            .tokens
            .as_ref()
            .unwrap()
            .entry(&entry.principal().id)
            .unwrap()
            .colour
    };
    assert_eq!(kept(&server), Some(165));
    // Asking again changes nothing.
    let same = colour(json!({ "colour": "165", "requested_by": "Samuel" }));
    assert_eq!(same["structuredContent"]["changed"], false);
    assert_eq!(
        same["structuredContent"]["next"],
        "They are already teal, so nothing changed; say so briefly."
    );

    // Back to their name's, by asking for their default.
    say(&mut quest, "back", "Go back to my own colour.");
    let back = colour(json!({ "colour": "default", "requested_by": "Samuel" }));
    assert_eq!(back["structuredContent"]["colour"], "default");
    assert_eq!(back["structuredContent"]["hue"], serde_json::Value::Null);
    assert_eq!(
        back["structuredContent"]["next"],
        "Tell them briefly that they are now their name's colour."
    );
    assert_eq!(kept(&server), None);

    // A colour that is no swatch's is said in degrees.
    let degrees = colour(json!({ "colour": "100", "requested_by": "Samuel" }));
    assert_eq!(degrees["structuredContent"]["colour"], "100°");
}

#[test]
fn colours_through_the_agent_are_counted_with_a_participant_s_own() {
    let server = TestServer::with_tokens_and_tools(true, |server| {
        server.with_tokens_over_plain_http_for_tests()
    });
    let (_, owner) = server.issue("Samuel", tomori_security::Grant::Owner);
    let mut quest = server.connect_with_token("quest", "Quest 3", &owner);
    quest.create_space("Kitchen");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    say(&mut quest, "colours", "Try every colour on me.");
    let swatches = tomori_protocol::hue::SWATCHES;
    for change in 0..tomori_server::COLOUR_LIMIT {
        let hue = swatches[change % swatches.len()].1;
        let coloured = call(
            &server,
            &token,
            &session,
            "set_my_colour",
            json!({ "colour": hue.to_string(), "requested_by": "Samuel" }),
        );
        assert_eq!(coloured["structuredContent"]["changed"], true, "{change}");
    }
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "set_my_colour",
            json!({ "colour": "default", "requested_by": "Samuel" })
        ))
        .starts_with(&format!(
            "Samuel has changed their colour {} times in the last minute",
            tomori_server::COLOUR_LIMIT
        ))
    );
}
