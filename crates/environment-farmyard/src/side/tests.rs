// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::Mutex;

use base_scheduler::Cancellation;
use tomori_environment::{Intent as Asked, Outlet};

use super::*;
use crate::behaviour::Doing;
use crate::flock::State;

/// What the side sent, as the server would keep it.
#[derive(Default)]
struct Kept {
    states: Mutex<Vec<String>>,
    events: Mutex<Vec<String>>,
    objects: Mutex<Vec<Vec<Object>>>,
    actions: Mutex<Vec<tomori_environment::Action>>,
}

impl Outlet for Kept {
    fn share(&self, state: String) -> Result<()> {
        self.states.lock().unwrap().push(state);
        Ok(())
    }

    fn announce(&self, event: String, state: String) -> Result<()> {
        self.events.lock().unwrap().push(event);
        self.states.lock().unwrap().push(state);
        Ok(())
    }

    fn describe(&self, objects: Vec<Object>) -> Result<()> {
        self.objects.lock().unwrap().push(objects);
        Ok(())
    }

    fn declare_actions(&self, actions: Vec<tomori_environment::Action>) -> Result<()> {
        *self.actions.lock().unwrap() = actions;
        Ok(())
    }
}

/// Wait up to a few seconds for `ready`.
fn eventually(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "not within 10 seconds");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_side_shares_its_flock_describes_it_and_answers_a_notice() {
    let cancellation = Cancellation::new();
    let kept = Arc::new(Kept::default());
    let space = Space::with_cancellation("space-1", "Fleet", "/", cancellation.child())
        .with_outlet(kept.clone());
    let inbox = space.inbox();
    let running = std::thread::spawn(move || FarmyardSide.run(&space));
    // At once, the flock as it stands, and what the agent is told of each chicken.
    eventually(|| {
        !kept.states.lock().unwrap().is_empty() && !kept.objects.lock().unwrap().is_empty()
    });
    let first: State = serde_json::from_str(&kept.states.lock().unwrap()[0]).unwrap();
    let objects = kept.objects.lock().unwrap()[0].clone();
    // Each hen, then the beach, and the campsite's bonfire and its sticks.
    assert_eq!(objects.len(), first.chickens.len() + 2 + campsite::STICKS);
    assert_eq!(objects[first.chickens.len()].id(), "beach");
    assert_eq!(objects[first.chickens.len() + 1].id(), "bonfire");
    assert_eq!(objects[0].id(), "chicken-0");
    assert!(
        objects[0]
            .description()
            .starts_with(&first.chickens[0].name)
    );
    // Each with a reference, which the side's inspector answers while it runs.
    let reference = format!("hen/{}", first.chickens[0].name.to_lowercase());
    assert_eq!(objects[0].reference(), Some(reference.as_str()));
    // And where she stands, on the ground of the yard.
    let [x, y, z] = objects[0].position().expect("described where she stands");
    assert!(
        x.is_finite() && z.is_finite() && y.abs() < 1.0,
        "{x} {y} {z}"
    );
    let inspector = inbox.inspector().expect("the side gives an inspector");
    let details: serde_json::Value =
        serde_json::from_str(&inspector.inspect(&reference).unwrap().unwrap()).unwrap();
    assert_eq!(details["name"], first.chickens[0].name.as_str());
    assert_eq!(inspector.inspect("hen/nobody"), Ok(None));
    // Something it cannot read is passed over; a notice confuses the chicken, announced with the state it leaves.
    assert!(inbox.push(Asked::new(7, "not json")));
    assert!(inbox.push(Asked::new(7, r#"{"notice": 0, "toward": [0.0, 0.0]}"#)));
    eventually(|| {
        kept.events.lock().unwrap().iter().any(|event| {
            let event: crate::flock::Event = serde_json::from_str(event).unwrap();
            event.chicken == 0 && matches!(event.behaviour.doing, Doing::Confused { .. })
        })
    });
    let last: State = serde_json::from_str(kept.states.lock().unwrap().last().unwrap()).unwrap();
    assert!(last.clock >= first.clock);
    // It stops with the space.
    cancellation.cancel();
    assert_eq!(running.join().unwrap(), Ok(()));
}

#[test]
fn the_side_declares_that_a_hen_may_be_made_to_peck_and_answers_the_agent() {
    let cancellation = Cancellation::new();
    let kept = Arc::new(Kept::default());
    let space = Space::with_cancellation("space-1", "Fleet", "/", cancellation.child())
        .with_outlet(kept.clone());
    let inbox = space.inbox();
    let running = std::thread::spawn(move || FarmyardSide.run(&space));
    eventually(|| !kept.objects.lock().unwrap().is_empty());
    // Declared once, a peck that changes only what the yard shows, a whole number of times, which each hen offers.
    assert_eq!(*kept.actions.lock().unwrap(), actions());
    assert_eq!(tomori_environment::check_actions(&actions()), Ok(()));
    assert!(!actions()[0].is_consequential());
    let objects = kept.objects.lock().unwrap()[0].clone();
    // Every hen offers it, and nothing at the campsite does.
    for object in &objects {
        let hen = object
            .reference()
            .is_some_and(|reference| reference.starts_with("hen/"));
        assert_eq!(object.offers().is_empty(), !hen, "{}", object.id());
    }
    let reference = objects[0].reference().unwrap().to_owned();
    // Taken on a hen standing about at the start, she pecks, announced to everyone; the agent is told it was done.
    let ticket = inbox.act(3, &reference, "peck", r#"{"pecks":2}"#).unwrap();
    let outcome = inbox.outcome(ticket, Duration::from_secs(10)).unwrap();
    assert!(outcome.is_done(), "{outcome:?}");
    assert!(
        outcome.message().ends_with("pecks at the ground 2 times."),
        "{outcome:?}"
    );
    eventually(|| {
        kept.events.lock().unwrap().iter().any(|event| {
            let event: crate::flock::Event = serde_json::from_str(event).unwrap();
            event.chicken == 0 && matches!(event.behaviour.doing, Doing::Peck { .. })
        })
    });
    // On a hen it does not have, it is refused, saying why.
    let ticket = inbox
        .act(3, "hen/nobody", "peck", r#"{"pecks":1}"#)
        .unwrap();
    let outcome = inbox.outcome(ticket, Duration::from_secs(10)).unwrap();
    assert_eq!(
        outcome,
        Outcome::refused("There is no such hen in the yard now.")
    );
    // It stops with the space.
    cancellation.cancel();
    assert_eq!(running.join().unwrap(), Ok(()));
}

/// An outlet that refuses everything, as one for a space that has gone might.
struct Refusing;

impl Outlet for Refusing {
    fn share(&self, _state: String) -> Result<()> {
        Err(Error::new("gone"))
    }

    fn announce(&self, _event: String, _state: String) -> Result<()> {
        Err(Error::new("gone"))
    }

    fn describe(&self, _objects: Vec<Object>) -> Result<()> {
        Err(Error::new("gone"))
    }
}

#[test]
fn the_side_stops_when_its_space_refuses_what_it_sends() {
    let chicken = Chicken::shared().unwrap();
    let space = Space::new("space-1", "Fleet", "/").with_outlet(Arc::new(Refusing));
    assert_eq!(
        run(&space, Some(chicken.clone()), Instant::now()),
        Err(Error::new("gone"))
    );
    // An outlet that keeps the state but refuses descriptions stops it as it describes.
    struct Sharing;
    impl Outlet for Sharing {
        fn share(&self, _state: String) -> Result<()> {
            Ok(())
        }
        fn announce(&self, _event: String, _state: String) -> Result<()> {
            Err(Error::new("gone"))
        }
        fn describe(&self, _objects: Vec<Object>) -> Result<()> {
            Err(Error::new("described"))
        }
    }
    let space = Space::new("space-1", "Fleet", "/").with_outlet(Arc::new(Sharing));
    assert_eq!(
        run(&space, Some(chicken.clone()), Instant::now()),
        Err(Error::new("described"))
    );
    // One that keeps descriptions but refuses events stops it at the first change, a while in.
    struct Describing;
    impl Outlet for Describing {
        fn share(&self, _state: String) -> Result<()> {
            Ok(())
        }
        fn announce(&self, _event: String, _state: String) -> Result<()> {
            Err(Error::new("announced"))
        }
        fn describe(&self, _objects: Vec<Object>) -> Result<()> {
            Ok(())
        }
    }
    let space = Space::new("space-1", "Fleet", "/").with_outlet(Arc::new(Describing));
    let started = Instant::now() - Duration::from_secs(60);
    assert_eq!(
        run(&space, Some(chicken.clone()), started),
        Err(Error::new("announced"))
    );
    // One that keeps the state but refuses its actions stops it as it declares them.
    struct Declaring;
    impl Outlet for Declaring {
        fn share(&self, _state: String) -> Result<()> {
            Ok(())
        }
        fn announce(&self, _event: String, _state: String) -> Result<()> {
            Ok(())
        }
        fn describe(&self, _objects: Vec<Object>) -> Result<()> {
            Ok(())
        }
        fn declare_actions(&self, _actions: Vec<tomori_environment::Action>) -> Result<()> {
            Err(Error::new("declared"))
        }
    }
    let space = Space::new("space-1", "Fleet", "/").with_outlet(Arc::new(Declaring));
    assert_eq!(
        run(&space, Some(chicken), Instant::now()),
        Err(Error::new("declared"))
    );
    // Without its chickens, it cannot run.
    let space = Space::new("space-1", "Fleet", "/");
    assert_eq!(
        run(&space, None, Instant::now()),
        Err(Error::new("The farmyard's chickens could not be loaded."))
    );
    // A cancelled space stops it at once, having shared the flock.
    let cancellation = Cancellation::new();
    cancellation.cancel();
    let kept = Arc::new(Kept::default());
    let space = Space::with_cancellation("space-1", "Fleet", "/", cancellation.child())
        .with_outlet(kept.clone());
    assert_eq!(FarmyardSide.run(&space), Ok(()));
    assert_eq!(kept.states.lock().unwrap().len(), 1);
}

#[test]
fn the_side_keeps_the_campsite_s_sticks_taking_the_first_grab_and_letting_go_of_what_a_leaver_held()
{
    let cancellation = Cancellation::new();
    let kept = Arc::new(Kept::default());
    let space = Space::with_cancellation("space-1", "Fleet", "/", cancellation.child())
        .with_outlet(kept.clone());
    let inbox = space.inbox();
    let running = std::thread::spawn(move || FarmyardSide.run(&space));
    eventually(|| !kept.states.lock().unwrap().is_empty());
    // Its state has every stick, on its log, beside the flock.
    let shared: campsite::Shared = serde_json::from_str(&kept.states.lock().unwrap()[0]).unwrap();
    assert_eq!(shared.sticks.len(), campsite::STICKS);
    assert!(shared.sticks.iter().all(campsite::Stick::fresh));
    // Two take stick 2 at once: the first holds it, announced to everyone; the second changes nothing.
    let sticks = |kept: &Kept| -> Vec<campsite::Event> {
        kept.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| serde_json::from_str::<campsite::Event>(event).ok())
            .collect()
    };
    inbox.set_present(vec![3, 4]);
    assert!(inbox.push(Asked::new(3, r#"{"grab": 2, "epoch": 0}"#)));
    assert!(inbox.push(Asked::new(4, r#"{"grab": 2, "epoch": 0}"#)));
    eventually(|| !sticks(&kept).is_empty());
    std::thread::sleep(Duration::from_millis(100));
    let told = sticks(&kept);
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].stick, told[0].now.held), (2, Some(3)));
    let state: campsite::Shared =
        serde_json::from_str(kept.states.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(state.sticks[2].held, Some(3));
    // Described in a hand, it is not said where it is, which only presence says; the others are, on their log.
    eventually(|| {
        kept.objects.lock().unwrap().last().is_some_and(|objects| {
            objects.iter().any(|object| {
                object.id() == "stick-2"
                    && object.description().contains("held")
                    && object.position().is_none()
            })
        })
    });
    let objects = kept.objects.lock().unwrap().last().unwrap().clone();
    assert!(
        objects
            .iter()
            .any(|object| object.id() == "stick-1" && object.position().is_some())
    );
    // The agent may inspect it, as the campsite's.
    let inspector = inbox.inspector().unwrap();
    let details: serde_json::Value =
        serde_json::from_str(&inspector.inspect("campsite/stick-2").unwrap().unwrap()).unwrap();
    assert_eq!(details["held_by"], 3);
    // Its holder gone from the space, it goes back on its log, fresh, for everyone.
    inbox.set_present(vec![4]);
    eventually(|| sticks(&kept).len() == 2);
    let back = &sticks(&kept)[1];
    assert_eq!(back.stick, 2);
    assert!(back.now.fresh());
    cancellation.cancel();
    assert_eq!(running.join().unwrap(), Ok(()));
}

#[test]
fn the_side_keeps_the_beach_s_stones_taking_one_within_reach_and_saying_so_with_the_stones_in_its_state()
 {
    let cancellation = Cancellation::new();
    let kept = Arc::new(Kept::default());
    let space = Space::with_cancellation("space-1", "Fleet", "/", cancellation.child())
        .with_outlet(kept.clone());
    let inbox = space.inbox();
    let running = std::thread::spawn(move || FarmyardSide.run(&space));
    eventually(|| !kept.states.lock().unwrap().is_empty());
    // Its state has every stone, at rest, beside the flock.
    let first: serde_json::Value = serde_json::from_str(&kept.states.lock().unwrap()[0]).unwrap();
    let stones: tomori_physics::message::State =
        serde_json::from_value(first["stones"].clone()).unwrap();
    assert_eq!(stones.bodies.len(), crate::beach::STONES);
    assert!(
        stones
            .bodies
            .iter()
            .all(|stone| !stone.moving && stone.held.is_none())
    );
    // Someone beside stone 5, their hand on it, takes it: everyone is told, as a change of the stones.
    let at = tomori_physics::Encoded(stones.bodies[5].at.0)
        .placement(tomori_physics::Precision::Rest)
        .position;
    inbox.set_present(vec![3]);
    inbox.set_presence(tomori_environment::Presence::new(
        3,
        tomori_protocol::Pose::at([at.x - 0.3, at.y + 1.4, at.z]),
        None,
        Some(tomori_protocol::Pose::at(at.to_array())),
        Instant::now(),
    ));
    let take = r#"{"take":{"body":5,"epoch":0,"hand":"right","offset":[0,0,0,0,0,0,10000]}}"#;
    assert!(inbox.push(Asked::new(3, take)));
    let changes = |kept: &Kept| -> Vec<Stones> {
        kept.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| serde_json::from_str::<Stones>(event).ok())
            .collect()
    };
    eventually(|| !changes(&kept).is_empty());
    let change = &changes(&kept)[0];
    assert_eq!(change.stones.holding[0].held.map(|held| held.by), Some(3));
    let last: serde_json::Value =
        serde_json::from_str(kept.states.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(last["stones"]["bodies"][5]["held"]["by"], 3);
    // The agent may inspect the beach, told how many are held.
    let inspector = inbox.inspector().unwrap();
    eventually(|| {
        let details: serde_json::Value =
            serde_json::from_str(&inspector.inspect("beach/shingle").unwrap().unwrap()).unwrap();
        details["held"] == 1
    });
    // Gone, they let go of it, where their hand was.
    inbox.set_present(Vec::new());
    eventually(|| {
        changes(&kept).iter().any(|change| {
            change
                .stones
                .holding
                .iter()
                .any(|holding| holding.held.is_none())
        })
    });
    cancellation.cancel();
    assert_eq!(running.join().unwrap(), Ok(()));
}

#[test]
fn the_side_announces_in_turn_at_the_pace_the_server_takes_and_drops_the_oldest_beyond_what_may_wait()
 {
    let now = Instant::now();
    let mut pace = Pace::new(now);
    for event in 0..100 {
        pace.push(event.to_string());
    }
    // The burst less the state it shared, then thirty a second, in order.
    let due = pace.due(now);
    assert_eq!(due.len(), tomori_environment::UPDATE_BURST as usize - 1);
    assert_eq!(due[0], "0");
    assert!(pace.due(now).is_empty());
    let later = pace.due(now + Duration::from_secs(1));
    assert_eq!(later.len(), tomori_environment::UPDATES_PER_SECOND as usize);
    assert_eq!(later[0], (tomori_environment::UPDATE_BURST - 1).to_string());
    // Beyond what may wait, the oldest goes.
    let mut pace = Pace::new(now);
    for event in 0..WAITING + 5 {
        pace.push(event.to_string());
    }
    assert_eq!(pace.due(now)[0], "5");
}
