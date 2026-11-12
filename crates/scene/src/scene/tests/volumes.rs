// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A volume pointed at as words begin: the node or edge the ray meets is named with the volume, by its id or index,
//! for the server to describe from its own copy, and anywhere else on it the volume alone; its solids and labels are
//! in the frame. See `design/diagrams.md`.

use super::*;
use tomori_protocol::diagram::{Diagram, Edge, Layout, Node};
use tomori_protocol::{Panel, Place, Pose, Resource, ServerMessage};

#[test]
fn words_begun_pointing_at_a_volume_name_the_node_or_edge_the_ray_meets() {
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.projection = true));
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -1.2);
    let pose = Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array());
    let mut diagram = Diagram::default();
    diagram.layout = Layout::Force;
    for (id, x) in [("a", -200), ("b", 200)] {
        let mut node = Node::new(id, id.to_uppercase());
        node.at = [x, 0, 0];
        diagram.nodes.push(node);
    }
    diagram.edges.push(Edge::new(0, 1));
    diagram.extent = [600, 300, 200];
    let panel = Panel::new("panel-1", "Pair", "- A\n- B", Place::Free { pose })
        .with_diagram(diagram.clone());
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![panel.clone()]),
        ),
    }));
    conversation.take_actions();
    let model = crate::board::model(&pose).unwrap()
        * crate::projection::volume::placed(&panel, &diagram, crate::projection::Fit::Text);
    let mut step = Step::default();
    let now = Instant::now();
    let mut named = Vec::new();
    for point in [
        Vec3::new(-200.0, 0.0, 0.0),
        Vec3::ZERO,
        Vec3::new(0.0, -120.0, -100.0),
    ] {
        let at = model.transform_point3(point);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.volumes.len(), 1);
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        named.extend(utterance.attention.first().cloned());
    }
    assert_eq!(
        named,
        [
            Resource::volume_at("panel-1", Some("a"), None),
            Resource::volume_at("panel-1", None, Some(0)),
            Resource::panel("panel-1"),
        ]
    );
}
