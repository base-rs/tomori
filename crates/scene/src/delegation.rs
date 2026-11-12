// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Bounded worker geometry. Positions belong to this client, never to the bridge.
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

/// Session-local layout and animation; no bridge mutation or progress estimates.
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
            .then_some(tomori_protocol::entourage::FLOOR + tomori_protocol::cluster::RING_HIGHEST);
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
        self.away = away.filter(|(centre, reach)| {
            centre.is_finite() && centre.abs().max_element() <= 90.0 && reach.is_finite()
        });
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
            if position.is_finite() && position.abs().max_element() <= 90.0 {
                self.coordinator_samples[self.coordinator_sample_count] = position;
                self.coordinator_sample_count += 1;
            }
        }
    }

    /// `workspace` must be the bridge projection for `root`. Its contract has
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
mod tests {
    use super::*;

    fn node(id: &str, parent: &str) -> Node {
        Node {
            id: id.into(),
            parent_id: Some(parent.into()),
            title: id.into(),
            state: State::Running,
        }
    }

    fn workspace(nodes: Vec<Node>) -> Workspace {
        Workspace {
            nodes,
            ..Default::default()
        }
    }

    fn anchors(scene: &Scene) -> BTreeMap<String, [f32; 3]> {
        scene
            .stations()
            .iter()
            .map(|s| (s.id.clone(), s.position))
            .collect()
    }

    fn settle(scene: &mut Scene, workspace: &Workspace) {
        for _ in 0..150 {
            scene.update(Some("root"), workspace, 1.0 / 60.0);
        }
    }

    #[test]
    fn a_partial_update_adds_no_worker_beyond_the_limit() {
        let mut scene = Scene::new();
        let full = workspace(
            (0..MAX_NODES)
                .map(|i| node(&format!("w{i}"), "root"))
                .collect(),
        );
        settle(&mut scene, &full);
        let partial = Workspace {
            truncated: true,
            ..workspace(vec![node("late", "root")])
        };
        scene.update(Some("root"), &partial, 1.0 / 60.0);
        assert!(!scene.workers.contains_key("late"));
        assert_eq!(scene.stations().len(), MAX_NODES);
    }

    #[test]
    fn a_cube_that_is_not_valid_geometry_is_not_drawn() {
        let mut cubes = Vec::new();
        for position in [Vec3::NAN, Vec3::splat(1000.0)] {
            push_cube(
                &mut cubes,
                position,
                Vec3::splat(0.1),
                Vec3::ONE,
                Quat::IDENTITY,
            );
        }
        assert!(cubes.is_empty());
        push_cube(
            &mut cubes,
            Vec3::ZERO,
            Vec3::splat(0.1),
            Vec3::ONE,
            Quat::IDENTITY,
        );
        assert_eq!(cubes.len(), 1);
    }

    fn assert_terminal_form(worker: &Worker) {
        assert!(!worker.flying);
        assert!(worker.terminal_motion.is_none());
        for i in 0..PARTICLES {
            let goal = terminal_goal(worker, i);
            assert!(worker.particles[i].distance(goal) < 0.001);
        }
        let center = worker.particles.iter().sum::<Vec3>() / PARTICLES as f32;
        assert!(
            center.distance(worker.anchor) < 0.00001,
            "terminal form drifted"
        );
    }

    // Used when every worker is visible enough to emit its full geometry.
    fn assert_only_worker_flocks(scene: &Scene) {
        let mut cubes = scene.cubes().iter();
        for worker in scene.workers.values() {
            if solid_completed(worker) {
                let cube = cubes.next().unwrap();
                assert_eq!(cube.position, worker.anchor.to_array());
                assert_eq!(cube.scale, [COMPLETED_SIDE * worker.visual_opacity; 3]);
            } else {
                let center = worker.particles.iter().copied().sum::<Vec3>() / PARTICLES as f32;
                for (position, scale) in worker.particles.iter().zip(worker.particle_scales) {
                    let cube = cubes.next().unwrap();
                    assert_eq!(
                        cube.position,
                        if worker.visual_opacity == 1.0 {
                            position.to_array()
                        } else {
                            (center + (*position - center) * worker.visual_opacity).to_array()
                        }
                    );
                    assert_eq!(cube.scale, (scale * worker.visual_opacity).to_array());
                }
            }
        }
        assert!(cubes.next().is_none());
    }

    // Compare rendered volumes across the one-cube / fitted-pieces boundary,
    // not just the hidden particle centers. A solid has the same 24 volumes.
    fn rendered_volumes(scene: &Scene) -> [(Vec3, Vec3); PARTICLES] {
        match scene.cubes() {
            [solid] => {
                let center = Vec3::from_array(solid.position);
                let side = Vec3::from_array(solid.scale);
                std::array::from_fn(|i| {
                    let offset = Quat::from_array(solid.orientation)
                        * terminal_position(Vec3::ZERO, State::Completed, i);
                    (
                        center + offset * side / COMPLETED_SIDE,
                        side / Vec3::new(4.0, 3.0, 2.0),
                    )
                })
            }
            pieces => {
                assert_eq!(pieces.len(), PARTICLES);
                std::array::from_fn(|i| {
                    (
                        Vec3::from_array(pieces[i].position),
                        Vec3::from_array(pieces[i].scale),
                    )
                })
            }
        }
    }

    fn assert_volume_step(
        before: &[(Vec3, Vec3); PARTICLES],
        scene: &Scene,
        position_limit: f32,
        scale_limit: f32,
    ) {
        for ((position, scale), (next_position, next_scale)) in
            before.iter().zip(rendered_volumes(scene))
        {
            assert!(
                position.distance(next_position) <= position_limit,
                "rendered position jumped: {position:?} -> {next_position:?}"
            );
            assert!(
                scale.distance(next_scale) <= scale_limit,
                "rendered scale jumped: {scale:?} -> {next_scale:?}"
            );
        }
    }

    fn assert_fitted_partition(pieces: &[Cube], solid: Cube) {
        assert_eq!(pieces.len(), PARTICLES);
        let center = Vec3::from_array(solid.position);
        let side = Vec3::from_array(solid.scale);
        let inverse = Quat::from_array(solid.orientation).inverse();
        let min = -side * 0.5;
        let max = side * 0.5;
        let mut actual_min = Vec3::splat(f32::INFINITY);
        let mut actual_max = Vec3::splat(f32::NEG_INFINITY);
        let mut volume = 0.0;
        for (i, piece) in pieces.iter().enumerate() {
            assert!(piece.valid());
            let position = inverse * (Vec3::from_array(piece.position) - center);
            assert!(
                Quat::from_array(piece.orientation)
                    .angle_between(Quat::from_array(solid.orientation))
                    < 0.001
            );
            let scale = Vec3::from_array(piece.scale);
            assert!(scale.distance(side / Vec3::new(4.0, 3.0, 2.0)) < 0.000001);
            let lo = position - scale * 0.5;
            let hi = position + scale * 0.5;
            assert!((lo - min).min_element() > -0.000001);
            assert!((max - hi).min_element() > -0.000001);
            actual_min = actual_min.min(lo);
            actual_max = actual_max.max(hi);
            volume += scale.x * scale.y * scale.z;
            for other in &pieces[i + 1..] {
                let other_position = inverse * (Vec3::from_array(other.position) - center);
                let other_scale = Vec3::from_array(other.scale);
                let overlap = hi.min(other_position + other_scale * 0.5)
                    - lo.max(other_position - other_scale * 0.5);
                assert!(
                    overlap.min_element() < 0.000001,
                    "overlapping partition pieces"
                );
            }
        }
        assert!(actual_min.distance(min) < 0.000001);
        assert!(actual_max.distance(max) < 0.000001);
        // Contained, non-overlapping volumes whose sum equals the solid leave
        // no holes. Matching outer bounds alone would miss scale-only shrinkage.
        assert!((volume - side.x * side.y * side.z).abs() < 0.00000001);
    }

    #[test]
    fn rotated_solid_split_and_all_state_changes_preserve_rendered_orientations() {
        for to in [
            State::Completed,
            State::Error,
            State::Running,
            State::Unknown,
        ] {
            let mut scene = Scene::new();
            let mut w = workspace(vec![Node {
                state: State::Completed,
                ..node("a", "root")
            }]);
            // More than a minute of rotation exercises a far-from-identity
            // partition, not just an almost-axis-aligned completed cube.
            for _ in 0..1500 {
                scene.update(Some("root"), &w, 0.05);
            }
            assert_eq!(scene.cubes().len(), 1);
            let solid = scene.cubes()[0];
            let rotation = Quat::from_array(solid.orientation);
            assert!(rotation.angle_between(Quat::IDENTITY) > 0.3);
            let volumes = rendered_volumes(&scene);
            w.nodes[0].state = to;
            scene.update(Some("root"), &w, 0.0);
            assert_volume_step(&volumes, &scene, 0.000001, 0.000001);
            assert!(
                scene
                    .cubes()
                    .iter()
                    .all(|cube| cube.orientation == solid.orientation)
            );
            for _ in 0..120 {
                let before = Quat::from_array(scene.cubes()[0].orientation);
                let volumes = rendered_volumes(&scene);
                scene.update(Some("root"), &w, 1.0 / 60.0);
                assert!(
                    Quat::from_array(scene.cubes()[0].orientation).angle_between(before) < 0.11
                );
                assert_volume_step(&volumes, &scene, 0.101, 0.025);
            }
        }
        let mut scene = Scene::new();
        let w = workspace(vec![Node {
            state: State::Completed,
            ..node("a", "root")
        }]);
        for _ in 0..1500 {
            scene.update(Some("root"), &w, 0.05);
        }
        let solid = scene.cubes()[0];
        scene.update(Some("root"), &Workspace::default(), 0.0);
        assert_fitted_partition(scene.cubes(), solid);
        assert!(
            scene
                .cubes()
                .iter()
                .all(|c| c.orientation == solid.orientation)
        );
        for _ in 0..10 {
            scene.update(Some("root"), &Workspace::default(), 1.0 / 60.0);
        }
        let before: Vec<_> = scene.cubes().iter().map(|c| c.orientation).collect();
        let volumes = rendered_volumes(&scene);
        scene.update(Some("root"), &w, 0.0);
        assert_volume_step(&volumes, &scene, 0.000001, 0.000001);
        assert_eq!(
            scene
                .cubes()
                .iter()
                .map(|c| c.orientation)
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn error_wave_is_small_slow_zero_mean_and_does_not_close_the_fracture() {
        let mut scene = Scene::new();
        let w = workspace(vec![Node {
            state: State::Error,
            ..node("a", "root")
        }]);
        scene.update(Some("root"), &w, 0.0);
        let initial = scene.workers["a"].particles;
        let mut maximum = 0.0_f32;
        for frame in 0..1200 {
            let before = scene.workers["a"].particles;
            let evidence = if frame % 3 == 0 {
                Workspace {
                    stale: true,
                    ..w.clone()
                }
            } else {
                w.clone()
            };
            scene.update(Some("root"), &evidence, 1.0 / 60.0);
            let worker = &scene.workers["a"];
            assert_terminal_form(worker);
            let radial = Vec3::new(worker.anchor.x, 0.0, worker.anchor.z).normalize();
            let tangent = Vec3::new(-radial.z, 0.0, radial.x);
            for (i, position) in worker.particles.iter().enumerate() {
                let displacement = position.distance(initial[i]);
                maximum = maximum.max(displacement);
                assert!(displacement <= 0.01201);
                assert!(position.distance(before[i]) < 0.00015);
                let x = (*position - worker.anchor).dot(tangent);
                assert!(if i < 12 { x < -0.06 } else { x > 0.06 });
            }
        }
        assert!(maximum > 0.01, "the fracture must actually wave");
    }

    #[test]
    fn cube_quaternions_are_defaulted_bounded_normalized_and_roundtrip() {
        let legacy: Cube = serde_json::from_str(
            r#"{"position":[0,1,-2],"scale":[0.24,0.24,0.24],"color":[0.3,0.8,0.5]}"#,
        )
        .unwrap();
        assert_eq!(legacy.orientation, Quat::IDENTITY.to_array());
        assert!(legacy.valid());
        let rotated = Cube {
            orientation: Quat::from_rotation_y(1.2).to_array(),
            ..legacy
        };
        let decoded: Cube =
            serde_json::from_str(&serde_json::to_string(&rotated).unwrap()).unwrap();
        assert_eq!(rotated.orientation, decoded.orientation);
        assert!(decoded.valid());
        for orientation in [
            [0.0; 4],
            [1.0; 4],
            [f32::NAN, 0.0, 0.0, 1.0],
            [f32::INFINITY, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 0.9],
            [0.0, 0.0, 0.0, 1.01],
        ] {
            assert!(
                !Cube {
                    orientation,
                    ..legacy
                }
                .valid()
            );
        }
    }

    #[test]
    fn completed_emits_one_solid_cube_after_history_arrival_or_observed_settle() {
        for discovery in 0..3 {
            let mut scene = Scene::new();
            let mut w = workspace(vec![Node {
                state: State::Completed,
                ..node("a", "root")
            }]);
            if discovery == 1 {
                scene.update(Some("root"), &Workspace::default(), 0.0);
            } else if discovery == 2 {
                w.nodes[0].state = State::Running;
                settle(&mut scene, &w);
                w.nodes[0].state = State::Completed;
            }
            scene.update(Some("root"), &w, 0.0);
            assert_eq!(scene.workers["a"].flying, discovery == 1);
            assert_eq!(scene.workers["a"].terminal_motion.is_some(), discovery == 2);
            settle(&mut scene, &w);
            assert!(solid_completed(&scene.workers["a"]));
            assert_eq!(scene.cubes().len(), 1, "discovery {discovery}");
            let solid = scene.cubes()[0];
            assert_eq!(solid.position, scene.workers["a"].anchor.to_array());
            assert_eq!(solid.scale, [COMPLETED_SIDE; 3]);
            assert!(Vec3::from_array(solid.color).distance(palette(State::Completed)) < 0.000001);
            for _ in 0..120 {
                scene.update(Some("root"), &w, 1.0 / 60.0);
                assert_eq!(scene.cubes().len(), 1);
                assert_eq!(scene.cubes()[0].position, solid.position);
                assert_eq!(scene.cubes()[0].scale, solid.scale);
            }
        }
    }

    #[test]
    fn solid_return_onset_exactly_tiles_full_and_uncertainty_faded_bounds() {
        for faded in [false, true] {
            let mut scene = Scene::new();
            let w = workspace(vec![Node {
                state: State::Completed,
                ..node("a", "root")
            }]);
            settle(&mut scene, &w);
            if faded {
                let stale = Workspace {
                    stale: true,
                    ..w.clone()
                };
                for _ in 0..20 {
                    scene.update(Some("root"), &stale, 0.05);
                }
                assert_eq!(scene.workers["a"].visual_opacity, 0.55);
            }
            assert_eq!(scene.cubes().len(), 1);
            let solid = scene.cubes()[0];
            let before = rendered_volumes(&scene);
            scene.update(Some("root"), &Workspace::default(), 0.0);
            assert_eq!(scene.workers["a"].retired, Some(0.0));
            assert_fitted_partition(scene.cubes(), solid);
            assert_volume_step(&before, &scene, 0.000001, 0.000001);
        }
    }

    #[test]
    fn completed_departure_emits_24_visible_pieces_smoothly_toward_captured_samples() {
        let mut scene = Scene::new();
        let w = workspace(vec![Node {
            state: State::Completed,
            ..node("a", "root")
        }]);
        settle(&mut scene, &w);
        let samples: Vec<_> = (0..PARTICLES)
            .map(|i| (CENTER + Vec3::new(i as f32 * 0.004, 0.0, -0.1)).to_array())
            .collect();
        scene.set_coordinator_samples(&samples);
        let start = scene.workers["a"].particles;
        let before = rendered_volumes(&scene);
        scene.update(Some("root"), &Workspace::default(), 0.0);
        assert_volume_step(&before, &scene, 0.000001, 0.000001);
        let targets = scene.workers["a"].return_end;
        assert!(targets.iter().all(|p| samples.contains(&p.to_array())));
        for (i, target) in targets.iter().enumerate() {
            assert!(!targets[i + 1..].contains(target));
        }
        // Moving coordinator samples do not teleport an in-flight destination.
        scene.set_coordinator_samples(&[[5.0, 5.0, 5.0]]);
        for frame in 1..=60 {
            let before = rendered_volumes(&scene);
            scene.update(Some("root"), &Workspace::default(), 1.0 / 60.0);
            assert_eq!(scene.cubes().len(), PARTICLES, "frame {frame}");
            assert!(scene.cubes().iter().all(Cube::valid));
            assert_volume_step(&before, &scene, 0.101, 0.025);
            assert_eq!(scene.workers["a"].return_end, targets);
            if frame == 30 {
                assert!(
                    scene
                        .cubes()
                        .iter()
                        .all(|cube| cube.scale.iter().all(|s| *s >= 0.023))
                );
                let worker = &scene.workers["a"];
                for i in 0..PARTICLES {
                    assert!(
                        worker.particles[i].distance(targets[i])
                            < start[i].distance(targets[i]) * 0.8
                    );
                    assert_eq!(worker.particle_scales[i], particle_scale(i));
                }
            }
        }
        assert!(
            scene.workers["a"]
                .particles
                .iter()
                .zip(targets)
                .all(|(p, target)| p.distance(target) < 0.4)
        );
        for _ in 0..30 {
            scene.update(Some("root"), &Workspace::default(), 1.0 / 60.0);
        }
        assert!(scene.workers.is_empty());
        assert!(scene.cubes().is_empty());
    }

    #[test]
    fn new_worker_renders_24_pieces_at_actual_samples_without_centroid_collapse() {
        let mut scene = Scene::new();
        scene.update(Some("root"), &Workspace::default(), 0.0);
        let samples: Vec<_> = (0..PARTICLES)
            .map(|i| (CENTER + Vec3::new(i as f32 * 0.03 - 0.35, 0.0, 0.0)).to_array())
            .collect();
        scene.set_coordinator_samples(&samples);
        let w = workspace(vec![node("a", "root")]);
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.cubes().len(), PARTICLES);
        for (i, cube) in scene.cubes().iter().enumerate() {
            assert!(samples.contains(&cube.position));
            assert_eq!(cube.scale, particle_scale(i).to_array());
        }
        for _ in 0..90 {
            let before = rendered_volumes(&scene);
            scene.update(Some("root"), &w, 1.0 / 60.0);
            assert_eq!(scene.cubes().len(), PARTICLES);
            assert_volume_step(&before, &scene, 0.101, 0.000001);
            assert!(scene.cubes().iter().all(Cube::valid));
            // Rendered centers, not only hidden simulation positions, follow
            // the bounded path starting at the actual coordinator samples.
            for (cube, position) in scene.cubes().iter().zip(scene.workers["a"].particles) {
                assert_eq!(cube.position, position.to_array());
            }
        }
    }

    #[test]
    fn rendered_positions_and_scales_are_continuous_across_every_state_pair() {
        let states = [
            State::Running,
            State::Waiting,
            State::Idle,
            State::Completed,
            State::Error,
            State::Stopped,
            State::Unknown,
        ];
        for from in states {
            for to in states {
                let mut scene = Scene::new();
                let mut w = workspace(vec![Node {
                    state: from,
                    ..node("a", "root")
                }]);
                settle(&mut scene, &w);
                let before = rendered_volumes(&scene);
                w.nodes[0].state = to;
                scene.update(Some("root"), &w, 0.0);
                assert_volume_step(&before, &scene, 0.000001, 0.000001);
                for _ in 0..180 {
                    let before = rendered_volumes(&scene);
                    scene.update(Some("root"), &w, 1.0 / 60.0);
                    assert_volume_step(&before, &scene, 0.101, 0.025);
                    assert!(scene.cubes().iter().all(Cube::valid));
                }
                assert_eq!(
                    scene.cubes().len(),
                    if to == State::Completed { 1 } else { PARTICLES }
                );
                assert_eq!(scene.stations()[0].state, to);
            }
        }
    }

    #[test]
    fn solid_resurrection_retains_rendered_scale_and_position_even_during_late_fade() {
        for return_frames in [0, 12, 60, 68] {
            for resumed_state in [
                State::Completed,
                State::Running,
                State::Error,
                State::Unknown,
            ] {
                let mut scene = Scene::new();
                let mut w = workspace(vec![Node {
                    state: State::Completed,
                    ..node("a", "root")
                }]);
                settle(&mut scene, &w);
                scene.update(Some("root"), &Workspace::default(), 0.0);
                for _ in 0..return_frames {
                    scene.update(Some("root"), &Workspace::default(), 1.0 / 60.0);
                }
                let before = rendered_volumes(&scene);
                w.nodes[0].state = resumed_state;
                scene.update(Some("root"), &w, 0.0);
                assert!(scene.workers["a"].retired.is_none());
                assert!(
                    scene.workers["a"]
                        .terminal_motion
                        .is_none_or(|motion| !motion.bounce)
                );
                assert_volume_step(&before, &scene, 0.000001, 0.000001);
                for _ in 0..180 {
                    let before = rendered_volumes(&scene);
                    scene.update(Some("root"), &w, 1.0 / 60.0);
                    // Up to 0.1 m particle motion plus the bounded expansion
                    // about the centroid when a faded return is resurrected.
                    assert_volume_step(&before, &scene, 0.12, 0.025);
                    assert!(scene.cubes().iter().all(Cube::valid));
                }
                assert_eq!(
                    scene.cubes().len(),
                    if resumed_state == State::Completed {
                        1
                    } else {
                        PARTICLES
                    }
                );
            }
        }
    }

    #[test]
    fn terminal_history_metadata_and_resumed_evidence_do_not_invent_launches() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![Node {
            state: State::Completed,
            ..node("history", "root")
        }]);
        scene.set_coordinator_samples(&[[0.4, 1.5, -2.4]]);
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.cubes().len(), 1);
        let position = scene.cubes()[0].position;
        w.presentations.push(Presentation {
            id: "historical-card".into(),
            source_id: "history".into(),
            source_message_id: "old-message".into(),
            ..Default::default()
        });
        w.nodes[0].title = "Historical result".into();
        scene.update(Some("root"), &w, 1000.0);
        assert_eq!(scene.cubes().len(), 1);
        assert_eq!(scene.cubes()[0].position, position);
        assert!(!scene.workers["history"].flying);
        assert!(scene.workers["history"].terminal_motion.is_none());
        for uncertain in [
            Workspace {
                stale: true,
                ..w.clone()
            },
            Workspace {
                truncated: true,
                ..Default::default()
            },
            Workspace {
                error: Some("offline".into()),
                ..Default::default()
            },
        ] {
            scene.update(Some("root"), &uncertain, 0.05);
            assert!(scene.workers["history"].retired.is_none());
            w.nodes.push(Node {
                state: State::Completed,
                ..node(&format!("recovered-{}", w.nodes.len()), "root")
            });
            scene.update(Some("root"), &w, 0.0);
            assert_eq!(scene.cubes().len(), w.nodes.len());
            assert!(scene.workers.values().all(|worker| !worker.flying
                && worker.terminal_motion.is_none()
                && worker.retired.is_none()));
        }
        // Only a genuinely new observation after continuous trusted evidence
        // launches 24 particles, each from an actual sampled coordinator cube.
        w.nodes.push(node("new", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert!(scene.workers["new"].flying);
        assert_eq!(
            scene.workers["new"].particles,
            [Vec3::new(0.4, 1.5, -2.4); PARTICLES]
        );
        for _ in 0..12 {
            scene.update(Some("root"), &w, 1.0 / 60.0);
        }
        assert_eq!(scene.cubes().len(), w.nodes.len() - 1 + PARTICLES);
        assert_only_worker_flocks(&scene);
    }

    #[test]
    fn resurrecting_near_the_end_of_split_does_not_snap_scale_at_zero_dt() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![Node {
            state: State::Completed,
            ..node("a", "root")
        }]);
        settle(&mut scene, &w);
        scene.update(Some("root"), &Workspace::default(), 0.0);
        for _ in 0..7 {
            scene.update(
                Some("root"),
                &Workspace::default(),
                (SPLIT_SECONDS - 0.001) / 7.0,
            );
        }
        let before = scene.workers["a"].particle_scales;
        assert!(before[0].distance(particle_scale(0)) > 0.0);
        assert!(before[0].distance(particle_scale(0)) < 0.00001);
        w.nodes[0].state = State::Running;
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["a"].particle_scales, before);
    }

    #[test]
    fn serde_defaults_and_wire_names() {
        let empty: Workspace = serde_json::from_str("{}").unwrap();
        assert!(empty.valid());
        assert!(!empty.stale); // Older bridges did not send this field.
        let n: Node = serde_json::from_str(r#"{"id":"n","parent_id":"root"}"#).unwrap();
        assert_eq!(n.state, State::Unknown);
        assert_eq!(
            serde_json::to_string(&State::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&Kind::Markdown).unwrap(),
            "\"markdown\""
        );
        assert!(serde_json::from_str::<State>("\"invented\"").is_err());
    }

    #[test]
    fn validates_ids_cycles_and_missing_parents() {
        assert!(workspace(vec![node("a", "absent")]).valid());
        assert!(!workspace(vec![node("a", "root"), node("a", "root")]).valid());
        assert!(!workspace(vec![node("a", "a")]).valid());
        assert!(!workspace(vec![node("a", "b"), node("b", "a")]).valid());
        assert!(!workspace(vec![node("", "root")]).valid());
        assert!(!workspace(vec![node(&"x".repeat(257), "root")]).valid());
        let mut w = workspace((0..65).map(|i| node(&i.to_string(), "root")).collect());
        assert!(!w.valid());
        w.nodes.pop();
        assert!(w.valid());
        w.nodes[0].title = "🦀".repeat(160);
        assert!(w.valid());
        w.nodes[0].title.push('x');
        assert!(!w.valid());
    }

    #[test]
    fn validates_bounded_rectangular_presentations() {
        let card = Presentation {
            id: "card".into(),
            source_id: "worker".into(),
            kind: Kind::Table,
            columns: vec!["one".into(), "two".into()],
            rows: vec![vec!["a".into(), "b".into()]],
            ..Default::default()
        };
        let mut w = Workspace {
            presentations: vec![card.clone()],
            ..Default::default()
        };
        assert!(w.valid());
        w.presentations[0].rows[0].pop();
        assert!(!w.valid());
        w.presentations[0] = card.clone();
        w.presentations[0].text = "é".repeat(2049);
        assert!(!w.valid());
        w.presentations[0] = card.clone();
        w.presentations[0].columns[0] = "x".repeat(81);
        assert!(!w.valid());
        w.presentations[0] = card.clone();
        w.presentations[0].rows[0][0] = "x".repeat(161);
        assert!(!w.valid());
        w.presentations = vec![card.clone(), card.clone()];
        assert!(!w.valid());
        w.presentations = (0..17)
            .map(|i| Presentation {
                id: i.to_string(),
                ..card.clone()
            })
            .collect();
        assert!(!w.valid());
    }

    #[test]
    fn cube_validation_rejects_bad_geometry() {
        let cube = Cube {
            position: CENTER.to_array(),
            scale: [0.02; 3],
            color: [0.5; 3],
            orientation: Quat::IDENTITY.to_array(),
        };
        assert!(cube.valid());
        for bad in [f32::NAN, f32::INFINITY, -f32::INFINITY, 101.0] {
            assert!(
                !Cube {
                    position: [bad, 1.0, 0.0],
                    ..cube
                }
                .valid()
            );
        }
        for bad in [f32::NAN, -0.1, 0.0, 1.1] {
            assert!(
                !Cube {
                    scale: [bad; 3],
                    ..cube
                }
                .valid()
            );
        }
        assert!(
            !Cube {
                color: [1.1; 3],
                ..cube
            }
            .valid()
        );
        assert!(
            !Cube {
                color: [f32::NAN; 3],
                ..cube
            }
            .valid()
        );
    }

    #[test]
    fn station_wire_roundtrip_and_validation() {
        let station = Station {
            facing: HOME_EYES,
            id: "worker".into(),
            position: [1.0, 2.0, -3.0],
            state: State::Waiting,
            title: "A worker".into(),
            opacity: 0.4,
            scale: 1.0,
            more: 0,
            size: None,
            highlighted: false,
        };
        assert!(station.valid());
        let encoded = serde_json::to_string(&station).unwrap();
        let decoded: Station = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.valid());
        assert_eq!(decoded.id, station.id);
        assert_eq!(decoded.opacity, station.opacity);
        let old: Station = serde_json::from_str(
            r#"{"id":"w","position":[0,1,-2],"state":"running","title":"Worker"}"#,
        )
        .unwrap();
        assert_eq!(old.opacity, 1.0);
        assert_eq!((old.scale, old.more), (1.0, 0));
        assert!(old.valid());
        assert!(!encoded.contains("more"));
        // A compact sign, and one for those gathered into a cluster's "+N more", are kept, within bounds.
        let gathered = Station {
            scale: tomori_protocol::cluster::COMPACT,
            more: 3,
            ..station.clone()
        };
        let read: Station =
            serde_json::from_str(&serde_json::to_string(&gathered).unwrap()).unwrap();
        assert_eq!((read.scale, read.more), (gathered.scale, 3));
        for (scale, more) in [(0.05, 0), (f32::NAN, 0), (1.2, 0), (1.0, MAX_WORKERS + 1)] {
            assert!(
                !Station {
                    scale,
                    more,
                    ..station.clone()
                }
                .valid()
            );
        }
        for bad in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(
                !Station {
                    facing: HOME_EYES,
                    opacity: bad,
                    ..station.clone()
                }
                .valid()
            );
        }
        assert!(
            !Station {
                facing: HOME_EYES,
                position: [f32::NAN, 0.0, 0.0],
                ..station.clone()
            }
            .valid()
        );
        assert!(
            !Station {
                facing: HOME_EYES,
                id: "x".repeat(161),
                ..station.clone()
            }
            .valid()
        );
        assert!(
            !Station {
                facing: HOME_EYES,
                title: "x".repeat(161),
                ..station
            }
            .valid()
        );
    }

    #[test]
    fn parentless_root_and_unrelated_entries_are_not_workers() {
        let mut scene = Scene::new();
        let w = workspace(vec![
            Node {
                id: "root".into(),
                ..Default::default()
            },
            Node {
                id: "unrelated".into(),
                ..Default::default()
            },
            node("child", "root"),
        ]);
        scene.update(Some("root"), &w, 0.02);
        assert_eq!(scene.stations().len(), 1);
        assert_eq!(scene.stations()[0].id, "child");
        scene.update(None, &w, 0.02);
        assert!(scene.stations().is_empty());
        assert!(scene.cubes().is_empty());
    }

    #[test]
    fn worker_supplied_positions_are_ignored() {
        let a: Workspace = serde_json::from_str(r#"{"nodes":[{"id":"a","parent_id":"root","title":"A","state":"running","position":[99,99,99],"xyz":[-80,80,80]}]}"#).unwrap();
        let b = workspace(vec![Node {
            title: "A".into(),
            ..node("a", "root")
        }]);
        let mut one = Scene::new();
        let mut two = Scene::new();
        settle(&mut one, &a);
        settle(&mut two, &b);
        assert_eq!(anchors(&one), anchors(&two));
        assert_eq!(
            serde_json::to_string(one.cubes()).unwrap(),
            serde_json::to_string(two.cubes()).unwrap()
        );
    }

    #[test]
    fn initial_order_and_animation_are_deterministic() {
        let a = workspace(vec![node("c", "b"), node("b", "root"), node("a", "root")]);
        let b = workspace(a.nodes.iter().cloned().rev().collect());
        let mut one = Scene::new();
        let mut two = Scene::default();
        settle(&mut one, &a);
        settle(&mut two, &b);
        assert_eq!(anchors(&one), anchors(&two));
        assert_eq!(
            serde_json::to_string(one.cubes()).unwrap(),
            serde_json::to_string(two.cubes()).unwrap()
        );
        assert_eq!(one.snapshot().len(), one.cubes().len());
    }

    #[test]
    fn surviving_anchors_do_not_move_on_add_remove_reorder_or_state() {
        let mut w = workspace(vec![node("b", "root"), node("c", "b")]);
        let mut scene = Scene::new();
        settle(&mut scene, &w);
        let before = anchors(&scene);
        w.nodes.reverse();
        w.nodes.push(node("a", "root"));
        w.nodes[0].state = State::Completed;
        scene.update(Some("root"), &w, 1.0 / 60.0);
        for id in ["b", "c"] {
            assert_eq!(anchors(&scene)[id], before[id]);
        }
        w.nodes.retain(|n| n.id != "b");
        settle(&mut scene, &w);
        assert_eq!(anchors(&scene)["c"], before["c"]);
        assert!(!anchors(&scene).contains_key("b"));
        // Archived parent provenance remains cached, rather than using CENTER.
        assert_eq!(slot_position(scene.slots["b"]).to_array(), before["b"]);
        w.nodes.push(node("b", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(anchors(&scene)["b"], before["b"]);
    }

    #[test]
    fn archived_parent_anchor_is_reserved_before_new_workers() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![node("parent", "root"), node("child", "parent")]);
        settle(&mut scene, &w);
        let parent_anchor = scene.workers["parent"].anchor;
        let child_anchor = scene.workers["child"].anchor;
        w.nodes.retain(|n| n.id != "parent");
        settle(&mut scene, &w);
        assert!(!scene.workers.contains_key("parent"));
        w.nodes.push(node("new", "root"));
        scene.update(Some("root"), &w, 0.02);
        assert_eq!(slot_position(scene.slots["parent"]), parent_anchor);
        assert_eq!(scene.workers["child"].anchor, child_anchor);
        assert_ne!(scene.workers["new"].anchor, parent_anchor);
    }

    #[test]
    fn root_switch_and_deselect_clear_all_previous_geometry() {
        let w = workspace(vec![node("old", "root")]);
        let mut scene = Scene::new();
        settle(&mut scene, &w);
        let stale = Workspace {
            error: Some("offline".into()),
            stale: true,
            ..w.clone()
        };
        scene.update(Some("new-root"), &stale, 0.02);
        assert!(scene.cubes().is_empty());
        assert!(scene.stations().is_empty());
        assert!(scene.slots.is_empty());
        scene.update(None, &w, 0.02);
        assert!(scene.cubes().is_empty());
        scene.update(Some("root"), &w, 0.02);
        let mut fresh = Scene::new();
        fresh.update(Some("root"), &w, 0.02);
        assert_eq!(anchors(&scene), anchors(&fresh));
    }

    #[test]
    fn switching_between_fresh_root_projections_does_not_retain_old_workers() {
        let mut scene = Scene::new();
        settle(&mut scene, &workspace(vec![node("old", "root")]));
        let next = workspace(vec![node("new", "other-root")]);
        scene.update(Some("other-root"), &next, 0.02);
        assert_eq!(scene.stations().len(), 1);
        assert_eq!(scene.stations()[0].id, "new");
        assert!(!scene.workers.contains_key("old"));
        assert!(!scene.slots.contains_key("old"));
    }

    #[test]
    fn partial_old_root_nodes_cannot_leak_into_new_root() {
        let mut scene = Scene::new();
        settle(&mut scene, &workspace(vec![node("old", "root")]));
        let partial = Workspace {
            nodes: vec![
                node("old", "root"),
                node("nested", "old"),
                node("new", "other-root"),
            ],
            truncated: true,
            ..Default::default()
        };
        scene.update(Some("other-root"), &partial, 0.02);
        assert_eq!(scene.stations().len(), 1);
        assert_eq!(scene.stations()[0].id, "new");
        assert_eq!(scene.stations()[0].state, State::Running);
    }

    #[test]
    fn stale_old_projection_cannot_add_nodes_even_when_new_root_is_a_descendant() {
        let mut scene = Scene::new();
        let old = workspace(vec![node("parent", "root"), node("child", "parent")]);
        settle(&mut scene, &old);
        let stale = Workspace { stale: true, ..old };
        scene.update(Some("parent"), &stale, 0.02);
        assert!(scene.stations().is_empty());
        scene.update(Some("parent"), &stale, 0.02);
        assert!(scene.stations().is_empty());
    }

    #[test]
    fn slots_are_bounded_distinct_without_hidden_radial_overflow() {
        let points: Vec<_> = (0..SLOT_COUNT).map(slot_position).collect();
        for (i, p) in points.iter().enumerate() {
            assert!(p.is_finite());
            assert!((0.2..12.5).contains(&p.y), "{i}: {p}");
            // On the band through the agent's default home, 2.3 m from home, or the arc 3.2 m from home above it.
            let radius = p.x.hypot(p.z);
            assert!((radius - 2.3).abs() < 0.0001 || (radius - 3.2).abs() < 0.0001);
            assert!(p.to_array().iter().all(|v| v.abs() <= 100.0));
            for q in points.iter().skip(i + 1) {
                assert!(p.distance(*q) > 0.54, "{p:?} {q:?}");
            }
        }
        assert!(points[128].y > points[0].y);
    }

    #[test]
    fn common_worker_counts_flank_coordinator_with_readable_labels() {
        // Direct workers, a deep chain, and independent missing parents must
        // all put the real stations in the first alternating row.
        for count in 1..=8 {
            for hierarchy in 0..3 {
                let nodes = (0..count)
                    .map(|i| {
                        let parent = match hierarchy {
                            0 => "root".to_owned(),
                            1 if i == 0 => "root".to_owned(),
                            1 => format!("worker-{:02}", i - 1),
                            _ => format!("absent-{i:02}"),
                        };
                        node(&format!("worker-{i:02}"), &parent)
                    })
                    .collect();
                let mut scene = Scene::new();
                scene.update(Some("root"), &workspace(nodes), 0.02);
                let stations = scene.stations();
                assert_eq!(stations.len(), count);
                for (i, station) in stations.iter().enumerate() {
                    let p = Vec3::from_array(station.position);
                    assert_eq!(p, slot_position(i));
                    // On the arc through the agent, as far from home as it is, about its height.
                    assert!((p.x.hypot(p.z) - (-CENTER.z)).abs() < 0.0001);
                    assert!(p.x.abs() > 1.6 && p.z < 0.0);
                    assert!((1.1..2.1).contains(&p.y));
                    for other in stations.iter().skip(i + 1) {
                        assert!(p.distance(Vec3::from_array(other.position)) >= 0.54);
                    }
                    // Conservative angular clearance: the coordinator's
                    // normal conversational envelope is ~1 m wide in each
                    // direction and ~0.42 m deep. Include the label's inner
                    // edge, not just the worker anchor, from a viewer at zero.
                    let inner_label_edge = p.x.abs() - 0.493;
                    assert!(inner_label_edge / -(p.z + 0.1) > 1.0 / -(CENTER.z + 0.42));
                }
                for (i, a) in stations.iter().enumerate() {
                    for b in stations.iter().skip(i + 1) {
                        // Current label bounds, projected from the origin;
                        // staggered heights separate adjacent columns too.
                        let interval = |s: &Station| {
                            let distance = -(s.position[2] + 0.1);
                            (
                                (s.position[0] - 0.48 - 3.0 * 0.0038) / distance,
                                (s.position[0] - 0.48 + 256.0 * 0.0038) / distance,
                            )
                        };
                        let (a0, a1) = interval(a);
                        let (b0, b1) = interval(b);
                        assert!(
                            a1 < b0 || b1 < a0 || (a.position[1] - b.position[1]).abs() > 0.13,
                            "{a:?} {b:?}"
                        );
                        // Distinct observer bearings, not world-X distance,
                        // determine clearance for the inward-facing signs.
                        let bearing = |s: &Station| s.position[0].atan2(-s.position[2]);
                        assert!(
                            (bearing(a) - bearing(b)).abs() > 0.3
                                || (a.position[1] - b.position[1]).abs() > 0.54
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn arc_is_balanced_curved_and_fixed_in_world_while_view_turns() {
        let w = workspace(
            (0..16)
                .map(|i| node(&format!("w-{i:02}"), "root"))
                .collect(),
        );
        let mut scene = Scene::new();
        scene.update(Some("root"), &w, 0.0);
        let original = anchors(&scene);
        for column in 0..2 {
            let left = slot_position(column * 2);
            let right = slot_position(column * 2 + 1);
            assert!((left.x + right.x).abs() < 0.00001);
            assert_eq!(left.z, right.z);
            if column > 0 {
                let previous = slot_position((column - 1) * 2 + 1);
                assert!(right.x > previous.x && right.z > previous.z);
            }
        }
        // Sixteen workers, all on the band through the agent, 2.3 m from home, which holds eighteen.
        assert!(
            scene
                .workers
                .values()
                .all(|worker| { (worker.anchor.x.hypot(worker.anchor.z) - 2.3).abs() < 0.0001 })
        );
        // Rotation changes the view coordinates, not Scene's world positions.
        let world = Vec3::from_array(original["w-00"]);
        let view = glam::Quat::from_rotation_y(0.7).inverse() * world;
        assert!(view.distance(world) > 0.1);
        scene.update(Some("root"), &w, 0.02);
        assert_eq!(anchors(&scene), original);
    }

    #[test]
    fn first_and_uncertain_discoveries_are_settled_not_launches() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![node("historical", "root")]);
        scene.update(Some("root"), &w, 0.0);
        let worker = &scene.workers["historical"];
        assert_eq!(worker.age, ARRIVAL_SECONDS);
        assert!(
            worker
                .particles
                .iter()
                .all(|p| p.distance(worker.anchor) < 0.25)
        );
        assert_eq!(scene.stations()[0].opacity, 1.0);

        w.nodes.push(node("partial", "root"));
        scene.update(
            Some("root"),
            &Workspace {
                truncated: true,
                ..w.clone()
            },
            0.0,
        );
        assert_eq!(scene.workers["partial"].age, ARRIVAL_SECONDS);
        w.nodes.push(node("after-partial", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["after-partial"].age, ARRIVAL_SECONDS);
        scene.update(
            Some("root"),
            &Workspace {
                stale: true,
                ..w.clone()
            },
            0.0,
        );
        w.nodes.push(node("after-stale", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["after-stale"].age, ARRIVAL_SECONDS);
        w.nodes.push(node("new", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["new"].age, 0.0);
        assert!(scene.workers["new"].particles.iter().all(|p| *p == CENTER));
    }

    #[test]
    fn bounded_actual_samples_drive_launch_and_return_with_center_fallback() {
        let mut scene = Scene::new();
        let mut w = Workspace::default();
        scene.update(Some("root"), &w, 0.0);
        let samples = [
            [f32::NAN, 1.0, -2.0],
            [0.2, 1.3, -2.2],
            [-0.4, 1.5, -2.5],
            [f32::INFINITY, 0.0, 0.0],
            [101.0, 1.4, -2.3],
        ];
        scene.set_coordinator_samples(&samples);
        assert_eq!(scene.coordinator_sample_count, 2);
        w.nodes.push(node("worker", "root"));
        scene.update(Some("root"), &w, 0.0);
        let worker = &scene.workers["worker"];
        assert!(
            worker
                .particles
                .iter()
                .all(|p| *p == Vec3::from_array(samples[1]) || *p == Vec3::from_array(samples[2]))
        );
        assert!(worker.particles.contains(&Vec3::from_array(samples[1])));
        assert!(worker.particles.contains(&Vec3::from_array(samples[2])));
        settle(&mut scene, &w);

        let destination = [0.3, 1.6, -2.4];
        scene.set_coordinator_samples(&[destination]);
        let before = scene.workers["worker"].particles;
        w.nodes.clear();
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["worker"].particles, before);
        assert!(
            scene.workers["worker"]
                .return_end
                .iter()
                .all(|p| *p == Vec3::from_array(destination))
        );
        scene.set_coordinator_samples(&[]);
        for _ in 0..60 {
            scene.update(Some("root"), &w, 1.0 / 60.0);
        }
        let worker = &scene.workers["worker"];
        assert!(worker.retired.unwrap() >= 0.99);
        assert!(
            worker
                .particles
                .iter()
                .all(|p| p.distance(Vec3::from_array(destination)) < 0.4)
        );
        assert!(scene.cubes().iter().all(Cube::valid));
        for _ in 0..20 {
            scene.update(Some("root"), &w, 1.0 / 60.0);
        }
        assert!(!scene.workers.contains_key("worker"));
        w.nodes.push(node("fallback", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert!(
            scene.workers["fallback"]
                .particles
                .iter()
                .all(|p| *p == CENTER)
        );
        w.nodes.clear();
        scene.update(Some("root"), &w, 0.0);
        assert!(
            scene.workers["fallback"]
                .return_end
                .iter()
                .all(|p| *p == CENTER)
        );

        let many = vec![CENTER.to_array(); 1000];
        scene.set_coordinator_samples(&many);
        assert_eq!(scene.coordinator_sample_count, MAX_COORDINATOR_SAMPLES);
    }

    #[test]
    fn distant_finite_coordinator_samples_extend_flight_without_teleports() {
        let mut scene = Scene::new();
        let mut w = Workspace::default();
        scene.update(Some("root"), &w, 0.0);
        let sample = Vec3::new(20.0, 1.4, -2.3);
        scene.set_coordinator_samples(&[sample.to_array()]);
        w.nodes.push(node("distant", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["distant"].particles[0], sample);
        assert!(scene.workers["distant"].arrival_seconds > ARRIVAL_SECONDS);
        for _ in 0..200 {
            let before = scene.workers["distant"].particles;
            scene.update(Some("root"), &w, 0.05);
            assert!(
                before
                    .iter()
                    .zip(scene.workers["distant"].particles)
                    .all(|(a, b)| a.distance(b) < 0.301)
            );
            assert!(scene.cubes().iter().all(Cube::valid));
        }
        let worker = &scene.workers["distant"];
        assert!(
            worker
                .particles
                .iter()
                .all(|p| p.distance(worker.anchor) < 0.25)
        );
        w.nodes.clear();
        scene.update(Some("root"), &w, 0.0);
        assert!(scene.workers["distant"].departure_seconds > DEPARTURE_SECONDS);
        for _ in 0..200 {
            let Some(worker) = scene.workers.get("distant") else {
                break;
            };
            let before = worker.particles;
            scene.update(Some("root"), &w, 0.05);
            if let Some(worker) = scene.workers.get("distant") {
                assert!(
                    before
                        .iter()
                        .zip(worker.particles)
                        .all(|(a, b)| a.distance(b) < 0.301)
                );
                assert!(scene.cubes().iter().all(Cube::valid));
            }
        }
        assert!(!scene.workers.contains_key("distant"));
    }

    #[test]
    fn return_is_continuous_pauses_on_uncertainty_and_reappearance_resumes_in_place() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![node("gone", "root"), node("healthy", "root")]);
        settle(&mut scene, &w);
        let healthy_anchor = scene.workers["healthy"].anchor;
        let start = scene.workers["gone"].particles;
        w.nodes.retain(|node| node.id != "gone");
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["gone"].particles, start);
        for _ in 0..30 {
            let before = scene.workers["gone"].particles;
            scene.update(Some("root"), &w, 1.0 / 60.0);
            assert!(
                before
                    .iter()
                    .zip(scene.workers["gone"].particles)
                    .all(|(a, b)| a.distance(b) < 0.1)
            );
            assert_eq!(scene.workers["healthy"].anchor, healthy_anchor);
            assert!(scene.workers["healthy"].retired.is_none());
        }
        let midway = scene.workers["gone"].particles;
        let distance =
            |points: &[Vec3; PARTICLES]| points.iter().map(|p| p.distance(CENTER)).sum::<f32>();
        assert!(distance(&midway) < distance(&start));
        for partial in [
            Workspace {
                stale: true,
                ..w.clone()
            },
            Workspace {
                truncated: true,
                ..w.clone()
            },
            Workspace {
                error: Some("offline".into()),
                ..w.clone()
            },
        ] {
            scene.update(Some("root"), &partial, 0.05);
            assert_eq!(scene.workers["gone"].particles, midway);
            assert!(scene.workers["healthy"].retired.is_none());
        }
        w.nodes.push(node("gone", "root"));
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["gone"].particles, midway);
        assert!(scene.workers["gone"].retired.is_none());
        scene.update(Some("root"), &w, 1.0 / 60.0);
        assert!(
            midway
                .iter()
                .zip(scene.workers["gone"].particles)
                .all(|(a, b)| a.distance(b) < 0.101)
        );
        settle(&mut scene, &w);
        assert!(
            scene.workers["gone"]
                .particles
                .iter()
                .all(|p| p.distance(scene.workers["gone"].anchor) < 0.25)
        );
    }

    #[test]
    fn later_tiers_favor_nearby_parents_without_skipping_tiers() {
        let mut nodes: Vec<_> = (0..8)
            .map(|i| node(&format!("worker-{i:02}"), "root"))
            .collect();
        nodes.push(node("child", "worker-06"));
        let mut scene = Scene::new();
        scene.update(Some("root"), &workspace(nodes), 0.02);
        let parent = scene.workers["worker-06"].anchor;
        let child = &scene.workers["child"];
        assert_eq!(child.origin, parent);
        assert!((8..16).contains(&child.slot));
        assert!((8..16).all(|slot| {
            child.anchor.distance_squared(parent)
                <= slot_position(slot).distance_squared(parent) + 0.0001
        }));
    }

    #[test]
    fn over_sixty_four_workers_are_rejected_without_displacing_live_stations() {
        let mut scene = Scene::new();
        let mut w = workspace(
            (0..64)
                .map(|i| node(&format!("w-{i:02}"), "root"))
                .collect(),
        );
        scene.update(Some("root"), &w, 0.02);
        let before = anchors(&scene);
        w.nodes.push(node("overflow", "root"));
        assert!(!w.valid());
        scene.update(Some("root"), &w, 0.02);
        assert_eq!(anchors(&scene), before);
        assert!(scene.stations().iter().all(|s| s.state == State::Unknown));
    }

    #[test]
    fn ordinary_state_changes_never_teleport_particles_or_stations() {
        let mut w = workspace(vec![node("a", "root")]);
        let mut scene = Scene::new();
        settle(&mut scene, &w);
        let anchor = scene.stations()[0].position;
        for state in [
            State::Waiting,
            State::Completed,
            State::Error,
            State::Unknown,
            State::Running,
        ] {
            let before = scene.workers["a"].particles;
            w.nodes[0].state = state;
            scene.update(Some("root"), &w, 1.0 / 72.0);
            for (a, b) in before.iter().zip(scene.workers["a"].particles) {
                assert!(a.distance(b) < 0.035);
            }
            assert_eq!(scene.stations()[0].position, anchor);
        }
        assert_eq!(scene.stations()[0].state, State::Running);
    }

    #[test]
    fn historical_terminal_forms_are_distinct_deterministic_and_gently_ambient() {
        for state in [State::Completed, State::Error] {
            let w = workspace(vec![Node {
                state,
                ..node("a", "root")
            }]);
            let mut first = Scene::new();
            let mut second = Scene::new();
            first.update(Some("root"), &w, 0.0);
            second.update(Some("root"), &w, 0.0);
            let worker = &first.workers["a"];
            let positions = worker.particles;
            assert!(worker.terminal_motion.is_none());
            assert!(!worker.flying);
            assert_eq!(positions, second.workers["a"].particles);
            for (i, p) in positions.iter().enumerate() {
                assert_eq!(*p, terminal_position(worker.anchor, state, i));
                for other in positions.iter().skip(i + 1) {
                    assert!(p.distance(*other) > 0.059, "{p:?} {other:?}");
                }
            }
            for _ in 0..180 {
                let before = first.workers["a"].particles;
                first.update(Some("root"), &w, 1.0 / 60.0);
                second.update(Some("root"), &w, 1.0 / 60.0);
                assert_eq!(first.workers["a"].particles, second.workers["a"].particles);
                assert!(
                    before
                        .iter()
                        .zip(first.workers["a"].particles)
                        .all(|(a, b)| a.distance(b) < 0.0004)
                );
                assert_terminal_form(&first.workers["a"]);
                assert_eq!(
                    first.cubes().len(),
                    if state == State::Completed {
                        1
                    } else {
                        PARTICLES
                    }
                );
            }
            assert_ne!(first.workers["a"].particles, positions);
            assert!(first.cubes().iter().all(Cube::valid));
        }

        // The error form has an open split in station-tangential coordinates.
        let anchor = slot_position(0);
        // A worker placed straight above or below home faces home's front.
        let radial = Vec3::new(anchor.x, 0.0, anchor.z)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let local_x = |state, i| (terminal_position(anchor, state, i) - anchor).dot(tangent);
        assert!((0..PARTICLES).all(|i| local_x(State::Error, i).abs() > 0.06));
        assert!((0..12).all(|i| local_x(State::Error, i) < 0.0));
        assert!((12..24).all(|i| local_x(State::Error, i) > 0.0));
    }

    #[test]
    fn terminal_bounce_occurs_once_then_resumes_and_replays_on_new_transition() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![node("a", "root")]);
        settle(&mut scene, &w);
        let initial = scene.workers["a"].particles;
        w.nodes[0].state = State::Completed;
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["a"].particles, initial);
        assert!(scene.workers["a"].terminal_motion.unwrap().bounce);
        for _ in 0..12 {
            let before = scene.workers["a"].particles;
            scene.update(Some("root"), &w, 1.0 / 60.0);
            assert!(
                before
                    .iter()
                    .zip(scene.workers["a"].particles)
                    .all(|(a, b)| a.distance(b) < 0.035)
            );
        }
        let worker = &scene.workers["a"];
        let motion = worker.terminal_motion.unwrap();
        let t = motion.elapsed / motion.duration;
        let baseline = worker.flight_start[0].lerp(
            terminal_position(worker.anchor, State::Completed, 0),
            smooth(t),
        );
        assert!(worker.particles[0].y - baseline.y > 0.03);
        settle(&mut scene, &w);
        assert!(scene.workers["a"].terminal_motion.is_none());
        for _ in 0..120 {
            let before = scene.workers["a"].particles;
            scene.update(Some("root"), &w, 1.0 / 60.0);
            assert!(
                before
                    .iter()
                    .zip(scene.workers["a"].particles)
                    .all(|(a, b)| a.distance(b) < 0.0004)
            );
            assert!(scene.workers["a"].terminal_motion.is_none());
        }

        let settled = scene.workers["a"].particles;
        w.nodes[0].state = State::Running;
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["a"].particles, settled);
        settle(&mut scene, &w);
        assert_ne!(scene.workers["a"].particles, settled);
        w.nodes[0].state = State::Completed;
        scene.update(Some("root"), &w, 0.0);
        assert!(scene.workers["a"].terminal_motion.unwrap().bounce);
    }

    #[test]
    fn newly_observed_terminal_worker_arrives_without_a_fabricated_bounce() {
        for state in [State::Completed, State::Error] {
            let mut scene = Scene::new();
            scene.update(Some("root"), &Workspace::default(), 0.0);
            let w = workspace(vec![Node {
                state,
                ..node("a", "root")
            }]);
            scene.update(Some("root"), &w, 0.0);
            assert!(scene.workers["a"].flying);
            assert!(scene.workers["a"].terminal_motion.is_none());
            assert!(scene.workers["a"].particles.iter().all(|p| *p == CENTER));
            for _ in 0..180 {
                let before = scene.workers["a"].particles;
                let before_scales = scene.workers["a"].particle_scales;
                scene.update(Some("root"), &w, 1.0 / 60.0);
                assert!(
                    before
                        .iter()
                        .zip(scene.workers["a"].particles)
                        .all(|(a, b)| a.distance(b) <= 0.101)
                );
                assert!(scene.workers["a"].terminal_motion.is_none());
                assert!(
                    before_scales
                        .iter()
                        .zip(scene.workers["a"].particle_scales)
                        .all(|(a, b)| a.distance(b) < 0.025)
                );
            }
            let worker = &scene.workers["a"];
            assert!(!worker.flying);
            assert_terminal_form(worker);
            for _ in 0..60 {
                let before = scene.workers["a"].particles;
                scene.update(Some("root"), &w, 1.0 / 60.0);
                assert!(
                    before
                        .iter()
                        .zip(scene.workers["a"].particles)
                        .all(|(a, b)| a.distance(b) < 0.0004)
                );
                assert_terminal_form(&scene.workers["a"]);
            }
        }
    }

    #[test]
    fn nonterminal_running_waiting_and_unknown_keep_moving() {
        for state in [State::Running, State::Waiting, State::Unknown] {
            let mut scene = Scene::new();
            let w = workspace(vec![Node {
                state,
                ..node("a", "root")
            }]);
            settle(&mut scene, &w);
            let before = scene.workers["a"].particles;
            for _ in 0..60 {
                scene.update(Some("root"), &w, 1.0 / 60.0);
            }
            assert_ne!(scene.workers["a"].particles, before);
        }
    }

    #[test]
    fn stale_partial_unknown_and_recovery_do_not_replay_terminal_bounce() {
        for state in [State::Completed, State::Error] {
            let mut scene = Scene::new();
            let mut w = workspace(vec![node("a", "root")]);
            settle(&mut scene, &w);
            w.nodes[0].state = state;
            scene.update(Some("root"), &w, 0.02);
            assert!(scene.workers["a"].terminal_motion.unwrap().bounce);
            let elapsed = scene.workers["a"].terminal_motion.unwrap().elapsed;
            scene.update(
                Some("root"),
                &Workspace {
                    stale: true,
                    ..w.clone()
                },
                0.02,
            );
            assert!(scene.workers["a"].terminal_motion.unwrap().elapsed > elapsed);
            scene.update(Some("root"), &w, 0.02);
            assert!(scene.workers["a"].terminal_motion.unwrap().elapsed > elapsed + 0.02);
            settle(&mut scene, &w);
            for uncertain in [
                Workspace {
                    stale: true,
                    ..w.clone()
                },
                Workspace {
                    truncated: true,
                    ..Default::default()
                },
                Workspace {
                    error: Some("offline".into()),
                    ..Default::default()
                },
            ] {
                for _ in 0..20 {
                    scene.update(Some("root"), &uncertain, 0.05);
                }
                assert_terminal_form(&scene.workers["a"]);
                assert!(scene.workers["a"].terminal_motion.is_none());
                scene.update(Some("root"), &w, 0.02);
                assert_terminal_form(&scene.workers["a"]);
                assert!(scene.workers["a"].terminal_motion.is_none());
            }
            let unknown = workspace(vec![Node {
                state: State::Unknown,
                ..node("a", "root")
            }]);
            let positions = scene.workers["a"].particles;
            scene.update(Some("root"), &unknown, 0.0);
            assert_eq!(scene.workers["a"].particles, positions);
            for _ in 0..60 {
                let before = scene.workers["a"].particles;
                scene.update(Some("root"), &unknown, 1.0 / 60.0);
                assert!(
                    before
                        .iter()
                        .zip(scene.workers["a"].particles)
                        .all(|(a, b)| a.distance(b) <= 0.101)
                );
            }
            assert_ne!(scene.workers["a"].particles, positions);
            let before = scene.workers["a"].particles;
            scene.update(Some("root"), &w, 0.0);
            assert_eq!(scene.workers["a"].particles, before);
            assert!(!scene.workers["a"].terminal_motion.unwrap().bounce);
            settle(&mut scene, &w);
            assert_terminal_form(&scene.workers["a"]);
        }

        let mut scene = Scene::new();
        let mut w = workspace(vec![node("a", "root")]);
        settle(&mut scene, &w);
        scene.update(
            Some("root"),
            &Workspace {
                stale: true,
                ..w.clone()
            },
            0.02,
        );
        w.nodes[0].state = State::Completed;
        scene.update(Some("root"), &w, 0.0);
        assert!(!scene.workers["a"].terminal_motion.unwrap().bounce);
        settle(&mut scene, &w);
        assert!(scene.workers["a"].terminal_motion.is_none());
    }

    #[test]
    fn status_changes_mid_flight_and_terminal_return_never_teleport() {
        for state in [State::Completed, State::Error] {
            let mut scene = Scene::new();
            let mut w = workspace(vec![node("a", "root")]);
            scene.update(Some("root"), &Workspace::default(), 0.0);
            scene.update(Some("root"), &w, 0.0);
            for _ in 0..25 {
                scene.update(Some("root"), &w, 1.0 / 60.0);
            }
            let before = scene.workers["a"].particles;
            let before_scales = scene.workers["a"].particle_scales;
            w.nodes[0].state = state;
            scene.update(Some("root"), &w, 0.0);
            assert_eq!(scene.workers["a"].particles, before);
            assert_eq!(scene.workers["a"].particle_scales, before_scales);
            assert!(scene.workers["a"].terminal_motion.unwrap().bounce);
            for _ in 0..180 {
                let before = scene.workers["a"].particles;
                let before_scales = scene.workers["a"].particle_scales;
                scene.update(Some("root"), &w, 1.0 / 60.0);
                assert!(
                    before
                        .iter()
                        .zip(scene.workers["a"].particles)
                        .all(|(a, b)| a.distance(b) <= 0.101)
                );
                assert!(scene.cubes().iter().all(Cube::valid));
                assert!(
                    before_scales
                        .iter()
                        .zip(scene.workers["a"].particle_scales)
                        .all(|(a, b)| a.distance(b) < 0.025)
                );
            }
            let shape = scene.workers["a"].particles;
            assert!(scene.workers["a"].terminal_motion.is_none());
            let empty = Workspace::default();
            scene.update(Some("root"), &empty, 0.0);
            assert_eq!(scene.workers["a"].particles, shape);
            for _ in 0..30 {
                let before = scene.workers["a"].particles;
                scene.update(Some("root"), &empty, 1.0 / 60.0);
                assert!(
                    before
                        .iter()
                        .zip(scene.workers["a"].particles)
                        .all(|(a, b)| a.distance(b) <= 0.101)
                );
            }
            let returning = scene.workers["a"].particles;
            scene.update(Some("root"), &w, 0.0);
            assert_eq!(scene.workers["a"].particles, returning);
            assert!(!scene.workers["a"].terminal_motion.unwrap().bounce);
            settle(&mut scene, &w);
            assert_terminal_form(&scene.workers["a"]);
        }
    }

    #[test]
    fn nested_workers_depart_from_actual_parent_even_when_missing() {
        let mut scene = Scene::new();
        let w = workspace(vec![node("child", "parent"), node("parent", "root")]);
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["parent"].origin, CENTER);
        assert_eq!(
            scene.workers["child"].origin,
            scene.workers["parent"].anchor
        );
        let mut missing = Scene::new();
        missing.update(
            Some("root"),
            &workspace(vec![node("child", "archived")]),
            0.02,
        );
        assert_eq!(
            missing.workers["child"].origin,
            slot_position(missing.slots["archived"])
        );
        assert_ne!(missing.workers["child"].origin, CENTER);
        assert_eq!(missing.stations().len(), 1);
    }

    #[test]
    fn a_nested_worker_returns_to_its_parent_unless_the_parent_leaves_too() {
        let mut scene = Scene::new();
        let coordinator = Vec3::new(0.3, 1.6, -2.4);
        scene.set_coordinator_samples(&[coordinator.to_array()]);
        let mut w = workspace(vec![
            node("lead", "root"),
            node("helper", "lead"),
            node("second", "lead"),
        ]);
        settle(&mut scene, &w);
        let lead = scene.workers["lead"].anchor;

        // Deleting the helper sends it back to the lead's station, not to the coordinator.
        w.nodes.retain(|node| node.id != "helper");
        scene.update(Some("root"), &w, 0.0);
        assert!(
            scene.workers["helper"]
                .return_end
                .iter()
                .all(|p| *p == lead)
        );
        let departure = scene.workers["helper"].departure_seconds;
        let mut elapsed = 0.0;
        while elapsed + 1.0 / 60.0 < departure {
            scene.update(Some("root"), &w, 1.0 / 60.0);
            elapsed += 1.0 / 60.0;
        }
        let helper = &scene.workers["helper"];
        assert!(helper.particles.iter().all(|p| p.distance(lead) < 0.05));

        // A parent leaving with its child is no destination: both return to the coordinator.
        w.nodes.clear();
        scene.update(Some("root"), &w, 0.0);
        for id in ["lead", "second"] {
            assert!(
                scene.workers[id]
                    .return_end
                    .iter()
                    .all(|p| *p == coordinator),
                "{id}"
            );
        }
    }

    /// A placement that stands each worker on a line ahead of its origin, by its slot, a metre further for each worker
    /// already standing, and records what it was told.
    #[derive(Default)]
    struct Line {
        told: std::sync::Mutex<Vec<Told>>,
    }

    /// What a placement was told: the worker, its parent, its state, its slot and the workers already standing.
    type Told = (String, Option<String>, State, usize, Vec<String>);

    impl crate::anchors::WorkerPlacement for Line {
        fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            self.told.lock().unwrap().push((
                worker.id.to_owned(),
                worker.parent.map(str::to_owned),
                worker.state,
                worker.slot,
                worker
                    .workers
                    .iter()
                    .map(|placed| placed.id.to_owned())
                    .collect(),
            ));
            origin.place(Vec3::new(
                worker.slot as f32,
                1.5,
                -2.0 - worker.workers.len() as f32,
            ))
        }
    }

    fn turned(position: Vec3) -> Anchor {
        Anchor {
            position,
            orientation: Quat::from_rotation_y(0.4),
        }
    }

    #[test]
    fn the_default_arc_stands_at_the_workers_anchor_turned_with_it() {
        let origin = turned(Vec3::new(1.0, 0.0, -6.0));
        let mut scene = Scene::new();
        // Arranged the same way as by default, nothing changes.
        scene.arrange(Anchor::AGENT, crate::anchors::EYES, None, None);
        let w = workspace(vec![node("a", "root"), node("b", "root")]);
        scene.update(Some("root"), &w, 0.0);
        for (id, position) in anchors(&scene) {
            assert_eq!(position, slot_position(scene.slots[&id]).to_array());
        }
        // Moved and turned, every live worker is placed again, about the anchor, glides there, its sign with it, and
        // points at it there.
        scene.arrange(origin, crate::anchors::EYES, None, None);
        scene.update(Some("root"), &w, 0.0);
        let before = anchors(&scene);
        for (id, position) in &before {
            assert_eq!(*position, slot_position(scene.slots[id]).to_array());
        }
        scene.update(Some("root"), &w, 0.1);
        for (id, position) in anchors(&scene) {
            let expected =
                crate::anchors::HomeArc::at(origin, crate::anchors::EYES, scene.slots[&id]);
            let from = Vec3::from_array(before[&id]);
            let moved = Vec3::from_array(position).distance(from);
            // Easing out, no faster than its greatest speed.
            assert!(moved > 0.01 && moved <= GLIDE_SPEED * 0.1 + 1e-4, "{moved}");
            assert!(Vec3::from_array(position).distance(expected) < from.distance(expected));
        }
        for _ in 0..400 {
            scene.update(Some("root"), &w, 1.0 / 60.0);
        }
        for (id, position) in anchors(&scene) {
            let expected =
                crate::anchors::HomeArc::at(origin, crate::anchors::EYES, scene.slots[&id]);
            assert_eq!(Vec3::from_array(position), expected);
            assert!(scene.targets().iter().any(|target| target.key == id
                && target.shape
                    == Shape::Sphere {
                        center: expected,
                        radius: POINTING_RADIUS
                    }));
        }
        // A worker arriving afterwards stands on the moved arc too.
        let more = workspace(vec![
            node("a", "root"),
            node("b", "root"),
            node("c", "root"),
        ]);
        scene.update(Some("root"), &more, 0.0);
        assert_eq!(
            scene.workers["c"].anchor,
            crate::anchors::HomeArc::at(origin, crate::anchors::EYES, scene.slots["c"])
        );
    }

    #[test]
    fn a_placement_places_arrivals_and_never_moves_live_workers_under_churn() {
        let line = std::sync::Arc::new(Line::default());
        let placement: Placement = line.clone();
        let origin = Anchor {
            position: Vec3::new(0.0, 0.0, -1.0),
            orientation: Quat::IDENTITY,
        };
        let mut scene = Scene::new();
        scene.arrange(origin, crate::anchors::EYES, None, Some(placement.clone()));
        let mut w = workspace(vec![node("a", "root"), node("b", "a")]);
        settle(&mut scene, &w);
        // Each was told of the workers already standing, its parent, but not the root, its state and its slot.
        let told = line.told.lock().unwrap().clone();
        assert_eq!(told.len(), 2);
        assert_eq!(told[0].0, "a");
        assert_eq!((told[0].1.as_deref(), told[0].2), (None, State::Running));
        assert_eq!(told[1].0, "b");
        assert_eq!(told[1].1.as_deref(), Some("a"));
        assert_eq!(told[1].4, ["a"]);
        let a = scene.workers["a"].anchor;
        assert_eq!(a, Vec3::new(told[0].3 as f32, 1.5, -3.0));
        let before = anchors(&scene);
        // Titles, states, order and membership change; live workers stay where they were placed, and the same
        // arrangement given again places nothing.
        w.nodes.reverse();
        w.nodes[0].title = "renamed".into();
        w.nodes[1].state = State::Waiting;
        w.nodes.push(node("c", "root"));
        for _ in 0..3 {
            scene.arrange(origin, crate::anchors::EYES, None, Some(placement.clone()));
            scene.update(Some("root"), &w, 1.0 / 60.0);
        }
        w.nodes.retain(|node| node.id != "b");
        scene.arrange(origin, crate::anchors::EYES, None, Some(placement.clone()));
        scene.update(Some("root"), &w, 1.0 / 60.0);
        let after = anchors(&scene);
        assert_eq!(after["a"], before["a"]);
        // The ray points at each placed worker where it stands, through the churn.
        for (id, position) in &after {
            let center = Vec3::from_array(*position);
            if id == "b" {
                continue;
            }
            assert!(scene.targets().iter().any(|target| &target.key == id
                && target.shape
                    == Shape::Sphere {
                        center,
                        radius: POINTING_RADIUS
                    }));
            let ray = Ray::new(Vec3::new(0.0, 1.6, 0.0), center - Vec3::new(0.0, 1.6, 0.0));
            assert_eq!(scene.point(ray), Some(id.as_str()));
        }
        assert_eq!(line.told.lock().unwrap().len(), 3);
        assert_eq!(scene.workers["c"].anchor.z, -1.0 - 2.0 - 2.0);
        // The worker leaving returns to its parent where it was placed.
        assert!(scene.workers["b"].return_end.iter().all(|p| *p == a));
        // Another arrangement places every live worker again, in the order of their slots.
        scene.arrange(Anchor::HOME, crate::anchors::EYES, None, Some(placement));
        let replaced: Vec<String> = line.told.lock().unwrap()[3..]
            .iter()
            .map(|told| told.0.clone())
            .collect();
        assert_eq!(replaced, ["a", "c"]);
        assert_eq!(
            scene.workers["c"].place,
            Vec3::new(scene.slots["c"] as f32, 1.5, -3.0)
        );
        // Back to the default, the arc again.
        scene.arrange(Anchor::AGENT, crate::anchors::EYES, None, None);
        assert_eq!(scene.workers["a"].place, slot_position(scene.slots["a"]));
    }

    #[test]
    fn a_child_arriving_with_its_parent_launches_from_where_the_parent_is_placed() {
        let placement: Placement = std::sync::Arc::new(Line::default());
        let mut scene = Scene::new();
        scene.arrange(Anchor::AGENT, crate::anchors::EYES, None, Some(placement));
        // Trusted twice, so that the next arrivals launch.
        let empty = workspace(Vec::new());
        scene.update(Some("root"), &empty, 0.0);
        scene.update(Some("root"), &empty, 0.0);
        // The child's identity sorts before its parent's, and its slot after.
        let w = workspace(vec![node("zz", "root"), node("aa", "zz")]);
        scene.update(Some("root"), &w, 0.0);
        assert!(scene.workers["aa"].flying);
        assert_eq!(scene.workers["aa"].origin, scene.workers["zz"].anchor);
        assert!(
            scene.workers["aa"]
                .particles
                .iter()
                .all(|p| *p == scene.workers["zz"].anchor)
        );
    }

    /// A placement that records the slots of the workers it is told stand already, and stands each worker as `Line`.
    #[derive(Default)]
    struct Slots(std::sync::Mutex<Vec<Vec<usize>>>);

    impl crate::anchors::WorkerPlacement for Slots {
        fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            self.0
                .lock()
                .unwrap()
                .push(worker.workers.iter().map(|placed| placed.slot).collect());
            origin.place(Vec3::new(worker.slot as f32, 1.5, -2.0))
        }
    }

    #[test]
    fn a_placement_is_told_of_the_workers_standing_in_the_order_of_their_slots_on_both_paths() {
        let slots = std::sync::Arc::new(Slots::default());
        let placement: Placement = slots.clone();
        let mut scene = Scene::new();
        scene.arrange(
            Anchor::AGENT,
            crate::anchors::EYES,
            None,
            Some(placement.clone()),
        );
        // Identities that sort against their slots: "z" takes slot 0, "a" slot 1, "m" slot 2.
        let first = workspace(vec![node("z", "root"), node("a", "z")]);
        scene.update(Some("root"), &first, 0.0);
        let mut second = workspace(vec![node("z", "root"), node("a", "z"), node("m", "a")]);
        second.nodes.push(node("b", "root"));
        scene.update(Some("root"), &second, 0.0);
        let told = slots.0.lock().unwrap().clone();
        for workers in &told {
            assert!(workers.is_sorted(), "{told:?}");
        }
        // Arriving: those standing, in slot order, with each just placed among them.
        assert_eq!(told[3].len(), 3);
        // Placed again: the same order.
        scene.arrange(
            turned(Vec3::ZERO),
            crate::anchors::EYES,
            None,
            Some(placement),
        );
        let again = slots.0.lock().unwrap()[4..].to_vec();
        assert_eq!(again.last().unwrap().len(), 3);
        for workers in &again {
            assert!(workers.is_sorted(), "{again:?}");
        }
    }

    /// A placement that panics, counting how often it is called.
    #[derive(Default)]
    struct Panics(std::sync::atomic::AtomicUsize);

    impl crate::anchors::WorkerPlacement for Panics {
        fn place(&self, _origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            panic!("no place for {}", worker.id)
        }
    }

    #[test]
    fn a_placement_that_panics_is_never_called_again_and_its_workers_stand_on_the_arc() {
        use std::sync::atomic::Ordering;
        // Arriving: two workers at once, then one more.
        let panics = std::sync::Arc::new(Panics::default());
        let placement: Placement = panics.clone();
        let mut scene = Scene::new();
        scene.arrange(
            Anchor::AGENT,
            crate::anchors::EYES,
            None,
            Some(placement.clone()),
        );
        let w = workspace(vec![node("a", "root"), node("b", "root")]);
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(panics.0.load(Ordering::Relaxed), 1);
        let more = workspace(vec![
            node("a", "root"),
            node("b", "root"),
            node("c", "root"),
        ]);
        for _ in 0..3 {
            scene.arrange(
                Anchor::AGENT,
                crate::anchors::EYES,
                None,
                Some(placement.clone()),
            );
            scene.update(Some("root"), &more, 0.0);
        }
        // Moved elsewhere with it, still not called.
        let away = turned(Vec3::new(0.0, 0.0, -3.0));
        scene.arrange(away, crate::anchors::EYES, None, Some(placement.clone()));
        assert_eq!(panics.0.load(Ordering::Relaxed), 1);
        for id in ["a", "b", "c"] {
            assert_eq!(
                scene.workers[id].place,
                crate::anchors::HomeArc::at(away, crate::anchors::EYES, scene.slots[id])
            );
        }
        // Placing again: the first call panics, and the rest stand on the arc.
        let again = std::sync::Arc::new(Panics::default());
        let fresh: Placement = again.clone();
        scene.arrange(Anchor::AGENT, crate::anchors::EYES, None, Some(fresh));
        assert_eq!(again.0.load(Ordering::Relaxed), 1);
        for id in ["a", "b", "c"] {
            assert_eq!(scene.workers[id].anchor, slot_position(scene.slots[id]));
        }
        // Neither is called again.
        scene.arrange(away, crate::anchors::EYES, None, Some(placement));
        assert_eq!(panics.0.load(Ordering::Relaxed), 1);
        assert_eq!(scene.poisoned.len(), 2);
    }

    #[test]
    fn a_child_arriving_later_launches_from_where_its_parent_was_placed() {
        let placement: Placement = std::sync::Arc::new(Line::default());
        let mut scene = Scene::new();
        scene.arrange(Anchor::AGENT, crate::anchors::EYES, None, Some(placement));
        let parent = workspace(vec![node("lead", "root")]);
        scene.update(Some("root"), &parent, 0.0);
        scene.update(Some("root"), &parent, 0.0);
        let lead = scene.workers["lead"].anchor;
        assert_ne!(lead, slot_position(scene.slots["lead"]));
        let both = workspace(vec![node("lead", "root"), node("helper", "lead")]);
        scene.update(Some("root"), &both, 0.0);
        assert!(scene.workers["helper"].flying);
        assert_eq!(scene.workers["helper"].origin, lead);
        assert!(scene.workers["helper"].particles.iter().all(|p| *p == lead));
    }

    #[test]
    fn after_a_rearrangement_workers_glide_to_their_new_places_at_a_bounded_speed() {
        let mut scene = Scene::new();
        let w = workspace(vec![node("a", "root"), node("b", "root")]);
        settle(&mut scene, &w);
        let away = Anchor {
            position: Vec3::new(0.0, 0.0, -6.0),
            orientation: Quat::IDENTITY,
        };
        scene.arrange(away, crate::anchors::EYES, None, None);
        const DT: f32 = 1.0 / 60.0;
        let mut previous: Vec<Vec3> = scene
            .cubes()
            .iter()
            .map(|cube| Vec3::from_array(cube.position))
            .collect();
        for _ in 0..300 {
            scene.update(Some("root"), &w, DT);
            let now: Vec<Vec3> = scene
                .cubes()
                .iter()
                .map(|cube| Vec3::from_array(cube.position))
                .collect();
            for (before, after) in previous.iter().zip(&now) {
                // The particles' bounded speed, 6 m/s, and a little for the flock's own motion.
                assert!(before.distance(*after) <= 6.0 * DT + 0.01);
            }
            previous = now;
        }
        // Arrived, each flock is about its new place.
        for id in ["a", "b"] {
            let worker = &scene.workers[id];
            let centre = worker.particles.iter().copied().sum::<Vec3>() / PARTICLES as f32;
            assert!(
                centre.distance(crate::anchors::HomeArc::at(
                    away,
                    crate::anchors::EYES,
                    worker.slot
                )) < 0.3,
                "{id}"
            );
        }
    }

    #[test]
    fn a_placement_standing_a_worker_out_of_bounds_or_above_home_stays_finite() {
        struct At(Vec3);
        impl crate::anchors::WorkerPlacement for At {
            fn place(&self, _origin: Anchor, _worker: &Placing<'_>) -> Vec3 {
                self.0
            }
        }
        let mut scene = Scene::new();
        scene.arrange(
            Anchor::AGENT,
            crate::anchors::EYES,
            None,
            Some(std::sync::Arc::new(At(Vec3::splat(f32::NAN)))),
        );
        let w = workspace(vec![node("a", "root")]);
        scene.update(Some("root"), &w, 0.0);
        assert_eq!(scene.workers["a"].anchor, slot_position(scene.slots["a"]));
        // Straight above home, its terminal forms still face somewhere.
        let mut above = Scene::new();
        above.arrange(
            Anchor::AGENT,
            crate::anchors::EYES,
            None,
            Some(std::sync::Arc::new(At(Vec3::new(0.0, 2.0, 0.0)))),
        );
        let mut done = node("a", "root");
        done.state = State::Error;
        settle(&mut above, &workspace(vec![done]));
        assert!(above.cubes().iter().all(Cube::valid));
        assert_eq!(above.stations()[0].position, [0.0, 2.0, 0.0]);
    }

    #[test]
    fn a_partial_projection_keeps_observed_children_of_a_parent_it_omits() {
        let mut scene = Scene::new();
        let complete = workspace(vec![node("parent", "root"), node("child", "parent")]);
        settle(&mut scene, &complete);

        // The snapshot budget left out the parent but kept its child, which completed and started a worker of its
        // own. Both are observed; the omitted parent is only unknown.
        let partial = Workspace {
            nodes: vec![
                Node {
                    state: State::Completed,
                    ..node("child", "parent")
                },
                node("grandchild", "child"),
                node("stranger", "elsewhere"),
            ],
            truncated: true,
            ..Default::default()
        };
        scene.update(Some("root"), &partial, 0.02);
        assert_eq!(scene.workers["parent"].state, State::Unknown);
        assert_eq!(scene.workers["child"].state, State::Completed);
        assert_eq!(scene.workers["grandchild"].state, State::Running);
        assert_eq!(
            scene.workers["grandchild"].parent_id.as_deref(),
            Some("child")
        );
        // A worker under a parent this space never showed is not taken into it.
        assert!(!scene.workers.contains_key("stranger"));

        let recovered = workspace(vec![
            node("parent", "root"),
            Node {
                state: State::Completed,
                ..node("child", "parent")
            },
            node("grandchild", "child"),
        ]);
        scene.update(Some("root"), &recovered, 0.02);
        let states: BTreeMap<_, _> = scene
            .stations()
            .iter()
            .map(|station| (station.id.as_str(), station.state))
            .collect();
        assert_eq!(
            states,
            BTreeMap::from([
                ("child", State::Completed),
                ("grandchild", State::Running),
                ("parent", State::Running),
            ])
        );
        assert!(
            scene
                .workers
                .values()
                .all(|worker| worker.retired.is_none())
        );
    }

    #[test]
    fn new_child_without_an_observed_parent_does_not_launch_from_a_fictional_station() {
        let mut scene = Scene::new();
        scene.update(Some("root"), &Workspace::default(), 0.0);
        let w = workspace(vec![node("child", "missing")]);
        scene.update(Some("root"), &w, 0.0);
        let child = &scene.workers["child"];
        assert_eq!(child.origin, slot_position(scene.slots["missing"]));
        assert_eq!(child.age, child.arrival_seconds);
        assert!(
            child
                .particles
                .iter()
                .all(|p| p.distance(child.anchor) < 0.25)
        );
        assert_eq!(scene.stations().len(), 1);
    }

    #[test]
    fn snapshots_and_changes_render_only_flocks_not_inferred_traffic() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![
            node("parent", "root"),
            node("child", "parent"),
            node("orphan", "absent"),
        ]);
        w.presentations.push(Presentation {
            id: "historical-card".into(),
            source_id: "parent".into(),
            source_message_id: "historical-message".into(),
            ..Default::default()
        });
        // An initial snapshot, including historical messages, is not a new
        // interaction. Even the first visible frame contains only flocks.
        scene.update(Some("root"), &w, 0.02);
        assert_only_worker_flocks(&scene);
        settle(&mut scene, &w);
        let original_anchors = anchors(&scene);
        let before = scene.workers["parent"].particles;
        for _ in 0..30 {
            scene.update(Some("root"), &w, 1.0 / 60.0);
            assert_only_worker_flocks(&scene);
        }
        // Natural status motion continues without a connector stream.
        assert_ne!(before, scene.workers["parent"].particles);

        w.nodes[0].state = State::Completed;
        w.nodes[1].title = "Updated title".into();
        w.presentations[0].source_message_id = "another-message".into();
        scene.update(Some("root"), &w, 0.02);
        assert_only_worker_flocks(&scene);
        assert_eq!(anchors(&scene), original_anchors);

        for partial in [
            Workspace {
                stale: true,
                ..w.clone()
            },
            Workspace {
                error: Some("unavailable".into()),
                ..w.clone()
            },
            Workspace {
                truncated: true,
                ..w.clone()
            },
        ] {
            scene.update(Some("root"), &partial, 0.02);
            assert_only_worker_flocks(&scene);
            assert_eq!(anchors(&scene), original_anchors);
        }

        // After the parent departs, neither its remembered slot nor the
        // independently absent parent produces a broken-end beacon.
        w.nodes.retain(|node| node.id != "parent");
        settle(&mut scene, &w);
        assert!(!scene.workers.contains_key("parent"));
        assert_eq!(
            slot_position(scene.slots["parent"]).to_array(),
            original_anchors["parent"]
        );
        assert_eq!(anchors(&scene)["child"], original_anchors["child"]);
        assert_only_worker_flocks(&scene);
    }

    #[test]
    fn flocks_form_smoothly_from_coordinator_and_parent_in_arrival_time() {
        let mut scene = Scene::new();
        let w = workspace(vec![node("parent", "root"), node("child", "parent")]);
        scene.update(Some("root"), &Workspace::default(), 0.0);
        scene.update(Some("root"), &w, 0.0);
        let parent_anchor = scene.workers["parent"].anchor;
        assert_eq!(scene.workers["parent"].origin, CENTER);
        assert_eq!(scene.workers["child"].origin, parent_anchor);
        for worker in scene.workers.values() {
            assert!(
                worker
                    .particles
                    .iter()
                    .all(|p| p.distance(worker.origin) < 0.05)
            );
        }
        let station_anchors = anchors(&scene);
        let dt = 1.0 / 60.0;
        // Include the first frame after the 1.5-second arrival boundary.
        for frame in 1..=91 {
            let before: BTreeMap<_, _> = scene
                .workers
                .iter()
                .map(|(id, worker)| (id.clone(), worker.particles))
                .collect();
            scene.update(Some("root"), &w, dt);
            assert_only_worker_flocks(&scene);
            assert_eq!(anchors(&scene), station_anchors);
            for (id, worker) in &scene.workers {
                for (previous, current) in before[id].iter().zip(worker.particles) {
                    assert!(previous.distance(current) <= 6.0 * dt + 0.00001);
                }
                if frame == 45 {
                    let centroid =
                        worker.particles.iter().copied().sum::<Vec3>() / PARTICLES as f32;
                    let path = worker.anchor - worker.origin;
                    let progress = (centroid - worker.origin).dot(path) / path.length_squared();
                    assert!((0.3..0.7).contains(&progress), "{id}: {progress}");
                }
                if frame == 90 {
                    assert!((worker.age - ARRIVAL_SECONDS).abs() < 0.00001);
                    assert!(
                        worker
                            .particles
                            .iter()
                            .all(|p| p.distance(worker.anchor) < 0.25)
                    );
                }
            }
        }
    }

    #[test]
    fn arrival_departure_and_completion_are_not_fake_progress() {
        let mut scene = Scene::new();
        let mut w = workspace(vec![node("a", "root")]);
        scene.update(Some("root"), &Workspace::default(), 0.0);
        scene.update(Some("root"), &w, 0.0);
        assert!(scene.workers["a"].particles[0].distance(CENTER) < 0.05);
        settle(&mut scene, &w);
        assert!(scene.workers["a"].particles[0].distance(scene.workers["a"].anchor) < 0.25);
        w.nodes[0].state = State::Completed;
        settle(&mut scene, &w);
        assert_eq!(scene.stations()[0].state, State::Completed);
        assert_eq!(scene.stations()[0].opacity, 1.0);
        scene.update(Some("root"), &Workspace::default(), 0.02);
        assert!(!scene.cubes().is_empty());
        assert!(scene.stations()[0].opacity < 1.0);
        settle(&mut scene, &Workspace::default());
        assert!(scene.stations().is_empty());
        assert!(scene.cubes().is_empty());
    }

    #[test]
    fn stale_partial_and_invalid_data_preserve_omitted_workers_as_unknown() {
        let w = workspace(vec![Node {
            state: State::Completed,
            ..node("a", "root")
        }]);
        let mut scene = Scene::new();
        settle(&mut scene, &w);
        for bad in [
            Workspace {
                error: Some("offline".into()),
                ..Default::default()
            },
            Workspace {
                truncated: true,
                ..Default::default()
            },
            Workspace {
                stale: true,
                ..w.clone()
            },
            workspace(vec![node("bad", "bad")]),
        ] {
            scene.update(Some("root"), &bad, 0.02);
            assert_eq!(scene.stations().len(), 1);
            assert_eq!(scene.stations()[0].state, State::Unknown);
            assert!(scene.cubes().iter().all(Cube::valid));
        }
        scene.update(Some("root"), &w, 0.02);
        assert_eq!(scene.stations()[0].state, State::Completed);
    }

    #[test]
    fn fresh_partial_tree_retains_omissions_as_unknown_and_observed_states() {
        let mut scene = Scene::new();
        let partial = Workspace {
            nodes: vec![
                Node {
                    state: State::Unknown,
                    ..node("a", "root")
                },
                Node {
                    state: State::Completed,
                    ..node("b", "root")
                },
            ],
            truncated: true,
            ..Default::default()
        };
        scene.update(Some("root"), &partial, 0.02);
        assert_eq!(scene.stations().len(), 2);
        assert_eq!(scene.workers["a"].state, State::Unknown);
        assert_eq!(scene.workers["b"].state, State::Completed);
        assert_eq!(scene.stations()[1].state, State::Completed);
        scene.update(
            Some("root"),
            &Workspace {
                nodes: vec![node("c", "root")],
                error: Some("unavailable".into()),
                ..Default::default()
            },
            0.02,
        );
        assert_eq!(scene.stations().len(), 3);
        assert_eq!(scene.stations()[0].state, State::Unknown);
        assert_eq!(scene.stations()[1].state, State::Unknown);
        assert_eq!(scene.stations()[2].state, State::Running);
        assert!(
            scene
                .workers
                .values()
                .all(|worker| worker.retired.is_none())
        );
    }

    #[test]
    fn truncated_sixty_four_workers_keep_observed_states() {
        let mut scene = Scene::new();
        let partial = Workspace {
            nodes: (0..64)
                .map(|i| Node {
                    state: if i == 7 {
                        State::Unknown
                    } else {
                        State::Completed
                    },
                    ..node(&format!("worker-{i:02}"), "root")
                })
                .collect(),
            truncated: true,
            ..Default::default()
        };
        scene.update(Some("root"), &partial, 0.02);
        assert_eq!(scene.stations().len(), 64);
        assert_eq!(scene.stations()[7].state, State::Unknown);
        assert!(scene.stations().iter().enumerate().all(|(i, s)| s.state
            == if i == 7 {
                State::Unknown
            } else {
                State::Completed
            }));
    }

    #[test]
    fn store_error_keeps_status_but_cannot_authorize_removals() {
        let mut scene = Scene::new();
        settle(
            &mut scene,
            &workspace(vec![node("a", "root"), node("b", "root")]),
        );
        let partial = Workspace {
            nodes: vec![Node {
                state: State::Completed,
                ..node("a", "root")
            }],
            error: Some("Presentation store unavailable".into()),
            ..Default::default()
        };
        settle(&mut scene, &partial);
        assert_eq!(scene.stations().len(), 2);
        assert_eq!(scene.stations()[0].state, State::Completed);
        assert_eq!(scene.stations()[1].state, State::Unknown);
        assert!(
            scene
                .workers
                .values()
                .all(|worker| worker.retired.is_none())
        );
        scene.update(
            Some("root"),
            &Workspace {
                stale: true,
                ..partial
            },
            0.02,
        );
        assert!(scene.stations().iter().all(|s| s.state == State::Unknown));
    }

    #[test]
    fn sixty_four_workers_and_rapid_churn_stay_within_budget() {
        let mut scene = Scene::new();
        for generation in 0..8 {
            let w = workspace(
                (0..64)
                    .map(|i| Node {
                        state: match i % 3 {
                            0 => State::Completed,
                            1 => State::Error,
                            _ => State::Running,
                        },
                        ..node(
                            &format!("worker-{generation}-{i}"),
                            &format!("missing-{generation}-{i}"),
                        )
                    })
                    .collect(),
            );
            for _ in 0..3 {
                scene.update(Some("root"), &w, 0.02);
                assert!(scene.cubes().len() <= MAX_CUBES);
                assert!(scene.cubes().iter().all(Cube::valid));
                assert_only_worker_flocks(&scene);
                assert_eq!(
                    scene.cubes().len(),
                    scene
                        .workers
                        .values()
                        .map(|worker| if solid_completed(worker) {
                            1
                        } else {
                            PARTICLES
                        })
                        .sum::<usize>()
                );
                assert!(scene.workers.len() <= MAX_WORKERS);
                assert!(scene.slots.len() <= SLOT_COUNT);
                assert!(scene.stations().len() <= MAX_WORKERS);
                assert!(scene.stations().iter().all(Station::valid));
                assert_eq!(
                    scene
                        .workers
                        .values()
                        .filter(|w| w.retired.is_none())
                        .count(),
                    64
                );
                assert!(
                    scene
                        .stations()
                        .iter()
                        .take(MAX_NODES)
                        .all(|station| scene.workers[&station.id].retired.is_none())
                );
                let live: BTreeSet<_> = scene
                    .workers
                    .values()
                    .filter(|w| w.retired.is_none())
                    .map(|w| w.slot)
                    .collect();
                assert_eq!(live.len(), 64);
                let required: BTreeSet<_> = w
                    .nodes
                    .iter()
                    .flat_map(|node| [node.id.as_str(), node.parent_id.as_deref().unwrap()])
                    .map(|id| scene.slots[id])
                    .collect();
                assert_eq!(required.len(), 128);
                if generation > 0 {
                    assert!(required.iter().any(|&slot| slot >= 128));
                }
            }
        }
    }

    #[test]
    fn nonfinite_negative_zero_and_resume_dt_are_safe() {
        let w = workspace(vec![node("a", "root")]);
        let mut scene = Scene::new();
        settle(&mut scene, &w);
        for dt in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            let before = scene.workers["a"].particles;
            scene.update(Some("root"), &w, dt);
            assert_eq!(before, scene.workers["a"].particles);
        }
        let before = scene.workers["a"].particles;
        scene.update(Some("root"), &w, 1000.0);
        for (a, b) in before.iter().zip(scene.workers["a"].particles) {
            assert!(a.distance(b) <= 0.301);
        }
        assert!(scene.cubes().iter().all(Cube::valid));
    }

    #[test]
    fn a_worker_further_off_is_as_easy_to_point_at_as_one_3_m_away() {
        // Within 3 m, the sphere is as it was; beyond, it keeps its angle.
        assert_eq!(
            pointing_radius(Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)),
            POINTING_RADIUS
        );
        assert!((pointing_radius(Vec3::ZERO, Vec3::new(0.0, 0.0, -9.0)) - 0.9).abs() < 1e-5);
        let origin = Anchor {
            position: crate::swarm::CENTER + Vec3::new(0.0, 0.0, -6.0),
            orientation: Quat::IDENTITY,
        };
        let mut scene = Scene::new();
        scene.arrange(origin, crate::anchors::EYES, None, None);
        let w = workspace(vec![node("a", "root")]);
        scene.update(Some("root"), &w, 0.0);
        let anchor = scene.workers["a"].anchor;
        let eyes = Vec3::new(0.0, 1.6, 0.0);
        let distance = eyes.distance(anchor);
        assert!(distance > 7.0, "{distance}");
        // A ray passing 0.5 m beside it, 4° off at that distance, points at it, as 0.25 m does at 3.6 m.
        let aside = (anchor - eyes).cross(Vec3::Y).normalize();
        let at = |offset: f32| Ray {
            origin: eyes,
            direction: (anchor + aside * offset - eyes).normalize(),
        };
        assert_eq!(scene.point(Some(at(0.5))), Some("a"));
        assert_eq!(scene.point(Some(at(1.2))), None);
    }
}

#[cfg(test)]
mod space_tests {
    use super::*;
    use tomori_protocol::{SpaceState, Worker as SpaceWorker, WorkerState};

    const SPACE: &str = "space-1";

    fn worker(id: &str, parent: Option<&str>, state: WorkerState) -> SpaceWorker {
        SpaceWorker {
            id: id.into(),
            parent: parent.map(Into::into),
            title: format!("{id} title"),
            state,
        }
    }

    fn space(workers: Vec<SpaceWorker>) -> SpaceState {
        SpaceState {
            id: SPACE.into(),
            workers,
            ..Default::default()
        }
    }

    fn stations(scene: &Scene) -> BTreeMap<&str, State> {
        scene
            .stations()
            .iter()
            .map(|station| (station.id.as_str(), station.state))
            .collect()
    }

    fn show(scene: &mut Scene, workers: &mut Workers, space: Option<(&SpaceState, bool)>) {
        for _ in 0..10 {
            let (root, workspace) = workers.update(space);
            scene.update(root, workspace, 1.0 / 60.0);
        }
    }

    #[test]
    fn a_long_error_is_cut_on_a_character_boundary() {
        let mut state = space(vec![]);
        state.error = Some(format!("a{}", "é".repeat(2048)));
        let workspace = Workspace::from_space(&state, true);
        assert_eq!(workspace.error.as_ref().map(String::len), Some(4095));
        assert!(workspace.valid());
    }

    #[test]
    fn a_space_without_workers_is_an_empty_trusted_root() {
        let state = space(vec![]);
        let mut workers = Workers::default();
        let (root, workspace) = workers.update(Some((&state, true)));
        assert_eq!(root, Some(SPACE));
        assert!(workspace.nodes.is_empty());
        assert!(workspace.valid());
        assert!(!workspace.stale && !workspace.truncated && workspace.error.is_none());
        let mut scene = Scene::new();
        show(&mut scene, &mut workers, Some((&state, true)));
        assert!(scene.stations().is_empty());
        assert!(scene.cubes().is_empty());
    }

    #[test]
    fn a_top_level_worker_hangs_off_the_space() {
        let state = space(vec![worker("a", None, WorkerState::Running)]);
        let workspace = Workspace::from_space(&state, true);
        assert_eq!(workspace.nodes.len(), 1);
        assert_eq!(workspace.nodes[0].id, "a");
        assert_eq!(workspace.nodes[0].parent_id.as_deref(), Some(SPACE));
        assert_eq!(workspace.nodes[0].title, "a title");
        assert_eq!(workspace.nodes[0].state, State::Running);

        let mut scene = Scene::new();
        show(&mut scene, &mut Workers::default(), Some((&state, true)));
        assert_eq!(stations(&scene), BTreeMap::from([("a", State::Running)]));
        assert!(!scene.cubes().is_empty());
        assert!(scene.cubes().iter().all(Cube::valid));
    }

    #[test]
    fn a_nested_worker_keeps_its_parent() {
        let state = space(vec![
            worker("a", None, WorkerState::Running),
            worker("b", Some("a"), WorkerState::Waiting),
        ]);
        let workspace = Workspace::from_space(&state, true);
        assert_eq!(workspace.nodes[1].parent_id.as_deref(), Some("a"));

        let mut scene = Scene::new();
        show(&mut scene, &mut Workers::default(), Some((&state, true)));
        assert_eq!(
            stations(&scene),
            BTreeMap::from([("a", State::Running), ("b", State::Waiting)])
        );
    }

    #[test]
    fn every_worker_state_maps_to_its_station_state() {
        for (from, to) in [
            (WorkerState::Running, State::Running),
            (WorkerState::Waiting, State::Waiting),
            (WorkerState::Idle, State::Idle),
            (WorkerState::Completed, State::Completed),
            (WorkerState::Error, State::Error),
            (WorkerState::Stopped, State::Stopped),
            (WorkerState::Stalled, State::Stalled),
            (WorkerState::Unanswered, State::Unanswered),
            (WorkerState::Unknown, State::Unknown),
        ] {
            let workspace = Workspace::from_space(&space(vec![worker("a", None, from)]), true);
            assert_eq!(workspace.nodes[0].state, to);
        }
    }

    /// A stalled or unanswered worker is amber, deeper than a waiting one's, still at work, and drifts slowly. See
    /// `design/events.md`.
    #[test]
    fn a_stalled_or_unanswered_worker_is_amber_and_still_at_work() {
        for state in [State::Stalled, State::Unanswered] {
            let colour = palette(state);
            assert_eq!(colour, Vec3::new(1.00, 0.55, 0.10));
            assert_ne!(colour, palette(State::Waiting));
            assert_ne!(colour, palette(State::Error));
            assert!(is_working(state) && !is_terminal(state));
            let offset = flock_offset(3, 0.5, 1.0, state);
            assert!(offset.is_finite() && offset.length() < 0.2);
        }
        assert!(!is_working(State::Completed));
    }

    #[test]
    fn state_changes_reach_the_scene_and_removals_retire_workers() {
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        let running = space(vec![
            worker("a", None, WorkerState::Running),
            worker("b", Some("a"), WorkerState::Running),
        ]);
        show(&mut scene, &mut workers, Some((&running, true)));
        let completed = space(vec![
            worker("a", None, WorkerState::Running),
            worker("b", Some("a"), WorkerState::Completed),
        ]);
        show(&mut scene, &mut workers, Some((&completed, true)));
        assert_eq!(
            stations(&scene),
            BTreeMap::from([("a", State::Running), ("b", State::Completed)])
        );
        let failed = space(vec![worker("a", None, WorkerState::Error)]);
        show(&mut scene, &mut workers, Some((&failed, true)));
        assert_eq!(scene.workers["a"].state, State::Error);
        assert!(scene.workers["b"].retired.is_some());
    }

    #[test]
    fn workers_are_pointed_at_where_they_stand_and_only_while_they_stay() {
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        let both = space(vec![
            worker("a", None, WorkerState::Running),
            worker("b", None, WorkerState::Running),
        ]);
        show(&mut scene, &mut workers, Some((&both, true)));
        let anchor = |scene: &Scene, id: &str| {
            Vec3::from_array(
                scene
                    .stations()
                    .iter()
                    .find(|station| station.id == id)
                    .unwrap()
                    .position,
            )
        };
        let (a, b) = (anchor(&scene, "a"), anchor(&scene, "b"));
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let at = |target: Vec3| Ray::new(eye, target - eye);
        // Each worker's flock, leader and sign.
        assert_eq!(scene.targets().len(), 6);
        // Pointing tests what the targets describe.
        // Including a ray through both, which points at the nearer.
        let through = Ray::new(a + (a - b), b - a);
        for ray in [at(a), at(b), at(Vec3::ZERO), at((a + b) / 2.0), through] {
            let expected = ray
                .and_then(|ray| crate::pointing::nearest(&ray, &scene.targets()))
                .map(|hit| hit.key);
            assert_eq!(scene.point(ray), expected.as_deref());
        }
        assert_eq!(scene.point(at(a)), Some("a"));
        // The ray ends where it passes nearest the worker's anchor.
        assert!(scene.hit_point().unwrap().distance(a) < 1e-4);
        assert_eq!(scene.point(at(b)), Some("b"));
        // Passing within the pointing radius of a worker's anchor still points at it; the floor is nothing.
        let aside = (b - eye).cross(Vec3::Y).normalize();
        assert_eq!(
            scene.point(at(b + aside * (POINTING_RADIUS * 0.8))),
            Some("b")
        );
        assert!(scene.hit_point().unwrap().distance(b) < POINTING_RADIUS * 0.8);
        assert_eq!(scene.point(Ray::new(eye, Vec3::NEG_Y)), None);
        assert_eq!(scene.point(None), None);
        assert_eq!(scene.hit_point(), None);

        // The worker pointed at is drawn larger, and stays valid geometry.
        let size = |scene: &Scene| scene.cubes().iter().map(|cube| cube.scale[0]).sum::<f32>();
        show(&mut scene, &mut workers, Some((&both, true)));
        let plain = size(&scene);
        scene.point(at(a));
        show(&mut scene, &mut workers, Some((&both, true)));
        let pointed = size(&scene);
        assert!(pointed > plain * 1.05, "{pointed} {plain}");
        assert!(scene.cubes().iter().all(Cube::valid));

        // A worker that leaves can no longer be pointed at.
        let only_b = space(vec![worker("b", None, WorkerState::Running)]);
        show(&mut scene, &mut workers, Some((&only_b, true)));
        assert!(scene.workers["a"].retired.is_some());
        assert_eq!(scene.targets().len(), 3);
        assert_ne!(scene.point(at(a)), Some("a"));
    }

    #[test]
    fn a_worker_pointed_at_by_its_flock_leader_or_sign_highlights_all_three_for_either_hand() {
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        let both = space(vec![
            worker("a", None, WorkerState::Running),
            worker("b", None, WorkerState::Running),
        ]);
        show(&mut scene, &mut workers, Some((&both, true)));
        let station = |scene: &Scene, id: &str| {
            scene
                .stations()
                .iter()
                .find(|station| station.id == id)
                .unwrap()
                .clone()
        };
        let highlighted = |scene: &Scene| ["a", "b"].map(|id| station(scene, id).highlighted);
        assert_eq!(highlighted(&scene), [false, false]);
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let at = |target: Vec3| Ray::new(eye, target - eye);
        let a = station(&scene, "a");
        let (anchor, drawn) = (Vec3::from_array(a.position), a.drawn());
        // Its flock, its leader half way up to its sign, and its sign's middle each point at it, the ray ending there.
        for (target, part) in [
            (anchor, "flock"),
            (anchor + Vec3::Y * (SIGN_LIFT * drawn / 2.0), "leader"),
            (
                anchor + Vec3::Y * ((SIGN_LIFT + SIGN_TOP) * drawn / 2.0),
                "sign",
            ),
        ] {
            assert_eq!(scene.point(at(target)), Some("a"), "{part}");
            assert!(
                scene.hit_point().unwrap().distance(target) < SIGN_HALF * drawn,
                "{part}"
            );
            show(&mut scene, &mut workers, Some((&both, true)));
            // Its sign and leader are drawn highlighted with its flock; the other's are not.
            assert_eq!(highlighted(&scene), [true, false], "{part}");
        }
        // Above its tallest sign, nothing.
        assert_eq!(
            scene.point(at(anchor + Vec3::Y * (SIGN_TOP * drawn + 0.3))),
            None
        );
        show(&mut scene, &mut workers, Some((&both, true)));
        assert_eq!(highlighted(&scene), [false, false]);
        // Each hand highlights what it points at, and only that.
        let b = Vec3::from_array(station(&scene, "b").position);
        scene.point_with(Side::Right, at(anchor));
        scene.point_with(Side::Left, at(b));
        show(&mut scene, &mut workers, Some((&both, true)));
        assert_eq!(highlighted(&scene), [true, true]);
        scene.point_with(Side::Right, None);
        show(&mut scene, &mut workers, Some((&both, true)));
        assert_eq!(highlighted(&scene), [false, true]);
    }

    #[test]
    fn leaving_the_space_clears_the_workers() {
        let state = space(vec![worker("a", None, WorkerState::Running)]);
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        show(&mut scene, &mut workers, Some((&state, true)));
        assert!(!scene.stations().is_empty());
        let (root, workspace) = workers.update(None);
        assert_eq!(root, None);
        assert!(workspace.nodes.is_empty());
        show(&mut scene, &mut workers, None);
        assert!(scene.stations().is_empty());
        assert!(scene.cubes().is_empty());
    }

    #[test]
    fn joining_another_space_is_a_new_root() {
        let first = space(vec![worker("a", None, WorkerState::Running)]);
        let second = SpaceState {
            id: "space-2".into(),
            ..space(vec![worker("c", None, WorkerState::Idle)])
        };
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        show(&mut scene, &mut workers, Some((&first, true)));
        show(&mut scene, &mut workers, Some((&second, true)));
        assert_eq!(workers.update(Some((&second, true))).0, Some("space-2"));
        assert_eq!(stations(&scene), BTreeMap::from([("c", State::Idle)]));
    }

    #[test]
    fn a_lost_connection_is_stale_and_a_space_error_is_partial() {
        let state = space(vec![worker("a", None, WorkerState::Running)]);
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        show(&mut scene, &mut workers, Some((&state, true)));
        assert!(workers.update(Some((&state, false))).1.stale);
        show(&mut scene, &mut workers, Some((&state, false)));
        assert_eq!(stations(&scene), BTreeMap::from([("a", State::Unknown)]));

        // With the backend lost, an empty worker list is no evidence that the worker finished.
        let errored = SpaceState {
            error: Some("é".repeat(3000)),
            ..space(vec![])
        };
        let (_, workspace) = workers.update(Some((&errored, true)));
        assert!(workspace.valid());
        assert!(workspace.error.as_ref().unwrap().len() <= 4096);
        show(&mut scene, &mut workers, Some((&errored, true)));
        assert!(scene.workers["a"].retired.is_none());
    }

    #[test]
    fn worker_freshness_is_independent_of_an_unrelated_space_error() {
        let mut state = space(vec![worker("a", None, WorkerState::Completed)]);
        state.error = Some("unrelated agent error".into());
        state.workers_authoritative = Some(true);
        let mut workers = Workers::default();
        let mut scene = Scene::new();
        show(&mut scene, &mut workers, Some((&state, true)));
        assert!(scene.workers.contains_key("a"));

        // A partial list does not prove that the absent worker was deleted.
        state.workers.clear();
        state.workers_authoritative = Some(false);
        show(&mut scene, &mut workers, Some((&state, true)));
        assert!(scene.workers["a"].retired.is_none());
        assert_eq!(
            workers.update(Some((&state, true))).1.error.as_deref(),
            Some("unrelated agent error")
        );

        // A later authoritative worker update may retire it even while the
        // unrelated error remains visible; freshness alone changed here.
        state.workers_authoritative = Some(true);
        show(&mut scene, &mut workers, Some((&state, true)));
        assert!(scene.workers["a"].retired.is_some());
        assert_eq!(
            workers.update(Some((&state, true))).1.error.as_deref(),
            Some("unrelated agent error")
        );
    }

    #[test]
    fn out_of_contract_workers_are_omitted_as_truncated() {
        let mut list: Vec<_> = (0..MAX_NODES + 3)
            .map(|i| worker(&format!("w{i}"), None, WorkerState::Running))
            .collect();
        list.insert(0, worker("", None, WorkerState::Running));
        let long = "p".repeat(MAX_ID_BYTES + 1);
        list.insert(1, worker("bad-parent", Some(&long), WorkerState::Running));
        list.last_mut().unwrap().title = "t".repeat(400);
        let workspace = Workspace::from_space(&space(list.clone()), true);
        assert_eq!(workspace.nodes.len(), MAX_NODES);
        // The server lists the oldest first; the newest are kept.
        assert_eq!(workspace.nodes[0].id, "w3");
        assert_eq!(
            workspace.nodes[MAX_NODES - 1].id,
            format!("w{}", MAX_NODES + 2)
        );
        assert_eq!(workspace.nodes[MAX_NODES - 1].title.chars().count(), 160);
        assert!(workspace.truncated);
        assert!(workspace.valid());

        let mut scene = Scene::new();
        show(
            &mut scene,
            &mut Workers::default(),
            Some((&space(list), true)),
        );
        assert_eq!(scene.stations().len(), MAX_NODES);
    }

    #[test]
    fn a_parent_cycle_is_rejected_by_validation() {
        let state = space(vec![
            worker("a", Some("b"), WorkerState::Running),
            worker("b", Some("a"), WorkerState::Running),
        ]);
        assert!(!Workspace::from_space(&state, true).valid());
        let mut scene = Scene::new();
        show(&mut scene, &mut Workers::default(), Some((&state, true)));
        assert!(scene.stations().is_empty());
    }

    #[test]
    fn a_worker_whose_parent_is_missing_keeps_that_parent() {
        let state = space(vec![worker("b", Some("gone"), WorkerState::Running)]);
        let workspace = Workspace::from_space(&state, true);
        assert!(workspace.valid());
        assert_eq!(workspace.nodes[0].parent_id.as_deref(), Some("gone"));
    }

    #[test]
    fn timers_stand_as_dials_above_the_agent_among_the_workers_and_the_nearer_is_pointed_at() {
        use tomori_protocol::{Timer, TimerState};
        const NOW: f64 = 1_800_000_000.0;
        let timers: Vec<Timer> = (0..8)
            .map(|index| {
                Timer::new(
                    format!("t{index}"),
                    "every hour",
                    "when present",
                    Some(NOW as u64 + 600),
                    TimerState::Waiting,
                )
            })
            .collect();
        // Many workers, and eight timers: the dials stand where they would with none, clear of every worker.
        let mut state = space(
            (0..MAX_NODES)
                .map(|index| worker(&format!("w{index:02}"), None, WorkerState::Running))
                .collect(),
        );
        state.timers = timers.clone();
        let mut scene = Scene::new();
        let mut workers = Workers::default();
        let reach = crate::dials::SWARM_REACH;
        scene.set_timers(&state.timers, Anchor::AGENT, reach, NOW);
        scene.set_timers(&state.timers, Anchor::AGENT, reach, NOW);
        show(&mut scene, &mut workers, Some((&state, true)));
        for _ in 0..40 {
            let (root, workspace) = workers.update(Some((&state, true)));
            scene.update(root, workspace, 0.05);
        }
        assert_eq!(scene.dials().len(), 8);
        let places = crate::dials::places(8, &scene.room);
        for (dial, (place, _)) in scene.dials().iter().zip(&places) {
            assert_eq!(Vec3::from_array(dial.position), *place);
        }
        assert!(scene.cubes().len() <= MAX_CUBES);
        assert!(scene.cubes().len() > MAX_NODES * PARTICLES);
        for station in scene.stations() {
            for (place, _) in &places {
                assert!(Vec3::from_array(station.position).distance(*place) > 0.75);
            }
        }
        // The dial the ray meets, nearer than any worker, is pointed at, and the ray ends on it.
        let first = places[0].0;
        let eyes = crate::anchors::EYES;
        assert_eq!(scene.point(Ray::new(eyes, first - eyes)), None);
        assert_eq!(scene.pointed_timer(), Some("t0"));
        assert!(scene.hit_point().unwrap().distance(first) < 1e-3);
        // A worker nearer along the same ray is pointed at instead.
        let (id, nearer) = scene
            .workers
            .iter()
            .map(|(id, worker)| (id.clone(), worker.anchor))
            .next()
            .unwrap();
        let behind = nearer + (nearer - eyes).normalize() * 0.5;
        let mut close = Scene::new();
        close
            .workers
            .insert(id.clone(), scene.workers.remove(&id).unwrap());
        close.dials = std::mem::take(&mut scene.dials);
        close.dials.update(
            &timers[..1],
            true,
            &crate::dials::Room::open(
                Anchor {
                    position: behind
                        - Vec3::Y * (reach.y + crate::dials::CLEAR + crate::dials::RADIUS),
                    ..Anchor::AGENT
                },
                eyes,
            ),
            NOW,
            1.0,
            &mut Vec::new(),
        );
        for _ in 0..40 {
            close.dials.update(
                &timers[..1],
                true,
                &crate::dials::Room::open(
                    Anchor {
                        position: behind
                            - Vec3::Y * (reach.y + crate::dials::CLEAR + crate::dials::RADIUS),
                        ..Anchor::AGENT
                    },
                    eyes,
                ),
                NOW,
                0.05,
                &mut Vec::new(),
            );
        }
        assert_eq!(
            close.point(Ray::new(eyes, nearer - eyes)),
            Some(id.as_str())
        );
        assert_eq!(close.pointed_timer(), None);
        // Nothing pointed at, neither.
        assert_eq!(scene.point(None), None);
        assert_eq!(scene.pointed_timer(), None);
        // An avatar's reach as it grows is taken at once, and as it shrinks only past SETTLE, so that an animated one
        // does not move the dials back and forth.
        let grown = reach + Vec3::Y * 0.2;
        scene.set_timers(&timers, Anchor::AGENT, grown, NOW);
        assert_eq!(scene.room.agent, grown);
        scene.set_timers(&timers, Anchor::AGENT, grown - Vec3::Y * 0.1, NOW);
        assert_eq!(scene.room.agent, grown);
        scene.set_timers(&timers, Anchor::AGENT, reach, NOW);
        assert_eq!(scene.room.agent, grown);
        scene.set_timers(&timers, Anchor::AGENT, reach - Vec3::Y * 0.1, NOW);
        assert_eq!(scene.room.agent, reach - Vec3::Y * 0.1);
        scene.set_timers(&timers, Anchor::AGENT, reach, NOW);
        // Outside a space, or in another, the dials are forgotten.
        let (root, workspace) = workers.update(None);
        scene.update(root, workspace, 0.05);
        assert!(scene.dials().is_empty());
        scene.set_timers(&timers, Anchor::AGENT, reach, NOW);
        let other = SpaceState {
            id: "space-2".into(),
            timers: timers[..1].to_vec(),
            ..Default::default()
        };
        let (root, workspace) = workers.update(Some((&other, true)));
        scene.update(root, workspace, 0.05);
        assert_eq!(scene.dials().len(), 8);
        assert!(scene.dials()[0].opacity < 0.5, "growing in afresh");
    }

    /// The farmyard's home, its eyes at the arrival, and its ring, all but straight behind the agent.
    const FARMYARD_HOME: Vec3 = Vec3::new(-2.5, 1.5, -0.84);
    const FARMYARD_EYES: Vec3 = Vec3::new(0.6, 1.6, -2.4);

    /// The dials and the workers in `scene`, with `workers` workers and eight timers, laid out by `arrange`, stand clear
    /// of one another and their signs, and within the room's ceiling and floor.
    fn clear_of_every_worker(
        arrange: impl Fn(&mut Scene),
        home: Anchor,
        eyes: Vec3,
        walls: &[tomori_protocol::Wall],
    ) -> Vec<Vec3> {
        use tomori_protocol::{Timer, TimerState};
        const NOW: f64 = 1_800_000_000.0;
        let timers: Vec<Timer> = (0..8)
            .map(|index| {
                Timer::new(
                    format!("t{index}"),
                    "every hour",
                    "when present",
                    Some(NOW as u64 + 600),
                    TimerState::Waiting,
                )
            })
            .collect();
        let mut stood = Vec::new();
        for count in [0, 8, MAX_NODES] {
            let mut state = space(
                (0..count)
                    .map(|index| worker(&format!("w{index:02}"), None, WorkerState::Running))
                    .collect(),
            );
            state.timers = timers.clone();
            let mut scene = Scene::new();
            arrange(&mut scene);
            let mut workers = Workers::default();
            for _ in 0..60 {
                scene.set_timers(&state.timers, home, crate::dials::SWARM_REACH, NOW);
                scene.set_walls(walls);
                let (root, workspace) = workers.update(Some((&state, true)));
                scene.update(root, workspace, 0.05);
            }
            assert_eq!(scene.dials().len(), 8);
            let dials: Vec<crate::dials::Extent> = scene
                .dials()
                .iter()
                .map(|dial| crate::dials::Extent::dial(Vec3::from_array(dial.position), dial.scale))
                .collect();
            for (index, dial) in dials.iter().enumerate() {
                assert!(dial.low >= scene.room.floor - 1e-4);
                assert!(
                    scene
                        .room
                        .ceiling
                        .is_none_or(|ceiling| dial.high <= ceiling + 1e-4)
                );
                // Clear of the agent's reach, above or below it.
                assert!(
                    dial.low >= home.position.y + crate::dials::SWARM_REACH.y
                        || dial.high <= home.position.y - crate::dials::SWARM_REACH.y
                );
                for other in &dials[..index] {
                    assert!(!dial.overlaps(other, 0.0));
                }
                for slot in crate::dials::Extent::walls(walls) {
                    assert!(
                        !dial.overlaps(&slot, 0.0),
                        "{count} workers: a wall's slot at {} and a dial at {}",
                        slot.centre,
                        dial.centre
                    );
                }
                for (id, worker) in &scene.workers {
                    let extent = crate::dials::Extent::worker(worker.place, worker.scale, eyes);
                    assert!(
                        !dial.overlaps(&extent, 0.0),
                        "{count} workers: {id} at {} and a dial at {}",
                        worker.place,
                        dial.centre
                    );
                }
            }
            stood = dials.iter().map(|dial| dial.centre).collect();
        }
        stood
    }

    #[test]
    fn on_today_s_arc_the_dials_stand_above_the_agent_clear_of_every_worker() {
        let stood = clear_of_every_worker(|_| {}, Anchor::AGENT, crate::anchors::EYES, &[]);
        assert!(stood.iter().all(|at| at.y > Anchor::AGENT.position.y));
        // With the status wall beneath the agent, where the server stands it by default.
        let stood = clear_of_every_worker(
            |_| {},
            Anchor::AGENT,
            crate::anchors::EYES,
            &[status_wall(Anchor::AGENT.position, crate::anchors::EYES)],
        );
        assert!(stood.iter().all(|at| at.y > Anchor::AGENT.position.y));
    }

    #[test]
    fn in_a_home_with_room_the_dials_stand_clear_of_the_arc_grid_and_compact_steps() {
        let home = Anchor::AGENT;
        let stood = clear_of_every_worker(
            |scene| {
                scene.arrange(
                    home,
                    crate::anchors::EYES,
                    Some(tomori_protocol::cluster::Extent::new(4.2, 1.6, 2.0)),
                    None,
                );
            },
            home,
            crate::anchors::EYES,
            &[status_wall(home.position, crate::anchors::EYES)],
        );
        assert_eq!(stood.len(), 8);
    }

    /// The space's own status wall, the Tomori wall, where the server stands it by default: 0.88 m beneath the agent's
    /// home `at`, curving about `eyes`.
    fn status_wall(at: Vec3, eyes: Vec3) -> tomori_protocol::Wall {
        let middle = at - Vec3::Y * 0.88;
        tomori_protocol::Wall::new(
            tomori_protocol::TOMORI_WALL,
            tomori_protocol::WallKind::Tomori,
            tomori_protocol::Pose::upright_facing(middle.to_array(), [eyes.x, middle.y, eyes.z]),
        )
        .with_shape(tomori_protocol::WallShape::Arc { radius: 2.3 })
    }

    fn farmyard() -> (Anchor, impl Fn(&mut Scene)) {
        let pose = tomori_protocol::Pose::at(FARMYARD_HOME.to_array())
            .facing_towards(FARMYARD_EYES.to_array());
        let home = Anchor::from_pose(&pose).unwrap();
        (home, move |scene: &mut Scene| {
            scene.arrange_as(
                home,
                FARMYARD_EYES,
                None,
                (tomori_protocol::cluster::Formation::Ring, Some(300.0)),
                None,
            );
        })
    }

    #[test]
    fn in_the_farmyard_s_ring_the_dials_stand_beneath_the_agent_within_its_height() {
        // Its status wall is its notice board, one of its own walls, across the yard.
        let board = tomori_protocol::Wall::new(
            "noticeboard",
            tomori_protocol::WallKind::Environment,
            tomori_protocol::Pose::upright_facing([-4.0, 1.5, -6.0], FARMYARD_EYES.to_array()),
        )
        .with_status(true);
        let (home, arrange) = farmyard();
        let stood = clear_of_every_worker(arrange, home, FARMYARD_EYES, &[board]);
        // Above the agent the ring keeps nothing over 2.5 m, so the dials stand beneath it.
        assert!(stood.iter().all(|at| at.y < FARMYARD_HOME.y));
    }

    #[test]
    fn in_a_ring_over_the_default_status_wall_the_dials_stand_clear_of_its_slots() {
        let (home, arrange) = farmyard();
        let wall = status_wall(FARMYARD_HOME, FARMYARD_EYES);
        let stood =
            clear_of_every_worker(arrange, home, FARMYARD_EYES, std::slice::from_ref(&wall));
        assert_eq!(stood.len(), 8);
        // And at the default home's height, lower over its floor.
        let low = Vec3::new(FARMYARD_HOME.x, 1.4, FARMYARD_HOME.z);
        let pose =
            tomori_protocol::Pose::at(low.to_array()).facing_towards(FARMYARD_EYES.to_array());
        let home = Anchor::from_pose(&pose).unwrap();
        clear_of_every_worker(
            |scene| {
                scene.arrange_as(
                    home,
                    FARMYARD_EYES,
                    None,
                    (tomori_protocol::cluster::Formation::Ring, Some(300.0)),
                    None,
                );
            },
            home,
            FARMYARD_EYES,
            &[status_wall(low, FARMYARD_EYES)],
        );
    }
}

#[cfg(test)]
mod clusters;
