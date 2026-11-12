// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::trail;

/// Where the eyes may be: above every corner of the ground participants land on, in the yard, along the trail and
/// round its clearings, and of the way out through the gate.
fn eyes() -> Vec<Vec3> {
    let ground = crate::walkable::navmesh()
        .triangles()
        .iter()
        .flatten()
        .map(|corner| Vec2::new(corner.x, corner.z));
    let way_out = trail::way_out()
        .into_iter()
        .flat_map(|(outline, _)| outline);
    ground
        .chain(way_out)
        .map(|corner| Vec3::new(corner.x, tomori_environment::sightlines::EYES, corner.y))
        .collect()
}

/// From `eye`, `position` is within the far plane, 2 m short of it, or else, from the beach's slope down to the river,
/// on the foothills' outer rings across the yard from it, or the bush on them, 46 m and more away, deep in the haze,
/// within the far plane or less than 1.5 m beyond it, where it cuts them off, as from the waterfall rock.
fn beyond_from_the_beach(eye: Vec3, position: Vec3) {
    // The eyes stand as high above the beach's slope as above the yard.
    let ground = Vec2::new(eye.x, eye.z);
    let eye = eye + Vec3::Y * crate::beach::height(ground).unwrap_or(0.0).min(0.0);
    let distance = eye.distance(position);
    if distance < FAR - 2.0 {
        return;
    }
    let at = Vec2::new(position.x, position.z) - BACKDROP_CENTRE;
    let towards = ground - BACKDROP_CENTRE;
    assert!(
        crate::beach::from_beach(ground) < 1e-3
            && at.length() > 30.0
            && at.dot(towards) < 0.0
            && distance > 46.0
            && distance < FAR + 1.5,
        "{eye} {position}: {distance} m, {} m out",
        at.length()
    );
}

/// From the eyes of one standing on the waterfall rock, which is 8 m beyond the trail's reach of the backdrop's middle,
/// `position` is within the far plane, or else one of the foothills' outer rings across the yard from it, which the
/// far plane cuts off, 46 m and more away, deep in the haze.
fn beyond_from_the_rock(position: Vec3) {
    let rock = layout::WATERFALL_ROCK + Vec3::Y * tomori_environment::sightlines::EYES;
    let distance = rock.distance(position);
    if distance >= FAR - 2.0 {
        let at = Vec2::new(position.x, position.z) - BACKDROP_CENTRE;
        let towards = Vec2::new(rock.x, rock.z) - BACKDROP_CENTRE;
        assert!(at.length() > 26.0 && at.dot(towards) < 0.0, "{position}");
        assert!(distance > 46.0);
    }
}

/// How far `point` is from the trail's middle, on the ground.
fn from_trail(point: Vec2) -> f32 {
    let line = trail::centreline();
    (0..line.len())
        .map(|index| {
            let (from, to) = (line[index], line[(index + 1) % line.len()]);
            let along = to - from;
            let t = ((point - from).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            point.distance(from + along * t)
        })
        .fold(f32::INFINITY, f32::min)
}

/// How far `point` is from the fence, on the ground, outside it.
fn beyond_fence(point: Vec2) -> f32 {
    let fence = layout::FENCE;
    (fence.low - point)
        .max(point - fence.high)
        .max(Vec2::ZERO)
        .length()
}

/// The headset's far plane: nothing nearer is cut off.
const FAR: f32 = 50.0;

#[test]
fn the_backdrop_stands_within_the_far_plane_from_where_participants_stand_at_the_campsite() {
    // A metre short of it rather than two, as the campsite stands a little further out than the trail.
    let eyes: Vec<Vec3> = crate::campsite::standing()
        .into_iter()
        .flat_map(|(middle, radius)| {
            (0..12).map(move |step| {
                let yaw = step as f32 / 12.0 * TAU;
                let at = middle + Vec2::new(yaw.sin(), yaw.cos()) * radius;
                Vec3::new(at.x, tomori_environment::sightlines::EYES, at.y)
            })
        })
        .collect();
    let foothills = model();
    for eye in &eyes {
        for position in &foothills.meshes[0].primitives[0].positions {
            assert!(eye.distance(*position) < FAR - 1.0, "{eye} {position}");
        }
        for plant in plants() {
            let (_, height) = plant.size();
            assert!(
                eye.distance(plant.at + Vec3::Y * height) < FAR - 1.0,
                "{plant:?}"
            );
        }
    }
}

#[test]
fn the_backdrop_stands_beyond_the_fence_within_the_far_plane() {
    let eyes = eyes();
    // The foothills' ground: none of it within 2.5 m of the fence, its edge level with the meadow, and all of it within
    // the far plane from anywhere participants land, in the yard and along the trail.
    let foothills = model();
    let ground = &foothills.meshes[0].primitives[0];
    for position in &ground.positions {
        assert!(
            beyond_fence(Vec2::new(position.x, position.z)) > 2.5,
            "{position}"
        );
        let at = Vec2::new(position.x, position.z);
        let radius = at.distance(BACKDROP_CENTRE);
        if radius < RINGS[0] + 0.01 {
            assert_eq!(position.y, MEADOW);
        }
        // Below the meadow only in the river's valley and the pool's hollow.
        if position.y < MEADOW {
            assert!(
                layout::nearest_river(at).1 < 8.0
                    || at.distance(Vec2::new(POOL.x, POOL.z)) < RIM[3],
                "{position}"
            );
        }
        for eye in &eyes {
            beyond_from_the_beach(*eye, *position);
        }
        beyond_from_the_rock(*position);
    }
    // The yard's meadow, a flat polygon, reaches the foothills' first ring, corner for corner, so that the two meet
    // without a step or a gap, and goes no further, so that it never covers the river's channel.
    let yard = props::model();
    let positions: Vec<Vec3> = yard
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.primitives)
        .flat_map(|primitive| primitive.positions.iter().copied())
        .collect();
    for corner in meadow() {
        assert!(
            ground.positions.contains(&corner) && positions.contains(&corner),
            "{corner}"
        );
        assert_eq!(corner.y, MEADOW);
    }
    for position in positions.iter().filter(|position| position.y == MEADOW) {
        assert!(Vec2::new(position.x, position.z).distance(BACKDROP_CENTRE) < RINGS[0] + 1e-3);
    }
    // Inside the first ring is the meadow, level.
    assert_eq!(super::ground(BACKDROP_CENTRE), 0.0);
    assert_eq!(super::ground(Vec2::new(5.0, 1.6)), 0.0);
    // Every plant at least 3 m beyond the fence, standing on the ground, none in the way in through the gate nor on the
    // trail, and its top within the far plane.
    for plant in plants() {
        let (reach, height) = plant.size();
        let at = Vec2::new(plant.at.x, plant.at.z);
        assert!(!layout::FENCE.grown(3.0 + reach).contains(at), "{plant:?}");
        assert!((plant.at.y - super::ground(at)).abs() < 1e-4);
        assert!(!(at.y > 4.0 && at.x.abs() < 6.0 + reach && at.distance(BACKDROP_CENTRE) < 26.0));
        assert!(clear(at, reach, height), "{plant:?}");
        for eye in &eyes {
            beyond_from_the_beach(*eye, plant.at + Vec3::Y * height);
        }
        beyond_from_the_rock(plant.at + Vec3::Y * height);
        // Nor on the trail, beyond the fence, where participants walk.
        assert!(from_trail(at) > reach + trail::HALF, "{plant:?}");
        let solid = plant.solid();
        let footprint =
            layout::Footprint::new([solid.low[0], solid.low[2]], [solid.high[0], solid.high[2]]);
        assert!(
            crate::walkable::navmesh()
                .triangles()
                .iter()
                .flatten()
                .all(|corner| !footprint.contains(Vec2::new(corner.x, corner.z))),
            "{plant:?}"
        );
        assert_eq!(crate::walkable::height_at(at), None, "{plant:?}");
    }
    // And the pines with them, out of the water.
    for (base, height) in props::pines() {
        let at = Vec2::new(base.x, base.z);
        assert!(open(at, height * 0.32, height), "{base}");
        assert!(base.y <= super::ground(at));
    }
    // Only where plants may stand: not in the yard, the river, the pool, on the cliff, in the gate's way or beyond the
    // far plane.
    for (point, reach) in [
        (layout::BACKDROP_CENTRE, 1.0),
        (Vec2::new(layout::RIVER[7].x, layout::RIVER[7].z), 1.0),
        (Vec2::new(POOL.x, POOL.z), 1.0),
        (Vec2::new(layout::WATERFALL.x, layout::WATERFALL.z), 1.0),
        (Vec2::new(0.0, 12.0), 1.0),
        (Vec2::new(0.0, -45.0), 1.0),
        (Vec2::new(-9.5, -4.0), 2.0),
    ] {
        assert!(!clear(point, reach, 3.0), "{point}");
    }
    assert!(clear(Vec2::new(-20.0, -4.0), 1.0, 3.0));
}

#[test]
fn the_waterfall_the_pool_and_the_river_are_seen_from_where_participants_arrive_and_from_the_workbench()
 {
    let solids: Vec<Solid> = layout::solids()
        .into_iter()
        .map(|(low, high)| Solid::new(low, high))
        .collect();
    let plants = plants();
    let views = views();
    assert_eq!(views.len(), 12);
    for (from, to) in &views {
        let (from, to) = (Vec3::from_array(*from), Vec3::from_array(*to));
        // Nothing in the yard, no plant and no pine stands in the way.
        assert!(
            !solids
                .iter()
                .any(|solid| solid.blocks(from.to_array(), to.to_array())),
            "{to}"
        );
        assert!(
            !plants
                .iter()
                .any(|plant| plant.solid().blocks(from.to_array(), to.to_array())),
            "{to}"
        );
        for (base, height) in props::pines() {
            let reach = height * 0.32;
            let pine = Solid::new(
                (base - Vec3::new(reach, 0.0, reach)).to_array(),
                (base + Vec3::new(reach, height, reach)).to_array(),
            );
            assert!(!pine.blocks(from.to_array(), to.to_array()), "{base} {to}");
        }
        // Nor does the ground rise into it, but where it ends.
        let length = from.distance(to);
        for step in 1..200 {
            let point = from.lerp(to, step as f32 / 200.0);
            if point.distance(to) < 1.2 {
                break;
            }
            assert!(
                super::ground(Vec2::new(point.x, point.z)) < point.y - 0.05,
                "{point} of {length}"
            );
        }
    }
    // Something standing in a view, beyond the fence, would block it, so that nothing may stand there.
    let within = Vec3::from_array(views[3].0).lerp(Vec3::from_array(views[3].1), 0.75);
    let within = Vec2::new(within.x, within.z);
    assert!(!layout::FENCE.grown(3.0).contains(within));
    assert!(!open(within, 0.5, 4.0));
    assert!(open(within + Vec2::new(-12.0, 0.0), 0.5, 4.0));
}

#[test]
fn the_waterfall_falls_from_its_cliff_into_its_pool_which_feeds_the_river_downhill() {
    // The cliff: its foot in the pool's basin, below its water, and its top behind the lip.
    let foot = Vec2::new(layout::WATERFALL_FOOT.x, layout::WATERFALL_FOOT.z);
    let lip = Vec2::new(layout::WATERFALL_LIP.x, layout::WATERFALL_LIP.z);
    assert!(super::ground(foot) < POOL.y - 0.2);
    assert!((super::ground(lip) - layout::CLIFF_TOP).abs() < 1e-3);
    assert!((foot.distance(BACKDROP_CENTRE) - layout::CLIFF[0]).abs() < 1e-2);
    assert!((lip.distance(BACKDROP_CENTRE) - layout::CLIFF[1]).abs() < 1e-2);
    let middle = (layout::WATERFALL_LIP + layout::WATERFALL_FOOT) * 0.5;
    assert!(middle.distance(layout::WATERFALL) < 0.01, "{middle}");
    // The pool's water covers its middle, and the river's its own, running downhill all the way.
    assert!(super::ground(Vec2::new(POOL.x, POOL.z)) < POOL.y - 0.1);
    for pair in RIVER.windows(2) {
        assert!(pair[1].y < pair[0].y, "{pair:?}");
        let between = pair[0].lerp(pair[1], 0.5);
        assert!(
            super::ground(Vec2::new(between.x, between.z)) < between.y - 0.05,
            "{between}"
        );
        // Below the trail and the yard, in its channel.
        assert!(pair[1].y < trail::HEIGHT - 0.3, "{pair:?}");
    }
    const { assert!(POOL.y < trail::HEIGHT - 0.25) };
    // Its banks rise out of it either side within its water's spread.
    for point in &RIVER[2..] {
        let (nearest, distance) = layout::nearest_river(Vec2::new(point.x, point.z));
        assert!(distance < 1e-4 && nearest.distance(*point) < 1e-4);
    }
    // It runs round the yard, never nearer the fence than 7.5 m.
    for point in layout::river_line(8) {
        assert!(beyond_fence(Vec2::new(point.x, point.z)) > 7.5, "{point}");
    }
    // The water: the river's surface at its levels, the waterfall from above its lip to the pool, and foam on the pool.
    let water = water();
    assert_eq!(water.len() % 3, 0);
    let falling: Vec<_> = water.iter().filter(|point| point.kind == FALLING).collect();
    let foam: Vec<_> = water.iter().filter(|point| point.kind == FOAM).collect();
    let highest = falling
        .iter()
        .map(|point| point.position.y)
        .fold(0.0, f32::max);
    let lowest = falling
        .iter()
        .map(|point| point.position.y)
        .fold(f32::INFINITY, f32::min);
    assert!(highest > layout::CLIFF_TOP && highest < layout::CLIFF_TOP + 0.1);
    assert!((lowest - POOL.y).abs() < 0.05);
    for point in &falling {
        // In front of the cliff's face, which it falls down, but where it comes over the lip, above the cliff's top.
        let (along, _) = by_waterfall(Vec2::new(point.position.x, point.position.z));
        if point.position.y < layout::CLIFF_TOP {
            assert!(along < layout::CLIFF[1] - 0.25, "{point:?}");
        } else {
            assert!(
                point.position.y > super::ground(Vec2::new(point.position.x, point.position.z))
            );
        }
        assert!(point.flow.y >= 0.0);
    }
    for point in &foam {
        assert!((point.position.y - POOL.y - 0.05).abs() < 0.01);
        assert!(
            Vec2::new(point.position.x, point.position.z).distance(Vec2::new(POOL.x, POOL.z))
                < POOL_RADIUS + 1.0
        );
    }
    // The mist rises from the pool about the fall's foot, low, before the cliff's lip.
    let mist: Vec<_> = water.iter().filter(|point| point.kind == MIST).collect();
    assert_eq!(mist.len(), 3 * 15 * 3);
    for point in &mist {
        let at = Vec2::new(point.position.x, point.position.z);
        assert!(point.position.y >= POOL.y && point.position.y <= POOL.y + 1.0);
        assert!(at.distance(Vec2::new(POOL.x, POOL.z)) < POOL_RADIUS);
        assert!(by_waterfall(at).0 < layout::CLIFF[1] - 0.25);
    }
    for point in water.iter().filter(|point| point.kind == SURFACE) {
        // At the pool's level or below, in the channel below the trail.
        assert!(point.position.y <= POOL.y + 0.03);
        assert!(point.flow.y >= 0.0);
    }
}

#[test]
fn the_sounds_are_named_where_the_waterfall_the_pool_the_river_and_the_bush_are() {
    let sounds = layout::sounds();
    let named = |name: &str| sounds.iter().filter(|(each, _)| *each == name).count();
    assert_eq!(
        [
            named("waterfall"),
            named("pool"),
            named("river"),
            named("bush")
        ],
        [1, 1, RIVER.len() - 1, layout::BUSH.len()]
    );
    assert_eq!(sounds.len(), 2 + RIVER.len() - 1 + layout::BUSH.len());
    assert_eq!(sounds[0], ("waterfall", layout::WATERFALL));
    assert_eq!(sounds[1], ("pool", POOL));
    let plants = plants();
    for (name, at) in &sounds {
        // All beyond the fence, 7 m and more from it.
        assert!(beyond_fence(Vec2::new(at.x, at.z)) > 7.0, "{name} {at}");
        if *name == "bush" {
            // On the ground, among the bush's plants.
            let ground = Vec2::new(at.x, at.z);
            assert!((at.y - super::ground(ground)).abs() < 1e-4);
            let (_, radius) = layout::BUSH
                .iter()
                .find(|(middle, _)| *middle == ground)
                .unwrap();
            let near = plants
                .iter()
                .filter(|plant| Vec2::new(plant.at.x, plant.at.z).distance(ground) < *radius)
                .count();
            assert!(near >= 3, "{at}: {near}");
        } else {
            // On the water.
            assert!(super::ground(Vec2::new(at.x, at.z)) < at.y, "{name} {at}");
        }
    }
}

#[test]
fn the_bush_is_the_same_every_time_each_plant_turned_and_sized_its_own_way() {
    let plants = plants();
    assert_eq!(plants, super::plants());
    for (kind, count, [least, most]) in COUNTS {
        let of: Vec<_> = plants.iter().filter(|plant| plant.kind == kind).collect();
        assert_eq!(of.len(), count, "{kind:?}");
        assert!(of.iter().all(|plant| (least..=most).contains(&plant.scale)));
        assert!(of.iter().all(|plant| (0.0..TAU).contains(&plant.turn)));
        assert!(
            of.windows(2)
                .all(|pair| pair[0].turn != pair[1].turn && pair[0].scale != pair[1].scale)
        );
        // Its model is the same each time it is built, and stands about its origin within its size; a tree fern, drawn
        // with the ferns' meshes, reaches as far as it does and stands as tall, give or take a tenth.
        let (Some(model), Some(again)) = (kind.model(), kind.model()) else {
            let (reach, height) = kind.size();
            let bush = crate::ferns::bush();
            assert_eq!(bush.len(), count);
            for (fern, plant) in bush.iter().zip(&of) {
                assert_eq!((fern.at, fern.turn), (plant.at, plant.turn));
                let (drawn, tall) = fern.size();
                assert!((drawn - reach * plant.scale).abs() < 1e-3, "{fern:?}");
                assert!(
                    (tall / (height * plant.scale) - 1.0).abs() < 0.15,
                    "{fern:?}"
                );
            }
            continue;
        };
        let primitive = &model.meshes[0].primitives[0];
        assert_eq!(primitive.positions, again.meshes[0].primitives[0].positions);
        let (reach, height) = kind.size();
        for position in &primitive.positions {
            assert!(
                Vec2::new(position.x, position.z).length() <= reach + 1e-3,
                "{kind:?} {position}"
            );
            assert!(
                position.y >= -SUNK - 1e-3 && position.y <= height + 1e-3,
                "{kind:?} {position}"
            );
        }
        let tallest = primitive
            .positions
            .iter()
            .map(|position| position.y)
            .fold(0.0, f32::max);
        assert!(tallest > height * 0.85, "{kind:?} {tallest}");
    }
    // None stands within another's reach, nor a pine's.
    for (index, plant) in plants.iter().enumerate() {
        for other in &plants[index + 1..] {
            let apart =
                Vec2::new(plant.at.x, plant.at.z).distance(Vec2::new(other.at.x, other.at.z));
            assert!(apart > (plant.size().0 + other.size().0) * 0.45);
        }
    }
    // A plant's place is its transform's.
    let plant = plants[0];
    assert!(
        plant
            .transform()
            .transform_point3(Vec3::ZERO)
            .distance(plant.at)
            < 1e-4
    );
    assert!((plant.transform().transform_vector3(Vec3::X).length() - plant.scale).abs() < 1e-4);
}

#[test]
fn the_ranges_ring_the_horizon_bluer_and_hazier_with_distance_the_highest_under_snow() {
    let ranges = ranges();
    assert_eq!(ranges.len() % 3, 0);
    let elevation = |far: &Far| far.direction.y.asin().to_degrees();
    for far in &ranges {
        assert!((far.direction.length() - 1.0).abs() < 1e-4);
        assert!(
            elevation(far) >= FOOT - 1e-3 && elevation(far) < 20.0,
            "{far:?}"
        );
        assert!((0.0..1.0).contains(&far.fade));
    }
    // From the farthest, drawn first, to the nearest, drawn over it: each less faded into the sky than the last.
    let fades: Vec<f32> = ranges.iter().map(|far| far.fade).collect();
    assert!(fades.windows(2).all(|pair| pair[1] <= pair[0]));
    assert_eq!(fades[0], RANGES[0].fade);
    assert_eq!(*fades.last().unwrap(), RANGES[2].fade);
    // All the way round, each range's ridge above the horizon.
    for (index, range) in RANGES.iter().enumerate() {
        let of: Vec<_> = ranges.iter().filter(|far| far.fade == range.fade).collect();
        for degree in (0..360).step_by(3) {
            let way = towards(degree as f32, 0.0);
            let above = of.iter().any(|far| {
                let flat = Vec3::new(far.direction.x, 0.0, far.direction.z).normalize();
                flat.dot(way) > (2.5f32).to_radians().cos() && elevation(far) > range.base * 0.9
            });
            assert!(above, "{index} {degree}");
        }
    }
    // Snow, bright under the moon, only on the farther ranges' highest faces.
    // Brighter than any rock could be, under the moon and the sky.
    let rock = colors::RANGE_FAR.x * 1.1 * (evening::SKY.x + evening::MOONLIGHT.x);
    let bright = |far: &&Far| far.color.x > rock;
    let snowy: Vec<_> = ranges.iter().filter(bright).collect();
    assert!(!snowy.is_empty());
    assert!(
        snowy
            .iter()
            .all(|far| far.fade > RANGES[2].fade && elevation(far) > 2.0)
    );
    let highest = ranges.iter().map(elevation).fold(0.0, f32::max);
    assert!(highest > 12.0, "{highest}");
}

#[test]
fn the_backdrop_is_modest_for_the_quest() {
    // Under the budget, about 17 thousand triangles, which the scenery draws in ten draws: the foothills, one for each
    // kind of plant with all its instances but the tree ferns, which the ferns draw, the river's stones, all instances of
    // one, the waterfall rock, the trail, the water and the ranges; and the stations' plaques, five small draws of text.
    let triangles = Triangles::counted();
    assert_eq!(
        triangles,
        Triangles {
            foothills: 2918,
            plants: 10_862,
            ranges: 1184,
            water: 197,
            stones: 765,
            rock: 40,
            trail: 1144,
        },
        "the counts the documentation gives"
    );
    assert_eq!(triangles.total(), 17_110);
    assert_eq!(stones().len(), 51);
    assert!(triangles.total() < BUDGET);
    // Each kind of plant is a few dozen to a little over a hundred triangles, drawn once for all its instances, but the
    // tree ferns, which the ferns draw and count.
    assert_eq!(Kind::Fern.model(), None);
    for (kind, count) in [
        (Kind::Canopy, 18),
        (Kind::Podocarp, 112),
        (Kind::CabbageTree, 145),
        (Kind::Flax, 39),
    ] {
        let model = kind.model().unwrap();
        assert_eq!(
            model.meshes[0].primitives[0].indices.len() / 3,
            count,
            "{kind:?}"
        );
    }
    assert_eq!(Kind::ALL.len(), 5);
    // A river stone is fifteen, drawn once for all of them.
    assert_eq!(stone().meshes[0].primitives[0].indices.len() / 3, 15);
}

/// How high the drawn ground is at `point`, from the triangle of `model`'s mesh beneath it, if any.
fn triangle_height(model: &Model, point: Vec2) -> Option<f32> {
    let primitive = &model.meshes[0].primitives[0];
    primitive.indices.chunks(3).find_map(|triangle| {
        let [a, b, c] = [0, 1, 2].map(|corner| primitive.positions[triangle[corner] as usize]);
        let flat = |p: Vec3| Vec2::new(p.x, p.z);
        let (a2, b2, c2) = (flat(a), flat(b), flat(c));
        let area = (b2 - a2).perp_dot(c2 - a2);
        let u = (b2 - point).perp_dot(c2 - point) / area;
        let v = (c2 - point).perp_dot(a2 - point) / area;
        let w = 1.0 - u - v;
        (u >= -1e-5 && v >= -1e-5 && w >= -1e-5).then_some(a.y * u + b.y * v + c.y * w)
    })
}

/// Points along the edges of the water's surface, at its level: the river's either side, but within the pool, and the
/// pool's rim, but where the river leaves it.
fn water_edges() -> Vec<Vec3> {
    let line = layout::river_line(layout::RIVER_STEPS);
    let half = RIVER_HALF_WIDTH + SPREAD;
    let pool = Vec2::new(POOL.x, POOL.z);
    let mut sides: [Vec<Vec3>; 2] = [Vec::new(), Vec::new()];
    for (index, middle) in line.iter().enumerate() {
        let before = line[index.saturating_sub(1)];
        let after = line[(index + 1).min(line.len() - 1)];
        let tangent = Vec2::new(after.x - before.x, after.z - before.z).normalize();
        let across = Vec3::new(-tangent.y, 0.0, tangent.x) * half;
        sides[0].push(*middle - across);
        sides[1].push(*middle + across);
    }
    let mut edges = Vec::new();
    for side in &sides {
        for pair in side.windows(2) {
            for step in 0..4 {
                let point = pair[0].lerp(pair[1], step as f32 / 4.0);
                if Vec2::new(point.x, point.z).distance(pool) > POOL_RADIUS + 0.6 {
                    edges.push(point);
                }
            }
        }
    }
    let rim = POOL_RADIUS + 0.6;
    for step in 0..96 {
        let yaw = step as f32 / 96.0 * TAU;
        let point = POOL + Vec3::new(yaw.sin(), 0.02, yaw.cos()) * Vec3::new(rim, 1.0, rim);
        if layout::nearest_river(Vec2::new(point.x, point.z)).1 > half {
            edges.push(point);
        }
    }
    edges
}

#[test]
fn the_river_and_the_pool_lie_between_banks_that_stand_above_their_water() {
    let foothills = model();
    // The ground as it is drawn is its triangles'.
    for step in 0..400 {
        let yaw = step as f32 * 0.37;
        let radius =
            RINGS[0] + 0.05 + (step as f32 * 0.61) % (RINGS[RINGS.len() - 1] - RINGS[0] - 0.1);
        let point = BACKDROP_CENTRE + way(yaw) * radius;
        let drawn = drawn_ground(point).unwrap();
        let triangle = triangle_height(&foothills, point).unwrap();
        assert!(
            (drawn - triangle).abs() < 1e-4,
            "{point} {drawn} {triangle}"
        );
    }
    assert_eq!(drawn_ground(BACKDROP_CENTRE), None);
    assert_eq!(drawn_ground(Vec2::new(0.0, 40.0)), None);
    // Wherever the water's surface ends, beneath the river's banks and the pool's rim, the ground as it is drawn stands
    // above it, so that the water lies between its banks, not above them; but where the river leaves the pool, whose
    // edge stands only a little above the river's water there.
    let edges = water_edges();
    assert!(edges.len() > 300);
    for edge in &edges {
        let drawn = drawn_ground(Vec2::new(edge.x, edge.z)).unwrap();
        assert!(drawn > edge.y + 0.04, "{edge} {drawn}");
    }
    let lip = layout::RIVER[1];
    assert!(lip.y < POOL.y && POOL.y + 0.02 - lip.y < 0.05);
    assert!(Vec2::new(lip.x, lip.z).distance(Vec2::new(POOL.x, POOL.z)) > POOL_RADIUS + 0.6);
    // The banks rise out of the water between the bed and their lip, held a lip above the water beyond its spread.
    const { assert!(BANKS[1] < RIVER_HALF_WIDTH + SPREAD && BANKS[2] > RIVER_HALF_WIDTH + SPREAD) };
    const { assert!(RIM[1] < POOL_RADIUS + 0.6 && RIM[2] > POOL_RADIUS + 0.6) };
    const { assert!(LIP >= 0.05) };
}

#[test]
fn the_flax_stands_on_the_banks_its_leaves_clear_of_the_water() {
    let flax: Vec<Plant> = plants()
        .into_iter()
        .filter(|plant| plant.kind == Kind::Flax)
        .collect();
    let pool = Vec2::new(POOL.x, POOL.z);
    let by_water = flax
        .iter()
        .filter(|plant| {
            layout::nearest_river(Vec2::new(plant.at.x, plant.at.z)).1 < 6.0
                || Vec2::new(plant.at.x, plant.at.z).distance(pool) < POOL_RADIUS + 5.0
        })
        .count();
    assert!(by_water > 20, "{by_water}");
    for plant in &flax {
        let at = Vec2::new(plant.at.x, plant.at.z);
        let (reach, _) = plant.size();
        // Its foot beyond where any of the water is drawn, beneath the banks too.
        let (nearest, river) = layout::nearest_river(at);
        assert!(river > RIVER_HALF_WIDTH + SPREAD, "{plant:?}");
        assert!(at.distance(pool) > POOL_RADIUS + 0.6, "{plant:?}");
        // And nowhere within its leaves' reach is the water open, the river's, where the bank stands above it, or the
        // pool's.
        for step in 0..16 {
            let yaw = step as f32 / 16.0 * TAU;
            let leaf = at + Vec2::new(yaw.sin(), yaw.cos()) * reach;
            let (_, distance) = layout::nearest_river(leaf);
            if distance < RIVER_HALF_WIDTH + SPREAD {
                let drawn = drawn_ground(leaf).unwrap_or(0.0);
                assert!(drawn > nearest.y, "{plant:?} {leaf}");
            }
            assert!(leaf.distance(pool) > POOL_RADIUS + 0.6, "{plant:?} {leaf}");
        }
    }
}

#[test]
fn the_river_s_stones_lie_along_its_banks_in_its_shallows_and_about_the_pool_the_same_every_time() {
    let stones = stones();
    assert_eq!(stones, super::stones());
    let count = stones.len();
    assert!(
        (40..=(SHORE_STONES + SHALLOW_STONES + POOL_STONES) as usize).contains(&count),
        "{count}"
    );
    let pool = Vec2::new(POOL.x, POOL.z);
    let (mut shore, mut shallow, mut pooled) = (0, 0, 0);
    let views = views();
    for (index, stone) in stones.iter().enumerate() {
        let at = Vec2::new(stone.at.x, stone.at.z);
        let (nearest, river) = layout::nearest_river(at);
        let top = stone.at.y + STONE_TOP * stone.size.y;
        if river < BANKS[0] {
            // In the shallows, set into the bed and standing out of the water.
            shallow += 1;
            assert!(top > nearest.y + 0.05, "{stone:?}");
            assert!(
                stone.at.y - STONE_BOTTOM * stone.size.y < drawn_ground(at).unwrap(),
                "{stone:?}"
            );
        } else {
            // On the ground as it is drawn, by the water's edge, of the river or the pool.
            let drawn = drawn_ground(at).unwrap();
            assert!((stone.at.y - (drawn - 0.03)).abs() < 1e-4, "{stone:?}");
            if river < BANKS[1] + 0.2 {
                shore += 1;
            } else {
                assert!(at.distance(pool) < POOL_RADIUS + 0.2, "{stone:?}");
                pooled += 1;
            }
        }
        // Grey, rounded, of every size, each apart from the others and out of the views the backdrop keeps open.
        assert!(
            (0.3..0.9).contains(&stone.size.x) && stone.size.y > stone.size.x * 0.7,
            "{stone:?}"
        );
        assert!((0.0..TAU).contains(&stone.turn));
        for other in &stones[index + 1..] {
            let apart = at.distance(Vec2::new(other.at.x, other.at.z));
            assert!(apart > (stone.size.x + other.size.x) * 0.6);
        }
        assert!(
            views
                .iter()
                .all(|(from, to)| !stone.solid().blocks(*from, *to))
        );
        assert!(
            stone
                .transform()
                .transform_point3(Vec3::ZERO)
                .distance(stone.at)
                < 1e-4
        );
    }
    assert!(
        shore >= 30 && shallow >= 5 && pooled >= 4,
        "{shore} {shallow} {pooled}"
    );
    let sizes: Vec<f32> = stones.iter().map(|stone| stone.size.x).collect();
    let (least, most) = sizes
        .iter()
        .fold((f32::INFINITY, 0.0f32), |(least, most), size| {
            (least.min(*size), most.max(*size))
        });
    assert!(most > least * 1.8, "{least} {most}");
    // Its model is a rounded stone of a metre across, sitting on its foot with a little of it below.
    let model = stone();
    let positions = &model.meshes[0].primitives[0].positions;
    for position in positions {
        assert!(
            Vec2::new(position.x, position.z).length() <= 0.53,
            "{position}"
        );
        assert!(
            (-STONE_BOTTOM - 1e-4..=STONE_TOP + 1e-4).contains(&position.y),
            "{position}"
        );
    }
}

#[test]
fn the_river_and_the_pool_lie_in_a_channel_below_their_banks_the_trail_and_the_yard() {
    let pool = Vec2::new(POOL.x, POOL.z);
    let spread = RIVER_HALF_WIDTH + SPREAD;
    // The water nearest `at`: the river's, or the pool's about its rim.
    let water = |at: Vec2| {
        let (nearest, _) = layout::nearest_river(at);
        if at.distance(pool) < RIM[3] + 2.0 {
            nearest.y.max(POOL.y)
        } else {
            nearest.y
        }
    };
    // Everywhere beyond the water's spread, the ground as it is drawn stands at least 0.25 m above the water nearest it,
    // a quarter of a metre apart over the whole backdrop, so that the water lies in its channel and its hollow, and
    // would stay there.
    let mut sampled = 0;
    for x in -160..160 {
        for z in -180..140 {
            let at = Vec2::new(x as f32 * 0.25, z as f32 * 0.25);
            let Some(drawn) = drawn_ground(at) else {
                continue;
            };
            if layout::nearest_river(at).1 < spread || at.distance(pool) < POOL_RADIUS + 0.6 {
                continue;
            }
            sampled += 1;
            assert!(drawn > water(at) + 0.25, "{at} {drawn}");
        }
    }
    assert!(sampled > 40_000, "{sampled}");
    // Its banks' tops, either side of the river beyond the pool where its water's spread ends, stand from 0.3 m to a
    // metre above it, varying along it; and its bed lies below it.
    let line = layout::river_line(8);
    let mut tops = Vec::new();
    for (index, middle) in line.iter().enumerate() {
        let at = Vec2::new(middle.x, middle.z);
        if at.distance(pool) < RIM[2] {
            continue;
        }
        assert!(drawn_ground(at).unwrap() < middle.y - 0.05, "{middle}");
        let before = line[index.saturating_sub(1)];
        let after = line[(index + 1).min(line.len() - 1)];
        let across = Vec2::new(after.x - before.x, after.z - before.z)
            .normalize()
            .perp();
        for side in [-1.0, 1.0] {
            let edge = at + across * side * spread;
            // Where a bend brings another reach nearer, that reach's.
            if layout::nearest_river(edge).1 < spread - 0.05 {
                continue;
            }
            let top = drawn_ground(edge).unwrap() - middle.y;
            assert!((0.3..1.0).contains(&top), "{edge} {top}");
            tops.push(top);
        }
    }
    assert!(tops.len() > 120, "{}", tops.len());
    let least = tops.iter().copied().fold(f32::INFINITY, f32::min);
    let most = tops.iter().copied().fold(0.0, f32::max);
    assert!(most - least > 0.25, "{least} {most}");
    const { assert!(LIP >= 0.4 && LIP + RISE <= 0.8) };
    // The pool's rim stands above its water all round, but where the river leaves it.
    for step in 0..96 {
        let yaw = step as f32 / 96.0 * TAU;
        let rim = pool + Vec2::new(yaw.sin(), yaw.cos()) * (POOL_RADIUS + 0.6);
        if layout::nearest_river(rim).1 > BANKS[2] {
            assert!(drawn_ground(rim).unwrap() > POOL.y + 0.3, "{rim}");
        }
    }
    // The trail and the yard stand above the water, 0.3 m and more wherever the trail passes the river, by the pool's
    // lip, and 0.4 m and more beside the river beyond it, so that it walks the higher ground beside it.
    for point in trail::centreline() {
        let (nearest, distance) = layout::nearest_river(point);
        if distance < 10.0 {
            assert!(trail::HEIGHT - nearest.y > 0.3, "{point} {nearest}");
        }
        if distance < 10.0 && nearest.y < RIVER[2].y {
            assert!(trail::HEIGHT - nearest.y > 0.4, "{point} {nearest}");
        }
    }
    assert!(RIVER.iter().all(|point| point.y < trail::HEIGHT - 0.25));
}

#[test]
fn the_waterfall_rock_stands_in_the_pool_before_the_fall_flat_topped_out_of_the_water_and_the_spray()
 {
    let rock = rock();
    let primitive = &rock.meshes[0].primitives[0];
    let middle = layout::WATERFALL_ROCK;
    let flat = |at: Vec3| Vec2::new(at.x, at.z);
    // Every corner within its reach of its middle, none above its top, its foot below the water, which hides the rest.
    for position in &primitive.positions {
        assert!(flat(*position).distance(flat(middle)) <= layout::ROCK_REACH + 1e-4);
        assert!(position.y <= middle.y + 1e-6 && position.y > layout::ROCK_FOOT);
    }
    let lowest = primitive
        .positions
        .iter()
        .map(|position| position.y)
        .fold(f32::INFINITY, f32::min);
    assert!(lowest < POOL.y - 0.05, "{lowest}");
    // Its top flat at its height, facing up, from its middle out to at least half a metre all round, where one stands.
    let top: Vec<[Vec3; 3]> = primitive
        .indices
        .chunks(3)
        .map(|triangle| {
            triangle
                .iter()
                .map(|index| primitive.positions[*index as usize])
        })
        .map(|mut corners| [(); 3].map(|_| corners.next().unwrap()))
        .filter(|corners| {
            corners
                .iter()
                .all(|corner| (corner.y - middle.y).abs() < 1e-6)
        })
        .collect();
    assert_eq!(top.len(), ROCK_SIDES);
    for corners in &top {
        let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        assert!(normal.y > 0.0);
        for corner in corners {
            let reach = flat(*corner).distance(flat(middle));
            assert!(reach < 1e-6 || reach > 0.5, "{corner}");
        }
    }
    assert_eq!(primitive.indices.len() / 3, 40);
    // Half a metre above the pool, in its water, on its basin, clear of its shore.
    assert!((middle.y - POOL.y - 0.5).abs() < 1e-6);
    let pool = Vec2::new(POOL.x, POOL.z);
    assert!(flat(middle).distance(pool) + layout::ROCK_REACH < POOL_RADIUS - 1.0);
    assert!(super::ground(flat(middle)) < layout::ROCK_FOOT + 0.06);
    // In front of the fall, within 3.2 m of its foot, close enough to feel its spray, and clear of its water by half a
    // metre and its mist by 30 cm.
    let foot = flat(layout::WATERFALL_FOOT);
    assert!(flat(middle).distance(foot) < 3.2);
    let (along, across) = by_waterfall(flat(middle));
    assert!(
        along < layout::CLIFF[0] - 2.0 && across > 1.0,
        "{along} {across}"
    );
    for point in water() {
        let at = flat(point.position);
        let clear = match point.kind {
            FALLING => 0.5,
            MIST => 0.3,
            _ => continue,
        };
        assert!(
            at.distance(flat(middle)) > layout::ROCK_REACH + clear,
            "{point:?}"
        );
    }
    // Out of every view the backdrop keeps open, but the lookout's of it.
    let solid = Solid::new(
        [middle.x - 0.9, layout::ROCK_FOOT, middle.z - 0.9],
        [middle.x + 0.9, middle.y, middle.z + 0.9],
    );
    for (from, to) in views() {
        assert!(!solid.blocks(from, to), "{to:?}");
    }
}
