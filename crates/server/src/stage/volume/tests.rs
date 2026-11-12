// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::stage::Walls;
use tomori_protocol::{Place, Pose, TOMORI_WALL};

const SKY: Walls<'static> = Walls {
    environment: "sky",
    tomori: Pose {
        position: [0.0, 0.52, -2.3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    },
    declared: &[],
    status: crate::stage::Status::Beneath,
};

fn block(body: &str) -> String {
    format!("```diagram\n{body}```\n")
}

#[test]
fn a_reply_projects_at_most_two_volumes_among_its_four_panels_and_none_of_nothing() {
    let one = block("title: One\n---\na --> b\n");
    let reply = format!("{one}{one}{one}```panel\nNotes.\n```\n```panel\nMore.\n```\n```panel\nMost.\n```\n");
    let found = crate::stage::panels(&reply);
    let diagrams = found.iter().filter(|found| found.content.diagram.is_some()).count();
    assert_eq!((diagrams, found.len()), (2, 4));
    // A block of no node, or too large, is not projected; one left open runs to the end of the reply.
    assert!(crate::stage::panels(&block("title: Empty\n---\n%% nothing\n")).is_empty());
    let large: String = (0..4000).map(|index| format!("n{index}\n")).collect();
    assert!(crate::stage::panels(&block(&large)).is_empty());
    let open = crate::stage::panels("```diagram\na --> b\n");
    assert_eq!(open.len(), 1);
    // Without a header, the whole block is its body.
    let (content, place) = Content::parse_diagram("a --> b\n").unwrap();
    assert_eq!((content.title.as_str(), place), ("", None));
}

#[test]
fn a_volume_is_laid_out_with_its_outline_and_how_many_warnings_it_has() {
    let (mut content, place) =
        Content::parse_diagram("title: T\nplace: beside\nlayout: force\na: 1\nb: 2\nc: 3\nd: 4\n---\nx --> y\nstyle x fill:#f00\n").unwrap();
    assert_eq!(place, Some(Placement::Beside(None)));
    assert_eq!(content.diagram.as_ref().unwrap().layout, Layout::Force);
    assert_eq!(content.warnings, ["line 10: style lines are not drawn: give a node a colour with @{ colour: teal }"]);
    content.lay_out(None);
    assert_eq!(content.body, "- x\n- y\n\n**Edges**\n- x → y");
    // Its metadata is full, so the count is not added to it.
    assert_eq!(content.meta.len(), 4);
    let (mut content, _) = Content::parse_diagram("layout: radial\n---\nx --> y\n").unwrap();
    content.lay_out(None);
    assert_eq!(content.meta, [("warnings".to_owned(), "1".to_owned())]);
    assert_eq!(
        content.warned("panel-3").as_deref(),
        Some("Tomori showed the diagram \"\" (panel-3) with 1 warnings: radial layout comes in a later version: laid out by force")
    );
    // Laid out again without them, the count goes.
    content.warnings.clear();
    content.diagram.as_mut().unwrap().layout = Layout::Force;
    content.lay_out(None);
    assert!(content.meta.is_empty());
    assert_eq!(content.warned("panel-3"), None);
    // A panel that is not a volume is not laid out.
    let mut text = Content::parse("Notes.").unwrap();
    text.lay_out(None);
    assert_eq!(text.body, "Notes.");
}

/// A stage showing a volume of `body`, floating, and the walls it stands in.
fn shown(body: &str) -> (Stage, String) {
    let mut stage = Stage::default();
    let walls = SKY;
    let (mut content, _) = Content::parse_diagram(body).unwrap();
    content.lay_out(None);
    let id = stage.project_to(
        &walls,
        content,
        &crate::stage::Destination::Free(Pose::at([0.0, 1.5, -1.0])),
        0,
    );
    (stage, id)
}

#[test]
fn the_agent_is_told_of_a_node_with_its_edges_and_of_an_edge_with_its_style() {
    let mut body = String::from("id: hub\n---\nhub{{Hub}}@{ detail: \"Spaces, under one lock\" }\nsubgraph g\nleaf[(Store)]\nend\nhub ==> leaf\n");
    for index in 0..10 {
        body.push_str(&format!("hub -.-> n{index}{{Choice}}\n"));
    }
    body.push_str("hub ~~> c[[Card]]\nhub --> s((Sphere))\nhub --> b[Box]\n");
    let (stage, id) = shown(&body);
    let walls = SKY;
    let describe = |node: Option<&str>, edge: Option<u16>| {
        stage
            .describe_at(&walls, &id, crate::stage::Pointed { node, edge, ..Default::default() }, 60)
            .unwrap()
    };
    let hub = describe(Some("hub"), None);
    assert!(hub.starts_with("node \"Hub\" (id hub) in diagram beginning \"- Hub: Spaces, under one lock\" (id hub, floating, projected a minute ago; prism; edges: to Store, to Choice"), "{hub}");
    assert!(hub.ends_with(" and 6 more; detail: \"Spaces, under one lock\")"), "{hub}");
    assert!(describe(Some("leaf"), None).ends_with("cylinder, in group \"g\"; edges: from Hub)"));
    for (node, shape) in [("n0", "diamond"), ("c", "card"), ("s", "sphere"), ("b", "box")] {
        assert!(describe(Some(node), None).contains(&format!("; {shape}; edges: from Hub")), "{node}");
    }
    for (edge, style) in [(0, "thick"), (1, "dashed"), (11, "a flow"), (12, "solid")] {
        assert!(describe(None, Some(edge)).ends_with(&format!("; {style})")), "{edge}");
    }
    assert!(describe(None, Some(1)).starts_with("edge \"Hub → Choice\" (edge 1) in diagram"));
    // What it does not have names the volume alone.
    assert!(describe(Some("nowhere"), Some(400)).starts_with("diagram beginning"));
    assert!(describe(None, None).starts_with("diagram beginning"));
    // A dotted edge, a node with a reference, and a group by its label.
    let (stage, id) = shown("title: T\n---\nsubgraph g [Group]\na@{ ref: crates/x }\nend\na --> b\n");
    let mut kept = stage.clone();
    let diagram = kept.here_mut("sky", &id).unwrap().content.diagram.as_mut().unwrap();
    diagram.edges[0].style = tomori_protocol::diagram::Style::Dotted;
    // A group without a label is named by its id, and an edge to a node not there, as a record changed by hand might
    // hold, is not told.
    diagram.groups[0].label.clear();
    diagram.edges.push(tomori_protocol::diagram::Edge::new(0, 40));

    let read = |stage: &Stage, node: Option<&str>, edge: Option<u16>| {
        stage
            .describe_at(&walls, &id, crate::stage::Pointed { node, edge, ..Default::default() }, 0)
            .unwrap()
    };
    assert!(read(&kept, None, Some(0)).ends_with("; dotted)"));
    assert!(read(&kept, Some("a"), None).contains("box, in group \"g\"; edges: to b; ref:"));
    assert!(read(&kept, Some("b"), None).ends_with("box; edges: from a)"));
    // A node of no edges says none.
    let (lone, lone_id) = shown("a\n");
    let told = lone
        .describe_at(&SKY, &lone_id, crate::stage::Pointed { node: Some("a"), ..Default::default() }, 0)
        .unwrap();
    assert!(told.ends_with("; box)"), "{told}");
    assert!(read(&stage, Some("a"), None).ends_with("box, in group \"Group\"; edges: to b; ref: \"crates/x\")"));
    assert_eq!(stage.named_diagram("sky", "nothing"), None);
}

#[test]
fn a_client_is_sent_as_many_diagrams_as_it_draws_and_the_rest_as_outlines() {
    let (stage, _) = shown("a --> b --> c\n");
    let walls = SKY;
    let panel = stage.view("space", &walls, 0).panels[0].clone();
    let text = Panel::new("panel-9", "Notes", "Text.", Place::Wall { wall: TOMORI_WALL.into(), slot: 0 });
    let panels = vec![panel.clone(), text.clone(), panel.clone(), panel];
    let sent = |diagrams: bool, limits: Limits| {
        within_volumes(panels.clone(), diagrams, &limits)
            .iter()
            .map(|panel| panel.diagram.is_some())
            .collect::<Vec<_>>()
    };
    let limits = Limits::TODAY.with_volumes(2, 1024);
    assert_eq!(sent(true, limits), [true, false, true, false]);
    assert_eq!(sent(true, Limits::TODAY.with_volumes(16, 5)), [true, false, false, false]);
    assert_eq!(sent(false, limits), [false; 4]);
}
