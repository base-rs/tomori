// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_gpu;
use glam::{Mat4, Vec3};
use std::sync::Arc;
use tomori_protocol::diagram::{Arrow, Diagram, Edge, Group, GroupKind, Node, Shape, Style};
use tomori_scene::projection::volume::Mesh;

#[test]
fn each_solid_is_a_unit_mesh_of_few_triangles_lit_by_its_normals() {
    for (solid, triangles) in [
        (Solid::Box, 12),
        (Solid::Sphere, 80),
        (Solid::Cylinder, 48),
        (Solid::Prism, 24),
        (Solid::Tube, 12),
        (Solid::Diamond, 8),
        (Solid::Cone, 16),
    ] {
        let vertices = mesh(solid);
        assert_eq!(vertices.len(), triangles * 3, "{solid:?}");
        for vertex in &vertices {
            assert!(vertex.position.iter().all(|value| value.abs() <= 0.5 + 1e-5), "{solid:?}");
            let normal = Vec3::from_array(vertex.normal);
            assert!((normal.length() - 1.0).abs() < 1e-4, "{solid:?}");
        }
    }
}

/// A diagram at every bound: 64 spheres, 128 edges of four waypoints each, dashed, and twelve containers.
fn largest() -> Diagram {
    let mut diagram = Diagram::default();
    for index in 0..64 {
        let mut node = Node::new(format!("n{index}"), "x".repeat(32));
        node.shape = Shape::Sphere;
        node.at = [(index % 8) as i16 * 150, (index / 8) as i16 * 150, 0];
        node.size = 5;
        diagram.nodes.push(node);
    }
    for index in 0..128u16 {
        let mut edge = Edge::new(index % 64, (index * 7 + 1) % 64);
        edge.via = vec![[0, 0, 300], [300, 300, 300], [600, 0, 300], [900, 300, 0]];
        edge.style = Style::Dashed;
        edge.direction = Arrow::Both;
        edge.label = "label".into();
        diagram.edges.push(edge);
    }
    for index in 0..12 {
        let mut group = Group::new(format!("g{index}"), "Group");
        group.kind = GroupKind::Container;
        group.high = [1000, 1000, 100];
        diagram.groups.push(group);
    }
    diagram.extent = [1200, 1200, 600];
    diagram
}

/// A diagram of thirty boxes and forty solid edges, as a typical one.
fn typical() -> Diagram {
    let mut diagram = Diagram::default();
    for index in 0..30 {
        let mut node = Node::new(format!("n{index}"), "Module");
        node.at = [(index % 6) as i16 * 150, (index / 6) as i16 * 150, 0];
        diagram.nodes.push(node);
    }
    for index in 0..40u16 {
        diagram.edges.push(Edge::new(index % 30, (index * 7 + 1) % 30));
    }
    diagram.extent = [900, 750, 200];
    diagram
}

fn drawn(diagram: &Diagram, count: usize) -> Vec<Drawn> {
    let mesh = Arc::new(Mesh::new(diagram));
    (0..count)
        .map(|index| Drawn {
            mesh: mesh.clone(),
            model: Mat4::from_translation(Vec3::X * index as f32) * Mat4::from_scale(Vec3::splat(0.001)),
            focus: None,
            time: 0.5,
        })
        .collect()
}

#[test]
fn a_frame_s_solids_are_drawn_within_the_budget_at_the_headset_s_limits() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let mut solids = Solids::new(&device);
        // Sixteen typical volumes are drawn whole.
        solids.update(&queue, &drawn(&typical(), 16));
        let (triangles, left_out) = solids.counts();
        assert_eq!(left_out, 0);
        assert!(triangles < TRIANGLES / 2, "{triangles}");
        // One at every bound fits alone; sixteen of them are held to the budget, the last drawn without their solids.
        solids.update(&queue, &drawn(&largest(), 1));
        let (one, left_out) = solids.counts();
        assert_eq!(left_out, 0);
        assert!(one <= TRIANGLES, "{one}");
        solids.update(&queue, &drawn(&largest(), 16));
        let (triangles, left_out) = solids.counts();
        assert!(triangles <= TRIANGLES && left_out > 0, "{triangles} {left_out}");
        // A frame of no volumes draws none.
        solids.update(&queue, &[]);
        assert_eq!(solids.counts(), (0, 0));
        // Many tiny ones are held to the most solids a frame draws, though their triangles would fit.
        let mut dots = Diagram::default();
        for index in 0..200 {
            let mut node = Node::new(format!("n{index}"), "");
            node.shape = Shape::Diamond;
            node.at = [index, 0, 0];
            dots.nodes.push(node);
        }
        dots.nodes.truncate(tomori_protocol::diagram::NODES);
        solids.update(&queue, &drawn(&dots, MOST_SOLIDS / 64 + 2));
        assert!(solids.counts().1 > 0);
    });
}
