// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Someone saying their name to a space's agent, which renames them, on their own device, through Tomori's tools. See `design/security.md` and `design/mcp.md`.

use serde_json::{Value, json};
use tomori_protocol::ClientMessage;
use tomori_server::test_support::{TestServer, eventually};

pub mod mcp_support;
use mcp_support::{call, failure, say};

#[test]
fn someone_wearing_an_owner_s_headset_says_their_name_and_keeps_it() {
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
    bob.space(|state| state.participants.len() == 2);

    // Ayoko puts on Samuel's headset and says their name; the agent renames whoever spoke.
    say(&mut quest, "hello", "Hey, my name is Ayoko.");
    // Not on behalf of anyone else.
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "set_my_name",
            json!({ "name": "Ayoko", "requested_by": "Bob" })
        ))
        .starts_with(
            "The words you were given last in this space are not from the participant you named"
        )
    );
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "set_my_name",
            json!({ "name": "Ayoko" })
        ))
        .starts_with("This tool needs requested_by")
    );
    assert_eq!(
        failure(&call(
            &server,
            &token,
            &session,
            "set_my_name",
            json!({ "requested_by": "Samuel" })
        )),
        "This tool needs name, as text."
    );
    let renamed = call(
        &server,
        &token,
        &session,
        "set_my_name",
        json!({ "name": "Ayoko", "requested_by": "Samuel" }),
    );
    let id = format!("Ayoko#{}", entry.tag.unwrap());
    assert_eq!(
        renamed["structuredContent"],
        json!({
            "space": space,
            "name": "Ayoko",
            "id": id,
            "changed": true,
            "next": "Greet them by their new name, briefly, as in \"Hi Ayoko\".",
        })
    );
    // Saying it again changes nothing; a name too long, or a requester that is not a name, is refused.
    let same = call(
        &server,
        &token,
        &session,
        "set_my_name",
        json!({ "name": "Ayoko.", "requested_by": "Samuel" }),
    );
    assert_eq!(same["structuredContent"]["changed"], false);
    assert_eq!(
        same["structuredContent"]["next"],
        "They are already called Ayoko, so nothing changed; say so briefly."
    );
    assert_eq!(
        failure(&call(
            &server,
            &token,
            &session,
            "set_my_name",
            json!({ "name": "a".repeat(65), "requested_by": "Samuel" })
        )),
        "The name is too long; the limit is 64 bytes."
    );
    assert_eq!(
        failure(&call(
            &server,
            &token,
            &session,
            "set_my_name",
            json!({ "name": "Ayoko", "requested_by": 5 })
        )),
        "requested_by is a participant's name, as text."
    );

    // Everyone in the space sees Ayoko by it, and it is kept with the headset's token.
    bob.space(|state| {
        let mut names: Vec<&str> = state
            .participants
            .iter()
            .map(|participant| participant.name.as_str())
            .collect();
        names.sort_unstable();
        names == ["Ayoko", "Bob"]
    });
    assert_eq!(
        server
            .tokens
            .as_ref()
            .unwrap()
            .entry(&entry.principal().id)
            .unwrap()
            .name,
        "Ayoko"
    );

    // Bob, who says they are Ayoko too, is told apart from the first Ayoko by their ids.
    say(&mut bob, "me too", "Call me Ayoko as well.");
    let shared = call(
        &server,
        &token,
        &session,
        "set_my_name",
        json!({ "name": "Ayoko", "requested_by": "Bob" }),
    );
    let bob_id = shared["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(bob_id, id);
    assert_eq!(
        shared["structuredContent"]["next"],
        format!(
            "Greet them by their new name, briefly, as in \"Hi Ayoko\". Someone else here is called Ayoko too, so where you must name one of them, use their id from presence: {bob_id} is whoever you renamed, and {id} is not."
        )
    );

    // The next session on the headset is still Ayoko's, with the owner's role as before.
    quest.finish();
    let again = server.connect_with_token("quest", "Quest 3", &owner);
    assert_eq!(again.role, tomori_protocol::ConnectionRole::Owner);
    let mut again = again;
    again.send(&ClientMessage::Join {
        space: space.clone(),
    });
    again.space(|state| {
        state
            .participants
            .iter()
            .any(|participant| participant.name == "Ayoko")
    });
}

#[test]
fn a_rename_refused_for_an_argument_set_my_name_does_not_take_uses_neither_the_limit_nor_the_words()
{
    let server = TestServer::with_tokens_and_tools(true, |server| {
        server.with_tokens_over_plain_http_for_tests()
    });
    let (_, owner) = server.issue("Samuel", tomori_security::Grant::Owner);
    let mut quest = server.connect_with_token("quest", "Quest 3", &owner);
    quest.create_space("Kitchen");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let rename = |arguments: Value| call(&server, &token, &session, "set_my_name", arguments);

    say(&mut quest, "hello", "Call me Ayoko.");
    // Refused before who asked is checked or a rename counted, more times than the limit allows renames.
    for _ in 0..4 {
        assert_eq!(
            failure(&rename(
                json!({ "name": "Ayoko", "requested_by": "Samuel", "everywhere": true })
            )),
            "everywhere is not an argument of set_my_name; it takes: name, requested_by."
        );
    }
    // The same words still stand behind each of the three renames a minute allows, and only then is one refused.
    for name in ["Ayoko", "Aiko", "Ayo"] {
        let renamed = rename(json!({ "name": name, "requested_by": "Samuel" }));
        assert_eq!(renamed["structuredContent"]["changed"], true, "{renamed}");
        assert_eq!(renamed["structuredContent"]["name"], name, "{renamed}");
    }
    let limited = failure(&rename(
        json!({ "name": "Ayoko", "requested_by": "Samuel" }),
    ));
    assert!(
        limited.contains("renamed 3 times in the last minute"),
        "{limited}"
    );
}
