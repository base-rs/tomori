// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent following whoever it is talking with, driven through the hub with the times its timer would wake at: it
//! turns as someone presses X and goes to them once they hold it, only turns when already near, waits its dwell before
//! leaving one partner for another, serves a huddle from one spot, glances while busy and goes as their words are
//! given, never for a visitor, stops where it is when nothing was said, goes home once the conversation is quiet, and
//! stays where it is pinned. See `design/environments.md`.
use super::*;
use crate::hub::following::{COMMIT, DWELL, EMPTY, GLANCE, LEFT};
use tomori_protocol::{AgentMovement, Movement};

/// What the agent of `space` does: where it is summoned to, where it faces, and whether it is pinned.
fn agent(hub: &Hub, space: &str) -> (Option<tomori_protocol::Pose>, Option<[f32; 3]>, bool) {
    let state = hub.lock();
    let record = state.space(space).unwrap();
    (
        record.summoned,
        record.following.faces(),
        record.following.pinned,
    )
}

/// A hub whose transcripts the test gives, with an idle space whose owner, Ada, chose that its agent follows the
/// speaker and goes home after `delay`.
fn following(
    delay: Option<u32>,
) -> (
    Arc<Hub>,
    async_channel::Receiver<Job>,
    String,
    async_channel::Receiver<SpaceCommand>,
    Client,
) {
    let (hub, jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Follow, delay),
    });
    assert!(ada.errors().is_empty());
    (hub, jobs, space, commands, ada)
}

/// `client` begins words for the agent, as `utterance`.
fn press(client: &Client, utterance: &str) {
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: None,
        progress: false,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
}

/// `client` lets go of X, ending `utterance`.
fn release(client: &Client, utterance: &str) {
    client.send(ClientMessage::UtteranceEnd {
        utterance: utterance.to_owned(),
        pointed: None,
    });
}

/// `client` cancels `utterance`.
fn cancel(client: &Client, utterance: &str) {
    client.send(ClientMessage::UtteranceCancel {
        utterance: utterance.to_owned(),
    });
}

/// Every utterance still being transcribed is heard as `text`, in the order they began.
fn heard(hub: &Hub, text: &str) {
    let mut utterances: Vec<UtteranceId> = hub.lock().utterances.keys().copied().collect();
    utterances.sort_by_key(|id| id.0);
    for utterance in utterances {
        hub.transcript_finished(utterance, text);
    }
}

/// The horizontal distance between `a` and `b`.
fn level(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// The space's state as `client` was last sent it.
fn last_state(client: &Client) -> Option<tomori_protocol::SpaceState> {
    client
        .read()
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::Space { state } => Some(*state),
            _ => None,
        })
        .next_back()
}

const ADA: [f32; 3] = [0.0, 1.6, 2.0];

#[test]
fn it_turns_as_someone_presses_x_and_goes_beside_them_once_they_hold_it_its_entourage_staying_home()
{
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    // Ada stands 4.3 m from the agent's home, looking towards it.
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    ada.read();
    let start = Instant::now();
    press(&ada, "u1");
    // It turns to her at once, for everyone, and moves nothing yet.
    assert_eq!(agent(&hub, &space), (None, Some(ADA), false));
    let state = last_state(&ada).expect("told");
    assert_eq!((state.facing, state.summoned), (Some(ADA), None));
    // A tap, let go before it commits, only turned it.
    release(&ada, "u1");
    hub.flush_snapshots(start + COMMIT + Duration::from_millis(100));
    assert_eq!(agent(&hub, &space).0, None);
    // Held, it goes to stand 1.5 m from her, to her left, a little above her eyes, facing her.
    press(&ada, "u2");
    let held = Instant::now() + COMMIT;
    assert!(hub.snapshot_deadline().is_some_and(|due| due <= held));
    hub.flush_snapshots(held);
    let (summoned, facing, pinned) = agent(&hub, &space);
    let at = summoned.expect("gone to her");
    assert!((level(at.position, ADA) - 1.5).abs() < 1e-3, "{at:?}");
    assert!(at.position[0] < -1.0 && at.position[2] < ADA[2]);
    assert!((at.position[1] - 1.7).abs() < 1e-4);
    assert_eq!((facing, pinned), (None, false));
    // Its wall and its workers stay home, and its way there is a glide.
    let state = last_state(&ada).expect("told");
    assert_eq!(
        (state.summoner, state.via, state.stream),
        (None, None, false)
    );
    // It stands for her, as clients are told.
    assert_eq!(state.partner, Some(ADA));
    // Arrived, a follow-up from where she stands only turns it.
    hub.flush_snapshots(held + Duration::from_secs(10));
    press(&ada, "u3");
    hub.flush_snapshots(Instant::now() + COMMIT);
    let (again, facing, _) = agent(&hub, &space);
    assert_eq!(again, Some(at));
    assert_eq!(facing, Some(ADA));
}

#[test]
fn a_tap_only_turns_it_even_once_its_words_are_given() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    release(&ada, "u1");
    heard(&hub, "Yes.");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, SpaceCommand::Send(_)))
    );
    assert_eq!(agent(&hub, &space), (None, Some(ADA), false));
    hub.flush_snapshots(Instant::now() + Duration::from_secs(1));
    assert_eq!(agent(&hub, &space).0, None);
}

#[test]
fn far_away_it_streams_there() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    let far = [0.0, 1.6, 8.0];
    ada.send(looking(far, [0.0, 1.6, 0.0]));
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    let state = last_state(&ada).expect("told");
    assert!(state.stream);
    assert!(state.summoned.is_some());
    // Once there, it no longer streams.
    hub.flush_snapshots(Instant::now() + Duration::from_secs(10));
    assert!(!last_state(&ada).expect("told").stream);
    let _ = space;
}

#[test]
fn it_waits_its_dwell_before_leaving_one_partner_for_another_unless_they_are_far() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    // Bob stands 5 m from Ada, too far to be one huddle, but within 6 m of where the agent goes for her.
    let near = [-4.0, 1.6, -1.0];
    bob.send(looking(near, [-4.0, 1.6, -4.0]));
    press(&ada, "u1");
    let committed = Instant::now() + COMMIT;
    hub.flush_snapshots(committed);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    // Arrived, her words are cancelled, which leaves it there.
    hub.flush_snapshots(committed + Duration::from_secs(5));
    cancel(&ada, "u1");
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // Bob speaks within its dwell: it only turns to him.
    press(&bob, "u2");
    hub.flush_snapshots(committed + Duration::from_millis(5500));
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    assert_eq!(agent(&hub, &space).1, Some(near));
    cancel(&bob, "u2");
    // Its dwell runs from when it arrived, some 4 s after it set off, not from when it set off.
    press(&bob, "u3");
    hub.flush_snapshots(committed + DWELL + Duration::from_secs(1));
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    cancel(&bob, "u3");
    // After it, he moves it.
    press(&bob, "u4");
    hub.flush_snapshots(committed + DWELL + Duration::from_secs(5));
    let for_bob = agent(&hub, &space).0.expect("gone to Bob");
    assert!((level(for_bob.position, near) - 1.5).abs() < 1e-3);
    hub.flush_snapshots(committed + DWELL + Duration::from_secs(8));
    cancel(&bob, "u4");
    assert_eq!(agent(&hub, &space).0, Some(for_bob));
    // Carl, far from where it stands for Bob now, moves it within its dwell.
    let carl = Client::greeted(&hub, Access::open(), "carl");
    carl.join(&space);
    carl.send(looking([8.0, 1.6, 8.0], [8.0, 1.6, 0.0]));
    press(&carl, "u5");
    hub.flush_snapshots(committed + DWELL + Duration::from_secs(9));
    let far = agent(&hub, &space).0.expect("gone to Carl");
    assert!(
        (level(far.position, [8.0, 1.6, 8.0]) - 1.5).abs() < 1e-3,
        "{far:?}"
    );
}

#[test]
fn a_huddle_who_spoke_to_it_lately_is_served_from_one_spot() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    // Ada and Bob stand a metre apart, looking the same way.
    let (at_ada, at_bob) = ([-0.5, 1.6, 2.0], [0.5, 1.6, 2.0]);
    ada.send(looking(at_ada, [-0.5, 1.6, -3.0]));
    bob.send(looking(at_bob, [0.5, 1.6, -3.0]));
    press(&ada, "u1");
    let start = Instant::now();
    hub.flush_snapshots(start + COMMIT);
    hub.flush_snapshots(start + Duration::from_secs(5));
    cancel(&ada, "u1");
    // Bob, beside her, speaks after its dwell, from when it arrived: it stands for both of them, clear of each.
    press(&bob, "u2");
    hub.flush_snapshots(start + DWELL + Duration::from_secs(5));
    let spot = agent(&hub, &space).0.expect("serving them");
    let nearest = level(spot.position, at_ada).min(level(spot.position, at_bob));
    assert!((1.5..1.7).contains(&nearest), "{spot:?}");
    // And it faces the middle of them.
    let ahead = crate::stage::geometry::rotate(spot.orientation, [0.0, 0.0, 1.0]);
    let towards = [-spot.position[0], 0.0, 2.0 - spot.position[2]];
    assert!((ahead[0].atan2(ahead[2]) - towards[0].atan2(towards[2])).abs() < 1e-3);
}

#[test]
fn while_it_answers_someone_a_press_only_glances_and_it_goes_to_them_as_their_words_are_given() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    let at_bob = [6.0, 1.6, 6.0];
    bob.send(looking(at_bob, [6.0, 1.6, 0.0]));
    // Ada's words, held, are given to the agent, which goes to her as it begins answering.
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(&ada, "u1");
    heard(&hub, "What is this?");
    let key = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt.key),
            _ => None,
        })
        .expect("given");
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    assert!((level(for_ada.position, ADA) - 1.5).abs() < 1e-3);
    hub.space_sent(&space, &key, Ok(()));
    hub.drive_event(&space, Event::Busy);
    // Bob speaks while it answers: it glances at him, for a moment, and moves nothing.
    press(&bob, "u2");
    let pressed = Instant::now();
    assert_eq!(agent(&hub, &space).1, Some(at_bob));
    hub.flush_snapshots(pressed + COMMIT);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    hub.flush_snapshots(pressed + GLANCE);
    assert_eq!(agent(&hub, &space).1, None);
    release(&bob, "u2");
    heard(&hub, "And this?");
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // Once it is free, his words are given, and it goes to him, far from Ada.
    hub.drive_event(&space, Event::Idle);
    let for_bob = agent(&hub, &space).0.expect("gone to Bob");
    assert!(
        (level(for_bob.position, at_bob) - 1.5).abs() < 1e-3,
        "{for_bob:?}"
    );
}

#[test]
fn words_that_answer_name_or_title_and_visitors_never_turn_or_move_it() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    ada.send(ClientMessage::UtteranceBegin {
        utterance: "u1".to_owned(),
        answer: None,
        progress: false,
        naming: true,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, None, false));
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u1".to_owned(),
    });
    let vic = Client::greeted(&hub, Access::visitor(), "vic");
    vic.join(&space);
    vic.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&vic, "u2");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, None, false));
    // Nor does anyone before they say where they are.
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    press(&bob, "u3");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, None, false));
    // Nor as their words are given.
    release(&bob, "u3");
    heard(&hub, "Hello?");
    assert_eq!(agent(&hub, &space), (None, None, false));
}

#[test]
fn with_nowhere_clear_beside_them_it_only_turns_and_a_call_stands_it_in_front_of_them() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    // Ada stands in a booth the scenery encloses.
    hub.set_environments(Environments::new([
        crate::Environment::new("sky", "Sky").with_solids([([-3.0, 0.0, 0.5], [3.0, 3.0, 5.0])])
    ]));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, Some(ADA), false));
    cancel(&ada, "u1");
    ada.send(ClientMessage::CallAgent);
    let (at, _, pinned) = agent(&hub, &space);
    assert!(pinned);
    assert!((at.expect("called").position[2] - (ADA[2] - 2.3)).abs() < 1e-4);
}

#[test]
fn turned_at_home_it_turns_back_once_the_conversation_is_quiet() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    release(&ada, "u1");
    let start = Instant::now() + Duration::from_secs(2);
    hub.flush_snapshots(start);
    assert_eq!(agent(&hub, &space), (None, Some(ADA), false));
    heard(&hub, "");
    hub.flush_snapshots(start + Duration::from_secs(1));
    hub.flush_snapshots(start + Duration::from_secs(91));
    assert_eq!(agent(&hub, &space), (None, None, false));
    assert!(!last_state(&ada).expect("told").facing.is_some());
}

#[test]
fn moving_only_when_asked_again_it_stays_where_it_followed_to() {
    let (hub, _jobs, space, _commands, ada) = following(Some(30));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    let start = Instant::now() + COMMIT;
    hub.flush_snapshots(start);
    hub.flush_snapshots(start + Duration::from_secs(5));
    cancel(&ada, "u1");
    let at = agent(&hub, &space).0.expect("followed");
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::default(),
    });
    hub.flush_snapshots(start + Duration::from_secs(6));
    hub.flush_snapshots(start + Duration::from_secs(600));
    assert_eq!(agent(&hub, &space).0, Some(at));
    assert_eq!(hub.lock().movement_deadline(), None);
}

#[test]
fn moving_only_when_asked_its_own_move_leaves_its_wall_and_workers_home() {
    let (hub, _jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    let (space, _commands) = idle(&hub);
    let ada = Client::projecting(&hub, Access::open(), "ada");
    ada.join(&space);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    ada.read();
    // It goes before her, and nothing goes with it: the projection is not sent again.
    hub.agent_move_agent("session", "front ada", None).unwrap();
    let state = last_state(&ada).expect("told");
    assert!(state.summoned.is_some());
    assert_eq!(state.summoner, None);
    assert!(ada.projections().is_empty());
}

#[test]
fn a_move_for_words_that_came_to_nothing_stops_where_it_has_reached() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // Cancelled before it commits, it never goes.
    press(&ada, "u0");
    cancel(&ada, "u0");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space).0, None);
    // Cancelled as it begins, it stays home.
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert!(agent(&hub, &space).0.is_some());
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u1".to_owned(),
    });
    assert_eq!(agent(&hub, &space).0, None);
    // Cancelled a second along its way, it stops there, 0.85 m from home.
    press(&ada, "u2");
    hub.flush_snapshots(Instant::now() + COMMIT);
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .backdate(COMMIT + Duration::from_secs(1));
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u2".to_owned(),
    });
    let stopped = agent(&hub, &space).0.expect("stopped on its way");
    let home = [0.0, 1.4, -2.3];
    let gone = (0..3)
        .map(|axis| (stopped.position[axis] - home[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!((0.8..1.3).contains(&gone), "{gone}");
    assert_eq!(last_state(&ada).expect("told").via, None);
    // Words with no speech in them stop it too; and the same said for someone else changes nothing.
    press(&ada, "u3");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(&ada, "u3");
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .backdate(COMMIT + Duration::from_secs(1));
    let before = agent(&hub, &space).0;
    hub.lock().agent_cancelled(&space, "nobody", Instant::now());
    assert_eq!(agent(&hub, &space).0, before);
    // As for words in a space since forgotten, or given for someone who has since left.
    hub.lock().agent_cancelled("gone", "ada", Instant::now());
    hub.lock().agent_given(&space, 999, Instant::now());
    assert_eq!(agent(&hub, &space).0, before);
    heard(&hub, "");
    assert_ne!(agent(&hub, &space).0, before);
    // A failed transcript does the same, here with nothing under way.
    press(&ada, "u4");
    release(&ada, "u4");
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_failed(utterance, "No speech engine.");
}

#[test]
fn it_goes_home_once_the_conversation_has_been_quiet_and_new_words_or_its_speech_put_that_off() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(&ada, "u1");
    heard(&hub, "What is this?");
    let key = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt.key),
            _ => None,
        })
        .expect("given");
    assert!(agent(&hub, &space).0.is_some());
    // While its turn runs, it stays.
    hub.space_sent(&space, &key, Ok(()));
    let start = Instant::now() + Duration::from_secs(2);
    hub.flush_snapshots(start);
    hub.flush_snapshots(start + Duration::from_secs(100));
    assert!(agent(&hub, &space).0.is_some());
    // Its turn over, a question of its own waiting holds it too.
    hub.drive_event(&space, Event::Idle);
    hub.drive_event(&space, Event::QuestionAsked(question()));
    hub.flush_snapshots(start + Duration::from_secs(101));
    hub.flush_snapshots(start + Duration::from_secs(200));
    assert!(agent(&hub, &space).0.is_some());
    hub.drive_event(
        &space,
        Event::QuestionResolved {
            id: "question-1".to_owned(),
        },
    );
    // Then quiet for 90 s, it goes home, unseen by nobody.
    let quiet = start + Duration::from_secs(201);
    hub.flush_snapshots(quiet);
    hub.flush_snapshots(quiet + Duration::from_secs(89));
    assert!(agent(&hub, &space).0.is_some());
    // Ada begins speaking again: the return waits for quiet once more.
    press(&ada, "u2");
    hub.flush_snapshots(quiet + Duration::from_secs(91));
    assert!(agent(&hub, &space).0.is_some());
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u2".to_owned(),
    });
    hub.flush_snapshots(quiet + Duration::from_secs(92));
    hub.flush_snapshots(quiet + Duration::from_secs(181));
    assert!(agent(&hub, &space).0.is_some());
    // So does its own speech, here a second of it just before it would have gone.
    hub.agent_audio(&space, 1, &frame(16_000));
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .spoke(quiet + Duration::from_secs(182), Duration::from_secs(1));
    hub.flush_snapshots(quiet + Duration::from_secs(183));
    hub.flush_snapshots(quiet + Duration::from_secs(184));
    hub.flush_snapshots(quiet + Duration::from_secs(273));
    assert!(agent(&hub, &space).0.is_some());
    hub.flush_snapshots(quiet + Duration::from_secs(275));
    assert_eq!(agent(&hub, &space), (None, None, false));
    // Home, nothing more is checked.
    assert_eq!(hub.lock().movement_deadline(), None);
}

#[test]
fn it_goes_home_sooner_once_its_partner_has_left_and_never_where_it_is_set_never_to() {
    let (hub, _jobs, space, _commands, ada) = following(Some(600));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    let start = Instant::now() + COMMIT;
    hub.flush_snapshots(start);
    hub.flush_snapshots(start + Duration::from_secs(5));
    cancel(&ada, "u1");
    assert!(agent(&hub, &space).0.is_some());
    // Ada takes off her headset: 15 s of quiet and it goes home.
    ada.send(ClientMessage::Engagement {
        state: tomori_protocol::Engagement::Away,
    });
    let quiet = start + Duration::from_secs(6);
    hub.flush_snapshots(quiet);
    hub.flush_snapshots(quiet + LEFT);
    assert_eq!(agent(&hub, &space), (None, None, false));
    // Set never to go home, it stays however long it is quiet.
    let (hub, _jobs, space, _commands, ada) = following(None);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    let start = Instant::now() + COMMIT;
    hub.flush_snapshots(start);
    hub.flush_snapshots(start + Duration::from_secs(4));
    release(&ada, "u1");
    heard(&hub, "Hello?");
    hub.flush_snapshots(start + Duration::from_secs(5));
    hub.flush_snapshots(start + Duration::from_secs(5000));
    assert!(agent(&hub, &space).0.is_some());
}

#[test]
fn called_it_comes_beside_them_and_stays_pinned_turning_only_until_sent_home_or_left_alone_long() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    ada.send(ClientMessage::CallAgent);
    let (summoned, _, pinned) = agent(&hub, &space);
    let at = summoned.expect("called");
    assert!(pinned);
    // Following the speaker, it stands beside them, its entourage home.
    assert!((level(at.position, ADA) - 1.5).abs() < 1e-3);
    let state = last_state(&ada).expect("told");
    assert!(state.pinned);
    assert_eq!(state.summoner, None);
    // Pinned, it only turns to whoever speaks, and stays however quiet it is.
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    let at_bob = [5.0, 1.6, 5.0];
    bob.send(looking(at_bob, [5.0, 1.6, 0.0]));
    press(&bob, "u1");
    let start = Instant::now();
    hub.flush_snapshots(start + COMMIT);
    assert_eq!(agent(&hub, &space), (Some(at), Some(at_bob), true));
    release(&bob, "u1");
    heard(&hub, "");
    hub.flush_snapshots(start + GLANCE);
    hub.flush_snapshots(start + Duration::from_secs(500));
    assert_eq!(agent(&hub, &space).0, Some(at));
    // A busy agent glances at a speaker even while pinned.
    hub.drive_event(&space, Event::Busy);
    press(&bob, "u2");
    assert_eq!(agent(&hub, &space).1, Some(at_bob));
    bob.send(ClientMessage::UtteranceCancel {
        utterance: "u2".to_owned(),
    });
    hub.drive_event(&space, Event::Idle);
    // With nobody in the space for half an hour, the pin ends and it goes home.
    ada.send(ClientMessage::Leave);
    bob.send(ClientMessage::Leave);
    let left = start + Duration::from_secs(600);
    hub.flush_snapshots(left);
    hub.flush_snapshots(left + Duration::from_secs(60));
    assert!(agent(&hub, &space).2);
    // Someone back meanwhile starts the half hour again.
    ada.join(&space);
    hub.flush_snapshots(left + Duration::from_secs(120));
    ada.send(ClientMessage::Leave);
    hub.flush_snapshots(left + Duration::from_secs(180));
    hub.flush_snapshots(left + EMPTY);
    assert!(agent(&hub, &space).2);
    hub.flush_snapshots(left + Duration::from_secs(180) + EMPTY);
    assert_eq!(agent(&hub, &space), (None, None, false));
}

#[test]
fn moving_only_when_asked_a_pinned_agent_still_turns_to_whoever_speaks_and_glances_while_busy() {
    let (hub, _jobs) = hub();
    let (space, _commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    ada.send(ClientMessage::CallAgent);
    let (at, _, pinned) = agent(&hub, &space);
    assert!(pinned && at.is_some());
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    let at_bob = [5.0, 1.6, 5.0];
    bob.send(looking(at_bob, [5.0, 1.6, 0.0]));
    // Pinned, it turns to Bob as he presses X, for everyone, and moves nothing.
    press(&bob, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (at, Some(at_bob), true));
    assert_eq!(last_state(&ada).expect("told").facing, Some(at_bob));
    cancel(&bob, "u1");
    // Busy, it glances at Ada, then turns back to Bob.
    hub.drive_event(&space, Event::Busy);
    press(&ada, "u2");
    let pressed = Instant::now();
    assert_eq!(agent(&hub, &space).1, Some(ADA));
    hub.flush_snapshots(pressed + GLANCE);
    assert_eq!(agent(&hub, &space), (at, Some(at_bob), true));
    cancel(&ada, "u2");
    // Sent home, its pin ends, and a press turns it no more.
    hub.lock()
        .agent_home(&space, "a test sent it", Instant::now());
    press(&bob, "u3");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, None, false));
}

#[test]
fn an_owner_sets_how_it_moves_and_keeping_it_home_brings_it_home_and_ends_its_pin() {
    let (hub, _jobs) = hub();
    let (space, _commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    // By default it moves only when asked: a press neither turns nor moves it.
    assert_eq!(
        hub.space_state(&space).movement,
        Some(AgentMovement::default())
    );
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, None, false));
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u1".to_owned(),
    });
    // Called, it stands beside her, as following would, nothing with it, pinned.
    ada.send(ClientMessage::CallAgent);
    let state = last_state(&ada).expect("told");
    assert_eq!(state.summoner, None);
    assert!(state.pinned);
    assert!((level(state.summoned.expect("called").position, ADA) - 1.5).abs() < 1e-3);
    // A way it does not know is refused; the same again changes nothing.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Unknown, None),
    });
    assert_eq!(
        ada.errors(),
        ["Tomori does not know that way for the agent to move."]
    );
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::default(),
    });
    assert!(last_state(&ada).is_none());
    // Following, it stays where it is, pinned, until the next words.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Follow, Some(5)),
    });
    let state = last_state(&ada).expect("told");
    assert_eq!(
        state.movement,
        Some(AgentMovement::new(Movement::Follow, Some(30)))
    );
    assert!(state.pinned && state.summoned.is_some());
    // Kept home, it goes home and its pin ends; and the space keeps the setting.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Home, Some(90)),
    });
    assert_eq!(agent(&hub, &space), (None, None, false));
    ada.send(ClientMessage::CallAgent);
    assert_eq!(
        ada.errors(),
        ["Tomori stays home in this space: an owner can let it move in the Space section."]
    );
    let kept = hub.lock().kept();
    assert_eq!(
        kept[0].movement,
        Some(AgentMovement::new(Movement::Home, Some(90)))
    );
    // The default is not written.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::default(),
    });
    assert_eq!(hub.lock().kept()[0].movement, None);
}

#[test]
fn a_space_kept_restores_how_its_agent_moves_and_a_way_a_newer_server_kept_leaves_the_default() {
    let directory =
        std::env::temp_dir().join(format!("tomori-hub-movement-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let kept = |id: &str, movement: serde_json::Value| {
        serde_json::json!({
            "id": id,
            "title": "Space",
            "backend": "fake",
            "directory": "/tmp",
            "session": format!("session-{id}"),
            "environment": "sky",
            "attribution": false,
            "movement": movement,
        })
    };
    std::fs::write(
        directory.join(crate::kept::SPACES_FILE),
        serde_json::json!({
            "spaces": [
                kept("following", serde_json::json!({ "mode": "follow", "return_after": 120 })),
                kept("newer", serde_json::json!({ "mode": "wander" })),
            ]
        })
        .to_string(),
    )
    .unwrap();
    let store = Arc::new(crate::kept::SpaceStore::open(&directory).unwrap());
    let (hub, _jobs) = hub();
    assert_eq!(hub.restore(store).len(), 2);
    let movement = |id: &str| hub.lock().space(id).unwrap().model.movement();
    assert_eq!(
        movement("following"),
        AgentMovement::new(Movement::Follow, Some(120))
    );
    assert_eq!(movement("newer"), AgentMovement::default());
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn its_own_tool_pins_it_or_takes_it_to_someone_its_entourage_home_while_it_follows() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // To Ada, in front of her: its entourage stays home, and it goes home again once quiet.
    hub.agent_move_agent("session", "front ada", None).unwrap();
    let state = last_state(&ada).expect("told");
    assert!(state.summoned.is_some() && !state.pinned);
    assert_eq!(state.summoner, None);
    assert_eq!(state.partner, Some(ADA));
    // Asked to stay, it is pinned.
    let stayed = hub.agent_move_agent("session", "stay", None).unwrap();
    assert_eq!(stayed["agent"]["mode"], "pinned");
    assert!(agent(&hub, &space).2);
    // Home, its pin ends, and it stands for nobody.
    hub.agent_move_agent("session", "home", None).unwrap();
    assert_eq!(agent(&hub, &space), (None, None, false));
    assert_eq!(last_state(&ada).expect("told").partner, None);
    // Pinned at home, changing environment ends the pin too.
    hub.agent_move_agent("session", "stay", None).unwrap();
    assert!(agent(&hub, &space).2);
    let mut state = hub.lock();
    let changed = hub.move_space(&mut state, &space, "farmyard".to_owned());
    assert_eq!(changed, Ok(true));
    assert!(!state.space(&space).unwrap().following.pinned);
}
