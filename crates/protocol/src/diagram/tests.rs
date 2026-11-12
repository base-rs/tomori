// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{Panel, Place, Pose};

fn texts(warnings: &[Warning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}

#[test]
fn a_diagram_has_a_compact_wire_format_and_reads_newer_values_as_defaults() {
    let (mut diagram, _) = parse("a[Renderer] -.-> b((Octree)) : queries\n");
    diagram.nodes[1].at = [10, -20, 30];
    diagram.edges[0].via = vec![[1, 2, 3]];
    diagram.extent = [400, 300, 200];
    let json = serde_json::to_value(&diagram).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "layout": {"kind": "layered", "direction": "down"},
            "nodes": [
                {"id": "a", "label": "Renderer", "shape": "box", "colour": "grey", "at": [0, 0, 0]},
                {"id": "b", "label": "Octree", "shape": "sphere", "colour": "grey", "at": [10, -20, 30]}
            ],
            "edges": [{"from": 0, "to": 1, "label": "queries", "direction": "forward", "style": "dashed", "via": [[1, 2, 3]]}],
            "extent": [400, 300, 200]
        })
    );
    let back: Diagram = serde_json::from_value(json).unwrap();
    assert_eq!(back, diagram);
    // A newer server's shapes, styles, arrows, kinds and layouts read as what every client draws.
    let newer: Diagram = serde_json::from_value(serde_json::json!({
        "layout": {"kind": "spiral"},
        "nodes": [{"id": "a", "label": "A", "shape": "torus", "colour": "mauve", "size": 9}],
        "edges": [{"from": 0, "to": 0, "style": "zigzag", "direction": "sideways"}],
        "groups": [{"id": "g", "kind": "ring"}],
        "newer": true
    }))
    .unwrap();
    assert_eq!(newer.layout, Layout::Unknown);
    assert_eq!(newer.nodes[0].shape, Shape::Box);
    assert_eq!(newer.nodes[0].colour, Colour::Grey);
    assert_eq!((newer.edges[0].style, newer.edges[0].direction), (Style::Solid, Arrow::Forward));
    assert_eq!(newer.groups[0].kind, GroupKind::Cluster);
    let rank: Layout = serde_json::from_value(serde_json::json!({"kind": "layered", "direction": "diagonal"})).unwrap();
    assert_eq!(rank, Layout::default());
    // A panel is a volume with one, and its body is the outline.
    let panel = Panel::new("panel-1", "Renderer", diagram.outline(), Place::Free { pose: Pose::default() })
        .with_diagram(diagram.clone());
    assert!(panel.same_structure(&panel.clone()));
    let mut moved = panel.clone();
    moved.diagram.as_mut().unwrap().nodes[0].at = [5, 5, 5];
    assert!(!panel.same_structure(&moved));
}

#[test]
fn layouts_are_named_as_blocks_and_the_tool_name_them() {
    for (text, layout) in [
        ("layered", Layout::Layered { direction: Rank::Down }),
        ("layered up", Layout::Layered { direction: Rank::Up }),
        ("layered left", Layout::Layered { direction: Rank::Left }),
        ("layered away", Layout::Layered { direction: Rank::Away }),
        ("TD", Layout::Layered { direction: Rank::Down }),
        ("RL", Layout::Layered { direction: Rank::Left }),
        ("radial", Layout::Radial { hub: None }),
        ("Force", Layout::Force),
        ("grid", Layout::Grid),
    ] {
        assert_eq!(Layout::parse(text), Some(layout), "{text}");
    }
    assert_eq!(Layout::parse(""), None);
    assert_eq!(Layout::parse("layered sideways"), None);
}

#[test]
fn attributes_keep_the_first_of_each_and_name_what_they_cannot_set() {
    let mut attributes = Attributes::default();
    for (key, value) in [
        ("label", "First"),
        ("title", "Second"),
        ("shape", "circle"),
        ("colour", "#102030"),
        ("size", "3"),
        ("detail", "What it does"),
        ("reference", "crates/x"),
        ("color", "red"),
    ] {
        attributes.set(key, value).unwrap();
    }
    assert_eq!(attributes.label.as_deref(), Some("First"));
    assert_eq!(attributes.shape, Some(Shape::Sphere));
    assert_eq!(attributes.colour, Some(Colour::Rgb([16, 32, 48])));
    assert_eq!(attributes.size, Some(3));
    assert_eq!(attributes.reference.as_deref(), Some("crates/x"));
    assert_eq!(
        [
            attributes.set("size", "9"),
            attributes.set("colour", "mauve"),
            attributes.set("fill", "#fff"),
        ],
        [
            Err("size \"9\" is not from 1 to 5".to_owned()),
            Err("\"mauve\" is not a colour of the palette: teal, amber, blue, violet, green, red, grey or white".to_owned()),
            Err("\"fill\" is not an attribute of a node".to_owned()),
        ]
    );
    assert_eq!(colour("Gray"), Some(Colour::Grey));
    for (name, shape) in [
        ("cylinder", Shape::Cylinder),
        ("hexagon", Shape::Prism),
        ("diamond", Shape::Diamond),
        ("card", Shape::Card),
        ("box", Shape::Box),
    ] {
        assert_eq!(super::shape(name), Some(shape));
    }
    for (name, kind) in [("layer", GroupKind::Layer), ("container", GroupKind::Container), ("cluster", GroupKind::Cluster)] {
        assert_eq!(group_kind(name), Some(kind));
    }
    for (name, style) in [("solid", Style::Solid), ("dashed", Style::Dashed), ("dotted", Style::Dotted), ("thick", Style::Thick), ("flow", Style::Flow)] {
        assert_eq!(super::style(name), Some(style));
    }
    for (name, direction) in [("forward", Arrow::Forward), ("back", Arrow::Back), ("both", Arrow::Both), ("none", Arrow::None)] {
        assert_eq!(arrow(name), Some(direction));
    }
    assert_eq!((super::style("wavy"), arrow("up"), group_kind("ring")), (None, None, None));
}

#[test]
fn the_tool_s_json_is_read_as_a_block_is_warning_of_what_it_names_that_is_not_there() {
    let (diagram, warnings) = Diagram::from_json(&serde_json::json!({
        "layout": {"kind": "layered", "direction": "right"},
        "groups": [
            {"id": "render", "label": "tomori-render", "kind": "container"},
            {"id": "gpu", "parent": "render", "kind": "ring"}
        ],
        "nodes": [
            {"id": "octree", "label": "Octree", "shape": "sphere", "size": 3, "colour": "teal", "group": "render", "detail": "Spatial index", "ref": "crates/octree"},
            {"id": "renderer", "label": "Renderer", "group": "nowhere", "shape": "blob"},
            {"id": "bad id!"}
        ],
        "edges": [
            {"from": "renderer", "to": "octre", "label": "queries", "style": "flow", "direction": "both", "colour": "amber", "weight": 2},
            {"from": "renderer", "to": "octree", "style": "wavy", "direction": "up", "weight": "x"},
            {"from": "", "to": "octree"}
        ]
    }))
    .unwrap();
    assert_eq!(diagram.layout, Layout::Layered { direction: Rank::Right });
    assert_eq!(diagram.groups[0].kind, GroupKind::Container);
    assert_eq!(diagram.groups[1].parent, Some(0));
    assert_eq!(diagram.nodes[0].group, Some(0));
    assert_eq!(diagram.nodes[0].detail, "Spatial index");
    assert_eq!(diagram.nodes.len(), 3, "the edge to octre made a box of it");
    assert_eq!(diagram.edges[0].style, Style::Flow);
    assert_eq!(diagram.edges[0].direction, Arrow::Both);
    assert_eq!(diagram.edges[0].colour, Some(Colour::Amber));
    assert_eq!(diagram.edges[0].weight, 2);
    assert_eq!(diagram.edges[1].weight, 1);
    assert_eq!(
        texts(&warnings),
        [
            "\"ring\" is not a kind of group: a cluster, layer or container",
            "node renderer: \"blob\" is not a shape: a box, sphere, cylinder, prism, diamond or card",
            "node renderer: \"nowhere\" is not a group",
            "\"bad id!\" is not a node's id: from 1 to 32 letters, digits, -, _ and .",
            "\"octre\" is not a node; made a box called octre (did you mean octree?)",
            "\"wavy\" is not a style of edge: solid, dashed, dotted, thick or flow",
            "\"up\" is not a direction of edge: forward, back, both or none",
            "the edge from \"\" to \"octree\" is left out: each end must be a node's id",
        ]
    );
    // A layout by name, an unknown one, and a node named nowhere else.
    let (diagram, warnings) = Diagram::from_json(&serde_json::json!({
        "layout": "force",
        "edges": [{"from": "a", "to": "b"}],
        "nodes": null
    }))
    .unwrap();
    assert_eq!(diagram.layout, Layout::Force);
    assert_eq!(
        texts(&warnings),
        ["\"a\" is not a node; made a box called a", "\"b\" is not a node; made a box called b"]
    );
    let (diagram, warnings) = Diagram::from_json(&serde_json::json!({"layout": "spiral", "nodes": [{"id": "a", "size": 2}]})).unwrap();
    assert_eq!(diagram.layout, Layout::default());
    assert_eq!(diagram.nodes[0].size, 2);
    assert_eq!(
        texts(&warnings),
        ["\"spiral\" is not a layout: layered with down, up, right, left or away, radial, force or grid; laid out layered down"]
    );
    assert_eq!(
        Diagram::from_json(&serde_json::json!({"nodes": {"id": "a"}})),
        Err("nodes must be a list.".to_owned())
    );
}

#[test]
fn bounds_keep_what_fits_in_the_order_written_and_count_the_rest() {
    let mut body: String = (0..70).map(|index| format!("n{index}\n")).collect();
    for _ in 0..6 {
        body.push_str("n0 --> n1\n");
    }
    for index in 0..140 {
        body.push_str(&format!("n{} --> n{}\n", index % 60, (index * 7 + 1) % 60));
    }
    body.push_str("n69 --> n0\nn2 --> n2\n");
    for index in 0..14 {
        body.push_str(&format!("subgraph g{index}\nend\n"));
    }
    body.push_str("subgraph a\nsubgraph b\nsubgraph c\nsubgraph d\nn3\nend\nend\nend\nend\n");
    let (diagram, warnings) = parse(&body);
    assert_eq!(diagram.nodes.len(), NODES);
    assert_eq!(diagram.edges.len(), EDGES);
    assert_eq!(diagram.groups.len(), GROUPS);
    assert!(
        diagram
            .edges
            .iter()
            .all(|edge| usize::from(edge.from) < NODES && usize::from(edge.to) < NODES)
    );
    let texts = texts(&warnings);
    assert!(texts.contains(&"the edge from n2 to itself is left out".to_owned()), "{texts:?}");
    assert!(texts.iter().any(|text| text.ends_with("edges left out: at most 4 between one pair of nodes")));
    assert!(texts.contains(&"nodes 65 to 70 left out: at most 64".to_owned()), "{texts:?}");
    assert!(texts.iter().any(|text| text.starts_with("edges 129 to ")), "{texts:?}");
    assert!(texts.contains(&"groups 13 to 17 left out: at most 12".to_owned()), "{texts:?}");
    assert!(texts.iter().any(|text| text.ends_with("group \"d\" nested 4 deep: its members put in \"c\"")), "{texts:?}");
    let many: Vec<Warning> = warnings.iter().cycle().take(WARNINGS + 3).cloned().collect();
    assert_eq!(told(&many).lines().count(), WARNINGS + 1);
}

#[test]
fn text_is_cleaned_and_cut() {
    let long = "x".repeat(300);
    let body = format!(
        "a[{long}]@{{ detail: {long}, ref: {} }}\na --> b : {long}\nsubgraph g [{long}]\nend\n",
        "é".repeat(200)
    );
    let (diagram, _) = parse(&body);
    assert_eq!(diagram.nodes[0].label.chars().count(), LABEL_CHARS + 1);
    assert_eq!(diagram.nodes[0].detail.chars().count(), DETAIL_CHARS + 1);
    assert!(diagram.nodes[0].reference.len() <= REF_BYTES);
    assert_eq!(diagram.edges[0].label.chars().count(), EDGE_LABEL_CHARS + 1);
    assert_eq!(diagram.groups[0].label.chars().count(), LABEL_CHARS + 1);
    // An empty label is the id's.
    let (diagram, _) = parse("a[ ]\n");
    assert_eq!(diagram.nodes[0].label, "a");
    assert_eq!(cut_bytes("éé", 3), "é");
}

#[test]
fn a_client_draws_a_diagram_within_bounds_whatever_it_is_sent() {
    let mut node = Node::new("a", "x".repeat(100));
    node.size = 0;
    node.group = Some(40);
    let mut nodes = vec![node; 70];
    nodes[1].size = 9;
    let mut edge = Edge::new(0, 1);
    edge.via = vec![[0; 3]; 9];
    edge.weight = 9;
    let mut groups = vec![Group::new("g", "G"); 14];
    groups[0].parent = Some(3);
    groups[2].parent = Some(1);
    let diagram = Diagram {
        layout: Layout::Force,
        nodes,
        edges: vec![edge.clone(), Edge::new(0, 90), Edge::new(2, 2)].into_iter().chain(std::iter::repeat_n(edge, 200)).collect(),
        groups,
        extent: [u16::MAX, 10, 10],
    }
    .bounded();
    assert_eq!((diagram.nodes.len(), diagram.edges.len(), diagram.groups.len()), (NODES, EDGES, GROUPS));
    assert_eq!((diagram.nodes[0].size, diagram.nodes[1].size), (1, SIZES));
    assert_eq!(diagram.nodes[0].group, None);
    assert_eq!(diagram.nodes[0].label.chars().count(), LABEL_CHARS + 1);
    assert_eq!((diagram.groups[0].parent, diagram.groups[2].parent), (None, Some(1)));
    assert_eq!(diagram.edges[0].via.len(), WAYPOINTS);
    assert_eq!(diagram.edges[0].weight, WEIGHTS);
    assert_eq!(diagram.extent[0], EXTENT_MM);
    assert!((diagram.nodes[1].width() - 2.0 * NODE_MM).abs() < 1e-3);
}

#[test]
fn edges_are_named_by_their_ends_and_label() {
    let (diagram, _) = parse("a[Frame step] --> b[faces] : panels changed\nb <-- c\nc --- a\n");
    assert_eq!(diagram.edge_name(0).as_deref(), Some("Frame step → faces: panels changed"));
    assert_eq!(diagram.edge_name(1).as_deref(), Some("faces ← c"));
    assert_eq!(diagram.edge_name(2).as_deref(), Some("c → Frame step"));
    assert_eq!(diagram.edge_name(3), None);
    let mut broken = diagram.clone();
    broken.edges[0].to = 9;
    assert_eq!(broken.edge_name(0), None);
    // The outline names every kind of edge, and a group by its id where it has no label, and skips an edge to nothing.
    let (mut diagram, _) = parse("a@{ detail: First }\na <--> b\na --- b\nsubgraph g\nc\nend\n");
    diagram.groups[0].label.clear();
    diagram.edges.push(Edge::new(0, 9));
    assert_eq!(
        diagram.outline(),
        "- a: First\n- b\n\n**g**\n- c\n\n**Edges**\n- a ↔ b\n- a — b"
    );
    assert_eq!(Diagram::default().outline(), "");
}

#[test]
fn builder_groups_by_id_and_names_its_kind() {
    let mut builder = Builder::new();
    let first = builder.group("g", "G", GroupKind::Cluster, None);
    assert_eq!(builder.group("g", "Again", GroupKind::Layer, None), first);
    builder.group_kind(first.unwrap(), GroupKind::Layer);
    builder.group_kind(40, GroupKind::Layer);
    let (diagram, warnings) = builder.finish();
    assert!(warnings.is_empty());
    assert_eq!(diagram.groups.len(), 1);
    assert_eq!(diagram.groups[0].kind, GroupKind::Layer);
    assert_eq!(edits("kitten", "sitting"), 3);
}

#[test]
fn a_block_warns_of_a_node_named_close_to_another_and_the_outline_names_its_groups() {
    let (diagram, warnings) = parse("octree((Octree))@{ size: 2, }\nrenderer --> octre\nsubgraph s [Scene]\nrenderer <-- leaf\nend\n");
    assert_eq!(
        texts(&warnings),
        ["line 2: \"octre\" is not a node; made a box called octre (did you mean octree?)"]
    );
    assert_eq!(
        diagram.outline(),
        // As in Mermaid, a node mentioned in a subgraph is its member, wherever it was first mentioned.
        "- Octree\n- octre\n\n**Scene**\n- renderer\n- leaf\n\n**Edges**\n- renderer → octre\n- renderer ← leaf"
    );
    // A label that is not text, such as a list, is none.
    let (diagram, _) = Diagram::from_json(&serde_json::json!({"nodes": [{"id": "a", "label": ["A"], "size": 2}]})).unwrap();
    assert_eq!((diagram.nodes[0].label.as_str(), diagram.nodes[0].size), ("a", 2));
    assert_eq!(
        serde_json::to_value(crate::Resource::volume_at("panel-7", Some("octree"), Some(2))).unwrap(),
        serde_json::json!({"kind": "panel", "panel": "panel-7", "node": "octree", "edge": 2})
    );
}

#[test]
fn what_cannot_be_named_is_left_out_quietly() {
    assert_eq!(told(&[Warning { line: Some(2), text: "x".into() }]), "line 2: x");
    // An id too long to be a node's makes no edge.
    let long = "n".repeat(ID_BYTES + 1);
    let (diagram, warnings) = parse(&format!("a --> {long}\n"));
    assert!(diagram.edges.is_empty());
    assert_eq!(diagram.nodes.len(), 1);
    assert!(!warnings.is_empty());
    for layout in [
        serde_json::json!({"kind": "radial", "hub": "hub"}),
        serde_json::json!({"kind": "force"}),
    ] {
        let (diagram, warnings) = Diagram::from_json(&serde_json::json!({"layout": layout})).unwrap();
        assert!(warnings.is_empty());
        assert_ne!(diagram.layout, Layout::default());
    }
    assert!(Diagram::from_json(&serde_json::json!({"groups": 3})).is_err());
    assert!(Diagram::from_json(&serde_json::json!({"edges": "a --> b"})).is_err());
    let (mut diagram, _) = parse("a --> b\n");
    diagram.edges.push(Edge::new(9, 0));
    assert_eq!(diagram.outline(), "- a\n- b\n\n**Edges**\n- a → b");
    assert_eq!(diagram.edge_name(1), None);
}
