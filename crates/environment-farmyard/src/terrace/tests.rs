// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::walkable;

/// The walkable ground's triangles on the terrace, at its height.
fn floor() -> Vec<[Vec3; 3]> {
    walkable::navmesh()
        .triangles()
        .iter()
        .filter(|triangle| triangle.iter().all(|corner| corner.y > TERRACE - 0.1))
        .copied()
        .collect()
}

/// Every 10 cm of the terrace's walkable ground, on the ground.
fn walked() -> Vec<Vec2> {
    let outline = outline();
    let (low, high) = outline.iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(low, high), corner| (low.min(*corner), high.max(*corner)),
    );
    let mut points = Vec::new();
    let steps = ((high - low) / 0.1).ceil();
    for x in 0..=steps.x as i32 {
        for z in 0..=steps.y as i32 {
            let point = low + Vec2::new(x as f32, z as f32) * 0.1;
            if walkable::height_at(point).is_some_and(|height| height > TERRACE - 0.1) {
                points.push(point);
            }
        }
    }
    points
}

#[test]
fn the_terrace_is_a_floor_of_its_own_at_its_height_on_the_mountain_at_the_top_of_the_falls() {
    // Drawn as ground at its height, 7.2 m above the pool, every face of it level, within its outline.
    let ground = ground();
    assert_eq!(ground.len(), OUTLINE.len() * 2);
    for triangle in &ground {
        let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
        assert!(normal.y > 0.0 && normal.x == 0.0 && normal.z == 0.0);
        for corner in triangle {
            assert_eq!(corner.y, TERRACE);
        }
        let middle = triangle.iter().sum::<Vec3>() / 3.0;
        assert!(contains(Vec2::new(middle.x, middle.z)));
    }
    // Its faces cover its outline once, and nothing beyond it.
    let outline = outline();
    let enclosed = (0..outline.len())
        .map(|index| outline[index].perp_dot(outline[(index + 1) % outline.len()]))
        .sum::<f32>()
        .abs()
        * 0.5;
    let drawn: f32 = ground
        .iter()
        .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
        .sum();
    assert!((drawn - enclosed).abs() < 1e-3, "{drawn} {enclosed}");
    assert!((TERRACE - layout::POOL.y - 7.2).abs() < 1e-5);
    const { assert!(TERRACE - layout::UPPER_POOL.y > 0.3) };
    // Its walkable ground is all at its height, and about 30 square metres, 7 m across and 4 m and more deep.
    let floor = floor();
    let area: f32 = floor
        .iter()
        .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
        .sum();
    println!("The terrace's walkable ground is {area:.1} m².");
    assert!((24.0..36.0).contains(&area), "{area}");
    for corner in floor.iter().flatten() {
        assert!((corner.y - TERRACE).abs() < 1e-5, "{corner}");
    }
    let spans = walked().iter().map(|point| on_the_fall(*point)).fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(low, high), at| (low.min(at), high.max(at)),
    );
    let span = spans.1 - spans.0;
    assert!(span.x > 4.0 && span.y > 7.0, "{span}");
    // Beneath its floor, the mountain's ground as it is drawn lies below it, by a centimetre and a half and more, as far
    // as its sides reach down, so that the floor never flickers against the ground and no gap shows beneath its edge.
    let mut sampled = 0;
    for x in -80..80 {
        for z in -400..-200 {
            let point = Vec2::new(x as f32 * 0.1, z as f32 * 0.1 + 0.05);
            if !contains(point) {
                continue;
            }
            sampled += 1;
            let drawn = backdrop::drawn_ground(point).unwrap();
            assert!(
                (TERRACE - SIDES..=TERRACE - 0.015).contains(&drawn),
                "{point} {drawn}"
            );
        }
    }
    assert!(sampled > 2000, "{sampled}");
}

/// Where `point` is along the waterfall's way and across it.
fn on_the_fall(point: Vec2) -> Vec2 {
    let (along, across) = backdrop::on_the_fall(point);
    Vec2::new(along, across)
}

#[test]
fn the_terrace_joins_no_other_ground_and_its_edge_stops_short_of_the_lip_and_the_water() {
    // Nowhere else participants walk is within 10 m of it, across the ground, so no step, path or backstep joins it.
    let terrace = walked();
    let others: Vec<Vec3> = walkable::navmesh()
        .triangles()
        .iter()
        .flatten()
        .filter(|corner| corner.y < TERRACE - 1.0)
        .copied()
        .collect();
    let nearest = terrace
        .iter()
        .flat_map(|point| {
            others
                .iter()
                .map(move |other| point.distance(Vec2::new(other.x, other.z)))
        })
        .fold(f32::INFINITY, f32::min);
    println!("The nearest other ground is {nearest:.1} m from the terrace.");
    assert!(nearest > 10.0, "{nearest}");
    // From every 10 cm of it, on its floor, the mountain's ground within 0.6 m falls no more than a metre below it:
    // nobody stands at the edge of the cliff, nor beside a drop.
    for point in &terrace {
        assert!(contains(*point), "{point}");
        for step in 0..16 {
            let yaw = step as f32 / 16.0 * TAU;
            let way = Vec2::new(yaw.sin(), yaw.cos());
            for reach in [0.3, 0.6] {
                let beyond = *point + way * reach;
                let drawn = backdrop::drawn_ground(beyond).unwrap();
                assert!(drawn > TERRACE - 1.0, "{point} {beyond} {drawn}");
            }
        }
        // Behind the cliff's lip by 0.6 m and more, and clear of the stream's and the upper pool's water.
        let (along, across) = backdrop::on_the_fall(*point);
        assert!(along > layout::CLIFF[1] + 0.6, "{point}");
        assert!(across > 1.6, "{point}");
        let pool = Vec2::new(layout::UPPER_POOL.x, layout::UPPER_POOL.z);
        assert!(
            point.distance(pool) > layout::UPPER_POOL_RADIUS + 0.7,
            "{point}"
        );
    }
    // A participant there stands short of the upper cliff, whose foot is behind the terrace's back.
    for point in &terrace {
        assert!(point.distance(layout::BACKDROP_CENTRE) < layout::UPPER_CLIFF[0] - 0.3);
    }
}

#[test]
fn the_terrace_stands_within_the_space_s_bounds_from_where_participants_arrive() {
    // The space's bounds, 30 m from home on each axis, as `tomori_scene::anchors::REACH` and the server keep them.
    const BOUNDS: f32 = 30.0;
    let arrival = tomori_protocol::Pose::at(layout::ARRIVAL.to_array())
        .facing_towards(layout::ARRIVAL_FACING.to_array());
    let carriage = tomori_protocol::Pose::carriage(&arrival);
    let carried = |point: Vec3| {
        carriage
            .carry(&tomori_protocol::Pose::at(point.to_array()))
            .position
    };
    let mut furthest: f32 = 0.0;
    for point in walked() {
        let at = carried(Vec3::new(point.x, TERRACE, point.y));
        furthest = furthest.max(at[0].abs()).max(at[2].abs());
    }
    println!("The terrace reaches {furthest:.2} m from home.");
    assert!(furthest < BOUNDS - 1.0, "{furthest}");
    let place = place();
    let targets = targets();
    for point in [
        place.at,
        place.towards,
        targets[0].foot,
        targets[0].arrival,
        targets[1].foot,
    ] {
        let at = carried(point);
        assert!(
            at.iter().all(|axis| axis.abs() < BOUNDS - 1.0),
            "{point} {at:?}"
        );
    }
    // Before the terrace, the waterfall's lip was beyond the bounds the space had, 20 m.
    let lip = carried(layout::WATERFALL_LIP);
    assert!(lip[2].abs() > 20.0);
}

#[test]
fn its_stones_ferns_flat_rock_and_cairn_stand_on_its_floor_and_are_kept_clear_of() {
    let floor = |point: Vec2| contains(point) || from_floor(point) < 0.6;
    for stone in stones() {
        let at = Vec2::new(stone.at.x, stone.at.z);
        assert!(floor(at), "{stone:?}");
        assert!((stone.at.y - (TERRACE - 0.03)).abs() < 1e-6);
        assert!(stone.size.y < stone.size.x * 1.15);
    }
    // Apart from each other.
    let stones = stones();
    for (index, stone) in stones.iter().enumerate() {
        for other in &stones[index + 1..] {
            let apart =
                Vec2::new(stone.at.x, stone.at.z).distance(Vec2::new(other.at.x, other.at.z));
            assert!(
                apart > (stone.size.x + other.size.x) * 0.5,
                "{stone:?} {other:?}"
            );
        }
    }
    // Its ferns at its back and front corners and by the stream, and across the stream over the waterfall's lip, on
    // the mountain's ground as drawn there, the trees' crowns over the floor 2.2 m and more above it.
    let ferns = ferns();
    assert_eq!(ferns.iter().filter(|fern| fern.kind.is_tree()).count(), 2);
    for fern in &ferns {
        let at = Vec2::new(fern.at.x, fern.at.z);
        let (along, across) = backdrop::on_the_fall(at);
        let over_the_lip = across < 0.0 && along < layout::CLIFF[1] + 3.0;
        assert!(floor(at) || over_the_lip, "{fern:?}");
        if !contains(at) {
            let drawn = backdrop::drawn_ground(at).unwrap();
            assert!((fern.at.y - drawn).abs() < 0.1, "{fern:?} {drawn}");
        }
        if fern.kind.is_tree() {
            let crown = fern.solids()[1];
            assert!(crown.low[1] > TERRACE + 2.2 - 1e-3, "{fern:?}");
        }
    }
    // Nobody lands within the clearance of any of them: of every stone, the flat rock, the cairn, or a fern's trunk or
    // clump, which the walkable ground is cut about.
    let walked = walked();
    for (low, high) in solids() {
        if low[1] > TERRACE + tomori_environment::navmesh::HEADROOM {
            continue;
        }
        let (low, high) = (Vec2::new(low[0], low[2]), Vec2::new(high[0], high[2]));
        for point in &walked {
            let apart = (low - *point).max(*point - high).max(Vec2::ZERO).length();
            assert!(apart > layout::CLEARANCE - 0.11, "{point} {low} {high}");
        }
    }
    // The flat rock is low enough to sit on, and the cairn as tall as it is pointed at.
    assert!((0.35..0.5).contains(&SEAT_SIZE.y));
    let [_, down] = targets();
    assert!((down.foot.y + down.height - cairn_top() - 0.05).abs() < 1e-5);
    assert!(cairn_top() - TERRACE > 0.5);
    // Drawn as one model, its floor, sides, flat rock and cairn, in 148 triangles, none of them facing down, so that
    // the flat rock's and the cairn's tops are lit from above.
    let model = model();
    let primitive = &model.meshes[0].primitives[0];
    let triangles: usize = primitive.indices.len() / 3;
    assert_eq!(triangles, 148, "the count the documentation gives");
    assert!(primitive.normals.iter().all(|normal| normal.y > -0.01));
}

#[test]
fn the_waterfall_takes_one_up_from_its_rock_and_the_cairn_back_down_onto_it() {
    let [up, down] = targets();
    let rock = layout::WATERFALL_ROCK;
    let flat = |at: Vec3| Vec2::new(at.x, at.z);
    // The way up: the falling water, about the lower part of its sheet before the cliff's face, as tall as a target may
    // be, from the pool; the sheet is within it there, either side of its middle.
    assert_eq!(up.name, "Upper terrace");
    assert_eq!(up.foot.y, layout::POOL.y);
    assert_eq!(up.height, FALL_HEIGHT);
    let sheet: Vec<Vec3> = backdrop::water()
        .into_iter()
        .filter(|point| point.kind == backdrop::FALLING && point.position.y < up.foot.y + up.height)
        .map(|point| point.position)
        .collect();
    assert!(sheet.len() > 20);
    let within = sheet
        .iter()
        .filter(|at| flat(**at).distance(flat(up.foot)) < up.radius)
        .count();
    assert!(within * 2 > sheet.len(), "{within} of {}", sheet.len());
    // Pointed at from the rock, wherever one stands on its top, but from nowhere else participants stand: the lookout,
    // the campsite, the beach and the trail are all beyond its reach; nor from the terrace above.
    let reach = up.within.unwrap();
    for step in 0..12 {
        let yaw = step as f32 / 12.0 * TAU;
        let standing = rock + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * layout::ROCK_TOP;
        for eyes in [1.1, tomori_environment::sightlines::EYES, 1.8] {
            assert!((standing + Vec3::Y * eyes).distance(up.foot) < reach - 0.2);
        }
    }
    for corner in walkable::navmesh().triangles().iter().flatten() {
        for eyes in [0.9, 1.9] {
            assert!(
                (*corner + Vec3::Y * eyes).distance(up.foot) > reach + 1.0,
                "{corner}"
            );
        }
    }
    // It takes one onto the terrace, on its walkable ground at its height, facing the upper falls.
    let arrival = up.arrival;
    assert_eq!(arrival.y, TERRACE);
    let floor = walkable::height_at(flat(arrival)).unwrap();
    assert!((floor - TERRACE).abs() < 1e-5);
    let level = |at: Vec3| Vec3::new(at.x, 0.0, at.z).normalize();
    assert!(
        up.facing.y == 0.0 && level(up.facing).dot(level(layout::UPPER_FALLS - arrival)) > 0.999
    );
    // The way down: the cairn by the lip, pointed at about all of it, from the terrace alone, taking one onto the rock's
    // top, facing the waterfall, as pointing at the rock does.
    assert_eq!(down.name, "Waterfall rock");
    assert_eq!(down.arrival, rock);
    assert!(down.radius > CAIRN_REACH);
    let (along, across) = CAIRN;
    assert!(flat(down.foot).distance(on_fall(along, across)) < 1e-5);
    assert!(level(down.facing).dot(level(layout::WATERFALL - rock)) > 0.999);
    let reach = down.within.unwrap();
    let terrace = walked();
    let pointed = terrace
        .iter()
        .filter(|point| Vec3::new(point.x, TERRACE + 1.3, point.y).distance(down.foot) <= reach)
        .count();
    assert!(
        pointed * 2 > terrace.len(),
        "{pointed} of {}",
        terrace.len()
    );
    for corner in walkable::navmesh().triangles().iter().flatten() {
        if corner.y < TERRACE - 1.0 {
            assert!((*corner + Vec3::Y * 1.9).distance(down.foot) > reach + 4.0);
        }
    }
    let top = layout::WATERFALL_ROCK + Vec3::Y * 1.9;
    assert!(top.distance(down.foot) > reach + 2.0);
    // The cairn stands by the lip, beside the stream, at the front of the terrace.
    let (along, across) = backdrop::on_the_fall(flat(down.foot));
    assert!(
        along < layout::CLIFF[1] + 1.5 && across < 3.0,
        "{along} {across}"
    );
}

#[test]
fn its_anchor_stands_behind_whoever_arrives_facing_them() {
    let place = place();
    assert_eq!(
        (place.name, place.kind, place.label),
        ("upper-terrace", "viewpoint", "The upper terrace")
    );
    let arrival = arrival();
    let flat = |at: Vec3| Vec2::new(at.x, at.z);
    let away = flat(place.at) - flat(arrival);
    assert!((away.length() - 1.6).abs() < 1e-4);
    let falls = (flat(layout::UPPER_FALLS) - flat(arrival)).normalize();
    assert!(away.normalize().dot(falls) < -0.99);
    assert!((place.at.y - TERRACE - 1.8).abs() < 1e-5);
    assert_eq!(place.towards, arrival + Vec3::Y * 1.8);
    assert_eq!(place.notify_within, None);
    // Over the terrace's floor.
    assert!(contains(flat(place.at)));
    assert_eq!(from_floor(flat(place.at)), 0.0);
    // Among the trail's, after the waterfall rock's.
    let places = crate::trail::places();
    assert_eq!(places[3], place);
    // One to eighteen workers fit its room there, clear of the terrace's solids, seen from the floor.
    use tomori_environment::sightlines::{Solid, fits};
    let solids: Vec<Solid> = solids()
        .into_iter()
        .map(|(low, high)| Solid::new(low, high))
        .collect();
    let pose =
        tomori_protocol::Pose::at(place.at.to_array()).facing_towards(place.towards.to_array());
    let [width, height, depth] = crate::trail::ROOM;
    let cluster = tomori_protocol::cluster::Cluster::new(
        &tomori_protocol::cluster::Anchor::new(place.name, pose).with_extent(width, height, depth),
    );
    assert_eq!(fits(&cluster, TERRACE, &solids), Ok(()));
}

#[test]
fn the_upper_falls_fall_from_the_upper_cliff_into_the_upper_pool_whose_stream_is_the_waterfall_s_lip()
 {
    let flat = |at: Vec3| Vec2::new(at.x, at.z);
    // The upper falls: from the upper cliff's lip, at its top, to its foot, in the pool, on the waterfall's way.
    let lip = flat(layout::UPPER_FALLS_LIP);
    let foot = flat(layout::UPPER_FALLS_FOOT);
    assert!((lip.distance(layout::BACKDROP_CENTRE) - layout::UPPER_CLIFF[1]).abs() < 1e-2);
    assert!((backdrop::ground(lip) - layout::UPPER_TOP).abs() < 1e-3);
    assert!(foot.distance(flat(layout::UPPER_POOL)) < layout::UPPER_POOL_RADIUS);
    let middle = (layout::UPPER_FALLS_LIP + layout::UPPER_FALLS_FOOT) * 0.5;
    assert!(middle.distance(layout::UPPER_FALLS) < 0.01, "{middle}");
    for at in [lip, foot, flat(layout::UPPER_POOL)] {
        assert!(backdrop::on_the_fall(at).1.abs() < 0.01, "{at}");
    }
    // Its pool's water covers its basin, below the terrace, and the stream from it runs at its level to the waterfall's
    // lip, which it leaves by, so that the two falls are one stream.
    assert!(backdrop::ground(flat(layout::UPPER_POOL)) < layout::UPPER_POOL.y - 0.2);
    assert_eq!(layout::UPPER_POOL.y, layout::CLIFF_TOP + 0.08);
    let water = backdrop::upper_water();
    assert_eq!(water.len() % 3, 0);
    let stream: Vec<_> = water
        .iter()
        .filter(|point| {
            point.kind == backdrop::SURFACE
                && (point.position.y - layout::UPPER_POOL.y).abs() < 1e-5
        })
        .collect();
    assert!(!stream.is_empty());
    let mouth = stream
        .iter()
        .map(|point| backdrop::on_the_fall(flat(point.position)).0)
        .fold(f32::INFINITY, f32::min);
    assert!((mouth - layout::CLIFF[1] - 0.9).abs() < 1e-3, "{mouth}");
    for point in &stream {
        let at = flat(point.position);
        assert!(backdrop::ground(at) < point.position.y + 0.3);
        assert!(point.flow.y >= 0.0);
    }
    let main = backdrop::water();
    let highest = main
        .iter()
        .filter(|point| point.kind == backdrop::FALLING)
        .map(|point| point.position.y)
        .fold(0.0, f32::max);
    assert!((highest - layout::UPPER_POOL.y).abs() < 1e-5);
    // The falls: from above the upper cliff's top to the pool, in front of its face, above its top where it comes over.
    let falling: Vec<_> = water
        .iter()
        .filter(|point| point.kind == backdrop::FALLING)
        .collect();
    assert!(falling.len() >= 12 * 3);
    for point in &falling {
        let at = flat(point.position);
        let radius = at.distance(layout::BACKDROP_CENTRE);
        if point.position.y < layout::UPPER_TOP {
            assert!(radius < layout::UPPER_CLIFF[1] - 0.1, "{point:?}");
        } else {
            assert!(point.position.y > backdrop::ground(at));
        }
        assert!(
            point.position.y >= layout::UPPER_POOL.y && point.position.y < layout::UPPER_TOP + 0.1
        );
    }
    // Foam on the pool, about its foot.
    for point in water.iter().filter(|point| point.kind == backdrop::FOAM) {
        assert!(
            flat(point.position).distance(flat(layout::UPPER_POOL))
                < layout::UPPER_POOL_RADIUS + 0.6
        );
        assert!((point.position.y - layout::UPPER_POOL.y - 0.05).abs() < 1e-4);
    }
    // Nothing of the bush stands on the terrace, by it or in its water.
    for plant in backdrop::plants() {
        let at = flat(plant.at);
        assert!(from_floor(at) > plant.size().0 + 1.0, "{plant:?}");
        assert!(at.distance(flat(layout::UPPER_POOL)) > 3.0, "{plant:?}");
    }
}
