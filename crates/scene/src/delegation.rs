// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Bounded worker geometry. Positions belong to this client, never to the server.
//!
//! Live anchors do not move when metadata, ordering, or membership changes. Slots
//! are remembered for the current root; inactive slots may be reused under churn,
//! but a returning identity gets its remembered slot whenever it is still free.
use std::collections::{BTreeMap, BTreeSet};

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::anchors::{Anchor, Placed, Placement, Placing};
#[cfg(test)]
use crate::pointing::Target;
use crate::pointing::{Ray, Shape};
use crate::sides::{Hands, Side};
use crate::swarm::CENTER;

pub const MAX_CUBES: usize = 4096;
const MAX_NODES: usize = 64;
const MAX_CARDS: usize = 16;
// Match the spectator's station ID bound.
const MAX_ID_BYTES: usize = 160;
const MAX_WORKERS: usize = MAX_NODES * 2;
pub(crate) const SLOT_COUNT: usize = MAX_NODES * 3;
const PARTICLES: usize = 24;
const MAX_COORDINATOR_SAMPLES: usize = 128;
const ARRIVAL_SECONDS: f32 = 1.5;
const DEPARTURE_SECONDS: f32 = 1.2;
const TERMINAL_SECONDS: f32 = 0.9;
// Local-frame 4×3×2 partition of the completed cube. Cube scales are full
// side lengths, not half extents, in both the local and spectator renderer.
const COMPLETED_SIDE: f32 = 0.24;
const COMPLETED_PIECE: Vec3 = Vec3::new(
    COMPLETED_SIDE / 4.0,
    COMPLETED_SIDE / 3.0,
    COMPLETED_SIDE / 2.0,
);
const SPLIT_SECONDS: f32 = 0.35;
/// How near a worker's anchor the ray must pass to point at it: a little beyond its flock or completed cube, so that a
/// worker 3 m away is easy to hold. See `design/interaction.md`.
pub const POINTING_RADIUS: f32 = 0.3;
/// How far from the ray's origin a worker's pointing sphere stays [`POINTING_RADIUS`], in metres: beyond it, the sphere
/// grows in proportion, so that a worker the environment stands further off, as about an agent's home across a yard or
/// on a balcony, is as easy to hold as one 3 m away. See `design/interaction.md`.
pub const POINTING_REACH: f32 = 3.0;

/// What points at a worker besides its flock, for one whose middle is at `anchor` and whose sign is drawn `drawn` times
/// its own size, standing in its own place straight above it: its leader, from its middle up to its sign, and its sign,
/// as far as the tallest reaches and as wide as the widest, however it turns; each with its middle. See
/// `design/interaction.md`.
pub fn sign_shapes(anchor: Vec3, drawn: f32) -> [(Vec3, Shape); 2] {
    let lift = SIGN_LIFT * drawn;
    let top = SIGN_TOP * drawn;
    [
        (lift / 2.0, LEADER_HALF * drawn, lift / 2.0),
        ((lift + top) / 2.0, SIGN_HALF * drawn, (top - lift) / 2.0),
    ]
    .map(|(middle, across, up)| {
        let center = anchor + Vec3::Y * middle;
        (
            center,
            Shape::Box {
                center,
                half_extents: Vec3::new(across, up, across),
            },
        )
    })
}

/// How far either side of a sign's leader the ray points at it, in metres, at the sign's own size.
const LEADER_HALF: f32 = 0.06;

/// How near a worker's anchor at `center` a ray from `origin` must pass to point at it: [`POINTING_RADIUS`], or, beyond
/// [`POINTING_REACH`], as much more as keeps the same angle.
pub fn pointing_radius(origin: Vec3, center: Vec3) -> f32 {
    POINTING_RADIUS * (origin.distance(center) / POINTING_REACH).max(1.0)
}
/// The most satellites that orbit the agent while it is away from home.
pub const MOTES: usize = 8;
/// How large a satellite is, in metres across.
pub const MOTE_SIDE: f32 = 0.05;
/// How long a finished worker's satellite pulses and fades for, in seconds.
pub const PULSE_SECONDS: f32 = 1.5;
/// How long the satellites take to go once round the agent, in seconds.
const ORBIT_SECONDS: f32 = 12.0;
/// How far beyond the agent's reach the satellites orbit, in metres.
const ORBIT_CLEARANCE: f32 = 0.15;

/// A satellite: the identity and slot of the worker it stands for, its colour, its angle about the agent once shown,
/// and how long it has pulsed for, once its worker finished.
struct Mote {
    id: String,
    slot: usize,
    color: Vec3,
    angle: Option<f32>,
    pulse: Option<f32>,
}

/// How much paler the worker pointed at is drawn, towards white.
const HIGHLIGHT_WHITENING: f32 = 0.45;
/// How much larger the worker pointed at is drawn.
const HIGHLIGHT_SCALE: f32 = 1.25;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub nodes: Vec<Node>,
    pub presentations: Vec<Presentation>,
    pub error: Option<String>,
    /// Worker-list freshness independent of unrelated space errors. Absent in
    /// older space and spectator snapshots.
    pub workers_authoritative: Option<bool>,
    pub truncated: bool,
    pub stale: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Node {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub state: State,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Waiting,
    Idle,
    Completed,
    Error,
    /// Ended before it finished without failing; it may be resumed.
    Stopped,
    /// Not finished, but showing no progress for longer than the server allows. See `design/events.md`.
    Stalled,
    /// Given a message it has not answered for longer than the server allows. See `design/events.md`.
    Unanswered,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Presentation {
    pub id: String,
    pub source_id: String,
    pub source_message_id: String,
    pub title: String,
    pub kind: Kind,
    pub text: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Markdown,
    Table,
}

impl Workspace {
    /// Structural validation deliberately allows absent parents (including the
    /// coordinator). A partial archive is not evidence of a new root edge.
    pub fn valid(&self) -> bool {
        if self.nodes.len() > MAX_NODES
            || self.presentations.len() > MAX_CARDS
            || self.error.as_ref().is_some_and(|s| s.len() > 4096)
        {
            return false;
        }
        let mut nodes = BTreeMap::new();
        for node in &self.nodes {
            if !valid_id(&node.id)
                || node.parent_id.as_ref().is_some_and(|id| !valid_id(id))
                || node.title.chars().count() > 160
                || nodes.insert(node.id.as_str(), node).is_some()
            {
                return false;
            }
        }
        // At most 64 links per start, including self-parent cycles. No recursion
        // or unbounded traversal of untrusted parent references.
        for node in &self.nodes {
            let mut current = node.id.as_str();
            let mut seen = BTreeSet::new();
            while let Some(node) = nodes.get(current) {
                if !seen.insert(current) {
                    return false;
                }
                let Some(parent) = node.parent_id.as_deref() else {
                    break;
                };
                current = parent;
            }
        }
        let mut cards = BTreeSet::new();
        for card in &self.presentations {
            if !valid_id(&card.id)
                || !cards.insert(&card.id)
                || card.source_id.len() > MAX_ID_BYTES
                || card.source_message_id.len() > MAX_ID_BYTES
                || card.title.chars().count() > 160
                || card.text.len() > 4096
                || card.columns.len() > 6
                || card.columns.iter().any(|s| s.len() > 80)
                || card.rows.len() > 16
                || card.rows.iter().any(|row| {
                    row.len() > 6
                        || row.iter().any(|s| s.len() > 160)
                        || (card.kind == Kind::Table && row.len() != card.columns.len())
                })
            {
                return false;
            }
        }
        true
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES
}

/// The joined space's delegation workspace, derived from its shared worker state, and its root.
///
/// The root stands for the space's own agent session and is named by the space identifier, so joining another space is a root change that clears the scene. Space state gives workers the space's session started directly no parent; they hang off the root. A delegated worker keeps its parent.
#[derive(Debug, Default)]
pub struct Workers {
    root: Option<String>,
    current: bool,
    error: Option<String>,
    workers_authoritative: Option<bool>,
    workers: Vec<tomori_protocol::Worker>,
    workspace: Workspace,
}

impl Workers {
    /// Follow the joined space and whether its state is current, as given by `Conversation::joined_space`, or `None` outside a space. The workspace is derived again only when the space, its workers, its error or currency change.
    pub fn update(
        &mut self,
        space: Option<(&tomori_protocol::SpaceState, bool)>,
    ) -> (Option<&str>, &Workspace) {
        match space {
            None => {
                if self.root.is_some() {
                    *self = Self::default();
                }
            }
            Some((space, current)) => {
                if self.root.as_deref() != Some(space.id.as_str())
                    || self.current != current
                    || self.error != space.error
                    || self.workers_authoritative != space.workers_authoritative
                    || self.workers != space.workers
                {
                    self.root = Some(space.id.clone());
                    self.current = current;
                    self.error.clone_from(&space.error);
                    self.workers_authoritative = space.workers_authoritative;
                    self.workers.clone_from(&space.workers);
                    self.workspace = Workspace::from_space(space, current);
                }
            }
        }
        (self.root.as_deref(), &self.workspace)
    }
}

impl Workspace {
    /// The projection of `space`'s workers under the root named by its identifier, for [`Scene::update`].
    ///
    /// Workers beyond the node limit, and any with an out-of-contract identifier, are omitted and the projection marked truncated, so the scene treats it as partial rather than removing what it cannot see. A space error marks it partial in the same way, and state that is not `current` marks it stale. Cycles and duplicate identifiers are left for [`Workspace::valid`] to reject.
    pub fn from_space(space: &tomori_protocol::SpaceState, current: bool) -> Self {
        let in_contract = |worker: &&tomori_protocol::Worker| {
            valid_id(&worker.id) && worker.parent.as_deref().is_none_or(valid_id)
        };
        let valid = space.workers.iter().filter(in_contract).count();
        let truncated = valid < space.workers.len() || valid > MAX_NODES;
        let mut nodes = Vec::with_capacity(valid.min(MAX_NODES));
        // The server lists workers oldest first; beyond the node limit, the newest are kept, as the server keeps them.
        for worker in space
            .workers
            .iter()
            .filter(in_contract)
            .skip(valid.saturating_sub(MAX_NODES))
        {
            nodes.push(Node {
                id: worker.id.clone(),
                parent_id: Some(worker.parent.clone().unwrap_or_else(|| space.id.clone())),
                title: worker.title.chars().take(160).collect(),
                state: match worker.state {
                    tomori_protocol::WorkerState::Running => State::Running,
                    tomori_protocol::WorkerState::Waiting => State::Waiting,
                    tomori_protocol::WorkerState::Idle => State::Idle,
                    tomori_protocol::WorkerState::Completed => State::Completed,
                    tomori_protocol::WorkerState::Error => State::Error,
                    tomori_protocol::WorkerState::Stopped => State::Stopped,
                    tomori_protocol::WorkerState::Stalled => State::Stalled,
                    tomori_protocol::WorkerState::Unanswered => State::Unanswered,
                    tomori_protocol::WorkerState::Unknown => State::Unknown,
                },
            });
        }
        Self {
            nodes,
            presentations: Vec::new(),
            error: space.error.as_deref().map(|error| {
                let mut end = error.len().min(4096);
                while !error.is_char_boundary(end) {
                    end -= 1;
                }
                error[..end].to_owned()
            }),
            workers_authoritative: space.workers_authoritative,
            truncated,
            stale: !current,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Cube {
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub color: [f32; 3],
    /// XYZW unit quaternion; old spectator frames remain axis-aligned.
    #[serde(default = "identity_orientation")]
    pub orientation: [f32; 4],
}

fn identity_orientation() -> [f32; 4] {
    Quat::IDENTITY.to_array()
}

impl Cube {
    /// Also suitable for validating geometry received by the spectator.
    pub fn valid(&self) -> bool {
        self.position
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 100.0)
            && self
                .scale
                .iter()
                .all(|v| v.is_finite() && *v > 0.0 && *v <= 1.0)
            && self
                .color
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self
                .orientation
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1.0)
            && (Quat::from_array(self.orientation).length_squared() - 1.0).abs() <= 0.0001
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Station {
    pub id: String,
    pub position: [f32; 3],
    pub state: State,
    pub title: String,
    #[serde(default = "full_opacity")]
    pub opacity: f32,
    /// Where the eyes are of whoever its sign faces, the agent's audience: those at home, or those of whoever summoned
    /// it.
    #[serde(default = "home_eyes")]
    pub facing: [f32; 3],
    /// How large its sign is drawn, as a share of its full size: smaller in a compact cluster.
    #[serde(default = "full_opacity")]
    pub scale: f32,
    /// How many workers its sign stands for, gathered into a cluster's "+N more" sign, or 0 for its own worker's.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more: usize,
    /// How much larger than one within [`SIGN_REACH`] of whoever it faces its sign is drawn, where its cluster draws its
    /// signs at one size, as a ring does ([`tomori_protocol::cluster::Cluster::sign_size`]); or else, `None`, as far
    /// beyond that as it stands from them ([`sign_scale`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Whether this client's either ray points at its worker, its leader or its sign, which are drawn highlighted
    /// together. See `design/interaction.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub highlighted: bool,
}

fn is_zero(more: &usize) -> bool {
    *more == 0
}

/// How far from whoever it faces a sign is drawn at its own size, in metres: the default arc's radius, within about the
/// distance its text is comfortable to read from (see `design/text.md`); beyond it, a sign is drawn larger in
/// proportion, so that it reads as one on the arc does.
pub const SIGN_REACH: f32 = tomori_protocol::cluster::ARC_RADIUS;
/// How far above its worker's middle a sign's bottom edge stands, at its own size, in metres: clear of its flock and
/// completed cube, its leader hanging between.
pub const SIGN_LIFT: f32 = 0.3;
/// How far above its worker's middle the top of the tallest sign stands, at its own size, in metres: its lift and six
/// rows of title, its state's row and its plate's margins.
pub const SIGN_TOP: f32 = 0.63;
/// How far from its upright a sign reaches, however it turns, at its own size, in metres: half of the widest sign.
pub const SIGN_HALF: f32 = 0.5;

/// How much larger the sign of a station at `station` is drawn than one within [`SIGN_REACH`] of whoever's eyes, at
/// `facing`, it faces: 1 within it, and in proportion to the station's distance from them, horizontally, beyond.
pub fn sign_scale(station: [f32; 3], facing: [f32; 3]) -> f32 {
    ((station[0] - facing[0]).hypot(station[2] - facing[2]) / SIGN_REACH).max(1.0)
}

/// Where the eyes are at home, which a station's sign faces unless the agent is summoned.
pub const HOME_EYES: [f32; 3] = [0.0, 1.6, 0.0];

fn full_opacity() -> f32 {
    1.0
}

fn home_eyes() -> [f32; 3] {
    HOME_EYES
}

impl Station {
    #[cfg(any(not(target_os = "android"), test))]
    pub fn valid(&self) -> bool {
        valid_id(&self.id)
            && self.title.chars().count() <= 160
            && self
                .position
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 100.0)
            && self
                .facing
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 100.0)
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && self.scale.is_finite()
            && (0.1..=1.0).contains(&self.scale)
            && self.more <= MAX_WORKERS
            && self
                .size
                .is_none_or(|size| size.is_finite() && (1.0..=MOST_SIGN_SIZE).contains(&size))
    }

    /// How large its sign is drawn, as a share of one within [`SIGN_REACH`] of whoever it faces: as large as its
    /// cluster draws it, or as far beyond that as it stands, and smaller in a compact cluster.
    pub fn drawn(&self) -> f32 {
        drawn(self.size, self.position, self.facing, self.scale)
    }
}

/// How large the sign of a station at `position` is drawn, facing whoever's eyes are at `facing`, as a share of one
/// within [`SIGN_REACH`] of them: `size`, its cluster's, or else as far beyond that as it stands, and `scale` of that in
/// a compact cluster.
fn drawn(size: Option<f32>, position: [f32; 3], facing: [f32; 3], scale: f32) -> f32 {
    let size = size.filter(|size| size.is_finite()).map_or_else(
        || sign_scale(position, facing),
        |size| size.clamp(1.0, MOST_SIGN_SIZE),
    );
    size * compact_scale(scale)
}

/// How large a sign is drawn in a cluster that draws it at `scale` of its size, as a share of its full size: within 0.1
/// and 1.
pub fn compact_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.1, 1.0)
    } else {
        1.0
    }
}

/// The most a cluster draws its signs larger than at reading distance: as one 100 m away, the farthest a station stands.
const MOST_SIGN_SIZE: f32 = 100.0 / SIGN_REACH;

struct Worker {
    slot: usize,
    parent_id: Option<String>,
    title: String,
    state: State,
    // The observed form is independent of temporary Unknown display status
    // caused by stale or incomplete evidence.
    shape_state: State,
    continuous_evidence: bool,
    #[cfg(test)]
    origin: Vec3,
    /// Where it stands now, as it is drawn and pointed at, gliding towards its `place`.
    anchor: Vec3,
    /// Where it was placed, which it glides to when the arrangement changes.
    place: Vec3,
    /// How large its sign is drawn, as a share of its full size: smaller in a compact cluster.
    scale: f32,
    /// How much larger than at reading distance its cluster draws every sign, where it draws them at one size, as a
    /// ring does.
    size: Option<f32>,
    /// Whether it is gathered into its cluster's "+N more" sign, rather than standing in a slot of its own.
    more: bool,
    /// The cluster anchor it stands at, by name, as the server seats it, or `None` for home.
    at: Option<String>,
    particles: [Vec3; PARTICLES],
    particle_scales: [Vec3; PARTICLES],
    flight_start: [Vec3; PARTICLES],
    scale_start: [Vec3; PARTICLES],
    return_end: [Vec3; PARTICLES],
    color: Vec3,
    age: f32,
    arrival_seconds: f32,
    flying: bool,
    terminal_motion: Option<TerminalMotion>,
    retired: Option<f32>,
    departure_seconds: f32,
    visual_opacity: f32,
    seed: f32,
    orientation: Quat,
    orientation_start: Quat,
    ambient_time: f64,
}

#[derive(Clone, Copy)]
struct TerminalMotion {
    elapsed: f32,
    duration: f32,
    bounce: bool,
}

impl Worker {
    fn begin_terminal(&mut self, bounce: bool) {
        self.flying = false;
        self.flight_start = self.particles;
        self.scale_start = self.particle_scales;
        let target = std::array::from_fn(|i| terminal_goal(self, i));
        self.terminal_motion = Some(TerminalMotion {
            elapsed: 0.0,
            duration: flight_duration(&self.flight_start, &target, TERMINAL_SECONDS),
            bounce,
        });
    }
}

/// Session-local layout and animation; no server mutation or progress estimates.
pub struct Scene {
    root: Option<String>,
    // Small, bounded slot history, reset at every root switch.
    slots: BTreeMap<String, usize>,
    workers: BTreeMap<String, Worker>,
    cubes: Vec<Cube>,
    stations: Vec<Station>,
    time: f64,
    previous_trusted: bool,
    coordinator_samples: [Vec3; MAX_COORDINATOR_SAMPLES],
    coordinator_sample_count: usize,
    /// The worker pointed at, drawn highlighted.
    highlighted: Hands<Option<String>>,
    /// Where the headset's ray ends on the worker pointed at.
    hit_point: Hands<Option<Vec3>>,
    /// The origin of the workers' arrangement, home unless the joined space's environment says otherwise, or the agent
    /// while it is summoned.
    origin: Anchor,
    /// Where the eyes are of whoever the agent faces, whom the workers gather about.
    audience: Vec3,
    /// The room the home cluster may take, as the joined space's environment gives it, or none for today's arc.
    extent: Option<tomori_protocol::cluster::Extent>,
    /// How the home cluster stands about its point, as the joined space's environment says: the arc, or a ring, taking
    /// as much of the circle as its sweep says, in degrees, or the whole of it.
    formation: tomori_protocol::cluster::Formation,
    sweep: Option<f32>,
    /// The home cluster, about the origin, gathered about the audience, in the extent.
    cluster: tomori_protocol::cluster::Cluster,
    /// How the home cluster stands for the slots its live workers take: it changes only as the highest slot taken
    /// crosses what one of its steps holds, when every live worker is placed again.
    layout: tomori_protocol::cluster::Arrangement,
    /// The cluster whose "+N more" sign the ray points at, by its anchor's name, `None` within it for home.
    pointed_more: Hands<Option<Option<String>>>,
    /// The joined space's cluster anchors other than home, by name, each with its cluster and how the server lays it
    /// out. See `design/environments.md`.
    named: BTreeMap<
        String,
        (
            tomori_protocol::cluster::Cluster,
            tomori_protocol::cluster::Arrangement,
        ),
    >,
    /// Where the server seats each worker, by identity: at which anchor, `None` for home, and in which slot; empty
    /// from an older server, whose workers this client gives slots at home itself.
    seats: BTreeMap<String, (Option<String>, usize)>,
    /// The span the server lays the home cluster out for, where it seats the workers.
    home_laid: Option<usize>,
    /// Where the agent stands while it is away from home, and how far it reaches, which its satellites orbit.
    away: Option<(Vec3, f32)>,
    /// The satellites orbiting the agent while it is away, one for each worker still working, by identity.
    motes: Vec<Mote>,
    /// The satellites as they are drawn this frame, also among [`Self::cubes`].
    satellites: Vec<Cube>,
    /// The slots the home cluster's live and leaving workers took as it was last laid out, so that it is laid out again
    /// only as they change.
    reslotted: Option<(Vec<usize>, Vec<usize>)>,
    /// How workers are placed about it: by the joined space's environment's registered placement, or the default arc.
    placement: Option<Placement>,
    /// The placements that have panicked, which this scene never calls again: at most one for each environment the
    /// build registers one for.
    poisoned: Vec<Placement>,
    /// The joined space's timers and the time, in seconds since the Unix epoch, they count down from, as
    /// [`Self::set_timers`] last said, and the room their dials may take about the agent's home, clear of what the home
    /// cluster stands there at its every step. See `design/timers.md`.
    timers: Vec<tomori_protocol::Timer>,
    walls: Vec<tomori_protocol::Wall>,
    now: f64,
    room: crate::dials::Room,
    /// The dials drawn for them, among [`Self::cubes`].
    dials: crate::dials::Dials,
    /// Whether the last [`Self::point`] met a dial nearer than any worker.
    pointed_timer: Hands<bool>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Self {
        let mut scene = Self {
            root: None,
            slots: BTreeMap::new(),
            workers: BTreeMap::new(),
            cubes: Vec::with_capacity(MAX_CUBES),
            stations: Vec::with_capacity(MAX_WORKERS),
            time: 0.0,
            previous_trusted: false,
            coordinator_samples: [CENTER; MAX_COORDINATOR_SAMPLES],
            coordinator_sample_count: 0,
            highlighted: Hands::default(),
            hit_point: Hands::default(),
            origin: Anchor::AGENT,
            audience: crate::anchors::EYES,
            extent: None,
            formation: tomori_protocol::cluster::Formation::Arc,
            sweep: None,
            cluster: crate::anchors::cluster(Anchor::AGENT, crate::anchors::EYES, None),
            layout: crate::anchors::cluster(Anchor::AGENT, crate::anchors::EYES, None).arrange(1),
            pointed_more: Hands::default(),
            named: BTreeMap::new(),
            seats: BTreeMap::new(),
            home_laid: None,
            away: None,
            motes: Vec::new(),
            satellites: Vec::with_capacity(MOTES),
            reslotted: None,
            placement: None,
            poisoned: Vec::new(),
            timers: Vec::new(),
            walls: Vec::new(),
            now: 0.0,
            room: crate::dials::Room::open(Anchor::AGENT, crate::anchors::EYES),
            dials: crate::dials::Dials::default(),
            pointed_timer: Hands::default(),
        };
        scene.furnish();
        scene
    }

    /// Show the joined space's `timers` as dials at the agent's `home`, clear of its reach about its middle, `agent`,
    /// across, up and down, and deep, counting down at `now`, in seconds since the Unix epoch, by this client's clock,
    /// from the next [`Self::update`]. See `design/timers.md`.
    pub fn set_timers(
        &mut self,
        timers: &[tomori_protocol::Timer],
        home: Anchor,
        agent: Vec3,
        now: f64,
    ) {
        if self.timers != timers {
            self.timers = timers.to_vec();
        }
        self.room.home = home;
        // Placed for the agent's reach as it grows at once, and as it shrinks only once it has shrunk by SETTLE, so that
        // an animated avatar's changing bounds do not move the dials back and forth.
        let settled = self.room.agent;
        if agent.cmpgt(settled).any() || (settled - agent).max_element() > crate::dials::SETTLE {
            self.room.agent = agent;
        }
        self.now = now;
    }

    /// The walls the joined space shows, `walls`, whose slots the dials keep clear of, its status wall among them,
    /// wherever it stands. See `design/timers.md`.
    pub fn set_walls(&mut self, walls: &[tomori_protocol::Wall]) {
        if self.walls != walls {
            self.walls = walls.to_vec();
            self.room.walls = crate::dials::Extent::walls(walls);
        }
    }

    /// What the home cluster stands about its point at its every step, and so where the dials may not, and the ceiling
    /// its formation keeps to, as the room for the dials.
    fn furnish(&mut self) {
        let mut extents: Vec<crate::dials::Extent> = Vec::new();
        for span in [
            1, 2, 3, 4, 5, 6, 8, 10, 12, 14, 16, 18, 20, 24, 28, 32, 40, 48, 64, 96, 128,
            SLOT_COUNT,
        ] {
            let arrangement = self.cluster.arrange(span);
            for slot in 0..span {
                let slot = self.cluster.slot(&arrangement, slot);
                let extent = crate::dials::Extent::worker(
                    Vec3::from_array(slot.position),
                    slot.scale,
                    self.audience,
                );
                if !extents.iter().any(|known| {
                    known.centre.distance(extent.centre) < 0.01 && known.radius >= extent.radius
                }) {
                    extents.push(extent);
                }
            }
        }
        self.room.audience = self.audience;
        self.room.ceiling = (self.formation == tomori_protocol::cluster::Formation::Ring)
            .then_some(tomori_protocol::cluster::FLOOR + tomori_protocol::cluster::RING_HIGHEST);
        self.room.extents = extents;
    }

    /// The dials drawn this frame, their cubes among [`Self::cubes`], for their signs. See `design/timers.md`.
    pub fn dials(&self) -> &[crate::dials::Dial] {
        self.dials.dials()
    }

    /// The timer whose dial the last [`Self::point`] met, by name, where it was nearer than any worker. See
    /// `design/timers.md`.
    pub fn pointed_timer(&self) -> Option<&str> {
        self.pointed_timer_by(Side::Right)
    }

    /// The timer whose dial `side`'s ray last met, nearer than any worker, by name.
    pub fn pointed_timer_by(&self, side: Side) -> Option<&str> {
        self.dials
            .pointed_by(side)
            .filter(|_| self.pointed_timer[side])
    }

    /// Arrange the workers about `origin`, gathered about whoever's eyes are at `audience`, by `placement`, or without
    /// one in the home cluster fitted to `extent`, or on the default arc without that, from now on. Each live worker is
    /// placed again, in the order of its slot, only when the arrangement changes, as when the space changes
    /// environment, and glides there, its sign with it; otherwise nothing moves. See `design/environments.md`.
    pub fn arrange(
        &mut self,
        origin: Anchor,
        audience: Vec3,
        extent: Option<tomori_protocol::cluster::Extent>,
        placement: Option<Placement>,
    ) {
        self.arrange_as(
            origin,
            audience,
            extent,
            (tomori_protocol::cluster::Formation::Arc, None),
            placement,
        );
    }

    /// Arrange the workers as [`Self::arrange`] does, the home cluster standing in `formation` about `origin`, with its
    /// sweep in degrees, if it gives one: on the arc, or in a ring about it, as the joined space's environment says. See
    /// `design/environments.md`.
    pub fn arrange_as(
        &mut self,
        origin: Anchor,
        audience: Vec3,
        extent: Option<tomori_protocol::cluster::Extent>,
        (formation, sweep): (tomori_protocol::cluster::Formation, Option<f32>),
        placement: Option<Placement>,
    ) {
        // One that has panicked is never called again: its workers stand on the arc.
        let placement = placement.filter(|placement| {
            !self
                .poisoned
                .iter()
                .any(|poisoned| std::sync::Arc::ptr_eq(placement, poisoned))
        });
        let same = self.origin == origin
            && self.audience == audience
            && self.extent == extent
            && self.formation == formation
            && self.sweep == sweep
            && match (&self.placement, &placement) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
        if same {
            return;
        }
        self.origin = origin;
        self.audience = audience;
        self.extent = extent;
        self.formation = formation;
        self.sweep = sweep;
        self.placement = placement;
        self.cluster = crate::anchors::cluster(origin, audience, extent)
            .with_formation(formation)
            .with_sweep(sweep.unwrap_or(tomori_protocol::cluster::MOST_SWEEP));
        self.layout = self
            .cluster
            .arrange(self.home_laid.unwrap_or_else(|| self.span(&[])));
        self.reslotted = None;
        self.furnish();
        self.place_live();
    }

    /// Lay out the joined space's cluster anchors other than home, `anchors`, in the space's coordinates, and seat its
    /// workers as the server's `layouts` say, from now on: each worker at its anchor, in its slot, each cluster laid out
    /// for the span the server keeps, so that every client lays them out alike. Without layouts, as from an older
    /// server, this client gives its workers slots at home itself. An anchor whose name, extent or pose is out of
    /// bounds is left out, and its workers stand at home, as are seats beyond the slots a client keeps. A worker whose
    /// seat or cluster changes glides to its new place; nothing else moves. Call before [`Self::update`]. See
    /// `design/environments.md`.
    pub fn seat(
        &mut self,
        anchors: &[tomori_protocol::cluster::Anchor],
        layouts: &[tomori_protocol::cluster::Layout],
    ) {
        use tomori_protocol::cluster::{Cluster, HOME, valid_name};
        let usable = |anchor: &&tomori_protocol::cluster::Anchor| {
            !anchor.is_home()
                && valid_name(&anchor.name)
                && anchor.extent.is_none_or(|extent| extent.valid())
                && Anchor::from_pose(&anchor.pose).is_some()
        };
        let clusters: BTreeMap<&str, Cluster> = anchors
            .iter()
            .filter(usable)
            .map(|anchor| (anchor.name.as_str(), Cluster::new(anchor)))
            .collect();
        let mut named = BTreeMap::new();
        let mut seats = BTreeMap::new();
        let mut home_laid = None;
        for layout in layouts {
            let span = (layout.span as usize).min(SLOT_COUNT);
            let at = if layout.anchor == HOME {
                home_laid = Some(span);
                None
            } else if let Some(cluster) = clusters.get(layout.anchor.as_str()) {
                named.insert(layout.anchor.clone(), (*cluster, cluster.arrange(span)));
                Some(layout.anchor.clone())
            } else {
                continue;
            };
            for seat in &layout.seats {
                if (seat.slot as usize) < SLOT_COUNT && valid_id(&seat.worker) {
                    seats.insert(seat.worker.clone(), (at.clone(), seat.slot as usize));
                }
            }
        }
        if named == self.named && seats == self.seats && home_laid == self.home_laid {
            return;
        }
        let named_before = std::mem::replace(&mut self.named, named);
        self.seats = seats;
        self.home_laid = home_laid;
        let layout_before = self.layout.clone();
        if let Some(laid) = home_laid {
            self.layout = self.cluster.arrange(laid);
        }
        // Each live worker whose seat or cluster changed is placed again, and glides there.
        let mut placed_again = Vec::new();
        let mut again_at_home = false;
        for (id, worker) in self
            .workers
            .iter()
            .filter(|(_, worker)| worker.retired.is_none())
        {
            let (at, slot) = self
                .seats
                .get(id)
                .cloned()
                .unwrap_or_else(|| (worker.at.clone(), worker.slot));
            let moved = at != worker.at || slot != worker.slot;
            let relaid = match &at {
                Some(name) => named_before.get(name) != self.named.get(name),
                None => self.placement.is_none() && layout_before != self.layout,
            };
            if moved || relaid {
                again_at_home |= at.is_none() && self.placement.is_some();
                placed_again.push((id.clone(), at, slot));
            }
        }
        for (id, at, slot) in &placed_again {
            let worker = self.workers.get_mut(id).expect("a live worker");
            worker.at.clone_from(at);
            worker.slot = *slot;
            self.slots.insert(id.clone(), *slot);
        }
        for mote in &mut self.motes {
            mote.slot = self.slots.get(&mote.id).copied().unwrap_or(mote.slot);
        }
        if again_at_home {
            // A registered placement is told of everyone, in the order of their slots.
            self.place_live();
            return;
        }
        for (id, at, slot) in placed_again {
            let placing = Placing::new(&id, slot);
            let (spot, _) = self.spot(at.as_deref(), &placing, false);
            let size = self.size_at(at.as_deref());
            let worker = self.workers.get_mut(&id).expect("a live worker");
            worker.place = spot.position;
            worker.scale = spot.scale;
            worker.size = size;
            worker.more = spot.more;
        }
    }

    /// Whether the server seats the workers, rather than this client giving them slots at home itself.
    fn seated(&self) -> bool {
        self.home_laid.is_some() || !self.seats.is_empty()
    }

    /// Where the worker `placing` names stands, in its slot of the cluster at the anchor `at`, or, for home, by the
    /// registered placement, unless it `panicked`, or in the home cluster; and whether the placement panicked.
    fn spot(
        &self,
        at: Option<&str>,
        placing: &Placing<'_>,
        panicked: bool,
    ) -> (crate::anchors::Spot, bool) {
        match at.and_then(|name| self.named.get(name)) {
            Some((cluster, arrangement)) => {
                let slot = cluster.slot(arrangement, placing.slot);
                (
                    crate::anchors::Spot {
                        position: Vec3::from_array(slot.position),
                        scale: slot.scale,
                        more: slot.more,
                    },
                    false,
                )
            }
            None => crate::anchors::place(
                self.origin,
                self.placement.as_ref().filter(|_| !panicked),
                placing,
                &self.cluster,
                &self.layout,
            ),
        }
    }

    /// The anchor whose cluster's "+N more" sign the last [`Self::point`] met, by name, `home` for home's, which stands
    /// for the workers gathered into it; or `None` where it met none. See `design/interaction.md`.
    pub fn pointed_gathered(&self) -> Option<&str> {
        self.pointed_gathered_by(Side::Right)
    }

    /// The cluster whose "+N more" sign `side`'s ray last met, as [`Self::pointed_gathered`] says of the right's.
    pub fn pointed_gathered_by(&self, side: Side) -> Option<&str> {
        self.pointed_more[side]
            .as_ref()
            .map(|at| at.as_deref().unwrap_or(tomori_protocol::cluster::HOME))
    }

    /// Close the gaps among the home cluster's slots, given room and no placement, as
    /// [`tomori_protocol::cluster::Cluster::reslot`] does, among the live workers and those `arriving`, keeping clear of
    /// the slots of those leaving and of the `absent` parents it remembers, and lay it out for the slots then taken, only
    /// as its workers come and go.
    fn reslot(&mut self, arriving: &mut [(usize, &str, &Node)], absent: &[usize]) {
        // Where the server seats them, it closes the gaps, and lays the clusters out.
        if self.seated() {
            return;
        }
        let live = |worker: &&Worker| worker.retired.is_none();
        let mut members: Vec<usize> = self
            .workers
            .values()
            .filter(live)
            .map(|worker| worker.slot)
            .chain(arriving.iter().map(|(slot, ..)| *slot))
            .collect();
        let mut held: Vec<usize> = self
            .workers
            .values()
            .filter(|worker| worker.retired.is_some())
            .map(|worker| worker.slot)
            .chain(absent.iter().copied())
            .collect();
        let bounded = self.cluster.bounded() && self.placement.is_none();
        let changed = bounded && self.cluster.reslot(&mut members, &held, &self.layout);
        let mut moved = false;
        if changed {
            let mut slots = members.iter().copied();
            for (id, worker) in self
                .workers
                .iter_mut()
                .filter(|(_, worker)| worker.retired.is_none())
            {
                let slot = slots.next().expect("a slot for each live worker");
                if worker.slot != slot {
                    worker.slot = slot;
                    self.slots.insert(id.clone(), slot);
                    moved = true;
                }
            }
            for (entry, slot) in arriving.iter_mut().zip(slots) {
                entry.0 = slot;
                self.slots.insert(entry.1.to_owned(), slot);
            }
            // The satellites keep to their workers' slots.
            for mote in &mut self.motes {
                mote.slot = self.slots.get(&mote.id).copied().unwrap_or(mote.slot);
            }
        }
        members.sort_unstable();
        held.sort_unstable();
        let key = (members, held);
        if !changed && self.reslotted.as_ref() == Some(&key) {
            return;
        }
        let span = key.0.last().map_or(0, |slot| slot + 1);
        self.reslotted = Some(key);
        self.fit(span, moved);
    }

    /// How many slots the home cluster lays out for: up to the highest that a live worker, or one of `arriving`, takes.
    fn span(&self, arriving: &[usize]) -> usize {
        self.workers
            .values()
            .filter(|worker| worker.retired.is_none())
            .map(|worker| worker.slot)
            .chain(arriving.iter().copied())
            .max()
            .map_or(0, |slot| slot + 1)
    }

    /// Lay the home cluster out for `span` slots, placing every live worker again if that changes how it stands, or a
    /// live worker took another slot, and no placement is registered, which places each worker only as it arrives.
    fn fit(&mut self, span: usize, moved: bool) {
        let layout = self.cluster.arrange(span);
        if layout != self.layout || moved {
            self.layout = layout;
            if self.placement.is_none() {
                self.place_live();
            }
        }
    }

    /// Place every live worker again, in the order of its slot, each told of those placed before it.
    fn place_live(&mut self) {
        let root = self.root.clone();
        // Each live worker's slot, identity, parent, state and the anchor it stands at.
        type Live<'a> = (usize, &'a str, Option<&'a str>, State, Option<&'a str>);
        let mut live: Vec<Live<'_>> = self
            .workers
            .iter()
            .filter(|(_, worker)| worker.retired.is_none())
            .map(|(id, worker)| {
                let parent = worker
                    .parent_id
                    .as_deref()
                    .filter(|parent| Some(*parent) != root.as_deref());
                (
                    worker.slot,
                    id.as_str(),
                    parent,
                    worker.state,
                    worker.at.as_deref(),
                )
            })
            .collect();
        live.sort_by_key(|(slot, ..)| *slot);
        let mut placed: Vec<Placed<'_>> = Vec::with_capacity(live.len());
        let mut spots = Vec::with_capacity(live.len());
        let mut panicked = false;
        for (slot, id, parent, state, at) in live {
            let mut placing = Placing::new(id, slot);
            placing.parent = parent;
            placing.state = state;
            placing.workers = &placed;
            placing.audience = self.audience;
            let (spot, panic) = self.spot(at, &placing, panicked);
            panicked |= panic;
            // Placed in the order of their slots, so those placed stay in that order.
            placed.push(Placed::new(id, slot, state, spot.position));
            spots.push((id.to_owned(), spot, self.size_at(at)));
        }
        // Each glides there, its flock and its sign with it.
        for (id, spot, size) in spots {
            let worker = self
                .workers
                .get_mut(&id)
                .expect("each live worker was placed");
            worker.place = spot.position;
            worker.scale = spot.scale;
            worker.size = size;
            worker.more = spot.more;
        }
        self.give_up(panicked);
    }

    /// Never call the placement again if it `panicked`, saying so once, so that its workers stand on the arc.
    fn give_up(&mut self, panicked: bool) {
        if panicked {
            log::warn!(
                "The environment's worker placement panicked; its workers stand on the default arc from now on."
            );
            self.poisoned.extend(self.placement.take());
        }
    }

    /// Where the identity `id`, in `slot`, stands: where its worker does, if it is live, or else where the default arc
    /// puts its slot about the origin, as for a parent that is not shown.
    fn standing(&self, id: &str, slot: usize) -> Vec3 {
        self.workers.get(id).map_or_else(
            || Vec3::from_array(self.cluster.slot(&self.layout, slot).position),
            |worker| worker.place,
        )
    }

    /// The workers that can be pointed at, keyed by identifier, where they stand now: each live worker's anchor, as a
    /// sphere a little larger than its flock, as from within [`POINTING_REACH`], and its leader and sign above it
    /// ([`sign_shapes`]). Workers leaving cannot be pointed at. [`Self::point`] tests the same, its sphere growing with
    /// the distance beyond that.
    #[cfg(test)]
    pub fn targets(&self) -> Vec<Target<String>> {
        self.workers
            .iter()
            .filter(|(_, worker)| worker.retired.is_none())
            .flat_map(|(id, worker)| {
                let flock = Shape::Sphere {
                    center: worker.anchor,
                    radius: POINTING_RADIUS,
                };
                std::iter::once(flock)
                    .chain(sign_shapes(worker.anchor, self.drawn(worker)).map(|(_, shape)| shape))
                    .map(|shape| Target {
                        key: id.clone(),
                        shape,
                    })
            })
            .collect()
    }

    /// Point at the workers with `ray`, or with nothing: highlight the nearest worker it meets and return its
    /// identifier, or, for one gathered into the "+N more" sign, highlight them all and list them on it, naming none.
    /// Pointing only highlights; it never takes the trigger. The satellites are never pointed at. See
    /// `design/interaction.md`.
    pub fn point(&mut self, ray: Option<Ray>) -> Option<&str> {
        self.point_with(Side::Right, ray)
    }

    /// Point at the workers with `side`'s ray, `ray`, as [`Self::point`] does with the right's: what either ray meets
    /// is highlighted.
    pub fn point_with(&mut self, side: Side, ray: Option<Ray>) -> Option<&str> {
        // Each frame, without allocating: the same test as [`Self::targets`] with [`crate::pointing::nearest`].
        // How far along the ray, which worker, and where the ray ends on it.
        let mut hit: Option<(f32, &str, Vec3)> = None;
        if let Some(ray) = ray {
            for (id, worker) in &self.workers {
                if worker.retired.is_some() {
                    continue;
                }
                let shape = Shape::Sphere {
                    center: worker.anchor,
                    radius: pointing_radius(ray.origin, worker.anchor),
                };
                // Among its flock, where the ray passes nearest its anchor, rather than where it enters the sphere.
                let flock = shape.intersect(&ray).map(|distance| {
                    let along = (worker.anchor - ray.origin)
                        .dot(ray.direction)
                        .max(distance);
                    (distance, ray.at(along))
                });
                // Or its leader or its sign, standing straight above it, the ray ending where it passes their upright,
                // as a sign faces whoever looks at it.
                let sign = sign_shapes(worker.anchor, self.drawn(worker))
                    .into_iter()
                    .filter_map(|(center, shape)| {
                        let distance = shape.intersect(&ray)?;
                        let along = (center - ray.origin).dot(ray.direction).max(distance);
                        Some((distance, ray.at(along)))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let met = [flock, sign]
                    .into_iter()
                    .flatten()
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((distance, point)) = met
                    && hit.is_none_or(|(nearest, _, _)| distance < nearest)
                {
                    hit = Some((distance, id, point));
                }
            }
        }
        // A timer's dial nearer than any worker is pointed at instead.
        let dial = self.dials.point_with(side, ray);
        let timer =
            dial.is_some_and(|(distance, _)| hit.is_none_or(|(nearest, ..)| distance < nearest));
        self.pointed_timer[side] = timer;
        let hit = hit.filter(|_| !timer);
        // The ray ends among the worker's flock, or on its leader or sign.
        self.hit_point[side] = hit.map(|(_, _, point)| point);
        if timer {
            self.hit_point[side] = dial.map(|(_, point)| point);
        }
        // One gathered into a "+N more" sign is pointed at as the sign, which stands for all of them.
        self.pointed_more[side] = hit
            .and_then(|(_, id, _)| self.workers.get(id))
            .filter(|worker| worker.more)
            .map(|worker| worker.at.clone());
        let more = self.pointed_more[side].is_some();
        let hit = hit.map(|(_, id, _)| id).filter(|_| !more);
        if self.highlighted[side].as_deref() != hit {
            self.highlighted[side] = hit.map(str::to_owned);
        }
        self.highlighted[side].as_deref()
    }

    /// How much larger than at reading distance `worker`'s sign is drawn, as its cluster draws it: one size for a ring's,
    /// or else as far beyond reading distance as it stands, as [`Station::drawn`] says of its station.
    fn drawn(&self, worker: &Worker) -> f32 {
        drawn(
            worker.size,
            worker.anchor.to_array(),
            self.audience.to_array(),
            worker.scale,
        )
    }

    /// How much larger than at reading distance the cluster at the anchor `at`, `None` for home, draws every sign, where
    /// it draws them at one size, as a ring does: its named anchor's cluster, or home's, unless a registered placement
    /// places its workers.
    fn size_at(&self, at: Option<&str>) -> Option<f32> {
        match at {
            Some(name) => self
                .named
                .get(name)
                .and_then(|(cluster, _)| cluster.sign_size()),
            None => self
                .placement
                .is_none()
                .then(|| self.cluster.sign_size())
                .flatten(),
        }
    }

    /// Where the last [`Self::point`] met the worker it highlights, or the dial, or `None` when it met neither.
    pub fn hit_point(&self) -> Option<Vec3> {
        self.hit_point.right
    }

    /// Where `side`'s ray last met the worker it highlights, or the dial, as [`Self::hit_point`] says of the right's.
    pub fn hit_point_by(&self, side: Side) -> Option<Vec3> {
        self.hit_point[side]
    }

    pub fn cubes(&self) -> &[Cube] {
        &self.cubes
    }

    /// Say where the agent stands while it is `away` from home, its middle and how far it reaches, or `None` while it
    /// is home: its satellites orbit it while it is away. Call before [`Self::update`]. See `design/interaction.md`.
    pub fn set_agent(&mut self, away: Option<(Vec3, f32)>) {
        self.away = away
            .filter(|(centre, reach)| crate::anchors::within(*centre, 90.0) && reach.is_finite());
    }

    /// The satellites drawn this frame, among [`Self::cubes`]: while the agent is away from home, a small mote for each
    /// of at most [`MOTES`] workers still working, running or waiting, in the order of their slots, in its state's
    /// colour, orbiting it; a worker finishing while it is away, its mote pulsing once and fading. They are never
    /// pointed at and have no signs. See `design/interaction.md`.
    pub fn satellites(&self) -> &[Cube] {
        &self.satellites
    }

    /// Move the satellites on by `dt` seconds about the agent, and draw them.
    fn orbit(&mut self, dt: f32) {
        self.satellites.clear();
        let Some((centre, reach)) = self.away else {
            self.motes.clear();
            return;
        };
        for (id, worker) in &self.workers {
            let working = worker.retired.is_none() && is_working(worker.state);
            let finished = worker.retired.is_none() && is_terminal(worker.state);
            match self.motes.iter_mut().find(|mote| mote.id == *id) {
                Some(mote) if working => {
                    mote.color = palette(worker.state);
                    mote.pulse = None;
                }
                Some(mote) if finished && mote.pulse.is_none() => {
                    mote.color = palette(worker.state);
                    mote.pulse = Some(0.0);
                }
                None if working => {
                    self.motes.push(Mote {
                        id: id.clone(),
                        slot: worker.slot,
                        color: palette(worker.state),
                        angle: None,
                        pulse: None,
                    });
                }
                _ => {}
            }
        }
        // A mote stays while its worker works, or for its one pulse.
        let workers = &self.workers;
        self.motes.retain_mut(|mote| match &mut mote.pulse {
            Some(pulse) => {
                *pulse += dt;
                *pulse < PULSE_SECONDS
            }
            None => workers
                .get(&mote.id)
                .is_some_and(|worker| worker.retired.is_none() && is_working(worker.state)),
        });
        // In the order of their slots, sorted in place, so that drawing them allocates nothing.
        self.motes.sort_unstable_by_key(|mote| mote.slot);
        let count = self.motes.len().min(MOTES);
        let spin = (self.time / f64::from(ORBIT_SECONDS)).fract() as f32 * std::f32::consts::TAU;
        let radius = reach.max(0.2) + ORBIT_CLEARANCE;
        for (index, mote) in self.motes.iter_mut().take(MOTES).enumerate() {
            // Evenly about it, each easing round to its place as others come and go.
            let target = index as f32 * std::f32::consts::TAU / count as f32;
            let angle = mote.angle.get_or_insert(target);
            let turn = (target - *angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            *angle += turn * (1.0 - (-4.0 * dt).exp());
            let around = *angle + spin;
            let size = MOTE_SIDE * mote.pulse.map_or(1.0, pulse);
            let position = centre
                + Vec3::new(
                    around.cos() * radius,
                    (around * 2.0).sin() * 0.05,
                    around.sin() * radius,
                );
            let before = self.cubes.len();
            push_cube(
                &mut self.cubes,
                position,
                Vec3::splat(size),
                mote.color,
                Quat::IDENTITY,
            );
            self.satellites.extend(self.cubes[before..].iter().copied());
        }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<Cube> {
        self.cubes.clone()
    }

    pub fn stations(&self) -> &[Station] {
        &self.stations
    }

    /// Copy at most 128 current coordinator cube positions, without allocating.
    /// Invalid or out-of-bounds positions (coordinates outside ±90 m) are ignored.
    /// An empty/invalid input restores the CENTER fallback. Call before update.
    pub fn set_coordinator_samples(&mut self, positions: &[[f32; 3]]) {
        self.coordinator_sample_count = 0;
        for position in positions.iter().take(MAX_COORDINATOR_SAMPLES) {
            let position = Vec3::from_array(*position);
            if crate::anchors::within(position, 90.0) {
                self.coordinator_samples[self.coordinator_sample_count] = position;
                self.coordinator_sample_count += 1;
            }
        }
    }

    /// `workspace` must be the server's projection for `root`. Its contract has
    /// no root identifier, so detached orphans cannot independently prove
    /// which root they belong to. A root change always clears prior state.
    pub fn update(&mut self, root: Option<&str>, workspace: &Workspace, dt: f32) {
        let before = self.root.clone();
        self.update_workers(root, workspace, dt);
        // The timers' dials stand above the agent's home, among the workers' cubes, only in a space.
        if self.root.is_none() || self.root != before {
            self.dials.clear();
        }
        if self.root.is_some() {
            self.dials.update(
                &self.timers,
                !workspace.stale,
                &self.room,
                self.now,
                dt,
                &mut self.cubes,
            );
        }
    }

    fn update_workers(&mut self, root: Option<&str>, workspace: &Workspace, dt: f32) {
        let root = root.filter(|id| valid_id(id));
        if self.root.as_deref() != root {
            self.root = root.map(str::to_owned);
            self.slots.clear();
            self.workers.clear();
            self.time = 0.0;
            self.previous_trusted = false;
        }
        self.cubes.clear();
        self.stations.clear();
        let Some(root) = root else { return };
        // NaN and resume stalls cannot poison the simulation or skip arrivals.
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.05)
        } else {
            0.0
        };
        self.time += f64::from(dt);
        let valid = workspace.valid();
        let fresh = valid && !workspace.stale;
        let trusted = fresh
            && workspace
                .workers_authoritative
                .unwrap_or(workspace.error.is_none())
            && !workspace.truncated;
        let launch_new = trusted && self.previous_trusted;
        if trusted {
            self.reconcile(root, &workspace.nodes, launch_new, None);
        } else if fresh {
            // Partial discovery still contains real observed workers. Preserve
            // existing stations when omissions aren't authoritative, and add
            // newly observed ones up to the same fixed budget. Omitted workers
            // become unknown; observed workers retain their independent states.
            let mut merged = BTreeMap::<String, Node>::new();
            for (id, worker) in &self.workers {
                if worker.retired.is_none() {
                    merged.insert(
                        id.clone(),
                        Node {
                            id: id.clone(),
                            parent_id: worker.parent_id.clone(),
                            title: worker.title.clone(),
                            state: State::Unknown,
                        },
                    );
                }
            }
            let candidates: BTreeMap<_, _> = workspace
                .nodes
                .iter()
                .map(|node| (node.id.as_str(), node))
                .collect();
            let mut observed = BTreeSet::new();
            for node in &workspace.nodes {
                // Only accept partial nodes whose observed parent chain reaches
                // this root, through this projection or through workers this
                // root already shows, such as a parent the projection left out.
                // Stale projections are never merged at all.
                let mut parent = node.parent_id.as_deref();
                let mut reaches_root = false;
                for _ in 0..MAX_NODES {
                    match parent {
                        Some(id) if id == root => {
                            reaches_root = true;
                            break;
                        }
                        Some(id) => {
                            parent = match candidates.get(id) {
                                Some(next) => next.parent_id.as_deref(),
                                None => self
                                    .workers
                                    .get(id)
                                    .filter(|known| known.retired.is_none())
                                    .and_then(|known| known.parent_id.as_deref()),
                            }
                        }
                        None => break,
                    }
                }
                if !reaches_root {
                    continue;
                }
                if merged.contains_key(&node.id) || merged.len() < MAX_NODES {
                    merged.insert(node.id.clone(), node.clone());
                    observed.insert(node.id.as_str());
                }
            }
            self.reconcile(
                root,
                &merged.into_values().collect::<Vec<_>>(),
                false,
                Some(&observed),
            );
        } else {
            for worker in self.workers.values_mut() {
                worker.continuous_evidence = false;
            }
        }
        self.previous_trusted = trusted;
        // Only stale or invalid status evidence is globally unknown. A fresh
        // partial hierarchy cannot remove workers, but can report their states.
        let time = self.time;
        let mut retiring_stations = Vec::new();
        let mut gathered: BTreeMap<Option<String>, Vec<(usize, Station)>> = BTreeMap::new();
        for (id, worker) in &mut self.workers {
            // Only a fully fitted completed form rotates as a rigid body. Its
            // hidden partition follows exactly, so splitting never snaps back
            // into an axis-aligned frame. No metadata refresh resets this clock.
            let was_solid = solid_completed(worker);
            // A live worker placed elsewhere, as when the agent is summoned or sent home, glides there.
            if dt > 0.0 && worker.retired.is_none() {
                worker.anchor = glide(worker.anchor, worker.place, dt);
            }
            if dt > 0.0 && worker.retired.is_none() {
                worker.ambient_time += f64::from(dt);
                if was_solid {
                    worker.orientation =
                        (Quat::from_axis_angle(Vec3::new(0.35, 1.0, 0.2).normalize(), 0.09 * dt)
                            * worker.orientation)
                            .normalize();
                    worker.particles = std::array::from_fn(|i| terminal_goal(worker, i));
                } else if worker.shape_state != State::Completed {
                    worker.orientation = worker
                        .orientation
                        .slerp(Quat::IDENTITY, 1.0 - (-2.0 * dt).exp())
                        .normalize();
                }
            } else if dt > 0.0
                && let Some(retired) = worker.retired
            {
                worker.orientation = worker
                    .orientation_start
                    .slerp(
                        Quat::IDENTITY,
                        smooth(
                            ((retired + if trusted { dt } else { 0.0 }) / SPLIT_SECONDS).min(1.0),
                        ),
                    )
                    .normalize();
            }
            worker.age = (worker.age + dt).min(3600.0);
            if trusted && let Some(retired) = &mut worker.retired {
                *retired += dt;
            }
            let state = if fresh { worker.state } else { State::Unknown };
            let target_color = palette(state);
            worker.color = worker.color.lerp(target_color, 1.0 - (-6.0 * dt).exp());
            let opacity = opacity(worker) * if fresh { 1.0 } else { 0.55 };
            if worker.retired.is_none()
                && let Some(motion) = &mut worker.terminal_motion
            {
                motion.elapsed = (motion.elapsed + dt).min(motion.duration);
            }
            let terminal_motion = worker.terminal_motion;
            let flying = worker.flying;
            let target_opacity = if let Some(retired) = worker.retired {
                1.0 - smooth(((retired - (worker.departure_seconds - 0.3)) / 0.3).clamp(0.0, 1.0))
            } else {
                1.0
            } * if fresh { 1.0 } else { 0.55 };
            // In particular, do not snap back to full size if a partially
            // faded departure is resurrected, or evidence becomes stale.
            worker.visual_opacity +=
                (target_opacity - worker.visual_opacity).clamp(-4.0 * dt, 4.0 * dt);
            let goals: [Vec3; PARTICLES] = std::array::from_fn(|i| terminal_goal(worker, i));
            for (i, position) in worker.particles.iter_mut().enumerate() {
                let goal = if is_terminal(worker.shape_state) {
                    // Goals are precomputed to avoid borrowing the worker
                    // while mutating its particle array.
                    goals[i]
                } else {
                    worker.anchor + flock_offset(i, worker.seed, time, state)
                };
                if let Some(retired) = worker.retired {
                    let t = (retired / worker.departure_seconds).min(1.0);
                    *position = worker.flight_start[i].lerp(worker.return_end[i], smooth(t))
                        + flight_arc(i, worker.seed, t, 0.20);
                } else if let Some(motion) = terminal_motion {
                    let t = (motion.elapsed / motion.duration).min(1.0);
                    let bounce = if motion.bounce {
                        0.06 * (std::f32::consts::TAU * t).sin() * (1.0 - t)
                    } else {
                        0.0
                    };
                    *position = worker.flight_start[i].lerp(goal, smooth(t)) + Vec3::Y * bounce;
                } else if flying {
                    let t = (worker.age / worker.arrival_seconds).min(1.0);
                    let next = worker.flight_start[i].lerp(goal, smooth(t))
                        + flight_arc(i, worker.seed, t, 0.35);
                    // A status goal may change mid-flight. Bound the adjustment
                    // rather than jumping to the new trajectory at the same t.
                    *position += (next - *position).clamp_length_max(6.0 * dt);
                } else if is_terminal(worker.shape_state) {
                    // A terminal worker first seen in flight may need a final
                    // small correction. Error goals carry only a slow wave.
                    let delta = goal - *position;
                    if delta.length() <= 0.0001 && delta.length() <= 6.0 * dt {
                        *position = goal;
                    } else {
                        *position +=
                            (delta * (1.0 - (-12.0 * dt).exp())).clamp_length_max(6.0 * dt);
                    }
                } else {
                    let change = ((goal - *position) * (1.0 - (-12.0 * dt).exp()))
                        .clamp_length_max(6.0 * dt);
                    *position += change;
                }
                let small = particle_scale(i);
                let goal_scale = if worker.shape_state == State::Completed {
                    COMPLETED_PIECE
                } else {
                    small
                };
                let scale = &mut worker.particle_scales[i];
                if let Some(retired) = worker.retired {
                    *scale = worker.scale_start[i]
                        .lerp(small, smooth((retired / SPLIT_SECONDS).min(1.0)));
                } else if let Some(motion) = terminal_motion {
                    *scale = worker.scale_start[i].lerp(
                        goal_scale,
                        smooth((motion.elapsed / motion.duration).min(1.0)),
                    );
                } else if flying {
                    let t = (worker.age / worker.arrival_seconds).min(1.0);
                    *scale = worker.scale_start[i]
                        .lerp(goal_scale, smooth(((t - 0.65) / 0.35).clamp(0.0, 1.0)));
                } else {
                    *scale = scale.lerp(goal_scale, 1.0 - (-12.0 * dt).exp());
                    // A resurrection can interrupt the split just before its
                    // target; a zero-time reconciliation must not snap it.
                    if dt > 0.0 && scale.distance(goal_scale) < 0.00001 {
                        *scale = goal_scale;
                    }
                }
            }
            if flying && worker.age >= worker.arrival_seconds {
                worker.flying = false;
            }
            if terminal_motion.is_some_and(|motion| motion.elapsed >= motion.duration) {
                worker.terminal_motion = None;
            }
            let color = worker.color * (0.70 + 0.30 * worker.visual_opacity);
            // The worker pointed at glows paler and a little larger, so the participant sees what they would ask about.
            let pointed = worker.retired.is_none()
                && (self
                    .highlighted
                    .any(|_, highlighted| highlighted.as_deref() == Some(id.as_str()))
                    || (worker.more
                        && self
                            .pointed_more
                            .any(|_, more| more.as_ref() == Some(&worker.at))));
            let (color, emphasis) = if pointed {
                (color.lerp(Vec3::ONE, HIGHLIGHT_WHITENING), HIGHLIGHT_SCALE)
            } else {
                (color, 1.0)
            };
            if solid_completed(worker) {
                push_cube(
                    &mut self.cubes,
                    worker.anchor,
                    Vec3::splat(COMPLETED_SIDE * worker.visual_opacity * emphasis),
                    color,
                    worker.orientation,
                );
            } else {
                // Scale the whole geometry about its centroid when fading. This
                // preserves a fitted partition, including at the solid/split
                // boundary, rather than opening gaps between shrinking pieces.
                let center = worker.particles.iter().copied().sum::<Vec3>() / PARTICLES as f32;
                for (position, scale) in worker.particles.iter().zip(worker.particle_scales) {
                    let position = if worker.visual_opacity == 1.0 && emphasis == 1.0 {
                        *position
                    } else {
                        center + (*position - center) * worker.visual_opacity * emphasis
                    };
                    push_cube(
                        &mut self.cubes,
                        position,
                        scale * worker.visual_opacity * emphasis,
                        color,
                        worker.orientation,
                    );
                }
            }
            let station = Station {
                facing: self.audience.to_array(),
                id: id.clone(),
                position: worker.anchor.to_array(),
                state,
                title: worker.title.clone(),
                opacity,
                scale: worker.scale,
                more: 0,
                size: worker.size,
                // Its sign and leader are highlighted with its flock.
                highlighted: pointed,
            };
            if worker.more {
                // Gathered into the "+N more" sign, it has none of its own.
                if worker.retired.is_none() {
                    gathered
                        .entry(worker.at.clone())
                        .or_default()
                        .push((worker.slot, station));
                }
            } else if worker.retired.is_none() {
                self.stations.push(station);
            } else {
                retiring_stations.push(station);
            }
        }
        // One sign for those gathered in each cluster, where the first of them stands, listing them while it is pointed
        // at.
        for (at, mut gathered) in gathered {
            gathered.sort_by_key(|(slot, _)| *slot);
            let mut sign = gathered[0].1.clone();
            sign.more = gathered.len();
            sign.title = if self.pointed_more.any(|_, more| more.as_ref() == Some(&at)) {
                let titles: Vec<&str> = gathered
                    .iter()
                    .map(|(_, station)| station.title.as_str())
                    .collect();
                titles.join(" · ").chars().take(160).collect()
            } else {
                String::new()
            };
            self.stations.push(sign);
        }
        self.orbit(dt);
        // Prefer live stations when departures and arrivals overlap.
        let remaining = MAX_WORKERS.saturating_sub(self.stations.len());
        self.stations
            .extend(retiring_stations.into_iter().take(remaining));
        self.workers
            .retain(|_, worker| worker.retired.is_none_or(|t| t < worker.departure_seconds));
        // Only worker flocks are rendered. Without interaction telemetry, links
        // would imply communication that this projection cannot establish.
        // At most 128 workers (including departures) emit 24 cubes each.
    }

    fn reconcile(
        &mut self,
        root: &str,
        nodes: &[Node],
        launch_new: bool,
        observed: Option<&BTreeSet<&str>>,
    ) {
        // A parentless entry can describe the root, but is not evidence that an
        // unrelated session belongs to this workspace.
        let nodes: BTreeMap<_, _> = nodes
            .iter()
            .filter(|n| n.id != root && n.parent_id.is_some())
            .map(|n| (n.id.as_str(), n))
            .collect();
        let mut returning = Vec::new();
        for (id, worker) in &mut self.workers {
            if let Some(node) = nodes.get(id.as_str()) {
                worker.parent_id.clone_from(&node.parent_id);
                worker.title.clone_from(&node.title);
                worker.state = node.state;
                let resurrected = worker.retired.take().is_some();
                let actually_observed = observed.is_none_or(|ids| ids.contains(id.as_str()));
                if actually_observed && node.state != State::Unknown {
                    if worker.shape_state != node.state {
                        let was_terminal = is_terminal(worker.shape_state);
                        worker.shape_state = node.state;
                        if is_terminal(node.state) {
                            worker.begin_terminal(worker.continuous_evidence && !resurrected);
                        } else {
                            worker.terminal_motion = None;
                            if was_terminal {
                                worker.flying = false;
                            }
                        }
                    }
                    worker.continuous_evidence = true;
                } else {
                    worker.continuous_evidence = false;
                    // An explicitly observed Unknown is a real display state,
                    // unlike a synthetic Unknown caused by an omission. Flow
                    // back to its flock without inventing a later completion.
                    if actually_observed && node.state == State::Unknown {
                        if is_terminal(worker.shape_state) {
                            worker.flying = false;
                        }
                        worker.shape_state = State::Unknown;
                        worker.terminal_motion = None;
                    }
                }
                if resurrected {
                    // Re-form without an invented status bounce, starting at
                    // the exact position reached by the interrupted return.
                    worker.flying = false;
                    if is_terminal(worker.shape_state) {
                        worker.begin_terminal(false);
                    }
                }
            } else if worker.retired.is_none() {
                worker.retired = Some(0.0);
                worker.continuous_evidence = false;
                worker.terminal_motion = None;
                worker.flight_start = worker.particles;
                worker.scale_start = worker.particle_scales;
                worker.orientation_start = worker.orientation;
                // A nested worker returns to the station of the parent it came from, while that parent stays; one
                // whose parent leaves too, or is not shown, returns to the coordinator.
                let parent = worker
                    .parent_id
                    .as_deref()
                    .filter(|parent| *parent != root && nodes.contains_key(parent))
                    .and_then(|parent| self.slots.get(parent).map(|slot| (parent, *slot)));
                match parent {
                    // Where the parent stands is known once this loop has let go of the workers.
                    Some((parent, slot)) => returning.push((id.clone(), parent.to_owned(), slot)),
                    None => {
                        worker.return_end = std::array::from_fn(|i| {
                            sample(
                                &self.coordinator_samples,
                                self.coordinator_sample_count,
                                id,
                                i,
                            )
                        });
                        worker.departure_seconds = flight_duration(
                            &worker.flight_start,
                            &worker.return_end,
                            DEPARTURE_SECONDS,
                        );
                    }
                }
            }
        }
        for (id, parent, slot) in returning {
            let end = self.standing(&parent, slot);
            let worker = self
                .workers
                .get_mut(&id)
                .expect("it has just begun to leave");
            worker.return_end = [end; PARTICLES];
            worker.departure_seconds =
                flight_duration(&worker.flight_start, &worker.return_end, DEPARTURE_SECONDS);
        }
        // Reserve room for authoritative nodes first. Keep at most 64 departures;
        // extreme churn drops the oldest departure rather than overflowing slots.
        let mut retiring: Vec<_> = self
            .workers
            .iter()
            .filter_map(|(id, w)| w.retired.map(|age| (id.clone(), age)))
            .collect();
        retiring.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (id, age) in retiring
            .iter()
            .take(retiring.len().saturating_sub(MAX_NODES))
        {
            let _ = age;
            self.workers.remove(id);
        }
        self.workers
            .retain(|_, w| w.retired.is_none_or(|t| t < w.departure_seconds));

        let mut required = BTreeSet::new();
        for node in nodes.values() {
            required.insert(node.id.as_str());
            if let Some(parent) = node.parent_id.as_deref().filter(|parent| *parent != root) {
                required.insert(parent);
            }
        }
        // Retain every active identity and currently referenced parent, then
        // evict excess old history deterministically rather than growing forever.
        let mut spare = SLOT_COUNT.saturating_sub(
            required.len().saturating_add(
                self.workers
                    .keys()
                    .filter(|id| !required.contains(id.as_str()))
                    .count(),
            ),
        );
        self.slots.retain(|id, _| {
            if required.contains(id.as_str()) || self.workers.contains_key(id) {
                true
            } else if spare > 0 {
                spare -= 1;
                true
            } else {
                false
            }
        });
        let mut order: Vec<_> = required.into_iter().collect();
        // Protect remembered absent-parent provenance, but do not let new
        // absent-parent slots consume the best positions before real workers.
        order.sort_by_key(|id| {
            let priority = if !nodes.contains_key(id) && self.slots.contains_key(*id) {
                0
            } else if nodes.contains_key(id) {
                1
            } else {
                2
            };
            (priority, depth(id, root, &nodes), *id)
        });
        let mut occupied = [false; SLOT_COUNT];
        for worker in self.workers.values() {
            occupied[worker.slot] = true;
        }
        for id in order {
            if self.workers.contains_key(id) {
                continue;
            }
            // Where the server seats it, there.
            if let Some((_, slot)) = self.seats.get(id) {
                self.slots.insert(id.to_owned(), *slot);
                continue;
            }
            let remembered = self.slots.get(id).copied().filter(|slot| !occupied[*slot]);
            let parent = nodes
                .get(id)
                .and_then(|n| n.parent_id.as_deref())
                .filter(|parent| *parent != root)
                .and_then(|parent| self.slots.get(parent).copied());
            let slot = remembered.or_else(|| {
                (0..SLOT_COUNT)
                    .filter(|slot| !occupied[*slot])
                    .min_by(|a, b| {
                        // Fill each eight-place tier before spilling into the
                        // next. The first tier alternates sides even for a
                        // chain of children; later tiers favor nearby parents.
                        let tier = |slot: usize| slot / 8;
                        let score = |slot| match parent {
                            Some(parent) if tier(slot) > 0 => {
                                slot_position(slot).distance_squared(slot_position(parent))
                            }
                            _ => slot as f32,
                        };
                        // Of equals, the first and so lowest slot is taken.
                        tier(*a)
                            .cmp(&tier(*b))
                            .then_with(|| score(*a).total_cmp(&score(*b)))
                    })
            });
            let slot = slot.expect("64 workers + 64 departures + 64 absent parents fit exactly");
            occupied[slot] = true;
            self.slots.insert(id.to_owned(), slot);
        }
        // Every node and every parent it names but the root was given a slot above, or kept one as a worker.
        let mut arriving: Vec<(usize, &str, &Node)> = nodes
            .iter()
            .filter(|(id, _)| !self.workers.contains_key(**id))
            .map(|(&id, &node)| (self.slots[id], id, node))
            .collect();
        // The home cluster's gaps close where that moves nobody who would not move anyway, and it lays out for the slots
        // taken now, those arriving with them, placing every live worker again only where that changes how it stands.
        let absent: Vec<usize> = nodes
            .values()
            .filter_map(|node| node.parent_id.as_deref())
            .filter(|parent| {
                *parent != root
                    && !nodes.contains_key(parent)
                    && !self.workers.contains_key(*parent)
            })
            .filter_map(|parent| self.slots.get(parent).copied())
            .collect();
        self.reslot(&mut arriving, &absent);
        if arriving.is_empty() {
            return;
        }
        // Each arriving worker is placed in the order of its slot, told where those already placed stand; a live
        // worker is never placed again here, so that it never moves as others come and go.
        arriving.sort_by_key(|(slot, ..)| *slot);
        // Those standing are told of in the order of their slots, as `arrange` tells of them.
        let mut placed: Vec<Placed<'_>> = self
            .workers
            .iter()
            .filter(|(_, worker)| worker.retired.is_none())
            .map(|(id, worker)| Placed::new(id, worker.slot, worker.state, worker.place))
            .collect();
        placed.sort_by_key(|placed| placed.slot);
        let mut anchors = Vec::with_capacity(arriving.len());
        let mut panicked = false;
        for &(slot, id, node) in &arriving {
            let mut placing = Placing::new(id, slot);
            placing.parent = node.parent_id.as_deref().filter(|parent| *parent != root);
            placing.state = node.state;
            placing.workers = &placed;
            placing.audience = self.audience;
            let at = self.seats.get(id).and_then(|(at, _)| at.as_deref());
            let (spot, panic) = self.spot(at, &placing, panicked);
            panicked |= panic;
            let at = placed.partition_point(|placed| placed.slot < slot);
            placed.insert(at, Placed::new(id, slot, node.state, spot.position));
            anchors.push(spot);
        }
        drop(placed);
        self.give_up(panicked);
        for (&(slot, id, node), &spot) in arriving.iter().zip(&anchors) {
            let anchor = spot.position;
            let parent_id = node.parent_id.as_deref().expect("nodes name their parent");
            // A parent arriving too stands where it was just placed.
            let origin = if parent_id == root {
                CENTER
            } else {
                arriving
                    .iter()
                    .position(|(_, arriving, _)| *arriving == parent_id)
                    .map_or_else(
                        || self.standing(parent_id, self.slots[parent_id]),
                        |index| anchors[index].position,
                    )
            };
            // A cached missing-parent slot preserves provenance, but is not an
            // observed station from which an outbound event can be shown. A
            // worker this update left out has already begun to leave.
            let launch = launch_new && (parent_id == root || nodes.contains_key(parent_id));
            let seed = (stable_hash(id) % 65536) as f32 / 65536.0 * std::f32::consts::TAU;
            let particles = if !launch {
                std::array::from_fn(|i| {
                    if is_terminal(node.state) {
                        terminal_position(anchor, node.state, i)
                    } else {
                        anchor + flock_offset(i, seed, self.time, node.state)
                    }
                })
            } else if parent_id == root {
                std::array::from_fn(|i| {
                    sample(
                        &self.coordinator_samples,
                        self.coordinator_sample_count,
                        id,
                        i,
                    )
                })
            } else {
                [origin; PARTICLES]
            };
            let arrival_seconds = if launch {
                flight_duration(&particles, &[anchor; PARTICLES], ARRIVAL_SECONDS)
            } else {
                ARRIVAL_SECONDS
            };
            let particle_scales = std::array::from_fn(|i| {
                if !launch && node.state == State::Completed {
                    COMPLETED_PIECE
                } else {
                    particle_scale(i)
                }
            });
            self.workers.insert(
                id.to_owned(),
                Worker {
                    slot,
                    parent_id: node.parent_id.clone(),
                    title: node.title.clone(),
                    state: node.state,
                    shape_state: node.state,
                    continuous_evidence: node.state != State::Unknown,
                    #[cfg(test)]
                    origin,
                    anchor,
                    place: anchor,
                    scale: spot.scale,
                    size: self.size_at(self.seats.get(id).and_then(|(at, _)| at.as_deref())),
                    more: spot.more,
                    at: self.seats.get(id).and_then(|(at, _)| at.clone()),
                    particles,
                    particle_scales,
                    flight_start: particles,
                    scale_start: particle_scales,
                    return_end: [CENTER; PARTICLES],
                    color: palette(node.state),
                    age: if launch { 0.0 } else { arrival_seconds },
                    arrival_seconds,
                    flying: launch,
                    terminal_motion: None,
                    retired: None,
                    departure_seconds: DEPARTURE_SECONDS,
                    // Spawn at full particle size on the actual coordinator
                    // samples. Centroid scaling must not collapse those source
                    // positions during an invented arrival fade.
                    visual_opacity: 1.0,
                    seed,
                    orientation: Quat::IDENTITY,
                    orientation_start: Quat::IDENTITY,
                    ambient_time: 0.0,
                },
            );
        }
    }
}

fn depth<'a>(mut id: &'a str, root: &str, nodes: &BTreeMap<&str, &'a Node>) -> usize {
    let mut depth = 0;
    // An absent parent ends the chain.
    while id != root && depth < MAX_NODES {
        let Some(parent) = nodes.get(id).and_then(|node| node.parent_id.as_deref()) else {
            break;
        };
        depth += 1;
        id = parent;
    }
    depth
}

fn stable_hash(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |h, byte| {
        (h ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

/// How quickly a worker placed elsewhere glides there: the rate its distance shrinks at, and its greatest speed, in
/// metres a second, faster than the agent's 1.06 m/s, since its arc about whoever the agent faces is wider.
const GLIDE_RATE: f32 = 2.5;
const GLIDE_SPEED: f32 = 2.0;

/// Where a worker at `from` stands after `dt` seconds gliding to `to`, easing in as it arrives.
fn glide(from: Vec3, to: Vec3, dt: f32) -> Vec3 {
    let step = ((to - from) * (1.0 - (-GLIDE_RATE * dt).exp())).clamp_length_max(GLIDE_SPEED * dt);
    let next = from + step;
    if next.distance(to) < 1e-4 { to } else { next }
}

/// World-anchored polar arc around the viewer's recenter home (0, 0 in XZ), through the agent's default home: the
/// default arc, [`crate::anchors::HomeArc`], about the eyes at home, through [`crate::anchors::Anchor::AGENT`], 2.3 m
/// ahead: its first two rows of five columns to each side of the agent, from 55⅔° and 26° apart, as far apart along the
/// arc as at 3.2 m, either side of the agent's height; and the rest on the arc 3.2 m from home, sixteen columns from 40°
/// and 18⅔° apart, above the band, the three columns that do not fit on it from two rows up, each row 85 cm above the
/// last, so that a sign above a worker never reaches the worker above it. All 192 remembered slots have distinct sightlines, not
/// hidden radial layers. Early slots flank the coordinator; large workspaces require looking around and up. This is
/// fixed scene-home geometry, never a head-locked overlay. [`crate::anchors::HomeArc`] stands the same arc about
/// whoever watches.
pub fn slot_position(slot: usize) -> Vec3 {
    crate::anchors::HomeArc::at(crate::anchors::Anchor::AGENT, crate::anchors::EYES, slot)
}

/// The side of `slot`'s column on the default arc, its column and its row, as [`tomori_protocol::cluster::lane`] says.
#[cfg(test)]
pub(crate) use tomori_protocol::cluster::lane as slot_lane;

/// How far a column of the default arc stands from the agent, as [`tomori_protocol::cluster::column_angle`] says.
#[cfg(test)]
pub(crate) use tomori_protocol::cluster::column_angle as slot_angle;

fn sample(samples: &[Vec3; MAX_COORDINATOR_SAMPLES], count: usize, id: &str, i: usize) -> Vec3 {
    if count == 0 {
        CENTER
    } else {
        samples[(stable_hash(id) as usize).wrapping_add(i) % count]
    }
}

fn flight_arc(i: usize, seed: f32, t: f32, height: f32) -> Vec3 {
    let arc = (std::f32::consts::PI * t).sin();
    Vec3::new(
        (seed + i as f32 * 2.4).sin() * 0.07,
        height + (i % 5) as f32 * 0.015,
        (seed + i as f32 * 2.4).cos() * 0.07,
    ) * arc
}

fn flight_duration(start: &[Vec3; PARTICLES], end: &[Vec3; PARTICLES], minimum: f32) -> f32 {
    let distance = start
        .iter()
        .zip(end)
        .map(|(a, b)| a.distance(*b))
        .fold(0.0_f32, f32::max);
    // Smoothstep's peak derivative is 1.5. Allow room for the arcing path
    // and flock offsets, even for an unusually distant coordinator sample.
    minimum.max((distance + 0.2) * 1.5 / 4.0)
}

/// How large a satellite pulsing for `elapsed` seconds is drawn, as a share of its size: growing to nearly twice it
/// and back over its first half second, then shrinking away by [`PULSE_SECONDS`].
fn pulse(elapsed: f32) -> f32 {
    let grow = 1.0 + 0.9 * (std::f32::consts::PI * (elapsed / 0.5).min(1.0)).sin();
    grow * (1.0 - smooth(((elapsed - 0.5) / (PULSE_SECONDS - 0.5)).clamp(0.0, 1.0)))
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn opacity(worker: &Worker) -> f32 {
    smooth((worker.age / 0.35).min(1.0))
        * (1.0 - smooth((worker.retired.unwrap_or(0.0) / worker.departure_seconds).min(1.0)))
}

fn is_terminal(state: State) -> bool {
    matches!(state, State::Completed | State::Error)
}

fn particle_scale(i: usize) -> Vec3 {
    Vec3::splat(0.024 + (i % 4) as f32 * 0.003)
}

fn solid_completed(worker: &Worker) -> bool {
    worker.shape_state == State::Completed
        && worker.retired.is_none()
        && !worker.flying
        && worker.terminal_motion.is_none()
        && (0..PARTICLES).all(|i| {
            worker.particles[i] == terminal_goal(worker, i)
                && worker.particle_scales[i] == COMPLETED_PIECE
        })
}

fn terminal_goal(worker: &Worker, i: usize) -> Vec3 {
    let base = terminal_position(worker.anchor, worker.shape_state, i);
    if worker.shape_state == State::Completed {
        worker.anchor + worker.orientation * (base - worker.anchor)
    } else {
        // Opposite fragments have opposite displacements: no mean drift and
        // no closing of the tangential fracture. Period is about ten seconds.
        let side = if i < 12 { -1.0 } else { 1.0 };
        let wave = (worker.ambient_time * 0.65) as f32;
        base + Vec3::Y * (side * 0.012 * wave.sin() * ((i % 12) as f32 * 0.4).cos())
    }
}

/// A fitted 4×3×2 axis-aligned partition, or two separated, offset
/// fragments. Error uses station-tangential X to keep its fracture visible.
fn terminal_position(anchor: Vec3, state: State, i: usize) -> Vec3 {
    if state == State::Completed {
        return anchor
            + Vec3::new(
                (i / 6) as f32 * COMPLETED_PIECE.x + COMPLETED_PIECE.x * 0.5 - COMPLETED_SIDE * 0.5,
                ((i % 6) / 2) as f32 * COMPLETED_PIECE.y + COMPLETED_PIECE.y * 0.5
                    - COMPLETED_SIDE * 0.5,
                (i % 2) as f32 * COMPLETED_PIECE.z + COMPLETED_PIECE.z * 0.5 - COMPLETED_SIDE * 0.5,
            );
    }
    let local = {
        let side = if i < 12 { -1.0 } else { 1.0 };
        let j = i % 12;
        let x = side * if j / 6 == 0 { 0.15 } else { 0.065 };
        let y = ((j % 6) / 2) as f32 * 0.09 - 0.09 + side * 0.025 + x * 0.12;
        let z = (j % 2) as f32 * 0.11 - 0.055 + side * 0.025;
        Vec3::new(x, y, z)
    };
    // A worker placed straight above or below home faces home's front.
    let radial = Vec3::new(anchor.x, 0.0, anchor.z)
        .try_normalize()
        .unwrap_or(Vec3::NEG_Z);
    let tangent = Vec3::new(-radial.z, 0.0, radial.x);
    anchor + tangent * local.x + Vec3::Y * local.y + radial * local.z
}

/// Whether a worker in `state` is still at work, as far as anyone knows: running, waiting, or stalled or unanswered,
/// which have not ended it.
fn is_working(state: State) -> bool {
    matches!(
        state,
        State::Running | State::Waiting | State::Stalled | State::Unanswered
    )
}

fn palette(state: State) -> Vec3 {
    match state {
        State::Running => Vec3::new(0.22, 0.72, 0.95),
        State::Waiting => Vec3::new(0.95, 0.68, 0.25),
        State::Idle => Vec3::new(0.35, 0.65, 0.65),
        State::Completed => Vec3::new(0.30, 0.85, 0.52),
        State::Error => Vec3::new(1.00, 0.25, 0.20),
        State::Stopped => Vec3::new(0.62, 0.52, 0.42),
        // Amber, deeper than waiting's: something may be wrong.
        State::Stalled | State::Unanswered => Vec3::new(1.00, 0.55, 0.10),
        State::Unknown => Vec3::new(0.50, 0.55, 0.65),
    }
}

fn flock_offset(i: usize, seed: f32, time: f64, state: State) -> Vec3 {
    let (shape, speed) = match state {
        State::Running => (Vec3::new(0.13, 0.13, 0.11), 0.85),
        State::Waiting => (Vec3::new(0.16, 0.065, 0.10), 0.22),
        State::Idle => (Vec3::new(0.12, 0.09, 0.09), 0.15),
        State::Completed | State::Error => {
            unreachable!("a terminal worker gathers into its terminal shape instead")
        }
        State::Stopped => (Vec3::new(0.11, 0.08, 0.09), 0.05),
        State::Stalled | State::Unanswered => (Vec3::new(0.12, 0.08, 0.10), 0.04),
        State::Unknown => (Vec3::new(0.13, 0.10, 0.13), 0.12),
    };
    let y = 1.0 - 2.0 * (i as f32 + 0.5) / PARTICLES as f32;
    let radius = (1.0 - y * y).sqrt();
    let phase = (time * speed) as f32 + seed;
    let angle = i as f32 * 2.399_963_1 + phase;
    let fill = 0.55 + (i % 5) as f32 * 0.11;
    // Filled, tumbling little clouds, not planar agent rings. The small wobble
    // adds organic motion without an O(n^2) boids simulation per worker.
    (Vec3::new(radius * angle.cos(), y, radius * angle.sin()) * fill
        + Vec3::new(
            (phase * 1.3 + y * 5.0).sin(),
            (phase + angle).cos(),
            (phase * 0.7 + y).sin(),
        ) * 0.10)
        * shape
}

fn push_cube(cubes: &mut Vec<Cube>, position: Vec3, scale: Vec3, color: Vec3, orientation: Quat) {
    if cubes.len() >= MAX_CUBES || scale.min_element() <= 0.00001 {
        return;
    }
    let cube = Cube {
        position: position.to_array(),
        scale: scale.to_array(),
        color: color.to_array(),
        orientation: orientation.to_array(),
    };
    if cube.valid() {
        cubes.push(cube);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod space_tests;

#[cfg(test)]
mod clusters;
