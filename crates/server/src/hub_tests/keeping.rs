// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent keeping with whoever it talks with through their exchange with it, driven through the hub with the times
//! its timer would wake at: as they speak to it, while it thinks and while it replies, it turns to them as they move and,
//! once they have stood still for a moment more than 3 m from it, or kept moving for 3 s, follows them on; never
//! closer, nor once its reply is over, nor while pinned, moving only when asked, or for a visitor; waiting its dwell
//! only for someone new; and serving a huddle from where it stands while the speaker moves within it. See
//! `design/environments.md`.
use super::following::{ADA, agent, cancel, following, heard, last_state, level, press, release};
use super::*;
use crate::follow::CONVERSATION;
use crate::hub::following::{COMMIT, DWELL, POLL, RESTLESS, SETTLE, TALKED};

/// Ada, held past a tap, asks the agent something, and the agent, gone to her, thinks about it: its turn is running.
fn asked(hub: &Hub, ada: &Client, commands: &async_channel::Receiver<SpaceCommand>, space: &str) {
    press(ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(ada, "u1");
    heard(hub, "What is this?");
    sent(hub, commands, space);
    arrived(hub, space);
}

/// The words last given to the agent of `space` are sent to it.
fn sent(hub: &Hub, commands: &async_channel::Receiver<SpaceCommand>, space: &str) {
    let key = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt.key),
            _ => None,
        })
        .expect("given");
    hub.space_sent(space, &key, Ok(()));
}

/// The agent of `space` has arrived where it glides to.
fn arrived(hub: &Hub, space: &str) {
    hub.lock()
        .space_mut(space)
        .unwrap()
        .following
        .backdate(Duration::from_secs(30));
}

/// `client` stands at `at`, looking towards `target`, level.
fn stands(client: &Client, at: [f32; 3], target: [f32; 3]) {
    client.send(looking(at, target));
}

#[test]
fn while_it_thinks_it_turns_to_whoever_it_answers_and_follows_them_once_they_settle_beyond_3_m() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    // A step straight back, along the way it came to her: it already faces her, and stays.
    stands(&ada, [0.0, 1.6, 2.3], [0.0, 1.6, -3.0]);
    assert_eq!(agent(&hub, &space), (Some(for_ada), None, false));
    // A step back and aside: it turns to her where she stands now, and stays.
    let back = [0.4, 1.6, 2.3];
    stands(&ada, back, [0.0, 1.6, -3.0]);
    assert_eq!(agent(&hub, &space), (Some(for_ada), Some(back), false));
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    assert_eq!(last_state(&ada).expect("told").facing, Some(back));
    // A sway of her head turns it no more.
    stands(&ada, [0.44, 1.6, 2.3], [0.0, 1.6, -3.0]);
    assert_eq!(agent(&hub, &space).1, Some(back));
    // She walks away, turning it as she goes, and stops more than 3 m from it.
    for step in [[1.0, 1.6, 3.0], [2.5, 1.6, 3.0]] {
        stands(&ada, step, [step[0], 1.6, 0.0]);
        assert_eq!(agent(&hub, &space).1, Some(step));
    }
    let far = [3.0, 1.6, 5.0];
    stands(&ada, far, [3.0, 1.6, 0.0]);
    // Not before she has stood still a moment.
    hub.flush_snapshots(Instant::now() + SETTLE / 2);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // Then it follows her, within its dwell, which is for someone else, to stand beside her again, facing her.
    hub.flush_snapshots(Instant::now() + SETTLE);
    let (summoned, facing, pinned) = agent(&hub, &space);
    let to_her = summoned.expect("followed");
    assert!(
        (level(to_her.position, far) - CONVERSATION).abs() < 1e-3,
        "{to_her:?}"
    );
    assert_eq!((facing, pinned), (None, false));
    let state = last_state(&ada).expect("told");
    assert_eq!((state.summoned, state.partner), (Some(to_her), Some(far)));
    // On its way, her moving turns it nowhere: it faces where it goes.
    stands(&ada, [3.0, 1.6, 5.3], [3.0, 1.6, 0.0]);
    assert_eq!(agent(&hub, &space).1, None);
}

#[test]
fn while_it_replies_it_follows_them_and_once_its_reply_is_over_it_stays_until_the_next_words() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    // Its turn over, its reply still plays.
    hub.drive_event(&space, Event::Idle);
    let now = Instant::now();
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .spoke(now, Duration::from_secs(5));
    let far = [3.0, 1.6, 5.0];
    stands(&ada, far, [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    let to_her = agent(&hub, &space).0.expect("followed");
    assert!((level(to_her.position, far) - CONVERSATION).abs() < 1e-3);
    arrived(&hub, &space);
    // Its reply heard, it keeps with her no more.
    let over = now + Duration::from_secs(6);
    hub.flush_snapshots(over);
    stands(&ada, [-3.0, 1.6, 6.0], [-3.0, 1.6, 0.0]);
    hub.flush_snapshots(over + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(to_her));
    // Nor while it speaks again, as for a worker's report.
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .spoke(Instant::now(), Duration::from_secs(5));
    stands(&ada, [-4.0, 1.6, 7.0], [-4.0, 1.6, 0.0]);
    hub.flush_snapshots(over + SETTLE * 2);
    assert_eq!(agent(&hub, &space).0, Some(to_her));
    // And it goes home once the conversation is quiet, as ever.
    hub.flush_snapshots(over + Duration::from_secs(10));
    hub.flush_snapshots(over + Duration::from_secs(100));
    assert_eq!(agent(&hub, &space), (None, None, false));
}

#[test]
fn as_they_speak_it_follows_them_but_not_for_a_tap_nor_once_their_words_come_to_nothing() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    arrived(&hub, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    // Still holding X, she walks away, and it follows her once she stops.
    let far = [-3.0, 1.6, 5.0];
    stands(&ada, far, [-3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    let to_her = agent(&hub, &space).0.expect("followed");
    assert_ne!(to_her, for_ada);
    assert!((level(to_her.position, far) - CONVERSATION).abs() < 1e-3);
    arrived(&hub, &space);
    // Moving again, then letting go of X with nothing said before it checks, she is kept with no more.
    stands(&ada, [3.0, 1.6, 5.0], [3.0, 1.6, 0.0]);
    cancel(&ada, "u1");
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(to_her));
    // A tap only turns it, and keeps with nobody, even once its words are given.
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    press(&ada, "u1");
    release(&ada, "u1");
    heard(&hub, "Yes.");
    stands(&ada, [3.0, 1.6, 5.0], [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space).0, None);
}

#[test]
fn within_3_m_it_only_turns_whichever_way_they_face_and_while_they_travel_it_waits_for_them_to_arrive()
 {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    // She steps towards where it stands and turns her back on it, or faces it, standing in her way: it only turns.
    let near = [-0.5, 1.6, 2.5];
    for target in [[5.0, 1.6, 5.0], [-5.0, 1.6, -2.0]] {
        stands(&ada, near, target);
        hub.flush_snapshots(Instant::now() + SETTLE);
        assert_eq!(agent(&hub, &space), (Some(for_ada), Some(near), false));
    }
    // While she keeps moving, it waits.
    for step in [[1.0, 1.6, 3.0], [2.0, 1.6, 4.0], [3.0, 1.6, 5.0]] {
        stands(&ada, step, [step[0], 1.6, 0.0]);
        hub.flush_snapshots(Instant::now() + SETTLE / 2);
        assert_eq!(agent(&hub, &space).0, Some(for_ada));
    }
    // Teleported far, it waits for her to arrive, then streams there.
    let far = [6.0, 1.6, 8.0];
    stands(&ada, far, [6.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE / 2);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    hub.flush_snapshots(Instant::now() + SETTLE);
    let state = last_state(&ada).expect("told");
    assert!(state.stream);
    assert!((level(state.summoned.expect("followed").position, far) - CONVERSATION).abs() < 1e-3);
}

#[test]
fn someone_who_never_stands_still_beyond_3_m_is_followed_on_after_3_s() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    stands(&ada, [3.0, 1.6, 5.0], [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE / 2);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // She has walked on, or looked about, for 3 s, never still for long: it follows her on to where she is now.
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .restless(RESTLESS);
    let on = [3.5, 1.6, 5.5];
    stands(&ada, on, [3.5, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now());
    let to_her = agent(&hub, &space).0.expect("followed");
    assert!(
        (level(to_her.position, on) - CONVERSATION).abs() < 1e-3,
        "{to_her:?}"
    );
}

#[test]
fn close_by_a_fresh_press_only_turns_it_whichever_way_she_looks() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    hub.drive_event(&space, Event::Idle);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    // It stands straight ahead of her, where it came from: she looks 10° to her right, then away from it, and asks
    // again each time.
    for (turn, utterance) in [(10.0_f32, "u2"), (150.0, "u3")] {
        let turn = turn.to_radians();
        let target = [3.0 * turn.sin(), 1.6, ADA[2] - 3.0 * turn.cos()];
        stands(&ada, ADA, target);
        press(&ada, utterance);
        hub.flush_snapshots(Instant::now() + COMMIT);
        assert_eq!(agent(&hub, &space), (Some(for_ada), Some(ADA), false));
        release(&ada, utterance);
        heard(&hub, "And this?");
        assert_eq!(agent(&hub, &space), (Some(for_ada), Some(ADA), false));
        sent(&hub, &commands, &space);
        hub.drive_event(&space, Event::Idle);
    }
}

#[test]
fn pinned_moving_only_when_asked_and_for_a_visitor_it_keeps_with_nobody() {
    let far = [3.0, 1.6, 5.0];
    // Called, it stays where it was called to, though she walks away as it thinks.
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    ada.send(ClientMessage::CallAgent);
    let called = agent(&hub, &space).0.expect("called");
    asked(&hub, &ada, &commands, &space);
    stands(&ada, far, [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space), (Some(called), Some(ADA), true));
    // Moving only when asked, it stays home.
    let (hub, _jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.join(&space);
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(&ada, "u1");
    heard(&hub, "What is this?");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, SpaceCommand::Send(_)))
    );
    stands(&ada, far, [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space), (None, None, false));
    // A visitor walking about and pressing X, while it answers Ada, neither turns nor moves it.
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space);
    let vic = Client::greeted(&hub, Access::visitor(), "vic");
    vic.join(&space);
    stands(&vic, [-1.0, 1.6, 1.0], [0.0, 1.6, -3.0]);
    press(&vic, "u9");
    stands(&vic, far, [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space), for_ada);
}

#[test]
fn it_waits_its_dwell_before_keeping_up_with_someone_new_but_never_for_whoever_it_went_to() {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    let near = [-4.0, 1.6, -1.0];
    stands(&bob, near, [-4.0, 1.6, -4.0]);
    press(&ada, "u1");
    let committed = Instant::now() + COMMIT;
    hub.flush_snapshots(committed);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    hub.flush_snapshots(committed + Duration::from_secs(5));
    cancel(&ada, "u1");
    // Bob speaks within its dwell, and moves as he speaks: it only turns to him.
    press(&bob, "u2");
    hub.flush_snapshots(committed + Duration::from_millis(5500));
    let aside = [-4.5, 1.6, -1.5];
    stands(&bob, aside, [-4.5, 1.6, -4.0]);
    hub.flush_snapshots(committed + Duration::from_secs(6));
    assert_eq!(agent(&hub, &space), (Some(for_ada), Some(aside), false));
    // Its dwell over, his moving again takes it to him.
    let again = [-4.5, 1.6, -2.5];
    stands(&bob, again, [-4.5, 1.6, -5.0]);
    hub.flush_snapshots(committed + DWELL + Duration::from_secs(5));
    let for_bob = agent(&hub, &space).0.expect("gone to Bob");
    assert!(
        (level(for_bob.position, again) - CONVERSATION).abs() < 1e-3,
        "{for_bob:?}"
    );
}

#[test]
fn in_a_huddle_it_stays_while_the_speaker_moves_within_it_and_follows_the_group_or_whoever_leaves_it()
 {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    stands(&ada, [-0.5, 1.6, 2.0], [-0.5, 1.6, -3.0]);
    stands(&bob, [1.0, 1.6, 3.5], [1.0, 1.6, -3.0]);
    press(&ada, "u1");
    let start = Instant::now();
    hub.flush_snapshots(start + COMMIT);
    hub.flush_snapshots(start + Duration::from_secs(5));
    cancel(&ada, "u1");
    // Bob speaks after its dwell: it serves them both, and he goes on speaking.
    press(&bob, "u2");
    hub.flush_snapshots(start + DWELL + Duration::from_secs(5));
    let served = agent(&hub, &space).0.expect("serving them");
    arrived(&hub, &space);
    // He steps about within the huddle: it stays.
    stands(&bob, [0.0, 1.6, 4.0], [0.0, 1.6, -3.0]);
    hub.flush_snapshots(start + DWELL + Duration::from_secs(6));
    assert_eq!(agent(&hub, &space).0, Some(served));
    // They walk off together: it follows them, standing for both.
    let (at_ada, at_bob) = ([5.5, 1.6, 2.0], [7.0, 1.6, 3.5]);
    stands(&ada, at_ada, [5.5, 1.6, -3.0]);
    stands(&bob, at_bob, [7.0, 1.6, -3.0]);
    hub.flush_snapshots(start + DWELL + Duration::from_secs(7));
    let group = agent(&hub, &space).0.expect("following them");
    let nearest = level(group.position, at_ada).min(level(group.position, at_bob));
    assert!(
        (CONVERSATION..CONVERSATION + 0.2).contains(&nearest),
        "{group:?}"
    );
    arrived(&hub, &space);
    // He leaves her: it follows him alone.
    let alone = [7.0, 1.6, 10.0];
    stands(&bob, alone, [7.0, 1.6, 6.0]);
    hub.flush_snapshots(start + DWELL + Duration::from_secs(8));
    let for_bob = agent(&hub, &space).0.expect("following him");
    assert!(
        (level(for_bob.position, alone) - CONVERSATION).abs() < 1e-3,
        "{for_bob:?}"
    );
}

#[test]
fn near_its_home_it_only_turns_and_keeps_turning_to_them_and_someone_back_is_kept_with_once_they_report()
 {
    let (hub, _jobs, space, _commands, ada) = following(Some(90));
    // Ada stands within 3 m of its home: held, her press only turns it, and it keeps turning to her as she moves.
    let near = [0.0, 1.6, 0.0];
    stands(&ada, near, [0.0, 1.6, -3.0]);
    press(&ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    assert_eq!(agent(&hub, &space), (None, Some(near), false));
    let aside = [0.6, 1.6, 0.0];
    stands(&ada, aside, [0.6, 1.6, -3.0]);
    assert_eq!(agent(&hub, &space), (None, Some(aside), false));
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space), (None, Some(aside), false));
    assert_eq!(last_state(&ada).expect("told").partner, None);
    // While it thinks, she moves away, then leaves and comes back before it checks: it waits until she says where she
    // is again, and then follows her.
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    let far = [3.0, 1.6, 5.0];
    stands(&ada, far, [3.0, 1.6, 0.0]);
    ada.send(ClientMessage::Leave);
    ada.join(&space);
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    stands(&ada, [3.0, 1.6, 5.5], [3.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE * 2);
    assert_ne!(agent(&hub, &space).0, Some(for_ada));
}

/// Where the farmyard stands in its own coordinates, as `tomori-environment-farmyard`'s layout put it in November 2026,
/// with its workbench (#314), which the core may not depend on: where participants arrive, the way they face, and the
/// agent's home, at the heart of the workbench's curve.
const FARMYARD_ARRIVAL: [f32; 3] = [0.6, 0.0, -2.4];
const FARMYARD_FACING: [f32; 3] = [-1.6, 0.0, -7.0];
const FARMYARD_HOME: [f32; 3] = [-2.5, 1.5, -0.84];

/// The farmyard's solids, as its layout gave them then: the coop, its ramp, the trough, the feeder, the hay, the notice
/// board, the workbench's five pieces and its rack's eleven boards.
const FARMYARD_SOLIDS: [([f32; 3], [f32; 3]); 22] = [
    ([-3.98, 0.0, -9.58], [-1.62, 2.3, -7.62]),
    ([-3.01, 0.0, -7.8], [-2.59, 0.6, -6.2]),
    ([3.0, 0.0, -8.5], [4.4, 0.38, -8.05]),
    ([0.52, 0.0, -7.08], [1.08, 0.35, -6.52]),
    ([3.0, 0.0, -10.7], [4.6, 1.2, -9.5]),
    ([4.585, 0.0, -5.665], [4.675, 2.05, -3.935]),
    ([-4.8, 0.0, -1.75], [-3.92, 0.97, -0.93]),
    ([-4.8, 0.0, -0.97], [-3.98, 0.97, -0.18]),
    ([-4.7, 0.0, -0.39], [-3.74, 0.97, 0.53]),
    ([-4.34, 0.0, 0.09], [-3.35, 0.97, 1.08]),
    ([-3.76, 0.0, 0.46], [-2.86, 0.97, 1.4]),
    ([-4.84, 0.0, -1.8], [-4.65, 1.9, -1.44]),
    ([-4.9, 0.0, -1.46], [-4.76, 1.9, -1.11]),
    ([-4.92, 0.0, -1.12], [-4.82, 1.9, -0.77]),
    ([-4.92, 0.0, -0.77], [-4.8, 1.9, -0.42]),
    ([-4.88, 0.0, -0.43], [-4.72, 1.9, -0.08]),
    ([-4.8, 0.0, -0.1], [-4.59, 1.9, 0.25]),
    ([-4.66, 0.0, 0.21], [-4.42, 1.9, 0.55]),
    ([-4.48, 0.0, 0.5], [-4.2, 1.9, 0.82]),
    ([-4.26, 0.0, 0.77], [-3.95, 1.9, 1.06]),
    ([-4.0, 0.0, 1.0], [-3.67, 1.9, 1.26]),
    ([-3.71, 0.0, 1.19], [-3.36, 1.9, 1.41]),
];

/// A hub with an idle space in the farmyard, laid out as Tomori's own server registers it, its workbench among it, whose
/// owner, Samuel, chose that its agent follows the speaker and goes home after 90 s.
fn in_the_farmyard() -> (
    Arc<Hub>,
    async_channel::Receiver<Job>,
    String,
    async_channel::Receiver<SpaceCommand>,
    Client,
) {
    use tomori_protocol::Pose;
    let (hub, jobs) = hub_waiting();
    let eyes = [
        FARMYARD_ARRIVAL[0],
        FARMYARD_ARRIVAL[1] + tomori_environment::sightlines::EYES,
        FARMYARD_ARRIVAL[2],
    ];
    hub.set_environments(Environments::new([
        crate::Environment::new("sky", "Sky"),
        crate::Environment::new("farmyard", "Farmyard")
            .with_arrival(Pose::at(FARMYARD_ARRIVAL).facing_towards(FARMYARD_FACING))
            .with_agent(Pose::at(FARMYARD_HOME).facing_towards(eyes))
            .with_solids(FARMYARD_SOLIDS),
    ]));
    let (space, commands) = idle(&hub);
    let samuel = Client::greeted(&hub, Access::open(), "Samuel");
    samuel.join(&space);
    samuel.send(ClientMessage::SetAgentMovement {
        movement: tomori_protocol::AgentMovement::new(tomori_protocol::Movement::Follow, Some(90)),
    });
    samuel.send(ClientMessage::SetEnvironment {
        environment: "farmyard".to_owned(),
    });
    assert!(samuel.errors().is_empty());
    (hub, jobs, space, commands, samuel)
}

/// The bearing, in degrees clockwise from −Z, from `from` to `to`, as `presence` says the way the agent faces.
fn bearing(from: [f32; 3], to: [f32; 3]) -> f32 {
    (to[0] - from[0]).atan2(-(to[2] - from[2])).to_degrees()
}

/// How far apart, in degrees, the bearings `a` and `b` are, either way round.
fn apart(a: f32, b: f32) -> f32 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

/// Where the agent of `space` stands at `now`, and the way it faces, in degrees clockwise from −Z, as its `presence`
/// tool says them.
fn presence(hub: &Hub, space: &str, now: Instant) -> ([f32; 3], f32) {
    let state = hub.lock();
    let report = Hub::agent_report(&state, &hub.environments(), space, now);
    let at: Vec<f32> = serde_json::from_value(report["position"].clone()).unwrap();
    (
        [at[0], at[1], at[2]],
        report["facing_yaw"].as_f64().unwrap() as f32,
    )
}

#[test]
fn in_the_farmyard_it_follows_and_faces_them_for_as_long_as_its_voice_plays_after_its_turn_ends() {
    let (hub, _jobs, space, commands, samuel) = in_the_farmyard();
    // Samuel sits by the workbench, a couple of metres from the agent's home, and asks it something: near enough, it
    // only turns to him.
    let by_the_bench = [-3.5, 1.1, 1.26];
    stands(&samuel, by_the_bench, [-3.5, 1.1, -3.0]);
    press(&samuel, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(&samuel, "u1");
    heard(&hub, "What is the coop made of?");
    // While its words wait to be sent, and it thinks, it keeps with him.
    hub.flush_snapshots(Instant::now() + Duration::from_millis(1500));
    sent(&hub, &commands, &space);
    hub.flush_snapshots(Instant::now() + Duration::from_millis(3000));
    // Its turn ends with its reply written, which its voice has queued but not yet begun to say: synthesis takes a
    // couple of seconds, through which he walks off across the yard, and settles there, more than 3 m from it, so it
    // goes to him.
    hub.speech_queued(&space, 2);
    hub.drive_event(&space, Event::Idle);
    let across = [4.46, 1.07, -6.79];
    stands(&samuel, across, [4.46, 1.07, -10.0]);
    hub.flush_snapshots(Instant::now() + Duration::from_millis(1000));
    hub.flush_snapshots(Instant::now() + Duration::from_millis(2000));
    // Then its voice plays, for twenty seconds, after its turn ended.
    let voice = Instant::now();
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .spoke(voice, Duration::from_secs(20));
    hub.flush_snapshots(Instant::now() + SETTLE);
    let to_him = agent(&hub, &space).0.expect("followed him across the yard");
    let gap = level(to_him.position, across);
    assert!(
        (crate::follow::DISTANCES[2] - 1e-3..=crate::follow::FALLBACK + 1e-3).contains(&gap),
        "{gap} m from him at {to_him:?}"
    );
    // It faces him from where it goes, and says so.
    let towards = bearing(to_him.position, across);
    assert!(
        apart(crate::follow::faces(&to_him).to_degrees(), towards).abs() < 3.0,
        "{to_him:?}"
    );
    arrived(&hub, &space);
    let (at, facing) = presence(&hub, &space, Instant::now());
    assert!(level(at, to_him.position) < 0.01, "{at:?}");
    assert!(apart(facing, towards).abs() < 3.0, "{facing}");
    // Still speaking, it turns to him as he steps aside, within 3 m, and its presence says the way it now faces.
    let aside = [across[0] + 1.0, across[1], across[2] + 0.5];
    stands(&samuel, aside, [aside[0], 1.07, -10.0]);
    assert_eq!(agent(&hub, &space), (Some(to_him), Some(aside), false));
    let turned = bearing(to_him.position, aside);
    let (_, facing) = presence(&hub, &space, Instant::now());
    assert!(apart(facing, turned).abs() < 3.0, "{facing} for {turned}");
    // Its voice heard, the last of it sent, its reply is over: it still faces him, and keeps with him no more.
    hub.speech_sent(&space);
    hub.speech_sent(&space);
    let over = voice + Duration::from_secs(21);
    hub.flush_snapshots(over);
    stands(&samuel, [0.0, 1.6, 0.0], [0.0, 1.6, -3.0]);
    hub.flush_snapshots(over + SETTLE * 2);
    assert_eq!(agent(&hub, &space).0, Some(to_him));
    let (_, facing) = presence(&hub, &space, over);
    assert!(apart(facing, turned).abs() < 3.0, "{facing} for {turned}");
}

#[test]
fn at_an_environment_s_follow_height_arrived_it_only_turns_to_presses_and_settles_within_3_m_across_the_floor()
 {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    // Its home floating 5.3 m up, it hovers 5.1 m above the floor as it comes to someone: about 4 m from Ada's eyes.
    let high = tomori_protocol::Pose::at([0.0, 5.3, -4.0]).facing_towards([0.0, 1.6, 0.0]);
    hub.set_environments(Environments::new([crate::Environment::new("sky", "Sky")
        .with_agent(high)
        .with_follow_height(5.1)]));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    asked(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    assert!((for_ada.position[1] - 5.1).abs() < 1e-4, "{for_ada:?}");
    assert!(crate::follow::distance(for_ada.position, ADA) > 4.0);
    // Settling a step aside, within 3 m of it across the floor, only turns it, again and again.
    for (n, step) in [[0.5, 1.6, 2.2], [-0.5, 1.6, 2.4], [0.3, 1.6, 2.6]]
        .into_iter()
        .enumerate()
    {
        stands(&ada, step, [0.0, 1.6, -3.0]);
        hub.flush_snapshots(Instant::now() + SETTLE * (n as u32 + 2));
        assert_eq!(agent(&hub, &space).0, Some(for_ada), "{step:?}");
    }
    // A fresh press and its words, from where she stands, only turn it.
    hub.drive_event(&space, Event::Idle);
    for utterance in ["u2", "u3"] {
        press(&ada, utterance);
        hub.flush_snapshots(Instant::now() + COMMIT * 2);
        release(&ada, utterance);
        cancel(&ada, utterance);
        assert_eq!(agent(&hub, &space).0, Some(for_ada), "{utterance}");
    }
}

/// Ada, held past a tap, asks the agent something, and its reply to her is over: its turn ended, with no speech to come.
fn answered(
    hub: &Hub,
    ada: &Client,
    commands: &async_channel::Receiver<SpaceCommand>,
    space: &str,
) {
    asked(hub, ada, commands, space);
    hub.drive_event(space, Event::Idle);
    hub.flush_snapshots(Instant::now() + POLL);
}

/// A sub-agent's report reaches the agent of `space`, not through Tomori, and it thinks about it.
fn reported(hub: &Hub, space: &str) {
    hub.drive_event(space, Event::Busy);
    hub.drive_event(
        space,
        Event::Delivered {
            key: "report".to_owned(),
        },
    );
}

#[test]
fn a_spoken_reply_to_a_worker_s_report_goes_to_whom_it_talks_with_and_keeps_with_them_through_it() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    answered(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    // She walks across the yard, her exchange with it over: it stays.
    let far = [12.0, 1.6, 8.0];
    stands(&ada, far, [12.0, 1.6, 0.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // A worker's report reaches it: thinking, it moves nothing.
    reported(&hub, &space);
    hub.flush_snapshots(Instant::now() + SETTLE * 2);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // Its reply's speech queued, it comes to her, 2 m from her, as though she had spoken to it.
    hub.speech_queued(&space, 2);
    let to_her = agent(&hub, &space).0.expect("followed");
    assert!(
        (level(to_her.position, far) - CONVERSATION).abs() < 1e-3,
        "{to_her:?}"
    );
    hub.speech_queued(&space, 1);
    assert_eq!(agent(&hub, &space).0, Some(to_her));
    // Its turn over, its voice still to come, it follows her on as she walks on.
    hub.drive_event(&space, Event::Idle);
    arrived(&hub, &space);
    let on = [12.0, 1.6, 14.0];
    stands(&ada, on, [12.0, 1.6, 20.0]);
    hub.flush_snapshots(Instant::now() + SETTLE);
    let on_her = agent(&hub, &space).0.expect("followed on");
    assert!(
        (level(on_her.position, on) - CONVERSATION).abs() < 1e-3,
        "{on_her:?}"
    );
    // Its voice heard, it keeps with her no more.
    for _ in 0..3 {
        hub.speech_sent(&space);
    }
    arrived(&hub, &space);
    hub.flush_snapshots(Instant::now() + POLL);
    stands(&ada, [4.0, 1.6, 14.0], [4.0, 1.6, 20.0]);
    hub.flush_snapshots(Instant::now() + POLL + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(on_her));
}

#[test]
fn home_it_still_speaks_to_whom_it_talks_with_but_not_once_they_spoke_long_ago_or_have_left() {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    stands(&bob, [-6.0, 1.6, 6.0], [0.0, 1.6, 0.0]);
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    answered(&hub, &ada, &commands, &space);
    // Sent home, it still talks with Ada.
    ada.send(ClientMessage::PlaceAgent { at: None });
    assert_eq!(agent(&hub, &space).0, None);
    let far = [12.0, 1.6, 8.0];
    stands(&ada, far, [12.0, 1.6, 0.0]);
    reported(&hub, &space);
    hub.speech_queued(&space, 1);
    let to_her = agent(&hub, &space).0.expect("gone to her");
    assert!((level(to_her.position, far) - CONVERSATION).abs() < 1e-3);
    hub.speech_sent(&space);
    hub.drive_event(&space, Event::Idle);
    ada.send(ClientMessage::PlaceAgent { at: None });
    // That reply was no conversation of hers: ten minutes after she last spoke to it, it speaks from home.
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .following
        .talked_ago(TALKED);
    reported(&hub, &space);
    hub.speech_queued(&space, 1);
    assert_eq!(agent(&hub, &space).0, None);
    hub.speech_sent(&space);
    hub.drive_event(&space, Event::Idle);
    // She speaks to it again, and leaves: it goes to nobody, nor to Bob, who never spoke to it.
    press(&ada, "u2");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(&ada, "u2");
    heard(&hub, "And this?");
    sent(&hub, &commands, &space);
    hub.drive_event(&space, Event::Idle);
    assert!(agent(&hub, &space).0.is_some());
    ada.send(ClientMessage::PlaceAgent { at: None });
    ada.send(ClientMessage::Leave);
    reported(&hub, &space);
    hub.speech_queued(&space, 1);
    assert_eq!(agent(&hub, &space).0, None);
}

#[test]
fn a_reply_to_a_worker_s_report_moves_nothing_unspoken_pinned_walking_with_another_or_moving_only_when_asked()
 {
    let (hub, _jobs, space, commands, ada) = following(Some(90));
    stands(&ada, ADA, [0.0, 1.6, -3.0]);
    answered(&hub, &ada, &commands, &space);
    let for_ada = agent(&hub, &space).0.expect("gone to Ada");
    let far = [12.0, 1.6, 8.0];
    stands(&ada, far, [12.0, 1.6, 0.0]);
    // Saying nothing aloud, it moves nothing.
    reported(&hub, &space);
    reply(&hub, &space, "m1", "Noted.");
    hub.drive_event(&space, Event::Idle);
    hub.flush_snapshots(Instant::now() + POLL);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    // Pinned, it stays.
    ada.send(ClientMessage::PinAgent { pinned: true });
    reported(&hub, &space);
    hub.speech_queued(&space, 1);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    hub.speech_sent(&space);
    hub.drive_event(&space, Event::Idle);
    // Walking with Bob, it stays with him.
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    let his = [-3.0, 1.6, 0.0];
    stands(&bob, his, [0.0, 1.6, 0.0]);
    bob.send(ClientMessage::WalkAgent { walking: true });
    let with_him = agent(&hub, &space).0.expect("walks with Bob");
    arrived(&hub, &space);
    reported(&hub, &space);
    hub.speech_queued(&space, 1);
    assert_eq!(agent(&hub, &space).0, Some(with_him));
    hub.speech_sent(&space);
    hub.drive_event(&space, Event::Idle);
    bob.send(ClientMessage::WalkAgent { walking: false });
    // Moving only when asked, it stays too.
    ada.send(ClientMessage::SetAgentMovement {
        movement: tomori_protocol::AgentMovement::new(tomori_protocol::Movement::Asked, Some(90)),
    });
    reported(&hub, &space);
    hub.speech_queued(&space, 1);
    assert_eq!(agent(&hub, &space).0, Some(with_him));
}
