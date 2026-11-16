// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Choosing your colour: kept with your token beside your name, on every device and after a restart, shown to everyone
//! in the space, cleared back to your name's, checked, and limited to a few changes a minute. See `design/people.md`.
use super::*;
use crate::hub::COLOUR_LIMIT;

/// The colour each participant chose in the latest space state among `messages`, by name.
fn colours(messages: &[ServerMessage]) -> Vec<(String, Option<u16>)> {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            ServerMessage::Space { state } => Some(
                state
                    .participants
                    .iter()
                    .map(|participant| (participant.name.clone(), participant.colour))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// What `client` is told as it joins `space`.
fn joining(client: &Client, space: &str) -> Vec<ServerMessage> {
    client.send(ClientMessage::Join {
        space: space.to_owned(),
    });
    client.read()
}

/// Whether the latest space state among `messages` says the server keeps colours.
fn keeps_colours(messages: &[ServerMessage]) -> bool {
    messages.iter().rev().any(|message| match message {
        ServerMessage::Space { state } => state.colours,
        _ => false,
    })
}

#[test]
fn a_chosen_colour_is_kept_with_the_token_on_every_device_and_after_a_restart() {
    // What is not a space's state says nothing of colours.
    let named = [ServerMessage::Name { name: "Ada".into() }];
    assert!(colours(&named).is_empty());
    assert!(!keeps_colours(&named));
    let store = Store::new("colours");
    let (entry, token) = store
        .tokens
        .issue("Ada", tomori_security::Grant::Owner)
        .unwrap();
    let (hub, _jobs) = hub_waiting();
    hub.set_tokens(store.tokens.clone());
    let id = stopped(&hub, Snapshot::default());
    let quest = Client::connect(&hub, admitted(&store.tokens, &token));
    hello_as(&quest, "quest", "Quest 3", true);
    quest.join(&id);
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&id);
    let told = quest.read();
    assert!(keeps_colours(&told));
    assert_eq!(
        colours(&told),
        [("Ada".to_owned(), None), ("bob".to_owned(), None)]
    );

    // Choosing teal keeps it with the token and shows everyone in the space.
    quest.send(ClientMessage::SetColour { colour: Some(165) });
    assert_eq!(
        colours(&bob.read()),
        [("Ada".to_owned(), Some(165)), ("bob".to_owned(), None)]
    );
    assert_eq!(
        store.tokens.entry(&entry.principal().id).unwrap().colour,
        Some(165)
    );
    // The same colour again changes nothing, and tells nobody.
    quest.read();
    quest.send(ClientMessage::SetColour { colour: Some(165) });
    assert!(quest.read().is_empty() && bob.read().is_empty());

    // A second device presenting the same token is the same colour, and choosing on either colours both.
    let laptop = Client::connect(&hub, admitted(&store.tokens, &token));
    hello_as(&laptop, "laptop", "Laptop", false);
    laptop.join(&id);
    assert_eq!(
        colours(&bob.read()),
        [
            ("Ada".to_owned(), Some(165)),
            ("bob".to_owned(), None),
            ("Ada".to_owned(), Some(165))
        ]
    );
    laptop.send(ClientMessage::SetColour { colour: Some(20) });
    assert_eq!(
        colours(&quest.read()),
        [
            ("Ada".to_owned(), Some(20)),
            ("bob".to_owned(), None),
            ("Ada".to_owned(), Some(20))
        ]
    );

    // After a restart, the server reading the same store, it is still theirs.
    let (again, _jobs) = hub_waiting();
    let reopened = Arc::new(tomori_security::TokenStore::open(&store.directory).unwrap());
    again.set_tokens(reopened.clone());
    let other = stopped(&again, Snapshot::default());
    let quest = Client::connect(&again, admitted(&store.tokens, &token));
    hello_as(&quest, "quest", "Quest 3", true);
    assert_eq!(
        colours(&joining(&quest, &other)),
        [("Ada".to_owned(), Some(20))]
    );

    // Back to the name's colour, for everyone holding the token.
    quest.send(ClientMessage::SetColour { colour: None });
    assert_eq!(colours(&quest.read()), [("Ada".to_owned(), None)]);
    assert_eq!(reopened.entry(&entry.principal().id).unwrap().colour, None);
}

#[test]
fn a_colour_no_person_may_be_is_refused_and_a_colour_kept_out_of_bounds_is_ignored() {
    let store = Store::new("refused-colours");
    let (entry, token) = store
        .tokens
        .issue("Ada", tomori_security::Grant::Owner)
        .unwrap();
    let (hub, _jobs) = hub_waiting();
    hub.set_tokens(store.tokens.clone());
    let id = stopped(&hub, Snapshot::default());
    let quest = Client::connect(&hub, admitted(&store.tokens, &token));
    hello_as(&quest, "quest", "Quest 3", true);
    quest.join(&id);
    quest.read();
    // The agent's violet, and no hue at all.
    for colour in [250, 400] {
        quest.send(ClientMessage::SetColour {
            colour: Some(colour),
        });
        let errors = quest.errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("Colour not changed: "), "{errors:?}");
    }
    assert_eq!(
        store.tokens.entry(&entry.principal().id).unwrap().colour,
        None
    );
    // One kept with the token that is no person's, as written by hand, is ignored as they connect.
    store.tokens.recolour(&entry.id, Some(250)).unwrap();
    let laptop = Client::connect(&hub, admitted(&store.tokens, &token));
    hello_as(&laptop, "laptop", "Laptop", false);
    assert_eq!(
        colours(&joining(&laptop, &id)),
        [("Ada".to_owned(), None), ("Ada".to_owned(), None)]
    );
}

#[test]
fn colours_change_a_few_times_a_minute_at_most() {
    let (hub, _jobs) = hub_waiting();
    let id = stopped(&hub, Snapshot::default());
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&id);
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&id);
    ada.read();
    let swatches = [20, 58, 90, 125, 165, 290, 325];
    for change in 0..COLOUR_LIMIT {
        ada.send(ClientMessage::SetColour {
            colour: Some(swatches[change % swatches.len()]),
        });
    }
    assert!(ada.errors().is_empty());
    // A participant without a token is coloured while the server keeps them.
    let kept = swatches[(COLOUR_LIMIT - 1) % swatches.len()];
    assert_eq!(colours(&bob.read())[0], ("ada".to_owned(), Some(kept)));
    ada.send(ClientMessage::SetColour { colour: None });
    assert_eq!(
        ada.errors(),
        [format!(
            "Colour not changed: you have changed it {COLOUR_LIMIT} times in the last minute, the most Tomori allows. Try again in a minute."
        )]
    );
    assert!(bob.read().is_empty());
    // Someone else's choice is counted apart.
    bob.send(ClientMessage::SetColour { colour: Some(20) });
    assert_eq!(colours(&ada.read())[1], ("bob".to_owned(), Some(20)));
}

#[test]
fn a_colour_that_cannot_be_kept_is_not_changed() {
    let store = Store::new("unkept-colour");
    let (entry, token) = store
        .tokens
        .issue("Ada", tomori_security::Grant::Owner)
        .unwrap();
    let (hub, _jobs) = hub_waiting();
    hub.set_tokens(store.tokens.clone());
    let id = stopped(&hub, Snapshot::default());
    let quest = Client::connect(&hub, admitted(&store.tokens, &token));
    hello_as(&quest, "quest", "Quest 3", true);
    quest.join(&id);
    quest.read();

    store.refuse_writes();
    quest.send(ClientMessage::SetColour { colour: Some(165) });
    assert_eq!(
        quest.errors(),
        ["Colour not changed: Your colour could not be kept. Try again."]
    );
    store.allow_writes();

    // A token revoked meanwhile keeps no colour.
    store.tokens.revoke(&entry.id).unwrap();
    quest.send(ClientMessage::SetColour { colour: Some(165) });
    assert_eq!(
        quest.errors(),
        ["Colour not changed: Your token is no longer issued."]
    );
}

#[test]
fn a_visitor_s_colour_is_theirs_while_they_are_kept_and_shown_in_no_space() {
    let (hub, _jobs) = hub_waiting();
    let id = stopped(&hub, Snapshot::default());
    let owner = Client::greeted(&hub, Access::open(), "owner");
    owner.join(&id);
    let visitor = Client::greeted(&hub, Access::visitor(), "visitor");
    visitor.join(&id);
    owner.read();
    visitor.read();
    visitor.send(ClientMessage::SetColour { colour: Some(90) });
    assert!(visitor.errors().is_empty());
    // A visitor takes no part, so nobody's state changes.
    assert!(owner.read().is_empty());
}
