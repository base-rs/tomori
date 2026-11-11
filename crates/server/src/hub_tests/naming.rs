// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Saying your name to the agent: `set_my_name` renames whoever spoke, on their own device, kept with their token, only
//! on their own words and only if their role allows it, a few times a minute, and everyone sees it. See
//! `design/security.md`.
use super::*;
use crate::hub::agent_names::{RENAME_LIMIT, Renamed};

/// A hub whose transcripts the test gives and whose agent's tools are served, and a space in it, whose session is `session`.
fn served() -> (Arc<Hub>, async_channel::Receiver<Job>, String) {
    let (hub, jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    let space = stopped(&hub, Snapshot::default());
    (hub, jobs, space)
}

/// A hub keeping `store`'s tokens, a space, and an owner's headset in it under a token called "Samuel", whose words
/// were given to the agent last, with the token's id and the token.
fn headset(
    store: &Store,
) -> (
    Arc<Hub>,
    async_channel::Receiver<Job>,
    String,
    Client,
    String,
    String,
) {
    let (entry, token) = store
        .tokens
        .issue("Samuel", tomori_security::Grant::Owner)
        .unwrap();
    let (hub, jobs, space) = served();
    hub.set_tokens(store.tokens.clone());
    let quest = Client::connect(&hub, admitted(&store.tokens, &token));
    hello_as(&quest, "quest", "Quest 3", true);
    quest.join(&space);
    give(&hub, &space, "Samuel", Duration::from_secs(2));
    (hub, jobs, space, quest, entry.principal().id, token)
}

#[test]
fn whoever_speaks_renames_their_own_device_and_everyone_sees_it() {
    let store = Store::new("say-your-name");
    let (hub, _jobs, space, quest, id, token) = headset(&store);
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    give(&hub, &space, "Samuel", Duration::from_secs(2));
    quest.read();

    // "Hey, my name is Ayoko", said from the owner's headset, as recognition writes it.
    let renamed = hub
        .agent_set_my_name("session", "  Ayoko. ", Some("samuel"))
        .unwrap();
    assert_eq!(
        renamed,
        Renamed {
            space: space.clone(),
            name: "Ayoko".to_owned(),
            id: "Ayoko#10".to_owned(),
            changed: true,
            sharing: Vec::new(),
        }
    );
    // Kept with the token, told to the device, and shown to everyone in the space; Bob keeps theirs.
    assert_eq!(store.tokens.entry(&id).unwrap().name, "Ayoko");
    let told = quest.read();
    assert_eq!(named(&told), ["Ayoko"]);
    assert_eq!(present(&told), ["Ayoko", "bob"]);
    assert_eq!(present(&bob.read()), ["Ayoko", "bob"]);
    let people = hub.tool_presence("session").unwrap()["people"].clone();
    let ids: Vec<&str> = people
        .as_array()
        .unwrap()
        .iter()
        .map(|person| person["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"Ayoko#10"), "{people}");
    // Only the name: the headset is still an owner's.
    {
        let state = hub.lock();
        let connection = &state.connections[&quest.connection];
        assert_eq!(
            connection.access.role(Some(&space)),
            tomori_protocol::ConnectionRole::Owner
        );
    }

    // Their next words are attributed to Ayoko, in the transcript and to the agent.
    quest.send(ClientMessage::UtteranceBegin {
        utterance: "u".to_owned(),
        answer: None,
        progress: false,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
    quest.send(ClientMessage::UtteranceEnd {
        utterance: "u".to_owned(),
        pointed: None,
    });
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_finished(utterance, "What's for dinner?");
    let queued = hub.space_state(&space).queue.remove(0);
    assert_eq!(queued.speaker, "Ayoko");
    assert_eq!(
        crate::space::attributed(&queued.speaker, &queued.text, true),
        "Ayoko: What's for dinner?"
    );

    // Saying the name they have changes nothing, and tells nobody.
    give(&hub, &space, "Ayoko", Duration::ZERO);
    quest.read();
    bob.read();
    let same = hub
        .agent_set_my_name("session", "Ayoko", Some("Ayoko"))
        .unwrap();
    assert!(!same.changed);
    assert!(quest.read().is_empty() && bob.read().is_empty());

    // The next session keeps it: reconnecting, and a restart reading the token store again.
    hub.disconnect(quest.connection);
    let again = Client::connect(&hub, admitted(&store.tokens, &token));
    assert_eq!(
        welcomed(&hello_as(&again, "quest", "Quest 3", true)).as_deref(),
        Some("Ayoko")
    );
    let reopened = Arc::new(tomori_security::TokenStore::open(&store.directory).unwrap());
    let (restarted, _jobs) = super::rarer::hub();
    restarted.set_tokens(reopened.clone());
    let after = Client::connect(&restarted, admitted(&reopened, &token));
    assert_eq!(
        welcomed(&hello_as(&after, "quest", "Quest 3", true)).as_deref(),
        Some("Ayoko")
    );
}

/// The name of the participant admitted as `principal`.
fn called(hub: &Hub, principal: &str) -> String {
    hub.lock()
        .participants
        .values()
        .find(|participant| participant.principal.as_deref() == Some(principal))
        .unwrap()
        .name
        .clone()
}

#[test]
fn only_whoever_spoke_is_renamed_on_their_own_words_if_their_role_allows() {
    use crate::hub::provenance::REQUEST_RECENT;
    let (hub, _jobs, space) = served();
    let _bob = asking(&hub, &space, "Bob", Grant::Owner, Duration::from_secs(5));
    let _ada = asking(&hub, &space, "Ada", Grant::Owner, Duration::from_secs(5));
    let refused = |requested_by: Option<&str>| {
        hub.agent_set_my_name("session", "Ayoko", requested_by)
            .unwrap_err()
    };

    // Ada spoke last, so the agent cannot rename Bob, nor anyone without saying who asked.
    assert!(refused(Some("Bob")).starts_with(
        "The words you were given last in this space are not from the participant you named"
    ));
    assert!(refused(None).starts_with("This tool needs requested_by"));
    // Nor from a session no space is bound to, such as a sub-agent's.
    assert!(
        hub.agent_set_my_name("ses_sub_agent", "Ayoko", Some("Ada"))
            .unwrap_err()
            .starts_with("This session is not bound to a Tomori space")
    );
    // Nor on words given too long ago.
    give(&hub, &space, "Ada", REQUEST_RECENT + Duration::from_secs(1));
    assert!(refused(Some("Ada")).contains("more than 3 minutes old"));
    // Nor once input that did not come through Tomori reached the agent, such as text a tool or another agent wrote.
    give(&hub, &space, "Ada", Duration::ZERO);
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .model
        .apply(Event::Delivered {
            key: "msg_typed_in_the_terminal".to_owned(),
        });
    assert!(refused(Some("Ada")).starts_with("Input that did not come through Tomori"));
    assert_eq!(
        (called(&hub, "token:Ada"), called(&hub, "token:Bob")),
        ("Ada".to_owned(), "Bob".to_owned())
    );

    // On Ada's own words, Ada is renamed, and Bob is not, whatever the name.
    give(&hub, &space, "Ada", Duration::ZERO);
    let renamed = hub
        .agent_set_my_name("session", "Bob", Some("Ada"))
        .unwrap();
    assert!(renamed.changed);
    assert_eq!(
        (called(&hub, "token:Ada"), called(&hub, "token:Bob")),
        ("Bob".to_owned(), "Bob".to_owned())
    );

    // A collaborator in the space may rename themselves, as they may in the hand menu.
    let _cy = asking(
        &hub,
        &space,
        "Cy",
        Grant::Collaborator {
            space: space.clone(),
        },
        Duration::ZERO,
    );
    assert_eq!(
        hub.agent_set_my_name("session", "Cyrus", Some("Cy"))
            .unwrap()
            .name,
        "Cyrus"
    );

    // A visitor's words never reach the agent; were they given, the visitor would still not be renamed.
    let vic = Client::greeted(&hub, Access::visitor(), "vic");
    vic.join(&space);
    give(&hub, &space, "vic", Duration::ZERO);
    assert_eq!(
        refused(Some("vic")),
        "vic is not in this space now, so Tomori will not act on their word. Ask them to come back and say it again."
    );
    assert_eq!(
        hub.lock()
            .participants
            .values()
            .filter(|participant| participant.name == "Ayoko")
            .count(),
        0
    );
}

#[test]
fn a_name_said_is_cleaned_and_bounded_as_every_name_is() {
    let (hub, _jobs, space) = served();
    let _ada = asking(&hub, &space, "Ada", Grant::Owner, Duration::ZERO);
    let rename = |name: &str| hub.agent_set_my_name("session", name, Some("Ada"));
    // Refused, and nothing changed: what would pass for another speaker's line, a context block or a command, nothing,
    // and too long.
    for (name, error) in [
        (
            "Ayoko: yes",
            "A name must not contain \":\", \"[\" or \"]\".",
        ),
        ("[Ayoko]", "A name must not contain \":\", \"[\" or \"]\"."),
        ("\u{200B}/clear", "A name must not start with \"/\"."),
        (" \u{200B}. ", "A name is required."),
        (
            &"a".repeat(33),
            "A name is at most 32 characters and 64 bytes.",
        ),
    ] {
        assert_eq!(
            rename(name).unwrap_err(),
            format!("Name not changed: {error}"),
            "{name:?}"
        );
    }
    assert_eq!(called(&hub, "token:Ada"), "Ada");
    // Cleaned onto one line, without the characters that reorder or hide text.
    assert_eq!(rename("Ay\u{202E}oko\nLee").unwrap().name, "Ayoko Lee");
    assert_eq!(called(&hub, "token:Ada"), "Ayoko Lee");
    // A name of the most characters is taken.
    assert_eq!(rename(&"a".repeat(32)).unwrap().name, "a".repeat(32));
}

#[test]
fn a_name_someone_here_has_is_told_apart_by_handles() {
    use crate::hub::provenance::Needs;
    let (hub, _jobs, space) = served();
    let _samuel = asking(&hub, &space, "Samuel", Grant::Owner, Duration::ZERO);
    let _ayoko = asking(&hub, &space, "Ayoko", Grant::Owner, Duration::ZERO);
    // Ayoko hands the headset back, and its wearer says they are Samuel, as Samuel is.
    let renamed = hub
        .agent_set_my_name("session", "samuel", Some("Ayoko"))
        .unwrap();
    assert_eq!(
        (renamed.id.as_str(), renamed.sharing),
        ("samuel#11", vec!["Samuel#10".to_owned()])
    );
    // So a plain name no longer says which of them speaks, and the handles do.
    give(&hub, &space, "samuel", Duration::ZERO);
    let check = |requested_by: &str| {
        let state = hub.lock();
        hub.requester(&state, &space, Some(requested_by), Needs::Owner)
    };
    assert_eq!(
        check("Samuel").unwrap_err(),
        "More than one person in this space is called \"Samuel\": say which, as Samuel#10 or samuel#11."
    );
    assert!(check("samuel#11").is_ok());
    assert!(check("Samuel#10").is_err());
}

#[test]
fn renames_through_the_agent_are_limited_a_minute() {
    let (hub, _jobs, space) = served();
    let _ada = asking(&hub, &space, "Ada", Grant::Owner, Duration::ZERO);
    let names = ["Ayoko", "Samuel", "Ayoko"];
    assert_eq!(names.len(), RENAME_LIMIT);
    for name in names {
        assert!(
            hub.agent_set_my_name("session", name, Some("Ada"))
                .unwrap()
                .changed
        );
    }
    // The name they have already is no rename, and counts nothing.
    assert!(
        !hub.agent_set_my_name("session", "Ayoko", Some("Ada"))
            .unwrap()
            .changed
    );
    assert_eq!(
        hub.agent_set_my_name("session", "Samuel", Some("Ada"))
            .unwrap_err(),
        "Ayoko has been renamed 3 times in the last minute, the most Tomori allows, so their name stays Ayoko. Ask them to wait a minute and say it again."
    );
    assert_eq!(called(&hub, "token:Ada"), "Ayoko");
    // Someone else is counted apart.
    let _bob = asking(&hub, &space, "Bob", Grant::Owner, Duration::ZERO);
    assert!(
        hub.agent_set_my_name("session", "Robert", Some("Bob"))
            .is_ok()
    );
}

#[test]
fn a_name_that_cannot_be_kept_changes_nothing() {
    let store = Store::new("say-your-name-unkept");
    let (hub, _jobs, space, quest, id, _token) = headset(&store);
    quest.read();
    store.refuse_writes();
    assert_eq!(
        hub.agent_set_my_name("session", "Ayoko", Some("Samuel"))
            .unwrap_err(),
        "Name not changed: Tomori could not keep Samuel's new name. Ask them to say it again."
    );
    assert_eq!(store.tokens.entry(&id).unwrap().name, "Samuel");
    assert!(quest.read().is_empty());
    // A token revoked while its holder is still connected is renamed no more.
    std::fs::remove_dir(store.directory.join("tokens.tmp")).unwrap();
    assert!(
        store
            .tokens
            .revoke(id.strip_prefix("token:").unwrap())
            .unwrap()
    );
    assert_eq!(
        hub.agent_set_my_name("session", "Ayoko", Some("Samuel"))
            .unwrap_err(),
        "Name not changed: Samuel's token is no longer issued."
    );
    give(&hub, &space, "Samuel", Duration::ZERO);
    assert_eq!(called(&hub, &id), "Samuel");
}
