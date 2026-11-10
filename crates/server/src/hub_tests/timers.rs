// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's timers through the hub, with the fake backend and the time given: they fire only while someone is
//! present, wait for the agent's turn to end, coalesce, ride on participants' words, are told as Tomori's notes, are
//! carried in the space's state and pointed at, and are kept with the space. See `design/timers.md`.
use super::*;
use crate::timers::Request;

/// The instant `text`, `YYYY-MM-DD HH:MM`, in UTC, the zone these tests set timers in.
fn at(text: &str) -> jiff::Timestamp {
    format!("{}:00Z", text.replace(' ', "T")).parse().unwrap()
}

/// A hub whose timers go by UTC from 09:17, serving tools; an idle space; and Ada, joined but not yet reporting
/// engagement.
fn space() -> (
    Arc<Hub>,
    async_channel::Receiver<Job>,
    String,
    async_channel::Receiver<SpaceCommand>,
    Client,
) {
    let (hub, jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    hub.set_zone("UTC");
    hub.set_clock(at("2026-11-11 09:17"));
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    (hub, jobs, space, commands, ada)
}

fn hourly(name: &str) -> Request {
    Request {
        name: Some(name.to_owned()),
        note: "Give a status update".to_owned(),
        every_minutes: Some(60),
        ..Request::default()
    }
}

fn engage(client: &Client, state: tomori_protocol::Engagement) {
    client.send(ClientMessage::Engagement { state });
}

/// What the agent was handed since last asked.
fn handed(commands: &async_channel::Receiver<SpaceCommand>) -> Vec<tomori_backend::Prompt> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt),
            _ => None,
        })
        .collect()
}

/// The agent takes `prompt` and ends its turn.
fn take(hub: &Hub, space: &str, prompt: &tomori_backend::Prompt) {
    hub.drive_event(
        space,
        Event::Delivered {
            key: prompt.key.clone(),
        },
    );
    hub.drive_event(space, Event::Idle);
}

#[test]
fn a_timer_fires_only_while_someone_is_present_and_waits_for_the_agent_s_turn() {
    let (hub, _jobs, space, commands, ada) = space();
    let set = hub.agent_set_timer("session", hourly("status")).unwrap();
    assert_eq!(set["timer"]["schedule"], "every hour");
    assert_eq!(set["timer"]["next"], "Wed 2026-11-11 10:00");
    assert_eq!(set["timer"]["next_in"], "43 min");
    assert_eq!(set["replaced"], false);
    assert!(
        set["next"]
            .as_str()
            .unwrap()
            .contains("not a request from anyone")
    );
    // Everyone sees it, paused while nobody is present.
    let seen = super::anchors::state(&ada).unwrap();
    assert_eq!(seen.timers.len(), 1);
    assert_eq!(seen.timers[0].state, tomori_protocol::TimerState::Paused);
    // Ada joined but reports nothing, as the spectator does: nobody is present, so 10:00 is skipped.
    hub.tick_timers(at("2026-11-11 10:00"));
    assert!(handed(&commands).is_empty());
    // Engaged, she is present, and the timer is waiting.
    engage(&ada, tomori_protocol::Engagement::Engaged);
    hub.tick_timers(at("2026-11-11 10:30"));
    assert_eq!(
        super::anchors::state(&ada).unwrap().timers[0].state,
        tomori_protocol::TimerState::Waiting
    );
    // Ada asks something, and the agent works on it: mid-turn, the timer waits.
    let words = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: words.key });
    hub.drive_event(&space, Event::Busy);
    hub.tick_timers(at("2026-11-11 11:00"));
    assert!(handed(&commands).is_empty());
    assert_eq!(
        super::anchors::state(&ada).unwrap().timers[0].state,
        tomori_protocol::TimerState::Fired
    );
    // The turn ends: a quiet moment first, then Tomori's note alone, in its brackets, with no speaker.
    hub.drive_event(&space, Event::Idle);
    hub.tick_timers(at("2026-11-11 11:00"));
    assert!(
        handed(&commands).is_empty(),
        "not straight after the agent spoke"
    );
    hub.lock().space_mut(&space).unwrap().model.quieten();
    hub.tick_timers(at("2026-11-11 11:00"));
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    assert!(crate::events::is_prompt(&prompts[0].key));
    assert_eq!(
        prompts[0].text,
        "[Timer \"status\" (every hour, when present): fired 11:00; skipped 1 while nobody was present; present: ada; note: \"Give a status update\"]"
    );
    // Delivered, the turn it began is nobody's request, so no sensitive tool may act on Ada's earlier words.
    take(&hub, &space, &prompts[0]);
    let state = hub.lock();
    let refused = state
        .requester(
            &space,
            Some("ada"),
            super::provenance::Needs::Participant,
            Instant::now(),
        )
        .unwrap_err();
    assert!(
        refused.contains("Tomori gave you its own notes"),
        "{refused}"
    );
}

#[test]
fn firings_coalesce_while_the_agent_works_and_ride_on_participants_words() {
    let (hub, _jobs, space, commands, ada) = space();
    engage(&ada, tomori_protocol::Engagement::Distracted);
    hub.agent_set_timer("session", hourly("status")).unwrap();
    hub.agent_set_timer(
        "session",
        Request {
            name: Some("tea".to_owned()),
            note: "Tea is ready".to_owned(),
            in_minutes: Some(50),
            ..Request::default()
        },
    )
    .unwrap();
    hub.drive_event(&space, Event::Busy);
    hub.tick_timers(at("2026-11-11 10:00"));
    hub.tick_timers(at("2026-11-11 10:07"));
    hub.tick_timers(at("2026-11-11 11:00"));
    assert!(handed(&commands).is_empty());
    hub.drive_event(&space, Event::Idle);
    // Ada speaks before the quiet moment passes: her words take every note waiting, a burst as one.
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    assert!(!crate::events::is_prompt(&prompt.key));
    assert_eq!(
        prompt.text,
        "ada: [Timer \"tea\" (once at 10:07, when present): fired 10:07; present: ada; its last firing; note: \"Tea is ready\"] \
         [Timer \"status\" (every hour, when present): fired 11:00; last fired 10:00; present: ada; note: \"Give a status update\"; \
         and 1 earlier, coalesced] what's this?"
    );
    // The one-off is gone; the hourly one stays.
    let seen = super::anchors::state(&ada).unwrap();
    let names: Vec<&str> = seen
        .timers
        .iter()
        .map(|timer| timer.name.as_str())
        .collect();
    assert_eq!(names, ["status"]);
}

#[test]
fn tomori_s_notes_wait_while_someone_speaks_and_a_timer_is_pointed_at() {
    let (hub, _jobs, space, commands, ada) = space();
    engage(&ada, tomori_protocol::Engagement::Engaged);
    hub.agent_set_timer("session", hourly("status")).unwrap();
    hub.tick_timers(at("2026-11-11 09:20"));
    // Pointing at it names it, its schedule and when it next fires, as of the hub's last look.
    let prompt = said(
        &hub,
        &ada,
        &commands,
        vec![
            tomori_protocol::Resource::timer("status"),
            tomori_protocol::Resource::timer("status"),
        ],
        None,
    );
    assert_eq!(
        prompt.text,
        "ada: [Pointing at timer \"status\" (every hour, when present; next in 40 min)] what's this?"
    );
    take(&hub, &space, &prompt);
    hub.lock().space_mut(&space).unwrap().model.quieten();
    // Ada begins speaking as it fires: the note waits for her words.
    ada.send(ClientMessage::UtteranceBegin {
        utterance: "u-held".to_owned(),
        answer: None,
        progress: false,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
    hub.tick_timers(at("2026-11-11 10:00"));
    assert!(handed(&commands).is_empty());
    // Words never ended, as by a client stuck holding X, hold nothing back past their bound; a client that drops takes
    // its open words with it as it disconnects.
    for utterance in hub.lock().utterances.values_mut() {
        utterance.begun -= crate::events::HOLD;
    }
    hub.tick_timers(at("2026-11-11 10:00"));
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].text.starts_with("[Timer \"status\""));
    take(&hub, &space, &prompts[0]);
    hub.disconnect(ada.connection);
    assert!(hub.lock().utterances.is_empty());
}

#[test]
fn the_timer_tools_list_and_cancel_and_need_the_space_s_own_session() {
    let (hub, _jobs, space, _commands, ada) = space();
    assert_eq!(
        hub.agent_cancel_timer("session", "status").unwrap_err(),
        "This space has no timers."
    );
    hub.agent_set_timer("session", hourly("status")).unwrap();
    let replaced = hub
        .agent_set_timer(
            "session",
            Request {
                every_minutes: Some(30),
                ..hourly("Status")
            },
        )
        .unwrap();
    assert_eq!(replaced["replaced"], true);
    let listed = hub.tool_timers("session").unwrap();
    assert_eq!(listed["space"], space.as_str());
    assert_eq!(listed["most"], 8);
    assert_eq!(listed["timers"][0]["name"], "Status");
    assert_eq!(listed["timers"][0]["schedule"], "every 30 min");
    assert_eq!(listed["timers"][0]["condition"], "present");
    assert_eq!(listed["timers"][0]["zone"], "UTC");
    assert_eq!(listed["timers"][0]["fired"], 0);
    assert_eq!(
        listed["timers"][0]["waiting_for_arrival"],
        serde_json::Value::Null
    );
    // One waiting for someone to arrive says since when.
    hub.agent_set_timer(
        "session",
        Request {
            name: Some("morning".to_owned()),
            note: "Summarise".to_owned(),
            daily_at: Some("10:00".to_owned()),
            condition: Some(crate::timers::Condition::PresentOrArrival),
            ..Request::default()
        },
    )
    .unwrap();
    hub.tick_timers(at("2026-11-11 10:00"));
    let listed = hub.tool_timers("session").unwrap();
    assert_eq!(
        listed["timers"][1]["waiting_for_arrival"],
        "Wed 2026-11-11 10:00"
    );
    hub.agent_cancel_timer("session", "morning").unwrap();
    assert!(
        hub.agent_set_timer(
            "session",
            Request {
                every_minutes: Some(1),
                ..hourly("x")
            }
        )
        .unwrap_err()
        .contains("at most every 5 minutes")
    );
    // A timer kept in a zone this server does not know says so, going by UTC.
    let kept = hub.lock().space(&space).unwrap().model.timers().kept();
    let mut odd = kept.clone();
    odd["timers"][0]["zone"] = "Nowhere/Here".into();
    *hub.lock().space_mut(&space).unwrap().model.timers_mut() =
        crate::timers::Timers::restore(&odd, &space);
    let listed = hub.tool_timers("session").unwrap();
    assert!(
        listed["timers"][0]["zone_note"]
            .as_str()
            .unwrap()
            .contains("goes by UTC")
    );
    *hub.lock().space_mut(&space).unwrap().model.timers_mut() =
        crate::timers::Timers::restore(&kept, &space);
    assert_eq!(
        hub.tool_timers("session").unwrap()["timers"][0]["zone_note"],
        serde_json::Value::Null
    );
    let refused = hub.agent_cancel_timer("session", "nope").unwrap_err();
    assert_eq!(
        refused,
        "This space has no timer called \"nope\"; its timers are Status."
    );
    let cancelled = hub.agent_cancel_timer("session", "status").unwrap();
    assert_eq!(cancelled["cancelled"], "Status");
    assert!(super::anchors::state(&ada).unwrap().timers.is_empty());
    // A session bound to no space has none.
    for refused in [
        hub.agent_set_timer("ses_other", hourly("x")).unwrap_err(),
        hub.tool_timers("ses_other").unwrap_err(),
        hub.agent_cancel_timer("ses_other", "x").unwrap_err(),
    ] {
        assert!(refused.contains("not bound to a Tomori space"), "{refused}");
    }
    // The clock's own time, without a test's.
    *hub.clock.write().unwrap() = None;
    assert!(hub.wall_now() > at("2026-01-01 00:00"));
}

#[test]
fn a_closed_space_s_timers_wait_and_are_kept_and_restored_with_it() {
    let directory = std::env::temp_dir().join(format!("tomori-hub-timers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let store = Arc::new(crate::kept::SpaceStore::open(&directory).unwrap());
    let (hub, _jobs, space, commands, ada) = space();
    assert!(hub.restore(store.clone()).is_empty());
    engage(&ada, tomori_protocol::Engagement::Engaged);
    hub.agent_set_timer("session", hourly("status")).unwrap();
    let kept = |store: &crate::kept::SpaceStore| {
        store
            .spaces()
            .into_iter()
            .find(|kept| kept.id == space)
            .unwrap()
            .timers
    };
    assert_eq!(kept(&store)["timers"][0]["name"], "status");
    hub.tick_timers(at("2026-11-11 10:00"));
    assert_eq!(handed(&commands).len(), 1);
    assert_eq!(kept(&store)["timers"][0]["fired"], 1, "kept as it fires");
    // Closed, it is left as it is.
    hub.drive_event(
        &space,
        Event::Closed {
            reason: Some("stopped".to_owned()),
        },
    );
    hub.tick_timers(at("2026-11-11 11:00"));
    assert!(handed(&commands).is_empty());
    assert_eq!(
        kept(&store)["timers"][0]["next"],
        at("2026-11-11 11:00").as_second()
    );
    // Restored by another run from what was kept, its timers come back, waiting until it is bound again.
    let (again, _jobs) = hub_waiting();
    again.restore(store);
    let state = again.lock();
    let restored = state.space(&space).unwrap();
    assert_eq!(restored.model.timers().list()[0].name, "status");
    drop(state);
    again.tick_timers(at("2026-11-11 12:00"));
    assert_eq!(
        again.lock().space(&space).unwrap().model.timers().list()[0].fired,
        1
    );
    let _ = std::fs::remove_dir_all(&directory);
}
