// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Projection: text panels the agent projects onto walls in a space, which participants pull off, put back and dismiss.
//! The first slice of agent projection (issue #82). It is the space's and the agent stage's, not an environment's, so it
//! has its own message rather than the environment state channel: [`crate::ServerMessage::Projection`], sent only to a
//! client whose `Hello` sets `projection`, and only by a server whose [`crate::SpaceState::projection`] is set, which
//! takes [`crate::ClientMessage::AddWall`], [`crate::ClientMessage::RemoveWall`],
//! [`crate::ClientMessage::PlacePanel`] and [`crate::ClientMessage::DismissPanel`]. See `design/projection.md`.
use serde::{Deserialize, Serialize};

use crate::Pose;

/// How many panels one wall holds, in slots numbered from 0, the most prominent first: three columns of two rows, read
/// left to right and then down, as seen from in front of the wall.
pub const WALL_SLOTS: u32 = 6;

/// The identity of the Tomori wall, the one wall every space has, which belongs to the space rather than to its
/// environment.
pub const TOMORI_WALL: &str = "tomori";

/// What the participants of a space see projected in it, sent whenever it changes, complete each time: a client
/// replaces what it had. It is the projection as seen in the space's current environment: the Tomori wall, the
/// environment's walls, and the panels on them or floating free. The walls and floating panels of other environments
/// the space has been in are kept by the server, hidden, and sent again when the space returns to them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Projection {
    /// The space it belongs to.
    pub space: String,
    /// The environment its walls, other than the Tomori wall, and its floating panels belong to.
    pub environment: String,
    /// The Tomori wall first, then the environment's own walls, then those its owners added.
    pub walls: Vec<Wall>,
    pub panels: Vec<Panel>,
}

impl Projection {
    /// The projection of `space` in `environment`, with nothing on it.
    pub fn new(space: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            space: space.into(),
            environment: environment.into(),
            walls: Vec::new(),
            panels: Vec::new(),
        }
    }

    /// The projection with `walls`.
    pub fn with_walls(mut self, walls: Vec<Wall>) -> Self {
        self.walls = walls;
        self
    }

    /// The projection with `panels`.
    pub fn with_panels(mut self, panels: Vec<Panel>) -> Self {
        self.panels = panels;
        self
    }

    /// The wall `id`, if it is shown.
    pub fn wall(&self, id: &str) -> Option<&Wall> {
        self.walls.iter().find(|wall| wall.id == id)
    }

    /// The panel `id`, if it is shown.
    pub fn panel(&self, id: &str) -> Option<&Panel> {
        self.panels.iter().find(|panel| panel.id == id)
    }
}

/// Whose a wall is. `Unknown` stands for a kind a newer server sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WallKind {
    /// The space's own wall, beside Tomori, which every space has and keeps across environments.
    Tomori,
    /// One the environment declares.
    Environment,
    /// One an owner added, which an owner may remove.
    Added,
    #[default]
    #[serde(other)]
    Unknown,
}

/// A wall panels are projected onto, upright, facing along its pose's +Z, in the space's coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Wall {
    /// Its identity, which [`crate::ClientMessage::PlacePanel`] and [`crate::ClientMessage::RemoveWall`] name:
    /// [`TOMORI_WALL`] for the Tomori wall.
    pub id: String,
    pub kind: WallKind,
    /// Where its middle is, and which way it faces.
    pub pose: Pose,
}

impl Wall {
    /// The wall `id` of `kind` at `pose`.
    pub fn new(id: impl Into<String>, kind: WallKind, pose: Pose) -> Self {
        Self {
            id: id.into(),
            kind,
            pose,
        }
    }
}

/// Where a panel is. `Unknown` stands for a place a newer server sends, which a client does not draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Place {
    /// In `slot`, from 0 to [`WALL_SLOTS`], of the wall `wall`.
    Wall { wall: String, slot: u32 },
    /// Floating free where a participant left it, upright, facing along its pose's +Z.
    Free { pose: Pose },
    #[serde(other)]
    Unknown,
}

/// One key and value of a panel's metadata, such as `source` and `CI`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Meta {
    pub key: String,
    pub value: String,
}

impl Meta {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// A text panel: a title, a body of Markdown-lite text, which is drawn as text and never as HTML, and a little
/// metadata, all bounded and cleaned by the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Panel {
    /// Its identity, unique in the space, which [`crate::Resource::Panel`] names.
    pub id: String,
    /// On one line; it may be empty.
    pub title: String,
    /// Its text, of several lines.
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meta: Vec<Meta>,
    /// How long ago it was projected, in seconds, when the server sent it.
    #[serde(default)]
    pub age: u64,
    pub place: Place,
}

impl Panel {
    /// The panel `id` at `place`, titled `title`, showing `body`.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        place: Place,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            meta: Vec::new(),
            age: 0,
            place,
        }
    }

    /// The panel with `meta`.
    pub fn with_meta(mut self, meta: Vec<Meta>) -> Self {
        self.meta = meta;
        self
    }

    /// The panel, projected `age` seconds ago.
    pub fn with_age(mut self, age: u64) -> Self {
        self.age = age;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projection_has_a_stable_wire_format_and_tolerates_newer_values() {
        let pose = Pose {
            position: [1.0, 1.5, -2.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let projection = Projection::new("s", "sky")
            .with_walls(vec![Wall::new(TOMORI_WALL, WallKind::Tomori, pose)])
            .with_panels(vec![
                Panel::new(
                    "panel-1",
                    "Build status",
                    "**3 crates** failed",
                    Place::Wall {
                        wall: TOMORI_WALL.into(),
                        slot: 0,
                    },
                )
                .with_meta(vec![Meta::new("source", "CI")])
                .with_age(60),
                Panel::new("panel-2", "", "Free", Place::Free { pose }),
            ]);
        let json = serde_json::json!({
            "space": "s", "environment": "sky",
            "walls": [{"id": "tomori", "kind": "tomori",
                "pose": {"position": [1.0, 1.5, -2.0], "orientation": [0.0, 0.0, 0.0, 1.0]}}],
            "panels": [
                {"id": "panel-1", "title": "Build status", "body": "**3 crates** failed",
                    "meta": [{"key": "source", "value": "CI"}], "age": 60,
                    "place": {"kind": "wall", "wall": "tomori", "slot": 0}},
                {"id": "panel-2", "title": "", "body": "Free", "age": 0,
                    "place": {"kind": "free",
                        "pose": {"position": [1.0, 1.5, -2.0], "orientation": [0.0, 0.0, 0.0, 1.0]}}}
            ]
        });
        assert_eq!(serde_json::to_value(&projection).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<Projection>(json).unwrap(),
            projection
        );
        assert_eq!(projection.wall(TOMORI_WALL).unwrap().kind, WallKind::Tomori);
        assert!(projection.wall("wall-9").is_none());
        assert_eq!(projection.panel("panel-2").unwrap().body, "Free");
        assert!(projection.panel("panel-9").is_none());
        // A newer kind of wall, a newer place and a panel without metadata or age are read.
        let newer: Projection = serde_json::from_value(serde_json::json!({
            "space": "s", "environment": "sky",
            "walls": [{"id": "w", "kind": "ceiling",
                "pose": {"position": [0.0, 0.0, 0.0], "orientation": [0.0, 0.0, 0.0, 1.0]}}],
            "panels": [{"id": "p", "title": "t", "body": "b", "place": {"kind": "orbit", "radius": 2}}]
        }))
        .unwrap();
        assert_eq!(newer.walls[0].kind, WallKind::Unknown);
        assert_eq!(newer.panels[0].place, Place::Unknown);
        assert_eq!((newer.panels[0].age, newer.panels[0].meta.len()), (0, 0));
    }

    #[test]
    fn an_upright_pose_turns_only_about_the_vertical_to_face_a_point() {
        // Facing a point straight above, or itself, it is not turned.
        assert_eq!(
            Pose::upright_facing([1.0, 1.0, 1.0], [1.0, 3.0, 1.0]).orientation,
            [0.0, 0.0, 0.0, 1.0]
        );
        // Facing a point to its right, along +X, it is turned a quarter about +Y, whatever the height.
        let [x, y, z, w] = Pose::upright_facing([0.0, 1.0, 0.0], [2.0, 0.0, 0.0]).orientation;
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!(x == 0.0 && z == 0.0 && (y - half).abs() < 1e-6 && (w - half).abs() < 1e-6);
    }

    #[test]
    fn projection_messages_have_a_stable_wire_format() {
        use crate::{ClientMessage, Resource, ServerMessage};
        let pose = Pose {
            position: [0.0, 1.5, -1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let pose_json =
            serde_json::json!({"position": [0.0, 1.5, -1.0], "orientation": [0.0, 0.0, 0.0, 1.0]});
        let cases = [
            (
                ClientMessage::AddWall { pose },
                serde_json::json!({"type": "add_wall", "pose": pose_json}),
            ),
            (
                ClientMessage::RemoveWall {
                    wall: "wall-2".into(),
                },
                serde_json::json!({"type": "remove_wall", "wall": "wall-2"}),
            ),
            (
                ClientMessage::PlacePanel {
                    panel: "panel-1".into(),
                    wall: Some(TOMORI_WALL.into()),
                    pose: None,
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "wall": "tomori"}),
            ),
            (
                ClientMessage::PlacePanel {
                    panel: "panel-1".into(),
                    wall: None,
                    pose: Some(pose),
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "pose": pose_json}),
            ),
            (
                ClientMessage::DismissPanel {
                    panel: "panel-1".into(),
                },
                serde_json::json!({"type": "dismiss_panel", "panel": "panel-1"}),
            ),
        ];
        for (message, json) in cases {
            assert_eq!(serde_json::to_value(&message).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<ClientMessage>(json).unwrap(),
                message
            );
        }
        let projection = ServerMessage::Projection {
            projection: Box::new(Projection::new("s", "sky")),
        };
        let json = serde_json::to_value(&projection).unwrap();
        assert_eq!(json["type"], "projection");
        assert_eq!(
            serde_json::from_value::<ServerMessage>(json).unwrap(),
            projection
        );
        // A panel attended to is named by its identity alone.
        let panel = Resource::panel("panel-1");
        let json = serde_json::json!({"kind": "panel", "panel": "panel-1"});
        assert_eq!(serde_json::to_value(&panel).unwrap(), json);
        assert_eq!(serde_json::from_value::<Resource>(json).unwrap(), panel);
        // An older client's Hello reads no projection, and a newer one's says it reads it.
        let hello: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello", "protocol": 2, "participant": "p", "name": "Ada"
        }))
        .unwrap();
        let ClientMessage::Hello { projection, .. } = hello else {
            unreachable!("a hello");
        };
        assert!(!projection);
        let hello: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello", "protocol": 2, "participant": "p", "name": "Ada", "projection": true
        }))
        .unwrap();
        assert!(matches!(
            hello,
            ClientMessage::Hello {
                projection: true,
                ..
            }
        ));
        // An older server's space state does not project.
        let state: crate::SpaceState = serde_json::from_value(serde_json::json!({
            "id": "s", "title": "t", "backend": "b", "directory": "/", "activity": "idle", "messages": [],
            "queue": [], "approval": null, "question": null, "workers": [], "presentations": [],
            "participants": [], "attribution": true, "error": null
        }))
        .unwrap();
        assert!(!state.projection);
    }
}
