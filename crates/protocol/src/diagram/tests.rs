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
    assert_eq!(
        (newer.edges[0].style, newer.edges[0].direction),
        (Style::Solid, Arrow::Forward)
    );
    assert_eq!(newer.groups[0].kind, GroupKind::Cluster);
    let rank: Layout =
        serde_json::from_value(serde_json::json!({"kind": "layered", "direction": "diagonal"}))
            .unwrap();
    assert_eq!(rank, Layout::default());
    // A panel is a volume with one, and its body is the outline.
    let panel = Panel::new(
        "panel-1",
        "Renderer",
        diagram.outline(),
        Place::Free {
            pose: Pose::default(),
        },
    )
    .with_diagram(diagram.clone());
    assert!(panel.same_structure(&panel.clone()));
    let mut moved = panel.clone();
    moved.diagram.as_mut().unwrap().nodes[0].at = [5, 5, 5];
    assert!(!panel.same_structure(&moved));
}

#[test]
fn layouts_are_named_as_blocks_and_the_tool_name_them() {
    for (text, layout) in [
        (
            "layered",
            Layout::Layered {
                direction: Rank::Down,
            },
        ),
        (
            "layered up",
            Layout::Layered {
                direction: Rank::Up,
            },
        ),
        (
            "layered left",
            Layout::Layered {
                direction: Rank::Left,
            },
        ),
        (
            "layered away",
            Layout::Layered {
                direction: Rank::Away,
            },
        ),
        (
            "TD",
            Layout::Layered {
                direction: Rank::Down,
            },
        ),
        (
            "RL",
            Layout::Layered {
                direction: Rank::Left,
            },
        ),
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
            Err("\"mauve\" is not a colour of the palette: teal, amber, blue, violet, green, red, yellow, grey or white".to_owned()),
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
    for (name, kind) in [
        ("layer", GroupKind::Layer),
        ("container", GroupKind::Container),
        ("cluster", GroupKind::Cluster),
    ] {
        assert_eq!(group_kind(name), Some(kind));
    }
    for (name, style) in [
        ("solid", Style::Solid),
        ("dashed", Style::Dashed),
        ("dotted", Style::Dotted),
        ("thick", Style::Thick),
        ("flow", Style::Flow),
    ] {
        assert_eq!(super::style(name), Some(style));
    }
    for (name, direction) in [
        ("forward", Arrow::Forward),
        ("back", Arrow::Back),
        ("both", Arrow::Both),
        ("none", Arrow::None),
    ] {
        assert_eq!(arrow(name), Some(direction));
    }
    assert_eq!(
        (super::style("wavy"), arrow("up"), group_kind("ring")),
        (None, None, None)
    );
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
    assert_eq!(
        diagram.layout,
        Layout::Layered {
            direction: Rank::Right
        }
    );
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
        [
            "\"a\" is not a node; made a box called a",
            "\"b\" is not a node; made a box called b"
        ]
    );
    let (diagram, warnings) = Diagram::from_json(
        &serde_json::json!({"layout": "spiral", "nodes": [{"id": "a", "size": 2}]}),
    )
    .unwrap();
    assert_eq!(diagram.layout, Layout::default());
    assert_eq!(diagram.nodes[0].size, 2);
    assert_eq!(
        texts(&warnings),
        [
            "\"spiral\" is not a layout: layered with down, up, right, left or away, radial, force or grid; laid out layered down"
        ]
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
    assert!(
        texts.contains(&"the edge from n2 to itself is left out".to_owned()),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.ends_with("edges left out: at most 4 between one pair of nodes"))
    );
    assert!(
        texts.contains(&"nodes 65 to 70 left out: at most 64".to_owned()),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text.starts_with("edges 129 to ")),
        "{texts:?}"
    );
    assert!(
        texts.contains(&"groups 13 to 17 left out: at most 12".to_owned()),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.ends_with("group \"d\" nested 4 deep: its members put in \"c\"")),
        "{texts:?}"
    );
    let many: Vec<Warning> = warnings
        .iter()
        .cycle()
        .take(WARNINGS + 3)
        .cloned()
        .collect();
    assert_eq!(told(&many).lines().count(), WARNINGS + 1);
}

#[test]
fn a_block_of_many_mentions_reads_within_its_bounds_whatever_it_writes() {
    // Two sides of 32 joined by & are 1,024 edges: the first read are kept, and the rest only counted.
    let side = |from: usize| -> String {
        (from..from + 32)
            .map(|index| format!("n{index}"))
            .collect::<Vec<_>>()
            .join(" & ")
    };
    let (diagram, warnings) = parse(&format!("{} --> {}\n", side(0), side(32)));
    assert_eq!((diagram.nodes.len(), diagram.edges.len()), (NODES, EDGES));
    assert!(
        texts(&warnings).contains(&"edges 129 to 1024 left out: at most 128".to_owned()),
        "{warnings:?}"
    );
    // Sides of thousands, as 16 KiB of a block can write, millions of pairs, are counted without being kept, and ids
    // past the nodes kept are not looked over for misspellings.
    let ids: Vec<String> = (0..2_000).map(|index| format!("x{index:03}")).collect();
    let line = format!("{} --> {}", ids[..1_000].join("&"), ids[1_000..].join("&"));
    let (diagram, warnings) = parse(&line);
    assert_eq!((diagram.nodes.len(), diagram.edges.len()), (NODES, 0));
    assert!(
        texts(&warnings).contains(&"nodes 65 to 2000 left out: at most 64".to_owned()),
        "{warnings:?}"
    );
    assert!(warnings.len() < NODES + 4, "{}", warnings.len());
    // The tool's JSON is held to as many edges read.
    let nodes: Vec<serde_json::Value> = (0..40)
        .map(|index| serde_json::json!({"id": format!("n{index}")}))
        .collect();
    let edges: Vec<serde_json::Value> = (0..40)
        .flat_map(|a| {
            (a + 1..40)
                .map(move |b| serde_json::json!({"from": format!("n{a}"), "to": format!("n{b}")}))
        })
        .take(600)
        .collect();
    let (diagram, warnings) =
        Diagram::from_json(&serde_json::json!({"nodes": nodes, "edges": edges})).unwrap();
    assert_eq!(diagram.edges.len(), EDGES);
    assert_eq!(texts(&warnings), ["edges 129 to 600 left out: at most 128"]);
    assert_eq!(EDGE_DRAFTS, 512);
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
        edges: vec![edge.clone(), Edge::new(0, 90), Edge::new(2, 2)]
            .into_iter()
            .chain(std::iter::repeat_n(edge, 200))
            .collect(),
        groups,
        extent: [u16::MAX, 10, 10],
        ..Diagram::default()
    }
    .bounded();
    assert_eq!(
        (
            diagram.nodes.len(),
            diagram.edges.len(),
            diagram.groups.len()
        ),
        (NODES, EDGES, GROUPS)
    );
    assert_eq!((diagram.nodes[0].size, diagram.nodes[1].size), (1, SIZES));
    assert_eq!(diagram.nodes[0].group, None);
    assert_eq!(diagram.nodes[0].label.chars().count(), LABEL_CHARS + 1);
    assert_eq!(
        (diagram.groups[0].parent, diagram.groups[2].parent),
        (None, Some(1))
    );
    assert_eq!(diagram.edges[0].via.len(), WAYPOINTS);
    assert_eq!(diagram.edges[0].weight, WEIGHTS);
    assert_eq!(diagram.extent[0], EXTENT_MM);
    assert!((diagram.nodes[1].width() - 2.0 * NODE_MM).abs() < 1e-3);
}

#[test]
fn edges_are_named_by_their_ends_and_label() {
    let (diagram, _) = parse("a[Frame step] --> b[faces] : panels changed\nb <-- c\nc --- a\n");
    assert_eq!(
        diagram.edge_name(0).as_deref(),
        Some("Frame step → faces: panels changed")
    );
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
    let (diagram, warnings) = parse(
        "octree((Octree))@{ size: 2, }\nrenderer --> octre\nsubgraph s [Scene]\nrenderer <-- leaf\nend\n",
    );
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
    let (diagram, _) =
        Diagram::from_json(&serde_json::json!({"nodes": [{"id": "a", "label": ["A"], "size": 2}]}))
            .unwrap();
    assert_eq!(
        (diagram.nodes[0].label.as_str(), diagram.nodes[0].size),
        ("a", 2)
    );
    assert_eq!(
        serde_json::to_value(crate::Resource::volume_at(
            "panel-7",
            Some("octree"),
            Some(2)
        ))
        .unwrap(),
        serde_json::json!({"kind": "panel", "panel": "panel-7", "node": "octree", "edge": 2})
    );
}

#[test]
fn what_cannot_be_named_is_left_out_quietly() {
    assert_eq!(
        told(&[Warning {
            line: Some(2),
            text: "x".into()
        }]),
        "line 2: x"
    );
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
        let (diagram, warnings) =
            Diagram::from_json(&serde_json::json!({"layout": layout})).unwrap();
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

#[test]
fn world_edges_point_at_their_references_whichever_way_they_were_written() {
    let (diagram, warnings) = parse(concat!(
        "fire := object farmyard:campsite/bonfire [The bonfire]\n",
        "me := agent\n",
        "a[1. Find the fire] --> fire : here\n",
        "fire --> b[2. Warm up]\n",
        "fire <-- c[3. Leave]\n",
        "d[4. Watch] <--> fire\n",
        "e[5. Mind it] --- fire\n",
        "me ==> a\n",
    ));
    assert!(warnings.is_empty(), "{warnings:?}");
    let ways: Vec<(u16, u16, Arrow)> = diagram
        .world_edges
        .iter()
        .map(|edge| (edge.from, edge.to, edge.direction))
        .collect();
    assert_eq!(
        ways,
        [
            (0, 0, Arrow::Forward),
            (1, 0, Arrow::Back),
            (2, 0, Arrow::Forward),
            (3, 0, Arrow::Both),
            (4, 0, Arrow::None),
            (0, 1, Arrow::Back),
        ]
    );
    assert_eq!(diagram.world_edges[5].style, Style::Thick);
    // The outline lists each, with what it refers to, so that a client drawing none still says it.
    assert_eq!(
        diagram.outline(),
        "- 1. Find the fire\n- 2. Warm up\n- 3. Leave\n- 4. Watch\n- 5. Mind it\n\n**Edges**\n\
         - 1. Find the fire → The bonfire (farmyard:campsite/bonfire): here\n\
         - 2. Warm up ← The bonfire (farmyard:campsite/bonfire)\n\
         - 3. Leave → The bonfire (farmyard:campsite/bonfire)\n\
         - 4. Watch ↔ The bonfire (farmyard:campsite/bonfire)\n\
         - 5. Mind it — The bonfire (farmyard:campsite/bonfire)\n\
         - 1. Find the fire ← agent"
    );
    assert_eq!(diagram.world_edge_name(9, false), None);
    // Whatever a server sent, a world edge whose ends are not there is named nothing.
    let mut sent = diagram.clone();
    sent.world_edges = vec![Edge::new(40, 0), Edge::new(0, 40)];
    assert_eq!(
        (
            sent.world_edge_name(0, false),
            sent.world_edge_name(1, false)
        ),
        (None, None)
    );
    // From a node whose id is too long to be one, nothing.
    let (long, warnings) = parse(&format!(
        "fire := anchor home\n{} --> fire\n",
        "n".repeat(40)
    ));
    assert!(long.world_edges.is_empty());
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    // Unlabelled, a target is named by what it refers to, once.
    let (diagram, _) = parse("lookout := anchor pool-lookout\na --> lookout\n");
    assert_eq!(
        diagram.world_edge_name(0, true).as_deref(),
        Some("a → pool-lookout")
    );
}

#[test]
fn a_binding_is_kept_apart_from_the_nodes_and_warned_of_where_they_meet() {
    let (diagram, warnings) = parse(concat!(
        "a --> fire\n",
        "fire := object farmyard:campsite/bonfire\n",
        "stick := object farmyard:campsite/stick-2\n",
        "stick := object farmyard:campsite/stick-3\n",
        "stick := object farmyard:campsite/stick-2\n",
        "stick[A stick] --> b\n",
        "stick --> seat\n",
        "seat := anchor pool-lookout\n",
        "turn --> stik\n",
        "1nvalid! := anchor home\n",
    ));
    assert_eq!(
        texts(&warnings),
        [
            "line 2: \"fire\" is a node already, so it is not bound to object farmyard:campsite/bonfire: bind a name before the edges that use it",
            "line 4: \"stick\" is bound already, to object farmyard:campsite/stick-2; the first kept",
            "line 6: \"stick\" is a world reference, not a node: give its label on its := line",
            "line 8: \"seat\" is a node already, so it is not bound to anchor pool-lookout: bind a name before the edges that use it",
            "line 9: \"stik\" is not a node or a world reference; made a box called stik (did you mean stick?)",
            "line 10: cannot read \"! := anchor home\" as an edge",
        ]
    );
    // `stick --> seat` is a world edge from a node called seat, declared before seat could be bound.
    assert_eq!(diagram.world.len(), 1);
    assert!(diagram.nodes.iter().any(|node| node.id == "seat"));
    let (_, warnings) = parse(
        "seat := anchor pool-lookout
",
    );
    assert_eq!(
        texts(&warnings),
        [
            "line 1: \"seat\" is bound to anchor pool-lookout but no edge uses it, so nothing points at it"
        ]
    );
    let (_, warnings) = parse("x := anchor home\ny := anchor yard\nx --> y\n");
    assert_eq!(
        texts(&warnings[..1]),
        [
            "line 3: the edge from x to y is left out: an edge to a thing in the world comes from a node"
        ]
    );
    assert_eq!(texts(&warnings[1..]).len(), 2);
    // An invalid id, as the builder is given it.
    let mut builder = Builder::new();
    assert!(!builder.reference(
        "no spaces",
        Reference::new(Kind::Anchor, "home"),
        Binding::default()
    ));
}

#[test]
fn world_references_and_their_edges_are_bounded() {
    let mut body = String::new();
    for index in 0..REFERENCES + 2 {
        body.push_str(&format!("r{index} := anchor place-{index}\n"));
    }
    for index in 0..REFERENCES + 2 {
        body.push_str(&format!("step{index} --> r{index}\n"));
    }
    // Ten to the first, past the most to one reference, and three more to the second, past the most of all.
    for index in 0..10 {
        body.push_str(&format!("m{index} --> r0\n"));
    }
    for index in 0..3 {
        body.push_str(&format!("p{index} --> r1\n"));
    }
    let (diagram, warnings) = parse(&body);
    assert_eq!(diagram.world.len(), REFERENCES);
    assert_eq!(diagram.world_edges.len(), WORLD_EDGES);
    assert_eq!(
        diagram
            .world_edges
            .iter()
            .filter(|edge| edge.to == 0)
            .count(),
        REFERENCE_EDGES
    );
    assert_eq!(
        texts(&warnings),
        [
            format!(
                "world references {} to {} left out, with their edges: at most {REFERENCES}",
                REFERENCES + 1,
                REFERENCES + 2
            ),
            format!("2 world edges left out: at most {WORLD_EDGES}"),
            format!("3 world edges left out: at most {REFERENCE_EDGES} to one world reference"),
        ]
    );
    // Past what is read to choose from, they are only counted, as are those from nodes past the most kept.
    let mut body = String::from("r := anchor home\n");
    for index in 0..WORLD_DRAFTS + 3 {
        body.push_str(&format!("n{index} --> r\n"));
    }
    let (diagram, warnings) = parse(&body);
    assert_eq!(diagram.nodes.len(), NODES);
    assert_eq!(diagram.world_edges.len(), REFERENCE_EDGES);
    assert!(texts(&warnings).contains(&"3 world edges left out: at most 24".to_owned()));
}

#[test]
fn the_tool_s_world_list_binds_references_as_a_block_does() {
    let (diagram, warnings) = Diagram::from_json(&serde_json::json!({
        "world": [
            {"id": "fire", "kind": "object", "name": "farmyard:campsite/bonfire", "label": "The bonfire", "mark": "halo", "colour": "amber"},
            {"id": "me", "kind": "agent"},
            {"id": "lookout", "kind": "place", "name": "pool-lookout"},
            {"id": "nowhere", "kind": "anchor"},
            {"id": "odd", "kind": "anchor", "name": "home", "mark": "glow"},
        ],
        "nodes": [{"id": "step", "label": "1. Find the fire"}, {"id": "fire", "label": "Not a node"}],
        "edges": [
            {"from": "step", "to": "fire", "label": "here"},
            {"from": "me", "to": "step"},
            {"from": "missing", "to": "lookout"},
            {"from": "step", "to": "odd"},
        ],
    }))
    .unwrap();
    assert_eq!(
        texts(&warnings),
        [
            "world reference lookout: \"place\" is not a kind: an object, anchor, wall, panel, worker, person or agent; taken as whatever the name names",
            "world reference nowhere is left out: it names nothing",
            "world reference odd: \"glow\" is not a mark: a ring, halo, outline or none",
            "\"fire\" is a world reference, not a node: give its label on its := line",
            "\"missing\" is not a node; made a box called missing",
        ]
    );
    assert_eq!(diagram.world.len(), 4);
    assert_eq!(
        (
            diagram.world[0].label.as_str(),
            diagram.world[0].mark,
            diagram.world[0].colour
        ),
        ("The bonfire", Mark::Halo, Some(Colour::Amber))
    );
    assert_eq!(diagram.world[2].reference.kind, Kind::Unknown);
    assert_eq!(diagram.world_edges.len(), 4);
    assert!(
        Diagram::from_json(&serde_json::json!({"world": {"id": "x"}}))
            .unwrap_err()
            .contains("world must be a list")
    );
}

#[test]
fn world_edges_have_a_compact_wire_format_and_are_bounded_as_a_client_draws_them() {
    let (mut diagram, _) = parse("fire := object farmyard:campsite/bonfire\na --> fire : here\n");
    let json = serde_json::to_value(&diagram).unwrap();
    assert_eq!(
        json["world"],
        serde_json::json!([{"id": "fire", "reference": {"kind": "object", "name": "farmyard:campsite/bonfire"}, "state": "unknown", "radius": f64::from(crate::world::DEFAULT_RADIUS)}])
    );
    assert_eq!(
        json["world_edges"],
        serde_json::json!([{"from": 0, "to": 0, "label": "here", "direction": "forward", "style": "solid"}])
    );
    let back: Diagram = serde_json::from_value(json).unwrap();
    assert_eq!(back, diagram);
    // Whatever a server sent: at most so many targets, every world edge between a node and one of them, without
    // waypoints, its weight held and its label cut.
    let target = diagram.world[0].clone();
    diagram.world = vec![target; REFERENCES + 3];
    diagram.world[0].label = "l".repeat(80);
    let mut edge = diagram.world_edges[0].clone();
    edge.via = vec![[1, 2, 3]];
    edge.weight = 9;
    edge.label = "e".repeat(80);
    diagram.world_edges = vec![edge.clone(), Edge::new(5, 0), Edge::new(0, 40)]
        .into_iter()
        .chain(std::iter::repeat_n(edge, WORLD_EDGES + 5))
        .collect();
    let bounded = diagram.bounded();
    assert_eq!(bounded.world.len(), REFERENCES);
    assert_eq!(
        bounded.world[0].label.chars().count(),
        crate::world::LABEL_CHARS + 1
    );
    assert_eq!(bounded.world_edges.len(), WORLD_EDGES);
    assert!(bounded.world_edges.iter().all(|edge| edge.via.is_empty()
        && edge.weight == WEIGHTS
        && edge.label.chars().count() == EDGE_LABEL_CHARS + 1));
}

#[test]
fn a_reference_s_attributes_are_set_once_each() {
    let mut binding = Binding::default();
    binding.set("label", "First").unwrap();
    binding.set("title", "Second").unwrap();
    binding.set("mark", "ring").unwrap();
    binding.set("colour", "teal").unwrap();
    binding.set("color", "red").unwrap();
    assert_eq!(binding.label.as_deref(), Some("First"));
    assert_eq!(binding.colour, Some(Colour::Teal));
    assert_eq!(
        binding.set("colour", "mauve").unwrap_err(),
        "\"mauve\" is not a colour of the palette: teal, amber, blue, violet, green, red, yellow, grey or white"
    );
}
