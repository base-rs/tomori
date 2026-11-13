// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Large primitives split into chunks across the ground: every triangle kept, once, exactly as it was, so that the
//! chunks meet without a seam; each chunk within its cell; small parts kept whole, and closed; and what fits in one
//! cell, is skinned or is given no size, kept whole.
use std::collections::HashMap;

use glam::{Vec2, Vec4};

use super::*;
use crate::Mesh;
use crate::Node;
use crate::Transform;
use crate::bounds::tests::cuboid;

/// A rolling ground `across` by `deep` quads, each `step` across, from the origin, each quad with corners of its own,
/// its own normal, colour and texture coordinates, as a faceted scenery's ground is drawn.
fn ground(across: u32, deep: u32, step: f32) -> Primitive {
    let height = |x: f32, z: f32| (x * 0.3).sin() * 0.5 + (z * 0.2).cos() * 0.4;
    let mut primitive = Primitive::default();
    for row in 0..deep {
        for column in 0..across {
            let corner = |dx: u32, dz: u32| {
                let (x, z) = ((column + dx) as f32 * step, (row + dz) as f32 * step);
                Vec3::new(x, height(x, z), z)
            };
            let corners = [corner(0, 0), corner(0, 1), corner(1, 1), corner(1, 0)];
            let normal = (corners[1] - corners[0])
                .cross(corners[2] - corners[0])
                .normalize();
            let start = primitive.positions.len() as u32;
            primitive.positions.extend(corners);
            primitive.normals.extend([normal; 4]);
            let shade = ((row * across + column) % 7) as f32 / 7.0;
            primitive
                .colors
                .extend([Vec4::new(0.3 + shade * 0.1, 0.4, 0.2, 1.0); 4]);
            primitive
                .uvs
                .extend(corners.map(|corner| Vec2::new(corner.x, corner.z)));
            primitive
                .indices
                .extend([start, start + 1, start + 2, start, start + 2, start + 3]);
        }
    }
    primitive.material = Some(0);
    primitive
}

/// `primitive` with `other`'s triangles after its own, `other`'s vertices facing up, white, at no texture coordinate.
fn with(mut primitive: Primitive, other: &Primitive) -> Primitive {
    let offset = primitive.positions.len() as u32;
    primitive.positions.extend(&other.positions);
    let count = other.positions.len();
    primitive
        .normals
        .extend(std::iter::repeat_n(Vec3::Y, count));
    primitive
        .colors
        .extend(std::iter::repeat_n(Vec4::ONE, count));
    primitive.uvs.extend(std::iter::repeat_n(Vec2::ZERO, count));
    primitive
        .indices
        .extend(other.indices.iter().map(|index| index + offset));
    primitive
}

/// A corner with every attribute it has, to the bit, its absent ones as nothing.
type Corner = [u32; 12];

/// Each triangle of `primitive`, its corners in its order.
fn triangles(primitive: &Primitive) -> Vec<[Corner; 3]> {
    let corner = |vertex: usize| {
        let mut corner = [0; 12];
        let values = primitive.positions[vertex]
            .to_array()
            .into_iter()
            .chain(
                primitive
                    .normals
                    .get(vertex)
                    .map_or([0.0; 3], |n| n.to_array()),
            )
            .chain(
                primitive
                    .colors
                    .get(vertex)
                    .map_or([0.0; 4], |c| c.to_array()),
            )
            .chain(
                primitive
                    .uvs
                    .get(vertex)
                    .map_or([0.0; 2], |uv| uv.to_array()),
            );
        for (slot, value) in corner.iter_mut().zip(values) {
            *slot = value.to_bits();
        }
        corner
    };
    primitive
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| triangle.map(|vertex| corner(vertex as usize)))
        .collect()
}

/// How many triangles have each edge, by where its corners are, to the bit, either way along it.
fn edges<'a>(
    triangles: impl IntoIterator<Item = &'a [Corner; 3]>,
) -> HashMap<[[u32; 3]; 2], usize> {
    let mut edges = HashMap::new();
    for triangle in triangles {
        for index in 0..3 {
            let [from, to] = [triangle[index], triangle[(index + 1) % 3]]
                .map(|corner| [corner[0], corner[1], corner[2]]);
            *edges
                .entry(if from < to { [from, to] } else { [to, from] })
                .or_default() += 1;
        }
    }
    edges
}

#[test]
fn a_large_primitive_is_split_into_chunks_that_together_draw_every_triangle_as_it_was() {
    // A ground 40 m across, and posts 0.2 m across standing across the lines between its cells, which are 8 m apart.
    let posts: Vec<Vec3> = [(8.0, 3.0), (16.05, 16.0), (31.9, 24.0), (20.0, 39.8)]
        .map(|(x, z)| Vec3::new(x, 0.0, z))
        .to_vec();
    let primitive = posts.iter().fold(ground(40, 40, 1.0), |primitive, post| {
        with(
            primitive,
            &cuboid(
                *post - Vec3::new(0.1, 0.0, 0.1),
                *post + Vec3::new(0.1, 1.2, 0.1),
                true,
            ),
        )
    });
    // And a rail 30 m long, closed, wider than a cell.
    let rail = cuboid(Vec3::new(4.0, 0.8, 20.0), Vec3::new(34.0, 0.9, 20.1), true);
    let primitive = with(primitive, &rail);
    let chunks = primitive.chunks(8.0);
    assert_eq!(chunks.len(), 25 + 1);
    // The rail a chunk of its own, after the cells', still closed.
    let last = chunks.last().unwrap();
    assert_eq!(last.indices.len(), rail.indices.len());
    assert!(last.closed());
    // Every triangle once, each corner as it was to the bit, in the order of the primitive within each chunk.
    let mut before = triangles(&primitive);
    let mut after: Vec<[Corner; 3]> = chunks.iter().flat_map(triangles).collect();
    assert_eq!(after.len(), before.len());
    for chunk in &chunks {
        let order: Vec<usize> = triangles(chunk)
            .iter()
            .map(|triangle| before.iter().position(|other| other == triangle).unwrap())
            .collect();
        assert!(order.is_sorted(), "{order:?}");
        assert_eq!(chunk.material, primitive.material);
        assert!(
            chunk
                .indices
                .iter()
                .all(|index| (*index as usize) < chunk.positions.len())
        );
        for attribute in [chunk.normals.len(), chunk.colors.len(), chunk.uvs.len()] {
            assert_eq!(attribute, chunk.positions.len());
        }
        assert!(chunk.joints.is_empty() && chunk.weights.is_empty());
        // Within a cell, 8 m across, and the quad or post beyond its line that its middle is within it.
        // Within its cell, 8 m across, and the quad or post beyond its line whose middle is within it, but the rail.
        let (low, high) = chunk.bounds();
        let extent = high - low;
        assert!(
            chunk == last || extent.x <= 9.0 && extent.z <= 9.0,
            "{low} {high}"
        );
    }
    before.sort();
    after.sort();
    assert_eq!(after, before);
    // So every edge where two triangles meet in different chunks is where it was, to the bit, as many times: no seam,
    // and no edge open that was not.
    let seams = chunks
        .iter()
        .enumerate()
        .flat_map(|(index, chunk)| {
            edges(&triangles(chunk))
                .into_keys()
                .map(move |edge| (edge, index))
        })
        .fold(
            HashMap::<_, Vec<usize>>::new(),
            |mut seams, (edge, chunk)| {
                seams.entry(edge).or_default().push(chunk);
                seams
            },
        )
        .into_values()
        .filter(|chunks| chunks.len() > 1)
        .count();
    assert!(seams >= 4 * 40 * 2, "{seams}");
    assert_eq!(edges(&after), edges(&before));
    // Each post whole, and still closed, in the chunk its middle is in, its line cut through by none.
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk
                .closed_triangles()
                .iter()
                .filter(|closed| **closed)
                .count())
            .sum::<usize>(),
        (posts.len() + 1) * 12
    );
    for chunk in &chunks {
        let closed = chunk
            .closed_triangles()
            .iter()
            .filter(|closed| **closed)
            .count();
        assert_eq!(closed % 12, 0);
    }
}

#[test]
fn what_fits_in_a_chunk_is_skinned_or_has_no_size_is_kept_whole() {
    let small = ground(4, 4, 1.0);
    assert_eq!(small.chunks(8.0), vec![small.clone()]);
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(ground(40, 40, 1.0).chunks(size).len(), 1, "{size}");
    }
    // Skinned, it is kept whole, as where it is drawn is its joints'.
    let arm = crate::pose::tests::arm();
    let skinned = &arm.meshes[0].primitives[0];
    assert_eq!(skinned.chunks(0.5), vec![skinned.clone()]);
    // A strip 20 m long and 2 m deep is cut along its length alone, into three cells of 6.7 m, each chunk as deep as
    // the strip, without the texture coordinates it has none of.
    let mut strip = ground(20, 2, 1.0);
    strip.uvs.clear();
    let chunks = strip.chunks(8.0);
    assert_eq!(chunks.len(), 3);
    for chunk in &chunks {
        let (low, high) = chunk.bounds();
        assert!(
            high.x - low.x <= 8.0 + 1e-4 && high.z - low.z == 2.0,
            "{low} {high}"
        );
        assert!(chunk.uvs.is_empty() && chunk.normals.len() == chunk.positions.len());
    }
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.indices.len())
            .sum::<usize>(),
        strip.indices.len()
    );
}

#[test]
fn a_model_s_meshes_are_chunked_but_those_a_skinned_node_carries() {
    // The arm's skinned node carries a ground of its own, and a node without a skin a ground too.
    let mut model = crate::pose::tests::arm();
    model.meshes[0].primitives = vec![ground(40, 40, 1.0)];
    model.meshes.push(Mesh::new(
        "ground",
        vec![ground(40, 40, 1.0), ground(2, 2, 1.0)],
    ));
    model
        .nodes
        .push(Node::new("ground", Transform::IDENTITY).with_mesh(1));
    let chunked = model.chunked(8.0);
    assert_eq!(chunked.meshes[0], model.meshes[0]);
    assert_eq!(chunked.meshes[1].primitives.len(), 25 + 1);
    assert_eq!(
        chunked.meshes[1].primitives[25],
        model.meshes[1].primitives[1]
    );
    assert_eq!(chunked.nodes, model.nodes);
    assert_eq!(chunked.skins, model.skins);
}
