// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// Where the eyes may be in the yard: above every corner of the ground participants land on.
fn eyes() -> Vec<Vec3> {
    layout::WALKABLE
        .iter()
        .flat_map(|footprint| {
            [
                Vec2::new(footprint.low.x, footprint.low.y),
                Vec2::new(footprint.low.x, footprint.high.y),
                Vec2::new(footprint.high.x, footprint.low.y),
                Vec2::new(footprint.high.x, footprint.high.y),
            ]
        })
        .map(|corner| Vec3::new(corner.x, tomori_environment::sightlines::EYES, corner.y))
        .collect()
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
fn the_backdrop_stands_beyond_the_fence_within_the_far_plane() {
    let eyes = eyes();
    // The foothills' ground: none of it within 2.5 m of the fence, its edge level with the meadow, and all of it within
    // the far plane from anywhere participants land.
    let foothills = model();
    let ground = &foothills.meshes[0].primitives[0];
    for position in &ground.positions {
        assert!(
            beyond_fence(Vec2::new(position.x, position.z)) > 2.5,
            "{position}"
        );
        let radius = Vec2::new(position.x, position.z).distance(BACKDROP_CENTRE);
        if radius < RINGS[0] + 0.01 {
            assert_eq!(position.y, 0.0);
        }
        assert!(position.y >= 0.0, "{position}");
        for eye in &eyes {
            assert!(eye.distance(*position) < FAR - 2.0, "{eye} {position}");
        }
    }
    // Inside the first ring is the meadow, level.
    assert_eq!(super::ground(BACKDROP_CENTRE), 0.0);
    assert_eq!(super::ground(Vec2::new(5.0, 1.6)), 0.0);
    // Every plant at least 3 m beyond the fence, standing on the ground, none in the way in through the gate, and its top
    // within the far plane.
    for plant in plants() {
        let (reach, height) = plant.size();
        let at = Vec2::new(plant.at.x, plant.at.z);
        assert!(!layout::FENCE.grown(3.0 + reach).contains(at), "{plant:?}");
        assert!((plant.at.y - super::ground(at)).abs() < 1e-4);
        assert!(!(at.y > 4.0 && at.x.abs() < 6.0 + reach && at.distance(BACKDROP_CENTRE) < 26.0));
        assert!(clear(at, reach, height), "{plant:?}");
        for eye in &eyes {
            assert!(
                eye.distance(plant.at + Vec3::Y * height) < FAR - 2.0,
                "{plant:?}"
            );
        }
        for walkable in layout::WALKABLE {
            let solid = plant.solid();
            let (low, high) = (solid.low, solid.high);
            assert!(!walkable.overlaps(&layout::Footprint::new(
                [low[0], low[2]],
                [high[0], high[2]]
            )));
        }
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
    assert_eq!(views.len(), 9);
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
    let within = Vec3::from_array(views[4].0).lerp(Vec3::from_array(views[4].1), 0.75);
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
        assert!(pair[1].y > 0.05);
    }
    // Its banks rise out of it either side within its water's spread.
    for point in &RIVER[2..] {
        let (nearest, distance) = layout::nearest_river(Vec2::new(point.x, point.z));
        assert!(distance < 1e-4 && nearest.distance(*point) < 1e-4);
    }
    // It runs round the yard, never nearer the fence than 7.5 m.
    for point in river_line(8) {
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
    for point in water.iter().filter(|point| point.kind == SURFACE) {
        assert!(point.position.y <= POOL.y + 0.03 && point.position.y > 0.05);
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
        // Its model is the same each time it is built, and stands about its origin within its size.
        let model = kind.model();
        let again = kind.model();
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
    // Under the budget, about 18 thousand triangles, which the scenery draws in eight draws: the foothills, one for
    // each kind of plant with all its instances, the water and the ranges.
    let triangles = Triangles::counted();
    assert_eq!(
        triangles,
        Triangles {
            foothills: 2880,
            plants: 14_022,
            ranges: 1184,
            water: 124,
        },
        "the counts the documentation gives"
    );
    assert_eq!(triangles.total(), 18_210);
    assert!(triangles.total() < BUDGET);
    // Each kind of plant is a few dozen to a little over a hundred triangles, drawn once for all its instances.
    for (kind, count) in [
        (Kind::Canopy, 18),
        (Kind::Podocarp, 112),
        (Kind::Fern, 79),
        (Kind::CabbageTree, 145),
        (Kind::Flax, 39),
    ] {
        let model = kind.model();
        assert_eq!(
            model.meshes[0].primitives[0].indices.len() / 3,
            count,
            "{kind:?}"
        );
    }
    assert_eq!(Kind::ALL.len(), 5);
}
