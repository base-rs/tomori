// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::diagram::{Edge, Group, Layout, Node};
use tomori_protocol::world::{Kind, Mark, Reference, State, Target};
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
    mesh.instances(Mat4::IDENTITY, None, 0.0, Vec3::ZERO, &mut instances);
    assert_eq!(instances.len(), mesh.solids());
    // A box for the box and the card, a sphere for the sphere and the flow's beads, and a diamond for the diamond and
    // the dotted edge's dots.
    assert_eq!(counted(&instances, Solid::Box), 2);
    assert_eq!(counted(&instances, Solid::Cylinder), 1);
    assert_eq!(counted(&instances, Solid::Prism), 1);
    assert!(counted(&instances, Solid::Diamond) > 1);
    assert_eq!(counted(&instances, Solid::Sphere), 1 + BEADS);
    // The solid edge through its waypoint narrows along itself in tubes, and the straight flow is one narrowing tube;
    // the dashed one is several tubes, and the thick one, pointing nowhere, one; the three that point one way have two
    // arrowheads each, one at their target and one past their middle.
    assert_eq!(counted(&instances, Solid::Cone), 3 * 2);
    assert_eq!(counted(&instances, Solid::Taper), 1);
    assert!(counted(&instances, Solid::Tube) > TAPER_PIECES + 2 + 1);
    // The card shows its detail on its label; six nodes' labels and the edge's.
    assert_eq!(mesh.labels(), 7);
    // Its flow moves: its beads stand elsewhere a moment later.
    let mut later = Vec::new();
    mesh.instances(Mat4::IDENTITY, None, 0.5, Vec3::ZERO, &mut later);
    assert_ne!(instances, later);
    assert_eq!(mesh.source(), &with_detail);
    assert_eq!(mesh.diagram().nodes.len(), 6);
}

#[test]
fn clusters_are_their_colour_alone_and_layers_floors_and_containers_frames_in_theirs() {
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
    mesh.instances(Mat4::IDENTITY, None, 0.0, Vec3::ZERO, &mut instances);
    // The node, no tray, a floor's four edges at its foot and a frame's twelve, each thin, in its group's colour.
    assert_eq!(counted(&instances, Solid::Box), 1 + 4 + 12);
    // The node's label alone: each group has a colour, which the legend names.
    assert_eq!(mesh.labels(), 1);
    use tomori_protocol::chart::Colour;
    let floor = Colour::Amber.rgb().map(|channel| channel * FRAMED);
    for bar in &instances[1..5] {
        assert_eq!((bar.colour, bar.position.y), (floor, -100.0));
        assert_eq!(bar.scale.min_element(), BAR_MM);
    }
    assert_eq!(
        instances[5].colour,
        Colour::Blue.rgb().map(|channel| channel * FRAMED)
    );
    // Focused on the node, the groups dim.
    let mut focused = Vec::new();
    mesh.instances(Mat4::IDENTITY, Some(0), 0.0, Vec3::ZERO, &mut focused);
    assert_eq!(focused[1].colour, floor.map(|channel| channel * DIMMED));
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
        viewer: Vec3::ZERO,
        legend: Vec::new(),
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
    mesh.instances(model, Some(0), 0.0, Vec3::ZERO, &mut focused);
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
    // Floating, 600 by 400 mm and 200 deep, a little wider than 30 columns and 15 rows, grown by a sixth as its front
    // is seen larger from 1.3 m away, and two rows kept for its labels.
    let (columns, rows) = fit(&panel, &drawn, Fit::Text);
    assert!(
        (40..=44).contains(&columns) && (21..=24).contains(&rows),
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
    assert_eq!(Solid::ALL.len(), 8);
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
    fn pointing_at_a_leader_names_its_world_edge_and_the_trigger_on_it_focuses_on_its_node() {
        let (mut projection, _) = floating();
        let mut fire = Target::new(
            "fire",
            Reference::new(Kind::Object, "farmyard:campsite/bonfire"),
        );
        fire.state = State::Here;
        fire.at = Some([1.5, 0.5, -1.2]);
        fire.radius = 0.2;
        let diagram = projection.panels[0].diagram.as_mut().unwrap();
        diagram.world = vec![fire];
        diagram.world_edges = vec![edge(2, 0, Style::Solid, Arrow::Forward)];
        let mut projector = Projector::default();
        let now = Instant::now();
        // At the near side of its ring, from home.
        let at_ring = towards(Vec3::new(1.3, 0.5, -1.2));
        let output = projector.update(input(
            Some(&projection),
            at_ring,
            false,
            Hold::default(),
            now,
        ));
        assert_eq!(output.part, Some(Part::WorldEdge(0)));
        projector.update(input(
            Some(&projection),
            at_ring,
            true,
            Hold::default(),
            now,
        ));
        assert_eq!(projector.focus(), Some(("panel-1", 2)));
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
        // Nor where something nearer, such as a card, takes the ray.
        projector.update(input(Some(&projection), off, false, Hold::default(), now));
        let mut card_in_front = input(Some(&projection), at_b, true, Hold::default(), now);
        card_in_front.nearer = 0.05;
        projector.update(card_in_front);
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

/// A target `id` of `kind`, here at `at`, marked `mark`.
fn target(id: &str, kind: Kind, at: [f32; 3], mark: Mark) -> Target {
    let mut target = Target::new(id, Reference::new(kind, id));
    target.state = State::Here;
    target.at = Some(at);
    target.mark = mark;
    target.radius = 0.2;
    target
}

/// A volume of two nodes a metre and a half up and a metre ahead, facing +Z, with a world edge from the first to each of
/// `targets`, in `styles` in turn.
fn pointing(targets: Vec<Target>, edges: &[(Style, Arrow)]) -> (Mesh, Mat4) {
    let mut drawn = diagram(
        &[
            ("a", Shape::Box, [-100, 0, 0]),
            ("b", Shape::Box, [100, 0, 0]),
        ],
        Vec::new(),
    );
    drawn.world_edges = edges
        .iter()
        .enumerate()
        .map(|(index, (style, direction))| {
            edge(0, (index % targets.len()) as u16, *style, *direction)
        })
        .collect();
    drawn.world = targets;
    let model =
        Mat4::from_translation(Vec3::new(0.0, 1.5, -1.0)) * Mat4::from_scale(Vec3::splat(0.001));
    (Mesh::new(&drawn), model)
}

#[test]
fn a_world_edge_is_a_leader_from_its_node_to_its_target_in_its_style_with_a_mark() {
    use tomori_protocol::world::Frame;
    let mut panel = target("notes", Kind::Panel, [2.0, 1.4, -2.0], Mark::ByKind);
    panel.frame = Some(Frame {
        orientation: [0.0, 0.0, 0.0, 1.0],
        size: [0.6, 0.36],
    });
    let mut absent = target("away", Kind::Person, [0.0; 3], Mark::ByKind);
    absent.state = State::Absent;
    let targets = vec![
        target("fire", Kind::Object, [1.5, 0.5, -1.0], Mark::ByKind),
        target("lookout", Kind::Anchor, [-2.0, 0.0, -3.0], Mark::ByKind),
        target("sam", Kind::Person, [0.0, 1.6, 1.0], Mark::ByKind),
        panel,
        target("board", Kind::Wall, [-1.0, 1.5, -4.0], Mark::Outline),
        target("rock", Kind::Object, [0.5, 0.2, -2.0], Mark::Unmarked),
        absent,
    ];
    let (mesh, model) = pointing(
        targets,
        &[
            (Style::Solid, Arrow::Forward),
            (Style::Dashed, Arrow::Back),
            (Style::Dotted, Arrow::Both),
            (Style::Thick, Arrow::None),
            (Style::Flow, Arrow::Forward),
            (Style::Flow, Arrow::Back),
            (Style::Solid, Arrow::Forward),
        ],
    );
    // One for each target here: the absent one has none.
    assert_eq!(mesh.leaders(), 6);
    assert_eq!((mesh.world_edge(5), mesh.world_edge(9)), (5, 0));
    assert_eq!((mesh.leader_node(0), mesh.leader_node(9)), (Some(0), None));
    let mut instances = Vec::new();
    mesh.instances(model, None, 0.25, Vec3::new(0.0, 1.6, 0.0), &mut instances);
    // Rings, halos and a frame are tubes; dots diamonds; beads spheres; arrowheads cones.
    assert!(
        counted(&instances, Solid::Tube) > 6 * 16,
        "{}",
        instances.len()
    );
    assert!(counted(&instances, Solid::Diamond) > 3);
    assert_eq!(counted(&instances, Solid::Sphere), 8);
    assert_eq!(counted(&instances, Solid::Cone), 6);
    // More than three, every leader is at half strength, its mark at a third.
    let colours: Vec<[f32; 3]> = instances.iter().map(|instance| instance.colour).collect();
    assert!(colours.contains(&EDGE.map(|channel| channel * 0.5)));
    assert!(colours.contains(&EDGE.map(|channel| channel * (1.0 / 3.0))));
    // Focused on their node, in full, with their labels; on the other, dimmed, without.
    let mut focused = Vec::new();
    mesh.instances(model, Some(0), 0.0, Vec3::ZERO, &mut focused);
    assert!(focused.iter().any(|instance| instance.colour == EDGE));
    let mut dimmed = Vec::new();
    mesh.instances(model, Some(1), 0.0, Vec3::ZERO, &mut dimmed);
    assert!(dimmed.iter().all(|instance| instance.colour != EDGE));
    let mut sheets = Vec::new();
    mesh.sheets(model, Vec3::new(0.0, 1.6, 0.0), Some(0), &mut sheets);
    // Its node's own label, and a label at each of its targets.
    assert_eq!(sheets.len(), 1 + 6);
    sheets.clear();
    mesh.sheets(model, Vec3::new(0.0, 1.6, 0.0), None, &mut sheets);
    assert_eq!(sheets.len(), 2);
}

#[test]
fn a_few_leaders_are_drawn_in_full_and_the_ray_meets_one_along_its_line_or_at_its_mark() {
    let (mesh, model) = pointing(
        vec![target("fire", Kind::Object, [1.5, 0.5, -1.0], Mark::Ring)],
        &[(Style::Solid, Arrow::Forward)],
    );
    let mut instances = Vec::new();
    mesh.instances(model, None, 0.0, Vec3::ZERO, &mut instances);
    assert!(instances.iter().any(|instance| instance.colour == EDGE));
    let mut sheets = Vec::new();
    mesh.sheets(model, Vec3::ZERO, None, &mut sheets);
    assert_eq!(sheets.len(), 3);
    // Labelled with the edge's label, or else the target's, or else its kind.
    let mut unlabelled = target("rock", Kind::Object, [1.0, 0.0, -1.0], Mark::Ring);
    unlabelled.label = String::new();
    let mut labelled = unlabelled.clone();
    labelled.label = "The rock".into();
    let (named, _) = pointing(
        vec![unlabelled, labelled],
        &[(Style::Solid, Arrow::Forward); 2],
    );
    let text = |leader: &leaders::Leader| -> String {
        leader.label.rows[0]
            .cells
            .iter()
            .map(|(c, _)| *c)
            .collect::<String>()
            .trim()
            .to_owned()
    };
    let texts: Vec<String> = named.leaders.iter().map(text).collect();
    assert_eq!(texts, ["object", "The rock"]);
    let mut with_label = mesh.diagram().clone();
    with_label.world_edges[0].label = "here".into();
    assert_eq!(text(&Mesh::new(&with_label).leaders[0]), "here");
    // Its ring about the fire, a fifth of a metre out, is met from above.
    let down = Ray::new(Vec3::new(1.7, 2.0, -1.0), -Vec3::Y).unwrap();
    assert_eq!(mesh.hit(model, &down).unwrap().0, Met::Leader(0));
    // And it leaves the volume through its face towards the fire, its right.
    let out = leaders::out_of(model, [600, 400, 200], Vec3::new(1.5, 0.5, -1.0));
    assert!((out - Vec3::X).length() < 1e-5, "{out:?}");
    let behind = leaders::out_of(model, [600, 400, 200], Vec3::new(0.0, 1.5, -3.0));
    assert!((behind + Vec3::Z).length() < 1e-5, "{behind:?}");
    // Along its middle, too.
    let path = mesh.leader_paths(model).remove(0);
    let middle = path[path.len() / 2];
    let at = Ray::new(Vec3::new(middle.x, middle.y, 1.0), -Vec3::Z).unwrap();
    assert_eq!(mesh.hit(model, &at).unwrap().0, Met::Leader(0));
}

#[test]
fn a_leader_to_something_far_off_is_drawn_with_no_more_dashes_or_dots_than_a_near_one() {
    for (style, solid) in [
        (Style::Dashed, Solid::Tube),
        (Style::Dotted, Solid::Diamond),
    ] {
        let count = |at: [f32; 3]| {
            let (mesh, model) = pointing(
                vec![target("far", Kind::Object, at, Mark::Unmarked)],
                &[(style, Arrow::None)],
            );
            let mut instances = Vec::new();
            mesh.instances(model, None, 0.0, Vec3::ZERO, &mut instances);
            counted(&instances, solid)
        };
        let (near, far) = (count([1.0, 0.5, -1.5]), count([0.0, 0.0, -900.0]));
        assert!(near > 4, "{style:?}: {near}");
        assert!(far <= 65, "{style:?}: {far}");
    }
}

#[test]
fn a_leader_ends_on_its_mark_nearest_where_it_comes_from() {
    use tomori_protocol::world::Frame;
    let mut board = target("board", Kind::Wall, [0.0, 1.5, -4.0], Mark::ByKind);
    board.frame = Some(Frame {
        orientation: [0.0, 0.0, 0.0, 1.0],
        size: [2.0, 1.0],
    });
    let (mesh, model) = pointing(
        vec![
            board,
            target("halo", Kind::Agent, [0.0, 1.2, -3.0], Mark::Outline),
            target("rock", Kind::Object, [3.0, 0.0, -1.0], Mark::Unmarked),
        ],
        &[
            (Style::Solid, Arrow::Forward),
            (Style::Solid, Arrow::Forward),
            (Style::Solid, Arrow::Forward),
        ],
    );
    let paths = mesh.leader_paths(model);
    // At the top of the board's frame, above a halo standing for an outline without a frame, and on top of the rock.
    let end = |index: usize| *paths[index].last().unwrap();
    assert!(
        (end(0).y - 2.0).abs() < 1e-4 && end(0).z > -4.0,
        "{:?}",
        end(0)
    );
    assert!((end(1).y - (1.2 + 0.2 + 0.1)).abs() < 1e-4, "{:?}", end(1));
    assert!(
        (end(2) - Vec3::new(3.0, 0.2, -1.0)).length() < 1e-4,
        "{:?}",
        end(2)
    );
    // Coming down onto it from above.
    let last = paths[2][paths[2].len() - 2];
    assert!(last.y > end(2).y);
}

#[test]
fn a_node_s_default_grey_keeps_its_brightness_and_other_colours_are_the_palette_s() {
    use tomori_protocol::chart::Colour;
    assert_eq!(rgb(Colour::Grey), NODE_GREY);
    assert!(NODE_GREY[0] > Colour::Grey.rgb()[0]);
    assert_eq!(rgb(Colour::Teal), Colour::Teal.rgb());
}

#[test]
fn a_label_s_lines_are_rows_wrapped_at_its_words_onto_at_most_three() {
    let rows = |canvas: &Canvas| -> Vec<String> {
        canvas
            .plain()
            .lines()
            .map(|row| row.trim_end().to_owned())
            .collect()
    };
    // Its lines as the agent broke them, as wide as the widest.
    let broken = label("Hub\nunder one lock", None, BRIGHT, 0.85);
    assert_eq!(rows(&broken), ["Hub", "under one lock"]);
    assert_eq!(broken.columns, "under one lock".len());
    // A long line wrapped at its words within the columns.
    let long = label("Frame step and its projector", None, BRIGHT, 0.85);
    assert_eq!(rows(&long), ["Frame step and its", "projector"]);
    assert!(long.columns <= LABEL_COLUMNS);
    // At most three rows, the last cut with "…" where more would follow.
    let many = label(
        "one two three four five six seven\neight nine ten eleven twelve",
        None,
        BRIGHT,
        0.85,
    );
    let shown = rows(&many);
    assert_eq!(shown.len(), LABEL_LINES);
    assert_eq!(shown[2], "eight nine ten…");
    // A card's detail below its rows.
    let card = label("Hub\nlock", Some("What it does"), BRIGHT, 0.92);
    assert_eq!(rows(&card), ["Hub", "lock", "What it does"]);
    // Its first row stands where a label of one would, the rest growing upwards.
    let mut drawn = diagram(&[("a", Shape::Box, [0, 0, 0])], Vec::new());
    let model =
        Mat4::from_translation(Vec3::new(0.0, 1.5, -1.0)) * Mat4::from_scale(Vec3::splat(0.001));
    // Seen from within the volume's box, nothing moves it.
    let viewer = Vec3::new(0.0, 1.5, -1.0);
    let top = |drawn: &Diagram| {
        let mut sheets = Vec::new();
        Mesh::new(drawn).sheets(model, viewer, None, &mut sheets);
        let sheet = &sheets[0];
        let (_, height) = sheet.canvas.as_ref().unwrap().size();
        sheet
            .model
            .unwrap()
            .transform_point3(Vec3::Y * -height / 2.0)
            .y
    };
    let one = top(&drawn);
    drawn.nodes[0].label = "A\nB\nC".into();
    assert!((top(&drawn) - one).abs() < 1e-5, "{} {one}", top(&drawn));
}

/// Thirty nodes in three ranks deep, each behind another, with groups, as a dense diagram stands.
fn dense() -> Diagram {
    let mut nodes = Vec::new();
    for z in 0..3_i16 {
        for y in 0..2_i16 {
            for x in 0..5_i16 {
                nodes.push((x, y, z));
            }
        }
    }
    let mut drawn = Diagram::default();
    drawn.nodes = nodes
        .iter()
        .enumerate()
        .map(|(index, (x, y, z))| {
            let mut node = Node::new(format!("n{index}"), format!("Node {index}\nrank {z}"));
            node.at = [(x - 2) * 130, (y * 2 - 1) * 90, (1 - z) * 120];
            node.shape = if index % 3 == 0 {
                Shape::Sphere
            } else {
                Shape::Box
            };
            node
        })
        .collect();
    drawn.edges = (0..20)
        .map(|index| {
            let mut edge = Edge::new(index, index + 10);
            edge.label = format!("e{index}");
            edge
        })
        .collect();
    let mut group = Group::new("g", "A group");
    group.low = [-330, -150, -200];
    group.high = [330, 150, 200];
    drawn.groups = vec![group];
    drawn.extent = [780, 420, 440];
    drawn
}

/// Whether anything of the volume `drawn` with `model`, a node's bounds, hides any of a label's corners or its middle
/// from `viewer`.
fn hidden(drawn: &Diagram, model: Mat4, viewer: Vec3, sheet: &Sheet) -> bool {
    let (width, height) = sheet.canvas.as_ref().unwrap().size();
    let placed = sheet.model.unwrap();
    let (scale, ..) = model.to_scale_rotation_translation();
    [
        (0.0, 0.0),
        (-0.5, -0.5),
        (0.5, -0.5),
        (-0.5, 0.5),
        (0.5, 0.5),
    ]
    .into_iter()
    .any(|(x, y)| {
        let point = placed.transform_point3(Vec3::new(x * width, y * height, 0.0));
        let far = point.distance(viewer);
        let ray = Ray::new(viewer, point - viewer).unwrap();
        drawn.nodes.iter().any(|node| {
            let centre = model.transform_point3(Vec3::from_array(node.at.map(f32::from)));
            // Every solid's corners are within three quarters of its width of its middle.
            let radius = node.width() * 0.75 * scale.x;
            let along = (centre - ray.origin).dot(ray.direction);
            let off = (ray.origin + ray.direction * along).distance(centre);
            off < radius && along - (radius * radius - off * off).sqrt() < far
        })
    })
}

#[test]
fn every_label_is_in_front_of_the_volume_s_nodes_from_wherever_it_is_seen() {
    let drawn = dense();
    let mesh = Mesh::new(&drawn);
    // Its group has a colour, which the legend names rather than a label.
    assert_eq!(mesh.labels(), 30 + EDGE_LABELS);
    let model = Mat4::from_translation(Vec3::new(0.0, 1.4, -1.3))
        * Mat4::from_rotation_y(0.3)
        * Mat4::from_scale(Vec3::splat(0.0011));
    let unmoved = label_scale(model);
    let mut hidden_where_they_were = 0;
    for viewer in [
        Vec3::new(0.0, 1.2, 0.0),
        Vec3::new(1.1, 1.6, -0.6),
        Vec3::new(-0.4, 2.2, -0.5),
        Vec3::new(0.3, 1.4, -2.6),
        Vec3::new(0.9, 0.9, -0.4),
        Vec3::new(-0.2, 0.7, -0.9),
    ] {
        let mut sheets = Vec::new();
        mesh.sheets(model, viewer, None, &mut sheets);
        assert_eq!(sheets.len(), mesh.labels());
        let mut last = f32::INFINITY;
        for sheet in &sheets {
            assert!(!hidden(&drawn, model, viewer, sheet), "from {viewer:?}");
            // Seen as large as before, from where it was, the farthest drawn first.
            let placed = sheet.model.unwrap();
            let at = placed.transform_point3(Vec3::ZERO);
            let label = mesh
                .labels
                .iter()
                .find(|label| Arc::ptr_eq(&label.canvas, sheet.canvas.as_ref().unwrap()))
                .unwrap();
            let rows = label.canvas.rows.len() - 1;
            let was = model.transform_point3(label.at)
                + Vec3::Y * (rows * CELL_HEIGHT) as f32 * PIXEL / 2.0 * unmoved;
            let share = placed.x_axis.truncate().length() / unmoved;
            assert!(share <= 1.0 + 1e-5);
            assert!(
                ((at - viewer) - (was - viewer) * share).length() < 1e-4,
                "{at:?} {was:?} {share}"
            );
            // Drawn farthest first by where they stood, so that a nearer node's label is drawn over a farther one's.
            let distance = was.distance(viewer);
            assert!(distance <= last + 1e-5);
            // Never out of the volume's box, grown by its half diagonal, so that what stands in front of it hides it.
            let (width, height) = label.canvas.size();
            let reach = unmoved * width.hypot(height) / 2.0 / 0.0011;
            let within = model.inverse().transform_point3(at).abs();
            let half = Vec3::from_array(drawn.extent.map(f32::from)) / 2.0 + Vec3::splat(reach);
            assert!(within.cmple(half + 1e-2).all(), "{within:?} {half:?}");
            last = distance;
            let mut where_it_was = sheet.clone();
            where_it_was.model = Some(crate::recording::facing(
                was,
                Vec3::new(viewer.x, was.y, viewer.z),
                unmoved,
            ));
            hidden_where_they_were += usize::from(hidden(&drawn, model, viewer, &where_it_was));
        }
    }
    // Where they stood among the nodes, many were hidden.
    assert!(hidden_where_they_were > 10, "{hidden_where_they_were}");
    // A detail card is brought in front too.
    let mut detailed = drawn.clone();
    detailed.nodes[25].detail = "Behind two others".into();
    let viewer = Vec3::new(0.0, 1.2, 0.0);
    let card = Mesh::new(&detailed).card(25, model, viewer).unwrap();
    assert!(!hidden(&drawn, model, viewer, &card));
    assert!(card.model.unwrap().x_axis.truncate().length() < unmoved);
    // The line from a viewer within the box enters it nowhere, nor one that misses it, nor one along a side beyond it.
    let half = Vec3::ONE;
    assert_eq!(entering(Vec3::ZERO, Vec3::X * 3.0, half), Some(0.0));
    assert_eq!(
        entering(Vec3::new(0.0, 0.0, 3.0), Vec3::new(0.0, 0.0, 4.0), half),
        None
    );
    assert_eq!(
        entering(Vec3::new(-3.0, 2.0, 0.0), Vec3::new(3.0, 2.0, 0.0), half),
        None
    );
    assert_eq!(
        entering(Vec3::new(0.0, 0.0, 3.0), Vec3::new(0.0, 0.0, -1.0), half),
        Some(0.5)
    );
}

#[test]
fn leaning_into_a_volume_its_labels_keep_their_distance_from_the_eyes_and_move_smoothly() {
    let drawn = dense();
    let mesh = Mesh::new(&drawn);
    // Its box's front, grown by a label's half diagonal, is about a quarter of a metre in front of its middle.
    let model =
        Mat4::from_translation(Vec3::new(0.0, 1.4, -1.0)) * Mat4::from_scale(Vec3::splat(0.001));
    let unmoved = label_scale(model);
    let seen = |z: f32| {
        let viewer = Vec3::new(0.05, 1.42, z);
        let mut sheets = Vec::new();
        mesh.sheets(model, viewer, None, &mut sheets);
        let mut placed: Vec<(usize, Vec3, f32)> = sheets
            .iter()
            .map(|sheet| {
                let index = mesh
                    .labels
                    .iter()
                    .position(|label| Arc::ptr_eq(&label.canvas, sheet.canvas.as_ref().unwrap()))
                    .unwrap();
                let model = sheet.model.unwrap();
                (
                    index,
                    model.transform_point3(Vec3::ZERO),
                    model.x_axis.truncate().length(),
                )
            })
            .collect();
        placed.sort_by_key(|(index, ..)| *index);
        (viewer, placed)
    };
    // From a metre away, through the box just outside and just inside it, to its middle.
    let mut last: Option<Vec<(usize, Vec3, f32)>> = None;
    let mut moved = 0;
    for step in 0..=120 {
        let (viewer, placed) = seen(0.2 - step as f32 * 0.01);
        for (index, at, scale) in &placed {
            let label = &mesh.labels[*index];
            let rows = label.canvas.rows.len() - 1;
            let was = model.transform_point3(label.at)
                + Vec3::Y * (rows * CELL_HEIGHT) as f32 * PIXEL / 2.0 * unmoved;
            // Never nearer the eyes than 0.4 m, unless it stood nearer, and seen where it stood.
            let distance = at.distance(viewer);
            assert!(
                distance >= NEAREST.min(was.distance(viewer)) - 1e-4,
                "{step}: {distance}"
            );
            assert!(((at - viewer) - (was - viewer) * (scale / unmoved)).length() < 1e-4);
            moved += usize::from(*scale < unmoved - 1e-6);
        }
        // A centimetre's step of the head moves no label far, nor makes it jump as the head enters the box.
        if let Some(last) = &last {
            for ((_, before, _), (_, after, _)) in last.iter().zip(&placed) {
                assert!(
                    before.distance(*after) < 0.05,
                    "{step}: {before:?} {after:?}"
                );
            }
        }
        last = Some(placed);
    }
    assert!(moved > 100, "{moved}");
    // A label already in front of the box stays where it is.
    let viewer = Vec3::new(0.0, 1.4, 1.0);
    let canvas = &mesh.labels[0].canvas;
    let ahead = crate::recording::facing(Vec3::new(0.0, 1.4, 0.2), viewer, unmoved);
    assert_eq!(mesh.before(model, viewer, ahead, canvas), ahead);
}

/// A diagram of a node in each group, nine at the top and one nested, an explicit colour and a node in none.
fn grouped() -> Diagram {
    let mut drawn = diagram(&[], Vec::new());
    for index in 0..9 {
        drawn
            .groups
            .push(Group::new(format!("g{index}"), format!("Group {index}")));
        let mut node = Node::new(format!("n{index}"), "N");
        node.group = Some(index);
        drawn.nodes.push(node);
    }
    let mut nested = Group::new("inner", "");
    nested.parent = Some(0);
    drawn.groups.push(nested);
    let mut node = Node::new("inner-node", "N");
    node.group = Some(9);
    drawn.nodes.push(node);
    let mut own = Node::new("own", "N");
    own.group = Some(1);
    own.colour = tomori_protocol::chart::Colour::Red;
    drawn.nodes.push(own);
    drawn.nodes.push(Node::new("loose", "N"));
    drawn
}

#[test]
fn nodes_take_their_group_s_colour_from_the_palette_nested_paler_and_none_past_it() {
    use tomori_protocol::chart::Colour;
    let drawn = grouped();
    let colours = group_colours(&drawn);
    for (index, colour) in Colour::SERIES.iter().enumerate() {
        assert_eq!(colours[index], Some(colour.rgb()));
    }
    assert_eq!(colours[8], None);
    let paler = colours[9].unwrap();
    let teal = Colour::Teal.rgb();
    assert!(
        (0..3).all(|channel| paler[channel] > teal[channel]),
        "{paler:?}"
    );
    let mesh = Mesh::new(&drawn);
    let mut instances = Vec::new();
    mesh.instances(Mat4::IDENTITY, None, 0.0, Vec3::ZERO, &mut instances);
    let colour = |node: usize| instances[node].colour;
    assert_eq!(colour(0), teal);
    assert_eq!(colour(1), Colour::Amber.rgb());
    // Past the palette, in none, and grey: the node's grey; its own colour, its own.
    assert_eq!(colour(8), NODE_GREY);
    assert_eq!(colour(9), paler);
    assert_eq!(colour(10), Colour::Red.rgb());
    assert_eq!(colour(11), NODE_GREY);
    // Its legend names each group with a colour, the one past the palette left out.
    let entries = legend(&drawn);
    assert_eq!(entries.len(), 9);
    assert_eq!(entries[0].text, "Group 0");
    assert_eq!(
        entries[8],
        Entry {
            text: "inner".into(),
            swatch: Swatch::Colour(paler)
        }
    );
}

#[test]
fn an_edge_within_a_group_is_its_colour_and_between_groups_neutral() {
    let mut drawn = grouped();
    drawn.edges = vec![
        edge(0, 9, Style::Solid, Arrow::Forward),
        edge(1, 10, Style::Solid, Arrow::Forward),
        edge(0, 1, Style::Solid, Arrow::Forward),
        edge(2, 3, Style::Solid, Arrow::Forward),
    ];
    // Within a group: the second group's, though one of its nodes has its own colour.
    drawn.nodes[3].group = Some(2);
    let mut coloured = edge(2, 11, Style::Solid, Arrow::Forward);
    coloured.colour = Some(tomori_protocol::chart::Colour::Violet);
    drawn.edges.push(coloured);
    let mesh = Mesh::new(&drawn);
    let colours: Vec<[f32; 3]> = mesh.edges.iter().map(|(_, colour)| *colour).collect();
    let within =
        |colour: tomori_protocol::chart::Colour| colour.rgb().map(|channel| channel * WITHIN);
    use tomori_protocol::chart::Colour;
    assert_eq!(
        colours,
        [
            EDGE,
            within(Colour::Amber),
            EDGE,
            within(Colour::Blue),
            Colour::Violet.rgb()
        ]
    );
}

#[test]
fn an_edge_ends_on_the_surface_of_each_solid_as_it_is_drawn() {
    // The point `surface` finds is on each solid's surface, whichever way.
    let directions: Vec<Vec3> = (0..40)
        .map(|index| {
            let a = index as f32 * 0.7;
            let b = index as f32 * 1.3;
            Vec3::new(a.cos() * b.sin(), b.cos(), a.sin() * b.sin()).normalize()
        })
        .chain([Vec3::X, Vec3::Y, Vec3::Z, -Vec3::Y])
        .collect();
    for shape in [
        Shape::Box,
        Shape::Card,
        Shape::Sphere,
        Shape::Cylinder,
        Shape::Prism,
        Shape::Diamond,
    ] {
        let (solid, scale, basis) = body(shape, 60.0);
        let half = scale / 2.0;
        for direction in &directions {
            let local = basis.transpose() * *direction;
            let point = local * surface(solid, scale, local);
            let p = point.abs() / half;
            // How far out the point is, where 1 is the surface.
            let out = match solid {
                Solid::Sphere => point.length() / half.x / 0.92,
                Solid::Diamond => p.x + p.y + p.z,
                Solid::Cylinder | Solid::Prism => {
                    let sides = if solid == Solid::Cylinder { 12.0 } else { 6.0 };
                    let sector = std::f32::consts::TAU / sides;
                    let angle = point.z.atan2(point.x).rem_euclid(sector);
                    let across = point.x.hypot(point.z) * (angle - sector / 2.0).cos()
                        / (half.x * (sector / 2.0).cos());
                    across.max(p.y)
                }
                _ => p.max_element(),
            };
            assert!((out - 1.0).abs() < 1e-3, "{shape:?} {direction} {out}");
        }
    }
    // An edge from a box above to a short cylinder below starts and ends within a millimetre of their faces.
    let drawn = diagram(
        &[
            ("a", Shape::Box, [0, 200, 0]),
            ("b", Shape::Cylinder, [0, 0, 0]),
            ("c", Shape::Prism, [200, 0, 0]),
        ],
        vec![
            edge(0, 1, Style::Solid, Arrow::None),
            edge(2, 1, Style::Solid, Arrow::Back),
        ],
    );
    let mesh = Mesh::new(&drawn);
    let path = &mesh.edges[0].0;
    assert!(
        (path[0].y - (200.0 - 21.0 + EMBED_MM)).abs() < 1e-3,
        "{:?}",
        path[0]
    );
    assert!(
        (path[1].y - (24.0 - EMBED_MM)).abs() < 1e-3,
        "{:?}",
        path[1]
    );
    // Pointing back, it runs from the cylinder to the prism, whose hexagon faces the front, its point to the side.
    let back = &mesh.edges[1].0;
    assert!(
        (back[0].x - (30.0 - EMBED_MM)).abs() < 1e-3,
        "{:?}",
        back[0]
    );
    assert!((back[back.len() - 1].x - (200.0 - 30.0 + EMBED_MM)).abs() < 1e-3);
    // Straight on, nowhere is there no surface.
    assert_eq!(surface(Solid::Box, Vec3::ONE, Vec3::Y), 0.5);
}

#[test]
fn an_edge_narrows_towards_where_it_points_curving_through_its_waypoints() {
    let mut drawn = diagram(
        &[
            ("a", Shape::Sphere, [-200, 0, 0]),
            ("b", Shape::Sphere, [200, 0, 0]),
        ],
        vec![edge(0, 1, Style::Solid, Arrow::Forward)],
    );
    drawn.edges[0].via = vec![[-60, 80, 0], [60, -80, 0]];
    let mesh = Mesh::new(&drawn);
    let pieces: Vec<&Part> = mesh
        .parts
        .iter()
        .filter(|part| part.solid == Solid::Tube)
        .collect();
    assert_eq!(pieces.len(), TAPER_PIECES);
    // Each narrower than the last, by as much, from about the source's width to about the target's.
    let step = (TAPER_MM.1 / TAPER_MM.0).powf(1.0 / TAPER_PIECES as f32);
    for pair in pieces.windows(2) {
        assert!((pair[1].scale.x / pair[0].scale.x - step).abs() < 1e-3);
    }
    assert!(pieces[0].scale.x < TAPER_MM.0 * 2.0 && pieces[0].scale.x > TAPER_MM.0 * 2.0 * step);
    // Straight, it is one narrowing tube, as wide as the edge at its source.
    let straight = Mesh::new(&diagram(
        &[
            ("a", Shape::Sphere, [-200, 0, 0]),
            ("b", Shape::Sphere, [200, 0, 0]),
        ],
        vec![edge(0, 1, Style::Solid, Arrow::Forward)],
    ));
    assert_eq!(straight.parts[2].solid, Solid::Taper);
    assert!((straight.parts[2].scale.x - TAPER_MM.0 * 2.0).abs() < 1e-4);
    assert!((TAPER_RATIO - TAPER_MM.1 / TAPER_MM.0).abs() < 1e-6);
    // Its arrowheads point where it points; one of no length has none.
    let cones: Vec<&Part> = straight
        .parts
        .iter()
        .filter(|part| part.solid == Solid::Cone)
        .collect();
    assert_eq!(cones.len(), 2);
    assert!(cones.iter().all(|cone| (cone.basis * Vec3::Y).x > 0.99));
    let together = Mesh::new(&diagram(
        // Two boxes whose faces, less what an edge reaches into them, meet.
        &[("a", Shape::Box, [0, 0, 0]), ("b", Shape::Box, [58, 0, 0])],
        vec![edge(0, 1, Style::Solid, Arrow::Forward)],
    ));
    assert!(together.parts.iter().all(|part| part.solid != Solid::Cone));
    // Its path is a smooth curve through both waypoints.
    let path = &mesh.edges[0].0;
    assert_eq!(path.len(), 3 * CURVE_PIECES + 1);
    for waypoint in [Vec3::new(-60.0, 80.0, 0.0), Vec3::new(60.0, -80.0, 0.0)] {
        assert!(path.iter().any(|point| (*point - waypoint).length() < 1e-3));
    }
    // Dashed, its dashes narrow too; dotted, its dots.
    for style in [Style::Dashed, Style::Dotted] {
        drawn.edges[0].style = style;
        let mesh = Mesh::new(&drawn);
        let widths: Vec<f32> = mesh
            .parts
            .iter()
            .filter(|part| matches!(part.owner, Owner::Edge(_)) && part.solid != Solid::Cone)
            .map(|part| part.scale.x)
            .collect();
        assert!(widths.first() > widths.last(), "{style:?} {widths:?}");
    }
}

#[test]
fn beads_move_along_the_focused_node_s_edges_that_point() {
    let drawn = diagram(
        &[
            ("a", Shape::Box, [-200, 0, 0]),
            ("b", Shape::Box, [0, 0, 0]),
            ("c", Shape::Box, [200, 0, 0]),
        ],
        vec![
            edge(0, 1, Style::Solid, Arrow::Forward),
            edge(1, 2, Style::Solid, Arrow::None),
            edge(2, 0, Style::Flow, Arrow::Forward),
        ],
    );
    let mesh = Mesh::new(&drawn);
    let beads = |focus: Option<usize>| {
        let mut instances = Vec::new();
        mesh.instances(Mat4::IDENTITY, focus, 0.3, Vec3::ZERO, &mut instances);
        counted(&instances, Solid::Sphere)
    };
    // The flow's always; focused on b, its edge from a too, but not the one that points nowhere.
    assert_eq!(beads(None), BEADS);
    assert_eq!(beads(Some(1)), 2 * BEADS);
    assert_eq!(beads(Some(0)), 2 * BEADS);
}

#[test]
fn a_legend_of_groups_and_kinds_stands_below_the_diagram_with_a_solid_for_each_kind() {
    let mut drawn = grouped();
    drawn.groups.truncate(2);
    for node in &mut drawn.nodes {
        node.group = node.group.filter(|group| *group < 2);
    }
    for (index, kind, shape) in [
        (0, "store", Shape::Cylinder),
        (1, "store", Shape::Cylinder),
        (2, "module", Shape::Box),
        (3, "process", Shape::Prism),
    ] {
        drawn.nodes[index].kind = kind.into();
        drawn.nodes[index].shape = shape;
    }
    let entries = legend(&drawn);
    let texts: Vec<&str> = entries.iter().map(|entry| entry.text.as_str()).collect();
    assert_eq!(texts, ["Group 0", "Group 1", "store", "process"]);
    let panel = Panel::new(
        "panel-1",
        "Kinds",
        "",
        Place::Free {
            pose: Pose::default(),
        },
    )
    .with_diagram(drawn.clone());
    let face = face(&panel, &drawn, Fit::Text, None);
    let text: Vec<String> = face
        .rows
        .iter()
        .map(|row| crate::ui_text::plain(&row.cells))
        .collect();
    assert!(
        text[text.len() - 2].contains("Group 0")
            && text[text.len() - 2].contains("store")
            && text[text.len() - 1].contains("process"),
        "{text:?}"
    );
    // Its rows add to a floating face's, and the diagram stands above them.
    let mut bare = drawn.clone();
    bare.groups.clear();
    for node in &mut bare.nodes {
        node.group = None;
        node.kind.clear();
    }
    assert!(legend(&bare).is_empty());
    let (_, rows) = fit(&panel, &drawn, Fit::Text);
    let (_, bare_rows) = fit(&panel, &bare, Fit::Text);
    assert_eq!(rows, bare_rows + 2 + LEGEND_GAP);
    // Room kept below the heading for the labels above its highest nodes.
    assert!(rows >= LABEL_ROOM + 2 + LEGEND_GAP + FIT_ROWS.0);
    // As large, and half its rows higher about the face's middle, as the face grows by them below it.
    let (with, without) = (
        placed(&panel, &drawn, Fit::Text),
        placed(&panel, &bare, Fit::Text),
    );
    assert_eq!(with.x_axis, without.x_axis);
    let higher = with.w_axis.y - without.w_axis.y;
    assert!(
        (higher - (2 + LEGEND_GAP) as f32 / 2.0 * (CELL_HEIGHT as f32) * PIXEL).abs() < 1e-5,
        "{higher}"
    );
    // A solid for each kind beside its name, on the face, as its nodes are drawn.
    let mut icons = Vec::new();
    legend_icons(
        &panel,
        &Mesh::new(&drawn),
        Fit::Text,
        Mat4::from_translation(Vec3::Z),
        &mut icons,
    );
    let solids: Vec<Solid> = icons.iter().map(|icon| icon.solid).collect();
    assert_eq!(solids, [Solid::Cylinder, Solid::Prism]);
    assert!(
        icons
            .iter()
            .all(|icon| icon.position.z > 1.0 && icon.scale.max_element() < 0.02)
    );
    // Too many to fit its rows, it counts the rest.
    let many: Vec<Entry> = (0..40)
        .map(|index| Entry {
            text: format!("Group {index}"),
            swatch: Swatch::Colour([1.0; 3]),
        })
        .collect();
    let widths: Vec<usize> = many.iter().map(Entry::cells).collect();
    let (rows, more) = legend_rows(&widths, 30, LEGEND_ROWS);
    assert_eq!(rows.len(), LEGEND_ROWS);
    assert_eq!(more, 40 - rows.iter().map(Vec::len).sum::<usize>());
    assert_eq!(legend_widths(&drawn), widths_of(&legend(&drawn)));
    let mut crowded = drawn.clone();
    for index in 2..8 {
        crowded
            .groups
            .push(Group::new(format!("a-long-group-{index}"), ""));
    }
    let panel = panel.with_diagram(crowded.clone());
    let face = super::face(
        &panel,
        &crowded,
        Fit::Slot(super::super::Cells::STANDARD),
        None,
    );
    let text: Vec<String> = face
        .rows
        .iter()
        .map(|row| crate::ui_text::plain(&row.cells))
        .collect();
    // On a standard slot, it takes at most a quarter of its body with the rows above it, counting the rest.
    let shown: Vec<&String> = text.iter().filter(|row| !row.trim().is_empty()).collect();
    assert_eq!(shown.len(), 3 + 1, "{text:?}");
    assert!(shown[3].contains('+'), "{text:?}");
    // On a slot too small to spare a quarter, none.
    let tiny = super::super::Cells {
        body: 8,
        ..super::super::Cells::STANDARD
    };
    let face = super::face(&panel, &crowded, Fit::Slot(tiny), None);
    assert!(
        face.rows[3..]
            .iter()
            .all(|row| crate::ui_text::plain(&row.cells).trim().is_empty())
    );
}

fn widths_of(entries: &[Entry]) -> Vec<usize> {
    entries.iter().map(Entry::cells).collect()
}
