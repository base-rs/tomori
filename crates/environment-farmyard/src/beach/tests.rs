// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn the_beach_lies_beside_the_riverbank_viewpoint_from_the_trail_s_edge_into_the_water() {
    let riverbank = trail::places()
        .into_iter()
        .find(|place| place.name == "riverbank")
        .unwrap();
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    assert!(Vec2::new(riverbank.at.x, riverbank.at.z).distance(middle) < 1.5);
    // Its top meets the trail's edge, and its foot is in the river's water.
    assert!((trail::from_middle(at(NEAR, 0.0)) - trail::HALF).abs() < 0.05);
    let level = water_level();
    assert!(height(at(NEAR + 0.01, 0.0)).unwrap() > level + 0.4);
    assert!(height(at(FAR - 0.01, 0.0)).unwrap() < level);
    assert_eq!(outline().len(), 4);
    assert!(tomori_physics::body::within(&outline(), middle));
    assert_eq!(height(at(FAR + 0.5, 0.0)), None);
    // How far from it a point is, within it less than none, as the ferns keep off it.
    assert!((from_beach(middle) + HALF_WIDTH).abs() < 1e-4);
    assert!((from_beach(at(FAR + 0.5, 0.0)) - 0.5).abs() < 1e-4);
    assert!((from_beach(at(FAR + 0.3, HALF_WIDTH + 0.4)) - 0.5).abs() < 1e-4);
}

/// The points of the beach to sample, a grid over it, a little off its faces' edges.
fn sampled() -> Vec<Vec2> {
    (0..=40)
        .flat_map(|step| {
            (0..=12).map(move |across| {
                let out = NEAR + (FAR - NEAR) * step as f32 / 40.0;
                let along = -HALF_WIDTH + 2.0 * HALF_WIDTH * across as f32 / 12.0;
                at(out, along) + Vec2::splat(1e-3) * (1 + step % 3) as f32
            })
        })
        .filter(|point| from_beach(*point) < -1e-3)
        .collect()
}

/// The triangles of `model` that face up, each as drawn.
fn facing_up(model: &Model) -> Vec<[Vec3; 3]> {
    model
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.primitives)
        .flat_map(|primitive| {
            primitive.indices.chunks(3).filter_map(|triangle| {
                let corners =
                    [0, 1, 2].map(|corner| primitive.positions[triangle[corner] as usize]);
                let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                (normal.normalize_or_zero().y > 0.5).then_some(corners)
            })
        })
        .collect()
}

#[test]
fn the_beach_is_the_ground_s_own_faces_shaded_as_shingle_with_nothing_laid_over_them() {
    // Its faces cover the beach's outline, each once, at the height the ground is drawn there.
    let area: f32 = ground_faces()
        .iter()
        .flat_map(|face| shapes::fan(face))
        .map(|[a, b, c]| {
            (Vec2::new(b.x - a.x, b.z - a.z))
                .perp_dot(Vec2::new(c.x - a.x, c.z - a.z))
                .abs()
                * 0.5
        })
        .sum();
    let outline_area = (FAR - NEAR) * 2.0 * HALF_WIDTH;
    assert!((area - outline_area).abs() < 1e-2, "{area} {outline_area}");
    for point in sampled() {
        let drawn = backdrop::drawn_ground(point).unwrap_or(backdrop::MEADOW);
        let beach = height(point).unwrap();
        assert!((beach - drawn).abs() < 1e-4, "{point}: {beach} {drawn}");
    }
    assert_eq!(height(at(FAR + 0.05, 0.0)), None);
    // The meadow and the foothills draw it, once at every point, the meadow's part of it and the foothills' in the
    // beach's own shingle, a stony grey-brown, and nothing else of the yard or the backdrop is drawn just above it.
    let ground: Vec<[Vec3; 3]> = facing_up(&crate::props::model())
        .into_iter()
        .chain(facing_up(&backdrop::model()))
        .collect();
    for point in sampled() {
        let beach = height(point).unwrap();
        let over: Vec<f32> = ground
            .iter()
            .filter_map(|triangle| on(triangle, point))
            .filter(|drawn| (drawn - beach).abs() < 0.05)
            .collect();
        assert_eq!(over.len(), 1, "{point}: {over:?}");
        assert!((over[0] - beach).abs() < 1e-4, "{point}");
    }
    let shades: Vec<Vec3> = (0..200).map(shingle).collect();
    for shade in &shades {
        assert!(
            shade.x > shade.z && shade.x < 0.3 && shade.z > 0.1,
            "{shade}"
        );
    }
    assert!(shades.windows(2).any(|pair| pair[0] != pair[1]));
    // The beach's own model is only its pebbles, a few centimetres across, each on the ground, its foot within it: no
    // carpet of shingle over the ground.
    let pebbles = pebbles();
    assert!(pebbles.len() > PEBBLES * 3 / 4, "{}", pebbles.len());
    let drawn = model();
    let primitive = &drawn.meshes[0].primitives[0];
    assert_eq!(primitive.indices.len() / 3, pebbles.len() * PEBBLE_SIDES);
    for position in &primitive.positions {
        let (middle, width, _) = pebbles
            .iter()
            .min_by(|a, b| a.0.distance(*position).total_cmp(&b.0.distance(*position)))
            .unwrap();
        assert!(middle.distance(*position) < *width, "{position}");
        let ground = height(Vec2::new(position.x, position.z)).unwrap();
        assert!(position.y >= ground - PEBBLE_SUNK - 1e-4, "{position}");
        assert!(position.y < ground + width * 0.5, "{position}");
    }
    let flat: f32 = facing_up(&drawn)
        .iter()
        .filter(|[a, b, c]| (*b - *a).cross(*c - *a).normalize().y > 0.95)
        .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
        .sum();
    assert!(flat < 0.01, "{flat}");
}

#[test]
fn participants_walk_from_the_trail_down_the_bank_to_the_water_s_edge_and_into_its_shallows() {
    let ground = ground();
    let level = water_level();
    let steepest = tomori_environment::navmesh::STEEPEST.cos();
    // All of it the ground as drawn, within the beach beyond the trail's edge, none steeper than Tomori walks, and
    // into the river no deeper than its shallows.
    for triangle in &ground {
        let [a, b, c] = *triangle;
        let normal = (b - a).cross(c - a).normalize();
        assert!(normal.y > steepest, "{a} {b} {c}");
        for corner in triangle {
            let point = Vec2::new(corner.x, corner.z);
            assert!(from_beach(point) < 1e-3, "{corner}");
            assert!(trail::from_middle(point) > trail::FOOT, "{corner}");
            assert!(corner.y >= level - SHALLOWS - 1e-4, "{corner}");
            assert!((corner.y - height(point).unwrap()).abs() < 1e-4, "{corner}");
        }
    }
    let lowest = ground
        .iter()
        .flatten()
        .map(|corner| corner.y)
        .fold(f32::INFINITY, f32::min);
    assert!(lowest < level - SHALLOWS + 0.01, "{lowest}");
    // Walkable from beside the trail all the way down to the water's edge and a step into it, but for the river's
    // stones on it, at the height drawn, along a line down the bank clear of them; and not where the river is deeper.
    let clear = [-1.2, -0.6, 0.0, 0.6, 1.2]
        .into_iter()
        .filter(|along| {
            (0..=40).all(|step| {
                let point = at(
                    WALKED + 0.05 + (FAR - WALKED - 0.05) * step as f32 / 40.0,
                    *along,
                );
                height(point).is_none_or(|ground| ground < level - SHALLOWS + 0.005)
                    || crate::walkable::height_at(point)
                        .is_some_and(|walked| (walked - height(point).unwrap()).abs() < 1e-3)
            })
        })
        .count();
    assert!(clear >= 3, "{clear}");
    let edge = (0..=500)
        .map(|step| NEAR + (FAR - NEAR) * step as f32 / 500.0)
        .find(|out| height(at(*out, 0.0)).unwrap() < level)
        .unwrap();
    assert!(crate::walkable::height_at(at(edge + 0.05, 0.0)).is_some());
    let deep = (0..=500)
        .map(|step| NEAR + (FAR - NEAR) * step as f32 / 500.0)
        .find(|out| height(at(*out, 0.0)).unwrap() < level - SHALLOWS - 0.01)
        .unwrap();
    assert_eq!(crate::walkable::height_at(at(deep, 0.0)), None);
    assert_eq!(solids().len(), boulders().len());
}

/// Stone `index`'s hull: every point on it, every face of four corners at most, its bottom one of them, flat, below
/// the rest; drawn from all of its points, as its hull's faces, every one turned outward, enclosing it; and its faces.
fn hull_checked(index: usize) -> Vec<(Vec3, Vec<usize>)> {
    let (_, height) = size(index);
    let points = stone(index);
    assert!(points.len() <= tomori_physics::HULL_POINTS);
    Body::convex(index as u32, points.clone()).check().unwrap();
    let faces = faces(&points);
    assert!(
        faces.iter().all(|(_, corners)| corners.len() <= 4),
        "{index}: {:?}",
        faces
            .iter()
            .map(|(_, corners)| corners.len())
            .collect::<Vec<_>>()
    );
    let lowest = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let bottom = faces
        .iter()
        .find(|(outward, _)| outward.y < -0.999)
        .expect("a flat bottom");
    assert_eq!(bottom.1.len(), 4, "{index}");
    assert!(
        bottom
            .1
            .iter()
            .all(|corner| (points[*corner].y - lowest).abs() < 1e-5)
    );
    let triangles = stone_triangles(index);
    assert!(
        (0..points.len()).all(|point| triangles.iter().any(|triangle| triangle.contains(&point))),
        "{index}"
    );
    for [a, b, c] in triangles {
        let normal = (points[*b] - points[*a]).cross(points[*c] - points[*a]);
        assert!(
            normal.dot(points[*a]) > 0.0,
            "stone {index}, triangle {a} {b} {c}"
        );
    }
    assert!(volume(index) > 0.0);
    assert!(outside(index, Vec3::ZERO) == 0.0);
    assert!((outside(index, Vec3::Y * (height * 0.5 + 0.1)) - 0.1).abs() < 0.05);
    faces
}

#[test]
fn the_stones_are_of_every_kind_and_size_each_a_hull_drawn_as_it_collides_its_faces_of_four_corners_at_most()
 {
    let mut kinds = std::collections::BTreeMap::new();
    let mut masses = Vec::new();
    for index in 0..RIVER {
        let (width, height) = size(index);
        let kind = kind(index);
        *kinds.entry(format!("{kind:?}")).or_insert(0) += 1;
        // Half of them big, as much as one hand holds; the rest a hand's breadth to two.
        let widths = if big(index) { 0.22..=0.37 } else { 0.115..=0.2 };
        assert!(widths.contains(&width), "{index}: {width}");
        let (least, most) = match kind {
            Kind::Slab => (0.24, 0.33),
            Kind::Cobble => (0.5, 0.65),
            Kind::Long => (0.34, 0.44),
            _ => (0.36, 0.48),
        };
        assert!(
            height >= width * least && height <= width * most,
            "{index}: {width} {height}"
        );
        hull_checked(index);
        masses.push(volume(index) * 2_650.0);
    }
    assert!(stone_triangles(STONES).is_empty());
    // Three slabs of every seven, two cobbles, a long one and an angular one; from a few hundred grams to a heavy armful.
    assert_eq!(
        kinds.into_iter().collect::<Vec<_>>(),
        [
            ("Angular".to_owned(), 4),
            ("Cobble".to_owned(), 8),
            ("Long".to_owned(), 4),
            ("Slab".to_owned(), 12)
        ]
    );
    let heaviest = masses.iter().copied().fold(0.0, f32::max);
    let lightest = masses.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(lightest > 0.2 && heaviest < 16.0, "{lightest} {heaviest}");
}

#[test]
fn the_building_stones_are_flagstones_and_blocks_flat_below_and_level_above_and_heavy() {
    let mut kinds = std::collections::BTreeMap::new();
    for index in RIVER..STONES {
        assert!(building(index) && !big(index));
        let (width, depth, height) = hewn(index);
        assert_eq!(size(index), (width, height));
        let kind = kind(index);
        *kinds.entry(format!("{kind:?}")).or_insert(0) += 1;
        // A flagstone half a metre to three quarters across and a hand's breadth thick; a block a third of a metre or
        // more each way, as high as a pillar of a dolmen.
        let ((widths, deeps), heights) = match kind {
            Kind::Flagstone => ((0.55..=0.75, 0.6..=0.75), 0.09..=0.14),
            _ => ((0.34..=0.44, 0.74..=0.88), 0.32..=0.35),
        };
        assert!(widths.contains(&width), "{index}: {width}");
        assert!(deeps.contains(&(depth / width)), "{index}: {depth}");
        assert!(heights.contains(&height), "{index}: {height}");
        // Flat and very nearly level on top, a face of four, as well as below, so that one laid on another lies level.
        let faces = hull_checked(index);
        let top = faces
            .iter()
            .find(|(outward, _)| outward.y > 0.9995)
            .expect("a level top");
        assert_eq!(top.1.len(), 4, "{index}");
        // Heavy, from 40 to 130 kg, heavier than any river stone; and held by its edge within what the side lets a hand
        // hold, its farthest point and the reach of a hand beyond it.
        let mass = volume(index) * 2_650.0;
        assert!((40.0..130.0).contains(&mass), "{index}: {mass} kg");
        let farthest = Body::convex(index as u32, stone(index)).radius();
        assert!(
            farthest + 0.05 < tomori_physics::HOLD,
            "{index}: {farthest}"
        );
        assert_eq!(stone_triangles(index).len(), 28, "{index}");
    }
    assert_eq!(
        kinds.into_iter().collect::<Vec<_>>(),
        [("Block".to_owned(), 3), ("Flagstone".to_owned(), 5)]
    );
}

#[test]
fn the_stones_lie_on_the_beach_apart_from_each_other_and_the_river_s_stones() {
    let homes = homes();
    assert_eq!(homes.len(), STONES);
    let boulders = boulders();
    for (index, home) in homes.iter().enumerate() {
        let point = Vec2::new(home.position.x, home.position.z);
        let shingle = height(point).unwrap();
        let (width, tall) = size(index);
        if index < RIVER {
            assert!(
                home.position.y > shingle + tall * 0.5 && home.position.y < shingle + tall,
                "{index}"
            );
        }
        // Each river stone apart from every other stone; the building stones as they are laid, below.
        for (other, there) in homes.iter().enumerate().skip(index + 1) {
            let apart = Vec2::new(there.position.x, there.position.z).distance(point);
            assert!(
                building(index) || apart > (width + size(other).0) * 0.5,
                "{index} and {other}"
            );
        }
        for boulder in &boulders {
            assert!(Vec2::new(boulder.at.x, boulder.at.z).distance(point) > boulder.size.x * 0.55);
        }
    }
    let bodies = bodies();
    assert_eq!(bodies.len(), STONES);
    assert_eq!(bodies[3].home, homes[3]);
    assert!(bodies.len() <= tomori_physics::BODY_LIMIT);
}

#[test]
fn the_building_stones_lie_in_a_pile_and_a_row_on_the_flat_shingle_off_the_trail() {
    let homes = homes();
    for (index, home) in homes.iter().enumerate().skip(RIVER) {
        // Every corner of it within the beach, half a metre beyond the trail's edge, over the flat shingle, at the
        // meadow's level short of where the bank begins to slope, and above it.
        for point in stone(index) {
            let point = home.transform_point(point);
            let ground = Vec2::new(point.x, point.z);
            assert!(from_beach(ground) < -0.02, "{index}: {point}");
            assert!(trail::from_middle(ground) > trail::FOOT + 0.3, "{index}");
            let flat = height(ground).unwrap();
            assert!(
                (flat - backdrop::MEADOW).abs() < 1e-5,
                "{index}: {point} {flat}"
            );
            assert!(point.y > height(ground).unwrap(), "{index}: {point}");
        }
    }
    // The flagstones in a pile, broadest at the bottom, each just above the one below it; the blocks apart in a row.
    let mut pile: Vec<usize> = (RIVER..STONES)
        .filter(|index| kind(*index) == Kind::Flagstone)
        .collect();
    pile.sort_by(|a, b| homes[*a].position.y.total_cmp(&homes[*b].position.y));
    for pair in pile.windows(2) {
        let [below, above] = [pair[0], pair[1]];
        assert!(size(below).0 >= size(above).0);
        let lowest = stone(above)
            .into_iter()
            .map(|point| homes[above].transform_point(point).y)
            .fold(f32::INFINITY, f32::min);
        let highest = stone(below)
            .into_iter()
            .map(|point| homes[below].transform_point(point).y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((0.0..0.01).contains(&(lowest - highest)), "{below} {above}");
    }
    // The blocks in a row along the beach, each its long way out from the trail, a few centimetres apart.
    let (_, way, _) = frame();
    let blocks: Vec<usize> = (RIVER..STONES)
        .filter(|index| kind(*index) == Kind::Block)
        .collect();
    assert_eq!(blocks.len(), BLOCKS_ALONG.len());
    let along = |index: usize| {
        (Vec2::new(homes[index].position.x, homes[index].position.z) - at(0.0, 0.0)).dot(way)
    };
    for (index, expected) in blocks.iter().zip(BLOCKS_ALONG) {
        assert!((along(*index) - expected).abs() < 1e-3, "{index}");
    }
    for pair in blocks.windows(2) {
        let [near, far] = [pair[0], pair[1]];
        let reach = |index: usize, sign: f32| {
            stone(index)
                .into_iter()
                .map(|point| {
                    let point = homes[index].transform_point(point);
                    (Vec2::new(point.x, point.z) - at(0.0, 0.0)).dot(way) * sign
                })
                .fold(f32::NEG_INFINITY, f32::max)
        };
        assert!(
            -reach(far, -1.0) - reach(near, 1.0) > 0.02,
            "{near} and {far}"
        );
    }
    assert!(
        pile.iter()
            .all(|index| along(*index) < BLOCKS_ALONG[0] - 0.6)
    );
}

#[test]
fn what_the_stones_rest_on_and_the_water_are_within_the_world_s_bounds() {
    let colliders = colliders();
    let triangles: usize = colliders.iter().map(Static::triangles).sum();
    assert!(triangles <= tomori_physics::STATIC_TRIANGLES, "{triangles}");
    assert!(colliders.len() >= 2);
    let waters = waters();
    // The dam reach, in parts, one to each band it crosses, then the five bands.
    let dam = waters
        .iter()
        .take_while(|water| water.returns_after.is_none())
        .count();
    assert!((1..=3).contains(&dam), "{dam}");
    assert_eq!(waters.len(), dam + 5);
    // The river beside the beach is its dam reach, which keeps what rests in it, across the channel to its far bank;
    // upstream and downstream of it, the river takes back what rests in it; and the beach above it is not water.
    let level = water_level();
    let in_river = |along: f32| {
        let out = (0..=400)
            .map(|step| NEAR + 10.0 * step as f32 / 400.0)
            .min_by(|a, b| {
                let from = |out: f32| layout::nearest_river(at(out, along)).1;
                from(*a).total_cmp(&from(*b))
            })
            .unwrap();
        let river = at(out, along);
        Vec3::new(river.x, level - 0.05, river.y)
    };
    for along in [-DAM_ALONG + 0.2, 0.0, DAM_ALONG - 0.2] {
        let middle = in_river(along);
        assert!(waters[..dam].iter().any(|water| water.contains(middle)) && stays(middle));
        let shallows = at(FAR - 0.1, along);
        assert!(stays(Vec3::new(shallows.x, level - 0.03, shallows.y)));
    }
    // Anywhere in the dam reach, at any height below the river's water there, up to its ends, a stone stays, though
    // the river is higher upstream than beside the beach; just beyond its ends, it returns.
    for along in [-DAM_ALONG + 0.02, -1.0, 0.0, 1.0, DAM_ALONG - 0.02] {
        let middle = in_river(along);
        let surface = waters[dam..]
            .iter()
            .find(|water| water.contains(middle.with_y(-10.0)))
            .unwrap()
            .height;
        for below in [0.002, 0.01, 0.05, 0.1] {
            let point = middle.with_y(surface - below);
            assert!(stays(point), "{point}");
        }
    }
    for along in [-DAM_ALONG - 0.02, DAM_ALONG + 0.02] {
        let middle = in_river(along);
        let surface = waters[dam..]
            .iter()
            .find(|water| water.contains(middle.with_y(-10.0)))
            .unwrap()
            .height;
        assert!(!stays(middle.with_y(surface - 0.002)), "{middle}");
    }
    // The far bank's water's edge is within the dam reach.
    let far = (0..=400)
        .map(|step| FAR + 4.0 * step as f32 / 400.0)
        .rfind(|out| backdrop::drawn_ground(at(*out, 0.0)).unwrap() < level)
        .unwrap();
    assert!(far < DAM_OUT - 0.3, "{far}");
    for along in [-DAM_ALONG - 1.0, DAM_ALONG + 1.0, DAM_ALONG + 2.5] {
        let river = in_river(along);
        assert!(
            waters
                .iter()
                .find(|water| water.contains(river))
                .is_some_and(|water| water.returns_after == Some(RETURNS)),
            "{river}"
        );
        assert!(!stays(river), "{river}");
    }
    let beach = at(NEAR + 0.5, 0.0);
    let point = Vec3::new(beach.x, height(beach).unwrap() + 0.05, beach.y);
    assert!(!waters.iter().any(|water| water.contains(point)) && stays(point));
    // The dam reach and the ground about it within the world's region, and the ground what is drawn.
    for corner in dam_reach() {
        assert!(corner.distance(at((NEAR + FAR) * 0.5, 0.0)) < REGION);
    }
    for triangle in drawn() {
        for corner in triangle {
            let point = Vec2::new(corner.x, corner.z);
            if from_beach(point) < -1e-3 && trail::from_middle(point) > trail::FOOT + 1e-3 {
                assert!((corner.y - height(point).unwrap()).abs() < 1e-4, "{corner}");
            }
        }
    }
    let settings = settings();
    assert_eq!(settings.lost_beyond, REGION);
    assert!(
        Vec2::new(settings.home.x, settings.home.z).distance(at((NEAR + FAR) * 0.5, 0.0)) < 1e-4
    );
}

#[test]
fn the_beach_and_its_stones_are_drawn_within_their_budget() {
    let triangles = |model: &Model| -> usize {
        model
            .meshes
            .iter()
            .flat_map(|mesh| &mesh.primitives)
            .map(|primitive| primitive.indices.len() / 3)
            .sum()
    };
    let (beach, stones) = (model(), stones_model());
    println!(
        "pebbles {}, stones {}",
        triangles(&beach),
        triangles(&stones)
    );
    assert_eq!(stones.nodes.len(), STONES);
    assert_eq!(stones.nodes[RIVER].name.as_deref(), Some("stone-28"));
    assert_eq!(
        stones.nodes[5].name.as_deref(),
        Some(stone_node(5).as_str())
    );
    let total = triangles(&beach) + triangles(&stones);
    assert!(total <= BUDGET, "{total}");
}

#[test]
fn the_ray_meets_a_stone_where_its_faces_are() {
    let placement = Placement::new(Vec3::new(1.0, 0.5, -2.0), Quat::from_rotation_y(0.6));
    let (_, height) = size(0);
    // Straight down onto its top, at its top.
    let along = met(Vec3::new(1.0, 2.0, -2.0), Vec3::NEG_Y, 0, &placement).unwrap();
    assert!(
        (along - (1.5 - height * 0.5)).abs() < height * 0.1,
        "{along}"
    );
    // Past it, or from beyond it going away, not at all.
    assert_eq!(
        met(Vec3::new(1.5, 2.0, -2.0), Vec3::NEG_Y, 0, &placement),
        None
    );
    assert_eq!(
        met(Vec3::new(1.0, -1.0, -2.0), Vec3::NEG_Y, 0, &placement),
        None
    );
    // A ray along a face's plane crosses nothing of it.
    let [a, b, c] = [Vec3::ZERO, Vec3::X, Vec3::Z];
    assert_eq!(
        crossing(Vec3::new(-1.0, 0.0, 0.2), Vec3::X, [a, b, c]),
        None
    );
    assert_eq!(
        crossing(Vec3::new(0.2, 1.0, 0.2), Vec3::NEG_Y, [a, b, c]),
        Some(1.0)
    );
    assert_eq!(
        crossing(Vec3::new(0.9, 1.0, 0.9), Vec3::NEG_Y, [a, b, c]),
        None
    );
}
