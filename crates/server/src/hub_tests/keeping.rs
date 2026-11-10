// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent keeping with whoever it talks with through their exchange with it, driven through the hub with the times
//! its timer would wake at: as they speak to it, while it thinks and while it replies, it turns to them as they move and,
//! once they have stood still for a moment more than 3 m from it, follows them on; never closer, nor while they keep
//! moving, nor once its reply is over, nor while pinned, moving only when asked, or for a visitor; waiting its dwell
//! only for someone new; and serving a huddle from where it stands while the speaker moves within it. See
//! `design/environments.md`.
use super::following::{ADA, agent, cancel, following, heard, last_state, level, press, release};
use super::*;
use crate::hub::following::{COMMIT, DWELL, SETTLE};

/// Ada, held past a tap, asks the agent something, and the agent, gone to her, thinks about it: its turn is running.
fn asked(hub: &Hub, ada: &Client, commands: &async_channel::Receiver<SpaceCommand>, space: &str) {
    press(ada, "u1");
    hub.flush_snapshots(Instant::now() + COMMIT);
    release(ada, "u1");
    heard(hub, "What is this?");
    let key = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt.key),
            _ => None,
        })
        .expect("given");
    hub.space_sent(space, &key, Ok(()));
    arrived(hub, space);
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
    // A step back: it turns to her where she stands now, and stays.
    let back = [0.0, 1.6, 2.3];
    stands(&ada, back, [0.0, 1.6, -3.0]);
    assert_eq!(agent(&hub, &space), (Some(for_ada), Some(back), false));
    hub.flush_snapshots(Instant::now() + SETTLE);
    assert_eq!(agent(&hub, &space).0, Some(for_ada));
    assert_eq!(last_state(&ada).expect("told").facing, Some(back));
    // A sway of her head turns it no more.
    stands(&ada, [0.04, 1.6, 2.3], [0.0, 1.6, -3.0]);
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
        (level(to_her.position, far) - 1.5).abs() < 1e-3,
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
    assert!((level(to_her.position, far) - 1.5).abs() < 1e-3);
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
    assert!((level(to_her.position, far) - 1.5).abs() < 1e-3);
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
    assert!((level(state.summoned.expect("followed").position, far) - 1.5).abs() < 1e-3);
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
        (level(for_bob.position, again) - 1.5).abs() < 1e-3,
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
    assert!((1.5..1.7).contains(&nearest), "{group:?}");
    arrived(&hub, &space);
    // He leaves her: it follows him alone.
    let alone = [7.0, 1.6, 10.0];
    stands(&bob, alone, [7.0, 1.6, 6.0]);
    hub.flush_snapshots(start + DWELL + Duration::from_secs(8));
    let for_bob = agent(&hub, &space).0.expect("following him");
    assert!(
        (level(for_bob.position, alone) - 1.5).abs() < 1e-3,
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
