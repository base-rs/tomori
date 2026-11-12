// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::diagram::{Edge, Group, Layout, Node};
use tomori_protocol::{Place, Pose};

/// A diagram of `nodes`, laid out by hand: each at its own point, in millimetres.
fn diagram(nodes: &[(&str, Shape, [i16; 3])], edges: Vec<Edge>) -> Diagram {
    let mut diagram = Diagram::default();
    diagram.layout = Layout::Force;
    diagram.nodes = nodes
        .iter()
        .map(|(id, shape, at)| {
            let mut node = Node::new(*id, id.to_uppercase());
            node.shape = *shape;
            node.at = *at;
            node
        })
        .collect();
    diagram.edges = edges;
    diagram.extent = [600, 400, 200];
    diagram
}

fn edge(from: u16, to: u16, style: Style, direction: Arrow) -> Edge {
    let mut edge = Edge::new(from, to);
    edge.style = style;
    edge.direction = direction;
    edge
}

fn counted(instances: &[Instance], solid: Solid) -> usize {
    instances
        .iter()
        .filter(|instance| instance.solid == solid)
        .count()
}

#[test]
fn every_shape_is_a_solid_and_every_edge_style_and_arrow_is_drawn() {
    let shapes = [
        ("a", Shape::Box, [-200, 0, 0]),
        ("b", Shape::Sphere, [200, 0, 0]),
        ("c", Shape::Cylinder, [0, 150, 0]),
        ("d", Shape::Prism, [0, -150, 0]),
        ("e", Shape::Diamond, [-200, 150, 50]),
        ("f", Shape::Card, [200, -150, -50]),
    ];
    let mut with_detail = diagram(
        &shapes,
        vec![
            edge(0, 1, Style::Solid, Arrow::Forward),
            edge(0, 2, Style::Dashed, Arrow::Back),
            edge(0, 3, Style::Dotted, Arrow::Both),
            edge(1, 2, Style::Thick, Arrow::None),
            edge(1, 3, Style::Flow, Arrow::Forward),
        ],
    );
    with_detail.nodes[5].detail = "A declaration".into();
    with_detail.edges[0].label = "queries".into();
    with_detail.edges[0].via = vec![[0, 60, 0]];
    let mesh = Mesh::new(&with_detail);
    let mut instances = Vec::new();
    mesh.instances(Mat4::IDENTITY, None, 0.0, &mut instances);
    assert_eq!(instances.len(), mesh.solids());
    // A box for the box and the card, and a sphere for the sphere, the dotted edge's dots and the flow's beads.
    assert_eq!(counted(&instances, Solid::Box), 2);
    assert_eq!(counted(&instances, Solid::Cylinder), 1);
    assert_eq!(counted(&instances, Solid::Prism), 1);
    assert_eq!(counted(&instances, Solid::Diamond), 1);
    assert!(counted(&instances, Solid::Sphere) > 1 + BEADS);
    // Forward, back, both ends' and the flow's arrowheads: five, and none on the thick edge.
    assert_eq!(counted(&instances, Solid::Cone), 5);
    // The solid edge through its waypoint is two tubes, the dashed one several, the thick and the flow one each.
    assert!(counted(&instances, Solid::Tube) > 2 + 2 + 2);
    // The card shows its detail on its label; six nodes' labels and the edge's.
    assert_eq!(mesh.labels(), 7);
    // Its flow moves: its beads stand elsewhere a moment later.
    let mut later = Vec::new();
    mesh.instances(Mat4::IDENTITY, None, 0.5, &mut later);
    assert_ne!(instances, later);
    assert_eq!(mesh.source(), &with_detail);
    assert_eq!(mesh.diagram().nodes.len(), 6);
}

#[test]
fn groups_are_trays_floors_and_frames_with_their_labels() {
    let mut drawn = diagram(&[("a", Shape::Box, [0, 0, 0])], Vec::new());
    for (id, kind) in [
        ("t", GroupKind::Cluster),
        ("f", GroupKind::Layer),
        ("c", GroupKind::Container),
    ] {
        let mut group = Group::new(id, "");
        group.kind = kind;
        group.low = [-100, -100, -100];
        group.high = [100, 100, 100];
        drawn.groups.push(group);
    }
    drawn.groups[0].label = "Tray".into();
    let mesh = Mesh::new(&drawn);
    let mut instances = Vec::new();
    mesh.instances(Mat4::IDENTITY, None, 0.0, &mut instances);
    // The node, a tray, a floor and a frame's twelve edges.
    assert_eq!(counted(&instances, Solid::Box), 1 + 1 + 1 + 12);
    assert_eq!(mesh.labels(), 1 + 3);
    // Focused on the node, the groups dim.
    let mut focused = Vec::new();
    mesh.instances(Mat4::IDENTITY, Some(0), 0.0, &mut focused);
    assert_eq!(focused[1].colour, TRAY.map(|channel| channel * DIMMED));
    // A volume floating is as large as its face's cells, as every panel is.
    let panel = Panel::new(
        "panel-1",
        "Groups",
        "",
        Place::Free {
            pose: Pose::default(),
        },
    )
    .with_diagram(drawn.clone());
    let (columns, rows) = fit(&panel, &drawn, Fit::Text);
    assert_eq!(crate::projection::fit(&panel, Fit::Text), (columns, rows));
    assert_eq!(
        crate::projection::fitted_size(&panel, Fit::Text),
        super::super::sized(columns, rows)
    );
}

#[test]
fn a_volume_is_moved_by_its_model_and_its_focus_dims_what_is_not_joined_to_the_node() {
    let drawn = diagram(
        &[
            ("a", Shape::Sphere, [-200, 0, 0]),
            ("b", Shape::Sphere, [0, 0, 0]),
            ("c", Shape::Sphere, [200, 0, 0]),
        ],
        vec![
            edge(0, 1, Style::Solid, Arrow::None),
            edge(1, 2, Style::Solid, Arrow::None),
        ],
    );
    let mesh = Arc::new(Mesh::new(&drawn));
    // A millimetre to a millimetre of the scene, turned a quarter about the vertical and a metre up.
    let model = Mat4::from_translation(Vec3::Y)
        * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::splat(0.001));
    let volume = Drawn {
        mesh: mesh.clone(),
        model,
        focus: None,
        time: 0.0,
    };
    let mut instances = Vec::new();
    volume.instances(&mut instances);
    let a = instances[0];
    assert!(
        (a.position - Vec3::new(0.0, 1.0, 0.2)).length() < 1e-4,
        "{:?}",
        a.position
    );
    assert!((a.scale - Vec3::splat(0.06)).length() < 1e-5);
    assert_eq!(volume, volume.clone());
    // Focused on a, b and the edge between them stay bright; c and its edge dim.
    let mut focused = Vec::new();
    mesh.instances(model, Some(0), 0.0, &mut focused);
    let bright = rgb(tomori_protocol::chart::Colour::Grey);
    assert_eq!([focused[0].colour, focused[1].colour], [bright, bright]);
    assert_eq!(focused[2].colour, bright.map(|channel| channel * DIMMED));
    let tubes: Vec<[f32; 3]> = focused
        .iter()
        .filter(|instance| instance.solid == Solid::Tube)
        .map(|instance| instance.colour)
        .collect();
    assert_eq!(tubes, [EDGE, EDGE.map(|channel| channel * DIMMED)]);
    // Its labels face the viewer, and only those focused on show.
    let mut sheets = Vec::new();
    mesh.sheets(model, Vec3::new(5.0, 1.0, 0.0), None, &mut sheets);
    assert_eq!(sheets.len(), 3);
    let facing = sheets[0].model.unwrap().z_axis.truncate().normalize();
    assert!(facing.x > 0.99, "{facing:?}");
    sheets.clear();
    mesh.sheets(model, Vec3::new(5.0, 1.0, 0.0), Some(0), &mut sheets);
    assert_eq!(sheets.len(), 2);
    // A node with no detail has no card; one with a detail has one, beside it.
    assert_eq!(mesh.card(0, model, Vec3::ZERO), None);
    assert_eq!(mesh.card(9, model, Vec3::ZERO), None);
    let mut detailed = drawn.clone();
    detailed.nodes[0].detail = "What it does".into();
    let card = Mesh::new(&detailed)
        .card(0, model, Vec3::new(5.0, 1.0, 0.0))
        .unwrap();
    assert!(card.canvas.unwrap().rows.len() == 2);
}

#[test]
fn the_ray_meets_the_nearest_node_or_an_edge_within_reach() {
    let drawn = diagram(
        &[
            ("a", Shape::Sphere, [-200, 0, 0]),
            ("b", Shape::Box, [200, 0, 0]),
            ("c", Shape::Box, [200, 0, -150]),
        ],
        vec![edge(0, 1, Style::Solid, Arrow::Forward)],
    );
    let mesh = Mesh::new(&drawn);
    let model =
        Mat4::from_translation(Vec3::new(0.0, 1.5, -1.0)) * Mat4::from_scale(Vec3::splat(0.001));
    let from = |x: f32| Ray::new(Vec3::new(x, 1.5, 0.0), -Vec3::Z).unwrap();
    // Straight at a, about a metre away less its radius; at b, which hides c behind it.
    let (met, distance) = mesh.hit(model, &from(-0.2)).unwrap();
    assert_eq!(met, Met::Node(0));
    assert!((distance - (1.0 - 0.033)).abs() < 1e-3, "{distance}");
    assert_eq!(mesh.hit(model, &from(0.2)).unwrap().0, Met::Node(1));
    // Between them, the edge; and past everything, nothing.
    assert_eq!(mesh.hit(model, &from(0.0)).unwrap().0, Met::Edge(0));
    assert_eq!(mesh.hit(model, &from(0.0 + 0.012)).unwrap().0, Met::Edge(0));
    assert_eq!(mesh.hit(model, &from(0.6)), None);
    // Pointed away, it meets nothing behind it.
    assert_eq!(
        mesh.hit(
            model,
            &Ray::new(Vec3::new(-0.2, 1.5, 0.0), Vec3::Z).unwrap()
        ),
        None
    );
    // Along an edge, as from its end, the edge is met at its start.
    let along = Ray::new(Vec3::new(-0.5, 1.5, -1.0), Vec3::X).unwrap();
    assert!(mesh.hit(model, &along).is_some());
    assert_eq!(passing(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Y), (0.0, 1.0));
}

#[test]
fn a_volume_fits_its_slot_and_floats_as_large_as_its_diagram_within_bounds() {
    let mut drawn = diagram(&[("a", Shape::Box, [0, 0, 0])], Vec::new());
    let panel = Panel::new(
        "panel-1",
        "Renderer",
        "",
        Place::Free {
            pose: Pose::default(),
        },
    );
    let cells = super::super::Cells::STANDARD;
    assert_eq!(
        fit(&panel, &drawn, Fit::Slot(cells)),
        (cells.columns, cells.body)
    );
    // Floating, 600 by 400 mm, a little wider than 30 columns and 15 rows.
    let (columns, rows) = fit(&panel, &drawn, Fit::Text);
    assert!(
        (34..=38).contains(&columns) && (17..=19).contains(&rows),
        "{columns} {rows}"
    );
    // Tiny, it grows to the least; huge, it shrinks to the most; never wider than its rows allow.
    drawn.extent = [10, 10, 10];
    let (columns, rows) = fit(&panel, &drawn, Fit::Text);
    assert!(columns >= super::super::FIT_COLUMNS.0 && rows <= FIT_ROWS.1);
    drawn.extent = [30_000, 1_000, 1_000];
    assert_eq!(fit(&panel, &drawn, Fit::Text).0, TABLE_COLUMNS);
    // In a slot, it is no deeper than half its height, and stands in front of its face.
    drawn.extent = [600, 400, 2_000];
    let slotted = placed(&panel, &drawn, Fit::Slot(cells));
    let back = slotted.transform_point3(Vec3::new(0.0, 0.0, -1000.0));
    let front = slotted.transform_point3(Vec3::new(0.0, 0.0, 1000.0));
    let (_, height) = super::super::sized(cells.columns, cells.body);
    assert!(
        back.z >= STAND_OFF - 1e-4 && front.z - back.z <= height / 2.0,
        "{back} {front}"
    );
    let floating = placed(&panel, &drawn, Fit::Text);
    assert!(floating.transform_point3(Vec3::new(0.0, 0.0, -1000.0)).z >= STAND_OFF - 1e-4);
    // Its face: dark in a slot, faint floating, its title first.
    let slot = face(&panel, &drawn, Fit::Slot(cells), None);
    let free = face(&panel, &drawn, Fit::Text, Some([1.0; 4]));
    assert!(slot.background[3] > 0.9 && free.background[3] < 0.5);
    assert_eq!(free.border, Some([1.0; 4]));
    assert_eq!(slot.rows.len(), 3 + cells.body);
}

#[test]
fn labels_are_drawn_smaller_with_the_volume_down_to_their_least() {
    let full = label_scale(Mat4::from_scale(Vec3::splat(0.001)));
    let small = label_scale(Mat4::from_scale(Vec3::splat(0.0001)));
    assert!((full - LABEL_PIXEL / PIXEL).abs() < 1e-6);
    assert!((small - LEAST_LABEL * LABEL_PIXEL / PIXEL).abs() < 1e-6);
    assert_eq!(
        point_along(&[Vec3::ZERO, Vec3::ZERO, Vec3::X], 0.5),
        Vec3::X * 0.5
    );
    assert_eq!(point_along(&[Vec3::ZERO, Vec3::X], 5.0), Vec3::X);
    assert_eq!(
        point_along(&[Vec3::ZERO, Vec3::ZERO, Vec3::X], 0.0),
        Vec3::ZERO
    );
    assert_eq!(Solid::ALL.len(), 7);
    // More nodes than are labelled: the largest are.
    let nodes: Vec<(String, Shape, [i16; 3])> = (0..40)
        .map(|index| (format!("n{index}"), Shape::Box, [index as i16 * 100, 0, 0]))
        .collect();
    let named: Vec<(&str, Shape, [i16; 3])> = nodes
        .iter()
        .map(|(id, shape, at)| (id.as_str(), *shape, *at))
        .collect();
    let mut many = diagram(&named, Vec::new());
    many.nodes[39].size = 5;
    let mesh = Mesh::new(&many);
    assert_eq!(mesh.labels(), NODE_LABELS);
    assert_eq!(mesh.labels[0].owner, Owner::Node(39));
}

mod projector {
    use super::*;
    use crate::arrival::HOME;
    use crate::projection::{Controls, Hold, Input, Part, Projector, editing::Rights};
    use std::time::{Duration, Instant};
    use tomori_protocol::Projection;

    /// A volume of three nodes, `a`, `b` and `c`, along a line, `a` to `b` and `b` to `c`, floating 1.2 m in front
    /// of home, facing it.
    fn floating() -> (Projection, Pose) {
        let pose = Pose::upright_facing([0.0, 1.5, -1.2], HOME.to_array());
        let mut drawn = diagram(
            &[
                ("a", Shape::Sphere, [-200, 0, 0]),
                ("b", Shape::Box, [0, 0, 0]),
                ("c", Shape::Sphere, [200, 0, 0]),
            ],
            vec![
                edge(0, 1, Style::Solid, Arrow::Forward),
                edge(1, 2, Style::Solid, Arrow::Forward),
            ],
        );
        drawn.nodes[1].detail = "In the middle".into();
        let panel = Panel::new("panel-1", "Line", "- a\n- b\n- c", Place::Free { pose })
            .with_diagram(drawn);
        (Projection::new("s", "sky").with_panels(vec![panel]), pose)
    }

    fn input(
        projection: Option<&Projection>,
        ray: Option<Ray>,
        trigger: bool,
        left: Hold,
        now: Instant,
    ) -> Input<'_> {
        Input {
            projection,
            ray,
            grip: Some(0.0),
            viewer: Some(HOME),
            active: true,
            pointing: true,
            nearer: f32::INFINITY,
            stick: glam::Vec2::ZERO,
            straighten: false,
            free: trigger,
            controls: Controls::Controllers,
            scroll: None,
            rights: Rights::default(),
            surfaces: None,
            left,
            now,
            dt: 1.0 / 72.0,
        }
    }

    /// Where `point`, in the volume's own millimetres, stands in the scene.
    fn world(projection: &Projection, pose: Pose, point: Vec3) -> Vec3 {
        let panel = &projection.panels[0];
        let model = crate::board::model(&pose).unwrap()
            * placed(panel, panel.diagram.as_ref().unwrap(), Fit::Text);
        model.transform_point3(point)
    }

    fn towards(point: Vec3) -> Option<Ray> {
        Ray::new(HOME, point - HOME)
    }

    #[test]
    fn pointing_at_a_volume_names_its_node_or_edge_with_either_hand_and_the_trigger_focuses_on_a_node()
     {
        let (projection, pose) = floating();
        let mut projector = Projector::default();
        let now = Instant::now();
        let at_a = towards(world(&projection, pose, Vec3::new(-200.0, 0.0, 0.0)));
        let left = Hold {
            ray: towards(world(&projection, pose, Vec3::new(100.0, 0.0, 0.0))),
            grip: Some(0.0),
            active: true,
            pointing: true,
            ..Hold::default()
        };
        let output = projector.update(input(Some(&projection), at_a, false, left, now));
        assert_eq!(
            output.pointed.as_ref().map(|(id, ..)| id.as_str()),
            Some("panel-1")
        );
        assert_eq!(output.part, Some(Part::Node("a".into())));
        assert_eq!(output.left.part, Some(Part::Edge(1)));
        assert_eq!(output.volumes.len(), 1);
        // Its face, and three nodes' labels.
        assert!(output.sheets.len() >= 4);
        // The trigger, pressed on a node, focuses on it, dimming the rest; held, it does nothing more; pressed again,
        // it ends the focus.
        let at_b = towards(world(&projection, pose, Vec3::ZERO));
        let output = projector.update(input(Some(&projection), at_b, true, Hold::default(), now));
        assert_eq!(projector.focus(), Some(("panel-1", 1)));
        assert_eq!(output.volumes[0].focus, Some(1));
        // Focused on b, which has a detail, its card shows beside it.
        let cards = output.sheets.len();
        projector.update(input(Some(&projection), at_b, true, Hold::default(), now));
        assert_eq!(projector.focus(), Some(("panel-1", 1)));
        projector.update(input(Some(&projection), at_b, false, Hold::default(), now));
        let output = projector.update(input(Some(&projection), at_b, true, Hold::default(), now));
        assert_eq!(projector.focus(), None);
        assert!(output.sheets.len() < cards);
        // On nothing of the volume, the trigger focuses on nothing; on its edge, neither.
        projector.update(input(Some(&projection), at_a, false, Hold::default(), now));
        let off = towards(Vec3::new(5.0, 1.5, -1.2));
        projector.update(input(Some(&projection), off, true, Hold::default(), now));
        assert_eq!(projector.focus(), None);
        // Time moves the flows on.
        let later = projector.update(input(
            Some(&projection),
            None,
            false,
            Hold::default(),
            now + Duration::from_secs(1),
        ));
        assert!(later.volumes[0].time >= 1.0);
    }

    #[test]
    fn a_volume_s_mesh_is_kept_until_its_diagram_changes_and_forgotten_with_it() {
        let (projection, pose) = floating();
        let mut projector = Projector::default();
        let now = Instant::now();
        let at_b = towards(world(&projection, pose, Vec3::ZERO));
        let first = projector.update(input(Some(&projection), at_b, true, Hold::default(), now));
        assert_eq!(projector.focus(), Some(("panel-1", 1)));
        let again = projector.update(input(Some(&projection), None, false, Hold::default(), now));
        assert!(Arc::ptr_eq(&first.volumes[0].mesh, &again.volumes[0].mesh));
        // A node fewer: built again, and the focus on a node it no longer has ends.
        let mut fewer = projection.clone();
        let diagram = fewer.panels[0].diagram.as_mut().unwrap();
        diagram.nodes.truncate(1);
        diagram.edges.clear();
        let changed = projector.update(input(Some(&fewer), None, false, Hold::default(), now));
        assert!(!Arc::ptr_eq(
            &first.volumes[0].mesh,
            &changed.volumes[0].mesh
        ));
        assert_eq!(projector.focus(), None);
        // A text panel in its place, or no projection at all, draws no volume.
        projector.update(input(Some(&projection), at_b, true, Hold::default(), now));
        let mut text = projection.clone();
        text.panels[0].diagram = None;
        let output = projector.update(input(Some(&text), None, false, Hold::default(), now));
        assert!(output.volumes.is_empty());
        assert_eq!(projector.focus(), None);
        projector.update(input(Some(&projection), None, false, Hold::default(), now));
        projector.update(input(Some(&projection), at_b, true, Hold::default(), now));
        assert!(
            projector
                .update(input(None, None, false, Hold::default(), now))
                .volumes
                .is_empty()
        );
        assert_eq!(projector.focus(), None);
    }
}
