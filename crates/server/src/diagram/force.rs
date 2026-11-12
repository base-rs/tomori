// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Force-directed layout, for graphs with no main direction, by stress majorisation in 3D: nodes placed so that their
//! distances in space follow their distances in the graph, each step solving for each node in turn from the others,
//! which converges without a random start, then separated so that no two overlap. A node in a group is drawn nearer
//! the rest of its group, and kept further from other groups. Laid out afresh, it starts from a hash of each node's id
//! and is turned so that it is widest across, then tallest, and shallowest in depth; laid out again, every node that
//! stood before starts there and is held there by a spring as strong as the rest of its forces together, and a node
//! added starts among its neighbours, so that it moves only as far as the change needs.
use super::{GAP_MM, Graph, Spot, cross, hashed, length, normalised};

/// How many steps the stress is majorised for, and how many passes separate nodes that overlap.
const STEPS: usize = 200;
const SEPARATIONS: usize = 40;

/// Each node's spot, from where `before` says each stood, where it did.
pub(super) fn lay_out(graph: &Graph, before: &[Option<Spot>]) -> Vec<Spot> {
    let count = graph.len();
    let unit = graph.pitch() * 1.5;
    let distances = distances(graph);
    let fresh = before.iter().all(Option::is_none);
    // Each node's start: where it stood, or among its neighbours that did, or else hashed from its id.
    let neighbours = graph.neighbours();
    let spread = unit * (count as f64).cbrt();
    let mut spots: Vec<Spot> = (0..count)
        .map(|node| {
            before[node].unwrap_or_else(|| {
                let near: Vec<Spot> = neighbours[node]
                    .iter()
                    .filter_map(|other| before[*other])
                    .collect();
                let jitter = hashed(&graph.ids[node]);
                if near.is_empty() {
                    jitter.map(|value| value * spread)
                } else {
                    [0, 1, 2].map(|axis| {
                        near.iter().map(|spot| spot[axis]).sum::<f64>() / near.len() as f64
                            + jitter[axis] * unit / 2.0
                    })
                }
            })
        })
        .collect();
    for _ in 0..STEPS {
        for node in 0..count {
            let mut sum = [0.0; 3];
            let mut weights = 0.0;
            for other in 0..count {
                if other == node {
                    continue;
                }
                let ideal = distances[node][other] * unit;
                let weight = 1.0 / (ideal * ideal);
                let apart = [0, 1, 2].map(|axis| spots[node][axis] - spots[other][axis]);
                // Two at one spot part along a direction of their own, hashed from their ids.
                let direction = normalised(apart).unwrap_or_else(|| {
                    let a = hashed(&graph.ids[node]);
                    let b = hashed(&graph.ids[other]);
                    normalised([a[0] - b[0], a[1] - b[1], a[2] - b[2] + 1e-3])
                        .unwrap_or([1.0, 0.0, 0.0])
                });
                for axis in 0..3 {
                    sum[axis] += weight * (spots[other][axis] + ideal * direction[axis]);
                }
                weights += weight;
            }
            if let Some(stood) = before[node] {
                for axis in 0..3 {
                    sum[axis] += weights * stood[axis];
                }
                weights *= 2.0;
            }
            if weights > 0.0 {
                spots[node] = sum.map(|value| value / weights);
            }
        }
    }
    separate(graph, &mut spots);
    if fresh {
        turned(&mut spots);
    }
    spots
}

/// How far apart each pair of nodes is in the graph, in edges, either way: one more than the furthest for nodes not
/// joined, at most two within a group and at least two between groups.
fn distances(graph: &Graph) -> Vec<Vec<f64>> {
    let count = graph.len();
    let neighbours = graph.neighbours();
    let mut hops = vec![vec![usize::MAX; count]; count];
    for (start, from) in hops.iter_mut().enumerate() {
        from[start] = 0;
        let mut queue = vec![start];
        let mut at = 0;
        while at < queue.len() {
            let node = queue[at];
            at += 1;
            for &next in &neighbours[node] {
                if from[next] == usize::MAX {
                    from[next] = from[node] + 1;
                    queue.push(next);
                }
            }
        }
    }
    let furthest = hops
        .iter()
        .flatten()
        .filter(|hops| **hops != usize::MAX)
        .max()
        .copied()
        .unwrap_or(0);
    (0..count)
        .map(|a| {
            (0..count)
                .map(|b| {
                    let mut distance = if hops[a][b] == usize::MAX {
                        furthest + 1
                    } else {
                        hops[a][b]
                    } as f64;
                    match (graph.groups[a], graph.groups[b]) {
                        (Some(x), Some(y)) if x == y => distance = distance.min(2.0),
                        (Some(_), _) | (_, Some(_)) => distance = distance.max(2.0),
                        _ => {}
                    }
                    distance
                })
                .collect()
        })
        .collect()
}

/// Push apart every two nodes nearer than their half widths and a gap, each half of what they lack, in turn.
pub(super) fn separate(graph: &Graph, spots: &mut [Spot]) {
    for _ in 0..SEPARATIONS {
        let mut moved = false;
        for a in 0..spots.len() {
            for b in a + 1..spots.len() {
                let least = (graph.widths[a] + graph.widths[b]) / 2.0 + GAP_MM;
                let apart = [0, 1, 2].map(|axis| spots[b][axis] - spots[a][axis]);
                let distance = length(apart);
                if distance >= least {
                    continue;
                }
                let direction = normalised(apart).unwrap_or([1.0, 0.0, 0.0]);
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

/// Turn `spots` about their middle so that they are widest across, then tallest, and shallowest in depth, which a
/// volume read from in front shows best: their principal axes, by power iteration on their covariance, signed so that
/// the first node stands left of and above the middle.
pub(super) fn turned(spots: &mut [Spot]) {
    let count = spots.len() as f64;
    let middle = [0, 1, 2].map(|axis| spots.iter().map(|spot| spot[axis]).sum::<f64>() / count);
    let centred: Vec<Spot> = spots
        .iter()
        .map(|spot| [0, 1, 2].map(|axis| spot[axis] - middle[axis]))
        .collect();
    let mut covariance = [[0.0; 3]; 3];
    for spot in &centred {
        for row in 0..3 {
            for column in 0..3 {
                covariance[row][column] += spot[row] * spot[column];
            }
        }
    }
    let principal = |matrix: &[[f64; 3]; 3], start: Spot| {
        let mut vector = start;
        for _ in 0..64 {
            let next = [0, 1, 2].map(|row| {
                (0..3)
                    .map(|column| matrix[row][column] * vector[column])
                    .sum::<f64>()
            });
            match normalised(next) {
                Some(next) => vector = next,
                None => break,
            }
        }
        vector
    };
    let first = principal(&covariance, [1.0, 0.3, 0.1]);
    // Without the first axis's share, the next.
    let along: f64 = (0..3)
        .map(|row| {
            (0..3)
                .map(|column| first[row] * covariance[row][column] * first[column])
                .sum::<f64>()
        })
        .sum();
    let mut deflated = covariance;
    for row in 0..3 {
        for column in 0..3 {
            deflated[row][column] -= along * first[row] * first[column];
        }
    }
    let second = principal(&deflated, [0.1, 1.0, 0.3]);
    // Square to the first, in case the deflation left a little of it.
    let dot: f64 = (0..3).map(|axis| second[axis] * first[axis]).sum();
    let second = normalised([0, 1, 2].map(|axis| second[axis] - dot * first[axis]))
        .unwrap_or([0.0, 1.0, 0.0]);
    let third = cross(first, second);
    let mut axes = [first, second, third];
    let project =
        |spot: &Spot, axis: &Spot| (0..3).map(|index| spot[index] * axis[index]).sum::<f64>();
    // Signed by the first node, of which there is always one where a layout is turned.
    let start = centred.first().copied().unwrap_or_default();
    if project(&start, &axes[0]) > 0.0 {
        axes[0] = axes[0].map(|value| -value);
        axes[2] = axes[2].map(|value| -value);
    }
    if project(&start, &axes[1]) < 0.0 {
        axes[1] = axes[1].map(|value| -value);
        axes[2] = axes[2].map(|value| -value);
    }
    for (spot, centred) in spots.iter_mut().zip(&centred) {
        *spot = axes.map(|axis| project(centred, &axis));
    }
}
