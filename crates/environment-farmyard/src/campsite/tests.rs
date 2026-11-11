// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::backdrop::RIM;
use crate::layout::{BACKDROP_CENTRE, POOL};
use tomori_environment::sightlines::Solid;

fn ground(point: Vec3) -> Vec2 {
    Vec2::new(point.x, point.z)
}

/// The boxes of the campsite's solids, as the server keeps them.
fn boxes() -> Vec<Solid> {
    solids()
        .into_iter()
        .map(|(low, high)| Solid::new(low, high))
        .collect()
}

/// How far apart, across the ground, a box and a point are.
fn apart(solid: &Solid, point: Vec2) -> f32 {
    let (low, high) = (
        Vec2::new(solid.low[0], solid.low[2]),
        Vec2::new(solid.high[0], solid.high[2]),
    );
    (low - point).max(point - high).max(Vec2::ZERO).length()
}

#[test]
fn the_campsite_stands_beside_the_lookout_s_seat_off_the_trail_and_clear_of_the_pool_and_the_bush()
{
    let fire = ground(FIRE);
    // Beside the lookout's log seat, a step or two from its end.
    let (seat_middle, seat_along, seat_length, _) = trail::seat();
    let seat_end = ground(seat_middle) + seat_along * seat_length * 0.5;
    let near_end = ground(seat_middle) - seat_along * seat_length * 0.5;
    assert!(
        fire.distance(seat_end).min(fire.distance(near_end)) < 2.0,
        "{fire}"
    );
    // Outside the fence, on the level meadow, where participants stand within its reach of the backdrop's middle, so
    // that from there the backdrop stands within the far plane, as the backdrop's own tests check.
    for (middle, radius) in standing() {
        assert!(
            middle.distance(BACKDROP_CENTRE) + radius < REACH,
            "{middle}"
        );
    }
    assert!(!layout::FENCE.grown(CLEARING + 1.0).contains(fire));
    // Its clearing drawn above the ground, as the backdrop draws it, and beneath the trail's track where it meets it.
    for edge in clearing() {
        let at = ground(edge);
        assert!(
            edge.y
                > backdrop::drawn_ground(at)
                    .unwrap_or(0.0)
                    .max(backdrop::ground(at)),
            "{edge}"
        );
        assert!(edge.y < trail::LIFT + 0.1, "{edge}");
        if trail::from_middle(at) < trail::HALF {
            assert!(edge.y < trail::LIFT, "{edge}");
        }
    }
    // Clear of the pool's rim, which rises above its water, beyond where it eases back into the land, and beyond where
    // the river's banks are held as it leaves the pool, its channel further still.
    assert!(fire.distance(Vec2::new(POOL.x, POOL.z)) > RIM[3] + CLEARING);
    assert!(layout::nearest_river(fire).1 > backdrop::BANKS[2] + CLEARING);
    // Its solids off the trail, and out of the backdrop's plants, its pines and the views it keeps open.
    let views = backdrop::views();
    for solid in boxes() {
        for index in 0..trail::centreline().len() {
            let (point, _, _) = trail::frame(index);
            assert!(
                apart(&solid, point) > trail::HALF + 0.2,
                "{solid:?} {point}"
            );
        }
        assert!(views.iter().all(|(from, to)| !solid.blocks(*from, *to)));
        // Out of every plant's crown, round about its trunk, as far as it reaches.
        for plant in backdrop::plants() {
            let (reach, _) = plant.size();
            assert!(
                apart(&solid, ground(plant.at)) > reach,
                "{solid:?} {plant:?}"
            );
        }
        for (base, height) in crate::props::pines() {
            assert!(apart(&solid, ground(base)) > height * 0.32 + 0.5);
        }
        // Nor in the fence, nor the yard's own solids.
        for (low, high) in layout::solids() {
            let other = Solid::new(low, high);
            let overlaps = (0..3).all(|axis| {
                solid.low[axis] < other.high[axis] && other.low[axis] < solid.high[axis]
            });
            assert!(!overlaps);
        }
    }
    // The second seat across the fire from the lookout's, low enough to sit on, facing the fire along it.
    let (middle, along, length, radius) = seat();
    assert!((1.2..1.6).contains(&ground(middle).distance(fire)));
    assert!((0.35..0.5).contains(&seat_top(radius)));
    assert!((1.0..1.6).contains(&length));
    assert!(along.dot((fire - ground(middle)).normalize()).abs() < 0.2);
}

#[test]
fn participants_stand_about_the_fire_off_its_stones_and_seats_and_reach_it_from_the_trail() {
    let fire = ground(FIRE);
    let solids = boxes();
    let standing = standing();
    assert!(standing.len() >= 4);
    for (middle, radius) in &standing {
        // Within the clearing, outside the fire's stones.
        assert!(middle.distance(fire) + radius <= CLEARING + 0.05);
        assert!(middle.distance(fire) - radius > PIT + 0.1);
        // Nowhere within a solid.
        for solid in &solids {
            assert!(apart(solid, *middle) > *radius * 0.5, "{middle} {solid:?}");
        }
    }
    // Joined to the trail's band, so that walking there from it never leaves the ground.
    assert!(
        standing
            .iter()
            .any(|(middle, radius)| trail::from_middle(*middle) < trail::HALF + radius)
    );
    // Each disc overlaps the next round the fire, so that one can walk round it.
    for (index, (middle, radius)) in standing.iter().enumerate() {
        let (next, other) = standing[(index + 1) % standing.len()];
        let joined = middle.distance(next) < radius + other;
        let blocked = solids
            .iter()
            .any(|solid| apart(solid, (*middle + next) * 0.5) == 0.0);
        assert!(joined || blocked, "{middle} {next}");
    }
}

#[test]
fn the_sticks_lean_on_their_log_their_marshmallows_towards_the_fire_but_out_of_its_heat() {
    let (middle, along, _, radius) = seat();
    let axis_point = middle + Vec3::Y * (radius + 0.04);
    let axis = Vec3::new(along.x, 0.0, along.y);
    for index in 0..STICKS {
        let pose = Pose::racked(index);
        assert!((pose.along.length() - 1.0).abs() < 1e-4);
        // Its butt on the ground, back from the log, away from the fire, and its marshmallow over the log towards it.
        assert!(pose.butt.y.abs() < 1e-4);
        let to_fire = FIRE - middle;
        let towards = (to_fire - axis * to_fire.dot(axis)).normalize();
        assert!((pose.butt - middle).dot(towards) < -0.3);
        assert!((pose.marshmallow() - middle).dot(towards) > 0.1);
        assert!(pose.marshmallow().y > seat_top(radius));
        // Touching the log, rather than through it or above it: its shaft as far from the log's axis as their radii.
        let shaft = pose.along;
        let normal = shaft.cross(axis).normalize();
        let distance = (axis_point - pose.butt).dot(normal).abs();
        assert!(
            (distance - (radius + STICK_RADIUS)).abs() < 0.012,
            "{index} {distance}"
        );
        // Out of the fire's heat, so that a marshmallow on its log never toasts.
        assert_eq!(heat(pose.marshmallow()), 0.0);
        // Apart from the others.
        for other in 0..index {
            assert!(
                pose.marshmallow()
                    .distance(Pose::racked(other).marshmallow())
                    > 0.15
            );
        }
        // Within the log's solid, with the sticks.
        let solid = boxes()[1];
        for point in [pose.butt, pose.tip()] {
            assert!(
                solid.contains(point.to_array()) || point.y <= 0.0,
                "{point}"
            );
        }
    }
}

#[test]
fn a_stick_is_held_along_the_hand_lies_level_and_is_met_by_a_ray_passing_near_it() {
    // Held, it runs forward through the fist from a little behind it.
    let turn = Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.4);
    let hand = Vec3::new(1.0, 1.2, -3.0);
    let held = Pose::held(hand, turn);
    assert!(held.along.distance(turn * Vec3::NEG_Z) < 1e-5);
    assert!((held.butt.distance(hand) - HOLD).abs() < 1e-5);
    assert!(held.from_handle(hand) < 1e-4);
    assert!(held.tip().distance(held.butt) - STICK_LENGTH < 1e-4);
    assert!(held.marshmallow().distance(held.tip()) - MARSHMALLOW_INSET < 1e-4);
    // A nonsense turn holds it straight ahead.
    assert_eq!(
        Pose::held(hand, Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)).along,
        Vec3::NEG_Z
    );
    // Its model runs from its butt along −Z, placed so.
    let transform = held.transform();
    assert!(transform.transform_point3(Vec3::ZERO).distance(held.butt) < 1e-5);
    assert!(
        transform
            .transform_point3(Vec3::NEG_Z * STICK_LENGTH)
            .distance(held.tip())
            < 1e-4
    );
    // Lying, it is level, its middle where it lies, heading as it was let go.
    let lying = Lying {
        at: [-4.0, 0.05, -15.5],
        yaw: 1.1,
    };
    let pose = Pose::lying(&lying);
    assert_eq!(pose.along.y, 0.0);
    assert!(pose.middle().distance(Vec3::from(lying.at)) < 1e-5);
    assert!((pose.yaw() - 1.1).abs() < 1e-5);
    // A ray passing within a few centimetres of it meets it, at about where it passes, and one passing further does not;
    // nor one that points away.
    let origin = pose.middle() + Vec3::new(0.0, 1.0, 0.03);
    assert!((pose.met(origin, Vec3::NEG_Y, 0.05).unwrap() - 1.0).abs() < 0.01);
    assert_eq!(pose.met(origin + Vec3::X * 0.3, Vec3::NEG_Y, 0.05), None);
    assert_eq!(pose.met(origin, Vec3::Y, 0.05), None);
    // Along it, as a ray along its line meets it from its butt.
    let along = pose.met(pose.butt - pose.along, pose.along, 0.05).unwrap();
    assert!((along - 1.0).abs() < 1e-3);
    // The handle is the half nearest the butt.
    assert!(pose.from_handle(pose.tip()) > STICK_LENGTH * 0.45);
}

#[test]
fn a_marshmallow_toasts_over_the_heart_of_the_fire_from_white_to_golden_brown_and_black() {
    assert_eq!(heat(heart()), 1.0);
    assert_eq!(heat(heart() + Vec3::X * 0.1), 1.0);
    assert!(heat(heart() + Vec3::X * 0.4) > 0.0 && heat(heart() + Vec3::X * 0.4) < 1.0);
    assert_eq!(heat(heart() + Vec3::X * 0.61), 0.0);
    assert_eq!(heat(Vec3::NAN), 0.0);
    // White to black in 20 seconds at the heart.
    assert!((BLACK / TOAST_RATE - 16.0).abs() < 1e-3);
    const { assert!(GOLDEN < BROWN && BROWN < BLACK && BLACK < 1.0) };
    assert_eq!(shade(0.0), 0);
    assert_eq!(shade(BLACK), SHADES - 1);
    assert_eq!(shade(1.0), SHADES - 1);
    assert_eq!(shade(f32::NAN), 0);
    // Each shade darker than the last, warm until black.
    let lightness = |colour: Vec3| colour.x + colour.y + colour.z;
    for pair in (0..SHADES).collect::<Vec<_>>().windows(2) {
        assert!(lightness(colour(pair[1])) < lightness(colour(pair[0])));
    }
    let golden = colour(shade(GOLDEN));
    assert!(golden.x > golden.y && golden.y > golden.z);
    assert_eq!(looks(0.0), "fresh and white");
    assert_eq!(looks(0.1), "just warming");
    assert_eq!(looks(GOLDEN), "toasted golden");
    assert_eq!(looks(BROWN), "toasted brown");
    assert_eq!(looks(BLACK), "burnt black");
    // Black, it burns while it is still in the fire's heat.
    assert!(burning(BLACK, heart()));
    assert!(!burning(BROWN, heart()));
    assert!(!burning(1.0, heart() + Vec3::X * 2.0));
}

#[test]
fn a_stick_let_go_rests_straight_down_on_what_is_beneath_it_or_goes_back_on_its_log() {
    let fire = ground(FIRE);
    // On the clearing's earth, level, its heading kept.
    let on_earth = fire + Vec2::new(0.9, 0.6);
    let lying = rest([on_earth.x, 1.1, on_earth.y], 0.4).unwrap();
    assert_eq!(
        lying.at,
        [
            on_earth.x,
            trail::CLEARING_HEIGHT + STICK_RADIUS,
            on_earth.y
        ]
    );
    assert_eq!(lying.yaw, 0.4);
    // On the lookout's log seat, on its top; the second seat is the sticks' log, and a stick let go on it goes back.
    let (middle, _, _, radius) = trail::seat();
    let lying = rest([middle.x, 1.0, middle.z], 0.0).unwrap();
    assert!((lying.at[1] - (seat_top(radius) + STICK_RADIUS)).abs() < 1e-5);
    // On the trail's track, a little above the meadow.
    let (point, _, _) = trail::frame(39);
    assert_eq!(beneath(point), Some(trail::LIFT));
    // The meadow beyond the clearing, off the trail, as the ground is.
    let meadow = fire + Vec2::new(-1.0, -3.0);
    assert!(trail::from_middle(meadow) > trail::HALF);
    assert_eq!(beneath(meadow), Some(backdrop::ground(meadow)));
    // A nonsense heading lies along −Z.
    assert_eq!(
        rest([on_earth.x, 1.0, on_earth.y], f32::NAN).unwrap().yaw,
        0.0
    );
    // Back on its log: let go by its log, in the fire, too far away, inside the fence, or nowhere at all.
    let (middle, ..) = seat();
    for nowhere in [
        ground(middle) + Vec2::new(0.1, 0.2),
        fire + Vec2::new(0.2, 0.1),
        fire + Vec2::new(DROP_REACH + 0.5, 0.0),
        Vec2::new(-4.0, -10.5),
        Vec2::NAN,
    ] {
        assert_eq!(rest([nowhere.x, 1.0, nowhere.y], 0.0), None, "{nowhere}");
    }
}

/// Stick `index`'s epoch in `campsite`.
fn epoch(campsite: &Campsite, index: usize) -> u32 {
    campsite.sticks()[index].epoch
}

#[test]
fn the_first_to_take_a_stick_holds_it_and_only_its_holder_toasts_it_and_lets_it_go() {
    let mut campsite = Campsite::new();
    assert_eq!(campsite.sticks().len(), STICKS);
    assert!(campsite.sticks().iter().all(Stick::fresh));
    // Ada takes stick 1 at the epoch she saw; Bea, taking it too at that epoch, is refused, so that the first wins.
    assert_eq!(
        campsite.ask(1, Intent::Grab { grab: 1, epoch: 0 }, 1_000),
        Some(1)
    );
    assert_eq!(campsite.sticks()[1].held, Some(1));
    assert_eq!(epoch(&campsite, 1), 1);
    assert_eq!(
        campsite.ask(2, Intent::Grab { grab: 1, epoch: 0 }, 1_001),
        None
    );
    assert_eq!(
        campsite.ask(2, Intent::Grab { grab: 1, epoch: 1 }, 1_001),
        None
    );
    assert_eq!(campsite.sticks()[1].held, Some(1));
    // Nobody holds two, nor takes a stick that is not there, nor at an epoch that has moved on.
    assert_eq!(
        campsite.ask(1, Intent::Grab { grab: 2, epoch: 0 }, 1_002),
        None
    );
    assert_eq!(
        campsite.ask(2, Intent::Grab { grab: 9, epoch: 0 }, 1_002),
        None
    );
    assert_eq!(
        campsite.ask(2, Intent::Grab { grab: 2, epoch: 5 }, 1_002),
        None
    );
    // Her toasting is taken, as far as the fire could have toasted it since: 10 s at most toasts it half way.
    let toast = |level| Intent::Toast {
        toast: 1,
        epoch: 1,
        level,
    };
    assert_eq!(campsite.ask(1, toast(0.9), 11_000), Some(1));
    let level = campsite.sticks()[1].toast;
    assert!((level - (TOAST_RATE * 10.0 + 0.03)).abs() < 1e-4, "{level}");
    // Never less than it was, nor nonsense, nor from anyone else, nor at another epoch.
    assert_eq!(campsite.ask(1, toast(0.1), 12_000), None);
    assert_eq!(campsite.ask(1, toast(f32::NAN), 13_000), None);
    assert_eq!(campsite.ask(2, toast(0.6), 14_000), None);
    assert_eq!(
        campsite.ask(
            1,
            Intent::Toast {
                toast: 1,
                epoch: 0,
                level: 0.6
            },
            14_000
        ),
        None
    );
    assert_eq!(campsite.sticks()[1].toast, level);
    // Let go on the clearing's earth, it lies there, as toasted as she said, and anyone may take it.
    let fire = ground(FIRE);
    let release = |epoch, at: Vec2, level| Intent::Release {
        release: 1,
        epoch,
        level,
        at: [at.x, 1.2, at.y],
        yaw: 0.5,
    };
    let earth = fire + Vec2::new(0.8, 0.8);
    assert_eq!(campsite.ask(2, release(1, earth, 0.6), 20_000), None);
    assert_eq!(campsite.ask(1, release(0, earth, 0.6), 20_000), None);
    assert_eq!(campsite.ask(1, release(1, earth, 0.6), 20_000), Some(1));
    let stick = &campsite.sticks()[1];
    assert_eq!((stick.held, stick.epoch), (None, 2));
    assert!((stick.toast - 0.6).abs() < 1e-6);
    assert_eq!(stick.lying, rest([earth.x, 1.2, earth.y], 0.5));
    assert!(!stick.fresh());
    // Bea takes it from where it lies, and puts it back on its log: it is fresh again.
    assert_eq!(
        campsite.ask(2, Intent::Grab { grab: 1, epoch: 2 }, 21_000),
        Some(1)
    );
    let (middle, ..) = seat();
    assert_eq!(
        campsite.ask(2, release(3, ground(middle), 0.5), 22_000),
        Some(1)
    );
    assert!(campsite.sticks()[1].fresh());
    assert_eq!(epoch(&campsite, 1), 4);
    // Dropped in the fire, it is taken by it, and a fresh one is on its log.
    assert_eq!(
        campsite.ask(1, Intent::Grab { grab: 1, epoch: 4 }, 23_000),
        Some(1)
    );
    assert_eq!(campsite.ask(1, release(5, fire, 0.9), 24_000), Some(1));
    assert!(campsite.sticks()[1].fresh());
}

#[test]
fn a_stick_its_holder_left_with_goes_back_on_its_log_and_an_empty_campsite_tidies_itself_after_an_hour()
 {
    let mut campsite = Campsite::new();
    campsite.ask(1, Intent::Grab { grab: 0, epoch: 0 }, 0);
    campsite.ask(2, Intent::Grab { grab: 3, epoch: 0 }, 0);
    let earth = ground(FIRE) + Vec2::new(0.0, -1.0);
    campsite.ask(
        2,
        Intent::Release {
            release: 3,
            epoch: 1,
            level: 0.02,
            at: [earth.x, 1.0, earth.y],
            yaw: 0.0,
        },
        1_000,
    );
    // Until the server says who is there, nothing changes.
    assert!(campsite.sweep(None, 2_000).is_empty());
    assert_eq!(campsite.sticks()[0].held, Some(1));
    // Ada gone, the stick she held goes back on its log; Bea's, lying, stays where it lies while she is there.
    assert_eq!(campsite.sweep(Some(&[2]), 3_000), [0]);
    assert!(campsite.sticks()[0].fresh());
    assert_eq!(epoch(&campsite, 0), 2);
    assert!(campsite.sweep(Some(&[2]), 4_000).is_empty());
    // Once nobody has been there for an hour, it tidies itself; not a moment before, nor if someone came back meanwhile.
    assert!(campsite.sweep(Some(&[]), 10_000).is_empty());
    assert!(campsite.sweep(Some(&[]), 10_000 + TIDY - 1).is_empty());
    assert!(campsite.sweep(Some(&[5]), 10_000 + TIDY).is_empty());
    assert!(campsite.sweep(Some(&[]), 20_000 + TIDY).is_empty());
    assert_eq!(campsite.sweep(Some(&[]), 20_000 + TIDY * 2), [3]);
    assert!(campsite.sticks().iter().all(Stick::fresh));
    // Nothing more to tidy.
    assert!(campsite.sweep(Some(&[]), 20_000 + TIDY * 3).is_empty());
    assert_eq!(Campsite::default().sticks(), Campsite::new().sticks());
}

#[test]
fn the_agent_is_told_of_the_bonfire_and_each_stick_and_may_inspect_them() {
    let mut campsite = Campsite::new();
    campsite.ask(4, Intent::Grab { grab: 2, epoch: 0 }, 0);
    let objects = campsite.describe();
    assert_eq!(objects.len(), STICKS + 1);
    let (id, reference, description, at) = &objects[0];
    assert_eq!(
        (id.as_str(), reference.as_str()),
        ("bonfire", "campsite/bonfire")
    );
    assert!(description.contains("bonfire"));
    assert_eq!(*at, Some(heart()));
    for (index, (id, reference, description, at)) in objects[1..].iter().enumerate() {
        assert_eq!(*id, format!("stick-{index}"));
        assert_eq!(*reference, format!("campsite/stick-{index}"));
        assert!(tomori_environment::valid_reference(reference));
        assert!(description.contains("marshmallow"));
        // Where it rests, but not in a hand, which only presence says.
        if index == 2 {
            assert!(description.contains("held"));
            assert_eq!(*at, None);
        } else {
            assert!(description.contains("leaning on the log"));
            assert_eq!(*at, Some(Pose::racked(index).marshmallow()));
        }
    }
    // Lying, it says so.
    let earth = ground(FIRE) + Vec2::new(0.0, 1.0);
    campsite.ask(
        4,
        Intent::Release {
            release: 2,
            epoch: 1,
            level: 0.0,
            at: [earth.x, 1.0, earth.y],
            yaw: 0.0,
        },
        100,
    );
    assert!(campsite.describe()[3].2.contains("lying"));
    let bonfire = campsite.inspect("campsite/bonfire").unwrap();
    assert_eq!(bonfire["kind"], "bonfire");
    let stick = campsite.inspect("campsite/stick-2").unwrap();
    assert_eq!(stick["looks"], "fresh and white");
    assert_eq!(stick["on_its_log"], false);
    assert_eq!(campsite.inspect("campsite/stick-9"), None);
    assert_eq!(campsite.inspect("campsite/stick-x"), None);
    assert_eq!(campsite.inspect("hen/pip"), None);
}

#[test]
fn its_intents_events_and_state_are_json_apart_from_the_flock_s() {
    // Each intent reads back as itself, and never as one of the flock's, nor the flock's as one of its.
    let intents = [
        Intent::Grab { grab: 1, epoch: 2 },
        Intent::Toast {
            toast: 1,
            epoch: 2,
            level: 0.25,
        },
        Intent::Release {
            release: 1,
            epoch: 2,
            level: 0.5,
            at: [1.0, 2.0, 3.0],
            yaw: 0.5,
        },
    ];
    for intent in intents {
        let text = serde_json::to_string(&intent).unwrap();
        assert_eq!(serde_json::from_str::<Intent>(&text).unwrap(), intent);
        assert!(
            serde_json::from_str::<crate::flock::Intent>(&text).is_err(),
            "{text}"
        );
    }
    assert!(serde_json::from_str::<Intent>(r#"{"notice":1,"toward":[0,0]}"#).is_err());
    assert!(serde_json::from_str::<Intent>(r#"{"away":1}"#).is_err());
    // A stick on its log says nothing of a holder or where it lies.
    let event = Event {
        clock: 5,
        stick: 1,
        now: Stick::default(),
    };
    let text = serde_json::to_string(&event).unwrap();
    assert_eq!(
        text,
        r#"{"clock":5,"stick":1,"now":{"epoch":0,"toast":0.0}}"#
    );
    assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
    // The sticks are read from the farmyard's state beside the flock's, and none from a state from before them.
    let shared: Shared = serde_json::from_str(
        r#"{"clock":1,"chickens":[],"sticks":[{"epoch":3,"toast":0.5,"held":7}]}"#,
    )
    .unwrap();
    assert_eq!(shared.sticks[0].held, Some(7));
    let before: Shared = serde_json::from_str(r#"{"clock":1,"chickens":[]}"#).unwrap();
    assert!(before.sticks.is_empty());
}

#[test]
fn the_campsite_and_its_sticks_are_low_poly_and_the_anchor_stands_beyond_the_fire() {
    let count = |model: &Model, nodes: &[&str]| -> usize {
        model
            .nodes
            .iter()
            .filter(|node| nodes.is_empty() || nodes.contains(&node.name.as_deref().unwrap_or("")))
            .filter_map(|node| node.mesh)
            .flat_map(|mesh| &model.meshes[mesh].primitives)
            .map(|primitive| primitive.indices.len() / 3)
            .sum()
    };
    let campsite = model();
    let stick = stick();
    let shade = shade_node(0);
    let drawn = count(&campsite, &[]) + STICKS * count(&stick, &["stick", shade.as_str()]);
    assert!(drawn < BUDGET - 100, "{drawn}");
    // The stick has a marshmallow of every shade.
    for shade in 0..SHADES {
        assert!(
            stick
                .nodes
                .iter()
                .any(|node| node.name.as_deref() == Some(shade_node(shade).as_str()))
        );
    }
    // Its embers glow; the rest is lit.
    assert!(
        campsite
            .materials
            .iter()
            .any(|material| material.emissive.length() > 0.5)
    );
    // The campsite's anchor, of kind area, high above the fire, a little towards the trail, facing it, clear of its
    // solids by more than a worker's reach above them.
    let place = place();
    assert_eq!((place.name, place.kind), ("campsite", "area"));
    assert!(place.label.chars().count() <= 80);
    assert_eq!(place.at.y, place.towards.y);
    assert!(trail::from_middle(ground(place.at)) < trail::from_middle(ground(FIRE)));
    assert!(ground(place.at).distance(ground(FIRE)) < 1.0);
    assert!(place.at.y > FLAMES + 1.0);
    assert!(trail::from_middle(ground(place.towards)) < 1e-3);
    assert_eq!(place.notify_within, None);
}
