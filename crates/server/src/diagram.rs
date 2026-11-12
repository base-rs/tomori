// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Laying volumes' diagrams out, on the server, once for each change, so that every participant sees the same
//! positions by construction and a client lays nothing out. Each layout runs in `f64` for a fixed number of steps over
//! the nodes in the order written, with no random number and no hash map's order: where a start is needed, a node's
//! comes from a hash of its id, so that the same diagram at the same previous layout is always laid out alike. A diagram
//! laid out again, as a block with its id changes it, starts from where it stood, so that a small edit moves little.
//! See `design/diagrams.md`.
use tomori_protocol::diagram::{Arrow, Diagram, Layout, NODE_MM, Point, Rank, WAYPOINTS, Warning};

mod force;
mod layered;

/// The gap kept between nodes, in millimetres of the volume.
const GAP_MM: f64 = 60.0;
/// The margin about everything laid out, within the volume's extent.
const MARGIN_MM: f64 = 40.0;
/// How far a group's box stands out from its members.
const GROUP_MM: f64 = 30.0;
/// How far apart, sideways, the edges between one pair of nodes are curved.
const PARALLEL_MM: f64 = 40.0;

/// A point while laying out, in millimetres.
type Spot = [f64; 3];

/// Lay `diagram` out, from where `previous`, the diagram it changes, stood, if it changes one: each node's position,
/// each edge's waypoints, each group's box and the volume's extent. Returns what it laid out otherwise than asked, such
/// as a layout that comes later, as warnings.
pub(crate) fn lay_out(diagram: &mut Diagram, previous: Option<&Diagram>) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let before: Vec<Option<Spot>> = diagram
        .nodes
        .iter()
        .map(|node| {
            previous
                .and_then(|previous| previous.nodes.iter().find(|old| old.id == node.id))
                .map(|old| old.at.map(f64::from))
        })
        .collect();
    let graph = Graph::of(diagram);
    let (spots, via) = match &diagram.layout {
        Layout::Layered { direction } => layered::lay_out(&graph, *direction, &before),
        Layout::Grid => (grid(&graph), Vec::new()),
        Layout::Force => (force::lay_out(&graph, &before), Vec::new()),
        _ => {
            warnings.push(Warning {
                line: None,
                text: "radial layout comes in a later version: laid out by force".into(),
            });
            (force::lay_out(&graph, &before), Vec::new())
        }
    };
    place(diagram, &graph, &spots, via);
    warnings
}

/// A diagram's nodes and edges as its layouts read them.
struct Graph {
    /// Each node's width, in millimetres.
    widths: Vec<f64>,
    /// Each node's group, by its index.
    groups: Vec<Option<usize>>,
    /// Each edge's ends, the way it flows: an edge whose arrow points back flows from its `to`.
    edges: Vec<(usize, usize)>,
    /// Each edge's ends as written.
    written: Vec<(usize, usize)>,
    /// Each node's id, from which its start is hashed.
    ids: Vec<String>,
}

impl Graph {
    fn of(diagram: &Diagram) -> Self {
        Self {
            widths: diagram
                .nodes
                .iter()
                .map(|node| f64::from(node.width()))
                .collect(),
            groups: diagram
                .nodes
                .iter()
                .map(|node| node.group.map(usize::from))
                .collect(),
            edges: diagram
                .edges
                .iter()
                .map(|edge| {
                    let (from, to) = (usize::from(edge.from), usize::from(edge.to));
                    if edge.direction == Arrow::Back {
                        (to, from)
                    } else {
                        (from, to)
                    }
                })
                .collect(),
            written: diagram
                .edges
                .iter()
                .map(|edge| (usize::from(edge.from), usize::from(edge.to)))
                .collect(),
            ids: diagram.nodes.iter().map(|node| node.id.clone()).collect(),
        }
    }

    fn len(&self) -> usize {
        self.widths.len()
    }

    /// How far apart neighbouring nodes stand at least, middle to middle: the widest node and a gap.
    fn pitch(&self) -> f64 {
        self.widths.iter().copied().fold(NODE_MM.into(), f64::max) + GAP_MM
    }

    /// Each node's neighbours, either way, in the order the edges were written.
    fn neighbours(&self) -> Vec<Vec<usize>> {
        let mut neighbours = vec![Vec::new(); self.len()];
        for &(from, to) in &self.edges {
            neighbours[from].push(to);
            neighbours[to].push(from);
        }
        neighbours
    }
}

/// Three numbers from -1 to 1 hashed from `id`, by FNV-1a, the same on every machine and in every run.
fn hashed(id: &str) -> Spot {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    [0, 21, 42].map(|shift| ((hash >> shift) & 0x1f_ffff) as f64 / f64::from(0x10_0000) - 1.0)
}

/// Rows and columns in the order written, as near square as they come, each a pitch apart.
fn grid(graph: &Graph) -> Vec<Spot> {
    let count = graph.len();
    let columns = (1..)
        .find(|columns| columns * columns >= count)
        .unwrap_or(1);
    let pitch = graph.pitch();
    (0..count)
        .map(|index| {
            [
                (index % columns) as f64 * pitch,
                -((index / columns) as f64) * pitch,
                0.0,
            ]
        })
        .collect()
}

/// Write `spots` into `diagram`'s nodes and `via` into its edges, centred in the volume, with each group's box about its
/// members and the volume's extent about everything, and curve apart the edges between one pair of nodes that have no
/// waypoints.
fn place(diagram: &mut Diagram, graph: &Graph, spots: &[Spot], mut via: Vec<Vec<Spot>>) {
    via.resize(diagram.edges.len(), Vec::new());
    // Parallel edges, either way between one pair, the second and later curved out to alternate sides.
    for (index, edge) in diagram.edges.iter().enumerate() {
        let (from, to) = (usize::from(edge.from), usize::from(edge.to));
        let earlier = diagram.edges[..index]
            .iter()
            .filter(|edge| {
                let ends = (usize::from(edge.from), usize::from(edge.to));
                ends == (from, to) || ends == (to, from)
            })
            .count();
        if earlier > 0 && via[index].is_empty() {
            // Sideways from the lower index's end, so that edges either way between the pair curve to opposite sides.
            let (a, b) = (spots[from.min(to)], spots[from.max(to)]);
            let middle = [0, 1, 2].map(|axis| (a[axis] + b[axis]) / 2.0);
            let along = [0, 1, 2].map(|axis| b[axis] - a[axis]);
            // Sideways across the edge, in the plane facing the viewer where it can be.
            let side = normalised(cross(along, [0.0, 0.0, 1.0])).unwrap_or([0.0, 1.0, 0.0]);
            let offset = PARALLEL_MM
                * (earlier.div_ceil(2)) as f64
                * if earlier % 2 == 1 { 1.0 } else { -1.0 };
            via[index] = vec![[0, 1, 2].map(|axis| middle[axis] + side[axis] * offset)];
        }
    }
    // Groups' boxes, from the innermost out, each about its members and the groups within it.
    let mut boxes: Vec<Option<(Spot, Spot)>> = vec![None; diagram.groups.len()];
    for group in (0..diagram.groups.len()).rev() {
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        let mut grow = |at: Spot, half: f64| {
            for axis in 0..3 {
                low[axis] = low[axis].min(at[axis] - half);
                high[axis] = high[axis].max(at[axis] + half);
            }
        };
        for (node, spot) in spots.iter().enumerate() {
            if graph.groups[node] == Some(group) {
                grow(*spot, graph.widths[node] / 2.0 + GROUP_MM);
            }
        }
        for (child, found) in boxes.iter().enumerate() {
            if let Some((child_low, child_high)) = found
                && diagram.groups[child].parent.map(usize::from) == Some(group)
            {
                grow(*child_low, GROUP_MM);
                grow(*child_high, GROUP_MM);
            }
        }
        boxes[group] = low[0].is_finite().then_some((low, high));
    }
    // The extent about every node, waypoint and group, with a margin, its middle the volume's.
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    let mut grow = |at: Spot, half: f64| {
        for axis in 0..3 {
            low[axis] = low[axis].min(at[axis] - half);
            high[axis] = high[axis].max(at[axis] + half);
        }
    };
    for (node, spot) in spots.iter().enumerate() {
        grow(*spot, graph.widths[node] / 2.0);
    }
    for spot in via.iter().flatten() {
        grow(*spot, 0.0);
    }
    for (box_low, box_high) in boxes.iter().flatten() {
        grow(*box_low, 0.0);
        grow(*box_high, 0.0);
    }
    if !low[0].is_finite() {
        diagram.extent = [0; 3];
        return;
    }
    let middle = [0, 1, 2].map(|axis| (low[axis] + high[axis]) / 2.0);
    let point =
        |spot: Spot| -> Point { [0, 1, 2].map(|axis| millimetres(spot[axis] - middle[axis])) };
    for (node, spot) in diagram.nodes.iter_mut().zip(spots) {
        node.at = point(*spot);
    }
    for (edge, via) in diagram.edges.iter_mut().zip(via) {
        edge.via = thinned(via).into_iter().map(point).collect();
    }
    for (group, found) in diagram.groups.iter_mut().zip(&boxes) {
        let (group_low, group_high) = found.unwrap_or((middle, middle));
        group.low = point(group_low);
        group.high = point(group_high);
    }
    diagram.extent = [0, 1, 2].map(|axis| {
        (high[axis] - low[axis] + 2.0 * MARGIN_MM)
            .round()
            .clamp(0.0, f64::from(tomori_protocol::diagram::EXTENT_MM)) as u16
    });
}

/// At most [`WAYPOINTS`] of `via`, evenly spread along it, its first and last among them.
fn thinned(via: Vec<Spot>) -> Vec<Spot> {
    if via.len() <= WAYPOINTS {
        return via;
    }
    (0..WAYPOINTS)
        .map(|index| via[index * (via.len() - 1) / (WAYPOINTS - 1)])
        .collect()
}

/// `value` in whole millimetres, within what a point holds.
fn millimetres(value: f64) -> i16 {
    value
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

fn cross(a: Spot, b: Spot) -> Spot {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Spot) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// `a` made a unit long, unless it is too short to have a direction.
fn normalised(a: Spot) -> Option<Spot> {
    let length = length(a);
    (length > 1e-9).then(|| a.map(|value| value / length))
}

/// Where, along, across and in depth, a layered layout's spot stands in the volume, for ranks going `direction`.
fn oriented(direction: Rank, [along, across, depth]: Spot) -> Spot {
    match direction {
        Rank::Up => [across, along, -depth],
        Rank::Right => [along, -across, -depth],
        Rank::Left => [-along, -across, -depth],
        Rank::Away => [across, -depth, -along],
        Rank::Down => [across, -along, -depth],
    }
}

#[cfg(test)]
mod tests;
