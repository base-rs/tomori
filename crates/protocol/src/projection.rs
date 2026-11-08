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

/// How a wall's [`WALL_SLOTS`] slots are laid out about its pose. `Unknown` stands for a shape a newer server sends,
/// which a client draws as a [`WallShape::Grid`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WallShape {
    /// A flat board, three columns of two rows, its middle at the pose.
    #[default]
    Grid,
    /// One row, curved along an arc about a point `radius` metres ahead of the wall, along its pose's +Z, so that each
    /// slot faces that point: the pose is the middle of the row, between its third and fourth slots.
    Arc { radius: f32 },
    #[serde(other)]
    Unknown,
}

impl WallShape {
    fn is_grid(&self) -> bool {
        *self == Self::Grid
    }
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
    /// How its slots are laid out: a flat grid, unless it says otherwise, as an older server's walls do not.
    #[serde(default, skip_serializing_if = "WallShape::is_grid")]
    pub shape: WallShape,
}

impl Wall {
    /// The flat wall `id` of `kind` at `pose`.
    pub fn new(id: impl Into<String>, kind: WallKind, pose: Pose) -> Self {
        Self {
            id: id.into(),
            kind,
            pose,
            shape: WallShape::Grid,
        }
    }

    /// The wall, its slots laid out as `shape` says.
    pub fn with_shape(mut self, shape: WallShape) -> Self {
        self.shape = shape;
        self
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

/// The most lines of code a code panel holds, all of which it shows: the first lines of its block, numbered from 1.
/// There is no scrolling yet, so a longer block shows "…N more lines" after them.
pub const CODE_LINES: usize = 16;

/// The most characters of a code panel's lines together, not counting the breaks between them, after which they are cut
/// with "…" and the lines after left out: as many as a headset's code panel, 16 rows of 45 columns of code, can show of
/// them, however they wrap, so that where they are cut is never shown. A line is never cut short of that, however long,
/// and one cut is always too tall for what is left of the panel: it is counted in "…N more lines", or, the first line,
/// shows what fits and ends with "…".
pub const CODE_CHARS: usize = 720;

/// The most spans of a code panel; text past the last is drawn plain.
pub const CODE_SPANS: usize = 256;

/// What a span of code is, as a highlighter classifies it: a small set every highlighter maps onto, and every client
/// maps onto one palette, so that code reads alike whichever highlighter classified it. `Plain` stands for text no
/// span covers, and for a kind a newer server sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Variable,
    Operator,
    Punctuation,
    Constant,
    Attribute,
    #[default]
    #[serde(other)]
    Plain,
}

impl TokenKind {
    /// Every kind, `Plain` last.
    pub const ALL: [TokenKind; 12] = [
        TokenKind::Keyword,
        TokenKind::String,
        TokenKind::Comment,
        TokenKind::Number,
        TokenKind::Function,
        TokenKind::Type,
        TokenKind::Variable,
        TokenKind::Operator,
        TokenKind::Punctuation,
        TokenKind::Constant,
        TokenKind::Attribute,
        TokenKind::Plain,
    ];
}

/// A run of text of one [`TokenKind`]: from its `start`, in characters (Unicode scalar values) from the start of the
/// text, for `length` characters. Sent as `[start, length, kind]`, to keep a code panel small.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "(u32, u32, TokenKind)", into = "(u32, u32, TokenKind)")]
#[non_exhaustive]
pub struct Span {
    pub start: u32,
    pub length: u32,
    pub kind: TokenKind,
}

impl Span {
    pub fn new(start: u32, length: u32, kind: TokenKind) -> Self {
        Self {
            start,
            length,
            kind,
        }
    }

    /// Where it ends, in characters, one past its last.
    pub fn end(&self) -> u64 {
        u64::from(self.start) + u64::from(self.length)
    }
}

impl From<(u32, u32, TokenKind)> for Span {
    fn from((start, length, kind): (u32, u32, TokenKind)) -> Self {
        Self::new(start, length, kind)
    }
}

impl From<Span> for (u32, u32, TokenKind) {
    fn from(span: Span) -> Self {
        (span.start, span.length, span.kind)
    }
}

/// Whether `spans` are valid for `text`: at most [`CODE_SPANS`], each of at least one character and inside the text, in
/// order and none overlapping another. A client draws code whose spans are not valid as plain text.
pub fn valid_spans(text: &str, spans: &[Span]) -> bool {
    let length = text.chars().count() as u64;
    let mut end = 0;
    spans.len() <= CODE_SPANS
        && spans.iter().all(|span| {
            let valid = span.length > 0 && u64::from(span.start) >= end && span.end() <= length;
            end = span.end();
            valid
        })
}

/// What makes a panel a code panel, its body the first lines of a block of code: its language, as the block's info
/// string names it, how many lines the block has in all, and its spans, over the body, by kind, without colours.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Code {
    /// In lower case, such as `rust`; empty when the block names none.
    #[serde(default)]
    pub language: String,
    /// How many lines the block has, of which the panel holds the first, at most [`CODE_LINES`].
    pub lines: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<Span>,
}

impl Code {
    /// The code of `language`, a block of `lines` lines.
    pub fn new(language: impl Into<String>, lines: u32) -> Self {
        Self {
            language: language.into(),
            lines,
            spans: Vec::new(),
        }
    }

    /// The code with `spans`.
    pub fn with_spans(mut self, spans: Vec<Span>) -> Self {
        self.spans = spans;
        self
    }
}

/// A panel: a title, a body, and a little metadata, all bounded and cleaned by the server. A text panel's body is
/// Markdown-lite text, which is drawn as text and never as HTML; a code panel's, with [`Panel::code`], the first lines
/// of a block of code, drawn as they are, coloured by its spans.
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
    /// What makes it a code panel, if it is one. An older client draws one as a text panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    /// Whether a participant has put it where it is, so that it stays when the agent's next code panels replace its
    /// last ones.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
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
            code: None,
            pinned: false,
        }
    }

    /// The panel, a code panel of `code`.
    pub fn with_code(mut self, code: Code) -> Self {
        self.code = Some(code);
        self
    }

    /// The panel, pinned where it is by a participant.
    pub fn with_pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
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
        assert_eq!(newer.walls[0].shape, WallShape::Grid);
        // A curved wall says so, and a newer shape is read as unknown.
        let curved = Wall::new(TOMORI_WALL, WallKind::Tomori, pose)
            .with_shape(WallShape::Arc { radius: 2.5 });
        let json = serde_json::json!({"id": "tomori", "kind": "tomori",
            "pose": {"position": [1.0, 1.5, -2.0], "orientation": [0.0, 0.0, 0.0, 1.0]},
            "shape": {"kind": "arc", "radius": 2.5}});
        assert_eq!(serde_json::to_value(&curved).unwrap(), json);
        assert_eq!(serde_json::from_value::<Wall>(json).unwrap(), curved);
        let newest: Wall = serde_json::from_value(serde_json::json!({"id": "w", "kind": "added",
            "pose": {"position": [0.0, 0.0, 0.0], "orientation": [0.0, 0.0, 0.0, 1.0]},
            "shape": {"kind": "dome", "rings": 3}}))
        .unwrap();
        assert_eq!(newest.shape, WallShape::Unknown);
        assert_eq!(newer.panels[0].place, Place::Unknown);
        assert_eq!((newer.panels[0].age, newer.panels[0].meta.len()), (0, 0));
    }

    #[test]
    fn a_code_panel_sends_its_spans_by_kind_compactly_and_tolerates_newer_kinds() {
        let place = Place::Wall {
            wall: TOMORI_WALL.into(),
            slot: 1,
        };
        let panel = Panel::new("panel-2", "rust", "fn main() {}", place)
            .with_code(Code::new("rust", 40).with_spans(vec![
                Span::new(0, 2, TokenKind::Keyword),
                Span::new(3, 4, TokenKind::Function),
            ]))
            .with_pinned(true);
        let json = serde_json::json!({
            "id": "panel-2", "title": "rust", "body": "fn main() {}", "age": 0,
            "place": {"kind": "wall", "wall": "tomori", "slot": 1},
            "code": {"language": "rust", "lines": 40, "spans": [[0, 2, "keyword"], [3, 4, "function"]]},
            "pinned": true
        });
        assert_eq!(serde_json::to_value(&panel).unwrap(), json);
        assert_eq!(serde_json::from_value::<Panel>(json).unwrap(), panel);
        // A text panel says nothing of code or pinning; code without spans or a language, and a kind a newer server
        // sends, are read, the newer kind as plain.
        let text = Panel::new("panel-1", "t", "b", Place::Unknown);
        let json = serde_json::to_value(&text).unwrap();
        assert!(json.get("code").is_none() && json.get("pinned").is_none());
        let newer: Code = serde_json::from_value(serde_json::json!({
            "lines": 3, "spans": [[0, 1, "macro"]], "colour": "red"
        }))
        .unwrap();
        assert_eq!(newer.language, "");
        assert_eq!(newer.spans, [Span::new(0, 1, TokenKind::Plain)]);
        assert_eq!(TokenKind::default(), TokenKind::Plain);
        assert_eq!(TokenKind::ALL.len(), 12);
        for kind in TokenKind::ALL {
            let json = serde_json::to_value(kind).unwrap();
            assert_eq!(serde_json::from_value::<TokenKind>(json).unwrap(), kind);
        }
    }

    #[test]
    fn spans_are_valid_only_inside_the_text_in_order_without_overlapping_and_bounded() {
        let text = "let é = 1;";
        let span = |start, length| Span::new(start, length, TokenKind::Keyword);
        assert!(valid_spans(text, &[]));
        assert!(valid_spans(text, &[span(0, 3), span(4, 1), span(9, 1)]));
        // Counted in characters, not bytes: "é" is one.
        assert!(valid_spans(text, &[span(0, 10)]));
        assert!(!valid_spans(text, &[span(0, 11)]));
        assert!(!valid_spans(text, &[span(10, 1)]));
        // Empty, overlapping or out of order.
        assert!(!valid_spans(text, &[span(0, 0)]));
        assert!(!valid_spans(text, &[span(0, 3), span(2, 1)]));
        assert!(!valid_spans(text, &[span(4, 1), span(0, 1)]));
        // Overflowing past the largest start.
        assert!(!valid_spans(text, &[span(u32::MAX, u32::MAX)]));
        // At most so many.
        let long = "x".repeat(CODE_SPANS + 1);
        let many: Vec<Span> = (0..CODE_SPANS as u32).map(|start| span(start, 1)).collect();
        assert!(valid_spans(&long, &many));
        let more: Vec<Span> = (0..=CODE_SPANS as u32)
            .map(|start| span(start, 1))
            .collect();
        assert!(!valid_spans(&long, &more));
        assert_eq!(span(u32::MAX, u32::MAX).end(), 2 * u64::from(u32::MAX));
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
                    slot: None,
                    pose: None,
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "wall": "tomori"}),
            ),
            (
                ClientMessage::PlacePanel {
                    panel: "panel-1".into(),
                    wall: Some(TOMORI_WALL.into()),
                    slot: Some(4),
                    pose: None,
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "wall": "tomori", "slot": 4}),
            ),
            (
                ClientMessage::PlacePanel {
                    panel: "panel-1".into(),
                    wall: None,
                    slot: None,
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
