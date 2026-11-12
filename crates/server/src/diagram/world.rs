// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A diagram's world references, resolved on the server against the things its space has, as the agent's own tools
//! read them, within the space alone: which thing each names, where it stands, how large it is, and what the agent is
//! told of it, or, for one that names nothing here, a warning with a suggestion. Then, laying the diagram out, a node
//! that points at a thing is pulled towards the side of the volume facing it, by a spring that weakens with the
//! thing's distance, so that its leader line leaves the volume short and uncrossed; and `place: near references` stands
//! the volume between whoever asked and what it points at. Positions are taken as they are now, once, as the diagram is
//! shown or changed; following what moves comes in a later stage. See `design/diagrams.md`.
use tomori_protocol::Pose;
use tomori_protocol::diagram::{Diagram, Warning};
use tomori_protocol::world::{Frame, Kind, Reference, State};

use super::{Spot, length, normalised};

/// A thing in the space a world reference may name, as the hub finds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Thing {
    /// The names it is found by, the first the one the agent is told: an object's id and its reference, an anchor's
    /// name, a wall's identity and names, a panel's id, the agent's id for it and its title, a worker's session and
    /// title, and a person's handle and name.
    pub names: Vec<String>,
    /// What its end is labelled with where the agent gave no label: an object's description to its first comma, an
    /// anchor's label, a panel's title, a person's name.
    pub label: String,
    /// What the agent is told it is, on one line.
    pub described: String,
    /// Where its middle stands, in the space's coordinates, while it has a place.
    pub at: Option<[f32; 3]>,
    pub radius: f32,
    pub frame: Option<Frame>,
    /// Why it has no place, where it has none, as the agent is told: "held by someone", "away".
    pub absent: Option<String>,
}

/// The things of a space world references may name, of each kind, as the hub finds them for its agent, and whose eyes
/// the agent is told distances from.
#[derive(Debug, Default)]
pub(crate) struct Things {
    /// The environment the space is in now, whose objects alone are named.
    pub environment: String,
    pub objects: Vec<Thing>,
    pub anchors: Vec<Thing>,
    pub walls: Vec<Thing>,
    pub panels: Vec<Thing>,
    pub workers: Vec<Thing>,
    pub people: Vec<Thing>,
    pub agent: Option<Thing>,
    /// Whoever asked, by name, and their eyes.
    pub asker: Option<(String, [f32; 3])>,
}

/// How many things of a kind a warning lists, as "there are a, b and c", beyond which it names the tool that lists them.
const LISTED: usize = 6;

/// The kinds a reference written without its kind, and without a `:`, is tried as, in order.
const INFERRED: [Kind; 4] = [Kind::Anchor, Kind::Wall, Kind::Panel, Kind::Person];

impl Things {
    fn of(&self, kind: Kind) -> &[Thing] {
        match kind {
            Kind::Object => &self.objects,
            Kind::Anchor => &self.anchors,
            Kind::Wall => &self.walls,
            Kind::Panel => &self.panels,
            Kind::Worker => &self.workers,
            Kind::Person => &self.people,
            Kind::Agent => self.agent.as_slice(),
            _ => &[],
        }
    }

    /// The thing of `kind` `name` names, or why none: none of that kind is called it here, or several are.
    fn find(&self, kind: Kind, name: &str) -> Result<&Thing, String> {
        if kind == Kind::Agent {
            return self
                .agent
                .as_ref()
                .ok_or_else(|| "the agent is not in this space".to_owned());
        }
        let wanted = key(name);
        let named: Vec<&Thing> = self
            .of(kind)
            .iter()
            .filter(|thing| thing.names.iter().any(|known| key(known) == wanted))
            .collect();
        match named.as_slice() {
            [one] => Ok(one),
            [] => Err(self.unknown(kind, name)),
            several => {
                // Named exactly by its first name, as a handle names one person, it is that one.
                if let Some(exact) = several.iter().find(|thing| {
                    thing
                        .names
                        .first()
                        .is_some_and(|first| first == name.trim())
                }) {
                    return Ok(exact);
                }
                let names: Vec<&str> = several
                    .iter()
                    .take(LISTED)
                    .filter_map(|thing| thing.names.first().map(String::as_str))
                    .collect();
                Err(format!(
                    "more than one {} here is called {}: say which, as {}",
                    kind.name(),
                    quoted(name),
                    names.join(" or ")
                ))
            }
        }
    }

    /// Why nothing of `kind` is called `name` here, with a suggestion: the name of one within two edits, or the few
    /// there are, or the tool that lists them. Never whether such a thing is anywhere else.
    fn unknown(&self, kind: Kind, name: &str) -> String {
        let things = self.of(kind);
        let mut text = format!("no {} {} in this space", kind.name(), quoted(name));
        let close = things
            .iter()
            .filter_map(|thing| thing.names.first())
            .find(|known| tomori_protocol::diagram::edits(&key(known), &key(name)) <= 2);
        if let Some(close) = close {
            text.push_str(&format!(" (did you mean {close}?)"));
            return text;
        }
        if kind == Kind::Object
            && let Some((environment, _)) = name.rsplit_once(':')
            && environment != self.environment
        {
            text.push_str(&format!(
                ": it is in the {} environment now",
                crate::space::one_line(&self.environment, 32)
            ));
            return text;
        }
        let names: Vec<&str> = things
            .iter()
            .filter_map(|thing| thing.names.first().map(String::as_str))
            .collect();
        match names.len() {
            0 => {}
            count if count <= LISTED => text.push_str(&format!(" (there are {})", listed(&names))),
            _ => text.push_str(&format!(
                " ({} lists them)",
                match kind {
                    Kind::Panel => "panels",
                    Kind::Worker => "workers",
                    _ => "presence",
                }
            )),
        }
        text
    }
}

/// `name` as it is compared: trimmed, without a leading "the ", in lower case.
fn key(name: &str) -> String {
    let name = name.trim();
    let name = name
        .get(..4)
        .filter(|the| the.eq_ignore_ascii_case("the "))
        .map_or(name, |_| &name[4..]);
    name.to_lowercase()
}

/// `text` as a warning quotes it: on one line, short, in double quotes, with none inside.
fn quoted(text: &str) -> String {
    format!(
        "\"{}\"",
        crate::space::one_line(text.trim(), 64).replace('"', "'")
    )
}

/// `names` as a sentence lists them: "a", "a and b", or "a, b and c".
fn listed(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Resolve each of `diagram`'s world references against `things`: its state, where it stands, its radius and frame,
/// its label where the agent gave none, and its kind where it was inferred. Returns what the agent is told each
/// resolved to, a line each, as `fire → the campsite's bonfire (farmyard:campsite/bonfire), 3.1 m from Samuel`, and a
/// warning for each that names nothing here or was inferred.
pub(crate) fn resolve(diagram: &mut Diagram, things: &Things) -> (Vec<String>, Vec<Warning>) {
    let mut told = Vec::new();
    let mut warnings = Vec::new();
    for target in &mut diagram.world {
        let reference = target.reference.clone();
        let found = match reference.kind {
            Kind::Unknown => INFERRED
                .iter()
                .find_map(|kind| {
                    things
                        .find(*kind, &reference.name)
                        .ok()
                        .map(|thing| (*kind, thing))
                })
                .map(|(kind, thing)| {
                    warnings.push(Warning {
                        line: None,
                        text: format!(
                            "{} names no kind, so it was taken as the {} {}: write it as {} := {} {}",
                            target.id,
                            kind.name(),
                            quoted(&reference.name),
                            target.id,
                            kind.name(),
                            reference.name
                        ),
                    });
                    target.reference = Reference::new(kind, &reference.name);
                    thing
                })
                .ok_or_else(|| {
                    format!(
                        "nothing in this space is called {}: write its kind, as {} := anchor NAME",
                        quoted(&reference.name),
                        target.id
                    )
                }),
            kind => things.find(kind, &reference.name),
        };
        let thing = match found {
            Ok(thing) => thing,
            Err(why) => {
                target.state = State::Unknown;
                target.at = None;
                warnings.push(Warning {
                    line: None,
                    text: format!("{}: {why}; nothing is drawn to it", target.id),
                });
                continue;
            }
        };
        if target.label.is_empty() {
            target.label =
                crate::space::one_line(&thing.label, tomori_protocol::world::LABEL_CHARS);
        }
        target.radius = thing.radius;
        target.frame = thing.frame;
        let name = thing
            .names
            .first()
            .map_or(String::new(), |name| format!(" ({name})"));
        match (thing.at, &thing.absent) {
            (Some(at), None) => {
                target.state = State::Here;
                target.at = Some(at);
                let from = things
                    .asker
                    .as_ref()
                    .map_or(String::new(), |(asker, eyes)| {
                        let apart = (0..3)
                            .map(|axis| (at[axis] - eyes[axis]).powi(2))
                            .sum::<f32>()
                            .sqrt();
                        format!(", {apart:.1} m from {asker}")
                    });
                told.push(format!("{} → {}{name}{from}", target.id, thing.described));
            }
            (_, why) => {
                target.state = State::Absent;
                target.at = None;
                let why = why.as_deref().unwrap_or("where it is not known");
                told.push(format!(
                    "{} → {}{name}, {why}, so nothing is drawn to it now",
                    target.id, thing.described
                ));
            }
        }
    }
    (told, warnings)
}

/// How firmly a node is pulled towards the side of its volume facing a target `metres` from the volume's middle:
/// firmly near, and barely for one far off, as `1 / (1 + (d / 2 m)²)`.
fn stiffness(metres: f64) -> f64 {
    1.0 / (1.0 + (metres / 2.0) * (metres / 2.0))
}

/// The most of the way to its port a node is pulled, by a target at the volume's middle.
const PULL: f64 = 0.6;

/// Pull each of `diagram`'s nodes that points at a target here towards its port, the point on the box about `spots`
/// where the line from their middle to the target leaves it, `pose` being where the volume stands: by [`stiffness`],
/// averaged over its targets, and, where `fixed` names an axis, as a layered layout's ranks, not along it. Each point
/// is in millimetres of the volume; the targets' are carried there from the space. A node that points at nothing, or a
/// diagram without a pose, is left where it is. Nodes this pushes together are parted again by the caller.
pub(super) fn pull(
    diagram: &Diagram,
    spots: &mut [Spot],
    widths: &[f64],
    pose: &Pose,
    fixed: Option<usize>,
) {
    if spots.is_empty() {
        return;
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for (spot, width) in spots.iter().zip(widths) {
        for axis in 0..3 {
            low[axis] = low[axis].min(spot[axis] - width / 2.0);
            high[axis] = high[axis].max(spot[axis] + width / 2.0);
        }
    }
    let middle = [0, 1, 2].map(|axis| (low[axis] + high[axis]) / 2.0);
    let half = [0, 1, 2].map(|axis| ((high[axis] - low[axis]) / 2.0).max(1.0));
    // Each target's place in the volume, in millimetres, as the volume stands.
    let inverse = [
        -pose.orientation[0],
        -pose.orientation[1],
        -pose.orientation[2],
        pose.orientation[3],
    ];
    let targets: Vec<Option<Spot>> = diagram
        .world
        .iter()
        .map(|target| {
            target.drawn_at().map(|at| {
                let offset = [0, 1, 2].map(|axis| at[axis] - pose.position[axis]);
                rotate(inverse, offset).map(|value| f64::from(value) * 1000.0)
            })
        })
        .collect();
    // Each node's ports, weighted, their weights and how many.
    let mut pulled = vec![([0.0; 3], 0.0, 0.0); spots.len()];
    for edge in &diagram.world_edges {
        let (node, target) = (usize::from(edge.from), usize::from(edge.to));
        let (Some(Some(at)), Some(_)) = (targets.get(target), spots.get(node)) else {
            continue;
        };
        let towards = [0, 1, 2].map(|axis| at[axis] - middle[axis]);
        let Some(direction) = normalised(towards) else {
            continue;
        };
        // Where the line from the middle leaves the box.
        let out = (0..3)
            .filter(|axis| direction[*axis].abs() > 1e-9)
            .map(|axis| half[axis] / direction[axis].abs())
            .fold(f64::INFINITY, f64::min);
        let port = [0, 1, 2].map(|axis| middle[axis] + direction[axis] * out);
        let weight = stiffness(length(towards) / 1000.0);
        let (sum, weights, count) = &mut pulled[node];
        for axis in 0..3 {
            sum[axis] += port[axis] * weight;
        }
        *weights += weight;
        *count += 1.0;
    }
    for (spot, (sum, weights, count)) in spots.iter_mut().zip(pulled) {
        if weights <= 0.0 {
            continue;
        }
        // Several targets average out, each as firmly as it pulls, and together at most [`PULL`] of the way.
        let port = sum.map(|value| value / weights);
        let share = PULL * weights / count;
        for axis in 0..3 {
            if Some(axis) != fixed {
                spot[axis] += (port[axis] - spot[axis]) * share;
            }
        }
    }
}

/// How many passes part nodes that a pull pushed together.
const PARTINGS: usize = 40;

/// Push apart every two nodes nearer than their half widths and a gap, each half of what they lack, in turn, as a force
/// layout's nodes are parted, but, where `fixed` names an axis, as a layered layout's ranks, never along it.
pub(super) fn part(graph: &super::Graph, spots: &mut [Spot], fixed: Option<usize>) {
    for _ in 0..PARTINGS {
        let mut moved = false;
        for a in 0..spots.len() {
            for b in a + 1..spots.len() {
                let least = (graph.widths[a] + graph.widths[b]) / 2.0 + super::GAP_MM;
                let mut apart = [0, 1, 2].map(|axis| spots[b][axis] - spots[a][axis]);
                if length(apart) >= least {
                    continue;
                }
                if let Some(fixed) = fixed {
                    apart[fixed] = 0.0;
                }
                // Two at one spot, or one above the other across ranks, part across: sideways, or, for ranks that run
                // sideways, up and down.
                let across = usize::from(fixed == Some(0));
                let direction = normalised(apart).unwrap_or_else(|| {
                    let mut direction = [0.0; 3];
                    direction[across] = 1.0;
                    direction
                });
                let distance = [0, 1, 2]
                    .map(|axis| (spots[b][axis] - spots[a][axis]) * direction[axis])
                    .iter()
                    .sum::<f64>();
                let push = (least - distance) / 2.0;
                for axis in 0..3 {
                    spots[a][axis] -= direction[axis] * push;
                    spots[b][axis] += direction[axis] * push;
                }
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// `vector` turned by the unit quaternion `rotation`.
fn rotate(rotation: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = rotation;
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = cross([x, y, z], vector).map(|value| value * 2.0);
    let turned = cross([x, y, z], t);
    [0, 1, 2].map(|axis| vector[axis] + t[axis] * w + turned[axis])
}

/// How near the middle of a volume's targets must be to whoever asked, in metres, for the volume to float between them,
/// and how far from their eyes it then floats; further off, how far from the targets' middle it stands, on their side.
const NEAR: f32 = 6.0;
const BETWEEN: f32 = 1.3;
const BY: f32 = 1.5;
/// How far it is turned off the line to the targets' middle, in degrees, so that it never covers them.
const TURNED: f32 = 25.0;
/// The widest the targets may be spread, as seen from whoever asked, in degrees, beyond which no one place sees them all.
const SPREAD: f32 = 120.0;

/// Where a volume pointing at `diagram`'s targets stands for whoever asked, whose head is at `head`, as `place: near
/// references` says: the middle of the targets that are here, each weighted by the world edges to it; within [`NEAR`]
/// of their eyes, floating [`BETWEEN`] from them, turned [`TURNED`] off the line to it, towards the side with fewer
/// targets, facing them; further, standing [`BY`] from it on their side at their eyes' height, facing back towards
/// them. `None` where none of its targets is here, or they are spread more than [`SPREAD`] about them, so that it goes
/// beside them instead.
pub(crate) fn near_references(diagram: &Diagram, head: &Pose) -> Option<Pose> {
    let eyes = head.position;
    let mut sum = [0.0f32; 3];
    let mut weights = 0.0f32;
    let mut bearings: Vec<f32> = Vec::new();
    for (index, target) in diagram.world.iter().enumerate() {
        let Some(at) = target.drawn_at() else {
            continue;
        };
        let weight = diagram
            .world_edges
            .iter()
            .filter(|edge| usize::from(edge.to) == index)
            .count() as f32;
        for axis in 0..3 {
            sum[axis] += at[axis] * weight;
        }
        weights += weight;
        bearings.push((at[0] - eyes[0]).atan2(-(at[2] - eyes[2])).to_degrees());
    }
    if weights <= 0.0 {
        return None;
    }
    let middle = sum.map(|value| value / weights);
    let towards = [middle[0] - eyes[0], 0.0, middle[2] - eyes[2]];
    let level = towards[0].hypot(towards[2]);
    if level < 1e-3 {
        return None;
    }
    let bearing = towards[0].atan2(-towards[2]).to_degrees();
    // Each target's bearing about the middle's, from -180 to 180.
    let about: Vec<f32> = bearings
        .iter()
        .map(|other| (other - bearing + 540.0).rem_euclid(360.0) - 180.0)
        .collect();
    let spread =
        about.iter().copied().fold(0.0, f32::max) - about.iter().copied().fold(0.0, f32::min);
    if spread > SPREAD {
        return None;
    }
    let ahead = [towards[0] / level, towards[2] / level];
    if level <= NEAR {
        // Turned towards the side with fewer targets: to the right, clockwise, where more are to the left.
        let right = about.iter().filter(|offset| **offset > 0.0).count();
        let left = about.iter().filter(|offset| **offset < 0.0).count();
        let turn = if right > left { -TURNED } else { TURNED }.to_radians();
        let (sin, cos) = turn.sin_cos();
        // Clockwise as seen from above, from -Z towards +X.
        let way = [
            ahead[0] * cos - ahead[1] * sin,
            ahead[0] * sin + ahead[1] * cos,
        ];
        let at = [
            eyes[0] + way[0] * BETWEEN,
            eyes[1],
            eyes[2] + way[1] * BETWEEN,
        ];
        return Some(Pose::upright_facing(at, eyes));
    }
    let at = [
        middle[0] - ahead[0] * BY,
        eyes[1],
        middle[2] - ahead[1] * BY,
    ];
    Some(Pose::upright_facing(at, eyes))
}

#[cfg(test)]
mod tests;
