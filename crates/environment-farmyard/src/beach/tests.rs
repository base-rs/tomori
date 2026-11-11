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
}

#[test]
fn the_shingle_is_drawn_above_the_ground_everywhere_and_slopes_gently() {
    let (corners, triangles) = surface();
    assert_eq!(corners.len(), COLUMNS * ROWS);
    assert_eq!(triangles.len(), (COLUMNS - 1) * (ROWS - 1) * 2);
    for step in 0..=40 {
        for across in 0..=12 {
            let out = NEAR + (FAR - NEAR) * step as f32 / 40.0;
            let along = -HALF_WIDTH + 2.0 * HALF_WIDTH * across as f32 / 12.0;
            let point = at(out, along) + Vec2::splat(1e-4) * (step % 2) as f32;
            let Some(shingle) = height(point) else {
                continue;
            };
            let drawn = backdrop::drawn_ground(point).unwrap_or(backdrop::MEADOW);
            assert!(
                shingle > drawn + 0.002,
                "{point}: shingle {shingle}, ground {drawn}"
            );
        }
    }
    // No steeper than a walk down a bank, so that stones lie where they are put.
    for triangle in &triangles {
        let [a, b, c] = triangle.map(|corner| corners[corner as usize]);
        let normal = (b - a).cross(c - a).normalize();
        assert!(normal.y.abs() > 0.88, "{normal}");
    }
}

#[test]
fn participants_walk_the_flat_shingle_where_the_backdrop_is_in_view_short_of_the_river_clear_of_its_stones()
 {
    let ground = ground();
    let (_, all) = surface();
    // A fifth of it or so, its flat shingle beside the trail, where the whole backdrop is within the far plane.
    let walked = ground.len();
    assert!(walked > all.len() / 6 && walked < all.len() / 2, "{walked}");
    let spread = layout::RIVER_HALF_WIDTH + backdrop::SPREAD;
    let lowest = ground
        .iter()
        .flatten()
        .map(|corner| corner.y)
        .fold(f32::INFINITY, f32::min);
    assert!((0.0..0.02).contains(&lowest), "{lowest}");
    for corner in ground.iter().flatten() {
        assert!(
            layout::nearest_river(Vec2::new(corner.x, corner.z)).1 > spread,
            "{corner}"
        );
        assert!(corner.y > water_level());
    }
    assert_eq!(solids().len(), boulders().len());
}

#[test]
fn every_stone_is_a_flattish_hull_a_hand_s_breadth_to_two_across_their_masses_within_ten_times() {
    let volumes: Vec<f32> = (0..STONES)
        .map(|index| {
            let (width, height) = size(index);
            assert!((0.11..=0.23).contains(&width), "{width}");
            assert!(
                height > width / 3.0 && height < width * 0.55,
                "{width} {height}"
            );
            let points = stone(index);
            assert_eq!(points.len(), STONE_POINTS);
            assert!(points.len() <= tomori_physics::HULL_POINTS);
            Body::convex(index as u32, points).check().unwrap();
            width * width * height
        })
        .collect();
    let most = volumes.iter().copied().fold(0.0, f32::max);
    let least = volumes.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(most / least <= 10.0, "{most} {least}");
    // Each drawn from all of its points, every face turned outward.
    let triangles = stone_triangles();
    assert_eq!(triangles.len(), 24);
    let mut used = [false; STONE_POINTS];
    for index in 0..STONES {
        let points = stone(index);
        for [a, b, c] in &triangles {
            used[*a] = true;
            used[*b] = true;
            used[*c] = true;
            let normal = (points[*b] - points[*a]).cross(points[*c] - points[*a]);
            let middle = (points[*a] + points[*b] + points[*c]) / 3.0;
            assert!(
                normal.dot(middle) > 0.0,
                "stone {index}, triangle {a} {b} {c}"
            );
        }
    }
    assert!(used.iter().all(|used| *used));
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
        assert!(
            home.position.y > shingle + tall * 0.5 && home.position.y < shingle + tall,
            "{index}"
        );
        for (other, there) in homes.iter().enumerate().skip(index + 1) {
            let apart = Vec2::new(there.position.x, there.position.z).distance(point);
            assert!(apart > (width + size(other).0) * 0.5, "{index} and {other}");
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
fn what_the_stones_rest_on_and_the_water_are_within_the_world_s_bounds() {
    let colliders = colliders();
    let triangles: usize = colliders.iter().map(Static::triangles).sum();
    assert!(triangles <= tomori_physics::STATIC_TRIANGLES, "{triangles}");
    assert!(colliders.len() >= 2);
    let waters = waters();
    assert_eq!(waters.len(), 5);
    // The river beside the beach is water that takes back what rests in it; the beach above it is not.
    let level = water_level();
    let river = at(FAR + 1.0, 0.0);
    let point = Vec3::new(river.x, level - 0.05, river.y);
    assert!(
        waters
            .iter()
            .any(|water| water.contains(point) && water.returns_after == Some(RETURNS))
    );
    let beach = at(NEAR + 0.5, 0.0);
    let point = Vec3::new(beach.x, height(beach).unwrap() + 0.05, beach.y);
    assert!(!waters.iter().any(|water| water.contains(point)));
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
    assert_eq!(stones.nodes.len(), STONES);
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
