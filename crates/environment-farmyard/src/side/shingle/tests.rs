// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_physics::message::{LetGo, Take};
use tomori_physics::{Body, Encoded, Precision, Side};
use tomori_protocol::Pose;

/// Someone standing beside stone `index`, their right hand on it, seen at `at`.
fn beside(shingle: &Shingle, index: u32, participant: u32, at: Instant) -> Presence {
    let stone = shingle.world().placement(index).unwrap().position;
    Presence::new(
        participant,
        Pose::at([stone.x - 0.4, stone.y + 1.5, stone.z]),
        None,
        Some(Pose::at(stone.to_array())),
        at,
    )
}

fn take(body: u32, epoch: u32) -> String {
    serde_json::to_string(&Intent::Take(Take {
        body,
        epoch,
        hand: Side::Right,
        offset: Encoded::default(),
    }))
    .unwrap()
}

fn let_go(body: u32, epoch: u32, at: Placement, velocity: [f32; 3]) -> String {
    serde_json::to_string(&Intent::LetGo(LetGo {
        body,
        epoch,
        at: at.encoded(Precision::Rest),
        velocity,
        spin: [0.0; 3],
        clock: 0,
    }))
    .unwrap()
}

#[test]
fn the_beach_s_stones_settle_on_the_shingle_within_seconds_and_then_cost_nothing() {
    let now = Instant::now();
    let mut shingle = Shingle::new(now);
    let world = shingle.world();
    assert!(!world.moving());
    let settled = world.clock();
    assert!(settled < 6_000, "settled in {settled} ms");
    assert_eq!(shingle.shared().bodies.len(), beach::STONES);
    for body in world.bodies() {
        let at = world.placement(body).unwrap().position;
        let shingle = beach::height(glam::Vec2::new(at.x, at.z)).unwrap();
        let (_, tall) = beach::size(body as usize);
        assert!(
            at.y > shingle && at.y < shingle + tall * 0.6,
            "stone {body} at {at}, shingle {shingle}"
        );
    }
    // Resting, there is nothing to say, however long nothing happens.
    assert_eq!(shingle.update(now + Duration::from_secs(1)), None);
    assert_eq!(shingle.update(now + Duration::from_secs(60)), None);
}

#[test]
fn a_stone_taken_within_reach_and_thrown_is_said_to_move_at_most_twenty_times_a_second_until_it_rests()
 {
    let start = Instant::now();
    let mut shingle = Shingle::new(start);
    let presence = vec![beside(&shingle, 0, 3, start)];
    assert!(shingle.ask(3, &take(0, 0), &presence));
    let said = shingle.update(start).unwrap();
    assert_eq!(said.holding[0].held.map(|held| held.by), Some(3));
    // Thrown out over the river, it is said as it flies, at most twenty times a second, and once where it rests.
    let from = beside(&shingle, 1, 3, start);
    let at = Placement::at(glam::Vec3::from(from.head.position) + glam::Vec3::new(0.3, -0.4, 0.0));
    let (_, out) = (beach::frame().1, beach::frame().2);
    assert!(shingle.ask(
        3,
        &let_go(0, 1, at, [out.x * 6.0, 2.0, out.y * 6.0]),
        &[from]
    ));
    let mut updates = 0;
    let mut rested = false;
    let mut now = start;
    for _ in 0..(60 * 10) {
        now += Duration::from_millis(16);
        if let Some(update) = shingle.update(now) {
            updates += 1;
            rested |= !update.rested.is_empty();
        }
    }
    assert!(
        updates <= 10 * tomori_physics::SNAPSHOT_RATE as usize + 1,
        "{updates}"
    );
    assert!(updates > 10, "{updates}");
    assert!(rested);
    // In the river now, it is soaking, to go back to the beach a minute on.
    assert!(shingle.world().moving());
}

#[test]
fn what_is_not_about_the_stones_or_is_out_of_reach_changes_nothing() {
    let now = Instant::now();
    let mut shingle = Shingle::new(now);
    assert!(!shingle.ask(3, r#"{"grab":0,"epoch":0}"#, &[]));
    assert!(!shingle.ask(3, "not json", &[]));
    // Unseen, or far away, the stone is not taken.
    assert!(shingle.ask(3, &take(0, 0), &[]));
    let far = Presence::new(3, Pose::at([0.0, 1.6, 0.0]), None, None, now);
    assert!(shingle.ask(3, &take(0, 0), &[far]));
    assert_eq!(shingle.world().held(0), None);
    assert_eq!(shingle.update(now), None);
}

#[test]
fn what_someone_who_has_gone_or_gone_quiet_holds_is_let_go_where_their_hand_was() {
    let now = Instant::now();
    let mut shingle = Shingle::new(now);
    let ada = beside(&shingle, 0, 3, now);
    let bea = beside(&shingle, 1, 4, now);
    shingle.ask(3, &take(0, 0), &[ada]);
    shingle.ask(4, &take(1, 0), &[bea]);
    assert_eq!(shingle.world().holders(), [3, 4]);
    // Both there and heard from, both keep theirs, whether or not the server has said who is there.
    shingle.sweep(Some(&[3, 4]), &[ada, bea], now + Duration::from_secs(1));
    shingle.sweep(None, &[ada, bea], now + Duration::from_secs(1));
    assert_eq!(shingle.world().holders(), [3, 4]);
    // Bea quiet too long lets go; Ada gone lets go, at her hand.
    let later = now + Duration::from_millis(SILENT);
    let ada_now = Presence::new(3, ada.head, None, ada.right, later);
    shingle.sweep(Some(&[3, 4]), &[ada_now, bea], later);
    assert_eq!(shingle.world().holders(), [3]);
    shingle.sweep(Some(&[4]), &[ada_now], later);
    assert!(shingle.world().holders().is_empty());
    let hand = glam::Vec3::from(ada.right.unwrap().position);
    assert!(
        shingle
            .world()
            .placement(0)
            .unwrap()
            .position
            .distance(hand)
            < 0.05
    );
}

#[test]
fn the_agent_is_told_of_the_beach_and_how_many_stones_are_held() {
    let now = Instant::now();
    let mut shingle = Shingle::new(now);
    let (id, reference, description, at) = shingle.describe();
    assert_eq!(
        (id.as_str(), reference.as_str()),
        ("beach", "beach/shingle")
    );
    assert!(
        description.contains("28 river stones") && !description.contains("hand"),
        "{description}"
    );
    assert!(beach::height(glam::Vec2::new(at.x, at.z)).is_some());
    let presence = vec![beside(&shingle, 2, 3, now)];
    shingle.ask(3, &take(2, 0), &presence);
    let (_, _, description, _) = shingle.describe();
    assert!(description.contains("1 in someone's hand"), "{description}");
    let inspected = inspect(&shingle.shared());
    assert_eq!(inspected["stones"], 28);
    assert_eq!(inspected["held"], 1);
    assert_eq!(inspected["moving"], 0);
}

#[test]
fn where_someone_was_seen_is_their_head_and_both_hands() {
    let now = Instant::now();
    let turned = Pose {
        position: [1.0, 1.0, 0.0],
        orientation: [0.0, 0.0, 0.0, 2.0],
    };
    let presence = [Presence::new(
        3,
        Pose::at([0.0, 1.6, 0.0]),
        Some(turned),
        None,
        now,
    )];
    let seen = seen(&presence, 3).unwrap();
    assert_eq!(seen.head, glam::Vec3::new(0.0, 1.6, 0.0));
    assert_eq!(seen.hands[0].unwrap().rotation, Quat::IDENTITY);
    assert_eq!(seen.hands[1], None);
    assert_eq!(super::seen(&presence, 4), None);
}

#[test]
fn a_cairn_of_the_beach_s_own_stones_stands_on_the_shingle_and_never_moves_asleep() {
    // Six of its slabs, the cairn's own stones, largest first, stacked a couple of millimetres apart on the flat shingle
    // beside the trail.
    let mut stones: Vec<usize> = (0..beach::STONES)
        .filter(|index| beach::kind(*index) == beach::Kind::Slab)
        .collect();
    stones.sort_by(|a, b| beach::size(*b).0.total_cmp(&beach::size(*a).0));
    stones = stones.into_iter().step_by(2).collect();
    stones.truncate(6);
    let (placed, rested) = cairn(&stones);
    // Standing: each stone on the one below it, leaning only as their tilted tops lean it, and none fallen, sunk no
    // more than the gaps it was placed with.
    standing(&placed, &rested);
}

#[test]
fn every_stone_of_the_beach_stands_on_a_big_slab_and_a_cobble_tops_a_cairn() {
    let slab = (0..beach::STONES)
        .find(|index| beach::kind(*index) == beach::Kind::Slab && beach::big(*index))
        .unwrap();
    for stone in (0..beach::STONES).filter(|stone| *stone != slab) {
        let (placed, rested) = cairn(&[slab, stone]);
        standing(&placed, &rested);
    }
    // Big slabs, then small ones, then a cobble on top.
    let mut slabs: Vec<usize> = (0..beach::STONES)
        .filter(|index| beach::kind(*index) == beach::Kind::Slab)
        .collect();
    slabs.sort_by(|a, b| beach::size(*b).0.total_cmp(&beach::size(*a).0));
    let cobble = (0..beach::STONES)
        .find(|index| beach::kind(*index) == beach::Kind::Cobble && !beach::big(*index))
        .unwrap();
    let mut stones = vec![slabs[0], slabs[3], slabs[7], slabs[10]];
    stones.push(cobble);
    let (placed, rested) = cairn(&stones);
    standing(&placed, &rested);
}

#[test]
fn the_heaviest_stones_put_on_the_lightest_settle_and_never_move_asleep() {
    // Stacked the wrong way up, as a participant may, thirty times the mass on top: each settles, standing or fallen,
    // within the seconds the others do, and then costs nothing.
    let mass = |index: usize| beach::volume(index);
    let mut light: Vec<usize> = (0..beach::STONES).collect();
    light.sort_by(|a, b| mass(*a).total_cmp(&mass(*b)));
    let heaviest = *light.last().unwrap();
    for below in light.iter().take(4) {
        cairn(&[*below, heaviest]);
    }
    let slabs: Vec<usize> = light
        .iter()
        .copied()
        .filter(|index| beach::kind(*index) == beach::Kind::Slab)
        .collect();
    cairn(&[
        slabs[0],
        slabs[slabs.len() - 1],
        slabs[1],
        slabs[slabs.len() - 2],
    ]);
}

/// Each stone in turn of `rested` on the one below it, as `placed`, leaning only as their tilted tops lean it, and none
/// fallen, sunk no more than the gaps it was placed with.
fn standing(placed: &[Placement], rested: &[Placement]) {
    for (level, (placed, at)) in placed.iter().zip(rested).enumerate() {
        assert!(
            placed.position.y - at.position.y < 0.006 * (level + 1) as f32,
            "stone {level} fell"
        );
        if level > 0 {
            let below = at.position - rested[level - 1].position;
            assert!(
                below.with_y(0.0).length() < 0.03,
                "stone {level} slid off the one below it"
            );
        }
    }
}

/// A cairn of the beach's stones `stones`, from the bottom up, stacked a couple of millimetres apart on the flat
/// shingle beside the trail, each turned its own way: where each was placed, and where each rests once it has settled,
/// which it must within 5 s, and then for two simulated minutes after.
fn cairn(stones: &[usize]) -> (Vec<Placement>, Vec<Placement>) {
    let foot = beach::at(beach::NEAR + 0.7, 0.0);
    let mut height = beach::height(foot).unwrap();
    let count = stones.len() as u32;
    let bodies: Vec<Body> = stones
        .iter()
        .enumerate()
        .map(|(level, index)| {
            let hull = beach::stone(*index);
            let lowest = hull
                .iter()
                .map(|point| point.y)
                .fold(f32::INFINITY, f32::min);
            let highest = hull
                .iter()
                .map(|point| point.y)
                .fold(f32::NEG_INFINITY, f32::max);
            let middle = glam::Vec3::new(foot.x, height - lowest + 0.002, foot.y);
            height = middle.y + highest;
            Body::convex(level as u32, hull)
                .at(Placement::new(middle, Quat::from_rotation_y(level as f32)))
        })
        .collect();
    let placed: Vec<Placement> = bodies.iter().map(|body| body.home).collect();
    let mut world = World::new(
        beach::settings(),
        beach::colliders(),
        beach::waters(),
        bodies,
    )
    .unwrap();
    assert!(world.settle(10.0), "{stones:?} never settled");
    let settled = world.clock();
    assert!(settled < 5_000, "{stones:?} settled in {settled} ms");
    let rested: Vec<Placement> = (0..count)
        .map(|body| world.placement(body).unwrap())
        .collect();
    for _ in 0..(120 * 4) {
        assert!(world.step(0.25).is_empty(), "{stones:?} moved asleep");
    }
    let after: Vec<Placement> = (0..count)
        .map(|body| world.placement(body).unwrap())
        .collect();
    assert_eq!(after, rested);
    (placed, rested)
}

#[test]
fn each_taking_and_letting_go_is_logged_taken_or_refused_and_why() {
    log::set_max_level(log::LevelFilter::Trace);
    let now = Instant::now();
    let mut shingle = Shingle::new(now);
    let ada = beside(&shingle, 0, 3, now);
    let bea = Presence::new(4, ada.head, None, ada.right, now);
    let near_one = beside(&shingle, 1, 3, now);
    let asked = |payload: &str| serde_json::from_str::<Intent>(payload).unwrap();
    let said = |shingle: &mut Shingle, by: u32, payload: &str, presence: &[Presence]| {
        shingle.answer(by, asked(payload), presence)
    };
    let refused = |line: String, why: &str| {
        assert!(line.contains(&format!("was refused: {why}.")), "{line}");
    };
    refused(
        said(&mut shingle, 3, &take(0, 0), &[]),
        "it is out of their reach, or the server has not seen where they are",
    );
    refused(
        said(&mut shingle, 3, &take(0, 5), &[ada]),
        "it has moved on to epoch 0",
    );
    refused(
        said(&mut shingle, 3, &take(99, 0), &[ada]),
        "no body numbered 99",
    );
    assert_eq!(
        said(&mut shingle, 3, &take(0, 0), &[ada]),
        "Participant 3 took stone 0 with their right hand, at epoch 1."
    );
    refused(
        said(&mut shingle, 4, &take(0, 1), &[bea]),
        "participant 3 holds it",
    );
    refused(
        said(&mut shingle, 3, &take(0, 7), &[ada]),
        "they hold it in their right hand",
    );
    refused(
        said(&mut shingle, 3, &take(1, 0), &[near_one]),
        "they hold something in that hand, or two already",
    );
    // Their letting go lost, they take it again.
    assert_eq!(
        said(&mut shingle, 3, &take(0, 2), &[ada]),
        "Participant 3 took stone 0 again, with their right hand, at epoch 3, as their letting go of it never arrived."
    );
    let at = Placement::at(glam::Vec3::from(ada.right.unwrap().position));
    refused(
        said(&mut shingle, 3, &let_go(0, 1, at, [0.0; 3]), &[ada]),
        "it has moved on to epoch 3",
    );
    refused(
        said(&mut shingle, 4, &let_go(0, 3, at, [0.0; 3]), &[bea]),
        "participant 3 holds it",
    );
    refused(
        said(&mut shingle, 3, &let_go(1, 0, at, [0.0; 3]), &[ada]),
        "nobody holds it",
    );
    let line = said(&mut shingle, 3, &let_go(0, 3, at, [0.0, 2.0, 0.0]), &[ada]);
    assert!(
        line.starts_with("Participant 3 let go of stone 0 at (")
            && line.ends_with("), moving at 2.0 m/s, at epoch 4."),
        "{line}"
    );
    // The left hand is named as such; and what is asked is logged.
    let left = serde_json::to_string(&Intent::Take(Take {
        body: 1,
        epoch: 0,
        hand: Side::Left,
        offset: Encoded::default(),
    }))
    .unwrap();
    let left_hand = Presence::new(3, near_one.head, near_one.right, None, now);
    assert_eq!(
        said(&mut shingle, 3, &left, &[left_hand]),
        "Participant 3 took stone 1 with their left hand, at epoch 1."
    );
    assert!(shingle.ask(3, &take(5, 0), &[ada]));
    // Beyond the burst, what is asked is still done, but not logged.
    for _ in 0..LOG_BURST as usize {
        assert!(shingle.ask(3, &take(99, 0), &[ada]));
    }
    assert!(shingle.logged.skipped > 0);
}

#[test]
fn the_stones_log_at_most_a_burst_and_then_a_few_a_second_saying_how_many_were_not() {
    let now = Instant::now();
    let mut logged = Logged::new(now);
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
