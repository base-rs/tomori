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
fn a_stick_is_held_along_the_hand_by_its_handle_and_met_by_a_ray_passing_near_it() {
    // Held, it runs forward through the fist from a little behind it.
    let turn = Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.4);
    let hand = Vec3::new(1.0, 1.2, -3.0);
    let held = Pose::held(hand, turn);
    assert!(held.along.distance(turn * Vec3::NEG_Z) < 1e-5);
    assert!((held.butt.distance(hand) - HOLD).abs() < 1e-5);
    assert!(held.from_handle(hand) < 1e-4);
    assert!(held.tip().distance(held.butt) - STICK_LENGTH < 1e-4);
    assert!(held.marshmallow().distance(held.tip()) - MARSHMALLOW_INSET < 1e-4);
    // Its body is held by its handle, within what a hand may hold, as the grip says.
    let body = Placement::new(hand, turn).then(&grip());
    let read = Pose::of(&body);
    assert!(read.butt.distance(held.butt) < 1e-5 && read.along.distance(held.along) < 1e-5);
    assert!(grip().position.length() < tomori_physics::HOLD);
    // A nonsense turn holds it straight ahead.
    assert_eq!(
        Pose::held(hand, Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)).along,
        Vec3::NEG_Z
    );
    // Its model runs from its butt along −Z, drawn so for its body, turned about its length as the body is.
    for transform in [held.transform(), drawn(&body)] {
        assert!(transform.transform_point3(Vec3::ZERO).distance(held.butt) < 1e-5);
        assert!(
            transform
                .transform_point3(Vec3::NEG_Z * STICK_LENGTH)
                .distance(held.tip())
                < 1e-4
        );
    }
    // A pose's placement reads back as the pose, whichever way it is turned about its length.
    let lying = Pose {
        butt: Vec3::new(-4.0, 0.05, -15.5),
        along: Vec3::new(0.6, 0.0, -0.8),
    };
    for roll in [0.0, 1.0, -2.5] {
        let back = Pose::of(&lying.placement(roll));
        assert!(back.butt.distance(lying.butt) < 1e-5 && back.along.distance(lying.along) < 1e-5);
    }
    // A ray passing within a few centimetres of it meets it, at about where it passes, and one passing further does not;
    // nor one that points away.
    let pose = lying;
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
fn a_stick_is_a_light_body_of_its_rod_and_marshmallow_drawn_as_it_collides() {
    let hull = hull();
    assert!(hull.len() <= tomori_physics::HULL_POINTS);
    // Every corner of its model, its rod and each shade of its marshmallow, is a point of its hull, about its middle.
    let model = stick();
    let corners: Vec<Vec3> = model
        .nodes
        .iter()
        .filter_map(|node| node.mesh)
        .flat_map(|mesh| &model.meshes[mesh].primitives)
        .flat_map(|primitive| primitive.positions.iter())
        .map(|corner| *corner + Vec3::Z * STICK_LENGTH * 0.5)
        .collect();
    for corner in &corners {
        assert!(
            hull.iter().any(|point| point.distance(*corner) < 1e-5),
            "{corner}"
        );
    }
    // Its middle is the hull's, between its butt and its tip.
    let (low, high) = hull.iter().fold(
        (Vec3::INFINITY, Vec3::NEG_INFINITY),
        |(low, high), point| (low.min(*point), high.max(*point)),
    );
    assert!(
        (low.z + STICK_LENGTH * 0.5).abs() < 1e-5 && (high.z - STICK_LENGTH * 0.5).abs() < 1e-5
    );
    let widest = hull
        .iter()
        .map(|point| point.truncate().length())
        .fold(0.0, f32::max);
    assert!((widest - MARSHMALLOW.1).abs() < 1e-5);
    // Bodies of their own numbers, which name their sticks, of dry wood, lighter than water, so that they float.
    let bodies = bodies();
    assert_eq!(bodies.len(), STICKS);
    const { assert!(DENSITY < 1_000.0) };
    for (index, body) in bodies.iter().enumerate() {
        assert_eq!(body.id, super::body(index));
        assert_eq!(super::index(body.id), Some(index));
        assert_eq!(body.check(), Ok(()));
        assert_eq!(body.kind, tomori_physics::Kind::Dynamic);
        assert_eq!(body.home, home(index));
        assert!(Pose::of(&body.home).butt.distance(Pose::racked(index).butt) < 1e-4);
    }
    assert_eq!(super::index(FIRST_STICK - 1), None);
    assert_eq!(super::index(super::body(STICKS)), None);
    assert_eq!(super::index(3), None);
}

#[test]
fn what_the_sticks_rest_on_is_drawn_and_within_the_physics_limits() {
    let colliders = colliders();
    assert!(colliders.iter().all(Static::valid));
    let triangles: usize = colliders.iter().map(Static::triangles).sum();
    assert!(triangles <= tomori_physics::STATIC_TRIANGLES, "{triangles}");
    // The ground about the fire as it is drawn, with its clearing, both log seats, the fire's ring of stones and its
    // bonfire.
    assert_eq!(colliders.len(), 1 + 2 + PIT_STONES as usize + 1);
    // The pool's water, below the campsite, within its reach, which takes back what rests in it.
    let pool = &waters()[0];
    assert_eq!(pool.returns_after, Some(RETURNS));
    assert!(in_pool(layout::POOL + Vec3::new(1.0, -0.1, 0.5)));
    assert!(!in_pool(heart()));
    let settings = settings();
    let far = Vec2::new(layout::POOL.x, layout::POOL.z) - ground(FIRE);
    assert!(far.length() + layout::POOL_RADIUS < settings.lost_beyond + 0.5);
    assert_eq!(settings.home, Vec3::new(FIRE.x, 0.0, FIRE.z));
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
fn a_marshmallow_held_in_the_heat_toasts_by_how_near_and_how_long() {
    // Ten seconds over the heart of the fire, half way; twice as long, to black and beyond, but never past 1.
    assert!((toasted(0.0, heart(), 10.0) - TOAST_RATE * 10.0).abs() < 1e-6);
    assert!(toasted(0.0, heart(), 20.0) > BLACK);
    assert_eq!(toasted(0.9, heart(), 60.0), 1.0);
    // Further off, slower; beyond its heat, not at all; nor for no time, nor nonsense.
    let edge = heart() + Vec3::X * 0.45;
    assert!(toasted(0.2, edge, 1.0) > 0.2 && toasted(0.2, edge, 1.0) < toasted(0.2, heart(), 1.0));
    assert_eq!(toasted(0.2, heart() + Vec3::X * 0.7, 30.0), 0.2);
    assert_eq!(toasted(0.2, heart(), 0.0), 0.2);
    assert_eq!(toasted(0.2, heart(), f32::NAN), 0.2);
    assert_eq!(toasted(0.2, heart(), -5.0), 0.2);
    // A marshmallow on its log never toasts.
    for index in 0..STICKS {
        assert_eq!(toasted(0.0, Pose::racked(index).marshmallow(), 60.0), 0.0);
    }
}

#[test]
fn a_stick_that_comes_to_rest_stays_where_it_lies_or_goes_back_on_its_log() {
    let fire = ground(FIRE);
    let at = |point: Vec2, height: f32| Vec3::new(point.x, height, point.y);
    // On the clearing's earth, on the lookout's log seat, on the trail or on the meadow, it stays.
    let (lookout, ..) = trail::seat();
    let (point, _, _) = trail::frame(39);
    let lying = |middle: Vec3, yaw: f32| Placement::new(middle, Quat::from_rotation_y(yaw));
    for stays in [
        at(fire + Vec2::new(0.9, 0.6), 0.02),
        lookout + Vec3::Y * 0.45,
        at(point, trail::LIFT),
        at(fire + Vec2::new(-1.0, -3.0), 0.0),
    ] {
        assert!(!goes_back(&lying(stays, 0.0)), "{stays}");
    }
    // Floating in the pool, it stays until the pool takes it back.
    assert!(!goes_back(&lying(
        layout::POOL + Vec3::new(0.5, -0.02, 0.0),
        1.0
    )));
    // By its log, in the fire, carried away or nowhere at all, it goes back on its log.
    let (middle, ..) = seat();
    assert!(by_its_log(middle + Vec3::new(0.1, 0.3, 0.2)));
    assert!(!by_its_log(at(fire, 0.0)));
    for back in [
        middle + Vec3::new(0.1, 0.4, 0.2),
        at(fire + Vec2::new(0.2, 0.1), 0.1),
        at(fire + Vec2::new(DROP_REACH + 0.5, 0.0), 0.0),
        Vec3::NAN,
    ] {
        assert!(goes_back(&lying(back, 0.3)), "{back}");
    }
    // Its middle beside the fire, but its tip among the embers, the fire takes it too; lying alongside, it does not.
    let beside = at(fire + Vec2::new(0.75, 0.0), 0.03);
    assert!(goes_back(&lying(beside, std::f32::consts::FRAC_PI_2)));
    assert!(!goes_back(&lying(beside, 0.0)));
    // Every stick begins on its log.
    for index in 0..STICKS {
        assert!(by_its_log(home(index).position));
    }
}

#[test]
fn what_the_campsite_shares_reads_back_apart_from_the_flock_s_and_the_beach_s() {
    // An event says the sticks' bodies and how toasted those that changed are, and nothing of either it need not.
    let event = Event {
        clock: 5,
        sticks: Update::default(),
        toast: vec![Toasted {
            body: body(1),
            toast: 0.5,
        }],
    };
    let text = serde_json::to_string(&event).unwrap();
    assert_eq!(
        text,
        r#"{"clock":5,"sticks":{},"toast":[{"body":101,"toast":0.5}]}"#
    );
    assert_eq!(serde_json::from_str::<Event>(&text).unwrap(), event);
    let quiet = Event {
        toast: Vec::new(),
        ..event.clone()
    };
    assert_eq!(
        serde_json::to_string(&quiet).unwrap(),
        r#"{"clock":5,"sticks":{}}"#
    );
    // Never one of the flock's events, nor the beach's, nor they one of its.
    assert!(serde_json::from_str::<crate::beach::Event>(&text).is_err());
    let flock = r#"{"clock":5,"chicken":1,"behaviour":{"began":0}}"#;
    assert!(serde_json::from_str::<Event>(flock).is_err());
    let beach = serde_json::to_string(&crate::beach::Event {
        clock: 5,
        stones: Update::default(),
    })
    .unwrap();
    assert!(serde_json::from_str::<Event>(&beach).is_err());
    // The campsite is read from the farmyard's state beside the flock's, and none from a state from before it.
    let state = serde_json::json!({
        "clock": 1,
        "chickens": [],
        "campsite": {"sticks": {"bodies": [{"body": 100, "epoch": 3, "at": [0, 0, 0, 0, 0, 0, 10000]}]}, "toast": [0.5]},
    });
    let read = shared(&state);
    assert_eq!(read.sticks.bodies[0].epoch, 3);
    assert_eq!(read.toast, [0.5]);
    assert_eq!(
        shared(&serde_json::json!({"clock": 1, "chickens": []})),
        Shared::default()
    );
    // A state from when the sticks were the campsite's own is none of them.
    let before = serde_json::json!({"clock": 1, "sticks": [{"epoch": 3, "toast": 0.5, "held": 7}]});
    assert_eq!(shared(&before), Shared::default());
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
