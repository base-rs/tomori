// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent stage: the projection walls of a space and the text panels the agent projects onto them, which
//! participants pull off, leave floating, put back and dismiss. The first slice of agent projection (issue #82). It is
//! the space's, not its environment's, and holds no I/O: the hub keeps one per space, sends what it shows, and keeps it
//! with the space across restarts. See `design/projection.md`.
//!
//! **Two kinds of wall.** Every space has one Tomori wall, which belongs to the space and stands beneath Tomori in every
//! environment, beneath the agent's home (`tomori_protocol::entourage::wall_beneath`), at the environment's `stage`
//! anchor, which overrides it, or, with neither, at [`default_stage`]; what is on it stays as the
//! space changes environment. Each environment also has walls of its own: those it declares, and those its owners add.
//! They and the panels left floating in it are kept for the space and that environment together, since positions mean
//! something only in that environment's geometry: they are hidden while the space is in another environment, and shown
//! again, where they were, when it comes back.
//!
//! **Layout.** A wall has a grid of slots, numbered from its top left, the most prominent first, laid out as
//! `tomori_protocol::wall` lays them out for every client too; participants and the agent change its grid, how large its
//! slots are and whether it is curved, and a panel keeps its column and row as they do, or, in one the wall no longer
//! has, goes into its nearest free slot or floats where it was. A panel the agent projects takes its wall's
//! most prominent free slot. A participant puts a panel in the slot they choose, and it is then arranged: when that slot
//! holds another panel, the two change places, the other going where the one put there came from, or into a free slot
//! on the same wall when it came from elsewhere and one is free. When a wall is full, the panel that has been on it
//! longest of those not arranged leaves it for a newcomer, or, when every one is arranged, the one there longest. Nothing
//! else ever moves, so that nobody's reading is disturbed.
mod blocks;
pub(crate) use tomori_protocol::checklist;
mod beside;
mod code;
pub(crate) mod context;
mod frames;
mod groups;
mod held;
mod image;
mod layouts;
mod pin;
mod placement;
mod published;
pub(crate) use beside::spot as beside_spot;
pub(crate) use beside::{SIZE as BESIDE_SIZE, level_ahead};
pub(crate) mod geometry;
mod status;
mod volume;
mod walls;

pub use blocks::{
    BLOCK_BYTES, BODY_CHARS, BODY_LINES, clean_body, panels, slug, title as clean_title,
};
pub use code::Source;
pub use image::without_image;
pub(crate) use layouts::{LAYOUT_WALLS, Planned};
pub use placement::Placement;
pub(crate) use published::Change;
#[cfg(test)]
pub(crate) use published::PUBLISHED;
pub use published::Publishing;
pub(crate) use published::Row;
#[cfg(test)]
pub(crate) use published::{WITHDRAWN, WITHDRAWN_SECS};
pub use volume::{DIAGRAM_BYTES, within_volumes};

use frames::{MESSAGE_BYTES, fit, json_size, walls_within};

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tomori_protocol::{
    Code, Grid, Meta, Panel, Place, Pose, Projection, Span, TOMORI_WALL, Wall, WallKind, WallShape,
    wall as layout,
};

/// The most walls owners may add in one environment of a space: sixty, ten times as many as before frames were
/// negotiated, so that, with the Tomori wall and [`DECLARED_WALLS`] of the environment's own, they are
/// [`tomori_protocol::Limits::MOST`]'s.
/// A client is sent only those within what it states it draws. See `design/projection.md`.
pub const ADDED_WALLS: usize = 60;

/// The most walls of an environment's own that are used, the first it declares; any more are ignored. Sixteen, where
/// six were before, so that a console of a wall for each of its stations fits with room to grow: the walls' slots
/// together are still bounded by [`SLOTS`], which bounds the projection's size, so more walls cost only their own few
/// hundred bytes each, and the headset and the desktop window state walls and slots enough to draw sixteen standard
/// walls with the Tomori wall and the walls owners add. See `design/projection.md`.
pub const DECLARED_WALLS: usize = 16;

/// The most panels left floating in one environment of a space, [`tomori_protocol::Limits::MOST`]'s; leaving another
/// floating takes away the one that has floated longest. A client is sent the most recently left of them, as many as it
/// states it draws.
pub const FLOATING: usize = tomori_protocol::Limits::MOST.floating as usize;

/// The most environments of one space whose walls and panels are kept; going to another forgets those of the
/// environment left longest ago.
pub const ROOMS: usize = 8;

/// The most slots the walls shown in one environment of a space have together: ten times as many as the Tomori wall and
/// six environment walls and six walls added had before frames were negotiated, so that a projection at every limit fits
/// in one of [`tomori_protocol::MOST_FRAME_BYTES`] however its walls are laid out. A change to a wall, or a new one,
/// that would take them past it is refused. [`tomori_protocol::Limits::MOST`]'s, which this server's `Welcome` states;
/// a client is sent walls only while their slots together are within what it states it draws.
pub const SLOTS: usize = tomori_protocol::Limits::MOST.slots as usize;

const _: () =
    assert!(1 + DECLARED_WALLS + ADDED_WALLS == tomori_protocol::Limits::MOST.walls as usize);
const _: () = assert!(SLOTS + FLOATING == tomori_protocol::Limits::MOST.panels as usize);

/// The most panels kept for a space, shown or hidden. A space shows at most [`SLOTS`] on its walls and [`FLOATING`]
/// floating, with one more floating for each slot its walls have fewer than [`SLOTS`], fewer than this, so that beyond
/// it the hidden panel put where it is longest ago is forgotten, and what a space keeps stays bounded however many
/// environments it has been in.
pub const PANELS: usize = 1200;

/// The widest and tallest a wall may be, in metres: a cinema's screen, as every client bounds it.
pub const WALL_SIZE: (f32, f32) = layout::MOST_SIZE;

/// The longest radius a wall is curved at, in metres; a longer one is all but flat.
pub const LONGEST_RADIUS: f32 = 50.0;

/// The most columns of the Tomori wall, in its one row, so that it stays within the 40° either side of straight ahead
/// of home that the workers' stations leave clear.
pub const TOMORI_COLUMNS: u32 = 6;

/// How far from the space's origin, on each axis, a wall or a floating panel may be, in metres.
pub const REACH: f32 = 30.0;

/// The version of the kept format, written with each space's stage.
pub const SCHEMA: u32 = 1;

/// How far ahead of the Tomori wall the point is that its row of slots curves about, in metres: from where it stands by
/// default, home, so that every slot faces a participant standing there.
pub const ARC_RADIUS: f32 = 2.3;

/// Where the Tomori wall stands in an environment that declares neither a `stage` nor an `agent` anchor: beneath the
/// swarm at its default home, as `tomori_protocol::entourage::wall_beneath` stands it, the middle of its curved row of
/// slots straight ahead of home, beneath the swarm's middle, low enough that the swarm and every worker station are seen
/// over it, upright and facing home.
pub fn default_stage() -> Pose {
    Pose::upright_facing([0.0, 0.52, -ARC_RADIUS], [0.0, 0.52, 0.0])
}

/// How a wall's slots are laid out, as the stage keeps it: its grid, how large its slots are, as a proportion of a flat
/// wall's, the size of their text, their shape, as their width over their height, if it is not a standard slot's, and
/// the radius of the arc it is curved along, if it is curved, how far it tilts from upright, in degrees, if it does,
/// and how large its panels' content is within its slots, if it is not as large as they give it. A stage kept before
/// walls tilted reads as upright, and one kept before content was scaled as drawn as it was.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub grid: Grid,
    pub scale: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_scale: Option<f32>,
}

impl Layout {
    /// The Tomori wall's unless it is changed: six in a curved row, about home.
    pub const TOMORI: Layout = Layout {
        grid: layout::ARC_GRID,
        scale: layout::ARC_SCALE,
        aspect: None,
        radius: Some(ARC_RADIUS),
        tilt: None,
        content_scale: None,
    };

    /// Every other wall's unless it is changed: three columns of two rows, flat.
    pub const FLAT: Layout = Layout {
        grid: layout::FLAT_GRID,
        scale: 1.0,
        aspect: None,
        radius: None,
        tilt: None,
        content_scale: None,
    };

    /// `wall`, laid out so, saying its grid and scale only where they are not its shape's, as an older client reads it,
    /// its slots' aspect only where it is not a standard slot's, its tilt only where it tilts, and its content scale
    /// only where it is not 1.
    pub(crate) fn apply(&self, wall: Wall) -> Wall {
        let shape = self
            .radius
            .map_or(WallShape::Grid, |radius| WallShape::Arc { radius });
        let wall = wall
            .with_shape(shape)
            .with_aspect(self.aspect)
            .with_tilt(self.tilt)
            .with_content_scale(self.content_scale);
        if wall.grid() == self.grid && wall.scale() == self.scale {
            wall
        } else {
            wall.with_grid(self.grid).with_scale(self.scale)
        }
    }
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has built what a connection is sent of a space, [`Stage::view_within`], so that
    /// tests see how many builds a change costs.
    pub(crate) static VIEWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one more view built, for the tests.
#[cfg(test)]
fn count_view() {
    VIEWS.with(|views| views.set(views.get() + 1));
}

/// What a participant or the agent changes of a wall: each left out stays as it is. `radius` is `Some(None)` to flatten
/// it, `aspect` `Some(None)` to give it standard slots, `tilt`, in degrees, `Some(None)` to stand it upright, and
/// `content_scale` `Some(None)` to draw its content as large as its slots give it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WallChange {
    pub grid: Option<Grid>,
    pub scale: Option<f32>,
    pub aspect: Option<Option<f32>>,
    pub radius: Option<Option<f32>>,
    pub pose: Option<Pose>,
    pub tilt: Option<Option<f32>>,
    pub content_scale: Option<Option<f32>>,
}

impl WallChange {
    /// The change `SetWall` asks for, or why it cannot be one: a shape a server does not know is refused, an `aspect`
    /// of 0 is a standard slot's, a `tilt` of 0 upright, and a `content_scale` of 1 as large as its slots give it.
    pub fn new(
        grid: Option<Grid>,
        scale: Option<f32>,
        aspect: Option<f32>,
        shape: Option<WallShape>,
        pose: Option<Pose>,
        tilt: Option<f32>,
        content_scale: Option<f32>,
    ) -> Result<Self, &'static str> {
        let radius = match shape {
            None => None,
            Some(WallShape::Grid) => Some(None),
            Some(WallShape::Arc { radius }) => Some(Some(radius)),
            Some(_) => return Err("Tomori does not know that shape of wall."),
        };
        Ok(Self {
            grid,
            scale,
            aspect: aspect.map(|aspect| (aspect != 0.0).then_some(aspect)),
            radius,
            pose,
            tilt: tilt.map(|tilt| (tilt != 0.0).then_some(tilt)),
            content_scale: content_scale.map(|scale| (scale != 1.0).then_some(scale)),
        })
    }
}

/// Where the swarm's middle stands in an environment that gives no `agent` anchor, facing home: `tomori-scene`'s
/// `swarm::CENTER`. See `design/environments.md`.
pub const AGENT: [f32; 3] = [0.0, 1.4, -ARC_RADIUS];

/// How far from an environment's origin, on each axis, its arrival and the anchors and walls it gives in its own
/// coordinates may stand, in metres, before they are carried into a space's, where they are bounded by [`REACH`].
pub const ARRIVAL_REACH: f32 = 1000.0;

/// What a panel shows: its title, on one line and possibly empty, its body, and its metadata, in order; for a code
/// panel, its language, length and spans, its body the first lines of its code; and for an image panel, its image, its
/// body the image's caption. Made only by [`Content::parse`], for a text panel, `code::Lines::content`, for a code
/// panel, and [`Content::image`], for an image panel, which bound and clean it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Content {
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meta: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    /// The agent's own id for a text panel, a short slug such as `agenda`, by which a later block with the same id, or
    /// the `update_panel` tool, updates it in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<tomori_protocol::Image>,
    /// For a volume, its diagram, as laid out; its body is the diagram's outline. See `design/diagrams.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram: Option<Box<tomori_protocol::diagram::Diagram>>,
    /// What reading a volume's block found wrong, which the agent is told as it is projected, and which is never kept.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// A panel found in a reply, as its block is read: its content, and, for a code panel, the code to highlight, which is
/// highlighted later, outside the hub's lock, since highlighting takes far longer than reading. Until then a code
/// panel is plain.
#[derive(Debug)]
pub struct Found {
    pub content: Content,
    source: Option<code::Source>,
    /// Where its block says it goes, if anywhere the server knows.
    pub place: Option<Placement>,
}

impl Found {
    /// The text panel `content`.
    pub fn text(content: Content) -> Self {
        Self {
            content,
            source: None,
            place: None,
        }
    }

    /// The code panel `content`, plain, whose code is `source`.
    fn code(content: Content, source: code::Source) -> Self {
        Self {
            content,
            source: Some(source),
            place: None,
        }
    }

    /// It, going where `place` says.
    fn placed(mut self, place: Option<Placement>) -> Self {
        self.place = place;
        self
    }
}

/// The panels of the replies that one event completed, in order, each with where its block says it goes, and each
/// reply's with where whoever asked for it stood and the way they faced, as the hub projects them.
pub type Replies = Vec<(Vec<(Content, Option<Placement>)>, Option<Pose>)>;

/// Where the agent's panel goes, as the hub decides from where it says and where people are. See
/// `design/projection.md`.
#[derive(Clone, Debug, PartialEq)]
pub enum Destination {
    /// Onto the wall nearest this point.
    Nearest([f32; 3]),
    /// Onto the wall with this identity, in this slot if it is free, or else in the wall's most prominent free slot, or
    /// else as a full wall gives one up. A wall not shown is the Tomori wall.
    Wall { wall: String, slot: Option<u32> },
    /// Floating at this pose.
    Free(Pose),
    /// Floating beside someone whose head is at `head`, clear of the environment's `solids`, its walls and what floats
    /// there already, as [`beside::spot`] places it, or else in front of them.
    Beside {
        head: Pose,
        solids: Vec<tomori_environment::sightlines::Solid>,
    },
    /// Onto the Tomori wall, as when nobody asked.
    Tomori,
    /// Floating at `pose`, its slot of the cluster at the cluster anchor `anchor`, by name, of which it stays a member
    /// while it floats there. See `design/projection.md`.
    Anchor { anchor: String, pose: Pose },
}

/// The panels of the replies that one event completed, waiting for their code to be highlighted before they are
/// projected, which the hub does outside its lock. See `design/projection.md`.
#[derive(Debug)]
pub struct Projecting {
    replies: Replies,
    /// The code panels to highlight, by reply and by panel, and their code: only the last reply's with any, since a
    /// reply's code panels replace the code panels of the replies before it as each is projected, and these are
    /// projected together, so that no earlier reply's would ever be seen. Those stay plain.
    code: Vec<(usize, usize, code::Source)>,
}

impl Projecting {
    /// The panels of `replies`, each with where whoever asked for it stood, in order.
    pub fn new(replies: Vec<(Vec<Found>, Option<Pose>)>) -> Self {
        let last = replies
            .iter()
            .rposition(|(panels, _)| panels.iter().any(|found| found.source.is_some()));
        let mut code = Vec::new();
        let replies = replies
            .into_iter()
            .enumerate()
            .map(|(reply, (panels, origin))| {
                let contents = panels
                    .into_iter()
                    .enumerate()
                    .map(|(panel, found)| {
                        if let Some(source) = found.source.filter(|_| Some(reply) == last) {
                            code.push((reply, panel, source));
                        }
                        (found.content, found.place)
                    })
                    .collect();
                (contents, origin)
            })
            .collect();
        Self { replies, code }
    }

    /// The code to highlight, in order.
    pub fn sources(&self) -> Vec<code::Source> {
        self.code
            .iter()
            .map(|(_, _, source)| source.clone())
            .collect()
    }

    /// The replies' panels, the code of each in [`Self::sources`] given the next of `spans`, which a highlighter found
    /// over its text: those past the last of `spans` stay plain.
    pub fn highlighted(mut self, spans: Vec<Vec<Span>>) -> Replies {
        for ((reply, panel, source), found) in self.code.into_iter().zip(spans) {
            let content = &mut self.replies[reply].0[panel].0;
            content.code = content
                .code
                .take()
                .map(|code| code.with_spans(source.shown(found)));
        }
        self.replies
    }

    /// The replies' panels, every code panel plain.
    pub fn plain(self) -> Replies {
        self.highlighted(Vec::new())
    }
}

/// Where the walls of a space stand in its current environment: the environment, the home there of the space's own
/// status wall, the Tomori wall, the walls the environment declares, as its registration gives them, carried into the
/// space's coordinates, and which is the status wall.
#[derive(Clone, Copy, Debug)]
pub struct Walls<'a> {
    pub environment: &'a str,
    pub tomori: Pose,
    pub declared: &'a [crate::EnvironmentWall],
    pub status: Status,
}

/// Which wall is the space's status wall in an environment, where Tomori's notices, what the agent projects unasked and
/// every placement's last fallback go. See `design/projection.md`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    /// The space's own, the Tomori wall, at its home beneath the agent's, laid out within its own limits.
    Beneath,
    /// The space's own, at the home its environment gives it, laid out by default as this says.
    At(Layout),
    /// The environment's own wall, by its place among those it declares: the Tomori wall is not shown.
    Own(usize),
}

impl Walls<'_> {
    /// The identity of the status wall shown: the Tomori wall, or the environment's own wall in the role.
    pub(crate) fn status_wall(&self) -> String {
        match self.status {
            Status::Own(index) if index < self.declared.len().min(DECLARED_WALLS) => {
                format!("environment-{index}")
            }
            _ => TOMORI_WALL.to_owned(),
        }
    }

    /// Whether the status wall is one of the environment's own, whose slots its panels and the space's status panels
    /// share.
    pub(crate) fn shared(&self) -> bool {
        self.status_wall() != TOMORI_WALL
    }
}
/// What is refused of a locked wall, by participants and the agent alike.
pub const LOCKED: &str = "That wall is locked: ask an owner to unlock it.";

/// What is refused of a change that would move or reshape a locked wall of the group of the wall changed.
pub const GROUP_LOCKED: &str = "A wall of its group is locked, and would move with it: ask an owner to unlock it, or take this wall out of the group first.";

/// What is refused of a content scale a wall cannot have.
pub const CONTENT_SCALE: &str = "A wall's text is drawn from half to twice the size its slots give it, and never smaller than half a standard slot's: give it larger slots to make its text smaller still.";

/// Where a participant puts a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Into the layout of the wall with this identity, in its most prominent free slot, as an older client asks.
    Wall(String),
    /// Into this slot of the wall with this identity, which the participant chose.
    Slot(String, u32),
    /// Floating free at this pose.
    Free(Pose),
}

/// Where a [`Destination`] puts a panel in the environment the space is in: onto a wall shown there, in a slot if that
/// is free, or floating.
enum Spot {
    Wall(String, Option<u32>),
    /// Onto the status wall, as a status panel, in this slot if it is free.
    Status(Option<u32>),
    Free(Pose),
}

/// Where a kept panel is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Placed {
    Wall { wall: String, slot: u32 },
    Free { pose: Pose },
}

/// A panel as the stage keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Kept {
    id: String,
    #[serde(flatten)]
    content: Content,
    /// When it was projected, in seconds since the epoch.
    created: u64,
    /// When the agent last updated it in place, if it has, in seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated: Option<u64>,
    /// When it was last put where it is, in the stage's own order, so that the one there longest leaves first.
    placed: u64,
    place: Placed,
    /// Whether a participant has put it where it is, so that the agent's next code panels do not replace it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pinned: bool,
    /// A participant put it on its wall, so that it leaves a full wall only after those the agent put there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    arranged: bool,
    /// What makes it one its environment published, if it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    published: Option<published::Published>,
    /// The cluster anchor it was put at, by name, and where it was put: a member of that anchor's cluster for as long
    /// as it floats there, so that moving it anywhere else, as a participant carrying it does, takes it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor: Option<(String, Pose)>,
    /// How large it floats, as a proportion of its own size, as a participant stretched or shrank it with both hands,
    /// within [`tomori_protocol::FLOAT_SIZES`]: kept only while it floats, since a wall's slot sizes the panels in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<f32>,
    /// Its own context's settings, for a panel its environment published, each fixing a parameter it depends on over
    /// its wall's. See `design/panel-context.md`.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "context::read"
    )]
    context: context::Settings,
}

impl Kept {
    /// The anchor it stands at, by name, while it floats where it was put there.
    fn anchored(&self) -> Option<&str> {
        match (&self.anchor, &self.place) {
            (Some((anchor, at)), Placed::Free { pose }) if at == pose => Some(anchor),
            _ => None,
        }
    }

    /// Whether it is on the wall `wall`, and in which slot.
    fn slot_on(&self, wall: &str) -> Option<u32> {
        match &self.place {
            Placed::Wall { wall: on, slot } if on == wall => Some(*slot),
            _ => None,
        }
    }

    /// It, as participants are sent it at `now`.
    fn panel(&self, now: u64) -> Panel {
        let place = match &self.place {
            Placed::Wall { wall, slot } => Place::Wall {
                wall: wall.clone(),
                slot: *slot,
            },
            Placed::Free { pose } => Place::Free { pose: *pose },
        };
        let size = self
            .size
            .filter(|_| matches!(self.place, Placed::Free { .. }));
        let mut panel = Panel::new(
            self.id.clone(),
            self.content.title.clone(),
            self.content.body.clone(),
            place,
        )
        .with_meta(
            self.content
                .meta
                .iter()
                .map(|(key, value)| Meta::new(key.clone(), value.clone()))
                .collect(),
        )
        .with_age(now.saturating_sub(self.created))
        .with_pinned(self.pinned)
        .with_slug(self.content.slug.clone())
        .with_size(size);
        panel.code = self.content.code.clone();
        panel.image = self.content.image.clone();
        panel.diagram = self.content.diagram.clone();
        panel.chart = self
            .published
            .as_ref()
            .and_then(|published| published.chart.clone())
            .map(Box::new);
        // A list's rows, by which a client names the row it points at and the one it chooses, and draws each as a row:
        // the one highlighted on its bar, in its tone, with its value.
        if let Some(published) = &self.published
            && let Some(rows) = &published.rows
        {
            let highlighted = published.highlighted.as_deref();
            panel = panel.with_rows(
                rows.iter()
                    .map(|row| {
                        let mut sent = tomori_protocol::ListRow::new(row.id.clone(), row.choosable)
                            .with_highlight(highlighted == Some(row.id.as_str()))
                            .with_tone(row.tone)
                            .with_cells(row.cells.clone());
                        // The revision of its key, never the key, which stays with the server.
                        sent.key = row.key;
                        sent.value = row.value;
                        sent
                    })
                    .collect(),
            );
            panel = panel.with_columns(published.columns.clone());
        }
        panel
    }

    /// Whether the agent's next code panels replace it: a code panel no participant has put where it is, which is on the
    /// wall the agent put it on, since only a participant leaves one floating.
    fn replaced(&self) -> bool {
        self.content.code.is_some() && !self.pinned
    }
}

/// A wall an owner added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AddedWall {
    id: String,
    pose: Pose,
    /// How it is laid out, when changed from a flat wall's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<Layout>,
    /// Where it is attached in its group, if it is in one and not the group's first: its pose then follows from the
    /// wall it is attached to, and is kept as it stands, so that a server that knows no groups reads it where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attach: Option<tomori_protocol::Attach>,
    /// Whether an owner locked it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    locked: bool,
}

/// What a space keeps for one environment: the walls owners added and the panels on the environment's walls or
/// floating in it. Written as [`RoomRecord`] is, what the environment published as where it rests.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct Room {
    /// When the space was last in it, in the stage's own order, so that the room left longest ago is forgotten first.
    #[serde(default)]
    used: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    walls: Vec<AddedWall>,
    /// How the environment's own walls are laid out in this space, by identity, where changed from how the environment
    /// lays them out.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    layouts: BTreeMap<String, Layout>,
    /// Where the environment's own walls stand in this space, by identity, where moved from where the environment puts
    /// them, once unlocked.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    moved: BTreeMap<String, Pose>,
    /// The environment's own walls an owner unlocked in this space, by identity; the rest are locked.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    unlocked: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    panels: Vec<Kept>,
    /// The panels its environment published that its side's current run has not published again, kept only where they
    /// are.
    #[serde(default)]
    resting: Vec<published::Resting>,
    /// The ids of the panels its environment published that a participant dismissed, which stay away until the
    /// environment retracts them, the space comes back to the environment or the server restarts.
    #[serde(skip)]
    dismissed: BTreeSet<String>,
    /// Each wall's own context's settings, by the wall's identity, which the panels its environment published on it
    /// inherit where they do not fix them themselves. See `design/panel-context.md`.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "context::read_walls"
    )]
    contexts: BTreeMap<String, context::Settings>,
    /// The parameters its environment's side declares in its run now, which belong to the run and are never written.
    #[serde(skip)]
    parameters: Vec<context::Parameter>,
}

/// A room as the space writes it: its panels but those its environment published, which are written where they are,
/// as they rest, without what they show, which no longer holds once their run has ended.
#[derive(Serialize)]
struct RoomRecord<'a> {
    #[serde(default)]
    used: u64,
    #[serde(skip_serializing_if = "<[AddedWall]>::is_empty")]
    walls: &'a [AddedWall],
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    layouts: &'a BTreeMap<String, Layout>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    moved: &'a BTreeMap<String, Pose>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    unlocked: &'a BTreeSet<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    panels: Vec<&'a Kept>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    resting: Vec<published::Resting>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    contexts: &'a BTreeMap<String, context::Settings>,
}

impl Serialize for Room {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // As they would rest were the run to end now, held to as many as may rest.
        // Every field named, so that one added to a room is a choice to write or not, never left out unawares.
        let Room {
            used,
            walls,
            layouts,
            moved,
            unlocked,
            panels,
            resting,
            dismissed: _,
            contexts,
            parameters: _,
        } = self;
        let live: Vec<published::Resting> = panels
            .iter()
            .filter_map(|kept| {
                kept.published
                    .as_ref()
                    .map(|published| published::Resting::of(kept, &published.id))
            })
            .collect();
        let resting = published::rested(live, resting);
        RoomRecord {
            used: *used,
            walls,
            layouts,
            moved,
            unlocked,
            panels: panels
                .iter()
                .filter(|kept| kept.published.is_none())
                .collect(),
            resting,
            contexts,
        }
        .serialize(serializer)
    }
}

/// One space's walls and panels, kept with the space across restarts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    #[serde(default = "schema")]
    version: u32,
    /// The last number given, to panels, walls and the stage's own order.
    #[serde(default)]
    next: u64,
    /// The space's status panels, on the Tomori wall, or on an environment's own wall in the role, or waiting for a slot
    /// on the status wall where they are on neither.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tomori: Vec<Kept>,
    /// What is kept for each environment, by its identifier.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    rooms: BTreeMap<String, Room>,
    /// The latest of each kind of Tomori's own notices, by kind, as [`Stage::project_notice`] projected it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    notices: BTreeMap<String, String>,
    /// How the Tomori wall is laid out, in every environment, when changed from [`Layout::TOMORI`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tomori_layout: Option<Layout>,
    /// Whether an owner locked the Tomori wall, in every environment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tomori_locked: bool,
}

fn schema() -> u32 {
    SCHEMA
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            version: SCHEMA,
            next: 0,
            tomori: Vec::new(),
            rooms: BTreeMap::new(),
            notices: BTreeMap::new(),
            tomori_layout: None,
            tomori_locked: false,
        }
    }
}

/// `pose`, with its orientation made a unit quaternion and upright, turned only about the vertical, as walls and panels
/// stand, if it is finite, within [`REACH`] and has an orientation. One tilted is turned to face the way its +Z faces,
/// levelled, or, where that is straight up or down, the way its +X faces turned a quarter towards it.
pub(crate) fn valid(pose: &Pose) -> Option<Pose> {
    valid_within(pose, REACH)
}

/// `pose`, as [`valid`] makes it, but within [`ARRIVAL_REACH`]: an environment's arrival, or an anchor or wall in its
/// own coordinates before it is carried into a space's.
pub(crate) fn valid_arrival(pose: &Pose) -> Option<Pose> {
    valid_within(pose, ARRIVAL_REACH)
}

fn valid_within(pose: &Pose, reach: f32) -> Option<Pose> {
    let length = pose
        .orientation
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !(pose.position.iter().all(|value| value.abs() <= reach)
        && length > 0.5
        && length.is_finite())
    {
        return None;
    }
    let orientation = pose.orientation.map(|value| value / length);
    let [x, y, z, w] = orientation;
    // Turned only about the vertical already, it is kept as it is.
    if x.abs() < 1e-6 && z.abs() < 1e-6 {
        return Some(Pose {
            position: pose.position,
            orientation,
        });
    }
    let (mut ahead_x, mut ahead_z) = (2.0 * (x * z + w * y), 1.0 - 2.0 * (x * x + y * y));
    if ahead_x.hypot(ahead_z) < 1e-3 {
        (ahead_x, ahead_z) = (2.0 * (w * y - x * z), 1.0 - 2.0 * (y * y + z * z));
    }
    let [px, py, pz] = pose.position;
    Some(Pose::upright_facing(
        pose.position,
        [px + ahead_x, py, pz + ahead_z],
    ))
}

/// The most charts linked to one pointed at that its description reads, as a participant's client draws their readouts
/// at the same time.
pub const LINKED_READINGS: usize = 3;

/// The longest reading of a chart linked to one pointed at, its title and what it reads, in characters.
const LINKED_READING_CHARS: usize = 120;

/// What the charts shown in `shown` that share the time of `pointed` read at `time`, as its readout is linked to theirs,
/// by their time axes, never their labels: live charts on its wall, by how long before now, where it is live, or fixed
/// windows anywhere, by the instant, where it is one, that cover the time,
/// at most [`LINKED_READINGS`], each its title and reading, in quotes, cut to [`LINKED_READING_CHARS`]. See the linked
/// time cursor in `design/projection.md`.
fn linked_readings(
    shown: &tomori_protocol::Projection,
    pointed: &tomori_protocol::Panel,
    time: Option<tomori_protocol::chart::ChartTime>,
) -> Vec<String> {
    let Some(time) = time else {
        return Vec::new();
    };
    let wall = |panel: &tomori_protocol::Panel| match &panel.place {
        Place::Wall { wall, .. } => Some(wall.clone()),
        _ => None,
    };
    let here = wall(pointed);
    shown
        .panels
        .iter()
        .filter(|panel| {
            panel.id != pointed.id
                && (matches!(time, tomori_protocol::chart::ChartTime::Instant { .. })
                    || (here.is_some() && wall(panel) == here))
        })
        .filter_map(|panel| {
            let chart = panel.chart.as_ref()?;
            let at = chart.at_time(time)?;
            Some(format!(
                "\"{}\"",
                crate::space::one_line(
                    &format!("{}: {}", panel.title, chart.read_at(at)),
                    LINKED_READING_CHARS
                )
            ))
        })
        .take(LINKED_READINGS)
        .collect()
}

/// How long ago `seconds` was, as the agent is told.
fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => "just now".to_owned(),
        60..120 => "a minute ago".to_owned(),
        120..3_600 => format!("{} minutes ago", seconds / 60),
        3_600..7_200 => "an hour ago".to_owned(),
        7_200..172_800 => format!("{} hours ago", seconds / 3_600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

impl Stage {
    /// Whether it holds nothing, so that a space keeps nothing for it.
    pub fn is_empty(&self) -> bool {
        self.tomori.is_empty()
            && self.rooms.is_empty()
            && self.tomori_layout.is_none()
            && !self.tomori_locked
    }

    /// The next number in the stage's order.
    fn tick(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// The walls panels can be put on in `walls`' environment: the Tomori wall, then the environment's own, then those
    /// its owners added.
    fn walls(&self, walls: &Walls<'_>) -> Vec<Wall> {
        let room = self.rooms.get(walls.environment);
        let status = walls.status_wall();
        let mut shown = Vec::new();
        if !walls.shared() {
            shown.push(
                self.tomori_layout(walls)
                    .apply(Wall::new(TOMORI_WALL, WallKind::Tomori, walls.tomori))
                    .with_locked(self.tomori_locked)
                    .with_status(true),
            );
        }
        shown.extend(walls.declared.iter().take(DECLARED_WALLS).enumerate().map(
            |(index, declared)| {
                let id = format!("environment-{index}");
                let layout = room
                    .and_then(|room| room.layouts.get(&id))
                    .copied()
                    .unwrap_or_else(|| declared.layout());
                let pose = room
                    .and_then(|room| room.moved.get(&id))
                    .copied()
                    .unwrap_or(declared.pose);
                let locked = !room.is_some_and(|room| room.unlocked.contains(&id));
                let role = id == status;
                layout
                    .apply(Wall::new(id, WallKind::Environment, pose))
                    .with_name(declared.name().map(str::to_owned))
                    .with_locked(locked)
                    .with_status(role)
            },
        ));
        if let Some(room) = room {
            let mut joins = room.joins();
            shown.extend(room.walls.iter().map(|wall| {
                let (group, joined) = joins.remove(&wall.id).unzip();
                wall.layout
                    .unwrap_or(Layout::FLAT)
                    .apply(Wall::new(wall.id.clone(), WallKind::Added, wall.pose))
                    .with_group(group, joined.unwrap_or_default())
                    .with_locked(wall.locked)
            }));
        }
        shown
    }

    /// The wall `wall` shown in `walls`' environment, if it is.
    fn shown_wall(&self, walls: &Walls<'_>, wall: &str) -> Option<Wall> {
        self.walls(walls).into_iter().find(|shown| shown.id == wall)
    }

    /// How many slots the wall `wall` shown in `walls`' environment has, or none when it is not shown.
    fn slots_of(&self, walls: &Walls<'_>, wall: &str) -> u32 {
        self.shown_wall(walls, wall)
            .map_or(0, |shown| shown.slots())
    }

    /// How many panels float in `environment`.
    fn floating(&self, environment: &str) -> usize {
        self.rooms.get(environment).map_or(0, |room| {
            room.panels
                .iter()
                .filter(|kept| matches!(kept.place, Placed::Free { .. }))
                .count()
        })
    }

    /// How many panels may float in an environment whose walls have `slots` slots together, counting those its walls
    /// gave up: [`FLOATING`], and one more for each slot fewer than [`SLOTS`], so that a wall giving up slots never takes
    /// a panel away, while the walls' slots and the panels floating stay within [`SLOTS`] and [`FLOATING`] together. A
    /// wall is given more slots only while it holds.
    fn floating_room(slots: usize) -> usize {
        FLOATING + SLOTS.saturating_sub(slots)
    }

    /// How many slots the walls shown in `walls`' environment have together.
    fn slots_shown(&self, walls: &Walls<'_>) -> usize {
        self.walls(walls)
            .iter()
            .map(|wall| wall.slots() as usize)
            .sum()
    }

    /// The room of `environment`, made if the space has none for it yet, forgetting the room left longest ago when
    /// [`ROOMS`] are kept; marked as the one the space is in now.
    fn room(&mut self, environment: &str) -> &mut Room {
        let used = self.tick();
        if !self.rooms.contains_key(environment)
            && self.rooms.len() >= ROOMS
            && let Some(oldest) = self
                .rooms
                .iter()
                .min_by_key(|(_, room)| room.used)
                .map(|(id, _)| id.clone())
        {
            self.rooms.remove(&oldest);
        }
        let room = self.rooms.entry(environment.to_owned()).or_default();
        room.used = used;
        room
    }

    /// The space leaves `environment` for another: what it keeps for it, if anything, is the room left most recently, and
    /// so the last forgotten.
    pub(crate) fn leave(&mut self, environment: &str) {
        // Its environment's run ends: what it published rests, and what a participant dismissed may come back.
        self.rest(environment);
        if let Some(room) = self.rooms.get_mut(environment) {
            room.dismissed.clear();
        }
        let used = self.next + 1;
        if let Some(room) = self.rooms.get_mut(environment) {
            room.used = used;
            self.next = used;
        }
    }

    /// Forget the hidden panels, those of environments other than `environment`, put where they are longest ago, while
    /// more than [`PANELS`] are kept.
    fn bound(&mut self, environment: &str) {
        let kept = self.tomori.len()
            + self
                .rooms
                .values()
                .map(|room| room.panels.len())
                .sum::<usize>();
        let mut hidden: Vec<(u64, &str)> = self
            .rooms
            .iter()
            .filter(|(id, _)| id.as_str() != environment)
            .flat_map(|(_, room)| {
                room.panels
                    .iter()
                    .map(|kept| (kept.placed, kept.id.as_str()))
            })
            .collect();
        hidden.sort_unstable();
        let forgotten: Vec<String> = hidden
            .into_iter()
            .take(kept.saturating_sub(PANELS))
            .map(|(_, id)| id.to_owned())
            .collect();
        for room in self.rooms.values_mut() {
            room.panels.retain(|kept| !forgotten.contains(&kept.id));
        }
    }

    /// The panels that the wall `wall` holds, or that float, in `environment`: the Tomori wall's own, or the room's.
    fn list(&mut self, environment: &str, wall: Option<&str>) -> &mut Vec<Kept> {
        match wall {
            Some(TOMORI_WALL) => &mut self.tomori,
            _ => &mut self.room(environment).panels,
        }
    }

    /// The panels kept in `environment`: those on the Tomori wall, then those on the environment's walls or floating in
    /// it.
    fn here<'a>(&'a self, environment: &str) -> impl Iterator<Item = &'a Kept> {
        let room = self
            .rooms
            .get(environment)
            .map(|room| room.panels.as_slice())
            .unwrap_or_default();
        self.tomori.iter().chain(room)
    }

    /// When the agent last updated the panel `panel` in `environment` in place, if it has, in seconds since the epoch.
    pub(crate) fn updated(&self, environment: &str, panel: &str) -> Option<u64> {
        self.kept(environment, panel).and_then(|kept| kept.updated)
    }

    /// The panel `panel` as it is kept in `environment`.
    fn kept(&self, environment: &str, panel: &str) -> Option<&Kept> {
        self.here(environment).find(|kept| kept.id == panel)
    }

    /// The panel `panel` as it is kept in `environment`, to change.
    fn here_mut(&mut self, environment: &str, panel: &str) -> Option<&mut Kept> {
        self.tomori
            .iter_mut()
            .chain(
                self.rooms
                    .get_mut(environment)
                    .map(|room| room.panels.iter_mut())
                    .into_iter()
                    .flatten(),
            )
            .find(|kept| kept.id == panel)
    }

    /// Take the panel `panel` out of where it is in `environment`, if it is there.
    fn take(&mut self, environment: &str, panel: &str) -> Option<Kept> {
        if let Some(index) = self.tomori.iter().position(|kept| kept.id == panel) {
            return Some(self.tomori.remove(index));
        }
        let room = self.rooms.get_mut(environment)?;
        let index = room.panels.iter().position(|kept| kept.id == panel)?;
        Some(room.panels.remove(index))
    }

    /// Whether every slot of the wall `wall`, of `slots` slots, in `environment` holds a panel its environment
    /// published, so that it takes no other: an environment's panel is never given up for a newcomer.
    fn crowded(&self, environment: &str, wall: &str, slots: u32) -> bool {
        let published = self
            .here(environment)
            .filter(|kept| kept.published.is_some() && kept.slot_on(wall).is_some());
        published.count() as u32 >= slots
    }

    /// Put `kept` on the wall `wall` in `walls`' environment, `arranged` by a participant or not, as a status panel or
    /// not, in its most prominent free slot, or else in the slot of the panel that has been on it longest of those not
    /// arranged, or, when all are, of them all, which leaves, but never of a panel its environment published, which
    /// stays; on an environment's own wall in the status role, as [`Stage::make_room`] makes room, or else floating
    /// before its right end. The wall is not [`Stage::crowded`], which the caller sees to.
    fn put_on(
        &mut self,
        walls: &Walls<'_>,
        mut kept: Kept,
        wall: &str,
        arranged: bool,
        status: bool,
    ) {
        let environment = walls.environment;
        kept.placed = self.tick();
        kept.arranged = arranged;
        let slot = match self.free_in(walls, wall) {
            Some(slot) => slot,
            None if walls.shared() && wall == walls.status_wall() => {
                match self.make_room(walls, status) {
                    Some(slot) => slot,
                    None => {
                        let pose = self.before_end(walls, wall);
                        return self.float(walls, kept, pose);
                    }
                }
            }
            None => {
                let list = self.list(environment, Some(wall));
                let (index, slot) = list
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| other.published.is_none())
                    .filter_map(|(index, other)| {
                        other
                            .slot_on(wall)
                            .map(|slot| (index, (other.arranged, other.placed), slot))
                    })
                    .min_by_key(|(_, order, _)| *order)
                    .map(|(index, _, slot)| (index, slot))
                    .expect("a full wall that is not crowded holds a panel to give up");
                list.remove(index);
                slot
            }
        };
        kept.place = Placed::Wall {
            wall: wall.to_owned(),
            slot,
        };
        self.list_for(walls, Some(wall), status).push(kept);
        self.settle(walls);
    }

    /// Put `kept`, which came from `from`, in the slot `slot` of the wall `wall` in `walls`' environment, arranged, as a
    /// status panel or not. A panel already there changes places with it, keeping its kind: into the slot it came from on
    /// the same wall, or else into the wall's most prominent free slot, or else where it came from, on another wall or
    /// floating.
    fn put_in(
        &mut self,
        walls: &Walls<'_>,
        mut kept: Kept,
        from: Placed,
        wall: &str,
        slot: u32,
        status: bool,
    ) {
        let environment = walls.environment;
        kept.placed = self.tick();
        kept.arranged = true;
        kept.place = Placed::Wall {
            wall: wall.to_owned(),
            slot,
        };
        let placed = self.tick();
        // The panel there, a status panel or not.
        let shared = wall == walls.status_wall();
        let status_occupant = self
            .tomori
            .iter()
            .position(|other| shared && other.slot_on(wall) == Some(slot));
        let (occupant, was_status) = match status_occupant {
            Some(index) => (Some(self.tomori.remove(index)), true),
            None => {
                let list = self.list(environment, Some(wall));
                let occupant = list
                    .iter()
                    .position(|other| other.slot_on(wall) == Some(slot))
                    .map(|index| list.remove(index));
                (occupant, wall == TOMORI_WALL)
            }
        };
        self.list_for(walls, Some(wall), status).push(kept);
        let free = self.free_in(walls, wall);
        let Some(mut occupant) = occupant else {
            return;
        };
        occupant.placed = placed;
        occupant.place = match (&from, free) {
            (Placed::Wall { wall: was, slot }, _) if was == wall => Placed::Wall {
                wall: wall.to_owned(),
                slot: *slot,
            },
            (_, Some(free)) => Placed::Wall {
                wall: wall.to_owned(),
                slot: free,
            },
            (from, None) => from.clone(),
        };
        let list = match &occupant.place {
            Placed::Wall { wall: to, .. } if to == wall => {
                self.list_for(walls, Some(to), was_status)
            }
            Placed::Wall { wall: to, .. } => {
                let to = to.clone();
                self.list(environment, Some(&to))
            }
            Placed::Free { .. } => self.list(environment, None),
        };
        list.push(occupant);
    }

    /// What participants see of the space `space` in `walls`' environment at `now`, in seconds since the epoch: every
    /// wall there, and the panels on them or floating there. Panels on a wall the environment no longer declares are
    /// kept, but not shown.
    pub(crate) fn view(&self, space: &str, walls: &Walls<'_>, now: u64) -> Projection {
        self.view_within(space, walls, now, &tomori_protocol::Limits::MOST)
    }

    /// What one connection is sent of the space `space` in `walls`' environment at `now`, within what it takes,
    /// `limits`: the walls [`walls_within`] keeps and the panels on them, the panels floating there that were left most
    /// recently, as many as its floating panels and one more for each slot its walls have fewer than its slots, and of
    /// those, the panels put where they are most recently, as many as its panels, in a frame of its frame limit. What a
    /// connection is not sent is kept, and shown to those that take more. See `design/projection.md`.
    pub(crate) fn view_within(
        &self,
        space: &str,
        walls: &Walls<'_>,
        now: u64,
        limits: &tomori_protocol::Limits,
    ) -> Projection {
        #[cfg(test)]
        count_view();
        let shown = walls_within(self.walls(walls), limits);
        let room = self
            .rooms
            .get(walls.environment)
            .map(|room| room.panels.as_slice())
            .unwrap_or_default();
        let status = walls.status_wall();
        let seats = self.slots_of(walls, &status);
        let kept: Vec<&Kept> = self
            .tomori
            .iter()
            .filter(|kept| kept.slot_on(&status).is_some_and(|slot| slot < seats))
            .chain(room)
            .filter(|kept| match &kept.place {
                Placed::Wall { wall, .. } => shown.iter().any(|shown| shown.id == *wall),
                Placed::Free { .. } => true,
            })
            .collect();
        // The panels put where they are longest ago are left out first, keeping the order of those sent.
        let slots: usize = shown.iter().map(|wall| wall.slots() as usize).sum();
        let room = (limits.floating as usize)
            .saturating_add((limits.slots as usize).saturating_sub(slots));
        let mut newest: Vec<usize> = (0..kept.len()).collect();
        newest.sort_by_key(|index| std::cmp::Reverse(kept[*index].placed));
        let mut sent = vec![false; kept.len()];
        let (mut floating, mut panels) = (0, 0);
        for index in newest {
            if panels == limits.panels as usize {
                break;
            }
            if matches!(kept[index].place, Placed::Free { .. }) {
                if floating == room {
                    continue;
                }
                floating += 1;
            }
            sent[index] = true;
            panels += 1;
        }
        let kept: Vec<&Kept> = kept
            .into_iter()
            .zip(sent)
            .filter_map(|(kept, sent)| sent.then_some(kept))
            .collect();
        let placed: Vec<u64> = kept.iter().map(|kept| kept.placed).collect();
        let panels = kept
            .iter()
            .map(|kept| {
                self.held(walls.environment, kept, kept.panel(now))
                    .with_context(self.panel_chips(walls.environment, kept))
            })
            .collect();
        let shown = shown
            .into_iter()
            .map(|wall| {
                let chips = self.wall_chips(walls.environment, &wall.id);
                wall.with_context(chips)
            })
            .collect();
        let mut projection = Projection::new(space, walls.environment)
            .with_walls(shown)
            .with_panels(panels);
        // Fitted before its walls and panels are given their versions, and its live panels their data seqs, each a
        // number of at most 20 digits with its key, 32 bytes: room is left for them, and for the message around it.
        let live = projection
            .panels
            .iter()
            .filter(|panel| panel.chart.is_some())
            .count();
        let versions = 32 * (projection.walls.len() + projection.panels.len() + live);
        let budget = limits.frame_limit() - MESSAGE_BYTES;
        fit(&mut projection, &placed, budget.saturating_sub(versions));
        projection
    }

    /// Project `content` at `now` onto the wall nearest `origin`, where the participant who asked for it stood, of the
    /// Tomori wall and `walls`' environment's walls, or onto the Tomori wall when nobody asked. Returns its identity.
    #[cfg(test)]
    pub(crate) fn project(
        &mut self,
        walls: &Walls<'_>,
        content: Content,
        origin: Option<[f32; 3]>,
        now: u64,
    ) -> String {
        let to = origin.map_or(Destination::Tomori, Destination::Nearest);
        self.project_to(walls, content, &to, now)
    }

    /// Where `to` puts a panel in `walls`' environment: onto a wall shown there, in a slot if that is free, or floating
    /// at a pose. A wall not shown there, and a pose that is not one or is beyond reach, are the Tomori wall.
    fn resolve(&self, walls: &Walls<'_>, to: &Destination) -> Spot {
        let shown = self.walls(walls);
        let tomori = Spot::Status(None);
        match to {
            Destination::Nearest(origin) => crate::presence::nearest_wall(&shown, *origin).map_or(
                tomori,
                |(wall, _)| match wall.id.as_str() {
                    TOMORI_WALL => Spot::Status(None),
                    _ => Spot::Wall(wall.id.clone(), None),
                },
            ),
            // The Tomori wall names the status role, wherever the status wall stands.
            Destination::Wall { wall, slot } if wall == TOMORI_WALL => {
                let slots = self.slots_of(walls, &walls.status_wall());
                Spot::Status(slot.filter(|slot| *slot < slots))
            }
            Destination::Wall { wall, slot } if shown.iter().any(|shown| shown.id == *wall) => {
                let slots = self.slots_of(walls, wall);
                Spot::Wall(wall.clone(), slot.filter(|slot| *slot < slots))
            }
            Destination::Free(pose) | Destination::Anchor { pose, .. } => {
                valid(pose).map_or(tomori, Spot::Free)
            }
            Destination::Beside { head, solids } => {
                let floating: Vec<Pose> = self
                    .rooms
                    .get(walls.environment)
                    .into_iter()
                    .flat_map(|room| &room.panels)
                    .filter_map(|kept| match &kept.place {
                        Placed::Free { pose } => Some(*pose),
                        Placed::Wall { .. } => None,
                    })
                    .collect();
                let pose = beside::spot(head, &shown, &floating, solids)
                    .unwrap_or_else(|| crate::presence::front_of(head));
                valid(&pose).map_or(tomori, Spot::Free)
            }
            Destination::Wall { .. } | Destination::Tomori => tomori,
        }
    }

    /// Project `content` at `now` where `to` says in `walls`' environment. Returns its identity.
    pub(crate) fn project_to(
        &mut self,
        walls: &Walls<'_>,
        content: Content,
        to: &Destination,
        now: u64,
    ) -> String {
        let id = format!("panel-{}", self.tick());
        let kept = Kept {
            id: id.clone(),
            content,
            created: now,
            updated: None,
            placed: 0,
            place: Placed::Free {
                pose: Pose::default(),
            },
            pinned: false,
            arranged: false,
            published: None,
            anchor: None,
            size: None,
            context: context::Settings::new(),
        };
        self.put(walls, kept, to);
        self.bound(walls.environment);
        id
    }

    /// The panels floating in `environment` at a cluster anchor, each by identity with the anchor's name and where it
    /// floats, in the order they were put there.
    pub(crate) fn anchored(&self, environment: &str) -> Vec<(String, String, Pose)> {
        let mut anchored: Vec<(u64, String, String, Pose)> = self
            .here(environment)
            .filter_map(|kept| {
                let (_, pose) = kept.anchor.as_ref()?;
                kept.anchored()
                    .map(|anchor| (kept.placed, kept.id.clone(), anchor.to_owned(), *pose))
            })
            .collect();
        anchored.sort_by_key(|(placed, ..)| *placed);
        anchored
            .into_iter()
            .map(|(_, id, anchor, pose)| (id, anchor, pose))
            .collect()
    }

    /// Take the panels floating in `environment` at the cluster anchor `anchor` out of its cluster, leaving them where they
    /// float, as the anchor goes. Returns how many there were.
    pub(crate) fn unanchor(&mut self, environment: &str, anchor: &str) -> usize {
        let mut count = 0;
        for kept in self
            .rooms
            .get_mut(environment)
            .into_iter()
            .flat_map(|room| room.panels.iter_mut())
        {
            if kept.anchored() == Some(anchor) {
                kept.anchor = None;
                count += 1;
            }
        }
        count
    }

    /// Project `content`, one of Tomori's own notices of `kind`, such as that workers were removed, at `now` onto the
    /// status wall, as [`Stage::project_to`] does when nobody asked. The previous notice of the same kind is taken off
    /// first, unless a participant has put it where it is, so that repeated notices take one slot rather than push the
    /// agent's panels off the wall. Returns its identity.
    pub(crate) fn project_notice(
        &mut self,
        walls: &Walls<'_>,
        kind: &str,
        content: Content,
        now: u64,
    ) -> String {
        if let Some(previous) = self.notices.remove(kind) {
            let unmoved = |kept: &Kept| kept.id == previous && !kept.pinned;
            self.tomori.retain(|kept| !unmoved(kept));
            for room in self.rooms.values_mut() {
                room.panels.retain(|kept| !unmoved(kept));
            }
        }
        let id = self.project_to(walls, content, &Destination::Tomori, now);
        self.notices.insert(kind.to_owned(), id.clone());
        id
    }

    /// Put the agent's panel `kept` where `to` says in `walls`' environment, not arranged: at a cluster anchor, a member
    /// of its cluster while it floats where it is put.
    fn put(&mut self, walls: &Walls<'_>, mut kept: Kept, to: &Destination) {
        kept.anchor = None;
        match self.resolve(walls, to) {
            Spot::Wall(wall, slot) => self.put_at(walls, kept, &wall, slot, false),
            Spot::Status(slot) => self.put_at(walls, kept, &walls.status_wall(), slot, true),
            Spot::Free(pose) => {
                // At the pose it floats at, as the stage keeps it.
                if let Destination::Anchor { anchor, .. } = to {
                    kept.anchor = Some((anchor.clone(), pose));
                }
                self.float(walls, kept, pose)
            }
        }
    }

    /// Put the agent's panel `kept` on the wall `wall` in `walls`' environment, as a status panel or not, in `slot` if
    /// it is free, and otherwise as [`Stage::put_on`] puts it; or, where every slot of the wall holds a panel its
    /// environment published, onto the status wall instead, as when nobody asked, since an environment's panel is never
    /// given up for it.
    fn put_at(
        &mut self,
        walls: &Walls<'_>,
        mut kept: Kept,
        wall: &str,
        slot: Option<u32>,
        status: bool,
    ) {
        // An environment's own panel takes the slot it names on its own wall in the status role from a status panel.
        if kept.published.is_some()
            && let Some(slot) = slot.filter(|slot| *slot < self.slots_of(walls, wall))
        {
            self.give_way(walls, wall, slot);
        }
        let taken = self.taken(walls, wall);
        let Some(slot) = slot.filter(|slot| !taken.contains(slot)) else {
            if self.crowded(walls.environment, wall, self.slots_of(walls, wall)) {
                return self.put_on(walls, kept, &walls.status_wall(), false, true);
            }
            return self.put_on(walls, kept, wall, false, status);
        };
        kept.placed = self.tick();
        kept.arranged = false;
        kept.place = Placed::Wall {
            wall: wall.to_owned(),
            slot,
        };
        self.list_for(walls, Some(wall), status).push(kept);
        self.settle(walls);
    }

    /// Leave `kept` floating at `pose` in `walls`' environment, not arranged; more than [`FLOATING`] floating, the one
    /// floating longest goes. Panels a wall gave up may float beyond it, as [`Stage::floating_room`] allows, and are the
    /// newest to float, so the last to go.
    fn float(&mut self, walls: &Walls<'_>, mut kept: Kept, pose: Pose) {
        let room = FLOATING;
        // Off a wall, it floats at its own size, as its slot no longer sizes it.
        if !matches!(kept.place, Placed::Free { .. }) {
            kept.size = None;
        }
        kept.place = Placed::Free { pose };
        kept.arranged = false;
        kept.placed = self.tick();
        let list = self.list(walls.environment, None);
        list.push(kept);
        let floating = list
            .iter()
            .filter(|kept| matches!(kept.place, Placed::Free { .. }))
            .count();
        if floating > room
            && let Some(index) = list
                .iter()
                .enumerate()
                .filter(|(_, kept)| matches!(kept.place, Placed::Free { .. }))
                .min_by_key(|(_, kept)| kept.placed)
                .map(|(index, _)| index)
            && let Some(published) = list.remove(index).published
        {
            // One its environment published stays away, as one a full wall gives up does.
            self.room(walls.environment).dismissed.insert(published.id);
        }
    }

    /// Project the panels of one reply, `contents`, at `now`, onto the wall nearest `origin`, as [`Stage::project`]
    /// projects each, replacing the code panels of the replies before as [`Stage::project_placed`] does.
    #[cfg(test)]
    pub(crate) fn project_reply(
        &mut self,
        walls: &Walls<'_>,
        contents: Vec<Content>,
        origin: Option<[f32; 3]>,
        now: u64,
    ) -> Vec<String> {
        let to = origin.map_or(Destination::Tomori, Destination::Nearest);
        let contents = contents
            .into_iter()
            .map(|content| (content, to.clone(), false))
            .collect();
        self.project_placed(walls, contents, now)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Project the panels of one reply, `contents`, at `now`, each where its destination says, and whether its block
    /// said where it goes. If the reply has code panels, they replace the code panels of the replies before, on any wall,
    /// but those a participant has pulled off or put where they are: those are taken off first. A text panel with the id
    /// of one shown in `walls`' environment updates that one in place, as [`Stage::revise`] does, moving it only if its
    /// block said where it goes; one with the id of a panel kept hidden with another environment takes that one's place,
    /// projected here as new, so that an id names one panel in a space. Returns the panels' identities, and whether each
    /// was updated in place.
    pub(crate) fn project_placed(
        &mut self,
        walls: &Walls<'_>,
        contents: Vec<(Content, Destination, bool)>,
        now: u64,
    ) -> Vec<(String, bool)> {
        if contents.iter().any(|(content, ..)| content.code.is_some()) {
            self.tomori.retain(|kept| !kept.replaced());
            for room in self.rooms.values_mut() {
                room.panels.retain(|kept| !kept.replaced());
            }
        }
        contents
            .into_iter()
            .map(|(content, to, said)| {
                let shown = content
                    .slug
                    .as_deref()
                    .filter(|_| content.code.is_none())
                    .and_then(|slug| self.named(walls.environment, slug));
                match shown {
                    Some(id) => {
                        let to = said.then_some(&to);
                        self.revise(walls, &id, content, to, now)
                            .expect("a panel shown is kept");
                        (id, true)
                    }
                    None => {
                        if let Some(slug) = &content.slug {
                            self.forget_named(slug);
                        }
                        (self.project_to(walls, content, &to, now), false)
                    }
                }
            })
            .collect()
    }

    /// The identity of the text panel the agent calls `slug` in `environment`: on the Tomori wall, or on the
    /// environment's walls or floating in it.
    pub(crate) fn named(&self, environment: &str, slug: &str) -> Option<String> {
        self.here(environment)
            .find(|kept| kept.content.code.is_none() && kept.content.slug.as_deref() == Some(slug))
            .map(|kept| kept.id.clone())
    }

    /// Forget every panel the agent calls `slug`, as one projected anew with that id takes their place.
    fn forget_named(&mut self, slug: &str) {
        let named = |kept: &Kept| kept.content.slug.as_deref() == Some(slug);
        self.tomori.retain(|kept| !named(kept));
        for room in self.rooms.values_mut() {
            room.panels.retain(|kept| !named(kept));
        }
    }

    /// What the text panel `panel` in `walls`' environment shows, if it is shown there and is not a code panel.
    pub(crate) fn content(&self, walls: &Walls<'_>, panel: &str) -> Option<&Content> {
        self.kept(walls.environment, panel)
            .filter(|kept| kept.published.is_none())
            .map(|kept| &kept.content)
            .filter(|content| content.code.is_none())
    }

    /// Update the text panel `panel` in `walls`' environment in place at `now`, to show `content`, keeping its identity,
    /// its id, its wall and slot or where it floats, whether a participant put it there,
    /// and its place in its wall's order of who leaves first, so that it is not a new panel for placement or for making
    /// room. With `to`, it then moves there, as [`Stage::relocate`] moves it, unless a participant put it where it
    /// is: only `move_panel` moves a panel a participant placed. Returns whether it changed, or why it was refused.
    pub(crate) fn revise(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        mut content: Content,
        to: Option<&Destination>,
        now: u64,
    ) -> Result<bool, &'static str> {
        let kept = self
            .here_mut(walls.environment, panel)
            .ok_or("That panel is not here.")?;
        if kept.published.is_some() {
            return Err(ENVIRONMENT_S);
        }
        if kept.content.code.is_some() || content.code.is_some() {
            return Err("A code panel cannot be updated in place.");
        }
        content.slug = kept.content.slug.clone();
        let pinned = kept.pinned;
        let changed = kept.content != content;
        if changed {
            kept.content = content;
            kept.updated = Some(now);
        }
        let moved = match to {
            Some(to) if !pinned => self.relocate(walls, panel, to).unwrap_or(false),
            _ => false,
        };
        Ok(changed || moved)
    }

    /// Tick, or untick, as `checked` says, the checklist item that begins on `line` of the text panel `panel`'s body in
    /// `walls`' environment, as a participant asked by pressing A while pointing at it: in place, keeping everything
    /// else about the panel, when the agent last updated it included, as [`checklist::set`] ticks it. Returns the item,
    /// or `None` where it was already so, or why it was refused: a panel not here, one with no body of items, such as a
    /// code or image panel, and a line that begins no item with a box. Whoever calls it has checked the version the
    /// participant saw, so that a tick never lands on another item of a body changed since. See `design/projection.md`.
    pub(crate) fn tick_item(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        line: u32,
        checked: bool,
    ) -> Result<Option<Ticked>, &'static str> {
        let kept = self
            .here_mut(walls.environment, panel)
            .ok_or("That panel is not here.")?;
        if kept.published.is_some() {
            return Err(ENVIRONMENT_S);
        }
        if kept.content.code.is_some()
            || kept.content.image.is_some()
            || kept.content.diagram.is_some()
        {
            return Err("That panel has no checklist.");
        }
        let content = &mut kept.content;
        let items = checklist::items(&content.body);
        let (index, item) = items
            .iter()
            .enumerate()
            .find(|(_, item)| u64::try_from(item.line).ok() == Some(u64::from(line)))
            .filter(|(_, item)| item.checked.is_some())
            .ok_or("That is not an item of the panel's checklist.")?;
        if item.checked == Some(checked) {
            return Ok(None);
        }
        let ticked = Ticked {
            number: index + 1,
            count: items.len(),
            text: item.text.to_owned(),
            checked,
            title: content.title.clone(),
            start: content.body.lines().next().unwrap_or_default().to_owned(),
            slug: content.slug.clone(),
        };
        content.body =
            checklist::set(&content.body, item.line, checked).expect("an item is on its line");
        Ok(Some(ticked))
    }

    /// Move the panel `panel` where `to` says in `walls`' environment, as its agent asked. One a participant put where it
    /// was stays theirs: still pinned, and arranged on a wall, so that the agent's next panels and code panels leave it
    /// as they leave what participants placed. Returns whether that moved it, or why it was refused.
    pub(crate) fn relocate(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        to: &Destination,
    ) -> Result<bool, &'static str> {
        let shown = self.view("", walls, 0);
        let Some(current) = shown.panel(panel) else {
            return Err("That panel is not here.");
        };
        let status = walls.status_wall();
        let unchanged = match (self.resolve(walls, to), &current.place) {
            (Spot::Wall(wall, slot), Place::Wall { wall: on, slot: at }) => {
                wall == *on && slot.is_none_or(|slot| slot == *at)
            }
            (Spot::Status(slot), Place::Wall { wall: on, slot: at }) => {
                status == *on && slot.is_none_or(|slot| slot == *at)
            }
            _ => false,
        };
        if unchanged {
            return Ok(false);
        }
        if self.leaves_environment(
            walls,
            panel,
            matches!(self.resolve(walls, to), Spot::Status(_)),
        ) {
            return Err(STAYS);
        }
        let kept = self
            .take(walls.environment, panel)
            .expect("a panel shown is kept");
        let pinned = kept.pinned;
        self.put(walls, kept, to);
        if pinned
            && let Some(kept) = self
                .tomori
                .iter_mut()
                .chain(
                    self.rooms
                        .values_mut()
                        .flat_map(|room| room.panels.iter_mut()),
                )
                .find(|kept| kept.id == panel)
        {
            kept.arranged = matches!(kept.place, Placed::Wall { .. });
        }
        Ok(true)
    }

    /// The environments the space keeps walls or panels for.
    pub(crate) fn environments(&self) -> Vec<String> {
        self.rooms.keys().cloned().collect()
    }

    /// Put the panel `panel` where `target` says in `walls`' environment. Returns whether that changed anything, or why it
    /// was refused.
    #[cfg(test)]
    pub(crate) fn place(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        target: Target,
    ) -> Result<bool, &'static str> {
        self.place_sized(walls, panel, target, None)
    }

    /// Put the panel `panel` where `target` says in `walls`' environment, as [`Stage::place`] does, and, left floating,
    /// `size` times as large as its own size, within [`tomori_protocol::FLOAT_SIZES`], as a participant stretched or
    /// shrank it with both hands: without one, it keeps the size it floated at. Returns whether that changed anything,
    /// or why it was refused.
    pub(crate) fn place_sized(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        target: Target,
        size: Option<f32>,
    ) -> Result<bool, &'static str> {
        let size = match size {
            Some(size) if !size.is_finite() => return Err("A panel's size must be a number."),
            Some(size) => Some(size.clamp(
                tomori_protocol::FLOAT_SIZES.0,
                tomori_protocol::FLOAT_SIZES.1,
            )),
            None => None,
        };
        let shown = |wall: &str| self.walls(walls).iter().any(|shown| shown.id == wall);
        let slots = |wall: &str| self.slots_of(walls, wall);
        let target = match target {
            Target::Wall(wall) if shown(&wall) => Target::Wall(wall),
            Target::Slot(wall, slot) if shown(&wall) && slot < slots(&wall) => {
                Target::Slot(wall, slot)
            }
            Target::Slot(wall, _) if shown(&wall) => return Err("That wall has no such slot."),
            Target::Wall(_) | Target::Slot(..) => return Err("That wall is not here."),
            Target::Free(pose) => Target::Free(
                valid(&pose).ok_or("A panel must float within 30 m of the space's middle.")?,
            ),
        };
        let onto_tomori =
            matches!(&target, Target::Wall(wall) | Target::Slot(wall, _) if wall == TOMORI_WALL);
        if self.leaves_environment(walls, panel, onto_tomori) {
            return Err(STAYS);
        }
        // A wall whose every slot holds its environment's own panels gives none of them up for another put on it,
        // unless it is on it already.
        if let Target::Wall(wall) = &target
            && self.crowded(walls.environment, wall, slots(wall))
            && self
                .kept(walls.environment, panel)
                .is_some_and(|kept| kept.slot_on(wall).is_none())
        {
            return Err(CROWDED);
        }
        // Into a slot, a panel changes places with the one there, which, on a wall with no free slot, goes where it
        // came from: not onto the Tomori wall, for a panel its environment published, which stays in its environment.
        if let Target::Slot(wall, slot) = &target
            && self
                .kept(walls.environment, panel)
                .is_some_and(|kept| kept.slot_on(TOMORI_WALL).is_some())
        {
            let there: Vec<&Kept> = self
                .here(walls.environment)
                .filter(|kept| kept.slot_on(wall).is_some())
                .collect();
            let held = there
                .iter()
                .any(|kept| kept.published.is_some() && kept.slot_on(wall) == Some(*slot));
            if held && there.len() as u32 >= slots(wall) {
                return Err(STAYS);
            }
        }
        let was_status = self.tomori.iter().any(|kept| kept.id == panel);
        let mut kept = self
            .take(walls.environment, panel)
            .ok_or("That panel is not here.")?;
        // Onto the Tomori wall, it is a status panel; moved between the status wall's slots, it keeps its kind; put on
        // an environment's own wall in the role by hand, it is the environment's.
        let status = match &target {
            Target::Wall(wall) | Target::Slot(wall, _) => {
                wall == TOMORI_WALL || (was_status && kept.slot_on(wall).is_some())
            }
            Target::Free(_) => false,
        };
        // Put somewhere by a participant, it is pinned there, and the agent's next code panels do not replace it.
        let pinned = !kept.pinned;
        kept.pinned = true;
        match target {
            // Put back on the wall it is on, or in the slot it is in, it keeps its slot, now arranged.
            Target::Wall(wall) if kept.slot_on(&wall).is_some() => {
                let changed = !std::mem::replace(&mut kept.arranged, true);
                self.list_for(walls, Some(&wall), status).push(kept);
                Ok(pinned || changed)
            }
            Target::Slot(wall, slot) if kept.slot_on(&wall) == Some(slot) => {
                let changed = !std::mem::replace(&mut kept.arranged, true);
                self.list_for(walls, Some(&wall), status).push(kept);
                Ok(pinned || changed)
            }
            Target::Wall(wall) => {
                self.put_on(walls, kept, &wall, true, status);
                Ok(true)
            }
            Target::Slot(wall, slot) => {
                let from = kept.place.clone();
                self.put_in(walls, kept, from, &wall, slot, status);
                self.settle(walls);
                Ok(true)
            }
            Target::Free(pose) => {
                let was = kept
                    .size
                    .filter(|_| matches!(kept.place, Placed::Free { .. }));
                let changed = pinned || kept.place != Placed::Free { pose };
                self.float(walls, kept, pose);
                // Sized after it floats, which leaves one floated off a wall its own size; its own size is kept as
                // none, as a panel nobody stretched has.
                let mut resized = false;
                if let Some(size) = size
                    && let Some(kept) = self.here_mut(walls.environment, panel)
                {
                    let size = Some(size).filter(|size| (size - 1.0).abs() > 1e-4);
                    resized = was != size;
                    kept.size = size;
                }
                self.settle(walls);
                Ok(changed || resized)
            }
        }
    }

    /// Dismiss the panel `panel` in `environment`, or say why not.
    /// One its environment published stays away until the environment retracts it, or the space comes back to the
    /// environment.
    pub(crate) fn dismiss(&mut self, walls: &Walls<'_>, panel: &str) -> Result<(), &'static str> {
        let environment = walls.environment;
        let kept = self
            .take(environment, panel)
            .ok_or("That panel is not here.")?;
        if let Some(published) = kept.published {
            self.room(environment).dismissed.insert(published.id);
        }
        self.settle(walls);
        Ok(())
    }

    /// Whether the panel `panel` in `walls`' environment is one its environment published, which `onto_tomori` would
    /// take onto the Tomori wall, and so out of the environment, as nothing of an environment's may go.
    fn leaves_environment(&self, walls: &Walls<'_>, panel: &str, onto_tomori: bool) -> bool {
        onto_tomori
            && self
                .kept(walls.environment, panel)
                .is_some_and(|kept| kept.published.is_some())
    }

    /// The panel `panel`, as the agent is told about one a participant points at in `walls`' environment at `now`, such
    /// as `panel "Build status" (on the status wall, projected 2 minutes ago; source: "CI")`, with its id, and, pointed
    /// at on the list item that begins on `line` of its body, which item, such as `panel "Agenda" (id agenda, on the
    /// Tomori wall, projected 5 minutes ago, updated just now; item 3 of 5, not ticked: "Decide the pairing window")`:
    /// everything in it cleaned onto one line, without double quotes or square brackets, so that it cannot end the
    /// context block. A line that begins no item is left out. `None` when it is not shown there.
    #[cfg(test)]
    pub(crate) fn describe(
        &self,
        walls: &Walls<'_>,
        panel: &str,
        line: Option<u32>,
        now: u64,
    ) -> Option<String> {
        self.describe_at(
            walls,
            panel,
            Pointed {
                line,
                ..Pointed::default()
            },
            now,
        )
    }

    /// As [`Self::describe`] says, of the panel pointed at on its row `row`, by the row's id, where the client named one:
    /// that row wherever it now is on the list, or, where it has gone, saying so, rather than the row on `line` now,
    /// so that the agent is told of the row the participant saw. See `design/panel-actions.md`.
    ///
    /// On a chart panel pointed at `across` its time axis, it says the time there and each series' value, read from the
    /// stage's own copy of the chart, as `; at the time pointed at: "-6 min: admitted 203,800 a minute"`, in quotes, as it
    /// is the environment's text. See `design/projection.md`.
    pub(crate) fn describe_at(
        &self,
        walls: &Walls<'_>,
        panel: &str,
        at: Pointed<'_>,
        now: u64,
    ) -> Option<String> {
        let Pointed {
            line,
            row,
            across,
            node,
            edge,
            world_edge,
            chip,
        } = at;
        let shown = self.view("", walls, now);
        let panel = shown.panel(panel)?;
        let updated = self
            .kept(walls.environment, &panel.id)
            .and_then(|kept| kept.updated);
        let kind = if panel.code.is_some() {
            "code panel"
        } else if panel.image.is_some() {
            "image panel"
        } else if panel.chart.is_some() {
            "chart panel"
        } else if panel.diagram.is_some() {
            "diagram"
        } else {
            "panel"
        };
        // On a volume, the node or edge pointed at, in the diagram it is in.
        let part = panel
            .diagram
            .as_deref()
            .and_then(|diagram| volume::part(diagram, node, edge, world_edge));
        let named = if panel.title.is_empty() {
            let start = panel.body.lines().next().unwrap_or_default();
            format!(
                "{kind} beginning \"{}\"",
                crate::space::one_line(start, blocks::META_VALUE_CHARS)
            )
        } else {
            format!(
                "{kind} \"{}\"",
                crate::space::one_line(&panel.title, blocks::TITLE_CHARS)
            )
        };
        let place = match &panel.place {
            Place::Wall { wall, .. } if *wall == walls.status_wall() => "on the status wall",
            Place::Wall { .. } => "on a wall",
            _ => "floating",
        };
        let mut details = String::new();
        if let Some(slug) = &panel.slug {
            // A slug holds nothing to clean, but its letters, digits, `-` and `_`.
            details.push_str(&format!("id {slug}, "));
        }
        let published = self.published(walls.environment, &panel.id);
        let verb = if published.is_some() {
            "published"
        } else {
            "projected"
        };
        details.push_str(&format!("{place}, {verb} {}", ago(panel.age)));
        if let Some(updated) = updated {
            details.push_str(&format!(", updated {}", ago(now.saturating_sub(updated))));
        }
        // An environment's panel says whose it is, and, where it has one, the id the agent may inspect it by; a chart
        // what it shows now, in quotes, as it is the environment's text.
        if let Some((_, reference)) = published {
            details.push_str(&format!(
                "; the {} environment's",
                crate::space::one_line(walls.environment, 64)
            ));
            if let Some(reference) = reference {
                details.push_str(&format!(
                    ", id {}",
                    crate::space::inspectable(walls.environment, reference)
                ));
            }
        }
        if panel.chart.is_some() {
            details.push_str(&format!(
                "; shows: \"{}\"",
                crate::space::one_line(&panel.body.replace('\n', ", "), blocks::BODY_CHARS)
            ));
        }
        // The chip of its context pointed at, what it shows and where that comes from, in quotes, as the environment's
        // text.
        if let Some(chip) =
            chip.and_then(|chip| panel.context.iter().find(|shown| shown.parameter == chip))
        {
            use tomori_protocol::context::ContextSetting;
            let value = crate::hub::agent_context::spoken(&chip.value);
            let from = match chip.setting {
                ContextSetting::Fixed => "fixed by its own setting",
                ContextSetting::Inherited => "inherited from its wall",
                _ => "following the environment",
            };
            details.push_str(&format!(
                "; pointing at its chip: \"{}\", {from}",
                crate::space::one_line(
                    &format!("{}: {value}", chip.label),
                    blocks::META_VALUE_CHARS
                )
            ));
        }
        if let (Some(chart), Some(across)) = (&panel.chart, across) {
            details.push_str(&format!(
                "; at the time pointed at: \"{}\"",
                crate::space::one_line(
                    &chart.read_at(chart.at_across(across)),
                    tomori_protocol::chart::READING_CHARS
                )
            ));
            let linked = linked_readings(&shown, panel, chart.time_of(chart.at_across(across)));
            if !linked.is_empty() {
                details.push_str(&format!(
                    "; linked charts at that time: {}",
                    linked.join(", ")
                ));
            }
        }
        // An image panel's size, and its caption in quotes, as it is the agent's text.
        if let Some(image) = &panel.image {
            details.push_str(&format!("; {} × {} pixels", image.width, image.height));
            if !panel.body.is_empty() {
                details.push_str(&format!(
                    "; caption: \"{}\"",
                    crate::space::one_line(&panel.body, blocks::META_VALUE_CHARS)
                ));
            }
        }
        // A code panel's language, in quotes, as it is from the agent's text, and the lines it shows, of how many.
        if let Some(code) = &panel.code {
            if !code.language.is_empty() {
                details.push_str(&format!(
                    "; language: \"{}\"",
                    crate::space::one_line(&code.language, blocks::META_KEY_CHARS)
                ));
            }
            let held = panel.body.split('\n').count();
            details.push_str(&if code.lines as usize > held {
                format!("; lines 1–{held} of {}", code.lines)
            } else {
                format!("; {} lines", code.lines)
            });
        }
        // Each value in quotes, as the title is, since it is the agent's text, and may be from what it read.
        for meta in &panel.meta {
            details.push_str(&format!(
                "; {}: \"{}\"",
                crate::space::one_line(&meta.key, blocks::META_KEY_CHARS),
                crate::space::one_line(&meta.value, blocks::META_VALUE_CHARS)
            ));
        }
        // The row pointed at of an environment's list, each row a line of its body, from the server's own copy: its
        // number, whether it is highlighted, its tone and its value, each in the server's own words, its id to inspect
        // it by and its text, or a table's row its cells, each after its column's title, in quotes, as it is the
        // environment's.
        let items = checklist::items(&panel.body);
        if let Some((rows, highlighted)) = self.rows(walls.environment, &panel.id) {
            let found = match row {
                Some(id) => rows.iter().enumerate().find(|(_, row)| row.id == id),
                None => line
                    .and_then(|line| usize::try_from(line).ok())
                    .and_then(|line| rows.get(line).map(|row| (line, row))),
            };
            if row.is_some() && found.is_none() {
                details.push_str("; a row since gone from the list");
            }
            if let Some((index, row)) = found {
                let highlighted = if highlighted == Some(row.id.as_str()) {
                    ", highlighted"
                } else {
                    ""
                };
                let id = row.reference.as_deref().map_or(String::new(), |reference| {
                    format!(
                        ", id {}",
                        crate::space::inspectable(walls.environment, reference)
                    )
                });
                let tone = row
                    .tone_word()
                    .map_or(String::new(), |tone| format!(", tone {tone}"));
                let value = row.percent().map_or(String::new(), |percent| {
                    format!(", at {percent}% of its limit")
                });
                // Its key, as data, each value cleaned as all text others choose is, so that none ends the context block,
                // quoted as JSON quotes it, so that none ends its quotation, and cut to a few words, so that a row's key
                // adds at most about 700 characters to what is said.
                let key = if row.fields.is_empty() {
                    String::new()
                } else {
                    let fields: Vec<String> = row
                        .fields
                        .iter()
                        .map(|(name, value)| {
                            let value = crate::space::one_line(value, KEY_TOLD_CHARS);
                            format!("{name}: {}", serde_json::Value::String(value))
                        })
                        .collect();
                    format!(", key {{{}}}", fields.join(", "))
                };
                // A table's row as its cells, each after its column's title, `: ` and ` · ` between them, within their
                // bounds, so that nothing is cut.
                let columns = self.columns(walls.environment, &panel.id);
                let most = if row.cells.is_empty() {
                    blocks::META_VALUE_CHARS
                } else {
                    tomori_environment::ROW_CHARS
                        + tomori_environment::COLUMN_LIMIT
                            * (tomori_environment::COLUMN_TITLE_CHARS + 5)
                };
                details.push_str(&format!(
                    "; row {} of {}{highlighted}{tone}{value}{id}{key}: \"{}\"",
                    index + 1,
                    rows.len(),
                    crate::space::one_line(&row.told(columns), most)
                ));
            }
        } else if panel.code.is_none()
            && let Some(line) = line
            && let Some((index, item)) = items
                .iter()
                .enumerate()
                .find(|(_, item)| item.line as u64 == u64::from(line))
        {
            let state = match item.checked {
                Some(true) => ", ticked",
                Some(false) => ", not ticked",
                None => "",
            };
            details.push_str(&format!(
                "; item {} of {}{state}: \"{}\"",
                index + 1,
                items.len(),
                crate::space::one_line(item.text, blocks::META_VALUE_CHARS)
            ));
        }
        match part {
            Some((part, about)) => Some(format!("{part} in {named} ({details}; {about})")),
            None => Some(format!("{named} ({details})")),
        }
    }
}

/// Where on a panel a participant pointed, as their client named it: the line of its body a list item begins on, a
/// list's row by its id, where across a chart's time axis, and a volume's node by its id, or its edge or world edge by
/// its index.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Pointed<'a> {
    pub line: Option<u32>,
    pub row: Option<&'a str>,
    pub across: Option<u16>,
    pub node: Option<&'a str>,
    pub edge: Option<u16>,
    /// A volume's world edge, by its index.
    pub world_edge: Option<u16>,
    /// The chip of its context pointed at, by its parameter. See `design/panel-context.md`.
    pub chip: Option<&'a str>,
}

/// Why a participant's tick is refused when the panel changed since the version they saw: the client cues it, and the
/// projection sent again shows the panel as it is, so that they look and tick again.
pub(crate) const STALE_TICK: &str = "That panel changed as you ticked it; look again.";

/// The most characters of each value of a row's key a description of the row pointed at tells, followed by `…` where
/// it is cut: the agent reads it whole with `panels` or `inspect`.
const KEY_TOLD_CHARS: usize = 48;

/// Why a participant's choice of a row is refused when the row's key changed since the one they saw, so that its
/// environment is never handed another key than the one shown: the client cues it, and they look and choose again.
pub(crate) const STALE_ROW: &str = "That row changed as you chose it; look again.";

/// Why a size given with both hands was refused, the panel having changed since they took hold of it.
pub(crate) const STALE_SIZE: &str = "That panel changed as you resized it; take hold of it again.";

/// Why a freeze is refused where the panel changed since the participant saw it.
pub(crate) const STALE_FREEZE: &str = "That panel changed as you froze it; look again.";

/// Why a participant's change to a wall is refused when the wall changed since the version they saw, as when another
/// participant changed it meanwhile: the client cues it, and the projection sent again shows the wall as it is, so that
/// they change it again from there rather than undo the other change.
pub(crate) const STALE_WALL: &str = "That wall changed as you changed it; look again.";

/// Why the agent and participants cannot change what an environment's panel shows.
const ENVIRONMENT_S: &str = "That panel is its environment's: only the environment updates it.";

/// Why an environment's panel cannot go onto the Tomori wall.
const STAYS: &str = "That panel is its environment's, and stays on its walls or floating in it.";

/// Why no panel is put onto a wall whose every slot holds a panel its environment published.
const CROWDED: &str = "That wall is full of its environment's own panels, which stay on it: put it on another wall, or leave it floating.";

/// A checklist item a participant ticked or unticked, as [`Stage::tick_item`] found it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ticked {
    /// Its number, from 1, of how many items its panel has, as `update_panel` numbers them.
    pub number: usize,
    pub count: usize,
    /// Its text, as the agent wrote it.
    pub text: String,
    /// Whether it is now ticked.
    pub checked: bool,
    /// Its panel's title, the first line of its body, for a panel without one, and the id the agent gave it.
    title: String,
    start: String,
    slug: Option<String>,
}

impl Ticked {
    /// What the agent is told of it, before the next words spoken in the space, as `who` ticked it, such as `Ada ticked
    /// item 2 of 5 of panel "Agenda" (id agenda): "Panel ids"`: everything in it cleaned onto one line, without double
    /// quotes or square brackets, as the context block's descriptions are, so that it cannot end its block.
    pub(crate) fn note(&self, who: &str) -> String {
        let verb = if self.checked { "ticked" } else { "unticked" };
        let named = if self.title.is_empty() {
            format!(
                "panel beginning \"{}\"",
                crate::space::one_line(&self.start, blocks::META_VALUE_CHARS)
            )
        } else {
            format!(
                "panel \"{}\"",
                crate::space::one_line(&self.title, blocks::TITLE_CHARS)
            )
        };
        // A slug holds nothing to clean, but its letters, digits, `-` and `_`.
        let id = self
            .slug
            .as_ref()
            .map(|slug| format!(" (id {slug})"))
            .unwrap_or_default();
        format!(
            "{} {verb} item {} of {} of {named}{id}: \"{}\"",
            crate::space::one_line(who, blocks::META_VALUE_CHARS),
            self.number,
            self.count,
            crate::space::one_line(&self.text, blocks::META_VALUE_CHARS)
        )
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod patch_tests;
