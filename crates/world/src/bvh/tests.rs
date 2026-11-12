// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hierarchy finds exactly what testing every box finds, however its boxes are spread or piled up.
use glam::Mat4;

use super::*;
use crate::view::tests::both;

/// A point of a fixed, scattered sequence in `[0, 1)`.
pub(crate) fn scatter(index: u32) -> f32 {
    let mixed = index.wrapping_mul(2_654_435_761) ^ (index >> 7).wrapping_mul(40_503);
    (mixed % 100_000) as f32 / 100_000.0
}

/// `count` boxes scattered over a yard 80 m across, of every size from a pebble to a hill.
pub(crate) fn scattered(count: u32, seed: u32) -> Vec<Aabb> {
    (0..count)
        .map(|index| {
            let at = |offset: u32| scatter(seed + index * 7 + offset);
            let middle = Vec3::new(at(1) * 80.0 - 40.0, at(2) * 6.0, at(3) * 80.0 - 40.0);
            let half = Vec3::new(at(4), at(5), at(6)) * at(7).powi(4) * 20.0 + 0.01;
            Aabb::new(middle - half, middle + half)
        })
        .collect()
}

fn leaves(boxes: &[Aabb]) -> Vec<Leaf> {
    boxes
        .iter()
        .enumerate()
        .map(|(index, bounds)| Leaf {
            bounds: *bounds,
            slot: index as u32,
            instance: index as u32 * 3,
        })
        .collect()
}

/// Which of `boxes` `bvh` finds `frustum` sees, by index.
fn found(bvh: &mut Bvh, frustum: &Frustum, count: usize) -> Vec<bool> {
    let mut seen = vec![false; count];
    bvh.walk(frustum, |leaf| {
        assert_eq!(leaf.instance, leaf.slot * 3);
        assert!(!seen[leaf.slot as usize], "a leaf is found once");
        seen[leaf.slot as usize] = true;
    });
    seen
}

/// Every eye's view the tests look from: about the yard, at the ground and the sky, near and far, turned all ways.
fn frustums() -> Vec<Frustum> {
    let mut frustums: Vec<Frustum> = (0..24)
        .map(|step| {
            let at = |offset: u32| scatter(5_000 + step * 11 + offset);
            let middle = Vec3::new(at(1) * 60.0 - 30.0, 1.6 + at(2) * 4.0, at(3) * 60.0 - 30.0);
            Frustum::enclosing(&both(middle, at(4) * std::f32::consts::TAU))
        })
        .collect();
    // From high above, looking down on everything, and a frustum that holds everything.
    frustums.push(Frustum::new(
        glam::camera::rh::proj::directx::perspective(1.5, 1.0, 0.05, 2_000.0)
            * glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 300.0, 0.0), Vec3::ZERO, Vec3::Z),
    ));
    frustums.push(Frustum::enclosing(&[]));
    frustums
}

#[test]
fn the_hierarchy_finds_exactly_what_testing_each_box_finds() {
    let mut bvh = Bvh::default();
    for (count, seed) in [(1, 0), (4, 10), (5, 20), (37, 30), (600, 40), (2_000, 50)] {
        let boxes = scattered(count, seed * 1_000);
        bvh.build(leaves(&boxes));
        assert_eq!(bvh.len(), boxes.len());
        let mut any = 0;
        for frustum in frustums() {
            let brute: Vec<bool> = boxes.iter().map(|bounds| frustum.sees(bounds)).collect();
            assert_eq!(
                found(&mut bvh, &frustum, boxes.len()),
                brute,
                "{count} boxes"
            );
            any += brute.iter().filter(|seen| **seen).count();
        }
        assert!(any > 0);
    }
}

#[test]
fn boxes_piled_up_or_in_a_line_are_split_at_their_median() {
    let mut bvh = Bvh::default();
    // Twenty boxes all in one place, which no bin divides.
    let piled = vec![Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5)); 20];
    bvh.build(leaves(&piled));
    assert!(bvh.nodes.len() > 1);
    let frustum = Frustum::enclosing(&both(Vec3::new(0.0, 0.0, 3.0), 0.0));
    assert!(
        found(&mut bvh, &frustum, piled.len())
            .iter()
            .all(|seen| *seen)
    );
    let away = Frustum::enclosing(&both(Vec3::new(0.0, 0.0, 3.0), std::f32::consts::PI));
    assert!(found(&mut bvh, &away, piled.len()).iter().all(|seen| !seen));
    // A line of boxes, split by surface area no deeper than twice, then at the median, finds the same.
    let line: Vec<Aabb> = (0..300)
        .map(|index| {
            let x = (index as f32).powi(2) * 0.01 - 40.0;
            Aabb::new(Vec3::new(x, 0.0, -1.0), Vec3::new(x + 0.05, 0.1, -0.9))
        })
        .collect();
    let mut shallow = Bvh::default();
    shallow.build_within(leaves(&line), 2);
    bvh.build(leaves(&line));
    for frustum in frustums() {
        let brute: Vec<bool> = line.iter().map(|bounds| frustum.sees(bounds)).collect();
        assert_eq!(found(&mut shallow, &frustum, line.len()), brute);
        assert_eq!(found(&mut bvh, &frustum, line.len()), brute);
    }
}

#[test]
fn an_empty_hierarchy_finds_nothing_and_one_built_again_forgets_what_it_held() {
    let mut bvh = Bvh::default();
    let frustum = Frustum::enclosing(&[]);
    assert!(found(&mut bvh, &frustum, 0).is_empty());
    bvh.build(leaves(&scattered(50, 3)));
    bvh.build([]);
    assert_eq!(bvh.len(), 0);
    assert!(found(&mut bvh, &frustum, 0).is_empty());
    assert_eq!(area(&Aabb::EMPTY), 0.0);
    // Moved into a scenery's coordinates, it finds what the moved frustum sees.
    let boxes = scattered(200, 77);
    bvh.build(leaves(&boxes));
    let carried = Frustum::enclosing(&both(Vec3::new(2.0, 1.6, 0.0), 1.0))
        .transformed(Mat4::from_rotation_y(0.3));
    let brute: Vec<bool> = boxes.iter().map(|bounds| carried.sees(bounds)).collect();
    assert_eq!(found(&mut bvh, &carried, boxes.len()), brute);
}
