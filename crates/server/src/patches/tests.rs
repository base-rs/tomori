// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_protocol::{Panel, Place, TOMORI_WALL, Wall, WallKind};

use super::*;

fn projection(environment: &str, panels: usize, revision: u64) -> Projection {
    let pose = tomori_protocol::Pose {
        position: [0.0, 1.5, -2.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    Projection::new("s", environment)
        .with_walls(vec![Wall::new(TOMORI_WALL, WallKind::Tomori, pose)])
        .with_panels(
            (0..panels)
                .map(|index| {
                    Panel::new(
                        format!("panel-{index}"),
                        "Title",
                        "Body",
                        Place::Wall {
                            wall: TOMORI_WALL.into(),
                            slot: index as u32,
                        },
                    )
                })
                .collect(),
        )
        .with_revision(revision)
}

#[test]
fn a_client_is_sent_a_patch_only_from_what_it_holds_of_the_same_space_and_environment() {
    let now = projection("sky", 4, 3);
    // Holding nothing, as on joining, after asking again, or taking no patches: whole.
    assert!(
        matches!(message(None, &now), ServerMessage::Projection { projection } if *projection == now)
    );
    // Holding the same space's in another environment, or another space's: whole.
    let elsewhere = projection("grid", 4, 2);
    assert!(matches!(
        message(Some(&elsewhere), &now),
        ServerMessage::Projection { .. }
    ));
    let mut other = projection("sky", 4, 2);
    other.space = "t".into();
    assert!(matches!(
        message(Some(&other), &now),
        ServerMessage::Projection { .. }
    ));
    // Holding an earlier revision of it: a patch from there.
    let mut earlier = projection("sky", 4, 2);
    earlier.panels[0].body = "Before".into();
    let ServerMessage::ProjectionPatch { patch } = message(Some(&earlier), &now) else {
        unreachable!("a patch");
    };
    assert_eq!((patch.from, patch.to, patch.ops.len()), (2, 3, 1));
}

#[test]
fn a_patch_sending_more_than_half_of_what_is_shown_whole_is_sent_whole_instead() {
    let now = projection("sky", 5, 3);
    // Three of six changed: a patch.
    let mut half = now.clone().with_revision(2);
    for panel in &mut half.panels[..3] {
        panel.body = "Before".into();
    }
    assert!(matches!(
        message(Some(&half), &now),
        ServerMessage::ProjectionPatch { .. }
    ));
    // Four of six: whole.
    let mut most = now.clone().with_revision(2);
    for panel in &mut most.panels[..4] {
        panel.body = "Before".into();
    }
    assert!(matches!(
        message(Some(&most), &now),
        ServerMessage::Projection { .. }
    ));
}
