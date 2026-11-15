// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A volume's live layout while someone pulls its nodes about by hand: Hooke's springs along its edges, each at rest at
//! the length the server laid it out at, a push between nodes that come too close, a weak spring holding each node
//! home, and damping, stepped at a fixed rate in the volume's own millimetres. A node held is fixed where the hand
//! holds it; while one alone is held, the nodes its edges come from fan out to its left and those they go to its right,
//! as its holder sees it, and the rest fall back, away from them. Let go, a node stays pinned exactly where it was let
//! go, within the volume or beyond it, the rest settling about it, until [`Springs::unpin`] puts every node back. With
//! nothing held or pinned, every node springs home, and once all are there it is done. It is this participant's alone and sends nothing. See `design/diagrams.md`.
use glam::Vec3;
use tomori_protocol::diagram::{Arrow, Diagram};

/// How long a step is, in seconds, and the most a frame takes, so that a slow frame never runs away.
pub const STEP: f32 = 1.0 / 60.0;
pub const MOST_STEPS: usize = 4;
/// How stiff each spring is, as an acceleration for each millimetre it is stretched, per second squared: along an
/// edge; holding a node home, weakly while something is held, so that it follows, more firmly while only pins hold
/// some, so that the rest settle about them soon, and pulling it home once nothing is; fanning a held node's
/// neighbours; and pushing apart two nodes nearer than [`APART_MM`].
const EDGE: f32 = 40.0;
const HOME_HELD: f32 = 3.0;
const HOME_PINNED: f32 = 20.0;
const HOME_FREE: f32 = 30.0;
const FAN: f32 = 70.0;
const APART: f32 = 160.0;
/// How near two nodes' middles come before they push apart, in millimetres.
pub const APART_MM: f32 = 85.0;
/// How far from a held node its neighbours fan, in millimetres, and across how wide an arc, in radians either way.
pub const FAN_MM: f32 = 150.0;
const FAN_ARC: f32 = 0.6;
/// How hard the nodes not joined to the one held fall back, away from its holder, in millimetres per second squared.
const BACK: f32 = 450.0;
/// How quickly motion dies away, as a share of speed lost each second, and the fastest a node moves, in millimetres a
/// second.
const DAMPING: f32 = 7.0;
const FASTEST: f32 = 1_500.0;
/// How slowly every node must move, in millimetres a second, for how long, in seconds, before the layout sleeps; and
/// how near home each must be, in millimetres, for it to be done once nothing is held or pinned.
const STILL_SPEED: f32 = 2.0;
const STILL_FOR: f32 = 0.5;
const HOME_MM: f32 = 0.5;

/// Which way an edge points, between its ends by their indices.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spring {
    from: usize,
    to: usize,
    rest: f32,
    /// Whether it points from `from` to `to`, the other way, or both ways or neither.
    way: Way,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    Forward,
    Back,
    Both,
}

/// Where a held node's neighbour fans: to its left, where edges come from, to its right, where they go, or above, both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    In,
    Out,
    Both,
}

/// A volume's live layout. See the module's documentation.
#[derive(Clone, Debug)]
pub struct Springs {
    home: Vec<Vec3>,
    at: Vec<Vec3>,
    velocity: Vec<Vec3>,
    pins: Vec<Option<Vec3>>,
    /// The node each hand holds, by index, and where it holds it: the right hand's, then the left's.
    held: [Option<(usize, Vec3)>; 2],
    springs: Vec<Spring>,
    /// Where the holder's eyes are, in the volume's millimetres, whose left and right the fan takes.
    viewer: Vec3,
    /// What is left over of the last frame's time, short of a step.
    spare: f32,
    /// How long every node has moved slowly enough to sleep.
    still: f32,
}

impl Springs {
    /// A layout of `diagram` as the server laid it out, every node home.
    pub fn new(diagram: &Diagram) -> Self {
        let home: Vec<Vec3> = diagram
            .nodes
            .iter()
            .map(|node| Vec3::from_array(node.at.map(f32::from)))
            .collect();
        let springs = diagram
            .edges
            .iter()
            .filter(|edge| usize::from(edge.from) < home.len() && usize::from(edge.to) < home.len())
            .map(|edge| {
                let (from, to) = (usize::from(edge.from), usize::from(edge.to));
                Spring {
                    from,
                    to,
                    rest: home[from].distance(home[to]),
                    way: match edge.direction {
                        Arrow::Forward => Way::Forward,
                        Arrow::Back => Way::Back,
                        _ => Way::Both,
                    },
                }
            })
            .collect();
        Self {
            at: home.clone(),
            velocity: vec![Vec3::ZERO; home.len()],
            pins: vec![None; home.len()],
            home,
            held: [None, None],
            springs,
            viewer: Vec3::Z * 1_000.0,
            spare: 0.0,
            still: 0.0,
        }
    }

    /// A layout of `diagram`, whose content changed from `was`, laid out as `old` was: each node of the same id where it
    /// stood, held or pinned as it was, the rest home, and every node's home where the server now laid it out, so that
    /// nothing is named by an index that now names another.
    pub fn carried(old: &Springs, was: &Diagram, diagram: &Diagram) -> Self {
        let mut springs = Self::new(diagram);
        let now = |index: usize| was.nodes.get(index).and_then(|node| diagram.node(&node.id));
        for (index, at) in old.at.iter().enumerate() {
            if let Some(now) = now(index) {
                springs.at[now] = *at;
                springs.pins[now] = old.pins[index];
            }
        }
        for (hand, held) in old.held.iter().enumerate() {
            springs.held[hand] = held.and_then(|(index, at)| Some((now(index)?, at)));
        }
        springs.viewer = old.viewer;
        springs
    }

    /// Where each node is now, in the volume's millimetres.
    pub fn positions(&self) -> &[Vec3] {
        &self.at
    }

    /// The node `hand`, 0 the right and 1 the left, holds, if any.
    pub fn held(&self, hand: usize) -> Option<usize> {
        self.held[hand].map(|(node, _)| node)
    }

    /// Whether `node` is pinned.
    pub fn pinned(&self, node: usize) -> bool {
        self.pins.get(node).is_some_and(Option::is_some)
    }

    /// Whether any node is pinned.
    pub fn any_pinned(&self) -> bool {
        self.pins.iter().any(Option::is_some)
    }

    /// Hold `node` with `hand` at `target`, taking it from the other hand if that held it, and from its pin.
    pub fn hold(&mut self, hand: usize, node: usize, target: Vec3) {
        if node >= self.at.len() {
            return;
        }
        if self.held[1 - hand].is_some_and(|(held, _)| held == node) {
            self.held[1 - hand] = None;
        }
        self.pins[node] = None;
        self.held[hand] = Some((node, target));
        self.still = 0.0;
    }

    /// Move what `hand` holds to `target`.
    pub fn move_to(&mut self, hand: usize, target: Vec3) {
        if let Some((_, at)) = &mut self.held[hand] {
            *at = target;
        }
    }

    /// Let go of what `hand` holds, pinned exactly where it is held.
    pub fn let_go(&mut self, hand: usize) {
        if let Some((node, target)) = self.held[hand].take() {
            self.pins[node] = Some(target);
            self.at[node] = target;
            self.velocity[node] = Vec3::ZERO;
            self.still = 0.0;
        }
    }

    /// Unpin every node, so that each springs home.
    pub fn unpin(&mut self) {
        self.pins.iter_mut().for_each(|pin| *pin = None);
        self.still = 0.0;
    }

    /// Whether nothing is held or pinned and every node is home, so that the server's layout is drawn again.
    pub fn done(&self) -> bool {
        self.held.iter().all(Option::is_none)
            && !self.any_pinned()
            && self
                .at
                .iter()
                .zip(&self.home)
                .all(|(at, home)| at.distance(*home) <= HOME_MM)
    }

    /// Whether it sleeps: nothing held, and every node still for a while.
    pub fn asleep(&self) -> bool {
        self.held.iter().all(Option::is_none) && self.still >= STILL_FOR
    }

    /// The neighbours of `node`, each with where it fans: where an edge from it comes from, goes to, or both.
    pub fn neighbours(&self, node: usize) -> Vec<(usize, Side)> {
        let mut found: Vec<(usize, Side)> = Vec::new();
        for spring in &self.springs {
            let (other, side) = if spring.to == node {
                (
                    spring.from,
                    match spring.way {
                        Way::Forward => Side::In,
                        Way::Back => Side::Out,
                        Way::Both => Side::Both,
                    },
                )
            } else if spring.from == node {
                (
                    spring.to,
                    match spring.way {
                        Way::Forward => Side::Out,
                        Way::Back => Side::In,
                        Way::Both => Side::Both,
                    },
                )
            } else {
                continue;
            };
            match found.iter_mut().find(|(known, _)| *known == other) {
                Some((_, known)) if *known != side => *known = Side::Both,
                Some(_) => {}
                None => found.push((other, side)),
            }
        }
        found
    }

    /// Run it on by `dt` seconds, its holder's eyes at `viewer`, in the volume's millimetres, in whole steps of
    /// [`STEP`], at most [`MOST_STEPS`] of them; asleep, it does nothing.
    pub fn step(&mut self, dt: f32, viewer: Vec3) {
        if viewer.is_finite() {
            self.viewer = viewer;
        }
        if self.asleep() || !dt.is_finite() {
            return;
        }
        self.spare = (self.spare + dt.max(0.0)).min(STEP * MOST_STEPS as f32);
        while self.spare >= STEP {
            self.spare -= STEP;
            self.tick();
        }
    }

    /// One step of [`STEP`].
    fn tick(&mut self) {
        let count = self.at.len();
        let held: Vec<(usize, Vec3)> = self.held.iter().flatten().copied().collect();
        let mut force = vec![Vec3::ZERO; count];
        let busy = !held.is_empty() || self.any_pinned();
        if busy {
            for spring in &self.springs {
                let apart = self.at[spring.to] - self.at[spring.from];
                let length = apart.length();
                if length > 1e-3 {
                    let pull = apart / length * (length - spring.rest) * EDGE;
                    force[spring.from] += pull;
                    force[spring.to] -= pull;
                }
            }
            for a in 0..count {
                for b in a + 1..count {
                    let apart = self.at[b] - self.at[a];
                    let length = apart.length();
                    if length < APART_MM {
                        let direction = apart.try_normalize().unwrap_or(Vec3::X);
                        let push = direction * (APART_MM - length) * APART;
                        force[a] -= push;
                        force[b] += push;
                    }
                }
            }
        }
        let home = if !held.is_empty() {
            HOME_HELD
        } else if busy {
            HOME_PINNED
        } else {
            HOME_FREE
        };
        for (node, force) in force.iter_mut().enumerate() {
            *force += (self.home[node] - self.at[node]) * home;
        }
        // One node held alone: its neighbours fan to either side of it, as its holder sees it, the rest fall back.
        if let [(node, _)] = held[..] {
            let centre = self.at[node];
            let towards = (self.viewer - centre).try_normalize().unwrap_or(Vec3::Z);
            let right = Vec3::Y.cross(towards).try_normalize().unwrap_or(Vec3::X);
            let up = towards.cross(right);
            let mut neighbours = self.neighbours(node);
            // Each side's from the highest down, so that they keep the order they stand in.
            neighbours.sort_by(|a, b| {
                a.1.cmp(&b.1)
                    .then(self.at[b.0].dot(up).total_cmp(&self.at[a.0].dot(up)))
            });
            for side in [Side::In, Side::Out, Side::Both] {
                let these: Vec<usize> = neighbours
                    .iter()
                    .filter(|(_, fans)| *fans == side)
                    .map(|(other, _)| *other)
                    .collect();
                for (index, other) in these.iter().enumerate() {
                    let share = if these.len() > 1 {
                        index as f32 / (these.len() - 1) as f32
                    } else {
                        0.5
                    };
                    let angle = FAN_ARC * (1.0 - 2.0 * share);
                    let (across, along) = match side {
                        Side::In => (-right, up),
                        Side::Out => (right, up),
                        Side::Both => (up, -right),
                    };
                    let target = centre + (across * angle.cos() + along * angle.sin()) * FAN_MM;
                    force[*other] += (target - self.at[*other]) * FAN;
                }
            }
            for (other, force) in force.iter_mut().enumerate() {
                if other != node && !neighbours.iter().any(|(known, _)| *known == other) {
                    *force -= towards * BACK;
                }
            }
        }
        let keep = (1.0 - DAMPING * STEP).max(0.0);
        let mut fastest: f32 = 0.0;
        for node in 0..count {
            let fixed = held
                .iter()
                .find(|(held, _)| *held == node)
                .map(|(_, at)| *at)
                .or(self.pins[node]);
            if let Some(at) = fixed {
                self.at[node] = at;
                self.velocity[node] = Vec3::ZERO;
                continue;
            }
            let velocity =
                ((self.velocity[node] + force[node] * STEP) * keep).clamp_length_max(FASTEST);
            self.velocity[node] = velocity;
            self.at[node] += velocity * STEP;
            fastest = fastest.max(velocity.length());
        }
        if fastest < STILL_SPEED {
            self.still += STEP;
        } else {
            self.still = 0.0;
        }
    }
}

/// The shortest path between the nodes `from` and `to` of `diagram`, by its edges either way, as the nodes along it
/// and the edges between them, by their indices; the first found of those as short, in the order written. `None` where
/// none joins them.
pub fn path(diagram: &Diagram, from: usize, to: usize) -> Option<(Vec<usize>, Vec<usize>)> {
    let count = diagram.nodes.len();
    if from >= count || to >= count {
        return None;
    }
    // Each node reached, by the node and edge it was reached by.
    let mut came: Vec<Option<(usize, usize)>> = vec![None; count];
    let mut seen = vec![false; count];
    seen[from] = true;
    let mut queue = std::collections::VecDeque::from([from]);
    while let Some(node) = queue.pop_front() {
        if node == to {
            break;
        }
        for (index, edge) in diagram.edges.iter().enumerate() {
            let (a, b) = (usize::from(edge.from), usize::from(edge.to));
            let other = if a == node {
                b
            } else if b == node {
                a
            } else {
                continue;
            };
            if other < count && !seen[other] {
                seen[other] = true;
                came[other] = Some((node, index));
                queue.push_back(other);
            }
        }
    }
    if !seen[to] {
        return None;
    }
    let (mut nodes, mut edges) = (vec![to], Vec::new());
    let mut at = to;
    while let Some((previous, edge)) = came[at] {
        nodes.push(previous);
        edges.push(edge);
        at = previous;
    }
    nodes.reverse();
    edges.reverse();
    Some((nodes, edges))
}

#[cfg(test)]
mod tests;
