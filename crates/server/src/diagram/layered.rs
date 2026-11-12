// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Layered layout, for pipelines, dependency graphs and anything with a direction, by Sugiyama's method as Graphviz's
//! `dot` lays graphs out, in 3D: cycles broken by reversing the edges a depth-first walk in the order written finds
//! going back; nodes ranked by their longest path, sources pulled down beside what they feed; long edges given
//! waypoints on the ranks they cross; crossings reduced by barycentre sweeps; and each rank a plane across the main
//! axis, a rank of more than five folded into two or three rows in depth rather than spread wide. Laid out again from a
//! previous layout, each rank keeps the order its nodes had, so that what was added settles in among them.
use super::{Graph, Spot, oriented};
use tomori_protocol::diagram::Rank;

/// How many nodes a rank holds across before it folds into rows in depth, and the most rows it folds into.
const ACROSS: usize = 5;
const ROWS: usize = 3;
/// How many sweeps, down and then up, reduce crossings.
const SWEEPS: usize = 4;

/// What stands on a rank: a node, or a waypoint of a long edge.
struct Item {
    rank: usize,
    node: Option<usize>,
    group: Option<usize>,
}

/// Each node's spot, oriented for `direction`, and each edge's waypoints, from its `from` to its `to`, laid out from
/// where `before` says each node stood, where it did.
pub(super) fn lay_out(graph: &Graph, direction: Rank, before: &[Option<Spot>]) -> (Vec<Spot>, Vec<Vec<Spot>>) {
    let count = graph.len();
    let flows = acyclic(graph);
    let ranks = ranked(count, &flows);
    // Each node is an item, then each long edge's waypoints, in order.
    let mut items: Vec<Item> = (0..count)
        .map(|node| Item {
            rank: ranks[node],
            node: Some(node),
            group: graph.groups[node],
        })
        .collect();
    let mut links: Vec<(usize, usize)> = Vec::new();
    let mut chains: Vec<Vec<usize>> = Vec::new();
    for &(from, to) in &flows {
        let mut chain = Vec::new();
        let mut last = from;
        for rank in ranks[from] + 1..ranks[to] {
            items.push(Item {
                rank,
                node: None,
                group: None,
            });
            let item = items.len() - 1;
            links.push((last, item));
            chain.push(item);
            last = item;
        }
        links.push((last, to));
        chains.push(chain);
    }
    let depth = ranks.iter().copied().max().map_or(0, |deepest| deepest + 1);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for (index, item) in items.iter().enumerate() {
        layers[item.rank].push(index);
    }
    let across = |spot: Spot| match direction {
        Rank::Right | Rank::Left => -spot[1],
        Rank::Up | Rank::Down | Rank::Away => spot[0],
    };
    // Laid out again, each rank begins in the order its nodes stood across, those new after them; and only what is new
    // moves in the sweeps, as its neighbours draw it.
    let key = |item: usize| items[item].node.and_then(|node| before[node]).map(across);
    let stable = before.iter().any(Option::is_some);
    for layer in &mut layers {
        layer.sort_by(|a, b| match (key(*a), key(*b)) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        });
    }
    let mut neighbours = vec![Vec::new(); items.len()];
    for &(a, b) in &links {
        neighbours[a].push(b);
        neighbours[b].push(a);
    }
    for sweep in 0..2 * SWEEPS {
        let order: Vec<usize> = if sweep % 2 == 0 {
            (1..depth).collect()
        } else {
            (0..depth.saturating_sub(1)).rev().collect()
        };
        for rank in order {
            let beside = if sweep % 2 == 0 { rank - 1 } else { rank + 1 };
            let at = |item: usize| layers[beside].iter().position(|other| *other == item);
            let mut keyed: Vec<(f64, usize, usize)> = layers[rank]
                .iter()
                .enumerate()
                .map(|(index, &item)| {
                    let found: Vec<f64> = neighbours[item]
                        .iter()
                        .filter_map(|other| at(*other))
                        .map(|position| position as f64)
                        .collect();
                    let moves = !(stable && key(item).is_some());
                    let barycentre = if found.is_empty() || !moves {
                        index as f64
                    } else {
                        found.iter().sum::<f64>() / found.len() as f64
                    };
                    (barycentre, index, item)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            layers[rank] = keyed.into_iter().map(|(.., item)| item).collect();
        }
    }
    // A group's members on a rank stand together, where the first of them stands.
    for layer in &mut layers {
        let first = |group: Option<usize>, index: usize, layer: &[usize]| {
            group
                .and_then(|group| layer.iter().position(|item| items[*item].group == Some(group)))
                .unwrap_or(index)
        };
        let mut keyed: Vec<(usize, usize, usize)> = layer
            .iter()
            .enumerate()
            .map(|(index, &item)| (first(items[item].group, index, layer), index, item))
            .collect();
        keyed.sort();
        *layer = keyed.into_iter().map(|(.., item)| item).collect();
    }
    // Each rank a plane, folded into rows in depth where it is wide, its items across in order, alternate rows between.
    let pitch = graph.pitch();
    let mut spots = vec![[0.0; 3]; items.len()];
    for (rank, layer) in layers.iter().enumerate() {
        let wide = layer.len();
        let rows = wide.div_ceil(ACROSS).clamp(1, ROWS);
        for (index, &item) in layer.iter().enumerate() {
            let across = (index as f64 - (wide as f64 - 1.0) / 2.0) * pitch / rows as f64;
            let row = (index % rows) as f64 - (rows as f64 - 1.0) / 2.0;
            spots[item] = [rank as f64 * pitch * 1.4, across, row * pitch * 1.2];
        }
    }
    let spots: Vec<Spot> = spots.into_iter().map(|spot| oriented(direction, spot)).collect();
    let via = graph
        .written
        .iter()
        .zip(&flows)
        .zip(&chains)
        .map(|((written, flow), chain)| {
            let mut via: Vec<Spot> = chain.iter().map(|item| spots[*item]).collect();
            if written.0 != flow.0 {
                via.reverse();
            }
            via
        })
        .collect();
    (spots[..count].to_vec(), via)
}

/// The graph's edges the way they flow, with those a depth-first walk in the order written finds going back to a node
/// it is still within reversed, so that what is left has no cycle.
fn acyclic(graph: &Graph) -> Vec<(usize, usize)> {
    let count = graph.len();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (index, &(from, _)) in graph.edges.iter().enumerate() {
        out[from].push(index);
    }
    // 0 not yet reached, 1 within the walk, 2 left.
    let mut state = vec![0u8; count];
    let mut reversed = vec![false; graph.edges.len()];
    for start in 0..count {
        if state[start] != 0 {
            continue;
        }
        let mut stack = vec![(start, 0)];
        state[start] = 1;
        while let Some((node, next)) = stack.pop() {
            let Some(&edge) = out[node].get(next) else {
                state[node] = 2;
                continue;
            };
            stack.push((node, next + 1));
            let to = graph.edges[edge].1;
            match state[to] {
                0 => {
                    state[to] = 1;
                    stack.push((to, 0));
                }
                1 => reversed[edge] = true,
                _ => {}
            }
        }
    }
    graph
        .edges
        .iter()
        .zip(reversed)
        .map(|(&(from, to), reversed)| if reversed { (to, from) } else { (from, to) })
        .collect()
}

/// Each node's rank, by its longest path from a node nothing flows into, with each such node, that feeds others, then
/// pulled down to the rank just above the nearest it feeds.
fn ranked(count: usize, flows: &[(usize, usize)]) -> Vec<usize> {
    let mut rank = vec![0; count];
    let mut into = vec![0; count];
    for &(_, to) in flows {
        into[to] += 1;
    }
    let sources: Vec<bool> = into.iter().map(|count| *count == 0).collect();
    let mut ready: Vec<usize> = (0..count).filter(|node| into[*node] == 0).collect();
    let mut at = 0;
    while at < ready.len() {
        let node = ready[at];
        at += 1;
        for &(from, to) in flows {
            if from == node {
                rank[to] = rank[to].max(rank[node] + 1);
                into[to] -= 1;
                if into[to] == 0 {
                    ready.push(to);
                }
            }
        }
    }
    for node in 0..count {
        let fed = flows
            .iter()
            .filter(|(from, _)| *from == node)
            .map(|(_, to)| rank[*to])
            .min();
        if sources[node]
            && let Some(fed) = fed
        {
            rank[node] = fed - 1;
        }
    }
    rank
}
