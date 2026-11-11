// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Watching the space's workers through the hub, with the fake backend reporting what they do and the time given: a
//! worker that leaves a message unanswered or fails is shown so to everyone and told to the agent once, through the
//! space's event queue, never in the middle of its turn, riding on participants' words, a failure at once and the rest
//! at most once every two minutes, and the `workers` tool says why. See `design/events.md`.
use super::*;
use tomori_backend::WorkerActivity;

/// The instant `minutes` after 09:00 UTC on 2026-11-11.
fn at(minutes: i64) -> jiff::Timestamp {
    "2026-11-11T09:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .checked_add(jiff::SignedDuration::from_mins(minutes))
        .unwrap()
}

fn millis(minutes: i64) -> u64 {
    u64::try_from(at(minutes).as_millisecond()).unwrap()
}

fn worker(id: &str, title: &str, state: tomori_backend::WorkerState) -> tomori_backend::Worker {
    tomori_backend::Worker {
        id: id.to_owned(),
        parent: Some("session".to_owned()),
        title: title.to_owned(),
        state,
    }
}

/// A hub at 09:00, an idle space, and Ada in it.
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
    hub.set_clock(at(0));
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    (hub, jobs, space, commands, ada)
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

/// The clock reads `minutes` past nine, and the hub looks at it.
fn tick(hub: &Hub, minutes: i64) {
    hub.set_clock(at(minutes));
    hub.tick_timers(at(minutes));
}

/// The state Ada was last sent the worker `id` in.
fn shown(ada: &Client, id: &str) -> Option<tomori_protocol::WorkerState> {
    super::anchors::state(ada)
        .expect("the space's state was sent")
        .workers
        .into_iter()
        .find(|worker| worker.id == id)
        .map(|worker| worker.state)
}

/// The state the space shows its first worker in.
fn modelled(hub: &Hub, space: &str) -> tomori_protocol::WorkerState {
    let state = hub.lock();
    let model = &state.space(space).unwrap().model;
    model.shown_worker(&model.workers()[0]).state
}

#[test]
fn an_unanswered_worker_is_shown_and_told_once_after_the_agent_s_turn() {
    let (hub, _jobs, space, commands, ada) = space();
    hub.drive_event(
        &space,
        Event::WorkersChanged {
            workers: vec![worker(
                "ses_1",
                "Deck concepts",
                tomori_backend::WorkerState::Running,
            )],
        },
    );
    // The coordinator sends it a follow-up at 09:00, and it never takes it up.
    hub.drive_event(
        &space,
        Event::WorkerActivity {
            worker: "ses_1".to_owned(),
            activity: WorkerActivity {
                progressed: millis(-3),
                message: Some(millis(0)),
                unanswered_since: Some(millis(0)),
                finish: Some("tool-calls".to_owned()),
                ..WorkerActivity::default()
            },
        },
    );
    tick(&hub, 4);
    assert!(handed(&commands).is_empty());
    assert_eq!(
        modelled(&hub, &space),
        tomori_protocol::WorkerState::Running
    );
    // Ada asks something, and the agent works on it: mid-turn, the worker is shown unanswered, and nothing is told.
    let words = super::said(&hub, &ada, &commands, Vec::new(), None);
    assert!(!words.text.contains("Worker"), "{}", words.text);
    hub.drive_event(&space, Event::Delivered { key: words.key });
    hub.drive_event(&space, Event::Busy);
    tick(&hub, 20);
    assert!(handed(&commands).is_empty());
    assert_eq!(
        shown(&ada, "ses_1"),
        Some(tomori_protocol::WorkerState::Unanswered)
    );
    // `workers` says so, with the evidence, still running as far as removing it goes.
    let listed = hub.tool_workers("session").unwrap();
    let listed = &listed["workers"][0];
    assert_eq!(listed["state"], "unanswered");
    assert_eq!(listed["running"], true);
    assert_eq!(listed["condition"]["kind"], "unanswered");
    assert_eq!(listed["condition"]["seconds"], 20 * 60);
    assert_eq!(listed["condition"]["last_step_ended"], "tool-calls");
    assert_eq!(listed["condition"]["tool"], serde_json::Value::Null);
    // The turn ends: after a quiet moment, Tomori's note alone, in its brackets.
    hub.drive_event(&space, Event::Idle);
    tick(&hub, 20);
    assert!(
        handed(&commands).is_empty(),
        "not straight after the agent spoke"
    );
    hub.lock().space_mut(&space).unwrap().model.quieten();
    tick(&hub, 20);
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    assert!(crate::events::is_prompt(&prompts[0].key));
    assert_eq!(
        prompts[0].text,
        "[Worker \"Deck concepts\" (fake session ses_1) has not answered a message sent 20 min ago]"
    );
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    hub.drive_event(&space, Event::Idle);
    // Told once, however long it lasts.
    hub.lock().space_mut(&space).unwrap().model.quieten();
    tick(&hub, 40);
    assert!(handed(&commands).is_empty());
    // Pointed at, it is described as everyone sees it.
    let words = super::said(
        &hub,
        &ada,
        &commands,
        vec![tomori_protocol::Resource::worker("ses_1")],
        None,
    );
    assert!(
        words.text.contains("(fake session ses_1, not answering)"),
        "{}",
        words.text
    );
    hub.drive_event(&space, Event::Delivered { key: words.key });
    hub.drive_event(&space, Event::Idle);
    // It answers at last: shown running again, with nothing to say.
    hub.drive_event(
        &space,
        Event::WorkerActivity {
            worker: "ses_1".to_owned(),
            activity: WorkerActivity {
                progressed: millis(41),
                ..WorkerActivity::default()
            },
        },
    );
    tick(&hub, 41);
    assert_eq!(
        modelled(&hub, &space),
        tomori_protocol::WorkerState::Running
    );
    assert_eq!(
        hub.tool_workers("session").unwrap()["workers"][0]["condition"],
        serde_json::Value::Null
    );
}

#[test]
fn failures_ride_on_participants_words_and_are_not_held_to_the_pace() {
    let (hub, _jobs, space, commands, ada) = space();
    let workers = |second: tomori_backend::WorkerState| Event::WorkersChanged {
        workers: vec![
            worker("ses_1", "Deck", tomori_backend::WorkerState::Error),
            worker("ses_2", "Review", second),
        ],
    };
    // The agent is at work as they fail.
    hub.drive_event(&space, Event::Busy);
    hub.drive_event(&space, workers(tomori_backend::WorkerState::Running));
    for id in ["ses_1", "ses_2"] {
        hub.drive_event(
            &space,
            Event::WorkerActivity {
                worker: id.to_owned(),
                activity: WorkerActivity {
                    progressed: millis(0),
                    error: Some("BadRequestError: insufficient credit".to_owned()),
                    ..WorkerActivity::default()
                },
            },
        );
    }
    tick(&hub, 0);
    assert_eq!(modelled(&hub, &space), tomori_protocol::WorkerState::Error);
    assert!(handed(&commands).is_empty());
    // Its turn ends, and Ada speaks at once: her words take the note, before them.
    hub.drive_event(&space, Event::Idle);
    let words = super::said(&hub, &ada, &commands, Vec::new(), None);
    assert_eq!(
        words.text,
        "ada: [Worker \"Deck\" (fake session ses_1) failed: BadRequestError: insufficient credit] what's this?"
    );
    hub.drive_event(&space, Event::Delivered { key: words.key });
    hub.drive_event(&space, Event::Idle);
    // The second fails a minute later: a failure is not held to the pace, and is told once the agent is quiet.
    hub.drive_event(&space, workers(tomori_backend::WorkerState::Error));
    tick(&hub, 1);
    assert!(
        handed(&commands).is_empty(),
        "not straight after the agent spoke"
    );
    hub.lock().space_mut(&space).unwrap().model.quieten();
    tick(&hub, 1);
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    assert_eq!(
        prompts[0].text,
        "[Worker \"Review\" (fake session ses_2) failed: BadRequestError: insufficient credit]"
    );
}

#[test]
fn a_stalled_worker_is_shown_and_described_so() {
    let (hub, _jobs, space, commands, ada) = space();
    hub.drive_event(
        &space,
        Event::WorkersChanged {
            workers: vec![worker(
                "ses_1",
                "Build",
                tomori_backend::WorkerState::Running,
            )],
        },
    );
    hub.drive_event(
        &space,
        Event::WorkerActivity {
            worker: "ses_1".to_owned(),
            activity: WorkerActivity {
                progressed: millis(0),
                tool: Some(tomori_backend::RunningTool {
                    name: "shell".to_owned(),
                    since: millis(0),
                }),
                ..WorkerActivity::default()
            },
        },
    );
    hub.drive_event(&space, Event::Busy);
    // Running a tool, it is given half an hour.
    tick(&hub, 29);
    assert_eq!(
        modelled(&hub, &space),
        tomori_protocol::WorkerState::Running
    );
    tick(&hub, 30);
    assert_eq!(
        shown(&ada, "ses_1"),
        Some(tomori_protocol::WorkerState::Stalled)
    );
    let listed = hub.tool_workers("session").unwrap();
    assert_eq!(listed["workers"][0]["state"], "stalled");
    assert_eq!(
        listed["workers"][0]["condition"]["tool"],
        serde_json::json!({"name": "shell", "seconds": 30 * 60})
    );
    hub.drive_event(&space, Event::Idle);
    let words = super::said(
        &hub,
        &ada,
        &commands,
        vec![tomori_protocol::Resource::worker("ses_1")],
        None,
    );
    assert_eq!(
        words.text,
        "ada: [Worker \"Build\" (fake session ses_1) has been running shell for 30 min with no other progress] \
         [Pointing at worker \"Build\" (fake session ses_1, stalled)] what's this?"
    );
}
