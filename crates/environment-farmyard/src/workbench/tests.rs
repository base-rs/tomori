// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// The workbench, built alone, and its lantern's glass.
fn built() -> (tomori_assets::Primitive, tomori_assets::Primitive) {
    let mut shapes = Shapes::new();
    let mut glass = Shapes::new();
    build(&mut shapes, &mut glass);
    (shapes.primitive(0), glass.primitive(1))
}

#[test]
fn the_workbench_is_modest_for_the_quest_and_drawn_within_its_solids() {
    let (wood, glass) = built();
    // Under a thousand triangles, with its lantern's glass, which glows.
    let triangles = (wood.indices.len() + glass.indices.len()) / 3;
    assert_eq!(triangles, 943, "the count the documentation gives");
    assert_eq!(glass.indices.len() / 3, 12);
    // Everything it draws stands within its solids, but the lantern, which hangs on its hook just before the rack.
    let solids = &layout::solids()[6..];
    let lantern = layout::workbench_lantern();
    for position in &wood.positions {
        let within = solids.iter().any(|(low, high)| {
            (0..3).all(|axis| {
                low[axis] - 0.01 <= position[axis] && position[axis] <= high[axis] + 0.01
            })
        });
        let hung = position.distance(lantern) < 0.32;
        assert!(within || hung, "{position}");
        assert!(position.y >= -1e-4 && position.y <= layout::RACK_HEIGHT + 1e-4);
    }
    // Its top at a workbench's height, nothing on it higher than its clutter, but the lantern hanging above it, and its
    // rack as tall as its posts.
    let on_top = |position: &&Vec3| {
        let at = Vec2::new(position.x, position.z);
        at.distance(layout::WORKBENCH) < layout::WORKBENCH_TOP[1] + 0.01
            && position.distance(lantern) >= 0.32
    };
    let highest = wood
        .positions
        .iter()
        .filter(on_top)
        .map(|position| position.y)
        .fold(0.0, f32::max);
    assert!(highest <= layout::WORKBENCH_CLUTTER + 1e-4, "{highest}");
    assert!(highest > layout::WORKBENCH_HEIGHT);
    let tallest = wood
        .positions
        .iter()
        .map(|position| position.y)
        .fold(0.0, f32::max);
    assert!((tallest - layout::RACK_HEIGHT).abs() < 1e-4);
    // Its glass in the lantern.
    let (low, high) = glass.bounds();
    assert!(low.distance(lantern - Vec3::new(0.065, 0.11, 0.065)) < 1e-4);
    assert!(high.distance(lantern + Vec3::new(0.065, 0.11, 0.065)) < 1e-4);
    // Words about what is near it name the middle of its top, before its rack.
    let middle = middle();
    assert!((middle.distance(layout::WORKBENCH) - 1.975).abs() < 1e-4);
}

#[test]
fn its_faces_face_out_of_the_bench_its_top_up() {
    let (wood, _) = built();
    // The first pieces drawn are the top's: their upper faces face up, their lower down, and the rest outwards from the
    // bench's middle or along its curve.
    let tops: Vec<Vec3> = wood.normals[..4].to_vec();
    assert!(tops.iter().all(|normal| normal.y > 0.99), "{tops:?}");
    let bottoms: Vec<Vec3> = wood.normals[4..8].to_vec();
    assert!(bottoms.iter().all(|normal| normal.y < -0.99), "{bottoms:?}");
    let inner = wood.normals[8];
    let middle = Vec3::new(layout::WORKBENCH.x, 0.0, layout::WORKBENCH.y);
    assert!(inner.dot(wood.positions[8] - middle) < 0.0);
    let outer = wood.normals[12];
    assert!(outer.dot(wood.positions[12] - middle) > 0.0);
}
