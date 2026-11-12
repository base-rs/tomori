// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::diagram::{Colour, Rank};
use crate::world::Mark;

fn texts(warnings: &[Warning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}

#[test]
fn shapes_come_from_their_brackets_as_mermaid_s_do() {
    let (diagram, warnings) = parse(
        "a[Box]\nb((Sphere))\nc[(Cylinder)]\nd{{Prism}}\ne{Diamond}\nf[[Card]]\ng(Rounded)\nh([Stadium])\ni(((Double)))\nj>Flag]\nk[\"Quoted, label\"]\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let shapes: Vec<Shape> = diagram.nodes.iter().map(|node| node.shape).collect();
    assert_eq!(
        shapes,
        [
            Shape::Box,
            Shape::Sphere,
            Shape::Cylinder,
            Shape::Prism,
            Shape::Diamond,
            Shape::Card,
            Shape::Box,
            Shape::Box,
            Shape::Sphere,
            Shape::Box,
            Shape::Box
        ]
    );
    assert_eq!(diagram.nodes[10].label, "Quoted, label");
    assert_eq!(diagram.nodes[3].label, "Prism");
}

#[test]
fn edges_take_every_style_arrow_and_label_form() {
    let (diagram, warnings) = parse(
        "a --> b\na --- c\na <--> d\na -.-> e\na ==> f\na ~~> g\na <-- h\na -->|one| b\na -- two --> c\na -. three .-> d\na --> e : four\na-->b\nmy-node-->b\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let edges: Vec<(Style, Arrow, &str)> = diagram
        .edges
        .iter()
        .map(|edge| (edge.style, edge.direction, edge.label.as_str()))
        .collect();
    assert_eq!(
        edges,
        [
            (Style::Solid, Arrow::Forward, ""),
            (Style::Solid, Arrow::None, ""),
            (Style::Solid, Arrow::Both, ""),
            (Style::Dashed, Arrow::Forward, ""),
            (Style::Thick, Arrow::Forward, ""),
            (Style::Flow, Arrow::Forward, ""),
            (Style::Solid, Arrow::Back, ""),
            (Style::Solid, Arrow::Forward, "one"),
            (Style::Solid, Arrow::Forward, "two"),
            (Style::Dashed, Arrow::Forward, "three"),
            (Style::Solid, Arrow::Forward, "four"),
            (Style::Solid, Arrow::Forward, ""),
            (Style::Solid, Arrow::Forward, ""),
        ]
    );
    // An id may hold `-` and `.`, but not before the link that follows it.
    assert!(diagram.node("my-node").is_some());
}

#[test]
fn chains_and_ampersands_make_an_edge_for_each_pair() {
    let (diagram, warnings) = parse("a & b --> c --> d & e\n");
    assert!(warnings.is_empty());
    let pairs: Vec<(u16, u16)> = diagram
        .edges
        .iter()
        .map(|edge| (edge.from, edge.to))
        .collect();
    assert_eq!(pairs, [(0, 2), (1, 2), (2, 3), (2, 4)]);
}

#[test]
fn attributes_classes_and_comments() {
    let (diagram, warnings) = parse(
        "%% the hub\nhub((Hub))@{ size: 4, colour: amber, detail: \"Spaces, under one lock\", ref: crates/server }:::big\nclassDef big colour: teal, size: 2, shape: prism\nclassDef small size: 1\nleaf:::small\nclass hub,leaf,ghost big\nclass leaf missing\nhub@{ label: Later, detail: ignored }\n",
    );
    let hub = &diagram.nodes[0];
    // The first of each attribute is kept: its own before its class's.
    assert_eq!(
        (hub.size, hub.colour, hub.shape),
        (4, Colour::Amber, Shape::Sphere)
    );
    assert_eq!(hub.detail, "Spaces, under one lock");
    assert_eq!(hub.reference, "crates/server");
    assert_eq!(hub.label, "Hub");
    let leaf = &diagram.nodes[1];
    assert_eq!(
        (leaf.size, leaf.colour, leaf.shape),
        (1, Colour::Teal, Shape::Prism)
    );
    assert_eq!(
        texts(&warnings),
        [
            "line 6: \"ghost\" is not a node",
            "line 7: \"missing\" is not a class"
        ]
    );
}

#[test]
fn subgraphs_group_their_nodes_nested_as_they_are_written() {
    let (diagram, warnings) = parse(
        "subgraph scene [tomori-scene]\n  kind: container\n  direction LR\n  step((Frame step))\n  subgraph inner[\"Inner\"]\n    faces --> shapes\n  end\nend\nsubgraph Loose\nx\nend\nsubgraph [Only a label]\nend\noutside --> step\n",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let ids: Vec<&str> = diagram
        .groups
        .iter()
        .map(|group| group.id.as_str())
        .collect();
    assert_eq!(ids, ["scene", "inner", "Loose", "group-4"]);
    assert_eq!(diagram.groups[0].kind, GroupKind::Container);
    assert_eq!(diagram.groups[0].label, "tomori-scene");
    assert_eq!(diagram.groups[1].parent, Some(0));
    assert_eq!(diagram.groups[3].label, "Only a label");
    let group = |id: &str| diagram.nodes[diagram.node(id).unwrap()].group;
    assert_eq!(
        [
            group("step"),
            group("faces"),
            group("shapes"),
            group("x"),
            group("outside")
        ],
        [Some(0), Some(1), Some(1), Some(2), None]
    );
}

#[test]
fn a_first_line_or_direction_says_the_layout() {
    let (diagram, _) = parse("flowchart LR\na --> b\n");
    assert_eq!(
        diagram.layout,
        Layout::Layered {
            direction: Rank::Right
        }
    );
    let (diagram, _) = parse("%% comment first\ngraph\ndirection BT\n");
    assert_eq!(
        diagram.layout,
        Layout::Layered {
            direction: Rank::Up
        }
    );
    // Not on a later line, where it is not read.
    let (diagram, warnings) = parse("a\nflowchart TD\n");
    assert_eq!(diagram.layout, Layout::default());
    assert_eq!(texts(&warnings), ["line 2: cannot read \"TD\" as an edge"]);
    let (_, warnings) = parse("direction sideways\n");
    assert_eq!(
        texts(&warnings),
        ["line 1: \"sideways\" is not a direction"]
    );
}

#[test]
fn a_line_that_cannot_be_read_is_skipped_and_named() {
    let (diagram, warnings) = parse(
        "a --> \nb[unclosed\n--> c\nd --x e\nend\nkind: layer\nsubgraph g\nkind: tower\nend\nclassDef\nclass solo\nstyle a fill:#f9f\nf@{ size 3, shape: blob }\ng@{ open\nh -- never\ni -->|open j\nk : label\nl -- label --> m\nn --> o & : x\nwhat? --> p\n",
    );
    assert_eq!(
        texts(&warnings),
        [
            "line 1: \"a -->\" ends in an edge to nothing",
            "line 2: [ is not closed by ]",
            "line 3: cannot read \"--> c\" as a node",
            "line 4: \"--\" is not an edge",
            "line 5: end without a subgraph to end",
            "line 6: kind: outside a subgraph",
            "line 8: \"tower\" is not a kind of group: a cluster, layer or container",
            "line 10: classDef needs a name and attributes",
            "line 11: class needs nodes and a class",
            "line 12: style lines are not drawn: give a node a colour with @{ colour: teal }",
            "line 13: \"size 3\" is not key: value",
            "line 13: \"blob\" is not a shape: a box, sphere, cylinder, prism, diamond or card",
            "line 14: @{ is not closed by }",
            "line 15: \"--\" is not closed",
            "line 16: |label is not closed by |",
            "line 17: a label after no edge",
            "line 19: cannot read \": x\" as a node",
            "line 20: cannot read \"? --> p\" as an edge",
        ]
    );
    // What was read of the lines that could be is kept: nodes, and the edge with a label.
    assert!(diagram.node("f").is_some());
    assert_eq!(diagram.edges.len(), 1);
    assert_eq!(diagram.edges[0].label, "label");
}

#[test]
fn a_subgraph_left_open_ends_at_the_end() {
    let (diagram, warnings) = parse("subgraph g\na\n");
    assert_eq!(texts(&warnings), ["1 subgraph left open, ended at the end"]);
    assert_eq!(diagram.nodes[0].group, Some(0));
}

#[test]
fn links_are_read_by_their_characters() {
    assert_eq!(link("-->"), Some((Style::Solid, Arrow::Forward)));
    assert_eq!(link("-.-"), Some((Style::Dashed, Arrow::None)));
    assert_eq!(link("<==>"), Some((Style::Thick, Arrow::Both)));
    assert_eq!(link("~~~"), Some((Style::Flow, Arrow::None)));
    assert_eq!(link(">"), None);
    assert_eq!(link("--"), None);
    assert_eq!(link("<--"), Some((Style::Solid, Arrow::Back)));
    assert_eq!(link(".-."), None);
    assert_eq!(link("-=-"), None);
}

#[test]
fn quoted_text_keeps_its_commas() {
    assert_eq!(
        split_unquoted("a: \"x, y\", b: z", ','),
        ["a: \"x, y\"", " b: z"]
    );
    assert_eq!(unquoted("\"quoted\""), "quoted");
    assert_eq!(unquoted("\"half"), "\"half");
    assert_eq!(
        subgraph("[Label]"),
        ("Label".to_owned(), "Label".to_owned())
    );
}

#[test]
fn the_first_label_of_an_edge_is_kept() {
    let (diagram, warnings) =
        parse("flowchart sideways\na -->|first| b : second\nc -- third -->|fourth| d\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(diagram.layout, Layout::default());
    assert_eq!(diagram.edges[0].label, "first");
    assert_eq!(diagram.edges[1].label, "third");
}

#[test]
fn a_binding_names_a_thing_in_the_world_by_its_kind_and_name() {
    let (diagram, warnings) = parse(concat!(
        "fire := object farmyard:campsite/bonfire [The bonfire]\n",
        "seat := Anchor pool-lookout[The lookout's seat] @{ mark: halo, colour: teal, }\n",
        "board := wall \"Notice [board]\" @{ mark: outline }\n",
        "notes := panel trial-checklist\n",
        "builder := worker \"Build the client\"\n",
        "samuel := person Samuel#11\n",
        "me := agent\n",
        "hen := farmyard:hen/biscuit\n",
        "lookout := pool-lookout\n",
        "a[A] --> fire & seat & board & notes & builder & samuel & me & hen & lookout\n",
    ));
    assert!(warnings.is_empty(), "{warnings:?}");
    let references: Vec<String> = diagram
        .world
        .iter()
        .map(|target| format!("{} {}", target.id, target.reference))
        .collect();
    assert_eq!(
        references,
        [
            "fire object farmyard:campsite/bonfire",
            "seat anchor pool-lookout",
            "board wall Notice [board]",
            "notes panel trial-checklist",
            "builder worker Build the client",
            "samuel person Samuel#11",
            "me agent",
            // Without a kind, a name with a colon is an object's, and any other the server finds.
            "hen object farmyard:hen/biscuit",
            "lookout thing pool-lookout",
        ]
    );
    assert_eq!(diagram.world[0].label, "The bonfire");
    assert_eq!(
        (diagram.world[1].mark, diagram.world[1].colour),
        (Mark::Halo, Some(crate::chart::Colour::Teal))
    );
    assert_eq!(diagram.world[2].mark, Mark::Outline);
    // Only `a` is a node; the rest are world edges, one to each.
    assert_eq!(diagram.nodes.len(), 1);
    assert!(diagram.edges.is_empty());
    assert_eq!(diagram.world_edges.len(), 9);
    assert!(diagram.world_edges.iter().all(|edge| edge.from == 0));
}

#[test]
fn a_binding_that_cannot_be_read_is_skipped_and_named() {
    let (diagram, warnings) = parse(concat!(
        "nothing := object\n",
        "open := wall \"Notice board\n",
        "label := anchor home [Home\n",
        "attributes := anchor home @{ mark: glow, size: 2, odd }\n",
        "unclosed := anchor home @{ mark: ring\n",
        "trailing := anchor home [Home] extra\n",
        "step --> attributes & trailing\n",
    ));
    assert_eq!(
        texts(&warnings),
        [
            "line 1: nothing := names nothing: write a kind and a name, as nothing := object farmyard:campsite/bonfire",
            "line 2: the name of open is not closed by \"",
            "line 3: [ is not closed by ]",
            "line 4: \"glow\" is not a mark: a ring, halo, outline or none",
            "line 4: \"size\" is not an attribute of a world reference: mark or colour",
            "line 4: \"odd\" is not key: value",
            "line 5: @{ is not closed by }",
            "line 6: \"extra\" after trailing's reference is not read",
        ]
    );
    assert_eq!(diagram.world.len(), 2);
    assert_eq!(diagram.world[1].label, "Home");
    // A line whose left side is not an id is read as nodes and edges, as Mermaid's.
    let (diagram, _) = parse("a --> b : x := y\n");
    assert_eq!(diagram.edges[0].label, "x := y");
    assert!(diagram.world.is_empty());
}
