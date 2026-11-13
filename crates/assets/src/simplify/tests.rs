// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Primitives simplified: within the error asked, every vertex kept where it was, the open edges where a ground was cut
//! into chunks as they were, so that the chunks still meet; closed parts closed; where colours meet kept; and what
//! cannot be simplified without folding or tearing it left as it was.
use std::collections::{HashMap, HashSet};

use glam::{Vec2, Vec3, Vec4};

use super::*;
use crate::bounds::tests::cuboid;

/// A grid `cells` quads across each way, `step` apart, from the origin, its height at each point `height`, each quad's
/// two triangles faceted, with corners of their own, a normal of their own and `color`'s colour where their middle is,
/// as the farmyard's grounds are built.
fn faceted(
    cells: u32,
    step: f32,
    height: impl Fn(f32, f32) -> f32,
    color: impl Fn(Vec2) -> Vec4,
) -> Primitive {
    let mut primitive = Primitive::default();
    let at = |x: u32, z: u32| {
        let (x, z) = (x as f32 * step, z as f32 * step);
        Vec3::new(x, height(x, z), z)
    };
    for z in 0..cells {
        for x in 0..cells {
            for triangle in [
                [at(x, z), at(x, z + 1), at(x + 1, z + 1)],
                [at(x, z), at(x + 1, z + 1), at(x + 1, z)],
            ] {
                let [a, b, c] = triangle;
                let start = primitive.positions.len() as u32;
                let middle = (a + b + c) / 3.0;
                primitive.positions.extend(triangle);
                primitive
                    .normals
                    .extend([(b - a).cross(c - a).normalize(); 3]);
                primitive
                    .colors
                    .extend([color(Vec2::new(middle.x, middle.z)); 3]);
                primitive.indices.extend([start, start + 1, start + 2]);
            }
        }
    }
    primitive.material = Some(1);
    primitive
}

/// A smooth grid as [`faceted`], flat, each point one vertex shared by the triangles about it, its normal leaning as
/// [`leaning`] says and its texture coordinates where it is, but along `x = seam`, where those right of it have their
/// own vertices, a tenth further in the texture.
fn smooth(cells: u32, step: f32, seam: u32) -> Primitive {
    let mut primitive = Primitive::default();
    let mut vertex = |x: u32, z: u32, right: bool| {
        let position = Vec3::new(x as f32 * step, 0.0, z as f32 * step);
        let shift = if right && x == seam { 0.1 } else { 0.0 };
        primitive.positions.push(position);
        primitive.normals.push(leaning(position));
        primitive.colors.push(Vec4::ONE);
        primitive
            .uvs
            .push(Vec2::new(position.x + shift, position.z));
        primitive.positions.len() as u32 - 1
    };
    let mut points = HashMap::new();
    let mut index = |x: u32, z: u32, right: bool| {
        *points
            .entry((x, z, right && x == seam))
            .or_insert_with(|| vertex(x, z, right))
    };
    let mut indices = Vec::new();
    for z in 0..cells {
        for x in 0..cells {
            let right = x >= seam;
            let [a, b, c, d] = [
                index(x, z, right),
                index(x, z + 1, right),
                index(x + 1, z + 1, right),
                index(x + 1, z, right),
            ];
            indices.extend([a, b, c, a, c, d]);
        }
    }
    primitive.indices = indices;
    primitive
}

/// A smooth surface's normal at `position`, leaning a little across it, as a rounded surface's do.
fn leaning(position: Vec3) -> Vec3 {
    Vec3::new(
        (position.x * 1.3).sin() * 0.1,
        1.0,
        (position.z * 0.7).cos() * 0.1,
    )
    .normalize()
}

/// Each triangle's corners, by where they are, to the bit.
fn corners(primitive: &Primitive) -> Vec<[[u32; 3]; 3]> {
    primitive
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            triangle.map(|vertex| {
                primitive.positions[vertex as usize]
                    .to_array()
                    .map(f32::to_bits)
            })
        })
        .collect()
}

/// The edges crossed by one triangle alone, by where their ends are, to the bit.
fn open(primitive: &Primitive) -> HashSet<[[u32; 3]; 2]> {
    let mut edges: HashMap<[[u32; 3]; 2], usize> = HashMap::new();
    for [a, b, c] in corners(primitive) {
        for (from, to) in [(a, b), (b, c), (c, a)] {
            *edges
                .entry(if from < to { [from, to] } else { [to, from] })
                .or_default() += 1;
        }
    }
    edges
        .into_iter()
        .filter(|(_, count)| *count == 1)
        .map(|(edge, _)| edge)
        .collect()
}

/// How much area its triangles cover across the ground, each by its own sign, so that one turned over takes away.
fn ground_area(primitive: &Primitive) -> f32 {
    corners(primitive)
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|bits| Vec3::from_array(bits.map(f32::from_bits)));
            (b - a).cross(c - a).y * 0.5
        })
        .sum()
}

/// Every one of `simplified`'s positions is one of `primitive`'s, to the bit.
fn kept(primitive: &Primitive, simplified: &Primitive) -> bool {
    let positions: HashSet<[u32; 3]> = primitive
        .positions
        .iter()
        .map(|position| position.to_array().map(f32::to_bits))
        .collect();
    simplified
        .positions
        .iter()
        .all(|position| positions.contains(&position.to_array().map(f32::to_bits)))
}

#[test]
fn a_flat_ground_of_one_colour_is_drawn_in_few_triangles_its_edges_as_they_were() {
    let ground = faceted(8, 0.5, |_, _| 0.0, |_| Vec4::new(0.2, 0.4, 0.1, 1.0));
    let (simplified, strayed) = ground.simplified(0.0);
    assert_eq!(strayed, 0.0);
    // Within its edge, every point that is not a corner of it goes, but the points along its edge, which stay.
    assert_eq!(ground.indices.len() / 3, 128);
    assert!(
        simplified.indices.len() / 3 <= 32,
        "{}",
        simplified.indices.len() / 3
    );
    assert_eq!(open(&simplified), open(&ground));
    assert!((ground_area(&simplified) - ground_area(&ground)).abs() < 1e-4);
    assert!(kept(&ground, &simplified));
    // Each corner as it was: of one normal, colour and material.
    assert!(
        simplified
            .normals
            .iter()
            .all(|normal| *normal == Vec3::NEG_Y || *normal == Vec3::Y)
    );
    assert_eq!(
        simplified.colors,
        vec![Vec4::new(0.2, 0.4, 0.1, 1.0); simplified.positions.len()]
    );
    assert_eq!(simplified.material, Some(1));
    assert!(simplified.uvs.is_empty() && simplified.joints.is_empty());
}

#[test]
fn a_ground_cut_into_chunks_and_each_simplified_still_meets_without_a_seam() {
    let height = |x: f32, z: f32| (x * 0.6).sin() * 0.4 + (z * 0.5).cos() * 0.3;
    let ground = faceted(24, 0.5, height, |_| Vec4::new(0.3, 0.4, 0.2, 1.0));
    let chunks = ground.chunks(4.0);
    assert!(chunks.len() >= 9);
    let open_whole = open(&ground);
    let mut triangles = 0;
    let mut edges: HashMap<[[u32; 3]; 2], usize> = HashMap::new();
    for chunk in &chunks {
        let (simplified, strayed) = chunk.simplified(0.02);
        assert!(strayed <= 0.02);
        assert!(kept(chunk, &simplified));
        // Its own edges, where it was cut, are as they were, and nothing within it is opened.
        assert_eq!(open(&simplified), open(chunk));
        triangles += simplified.indices.len() / 3;
        for edge in open(&simplified) {
            *edges.entry(edge).or_default() += 1;
        }
    }
    // Every edge where two chunks meet is each's, and the rest the ground's own edge.
    let lone: HashSet<_> = edges
        .iter()
        .filter(|(_, count)| **count == 1)
        .map(|(edge, _)| *edge)
        .collect();
    assert_eq!(lone, open_whole);
    assert!(edges.values().all(|count| *count <= 2));
    assert!(triangles < ground.indices.len() / 3 * 3 / 4, "{triangles}");
    // Asking for more strays it further, with fewer triangles; and asking for nothing, or less than nothing, only
    // takes away what strays it not at all.
    let counts: Vec<usize> = [0.0, 0.01, 0.05, 0.2]
        .into_iter()
        .map(|error| ground.simplified(error).0.indices.len())
        .collect();
    assert!(
        counts.windows(2).all(|pair| pair[1] <= pair[0]),
        "{counts:?}"
    );
    assert_eq!(ground.simplified(-1.0).0, ground.simplified(0.0).0);
}

/// How far `point` is from the nearest of `primitive`'s triangles' planes, within each triangle's reach.
fn from_surface(primitive: &Primitive, point: Vec3) -> f32 {
    corners(primitive)
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|bits| Vec3::from_array(bits.map(f32::from_bits)));
            let normal = (b - a).cross(c - a).normalize();
            let middle = (a + b + c) / 3.0;
            let reach = a
                .distance(middle)
                .max(b.distance(middle))
                .max(c.distance(middle));
            if point.distance(middle) <= reach * 1.5 {
                normal.dot(point - a).abs()
            } else {
                f32::INFINITY
            }
        })
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn a_rolling_ground_strays_no_further_than_asked() {
    let height = |x: f32, z: f32| (x * 0.9).sin() * 0.3 + (z * 0.7).cos() * 0.25;
    let ground = faceted(16, 0.4, height, |_| Vec4::ONE);
    for error in [0.005, 0.02, 0.08] {
        let (simplified, strayed) = ground.simplified(error);
        assert!(strayed <= error && strayed > 0.0, "{error}: {strayed}");
        assert!(simplified.indices.len() < ground.indices.len());
        // Each point of the ground it no longer has is within about the error of what it drew instead.
        for position in &ground.positions {
            let off = from_surface(&simplified, *position);
            assert!(
                off <= error * 1.5 + 1e-4,
                "{error}: {position} is {off} off"
            );
        }
        assert_eq!(open(&simplified), open(&ground));
        assert!((ground_area(&simplified) - ground_area(&ground)).abs() < 1e-3);
    }
}

#[test]
fn where_faces_of_other_colours_meet_stays_where_it_was() {
    // Two colours either side of x = 2, on one flat ground: moving where they meet strays no plane, but would move
    // colours, so it stays; faces varied a little in colour, as a model's are, hold a little, and faces of one colour
    // not at all.
    let red = Vec4::new(0.6, 0.1, 0.1, 1.0);
    let green = |at: Vec2| Vec4::new(0.1, 0.5 + (at.y * 3.1).sin() * 0.03, 0.1, 1.0);
    let ground = faceted(
        8,
        0.5,
        |_, _| 0.0,
        |at| if at.x < 2.0 { red } else { green(at) },
    );
    let (simplified, _) = ground.simplified(0.01);
    let triangles = |red_side: bool| {
        simplified
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|triangle| (simplified.colors[triangle[0] as usize] == red) == red_side)
            .count()
    };
    assert!(triangles(true) <= 24, "{}", triangles(true));
    assert!(
        triangles(false) > triangles(true) * 2,
        "{}",
        triangles(false)
    );
    let area = |primitive: &Primitive, red_side: bool| {
        primitive
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|triangle| (primitive.colors[triangle[0] as usize] == red) == red_side)
            .map(|triangle| {
                let [a, b, c] = triangle.map(|vertex| primitive.positions[vertex as usize]);
                assert_eq!(
                    (a.x + b.x + c.x) / 3.0 < 2.0,
                    red_side,
                    "a triangle of the other colour's side"
                );
                (b - a).cross(c - a).y * 0.5
            })
            .sum::<f32>()
    };
    assert!((area(&simplified, true) - 8.0).abs() < 1e-4);
    assert!((area(&simplified, false) - 8.0).abs() < 1e-4);
}

#[test]
fn a_faceted_surface_s_faces_keep_their_own_place_in_a_texture_where_it_changes_across_them() {
    // Each quad's own patch of its texture, as a low-poly model painted from a palette has: where four meet, a corner
    // moved keeps its own patch where the point it moves onto has none of it.
    let mut ground = faceted(8, 0.5, |_, _| 0.0, |_| Vec4::ONE);
    ground.uvs = (0..ground.positions.len())
        .map(|vertex| Vec2::new((vertex / 6) as f32, 0.5))
        .collect();
    let (simplified, _) = ground.simplified(1.5);
    assert!(simplified.indices.len() < ground.indices.len());
    for triangle in simplified.indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|vertex| simplified.uvs[vertex as usize]);
        assert!(a == b && b == c, "{a} {b} {c}");
    }
    assert!((ground_area(&simplified) - 16.0).abs() < 1e-3);
}

#[test]
fn a_closed_part_stays_closed_and_a_box_keeps_its_corners() {
    // A faceted ball, closed: simplified, it is closed still, and every triangle faces out.
    let ball = {
        let mut primitive = Primitive::default();
        let (rings, segments) = (12, 16);
        // Each point once, to the bit: the poles, and the last segment the first's.
        let at = |ring: u32, segment: u32| {
            if ring == 0 || ring == rings {
                return Vec3::new(0.0, if ring == 0 { 1.0 } else { -1.0 }, 0.0);
            }
            let (theta, phi) = (
                ring as f32 / rings as f32 * std::f32::consts::PI,
                (segment % segments) as f32 / segments as f32 * std::f32::consts::TAU,
            );
            Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            )
        };
        for ring in 0..rings {
            for segment in 0..segments {
                let [a, b, c, d] = [
                    at(ring, segment),
                    at(ring + 1, segment),
                    at(ring + 1, segment + 1),
                    at(ring, segment + 1),
                ];
                for triangle in [[a, d, c], [a, c, b]] {
                    let [p, q, r] = triangle;
                    if p == q || q == r || r == p {
                        continue;
                    }
                    let start = primitive.positions.len() as u32;
                    primitive.positions.extend(triangle);
                    primitive
                        .normals
                        .extend([(q - p).cross(r - p).normalize(); 3]);
                    primitive.indices.extend([start, start + 1, start + 2]);
                }
            }
        }
        primitive
    };
    assert!(ball.closed_triangles().iter().all(|closed| *closed));
    let (simplified, strayed) = ball.simplified(0.2);
    assert!(strayed <= 0.2);
    assert!(simplified.indices.len() < ball.indices.len() * 3 / 4);
    assert!(simplified.closed_triangles().iter().all(|closed| *closed));
    assert!(kept(&ball, &simplified));
    for triangle in corners(&simplified) {
        let [a, b, c] = triangle.map(|bits| Vec3::from_array(bits.map(f32::from_bits)));
        assert!(
            (b - a).cross(c - a).dot(a + b + c) > 0.0,
            "a triangle facing in"
        );
    }
    // A box's corners, where three faces meet, stay at any error less than it is large.
    for faceted in [true, false] {
        let block = cuboid(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.4, 3.0, 3.6), faceted);
        assert_eq!(block.simplified(0.1).0.indices.len(), block.indices.len());
    }
}

#[test]
fn a_smooth_surface_s_seam_moves_only_along_itself_each_side_keeping_its_attributes() {
    let grid = smooth(8, 0.5, 4);
    let (simplified, _) = grid.simplified(0.001);
    assert!(simplified.indices.len() < grid.indices.len() / 2);
    assert_eq!(open(&simplified), open(&grid));
    // Each corner has the attributes some vertex where it is had, and each triangle lies on one side of the seam, its
    // texture as the side's.
    let at = |primitive: &Primitive, vertex: u32| primitive.positions[vertex as usize];
    for triangle in simplified.indices.as_chunks::<3>().0 {
        let middle = triangle
            .iter()
            .map(|vertex| at(&simplified, *vertex))
            .sum::<Vec3>()
            / 3.0;
        let right = middle.x > 2.0;
        for vertex in triangle {
            let position = at(&simplified, *vertex);
            let uv = simplified.uvs[*vertex as usize];
            let shift = if right && position.x == 2.0 { 0.1 } else { 0.0 };
            assert_eq!(uv, Vec2::new(position.x + shift, position.z));
            assert_eq!(simplified.normals[*vertex as usize], leaning(position));
        }
    }
    assert!((ground_area(&simplified) - 16.0).abs() < 1e-3);
}

#[test]
fn what_cannot_be_simplified_without_tearing_it_is_kept() {
    // A card drawn on both sides, each of its triangles twice, wound both ways, shares every edge four times.
    let mut card = faceted(4, 0.5, |_, _| 0.0, |_| Vec4::ONE);
    let back: Vec<u32> = card
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|[a, b, c]| [*a, *c, *b])
        .collect();
    card.indices.extend(back);
    assert_eq!(card.simplified(0.1).0.indices.len(), card.indices.len());
    // A triangle with two corners at one point draws nothing, and is left out; nothing at all stays nothing.
    let mut degenerate = faceted(1, 1.0, |_, _| 0.0, |_| Vec4::ONE);
    degenerate.positions.push(Vec3::ZERO);
    degenerate.indices.extend([0, 6, 1]);
    assert_eq!(degenerate.simplified(0.0).0.indices.len(), 6);
    // Nor is a triangle along a line, facing nowhere, nor a smooth surface whose every edge is a seam, its texture's
    // pieces each a quad's.
    let line = Primitive::new(
        vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0],
        Vec::new(),
        vec![0, 1, 2],
    );
    assert_eq!(line.simplified(1.0).0, line);
    let mut pieces = smooth(4, 0.5, 9);
    let (positions, normals) = (pieces.positions.clone(), pieces.normals.clone());
    pieces.indices = (0..pieces.indices.len() as u32).collect();
    pieces.positions = grid_corners(&smooth(4, 0.5, 9), &positions);
    pieces.normals = grid_corners(&smooth(4, 0.5, 9), &normals);
    pieces.colors = vec![Vec4::ONE; pieces.indices.len()];
    pieces.uvs = (0..pieces.indices.len())
        .map(|corner| Vec2::new(corner as f32, 0.0))
        .collect();
    assert_eq!(pieces.simplified(1.0).0.indices.len(), pieces.indices.len());
    assert_eq!(
        Primitive::default().simplified(1.0),
        (Primitive::default(), 0.0)
    );
}

/// Each corner of `grid`'s triangles, in order, of `values`, one for each of its vertices.
fn grid_corners<T: Copy>(grid: &Primitive, values: &[T]) -> Vec<T> {
    grid.indices
        .iter()
        .map(|vertex| values[*vertex as usize])
        .collect()
}

#[test]
fn a_model_is_simplified_primitive_by_primitive_its_nodes_and_skins_as_they_were() {
    let ground = faceted(
        8,
        0.5,
        |x, z| (x * 0.9).sin() * 0.3 + z * 0.01,
        |_| Vec4::ONE,
    );
    let mut skinned = ground.clone();
    skinned.joints = vec![[0; crate::INFLUENCES]; skinned.positions.len()];
    skinned.weights = vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; skinned.positions.len()];
    let model = Model {
        meshes: vec![crate::Mesh::new("ground", vec![ground.clone(), skinned])],
        ..Model::default()
    };
    let (simplified, strayed) = model.simplified(0.05);
    assert!(strayed > 0.0 && strayed <= 0.05);
    let primitives = &simplified.meshes[0].primitives;
    assert_eq!(primitives[0], ground.simplified(0.05).0);
    assert_eq!(primitives[1].indices.len(), primitives[0].indices.len());
    assert!(
        primitives[1]
            .weights
            .iter()
            .all(|weights| weights[0] == 1.0)
    );
    assert_eq!(simplified.nodes, model.nodes);
    // Candidates are taken least cost first.
    let candidate = |cost, from| Candidate {
        cost,
        from,
        to: 0,
        versions: (0, 0),
    };
    let mut heap = BinaryHeap::from([candidate(2.0, 1), candidate(1.0, 2), candidate(1.0, 1)]);
    assert_eq!(heap.pop().map(|c| (c.cost, c.from)), Some((1.0, 1)));
    assert_eq!(
        candidate(1.0, 1).partial_cmp(&candidate(1.0, 1)),
        Some(Ordering::Equal)
    );
    assert_eq!(heap.pop().map(|c| c.from), Some(2));
}
