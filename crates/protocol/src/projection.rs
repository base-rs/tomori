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

pub mod group;
pub mod layout;
mod patch;

pub use group::{Align, Attach, Edge, Join};
pub use layout::{Grid, SlotFrame};
pub use patch::{PatchError, PatchOp, ProjectionPatch};

/// How many panels a wall holds unless it says otherwise, in slots numbered from 0, the most prominent first: a flat
/// wall's three columns of two rows, read left to right and then down, as seen from in front of the wall, or a curved
/// wall's six in a row. A wall's [`Wall::grid`] says how many it has.
pub const WALL_SLOTS: u32 = 6;

/// The identity of the Tomori wall, the one wall every space has, which belongs to the space rather than to its
/// environment.
pub const TOMORI_WALL: &str = "tomori";

/// What the participants of a space see projected in it, complete: a client replaces what it had. It is the
/// projection as seen in the space's current environment: the Tomori wall, the environment's walls, and the panels on
/// them or floating free. The walls and floating panels of other environments the space has been in are kept by the
/// server, hidden, and sent again when the space returns to them.
///
/// It is sent on joining, on a client's [`crate::ClientMessage::ResyncProjection`], and to a client that does not apply
/// patches, whenever it changes; a client that does is sent a [`ProjectionPatch`] from its [`Projection::revision`]
/// instead. See `design/projection.md`.
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
    /// The space's projection revision this is: a count the server raises with every change to what the space
    /// projects, so that a [`ProjectionPatch`] says the revision it applies to. 0, and absent, from older servers, which
    /// send no patches.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub revision: u64,
}

impl Projection {
    /// The projection of `space` in `environment`, with nothing on it.
    pub fn new(space: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            space: space.into(),
            environment: environment.into(),
            walls: Vec::new(),
            panels: Vec::new(),
            revision: 0,
        }
    }

    /// The projection at `revision` of its space's.
    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
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

/// How a wall's slots are laid out about its pose: flat or curved. `Unknown` stands for a shape a newer server sends,
/// which a client draws as a [`WallShape::Grid`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WallShape {
    /// A flat board, its middle at the pose: three columns of two rows unless its [`Wall::grid`] says otherwise.
    #[default]
    Grid,
    /// Curved along an arc about a vertical line `radius` metres ahead of the wall, along its pose's +Z, so that each
    /// slot faces that line: the pose is the middle of its slots, six in one row unless its [`Wall::grid`] says
    /// otherwise, its rows one above another.
    Arc { radius: f32 },
    #[serde(other)]
    Unknown,
}

impl WallShape {
    fn is_grid(&self) -> bool {
        *self == Self::Grid
    }
}

/// A wall panels are projected onto, facing along its pose's +Z, in the space's coordinates: upright, or tilted by its
/// [`Wall::tilt`] about its horizontal middle line. Where its slots stand is [`Wall::slot_frames`], in its
/// [`Wall::frame`].
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
    /// How many columns and rows of slots it has, as `[columns, rows]`: its shape's, unless it says otherwise, as an
    /// older server's walls do not. An older client draws its shape's, and none past them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Grid>,
    /// How large its slots are, as a proportion of a flat wall's, [`layout::SLOT_SIZE`]: its shape's, unless it says
    /// otherwise, 1 when flat and [`layout::ARC_SCALE`] when curved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// The shape of its slots, as their width over their height: a standard slot's, [`layout::STANDARD_ASPECT`], unless
    /// it says otherwise, within [`layout::ASPECTS`]. Its `scale` is the size of the text in its slots, and the slots
    /// are as large as that and taller or wider, so that a panel in one shows more rows or more columns. An older client
    /// draws standard slots at that scale, and so each panel at its text's size, smaller than its slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<f32>,
    /// How large its panels' content is drawn within its slots, as a proportion of the text its `scale` gives them:
    /// 1, unless it says otherwise, within [`layout::CONTENT_SCALES`] and never below [`Wall::least_content_scale`].
    /// Smaller, each slot shows more and longer lines, as a big wall that holds more; larger, fewer, to read from
    /// further. Its slots, their size and their number are the same whatever it is. An older client draws each panel at
    /// 1, as it was. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_scale: Option<f32>,
    /// The group it is snapped into, if it is in one, named by the identity of the group's first wall: carrying any of
    /// its walls carries them all. An older client draws each wall alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Its edges joined to another wall of its group, which stands across each, its slots a gutter from this wall's, as
    /// the server finds them: a client leaves the border and outer margin off where they meet, so that the group reads
    /// as one board. See [`group`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joined: Vec<Join>,
    /// Whether it is locked: a locked wall is neither moved, resized, reshaped, tilted, bent nor removed, by anyone,
    /// until an owner unlocks it with [`crate::ClientMessage::LockWall`], though panels still go onto it and come off
    /// it, and it snaps to nothing, nor anything to it. An environment's own walls are locked unless a space's owner
    /// unlocked them, and others unlocked unless one locked them. Sent only by a server whose
    /// [`crate::SpaceState::wall_locks`] is set; an older client, which knows none, offers its controls, and the server
    /// refuses what they ask. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// How far it tilts from upright about its horizontal middle line, in degrees, within [`layout::TILTS`]: positive
    /// with its top towards the front, so that it faces down, as a wall above the people in front of it does, and
    /// negative with its top away, facing up. Upright unless it says otherwise; an older client draws it upright. Its
    /// slots stand in [`Wall::frame`], its pose tilted so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<f32>,
    /// What an environment calls its own wall, such as `Details`: shown in capitals as the wall's label in place of
    /// `WALL`, and the name by which the agent names it, as `place: wall Details slot 2`, in place of `environment wall
    /// 1`. Only an environment's own wall has one, and only if it declares one; an older client draws `WALL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether it is the space's status wall, where Tomori's notices and what it projects unasked go: the Tomori wall,
    /// or an environment's own wall that its environment gives the role, such as the farmyard's notice board. Sent only
    /// where it is; an older client ignores it. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub status: bool,
    /// Its version: the space's projection revision, [`Projection::revision`], at which it last changed in anything it
    /// says, so that a change a client asks for, as by a wall's handles, says the version it saw, and the server refuses
    /// it where the wall changed since. Sent as `revision`, as a panel's is; 0, and absent, from older servers. See
    /// `design/projection.md`.
    #[serde(rename = "revision", default, skip_serializing_if = "is_zero")]
    pub version: u64,
    /// Its context: a chip for each setting of its own, which the panels its environment published on it inherit where
    /// they do not fix it themselves, at most [`crate::context::CONTEXT_CHIPS`]; none where it only follows. An older
    /// client ignores it. See `design/panel-context.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<crate::context::ContextChip>,
}

impl Wall {
    /// The flat wall `id` of `kind` at `pose`, unlocked.
    pub fn new(id: impl Into<String>, kind: WallKind, pose: Pose) -> Self {
        Self {
            id: id.into(),
            kind,
            pose,
            shape: WallShape::Grid,
            grid: None,
            scale: None,
            aspect: None,
            content_scale: None,
            group: None,
            joined: Vec::new(),
            locked: false,
            tilt: None,
            name: None,
            status: false,
            version: 0,
            context: Vec::new(),
        }
    }

    /// The wall, with the chips of its context.
    pub fn with_context(mut self, context: Vec<crate::context::ContextChip>) -> Self {
        self.context = context;
        self
    }

    /// The wall, called `name` by its environment, or nothing with `None`.
    pub fn with_name(mut self, name: Option<String>) -> Self {
        self.name = name;
        self
    }

    /// The wall, the space's status wall or not.
    pub fn with_status(mut self, status: bool) -> Self {
        self.status = status;
        self
    }

    /// The wall at `version`, the projection revision at which it last changed.
    pub fn with_version(mut self, version: u64) -> Self {
        self.version = version;
        self
    }

    /// Whether it and `other` say the same, but for their versions.
    pub fn same_but_version(&self, other: &Wall) -> bool {
        Wall {
            version: other.version,
            ..self.clone()
        } == *other
    }

    /// The wall, locked or not.
    pub fn with_locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// Whether it is locked, so that nothing changes it, moves it or snaps to it, nor it to anything, until an owner
    /// unlocks it.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// The wall, in the group `group`, joined to others across `joined`.
    pub fn with_group(mut self, group: Option<String>, joined: Vec<Join>) -> Self {
        self.group = group;
        self.joined = joined;
        self
    }

    /// The wall, its slots laid out as `shape` says.
    pub fn with_shape(mut self, shape: WallShape) -> Self {
        self.shape = shape;
        self
    }

    /// The wall, with `grid` of slots.
    pub fn with_grid(mut self, grid: Grid) -> Self {
        self.grid = Some(grid);
        self
    }

    /// The wall, its slots `scale` times a flat wall's.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = Some(scale);
        self
    }

    /// The wall, its slots `aspect` times as wide as they are tall, or a standard slot's shape with `None`.
    pub fn with_aspect(mut self, aspect: Option<f32>) -> Self {
        self.aspect = aspect;
        self
    }

    /// The wall, tilted `tilt` degrees from upright about its horizontal middle line, its top towards the front for a
    /// positive tilt, or upright with `None`.
    pub fn with_tilt(mut self, tilt: Option<f32>) -> Self {
        self.tilt = tilt;
        self
    }

    /// The wall, its panels' content drawn `content_scale` times the size its slots give it, or as large with `None`.
    pub fn with_content_scale(mut self, content_scale: Option<f32>) -> Self {
        self.content_scale = content_scale;
        self
    }
}

/// Where a panel is. `Unknown` stands for a place a newer server sends, which a client does not draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Place {
    /// In `slot`, from 0 to the last of its [`Wall::grid`], of the wall `wall`.
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

/// The most lines of code a code panel holds: the first lines of its block, numbered from 1, which a participant
/// scrolls through. A longer block says how many more lines it has, which are only in the transcript.
pub const CODE_LINES: usize = 200;

/// The most characters of a code panel's lines together, not counting the breaks between them, after which they are cut
/// with "…" and the lines after left out. A line is never cut short of that, however long. The server may send fewer,
/// down to the first 16 lines and 720 characters, where a projection would otherwise not fit in one frame.
pub const CODE_CHARS: usize = 8_000;

/// The most spans of a code panel; text past the last is drawn plain. An older client, which took at most 256, draws a
/// code panel with more as plain text.
pub const CODE_SPANS: usize = 2_048;

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
    /// How many lines the block has, of which the panel holds the first, at most [`CODE_LINES`]: a client says how many
    /// it does not hold.
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

/// The longest side of an image panel's image, in pixels: the server scales a larger image down to it.
pub const IMAGE_SIDE: u32 = 2048;

/// The most bytes of an image panel's image, as it is sent: the server scales an image down further until its PNG fits.
pub const IMAGE_BYTES: u32 = 8 * 1024 * 1024;

/// The most bytes of an image in one [`crate::FrameKind::Image`] frame, so that an image never holds up the frames
/// behind it for long.
pub const IMAGE_CHUNK_BYTES: usize = 32 * 1024;

/// The bytes of an [`ImageChunk`]'s header: the image's SHA-256, where the chunk starts and how long the image is.
const CHUNK_HEADER: usize = 32 + 4 + 4;

/// What makes a panel an image panel: the image the server made of a file the agent showed, by its SHA-256, which a
/// client fetches with [`crate::ClientMessage::FetchImage`] and keeps by, so that it fetches each image once however
/// many panels show it. Its body is the image's caption. The image is a PNG of 8-bit RGB, no larger than
/// [`IMAGE_SIDE`] on its longer side or [`IMAGE_BYTES`], without metadata, which is the only format a client decodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Image {
    /// The SHA-256 of its PNG, as 64 lower-case hexadecimal digits.
    pub hash: String,
    /// Its size, in pixels.
    pub width: u32,
    pub height: u32,
    /// How many bytes its PNG has.
    pub bytes: u32,
}

impl Image {
    /// The image whose PNG of `bytes` bytes has the SHA-256 `hash`, `width` by `height` pixels.
    pub fn new(hash: impl Into<String>, width: u32, height: u32, bytes: u32) -> Self {
        Self {
            hash: hash.into(),
            width,
            height,
            bytes,
        }
    }

    /// Whether it is one a client can fetch and show: its hash is 64 lower-case hexadecimal digits, it has pixels, and
    /// it is within [`IMAGE_SIDE`] and [`IMAGE_BYTES`].
    pub fn is_valid(&self) -> bool {
        hash_bytes(&self.hash).is_some()
            && (1..=IMAGE_SIDE).contains(&self.width)
            && (1..=IMAGE_SIDE).contains(&self.height)
            && (1..=IMAGE_BYTES).contains(&self.bytes)
    }
}

/// `hash`, 64 lower-case hexadecimal digits, as the 32 bytes of a SHA-256, or `None` if it is not one.
pub fn hash_bytes(hash: &str) -> Option<[u8; 32]> {
    let digits = hash.as_bytes();
    if digits.len() != 64 {
        return None;
    }
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut bytes = [0; 32];
    for (index, pair) in digits.chunks(2).enumerate() {
        bytes[index] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(bytes)
}

/// The 32 bytes of a SHA-256 as 64 lower-case hexadecimal digits, as [`Image::hash`] names it.
pub fn hash_text(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Why an [`ImageChunk`] could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageChunkError {
    /// Shorter than its header.
    Truncated,
    /// It reaches past the end of its image, or its image is larger than [`IMAGE_BYTES`].
    OutOfBounds,
}

impl std::fmt::Display for ImageChunkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(formatter, "header is truncated"),
            Self::OutOfBounds => write!(formatter, "chunk is outside its image"),
        }
    }
}

impl std::error::Error for ImageChunkError {}

/// Part of an image's PNG, sent in a [`crate::FrameKind::Image`] frame: the image's SHA-256, where the part starts in
/// it, how many bytes the whole image has, and the part, at most [`IMAGE_CHUNK_BYTES`]. Encoded as the hash's 32 bytes,
/// then the start and the length, each a little-endian `u32`, then the part. The server sends an image's chunks in
/// order, from its start, and nothing else of that image between them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageChunk {
    pub hash: [u8; 32],
    pub offset: u32,
    pub total: u32,
    pub data: Vec<u8>,
}

impl ImageChunk {
    pub fn new(hash: [u8; 32], offset: u32, total: u32, data: Vec<u8>) -> Self {
        Self {
            hash,
            offset,
            total,
            data,
        }
    }

    /// The chunks of `image`, whose SHA-256 is `hash`, in order, each of at most [`IMAGE_CHUNK_BYTES`].
    pub fn split(hash: [u8; 32], image: &[u8]) -> impl Iterator<Item = ImageChunk> + '_ {
        let total = image.len() as u32;
        image
            .chunks(IMAGE_CHUNK_BYTES)
            .enumerate()
            .map(move |(index, data)| {
                ImageChunk::new(
                    hash,
                    (index * IMAGE_CHUNK_BYTES) as u32,
                    total,
                    data.to_vec(),
                )
            })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CHUNK_HEADER + self.data.len());
        bytes.extend_from_slice(&self.hash);
        bytes.extend_from_slice(&self.offset.to_le_bytes());
        bytes.extend_from_slice(&self.total.to_le_bytes());
        bytes.extend_from_slice(&self.data);
        bytes
    }

    /// The chunk `bytes` encode, if it lies within its image, and its image within [`IMAGE_BYTES`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ImageChunkError> {
        if bytes.len() < CHUNK_HEADER {
            return Err(ImageChunkError::Truncated);
        }
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        let (offset, total) = (word(32), word(36));
        let data = bytes[CHUNK_HEADER..].to_vec();
        let end = u64::from(offset) + data.len() as u64;
        if total > IMAGE_BYTES || end > u64::from(total) {
            return Err(ImageChunkError::OutOfBounds);
        }
        Ok(Self {
            hash: bytes[..32].try_into().expect("32 bytes"),
            offset,
            total,
            data,
        })
    }
}

/// A panel: a title, a body, and a little metadata, all bounded and cleaned by the server. A text panel's body is
/// Markdown-lite text, which is drawn as text and never as HTML; a code panel's, with [`Panel::code`], the first lines
/// of a block of code, drawn as they are, coloured by its spans; an image panel's, with [`Panel::image`], its image's
/// caption, drawn below it; a chart panel's, with [`Panel::chart`], a plain summary of its chart, drawn only by a client
/// that cannot draw the chart.
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
    /// The agent's own id for it, a short slug such as `agenda`, by which the agent updates it in place, if it gave one.
    /// An older client ignores it, and draws the panel's new text as it draws any change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// What makes it an image panel, if it is one. Sent only to a client whose `Hello` sets `images`; another is sent
    /// the panel as a text panel saying that it is an image, with its caption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    /// Its version: the space's projection revision, [`Projection::revision`], at which it last changed, in anything but
    /// its age and its live data ([`Panel::data_seq`]): its title, body, metadata, kind, id, chart's structure, where it
    /// is and whether it is pinned there. A client keeps it as each projection and patch sends it, and says the version
    /// it saw when it ticks an item, so that the server refuses a tick, or an agent's update, made against a panel that
    /// changed since. Sent as `revision`, the name a client that ticks has always sent back; 0, and absent, from older
    /// servers. See `design/projection.md`.
    #[serde(rename = "revision", default, skip_serializing_if = "is_zero")]
    pub version: u64,
    /// The projection revision at which its live data last changed, for a panel written only by its environment's server
    /// side, as a chart panel's points are: each series' values and breaks, its badge, status line and summary, which a
    /// patch's [`PatchOp::ChartAppend`] and [`PatchOp::ChartReplace`] change without changing its version, so that
    /// live data never refuses anyone's edit. A client applies such an op only where it is newer than what it holds. 0,
    /// and absent, for a panel with no live data, and from older servers.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub data_seq: u64,
    /// What makes it a chart panel, if it is one: its chart, which its environment's server side published. Its body is
    /// a summary of what the chart shows now, in plain text, which an older client, which knows no charts, draws as a
    /// text panel's. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<Box<crate::chart::Chart>>,
    /// What makes it a list panel, if its environment's server side published it as one: each of its rows, in order,
    /// the first on the first line of its body and each on the next, with the environment's id for it and whether a
    /// participant may choose it, by which a client says which row it points at and which it chose
    /// ([`crate::ClientMessage::ChoosePanelRow`]), and whether it is highlighted, its tone and its value, by which a
    /// client draws it as a row. An older client ignores it, and draws the body as any list. See
    /// `design/panel-actions.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<ListRow>,
    /// What makes a list panel a table, if its environment gave its rows in columns: each column's title, which may be
    /// empty, how its cells are aligned and whether its rows' values are drawn under it, the first
    /// [`LIST_COLUMNS`] of them, each row's cells ([`ListRow::cells`]) in their order. A client that draws tables lays
    /// its rows out as a grid under a header, and an older client ignores it, drawing the body, a line of each row's
    /// text, as any list. See `design/panel-actions.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ListColumn>,
    /// What makes it a volume, a panel with depth, if it is one: its diagram, laid out by the server. Its body is the
    /// diagram's outline, which a client that draws no volumes shows; the server sends it only to a client whose
    /// `Hello` sets `diagrams`, within its [`crate::Limits::volumes`] and [`crate::Limits::nodes`]. See
    /// `design/diagrams.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram: Option<Box<crate::diagram::Diagram>>,
    /// How large it floats, as a proportion of its own size, where a participant stretched or shrank it with both
    /// hands: within [`FLOAT_SIZES`], and absent for its own size. Only a panel floating has one, since a wall's slot
    /// sizes the panels in it. An older client ignores it, drawing the panel at its own size. See
    /// `design/projection.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Its context, for a panel its environment published that depends on any: a chip for each parameter it depends on
    /// and its time, as it is shown now, each following, fixed by its own setting or inherited from its wall's, at most
    /// [`crate::context::CONTEXT_CHIPS`]. Part of its version. An older client ignores it. See
    /// `design/panel-context.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<crate::context::ContextChip>,
}

/// How small and how large a floating panel is stretched, as a proportion of its own size ([`Panel::size`]): never so
/// small that its text is smaller than half a standard slot's, as a wall's never is ([`crate::wall::LEAST_TEXT`]), and
/// at most four times as large, for reading from across a room.
pub const FLOAT_SIZES: (f32, f32) = (crate::wall::LEAST_TEXT, 4.0);

/// The most columns of a list panel a client lays out ([`Panel::columns`]); the rest are left out.
pub const LIST_COLUMNS: usize = 8;

/// The most characters of a list panel's cell, or a column's title, a client lays out; the rest are left out.
pub const LIST_CELL_CHARS: usize = 32;

/// A column of a list panel laid out as a table, [`Panel::columns`]: its title, how its cells are aligned, and whether
/// each row's value ([`ListRow::value`]) is drawn as a bar under its cell rather than under the whole row.
///
/// ```
/// use tomori_protocol::{CellAlign, ListColumn};
///
/// let busy = ListColumn::new("Busy").aligned(CellAlign::Right).with_value(true);
/// assert_eq!(
///     serde_json::to_string(&busy).unwrap(),
///     r#"{"title":"Busy","align":"right","value":true}"#
/// );
/// assert_eq!(serde_json::to_value(ListColumn::new("Name")).unwrap(), serde_json::json!({"title": "Name"}));
/// // An alignment a newer server sends reads as left.
/// let newer: ListColumn = serde_json::from_str(r#"{"title": "x", "align": "decimal"}"#).unwrap();
/// assert_eq!(newer.align, CellAlign::Left);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListColumn {
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "is_left")]
    pub align: CellAlign,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub value: bool,
}

impl ListColumn {
    /// The column `title`, its cells aligned left, with no values under it.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            align: CellAlign::Left,
            value: false,
        }
    }

    /// The column, its cells aligned as `align` says.
    pub fn aligned(mut self, align: CellAlign) -> Self {
        self.align = align;
        self
    }

    /// The column, with each row's value drawn under its cell where `value` says.
    pub fn with_value(mut self, value: bool) -> Self {
        self.value = value;
        self
    }
}

/// How the cells of a list panel's column are aligned: left, as text reads, right, as numbers read, so that their
/// digits line up, every character taking one cell of a panel, or centred. One a client does not know reads as left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CellAlign {
    Right,
    Centre,
    #[default]
    #[serde(other)]
    Left,
}

fn is_left(align: &CellAlign) -> bool {
    *align == CellAlign::Left
}

/// A row of a list panel, [`Panel::rows`]: its environment's id for it, at most 64 bytes of ASCII letters, digits,
/// `-`, `_`, `.` and `/`, whether a participant may choose it, and how a client that draws rows draws it: whether it is
/// the row its environment highlights, on a bar of its own, its tone, as a strip at its start and a mark in place of
/// its bullet, and a value from 0 to 1, as a thin bar under its text. An older client reads none of the three, and
/// draws the body's list as it is. See `design/panel-actions.md`.
///
/// ```
/// use tomori_protocol::{ListRow, chart::Tone};
///
/// let row = ListRow::new("pod112", true).with_highlight(true).with_tone(Tone::Warn).with_value(0.45);
/// assert_eq!(
///     serde_json::to_string(&row).unwrap(),
///     r#"{"id":"pod112","choosable":true,"highlighted":true,"tone":"warn","value":0.45}"#
/// );
/// // A row that says none of them is as an older server sent it.
/// assert_eq!(serde_json::to_value(ListRow::new("pod7", false)).unwrap(), serde_json::json!({"id": "pod7"}));
/// // A tone a newer server sends reads as neutral, and a value is drawn within 0 to 1, or not at all.
/// let newer: ListRow = serde_json::from_str(r#"{"id": "pod9", "tone": "urgent", "value": 7.5}"#).unwrap();
/// assert_eq!((newer.tone, newer.fraction()), (Tone::Neutral, Some(1.0)));
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListRow {
    pub id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub choosable: bool,
    /// Whether it is the row its environment highlights, which the body marks with its pointer too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub highlighted: bool,
    /// How it reads: neutral, good, a warning or bad, named for what it means rather than a colour. One a client
    /// does not know reads as neutral.
    #[serde(default, skip_serializing_if = "is_neutral")]
    pub tone: crate::chart::Tone,
    /// A value against its limit, from 0 to 1, such as how full a queue is, if it has one: read with
    /// [`ListRow::fraction`], which holds whatever was sent within that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
    /// Its cells, for a list laid out as a table ([`Panel::columns`]), one for each column, in their order, each on one
    /// line: a client lays out those of the columns it lays out, each cut to [`LIST_CELL_CHARS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<String>,
    /// The revision of its key, for a row of a list whose rows have keys: a number the server gives the key its
    /// environment's side gave the row, the same for as long as that key is unchanged, and never the key itself, which
    /// stays with the server. A client says it back as it chooses the row
    /// ([`crate::ClientMessage::ChoosePanelRow`]'s `key`), so that the server hands the side the key the participant
    /// saw, or refuses the choice as stale. Absent from an older server's, and for a list without keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<u64>,
}

impl ListRow {
    /// The row `id`, which a participant may choose where `choosable` says, neutral, not highlighted and without a
    /// value.
    pub fn new(id: impl Into<String>, choosable: bool) -> Self {
        Self {
            id: id.into(),
            choosable,
            highlighted: false,
            tone: crate::chart::Tone::Neutral,
            value: None,
            cells: Vec::new(),
            key: None,
        }
    }

    /// The row, its key at revision `key`.
    pub fn with_key(mut self, key: u64) -> Self {
        self.key = Some(key);
        self
    }

    /// The row, with `cells`, one for each of its list's columns.
    pub fn with_cells(mut self, cells: Vec<String>) -> Self {
        self.cells = cells;
        self
    }

    /// The row, highlighted by its environment where `highlighted` says.
    pub fn with_highlight(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    /// The row, in `tone`.
    pub fn with_tone(mut self, tone: crate::chart::Tone) -> Self {
        self.tone = tone;
        self
    }

    /// The row, with `value` of its limit, as [`row_value`] holds it: none where it is not finite.
    pub fn with_value(mut self, value: f32) -> Self {
        self.value = row_value(value);
        self
    }

    /// Its value as it is drawn, from 0 to 1: none where it has none, or one that is not finite, as a number too
    /// large for the wire's reader is.
    pub fn fraction(&self) -> Option<f32> {
        self.value.and_then(row_value)
    }
}

/// `value` as a row's value, held within 0 to 1, or none where it is not finite.
///
/// ```
/// use tomori_protocol::row_value;
///
/// assert_eq!([row_value(0.45), row_value(-2.0), row_value(7.5)], [Some(0.45), Some(0.0), Some(1.0)]);
/// assert_eq!([row_value(f32::NAN), row_value(f32::INFINITY)], [None, None]);
/// ```
pub fn row_value(value: f32) -> Option<f32> {
    value.is_finite().then(|| value.clamp(0.0, 1.0))
}

fn is_neutral(tone: &crate::chart::Tone) -> bool {
    *tone == crate::chart::Tone::Neutral
}

fn is_zero(value: &u64) -> bool {
    *value == 0
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
            slug: None,
            image: None,
            version: 0,
            data_seq: 0,
            chart: None,
            rows: Vec::new(),
            columns: Vec::new(),
            diagram: None,
            size: None,
            context: Vec::new(),
        }
    }

    /// The panel, floating `size` times as large as its own size, within [`FLOAT_SIZES`], or at its own size for
    /// `None`, 1, or a size that is not finite.
    pub fn with_size(mut self, size: Option<f32>) -> Self {
        self.size = size
            .filter(|size| size.is_finite() && (size - 1.0).abs() > 1e-4)
            .map(|size| size.clamp(FLOAT_SIZES.0, FLOAT_SIZES.1));
        self
    }

    /// How large it is drawn floating, as a proportion of its own size: its [`Panel::size`] within [`FLOAT_SIZES`], or 1.
    /// On a wall, its slot sizes it instead.
    ///
    /// ```
    /// use tomori_protocol::{FLOAT_SIZES, Panel, Place, Pose};
    ///
    /// let pose = Pose::upright_facing([0.0, 1.5, -1.0], [0.0, 1.5, 0.0]);
    /// let panel = Panel::new("p", "Notes", "", Place::Free { pose });
    /// assert_eq!(panel.floating_size(), 1.0);
    /// assert_eq!(panel.clone().with_size(Some(1.5)).floating_size(), 1.5);
    /// assert_eq!(panel.clone().with_size(Some(40.0)).floating_size(), FLOAT_SIZES.1);
    /// assert_eq!(panel.clone().with_size(Some(f32::NAN)).size, None);
    /// ```
    pub fn floating_size(&self) -> f32 {
        self.size
            .filter(|size| size.is_finite())
            .map_or(1.0, |size| size.clamp(FLOAT_SIZES.0, FLOAT_SIZES.1))
    }

    /// The panel, with the chips of its context.
    pub fn with_context(mut self, context: Vec<crate::context::ContextChip>) -> Self {
        self.context = context;
        self
    }

    /// The panel, a list panel of `rows`, each on its line of the body, in order.
    pub fn with_rows(mut self, rows: Vec<ListRow>) -> Self {
        self.rows = rows;
        self
    }

    /// The panel, a list laid out as a table of `columns`.
    pub fn with_columns(mut self, columns: Vec<ListColumn>) -> Self {
        self.columns = columns;
        self
    }

    /// The row on `line` of its body, from 0, if it is a list panel with a row there.
    pub fn row_on(&self, line: u32) -> Option<&ListRow> {
        self.rows.get(line as usize)
    }

    /// The panel at `version`, the projection revision at which it last changed.
    pub fn with_version(mut self, version: u64) -> Self {
        self.version = version;
        self
    }

    /// The panel, its live data last changed at the projection revision `seq`.
    pub fn with_data_seq(mut self, seq: u64) -> Self {
        self.data_seq = seq;
        self
    }

    /// Whether it and `other` are alike in everything its version covers but where they are: their identity, title,
    /// metadata, kind, id, code, image, chart's structure, rows and columns, and, but for a chart panel, whose body is a summary of
    /// its live data, their body. See [`Panel::version`].
    pub fn same_structure(&self, other: &Panel) -> bool {
        let Panel {
            id,
            title,
            body,
            meta,
            age: _,
            place: _,
            code,
            pinned: _,
            slug,
            image,
            version: _,
            data_seq: _,
            chart,
            rows,
            columns,
            diagram,
            size,
            context,
        } = self;
        let charts = match (chart, &other.chart) {
            (None, None) => true,
            (Some(chart), Some(other)) => chart.same_structure(other),
            _ => false,
        };
        *id == other.id
            && *title == other.title
            && (chart.is_some() || *body == other.body)
            && *meta == other.meta
            && *code == other.code
            && *slug == other.slug
            && *image == other.image
            && *rows == other.rows
            && *columns == other.columns
            && *diagram == other.diagram
            && *size == other.size
            && *context == other.context
            && charts
    }

    /// Whether it and `other` show the same live data: for chart panels, their charts' data and their summaries, the
    /// bodies; for any other, always. See [`Panel::data_seq`].
    pub fn same_data(&self, other: &Panel) -> bool {
        match (&self.chart, &other.chart) {
            (Some(chart), Some(other_chart)) => {
                chart.same_data(other_chart) && self.body == other.body
            }
            _ => true,
        }
    }

    /// The panel, a chart panel of `chart`, its body a summary of it.
    pub fn with_chart(mut self, chart: crate::chart::Chart) -> Self {
        self.chart = Some(Box::new(chart));
        self
    }

    /// The panel, a volume of `diagram`, its body its outline.
    pub fn with_diagram(mut self, diagram: crate::diagram::Diagram) -> Self {
        self.diagram = Some(Box::new(diagram));
        self
    }

    /// The panel, which the agent calls `slug`.
    pub fn with_slug(mut self, slug: Option<String>) -> Self {
        self.slug = slug;
        self
    }

    /// The panel, an image panel of `image`.
    pub fn with_image(mut self, image: Image) -> Self {
        self.image = Some(image);
        self
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
    fn a_floating_panel_s_size_is_sent_only_where_it_has_one_and_read_within_its_bounds() {
        let pose = Pose::upright_facing([0.0, 1.5, -1.0], [0.0, 1.5, 0.0]);
        let panel = Panel::new("p", "Notes", "", Place::Free { pose });
        // At its own size it says none, as an older server sends it.
        let json = serde_json::to_value(&panel).unwrap();
        assert!(json.get("size").is_none());
        assert_eq!(panel.clone().with_size(Some(1.0)).size, None);
        let sized = panel.clone().with_size(Some(1.5));
        assert_eq!(serde_json::to_value(&sized).unwrap()["size"], 1.5);
        // A newer or stranger server's is read within the bounds a floating panel has.
        let mut read = json.clone();
        read["size"] = serde_json::json!(9.0);
        let read: Panel = serde_json::from_value(read).unwrap();
        assert_eq!(read.floating_size(), FLOAT_SIZES.1);
        let mut small = json;
        small["size"] = serde_json::json!(0.01);
        let small: Panel = serde_json::from_value(small).unwrap();
        assert_eq!(small.floating_size(), FLOAT_SIZES.0);
        let nonsense = Panel {
            size: Some(f32::INFINITY),
            ..panel.clone()
        };
        assert_eq!(nonsense.floating_size(), 1.0);
        // Its size is part of its structure.
        assert!(!panel.same_structure(&sized));
    }

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
        // A wall of its own grid and size says so; an older server's says neither.
        let sized = Wall::new("wall-2", WallKind::Added, pose)
            .with_grid(Grid::from((4, 1)))
            .with_scale(1.25);
        let json = serde_json::json!({"id": "wall-2", "kind": "added",
            "pose": {"position": [1.0, 1.5, -2.0], "orientation": [0.0, 0.0, 0.0, 1.0]},
            "grid": [4, 1], "scale": 1.25});
        assert_eq!(serde_json::to_value(&sized).unwrap(), json);
        assert_eq!(serde_json::from_value::<Wall>(json).unwrap(), sized);
        assert_eq!((sized.slots(), sized.scale()), (4, 1.25));
        assert_eq!(
            (curved.grid, curved.scale, curved.aspect),
            (None, None, None)
        );
        // Slots of their own shape say so, and a reader that knows no aspect reads the rest, its slots its scale.
        let tall = sized.clone().with_aspect(Some(0.5625));
        let mut json = serde_json::to_value(&tall).unwrap();
        assert_eq!(json["aspect"], 0.5625);
        assert_eq!(serde_json::from_value::<Wall>(json.clone()).unwrap(), tall);
        json.as_object_mut().unwrap().remove("aspect");
        let older = serde_json::from_value::<Wall>(json).unwrap();
        assert_eq!(
            (older.scale(), older.slot_size()),
            (
                1.25,
                (layout::SLOT_SIZE.0 * 1.25, layout::SLOT_SIZE.1 * 1.25)
            )
        );
        assert_eq!((curved.slots(), curved.scale()), (6, layout::ARC_SCALE));
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
    fn an_image_panel_names_its_image_by_hash_and_older_panels_have_none() {
        let hash = "0123456789abcdef".repeat(4);
        let place = Place::Wall {
            wall: TOMORI_WALL.into(),
            slot: 2,
        };
        let panel = Panel::new("panel-3", "Layout", "The session browser", place)
            .with_image(Image::new(hash.clone(), 1600, 900, 123_456));
        let json = serde_json::json!({
            "id": "panel-3", "title": "Layout", "body": "The session browser", "age": 0,
            "place": {"kind": "wall", "wall": "tomori", "slot": 2},
            "image": {"hash": hash, "width": 1600, "height": 900, "bytes": 123_456}
        });
        assert_eq!(serde_json::to_value(&panel).unwrap(), json);
        assert_eq!(serde_json::from_value::<Panel>(json).unwrap(), panel);
        assert!(panel.image.as_ref().unwrap().is_valid());
        // A text panel says nothing of images, and a newer image's fields are ignored.
        let text = Panel::new("panel-1", "t", "b", Place::Unknown);
        assert!(serde_json::to_value(&text).unwrap().get("image").is_none());
        let newer: Image = serde_json::from_value(serde_json::json!({
            "hash": hash, "width": 1, "height": 1, "bytes": 1, "format": "avif"
        }))
        .unwrap();
        assert!(newer.is_valid());
        // Only an image a client can fetch and show is valid.
        let invalid = [
            Image::new("ABCDEF".repeat(10) + "abcd", 10, 10, 10),
            Image::new(hash.replace('0', "g"), 10, 10, 10),
            Image::new(&hash[1..], 10, 10, 10),
            Image::new(hash.clone(), 0, 10, 10),
            Image::new(hash.clone(), 10, IMAGE_SIDE + 1, 10),
            Image::new(hash.clone(), 10, 10, 0),
            Image::new(hash.clone(), 10, 10, IMAGE_BYTES + 1),
        ];
        for image in invalid {
            assert!(!image.is_valid(), "{image:?}");
        }
        // Hashes are read from and written as lower-case hexadecimal.
        let bytes = hash_bytes(&hash).unwrap();
        assert_eq!(bytes[..2], [0x01, 0x23]);
        assert_eq!(hash_text(&bytes), hash);
        assert!(hash_bytes("é".repeat(32).as_str()).is_none());
        assert!(hash_bytes(&format!("0g{}", "0".repeat(62))).is_none());
        // Fetching one names it by its hash.
        let fetch = crate::ClientMessage::FetchImage {
            image: hash.clone(),
        };
        let json = serde_json::json!({"type": "fetch_image", "image": hash});
        assert_eq!(serde_json::to_value(&fetch).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<crate::ClientMessage>(json).unwrap(),
            fetch
        );
        // An older client's Hello shows no images; a newer one's says it does.
        let hello: crate::ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello", "protocol": 2, "participant": "p", "name": "Ada", "images": true
        }))
        .unwrap();
        assert!(matches!(
            hello,
            crate::ClientMessage::Hello { images: true, .. }
        ));
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
    fn a_wall_says_it_is_locked_only_where_it_is_and_a_reader_that_knows_no_lock_reads_the_rest() {
        let pose = Pose {
            position: [3.0, 1.4, -4.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let open =
            Wall::new("environment-0", WallKind::Environment, pose).with_grid(Grid::from((2, 1)));
        assert!(!open.is_locked());
        let json = serde_json::to_value(&open).unwrap();
        assert!(json.get("locked").is_none(), "{json}");
        let locked = open.clone().with_locked(true);
        assert!(locked.is_locked());
        let mut json = serde_json::to_value(&locked).unwrap();
        assert_eq!(json["locked"], true);
        assert_eq!(
            serde_json::from_value::<Wall>(json.clone()).unwrap(),
            locked
        );
        // An older server's wall, which says nothing of a lock, is unlocked; an older client, which knows none, reads
        // the rest of a locked wall as it is.
        json.as_object_mut().unwrap().remove("locked");
        assert_eq!(serde_json::from_value::<Wall>(json).unwrap(), open);
        // Locking and unlocking, and putting back, on the wire; `reset` only where it is asked.
        use crate::ClientMessage;
        for (message, json) in [
            (
                ClientMessage::LockWall {
                    wall: "environment-0".into(),
                    locked: false,
                    reset: false,
                },
                serde_json::json!({"type": "lock_wall", "wall": "environment-0", "locked": false}),
            ),
            (
                ClientMessage::LockWall {
                    wall: "environment-0".into(),
                    locked: true,
                    reset: true,
                },
                serde_json::json!({"type": "lock_wall", "wall": "environment-0", "locked": true, "reset": true}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&message).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<ClientMessage>(json).unwrap(),
                message
            );
        }
        // A space's state says the server locks walls only where it does; an older server's says nothing, read as not.
        let state = crate::SpaceState {
            wall_locks: true,
            ..Default::default()
        };
        let mut json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["wall_locks"], true);
        json.as_object_mut().unwrap().remove("wall_locks");
        assert!(
            !serde_json::from_value::<crate::SpaceState>(json)
                .unwrap()
                .wall_locks
        );
        assert!(
            serde_json::to_value(crate::SpaceState::default())
                .unwrap()
                .get("wall_locks")
                .is_none()
        );
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
                    size: None,
                    revision: None,
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "wall": "tomori"}),
            ),
            (
                ClientMessage::PlacePanel {
                    panel: "panel-1".into(),
                    wall: Some(TOMORI_WALL.into()),
                    slot: Some(4),
                    pose: None,
                    size: None,
                    revision: None,
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "wall": "tomori", "slot": 4}),
            ),
            (
                ClientMessage::PlacePanel {
                    panel: "panel-1".into(),
                    wall: None,
                    slot: None,
                    pose: Some(pose),
                    size: None,
                    revision: None,
                },
                serde_json::json!({"type": "place_panel", "panel": "panel-1", "pose": pose_json}),
            ),
            (
                ClientMessage::SetWall {
                    wall: "wall-2".into(),
                    grid: Some(Grid::from((4, 1))),
                    scale: None,
                    aspect: None,
                    shape: Some(WallShape::Arc { radius: 3.0 }),
                    pose: None,
                    tilt: None,
                    content_scale: None,
                    revision: None,
                },
                serde_json::json!({"type": "set_wall", "wall": "wall-2", "grid": [4, 1],
                    "shape": {"kind": "arc", "radius": 3.0}}),
            ),
            (
                ClientMessage::SetWall {
                    wall: "wall-2".into(),
                    grid: None,
                    scale: Some(1.5),
                    aspect: Some(0.5625),
                    shape: Some(WallShape::Grid),
                    pose: Some(pose),
                    tilt: Some(-12.0),
                    content_scale: None,
                    revision: None,
                },
                serde_json::json!({"type": "set_wall", "wall": "wall-2", "scale": 1.5,
                    "aspect": 0.5625, "shape": {"kind": "grid"}, "pose": pose_json, "tilt": -12.0}),
            ),
            (
                ClientMessage::DismissPanel {
                    panel: "panel-1".into(),
                },
                serde_json::json!({"type": "dismiss_panel", "panel": "panel-1"}),
            ),
            (
                ClientMessage::CheckPanelItem {
                    panel: "panel-1".into(),
                    line: 3,
                    checked: true,
                    revision: Some(4),
                },
                serde_json::json!({"type": "check_panel_item", "panel": "panel-1", "line": 3,
                    "checked": true, "revision": 4}),
            ),
            // An older client says no revision, and its tick goes by its line alone.
            (
                ClientMessage::CheckPanelItem {
                    panel: "panel-1".into(),
                    line: 3,
                    checked: false,
                    revision: None,
                },
                serde_json::json!({"type": "check_panel_item", "panel": "panel-1", "line": 3,
                    "checked": false}),
            ),
            (
                ClientMessage::ChoosePanelRow {
                    panel: "panel-1".into(),
                    row: "pod/112".into(),
                    revision: Some(9),
                    key: Some(4),
                },
                serde_json::json!({"type": "choose_panel_row", "panel": "panel-1",
                    "row": "pod/112", "revision": 9, "key": 4}),
            ),
            (
                ClientMessage::ChoosePanelRow {
                    panel: "panel-1".into(),
                    row: "pod7".into(),
                    revision: None,
                    key: None,
                },
                serde_json::json!({"type": "choose_panel_row", "panel": "panel-1", "row": "pod7"}),
            ),
        ];
        // A panel's version is sent as its revision, the name a client that ticks sends back, from a server that keeps
        // one, and read as 0 from an older one; so is a wall's, and a live panel's data seq.
        let panel = Panel::new(
            "panel-1",
            "Agenda",
            "- [ ] One",
            Place::Wall {
                wall: "tomori".into(),
                slot: 0,
            },
        )
        .with_version(3)
        .with_data_seq(4);
        let json = serde_json::to_value(&panel).unwrap();
        assert_eq!(
            (&json["revision"], &json["data_seq"]),
            (&serde_json::json!(3), &serde_json::json!(4))
        );
        let mut older = json.clone();
        older.as_object_mut().unwrap().remove("revision");
        older.as_object_mut().unwrap().remove("data_seq");
        let older = serde_json::from_value::<Panel>(older).unwrap();
        assert_eq!((older.version, older.data_seq), (0, 0));
        let unversioned = serde_json::to_value(panel.with_version(0).with_data_seq(0)).unwrap();
        assert!(unversioned.get("revision").is_none() && unversioned.get("data_seq").is_none());
        let wall = Wall::new("wall-1", WallKind::Added, Pose::default()).with_version(7);
        let json = serde_json::to_value(&wall).unwrap();
        assert_eq!(json["revision"], 7);
        assert_eq!(serde_json::from_value::<Wall>(json).unwrap(), wall);
        assert!(
            serde_json::to_value(wall.with_version(0))
                .unwrap()
                .get("revision")
                .is_none()
        );
        // A refused tick names its panel, which an older client ignores; other errors name none.
        let refused = ServerMessage::Error {
            error: "That panel has changed.".into(),
            space: None,
            panel: Some("panel-1".into()),
            wall: None,
        };
        let json = serde_json::to_value(&refused).unwrap();
        assert_eq!(json["panel"], "panel-1");
        assert_eq!(
            serde_json::from_value::<ServerMessage>(json).unwrap(),
            refused
        );
        assert!(
            serde_json::to_value(ServerMessage::Error {
                error: "x".into(),
                space: None,
                panel: None,
                wall: None,
            })
            .unwrap()
            .get("panel")
            .is_none()
        );
        for (message, json) in cases {
            assert_eq!(serde_json::to_value(&message).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<ClientMessage>(json).unwrap(),
                message
            );
        }
        // A server says it takes choices only when it does; an older one's state takes none.
        let state = crate::SpaceState {
            choices: true,
            ..Default::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["choices"], true);
        let mut older = json.clone();
        older.as_object_mut().unwrap().remove("choices");
        assert!(
            !serde_json::from_value::<crate::SpaceState>(older)
                .unwrap()
                .choices
        );
        // A server says it takes ticking only when it does; an older one's state, which says nothing, takes none.
        let state = crate::SpaceState {
            checks: true,
            ..Default::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["checks"], true);
        let mut older = json.clone();
        older.as_object_mut().unwrap().remove("checks");
        assert!(
            !serde_json::from_value::<crate::SpaceState>(older)
                .unwrap()
                .checks
        );
        assert!(
            serde_json::to_value(crate::SpaceState::default())
                .unwrap()
                .get("checks")
                .is_none()
        );
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
        // Pointed at on a list item, it names the line of the body the item begins on, which a reader that knows only
        // the identity skips, as it skips any field it does not know.
        let item = Resource::panel_at("panel-1", Some(4));
        let json = serde_json::json!({"kind": "panel", "panel": "panel-1", "line": 4});
        assert_eq!(serde_json::to_value(&item).unwrap(), json);
        assert_eq!(serde_json::from_value::<Resource>(json).unwrap(), item);
        assert_eq!(Resource::panel_at("panel-1", None), panel);
        // Pointed at on a chart's plot, it says where across its time axis, which an older server skips.
        let chart = Resource::chart_at("panel-1", 6_000);
        let json = serde_json::json!({"kind": "panel", "panel": "panel-1", "across": 6000});
        assert_eq!(serde_json::to_value(&chart).unwrap(), json);
        assert_eq!(serde_json::from_value::<Resource>(json).unwrap(), chart);
        // A panel with the agent's id carries it only when it has one, and a client that knows no id, as an older one,
        // reads the rest as before, so that a panel updated in place shows its new text there too.
        let plain = Panel::new("panel-1", "Agenda", "- [ ] One", Place::Unknown);
        assert!(serde_json::to_value(&plain).unwrap().get("slug").is_none());
        let named = plain.with_slug(Some("agenda".into()));
        let json = serde_json::to_value(&named).unwrap();
        assert_eq!(json["slug"], "agenda");
        assert_eq!(
            serde_json::from_value::<Panel>(json.clone()).unwrap(),
            named
        );
        #[derive(Deserialize)]
        struct Older {
            id: String,
            title: String,
            body: String,
        }
        let older: Older = serde_json::from_value(json).unwrap();
        assert_eq!(
            (older.id.as_str(), older.title.as_str(), older.body.as_str()),
            ("panel-1", "Agenda", "- [ ] One")
        );
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
        assert!(!state.walls);
    }

    #[test]
    fn revisions_patches_and_limits_are_absent_from_older_peers() {
        use crate::{ClientMessage, ServerMessage};
        // An older server's projection has no revision, and an older client's Hello takes no patches and states no
        // limits, which read as today's.
        let older: Projection = serde_json::from_value(serde_json::json!({
            "space": "s", "environment": "sky", "walls": [], "panels": []
        }))
        .unwrap();
        assert_eq!(older.revision, 0);
        assert_eq!(
            serde_json::to_value(Projection::new("s", "sky").with_revision(7)).unwrap(),
            serde_json::json!({
                "space": "s", "environment": "sky", "walls": [], "panels": [], "revision": 7
            })
        );
        let hello: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello", "protocol": 2, "participant": "p", "name": "Ada", "projection": true
        }))
        .unwrap();
        let ClientMessage::Hello {
            projection_patches,
            limits,
            ..
        } = hello
        else {
            unreachable!("a hello");
        };
        assert!(!projection_patches);
        assert_eq!(limits.unwrap_or_default(), crate::Limits::TODAY);
        let hello: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello", "protocol": 2, "participant": "p", "name": "Ada", "projection": true,
            "projection_patches": true, "limits": {"slots": 500}
        }))
        .unwrap();
        let ClientMessage::Hello {
            projection_patches: true,
            live_panels: false,
            limits: Some(limits),
            ..
        } = hello
        else {
            unreachable!("a hello taking patches");
        };
        assert_eq!(
            limits,
            crate::Limits::TODAY.with_projection(13, 500, 12, 90)
        );
        // A newer client says it applies data ops too.
        let hello: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "hello", "protocol": 2, "participant": "p", "name": "Ada", "projection": true,
            "projection_patches": true, "live_panels": true
        }))
        .unwrap();
        assert!(matches!(
            hello,
            ClientMessage::Hello {
                live_panels: true,
                ..
            }
        ));
        // A wall's change says the version it was made against, as `revision`, which an older client says not, and a
        // refusal for it names the wall, which an older client ignores.
        let change = ClientMessage::SetWall {
            wall: "wall-1".into(),
            grid: None,
            scale: None,
            aspect: None,
            shape: None,
            pose: None,
            tilt: Some(10.0),
            content_scale: None,
            revision: Some(7),
        };
        let json = serde_json::to_value(&change).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "set_wall", "wall": "wall-1", "tilt": 10.0, "revision": 7})
        );
        assert_eq!(
            serde_json::from_value::<ClientMessage>(json).unwrap(),
            change
        );
        let older: ClientMessage = serde_json::from_value(
            serde_json::json!({"type": "set_wall", "wall": "wall-1", "tilt": 10.0}),
        )
        .unwrap();
        let ClientMessage::SetWall { revision, .. } = older else {
            unreachable!("a wall's change");
        };
        assert_eq!(revision, None);
        let refused = ServerMessage::Error {
            error: "That wall changed as you changed it; look again.".into(),
            space: None,
            panel: None,
            wall: Some("wall-1".into()),
        };
        let json = serde_json::to_value(&refused).unwrap();
        assert_eq!(json["wall"], "wall-1");
        assert_eq!(
            serde_json::from_value::<ServerMessage>(json).unwrap(),
            refused
        );
        // An older server's Welcome states no limits.
        let welcome: ServerMessage = serde_json::from_value(serde_json::json!({
            "type": "welcome", "protocol": 2, "server": "tomori", "source": 1, "backends": [], "roots": []
        }))
        .unwrap();
        let ServerMessage::Welcome { limits, .. } = welcome else {
            unreachable!("a welcome");
        };
        assert_eq!(limits, None);
        // A patch and a resync have their own types.
        let patch = ServerMessage::ProjectionPatch {
            patch: Box::new(ProjectionPatch::new("s", "sky", 1, 2, Vec::new())),
        };
        let json = serde_json::to_value(&patch).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "projection_patch",
                "patch": {"space": "s", "environment": "sky", "from": 1, "to": 2}
            })
        );
        assert_eq!(
            serde_json::from_value::<ServerMessage>(json).unwrap(),
            patch
        );
        let resync = ClientMessage::ResyncProjection { revision: 4 };
        let json = serde_json::to_value(&resync).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "resync_projection", "revision": 4})
        );
        assert_eq!(
            serde_json::from_value::<ClientMessage>(json).unwrap(),
            resync
        );
    }

    #[test]
    fn a_list_panel_s_rows_are_sent_only_where_it_has_them_and_count_in_its_version() {
        let place = Place::Wall {
            wall: "environment-0".into(),
            slot: 0,
        };
        let list =
            Panel::new("panel-1", "Queue", "- pod7\n- pod112", place.clone()).with_rows(vec![
                ListRow::new("pod7", false),
                ListRow::new("pod112", true),
            ]);
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(
            json["rows"],
            serde_json::json!([{"id": "pod7"}, {"id": "pod112", "choosable": true}])
        );
        assert_eq!(serde_json::from_value::<Panel>(json.clone()).unwrap(), list);
        // An older server sends none, and a panel that is not a list says none.
        let mut older = json;
        older.as_object_mut().unwrap().remove("rows");
        assert!(
            serde_json::from_value::<Panel>(older)
                .unwrap()
                .rows
                .is_empty()
        );
        let text = Panel::new("panel-1", "Queue", "- pod7\n- pod112", place);
        assert!(serde_json::to_value(&text).unwrap().get("rows").is_none());
        // Its rows are found by line, and are part of what its version covers.
        assert_eq!(list.row_on(1).map(|row| row.id.as_str()), Some("pod112"));
        assert_eq!(list.row_on(2), None);
        assert_eq!(text.row_on(0), None);
        assert!(!list.same_structure(&text));
        assert!(list.same_structure(&list.clone().with_age(9)));
    }

    #[test]
    fn a_row_s_highlight_tone_and_value_are_sent_only_where_it_has_them() {
        use crate::chart::Tone;
        let row = ListRow::new("pod112", true)
            .with_highlight(true)
            .with_tone(Tone::Bad)
            .with_value(0.5);
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"id": "pod112", "choosable": true, "highlighted": true, "tone": "bad", "value": 0.5})
        );
        assert_eq!(serde_json::from_value::<ListRow>(json).unwrap(), row);
        // A row that says none of them is sent as an older server sent it, and read so from one.
        let plain = ListRow::new("pod7", false);
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            serde_json::json!({"id": "pod7"})
        );
        assert_eq!(
            serde_json::from_value::<ListRow>(serde_json::json!({"id": "pod7"})).unwrap(),
            plain
        );
        assert_eq!(plain.fraction(), None);
        // A tone a newer server sends is neutral, and a value is held within 0 to 1, or left out.
        let newer: ListRow =
            serde_json::from_str(r#"{"id": "pod9", "tone": "urgent", "value": -3.0}"#).unwrap();
        assert_eq!((newer.tone, newer.fraction()), (Tone::Neutral, Some(0.0)));
        let mut odd = ListRow::new("pod9", false);
        odd.value = Some(f32::NAN);
        assert_eq!(odd.fraction(), None);
        assert_eq!(
            ListRow::new("pod9", false).with_value(f32::INFINITY).value,
            None
        );
        assert_eq!(
            [
                row_value(0.45),
                row_value(-2.0),
                row_value(7.5),
                row_value(f32::NAN)
            ],
            [Some(0.45), Some(0.0), Some(1.0), None]
        );
        // Its highlight, tone and value are part of what the panel's version covers.
        let place = Place::Wall {
            wall: "environment-0".into(),
            slot: 0,
        };
        let list = |row: ListRow| {
            Panel::new("panel-1", "Queue", "- pod7", place.clone()).with_rows(vec![row])
        };
        let before = list(ListRow::new("pod7", false));
        assert!(!before.same_structure(&list(ListRow::new("pod7", false).with_tone(Tone::Good))));
        assert!(!before.same_structure(&list(ListRow::new("pod7", false).with_value(0.1))));
        assert!(!before.same_structure(&list(ListRow::new("pod7", false).with_highlight(true))));
    }
    #[test]
    fn a_list_s_columns_and_cells_are_sent_only_where_it_has_them_and_count_in_its_version() {
        let place = Place::Wall {
            wall: "environment-0".into(),
            slot: 0,
        };
        let row = ListRow::new("grpc", true).with_cells(vec!["GRPC".into(), "100%".into()]);
        let table = Panel::new("panel-1", "Observers", "- GRPC · Busy 100%", place.clone())
            .with_rows(vec![row.clone()])
            .with_columns(vec![
                ListColumn::new("Name"),
                ListColumn::new("Busy")
                    .aligned(CellAlign::Right)
                    .with_value(true),
            ]);
        let json = serde_json::to_value(&table).unwrap();
        assert_eq!(
            json["columns"],
            serde_json::json!([{"title": "Name"}, {"title": "Busy", "align": "right", "value": true}])
        );
        assert_eq!(
            json["rows"][0]["cells"],
            serde_json::json!(["GRPC", "100%"])
        );
        assert_eq!(
            serde_json::from_value::<Panel>(json.clone()).unwrap(),
            table
        );
        // An older server sends neither, and a list without columns says none.
        let mut older = json;
        older.as_object_mut().unwrap().remove("columns");
        older["rows"][0].as_object_mut().unwrap().remove("cells");
        let older = serde_json::from_value::<Panel>(older).unwrap();
        assert!(older.columns.is_empty() && older.rows[0].cells.is_empty());
        let list = Panel::new("panel-1", "Observers", "- GRPC", place)
            .with_rows(vec![ListRow::new("grpc", true)]);
        let json = serde_json::to_value(&list).unwrap();
        assert!(json.get("columns").is_none() && json["rows"][0].get("cells").is_none());
        // Both are part of what its version covers.
        let cells = |cells: &[&str]| {
            table.clone().with_rows(vec![
                row.clone()
                    .with_cells(cells.iter().map(|cell| (*cell).to_owned()).collect()),
            ])
        };
        assert!(table.same_structure(&cells(&["GRPC", "100%"])));
        assert!(!table.same_structure(&cells(&["GRPC", "99%"])));
        assert!(!table.same_structure(&table.clone().with_columns(Vec::new())));
        assert_eq!((LIST_COLUMNS, LIST_CELL_CHARS), (8, 32));
    }

    #[test]
    fn a_row_s_key_revision_is_sent_only_where_it_has_one_and_counts_in_its_version() {
        let row = ListRow::new("pod-a", true).with_key(3);
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            serde_json::json!({"id": "pod-a", "choosable": true, "key": 3})
        );
        let older: ListRow = serde_json::from_value(serde_json::json!({"id": "pod-a"})).unwrap();
        assert_eq!(older.key, None);
        let place = Place::Wall {
            wall: "environment-0".into(),
            slot: 0,
        };
        let list =
            |row: ListRow| Panel::new("panel-1", "Q", "- a", place.clone()).with_rows(vec![row]);
        assert!(!list(row.clone()).same_structure(&list(ListRow::new("pod-a", true).with_key(4))));
    }
}
