// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::backdrop::RIM;
use crate::layout::{self, BACKDROP_CENTRE, POOL};
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
    layout::Footprint::new([solid.low[0], solid.low[2]], [solid.high[0], solid.high[2]])
        .distance(point)
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
    // that from there the backdrop stands within its 50 m bound, as the backdrop's own tests check.
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
        // Out of every plant's crown, round about its trunk, as far as it reaches; but for a tree fern's, beneath it, its
        // crown high overhead and its trunk well clear, as the fireside log lies beneath a ponga's.
        let mut bush = crate::ferns::bush().into_iter();
        for plant in backdrop::plants() {
            let (reach, _) = plant.size();
            let fern = (plant.kind == backdrop::Kind::Fern).then(|| bush.next().unwrap().solids());
            let beneath = fern.is_some_and(|fern| {
                let trunk = Vec2::new(fern[0].low[0], fern[0].low[2]);
                let trunk = (trunk + Vec2::new(fern[0].high[0], fern[0].high[2])) * 0.5;
                apart(&solid, trunk) > 1.0 && solid.high[1] + 1.8 < fern[1].low[1]
            });
            assert!(
                apart(&solid, ground(plant.at)) > reach || beneath,
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
    assert!((1.5..1.8).contains(&ground(middle).distance(fire)));
    assert!((0.35..0.5).contains(&seat_top(radius)));
    assert!((1.0..1.6).contains(&length));
    assert!(along.dot((fire - ground(middle)).normalize()).abs() < 0.2);
}

#[test]
fn the_fireside_log_lies_between_the_fire_and_the_pool_facing_the_fire_and_seats_whoever_points_at_it()
 {
    let fire = ground(FIRE);
    let pool = Vec2::new(POOL.x, POOL.z);
    let (middle, along, length, radius) = fireside();
    // Between the fire and the pool, a step beyond the ring participants walk round the fire, lying across the way to
    // the pool, so that one sitting on it faces the fire; low enough to sit on.
    assert!((1.5..1.8).contains(&ground(middle).distance(fire)));
    assert!(ground(middle).distance(pool) < fire.distance(pool) - 1.4);
    assert!(along.dot((pool - fire).normalize()).abs() < 1e-4);
    assert!((0.35..0.5).contains(&seat_top(radius)));
    assert!((1.0..1.6).contains(&length));
    // On the ground beneath its chocks, which rises towards the pool: none higher than it lies, nor much lower.
    for end in [-0.32, 0.32] {
        let at = ground(middle) + along * length * end;
        let beneath = backdrop::drawn_ground(at)
            .unwrap_or(backdrop::MEADOW)
            .max(trail::CLEARING_HEIGHT);
        assert!(
            beneath <= middle.y + 1e-5 && beneath > middle.y - 0.04,
            "{at}"
        );
    }
    // Solid, to the agent and to where participants stand, and to the sticks, which rest on it as on the other seats.
    let solid = Solid::new(furniture()[2].0, furniture()[2].1);
    assert!(solid.contains((middle + Vec3::Y * seat_top(radius) * 0.5).to_array()));
    assert_eq!(colliders().len(), 1 + 3 + PIT_STONES as usize + 2);
    // Whoever points at it arrives sitting on it: on the ground beneath its seat, a little before its middle, facing the
    // fire, from anywhere within reach.
    let (arrival, facing) = seated();
    assert!(arrival.distance(middle) < radius);
    assert!((arrival - middle).dot(FIRE - middle) > 0.0);
    let towards = (FIRE - arrival) * Vec3::new(1.0, 0.0, 1.0);
    assert!(facing.normalize().dot(towards.normalize()) > 0.999);
    let [target] = targets();
    assert_eq!(
        (target.name, target.foot, target.arrival, target.within),
        ("Fireside log", middle, arrival, None)
    );
    assert!(target.radius < length * 0.5 && target.height > seat_top(radius));
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
        let beneath = backdrop::drawn_ground(ground(pose.butt)).unwrap();
        assert!((pose.butt.y - beneath - STICK_RADIUS).abs() < 1e-4);
        let to_fire = FIRE - middle;
        let towards = (to_fire - axis * to_fire.dot(axis)).normalize();
        assert!((pose.butt - middle).dot(towards) < -0.3);
        assert!((pose.marshmallow() - middle).dot(towards) > 0.05);
        assert!(pose.marshmallow().y > seat_top(radius));
        // Touching the log, rather than through it or above it: its shaft as far from the log's axis as the flats of
        // the log's sides and of the stick's hull, at its girth where it crosses it, and no further than their corners;
        // crossing it well beyond the stick's middle of mass, so that it does not tip over it.
        let shaft = pose.along;
        let normal = shaft.cross(axis).normalize();
        let distance = (axis_point - pose.butt).dot(normal).abs();
        let offset = pose.butt - axis_point;
        let (b, d, e) = (shaft.dot(axis), shaft.dot(offset), axis.dot(offset));
        let crossing = (b * e - d) / (1.0 - b * b);
        assert!(
            crossing > 0.7 && crossing < STICK_LENGTH * 0.8,
            "{crossing}"
        );
        let flats = radius * (PI / 8.0).cos() + girth(crossing) * (PI / 6.0).cos();
        assert!(
            distance > flats - 0.002 && distance < radius + girth(crossing),
            "{index} {distance}"
        );
        // Its marshmallow beyond the log and clear of it, from its near end to its far one.
        for end in [-0.5, 0.5] {
            let at = pose.marshmallow() + pose.along * MARSHMALLOW.0 * end;
            let from_log = (at - axis_point).reject_from(axis).length();
            assert!(
                from_log > radius + MARSHMALLOW.1 + 0.02,
                "{index} {from_log}"
            );
        }
        // Out of the fire's heat, so that a marshmallow on its log never toasts.
        assert_eq!(heat(pose.marshmallow()), 0.0);
        // Apart from the others, by more than twice their width.
        for other in 0..index {
            assert!(
                pose.marshmallow()
                    .distance(Pose::racked(other).marshmallow())
                    > MARSHMALLOW.1 * 4.0
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
fn a_marshmallow_is_big_on_the_end_of_its_stick_and_toasts_from_its_middle() {
    // Twice what it was, 9 cm long and 7.6 cm across, on the end of the stick, its tip showing through it.
    let (length, radius) = MARSHMALLOW;
    assert!((length - 0.09).abs() < 1e-6 && (radius - 0.038).abs() < 1e-6);
    let beyond = MARSHMALLOW_INSET - length * 0.5;
    assert!(beyond > 0.01 && beyond < 0.025, "{beyond}");
    assert!(girth(0.0) == STICK_RADIUS && girth(STICK_LENGTH) == radius);
    // Toasted from the middle of its marshmallow, wherever the stick is held, by how near that is to the heart of the
    // fire, as far from the hand as it ever was along the stick, less the marshmallow's own middle's step back.
    let turn = Quat::from_rotation_y(-0.4) * Quat::from_rotation_x(-0.3);
    let along = turn * Vec3::NEG_Z;
    let hand = heart() - along * (STICK_LENGTH - MARSHMALLOW_INSET - HOLD);
    let held = Pose::held(hand, turn);
    let middle = held.marshmallow();
    assert!(middle.distance(heart()) < 1e-4, "{middle}");
    assert!(middle.distance(held.tip()) - MARSHMALLOW_INSET < 1e-4);
    assert_eq!(toasted(0.0, middle, 1.0), TOAST_RATE);
    // Its middle drawn where the stick's model draws it.
    let model = stick();
    let node = model
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some(shade_node(0).as_str()))
        .unwrap();
    let corners = &model.meshes[node.mesh.unwrap()].primitives[0].positions;
    let (low, high) = corners.iter().fold(
        (Vec3::INFINITY, Vec3::NEG_INFINITY),
        |(low, high), corner| (low.min(*corner), high.max(*corner)),
    );
    let drawn = (low + high) * 0.5;
    assert!(drawn.distance(Vec3::NEG_Z * (STICK_LENGTH - MARSHMALLOW_INSET)) < 1e-4);
    assert!((high.z - low.z - length).abs() < 1e-4);
    // Heat measured from its middle, so that it toasts as far from the flames as a marshmallow ever did: its middle a
    // little within 0.6 m of the heart, it toasts, and a little beyond, not, though its near side be within.
    let held_at = |middle: Vec3| {
        Pose::held(
            middle - along * (STICK_LENGTH - MARSHMALLOW_INSET - HOLD),
            turn,
        )
    };
    for (off, toasts) in [(0.58, true), (0.62, false)] {
        let middle = heart() + along.cross(Vec3::Y).normalize() * off;
        let at = held_at(middle).marshmallow();
        assert!(at.distance(middle) < 1e-4);
        assert_eq!(toasted(0.0, at, 1.0) > 0.0, toasts, "{off}");
    }
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
    assert_eq!(bodies.len(), STICKS + LOGS);
    const { assert!(DENSITY < 1_000.0) };
    for (index, body) in bodies[..STICKS].iter().enumerate() {
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
    // The ground about the fire as it is drawn, with its clearing, the three log seats, the fire's ring of stones, its
    // bonfire and the woodpile's stone.
    assert_eq!(colliders.len(), 1 + 3 + PIT_STONES as usize + 2);
    // The pool's water, below the campsite, within its reach, which takes back what rests in it.
    let pool = &waters()[0];
    assert_eq!(pool.returns_after, Some(RETURNS));
    assert!(in_pool(&Placement::at(
        layout::POOL + Vec3::new(1.0, -0.1, 0.5)
    )));
    assert!(!in_pool(&Placement::at(heart())));
    // Floating with its middle a little above the water, its big marshmallow in it, it is in the pool; but not lying
    // beside it, nor held above it.
    let tipped = Quat::from_rotation_x(-0.1);
    let afloat = Placement::new(layout::POOL + Vec3::new(0.3, 0.02, 0.5), tipped);
    assert!(in_pool(&afloat));
    assert!(!in_pool(&Placement::new(
        layout::POOL + Vec3::new(0.3, 0.2, 0.5),
        tipped
    )));
    let beside = layout::POOL + Vec3::X * (layout::POOL_RADIUS + 0.5);
    assert!(!in_pool(&Placement::at(beside + Vec3::NEG_Y * 0.2)));
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
fn a_burnt_marshmallow_burns_on_out_of_the_heat_for_a_while_and_goes_out_or_under_the_water() {
    let away = heart() + Vec3::X * 2.0;
    // Caught in the heat, it may burn on for as long as it burns.
    assert_eq!(burns(0.0, BLACK, heart(), 0.1), BURNS);
    assert_eq!(burns(3.0, 1.0, heart(), 0.1), BURNS);
    // Out of it, less and less, and then not at all.
    assert!((burns(BURNS, BLACK, away, 1.5) - (BURNS - 1.5)).abs() < 1e-5);
    assert_eq!(burns(1.0, BLACK, away, 2.0), 0.0);
    // Never unlit beyond the heat, nor unless it is black, nor under the pool's water.
    assert_eq!(burns(0.0, BLACK, away, 0.1), 0.0);
    assert_eq!(burns(BURNS, BROWN, heart(), 0.1), 0.0);
    assert_eq!(burns(BURNS, f32::NAN, heart(), 0.1), 0.0);
    let under = layout::POOL - Vec3::Y * 0.05;
    assert!(doused(under));
    assert!(!doused(layout::POOL + Vec3::Y * 0.05));
    assert!(!doused(away));
    assert_eq!(burns(BURNS, BLACK, under, 0.1), 0.0);
    // Nonsense takes nothing off it, or puts it out.
    assert_eq!(burns(10.0, BLACK, away, f32::NAN), 10.0);
    assert_eq!(burns(10.0, BLACK, away, -1.0), 10.0);
    assert_eq!(burns(f32::INFINITY, BLACK, away, 0.1), 0.0);
    // Beyond a small fire's heat, it catches in a big one's, which reaches further.
    let further = heart_at(1.0) + Vec3::X * 0.9;
    assert_eq!(burns(0.0, BLACK, further, 0.1), 0.0);
    assert_eq!(burns_at(0.0, BLACK, further, 0.1, 1.0), BURNS);
    assert_eq!(burns(BURNS * 2.0, BLACK, away, 0.0), BURNS);
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
    // An event says the sticks' bodies and how toasted those that changed are, and nothing of either it need not, nor
    // of the fire's size or the logs' burning, which say nothing unchanged.
    let event = Event {
        clock: 5,
        sticks: Update::default(),
        toast: vec![Toasted {
            body: body(1),
            toast: 0.5,
        }],
        size: None,
        burnt: Vec::new(),
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

#[test]
fn a_bigger_fire_toasts_from_further_off_and_by_default_toasts_as_it_always_did() {
    // By default, toasting is as it was: the heart where it was, hottest a hand's breadth about it, nothing beyond 0.6 m.
    assert_eq!(heart_at(0.0), heart());
    assert_eq!(flames(0.0), SMALL);
    const { assert!(SMALL < 1.0 && BIG > 1.0) };
    for offset in [0.0, 0.3, 0.5, 0.59, 0.7] {
        let at = heart() + Vec3::X * offset;
        assert_eq!(heat_at(at, 0.0), heat(at));
        assert_eq!(toasted_at(0.1, at, 3.0, 0.0), toasted(0.1, at, 3.0));
    }
    // At its biggest, its flames twice as big, its heart higher and its heat reaching twice as far.
    assert_eq!(flames(1.0), BIG);
    assert_eq!(reach(1.0), 2.0);
    assert!(heart_at(1.0).y > heart().y + 0.4);
    assert!(heat_at(heart_at(1.0) + Vec3::X * 0.3, 1.0) > 0.999);
    let far = heart_at(1.0) + Vec3::X * 1.0;
    assert!(heat_at(far, 1.0) > 0.0 && heat(far) == 0.0);
    assert_eq!(heat_at(heart_at(1.0) + Vec3::X * 1.21, 1.0), 0.0);
    assert!(burning_at(BLACK, far, 1.0) && !burning(BLACK, far));
    // In between, between them; and nonsense is its default.
    assert!(heat_at(far, 0.5) < heat_at(far, 1.0));
    assert_eq!(flames(f32::NAN), SMALL);
    assert_eq!(flames(3.0), BIG);
}

#[test]
fn a_log_in_the_fire_burns_away_feeding_it_and_the_fire_dies_back_without_more() {
    // A log burns away over BURN seconds, catching over the first CATCH of them, which feed the fire.
    assert_eq!(burnt(0.0, BURN), 1.0);
    assert!((burnt(0.25, BURN * 0.5) - 0.75).abs() < 1e-6);
    assert_eq!(burnt(0.5, f32::NAN), 0.5);
    assert!(!catching(0.0));
    assert!(catching(1e-3) && catching((CATCH - 0.1) / BURN));
    assert!(!catching((CATCH + 0.1) / BURN));
    // One log catching grows it by its fuel, less what it dies back meanwhile; three, past its biggest, to its biggest.
    let one = grown(0.0, 1, CATCH);
    assert!((one - (FUEL - CATCH / DIES_BACK)).abs() < 1e-5);
    assert_eq!(grown(0.0, 3, CATCH), 1.0);
    // Without more wood, it dies back from its biggest to its default over DIES_BACK, and no further.
    assert!((grown(1.0, 0, DIES_BACK * 0.5) - 0.5).abs() < 1e-5);
    assert_eq!(grown(1.0, 0, DIES_BACK), 0.0);
    assert_eq!(grown(0.0, 0, 60.0), 0.0);
    assert_eq!(grown(f32::NAN, 0, 1.0), 0.0);
    assert_eq!(grown(0.5, 0, f32::NAN), 0.5);
    assert_eq!(
        (unit(0.4), unit(2.0), unit(-1.0), unit(f32::NAN)),
        (0.4, 1.0, 0.0, 0.0)
    );
    // As it burns it chars, glows and shrinks.
    assert_eq!((log_shade(0.0), log_shade(0.05), log_shade(0.5)), (0, 1, 2));
    assert_eq!(charred(0.0), 1.0);
    assert!((charred(1.0) - 0.3).abs() < 1e-6);
    assert_eq!(charred(f32::NAN), 1.0);
    // What the agent is told it looks like.
    assert_eq!(blaze(0.0), "burning low");
    assert_eq!(blaze(0.3), "burning well");
    assert_eq!(blaze(0.5), "burning high");
    assert_eq!(blaze(1.0), "blazing");
}

#[test]
fn the_woodpile_s_logs_are_light_bodies_crossed_on_a_flat_stone_beside_the_fire() {
    // Each log a six-sided body of dry wood, a hand's span across and held in one hand, its faces of four corners
    // flat beneath it and on top.
    let hull = log_hull();
    assert_eq!(hull.len(), LOG_SIDES as usize * 2);
    let flat = hull
        .iter()
        .filter(|point| (point.y - log_depth() * 0.5).abs() < 1e-6)
        .count();
    assert_eq!(flat, 4);
    let mass = DENSITY * 3.0 * 3f32.sqrt() / 2.0 * LOG_RADIUS * LOG_RADIUS * LOG_LENGTH;
    assert!((1.0..3.0).contains(&mass), "{mass}");
    let bodies = bodies();
    for (index, body) in bodies[STICKS..].iter().enumerate() {
        assert_eq!(body.id, log_body(index));
        assert_eq!(log_index(body.id), Some(index));
        assert_eq!(super::index(body.id), None);
        assert_eq!(body.check(), Ok(()));
        assert_eq!(body.home, log_home(index));
    }
    assert_eq!(log_index(FIRST_LOG - 1), None);
    assert_eq!(log_index(log_body(LOGS)), None);
    assert!(FIRST_LOG > super::body(STICKS));
    // Stacked in crossed pairs, each pair on the one beneath, on the stone's flat top.
    let (least, greatest) = pile_stone();
    for index in 0..LOGS {
        let home = log_home(index);
        let layer = (index / 2) as f32;
        let bottom = home.position.y - log_depth() * 0.5;
        assert!(
            (bottom - (greatest.y + log_depth() * layer)).abs() < 0.002,
            "{index}"
        );
        let along = home.rotation * Vec3::X;
        let across = if (index / 2) % 2 == 0 {
            Vec3::Z
        } else {
            Vec3::X
        };
        assert!(along.dot(across).abs() > 0.999, "{index}");
        // Over the stone.
        assert!(home.position.x > least.x && home.position.x < greatest.x);
        assert!(home.position.z > least.z && home.position.z < greatest.z);
    }
    // Its stone on the ground, its top above all of it, beside the fire, beyond the ring participants walk round it,
    // apart from the seats and clear of where a log goes back or burns.
    let pile = woodpile();
    let fire = ground(FIRE);
    let middle = ground(pile);
    assert!((1.5..2.0).contains(&middle.distance(fire)));
    let inner = [least, greatest]
        .iter()
        .flat_map(|a| [least, greatest].map(|b| Vec2::new(a.x, b.z)))
        .map(|corner| corner.distance(fire))
        .fold(f32::INFINITY, f32::min);
    assert!(inner > 1.25, "{inner}");
    for at in [Pose::racked(0).butt, fireside().0] {
        assert!(ground(at).distance(middle) > 1.0);
    }
    assert!(!in_fire(&log_home(0)));
    assert!(!log_goes_back(&log_home(0)));
    // A solid, to the agent and to where participants stand, as tall as its stack.
    let solid = Solid::new(furniture()[1].0, furniture()[1].1);
    assert!(solid.contains((log_home(LOGS - 1).position).to_array()));
}

#[test]
fn a_log_lying_in_the_fire_burns_one_carried_off_goes_back_and_one_by_the_fire_stays() {
    let lying = |at: Vec3, yaw: f32| Placement::new(at, Quat::from_rotation_y(yaw));
    // On the embers, or across the stones with its end among them, the fire takes it.
    assert!(in_fire(&lying(FIRE + Vec3::Y * 0.1, 0.0)));
    assert!(in_fire(&lying(FIRE + Vec3::new(0.55, 0.12, 0.0), 0.0)));
    // Alongside the stones, held above the flames, or nowhere, it does not.
    assert!(!in_fire(&lying(
        FIRE + Vec3::new(0.55, 0.12, 0.0),
        std::f32::consts::FRAC_PI_2
    )));
    assert!(!in_fire(&lying(FIRE + Vec3::Y * 1.0, 0.0)));
    assert!(!in_fire(&lying(Vec3::NAN, 0.0)));
    // About the fire it stays; carried off beyond its reach, or nowhere, it goes back on the woodpile.
    assert!(!log_goes_back(&lying(
        FIRE + Vec3::new(1.2, 0.05, 0.5),
        0.3
    )));
    assert!(log_goes_back(&lying(
        FIRE + Vec3::new(DROP_REACH + 0.5, 0.0, 0.0),
        0.3
    )));
    assert!(log_goes_back(&lying(Vec3::NAN, 0.0)));
}

#[test]
fn a_log_is_drawn_as_it_collides_in_each_of_its_shades() {
    let model = log_model();
    let hull = log_hull();
    for shade in 0..LOG_SHADES {
        let node = model
            .nodes
            .iter()
            .find(|node| node.name.as_deref() == Some(log_node(shade).as_str()))
            .unwrap();
        for corner in &model.meshes[node.mesh.unwrap()].primitives[0].positions {
            assert!(
                hull.iter().any(|point| point.distance(*corner) < 1e-5),
                "{corner}"
            );
        }
    }
    // Glowing, it is drawn with the embers' material.
    let glowing = &model.nodes[2];
    let material = model.meshes[glowing.mesh.unwrap()].primitives[0].material;
    assert!(model.materials[material.unwrap()].emissive.length() > 0.5);
}
