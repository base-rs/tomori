// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_physics::message::{LetGo, Take};
use tomori_physics::{Body, Encoded, Precision, Settings, Static};
use tomori_protocol::Pose;

#[test]
fn the_log_says_at_most_a_burst_and_then_a_few_a_second_saying_how_many_were_not() {
    let now = Instant::now();
    let mut logged = Logged::new("stones", now);
    for line in 0..LOG_BURST as usize {
        assert_eq!(logged.line(line.to_string(), now), Some(line.to_string()));
    }
    assert_eq!(logged.line("skipped".to_owned(), now), None);
    assert_eq!(logged.line("skipped".to_owned(), now), None);
    let later = now + Duration::from_secs(1);
    assert_eq!(
        logged.line("next".to_owned(), later).unwrap(),
        "next (2 more of the stones' takings and lettings go were not logged.)"
    );
    assert_eq!(logged.line("then".to_owned(), later).unwrap(), "then");
    assert_eq!(logged.line("beyond".to_owned(), later), None);
}

/// A world of two little blocks on the ground, called sticks, numbered 7 and 8.
fn blocks(now: Instant) -> Loose {
    let block = |id: u32, x: f32| {
        let points = (0..8)
            .map(|corner| {
                let sign = |bit: u32| if corner & bit == 0 { -0.05 } else { 0.05 };
                Vec3::new(sign(1), sign(2), sign(4))
            })
            .collect();
        Body::convex(id, points).at(Placement::at(Vec3::new(x, 0.05, 0.0)))
    };
    let ground = Static::cuboid(
        Vec3::new(-5.0, -1.0, -5.0),
        Vec3::new(5.0, 0.0, 5.0),
        Quat::IDENTITY,
    );
    let world = World::new(
        Settings::default(),
        vec![ground],
        Vec::new(),
        vec![block(7, 0.0), block(8, 0.5)],
    )
    .unwrap();
    Loose::new(
        world,
        Noun {
            one: |body| format!("stick {}", body - 7),
            many: "sticks",
        },
        now,
    )
}

#[test]
fn what_is_taken_and_let_go_is_named_as_its_noun_says_and_what_was_let_go_is_said_once() {
    let now = Instant::now();
    let mut loose = blocks(now);
    let ada = Presence::new(
        3,
        Pose::at([0.0, 1.5, 0.4]),
        None,
        Some(Pose::at([0.0, 0.1, 0.0])),
        now,
    );
    let take = Intent::Take(Take {
        body: 7,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    });
    assert_eq!(
        loose.answer(3, take, std::slice::from_ref(&ada)),
        "Participant 3 took stick 0 with their right hand, at epoch 1."
    );
    assert!(loose.released().is_empty());
    let let_go = LetGo {
        body: 7,
        epoch: 1,
        at: Placement::at(Vec3::new(0.0, 0.3, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    assert!(loose.ask(
        3,
        &serde_json::to_string(&Intent::LetGo(let_go)).unwrap(),
        std::slice::from_ref(&ada)
    ));
    assert_eq!(loose.released(), [7]);
    assert!(loose.released().is_empty());
    // A letting go refused is not one let go.
    assert!(loose.ask(
        3,
        &serde_json::to_string(&Intent::LetGo(let_go)).unwrap(),
        &[ada]
    ));
    assert!(loose.released().is_empty());
    assert!(loose.world_mut().moving());
}
