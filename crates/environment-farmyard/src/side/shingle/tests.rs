// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::Duration;

use glam::Quat;

use super::*;
use tomori_environment::Presence;
use tomori_physics::message::{Intent, LetGo, Take};
use tomori_physics::world::World;
use tomori_physics::{Body, Encoded, Placement, Precision, SILENT, Side};
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
    let homes = beach::homes();
    for body in world.bodies() {
        let at = world.placement(body).unwrap().position;
        let shingle = beach::height(glam::Vec2::new(at.x, at.z)).unwrap();
        let (_, tall) = beach::size(body as usize);
        if beach::building(body as usize) {
            // The building stones where they were laid, but as their gaps and the shingle's slope settle them.
            let home = homes[body as usize].position;
            assert!(
                (at - home).with_y(0.0).length() < 0.03 && (home.y - at.y) < 0.06,
                "stone {body} at {at}, laid at {home}"
            );
        } else {
            assert!(
                at.y > shingle && at.y < shingle + tall * 0.6,
                "stone {body} at {at}, shingle {shingle}"
            );
        }
    }
    // The flagstones still a pile, each on the one below it.
    let mut pile: Vec<u32> = world
        .bodies()
        .filter(|body| beach::kind(*body as usize) == beach::Kind::Flagstone)
        .collect();
    let at = |body: u32| world.placement(body).unwrap();
    pile.sort_by(|a, b| at(*a).position.y.total_cmp(&at(*b).position.y));
    for pair in pile.windows(2) {
        let (below, above) = (at(pair[0]).position, at(pair[1]).position);
        let thick = (beach::size(pair[0] as usize).1 + beach::size(pair[1] as usize).1) * 0.5;
        assert!(
            (above - below).with_y(0.0).length() < 0.05 && above.y - below.y > thick - 0.015,
            "{pair:?}: {below} {above}"
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
    // In the river's dam reach now, where it rests, it stays, as part of a dam, rather than soaking to go back.
    let at = shingle.world().placement(0).unwrap().position;
    let waters = beach::waters();
    let water = waters.iter().find(|water| water.contains(at));
    assert!(water.is_some_and(|water| water.returns_after.is_none()) && beach::stays(at));
    assert!(!shingle.world().moving());
}

/// The middle of the river's channel `along` the beach from its middle, on the ground, where it is deepest.
fn in_river(along: f32) -> glam::Vec2 {
    let out = (0..=400)
        .map(|step| beach::NEAR + 10.0 * step as f32 / 400.0)
        .min_by(|a, b| {
            let from = |out: f32| crate::layout::nearest_river(beach::at(out, along)).1;
            from(*a).total_cmp(&from(*b))
        })
        .unwrap();
    beach::at(out, along)
}

#[test]
fn stones_laid_in_the_dam_reach_stay_while_one_resting_in_the_river_beyond_it_goes_back() {
    // Two stones laid in the dam reach, one in the shallows by the beach and one in the channel's middle, and one in
    // the river downstream of the dam reach, each let go a couple of millimetres above the river's bed.
    let level = beach::water_level();
    let places = [
        (2, beach::at(beach::FAR - 0.15, 0.3)),
        (4, in_river(-0.4)),
        (6, in_river(beach::DAM_ALONG + 2.0)),
    ];
    let mut world = World::new(
        beach::settings(),
        beach::colliders(),
        beach::waters(),
        beach::bodies(),
    )
    .unwrap();
    assert!(world.settle(10.0));
    for (stone, point) in places {
        let lowest = beach::stone(stone)
            .iter()
            .map(|corner| corner.y)
            .fold(f32::INFINITY, f32::min);
        let bed = crate::backdrop::drawn_ground(point).unwrap();
        assert!(bed < level - 0.03, "{point}");
        let body = stone as u32;
        let at = Placement::at(glam::Vec3::new(point.x, bed + 0.002 - lowest, point.y));
        // Seen with a hand on it where it lies, and then with that hand where it is let go.
        let seen = |hand: Placement| tomori_physics::Seen {
            head: hand.position + glam::Vec3::new(-0.3, 1.2, 0.0),
            hands: [Some(hand), Some(hand)],
        };
        let epoch = world
            .take(
                1,
                &Take {
                    body,
                    epoch: world.epoch(body).unwrap(),
                    hand: Side::Right,
                    offset: Encoded::default(),
                },
                Some(&seen(world.placement(body).unwrap())),
            )
            .unwrap();
        let Intent::LetGo(letting) =
            serde_json::from_str(&let_go(body, epoch, at, [0.0; 3])).unwrap()
        else {
            unreachable!()
        };
        world.let_go(1, &letting, Some(&seen(at))).unwrap();
    }
    // Each comes to rest in the water, partly under it, where it was let go.
    let mut returned = Vec::new();
    for _ in 0..(5 * 4) {
        returned.extend(world.step(0.25).returning.into_iter().map(|back| back.body));
    }
    let laid: Vec<Placement> = places
        .iter()
        .map(|(stone, _)| world.placement(*stone as u32).unwrap())
        .collect();
    for ((stone, point), at) in places.iter().zip(&laid) {
        assert!(
            (at.position.with_y(0.0) - glam::Vec3::new(point.x, 0.0, point.y)).length() < 0.05,
            "stone {stone} at {}",
            at.position
        );
        let low = beach::stone(*stone)
            .iter()
            .map(|corner| at.transform_point(*corner).y)
            .fold(f32::INFINITY, f32::min);
        assert!(low < level, "stone {stone} is not in the water");
    }
    assert!(beach::stays(laid[0].position) && beach::stays(laid[1].position));
    assert!(!beach::stays(laid[2].position));
    // A minute and more on, the stones in the dam reach still lie where they were laid, and the one beyond it has gone
    // back to where it began on the beach, fading out and in, and only it.
    for _ in 0..(70 * 4) {
        returned.extend(world.step(0.25).returning.into_iter().map(|back| back.body));
    }
    assert_eq!(returned, [6]);
    for ((stone, _), at) in places.iter().zip(&laid).take(2) {
        assert_eq!(
            world.placement(*stone as u32).unwrap(),
            *at,
            "stone {stone}"
        );
    }
    assert!(world.at_home(6, 0.05));
    assert!(!world.moving());
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
        description.contains("28 river stones")
            && description.contains("8 big building stones")
            && !description.contains("hand"),
        "{description}"
    );
    assert!(beach::height(glam::Vec2::new(at.x, at.z)).is_some());
    let presence = vec![beside(&shingle, 2, 3, now)];
    shingle.ask(3, &take(2, 0), &presence);
    let (_, _, description, _) = shingle.describe();
    assert!(description.contains("1 in someone's hand"), "{description}");
    let inspected = inspect(&shingle.shared());
    assert_eq!(inspected["stones"], 36);
    assert_eq!(inspected["building"], 8);
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
    let slab = (0..beach::RIVER)
        .find(|index| beach::kind(*index) == beach::Kind::Slab && beach::big(*index))
        .unwrap();
    for stone in (0..beach::RIVER).filter(|stone| *stone != slab) {
        let (placed, rested) = cairn(&[slab, stone]);
        standing(&placed, &rested);
    }
    // Big slabs, then small ones, then a cobble on top.
    let mut slabs: Vec<usize> = (0..beach::RIVER)
        .filter(|index| beach::kind(*index) == beach::Kind::Slab)
        .collect();
    slabs.sort_by(|a, b| beach::size(*b).0.total_cmp(&beach::size(*a).0));
    let cobble = (0..beach::RIVER)
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
    let mut light: Vec<usize> = (0..beach::RIVER).collect();
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

/// One stone of a structure built of the beach's stones: which, where its middle stands from the structure's foot, out
/// from the trail and along the beach, turned how far about the upright from lying along the beach, and on which of
/// the structure's stones before it it rests, or, none, on the shingle.
struct Part {
    stone: usize,
    out: f32,
    along: f32,
    turn: f32,
    on: &'static [usize],
}

const fn part(stone: usize, out: f32, along: f32, turn: f32, on: &'static [usize]) -> Part {
    Part {
        stone,
        out,
        along,
        turn,
        on,
    }
}

/// The building stones of `kind`, broadest first.
fn building(kind: beach::Kind) -> Vec<usize> {
    let mut stones: Vec<usize> = (beach::RIVER..beach::STONES)
        .filter(|index| beach::kind(*index) == kind)
        .collect();
    stones.sort_by(|a, b| beach::size(*b).0.total_cmp(&beach::size(*a).0));
    stones
}

/// `parts`, built on the flat shingle beside the trail, each placed a couple of millimetres above what it rests on, as
/// a careful hand would: the world of them, settled, and where each was placed.
fn built(parts: &[Part]) -> (World, Vec<Placement>) {
    let (_, way, _) = beach::frame();
    let along_beach = Quat::from_rotation_y((-way.y).atan2(way.x));
    let mut placed: Vec<Placement> = Vec::new();
    let points = |stone: usize, at: &Placement| -> Vec<glam::Vec3> {
        beach::stone(stone)
            .into_iter()
            .map(|point| at.transform_point(point))
            .collect()
    };
    for part in parts {
        let foot = beach::at(beach::NEAR + 0.8 + part.out, part.along);
        let mut at = Placement::new(
            glam::Vec3::new(foot.x, 0.0, foot.y),
            along_beach * Quat::from_rotation_y(part.turn),
        );
        let lowest = points(part.stone, &at)
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let beneath = if part.on.is_empty() {
            points(part.stone, &at)
                .iter()
                .filter_map(|point| beach::height(glam::Vec2::new(point.x, point.z)))
                .fold(f32::NEG_INFINITY, f32::max)
        } else {
            part.on
                .iter()
                .flat_map(|below| points(parts[*below].stone, &placed[*below]))
                .map(|point| point.y)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        at.position.y = beneath + 0.002 - lowest;
        placed.push(at);
    }
    let bodies = parts
        .iter()
        .zip(&placed)
        .enumerate()
        .map(|(number, (part, at))| Body::convex(number as u32, beach::stone(part.stone)).at(*at))
        .collect();
    let mut world = World::new(
        beach::settings(),
        beach::colliders(),
        beach::waters(),
        bodies,
    )
    .unwrap();
    world.settle(15.0);
    (world, placed)
}

/// `parts`, built, standing: settled and asleep within 5 s, every stone within a centimetre and a half of where it was
/// placed, none fallen from what it was placed on, and then, for ten simulated minutes, nothing moves, nothing is said
/// and every stone stays put to the bit.
fn stands(name: &str, parts: &[Part]) {
    let (mut world, placed) = built(parts);
    assert!(!world.moving(), "the {name} never settled");
    let settled = world.clock();
    assert!(settled < 5_000, "the {name} settled in {settled} ms");
    let rested: Vec<Placement> = (0..parts.len() as u32)
        .map(|number| world.placement(number).unwrap())
        .collect();
    // Each no lower on what it rests on than the gap it was placed with and a little more, as contacts settle and
    // tilted tops lean it: none fell.
    let sank: Vec<f32> = placed
        .iter()
        .zip(&rested)
        .map(|(placed, rested)| placed.position.y - rested.position.y)
        .collect();
    for (number, (placed, rested)) in placed.iter().zip(&rested).enumerate() {
        let crept = (rested.position - placed.position).with_y(0.0).length();
        let beneath = parts[number]
            .on
            .iter()
            .map(|below| sank[*below])
            .fold(0.0, f32::max);
        let lower = sank[number] - beneath;
        assert!(crept < 0.015, "the {name}'s stone {number} crept {crept} m");
        assert!(lower < 0.015, "the {name}'s stone {number} sank {lower} m");
    }
    for _ in 0..(600 * 4) {
        assert!(world.step(0.25).is_empty(), "the {name} moved asleep");
    }
    let after: Vec<Placement> = (0..parts.len() as u32)
        .map(|number| world.placement(number).unwrap())
        .collect();
    assert_eq!(after, rested, "the {name} moved asleep");
}

#[test]
fn a_dolmen_of_two_blocks_and_a_flagstone_stands_and_never_moves_asleep() {
    let [first, _, second] = building(beach::Kind::Block)[..] else {
        panic!("three blocks")
    };
    let lintel = building(beach::Kind::Flagstone)[0];
    let quarter = std::f32::consts::FRAC_PI_2;
    stands(
        "dolmen",
        &[
            part(first, 0.0, -0.24, quarter, &[]),
            part(second, 0.0, 0.24, quarter, &[]),
            part(lintel, 0.0, 0.0, 0.0, &[0, 1]),
        ],
    );
}

#[test]
fn a_flagstone_on_a_cairn_of_big_slabs_stands_and_never_moves_asleep() {
    let mut slabs: Vec<usize> = (0..beach::RIVER)
        .filter(|index| beach::kind(*index) == beach::Kind::Slab && beach::big(*index))
        .collect();
    slabs.sort_by(|a, b| beach::size(*b).0.total_cmp(&beach::size(*a).0));
    let flagstones = building(beach::Kind::Flagstone);
    stands(
        "flagstone on a cairn",
        &[
            part(slabs[0], 0.0, 0.0, 0.0, &[]),
            part(slabs[1], 0.0, 0.0, 0.9, &[0]),
            part(slabs[2], 0.0, 0.0, 1.8, &[1]),
            part(flagstones[flagstones.len() - 1], 0.0, 0.0, 0.3, &[2]),
        ],
    );
}

#[test]
fn a_short_wall_of_three_blocks_and_its_coping_stands_and_never_moves_asleep() {
    let blocks = building(beach::Kind::Block);
    let coping = building(beach::Kind::Flagstone)[0];
    let width = |stone: usize| beach::size(stone).0;
    let (left, middle, right) = (blocks[1], blocks[0], blocks[2]);
    let apart = |a: usize, b: usize| (width(a) + width(b)) * 0.5 + 0.01;
    stands(
        "wall",
        &[
            part(middle, 0.0, 0.0, 0.0, &[]),
            part(left, 0.0, -apart(left, middle), 0.0, &[]),
            part(right, 0.0, apart(middle, right), 0.0, &[]),
            part(coping, 0.0, 0.0, 0.0, &[0, 1, 2]),
        ],
    );
}

#[test]
fn a_tower_of_three_blocks_capped_with_a_flagstone_stands_and_never_moves_asleep() {
    let blocks = building(beach::Kind::Block);
    let cap = building(beach::Kind::Flagstone)[1];
    let quarter = std::f32::consts::FRAC_PI_2;
    stands(
        "tower",
        &[
            part(blocks[0], 0.0, 0.0, 0.0, &[]),
            part(blocks[1], 0.0, 0.0, quarter, &[0]),
            part(blocks[2], 0.0, 0.0, 0.1, &[1]),
            part(cap, 0.0, 0.0, quarter + 0.2, &[2]),
        ],
    );
}

#[test]
fn a_flagstone_laid_beyond_its_one_pillar_falls_rather_than_being_held_up() {
    // A lintel on one pillar, its middle over the pillar's edge or beyond it, tips off and lies on the shingle; on the
    // pillar's middle, it stands. The stillness rule puts to sleep only what has stopped.
    let block = building(beach::Kind::Block)[0];
    let lintel = building(beach::Kind::Flagstone)[0];
    let half = beach::hewn(block).1 * 0.5;
    for (beyond, stands) in [(-half, true), (half * 0.3, false), (half + 0.1, false)] {
        let parts = [
            part(block, 0.0, 0.0, std::f32::consts::FRAC_PI_2, &[]),
            part(lintel, 0.0, half + beyond, 0.0, &[0]),
        ];
        let (world, placed) = built(&parts);
        assert!(!world.moving(), "{beyond}");
        let fell = placed[1].position.y - world.placement(1).unwrap().position.y;
        assert_eq!(fell < 0.01, stands, "{beyond}: it fell {fell} m");
    }
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
