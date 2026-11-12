// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::diagram::{Diagram, parse};

/// `body` read and laid out, from `previous` where it changes one.
fn laid(body: &str, previous: Option<&Diagram>) -> (Diagram, Vec<String>) {
    let (mut diagram, warnings) = parse(body);
    assert!(warnings.is_empty(), "{warnings:?}");
    let warnings = lay_out(&mut diagram, previous);
    (diagram, warnings.iter().map(ToString::to_string).collect())
}

fn at(diagram: &Diagram, id: &str) -> [f64; 3] {
    diagram.nodes[diagram.node(id).unwrap()].at.map(f64::from)
}

/// Where each of `ids` stands about their middle, so that a change that moves them all together moves none.
fn relative(diagram: &Diagram, ids: &[&str]) -> Vec<[f64; 3]> {
    let spots: Vec<[f64; 3]> = ids.iter().map(|id| at(diagram, id)).collect();
    let middle =
        [0, 1, 2].map(|axis| spots.iter().map(|spot| spot[axis]).sum::<f64>() / spots.len() as f64);
    spots
        .iter()
        .map(|spot| [0, 1, 2].map(|axis| spot[axis] - middle[axis]))
        .collect()
}

fn apart(a: [f64; 3], b: [f64; 3]) -> f64 {
    length([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

const PIPELINE: &str = "flowchart TD\nheadset --> listener\nlistener --> hub\nhub --> stage\nhub --> backend\nbackend --> opencode\nstage --> listener\n";

#[test]
fn a_layered_layout_is_golden_ranked_down_with_its_cycle_broken() {
    let (diagram, warnings) = laid("a --> b\na --> c\nb --> d\nc --> d\nd --> a\n", None);
    assert!(warnings.is_empty());
    let spots: Vec<[i16; 3]> = diagram.nodes.iter().map(|node| node.at).collect();
    // Ranked a, then b and c beside each other, then d, top to bottom; the edge back up to a reversed for ranking, its
    // waypoint beside them, so that it passes them by.
    assert_eq!(
        spots,
        [[15, 168, 0], [-105, 0, 0], [15, 0, 0], [15, -168, 0]]
    );
    assert_eq!(diagram.edges[4].via, [[135, 0, 0]]);
    assert_eq!(diagram.extent, [350, 476, 140]);
    // Every node's middle within the extent, about the volume's middle.
    for node in &diagram.nodes {
        for axis in 0..3 {
            assert!(f64::from(node.at[axis]).abs() <= f64::from(diagram.extent[axis]) / 2.0);
        }
    }
}

#[test]
fn the_same_diagram_is_laid_out_alike_every_time() {
    for body in [
        PIPELINE.to_owned(),
        PIPELINE.replace("flowchart TD", "flowchart LR"),
        format!("%% force\n{}", PIPELINE.replace("flowchart TD\n", "")),
    ] {
        let mut diagram = parse(&body).0;
        if body.starts_with("%% force") {
            diagram.layout = tomori_protocol::diagram::Layout::Force;
        }
        let mut again = diagram.clone();
        lay_out(&mut diagram, None);
        lay_out(&mut again, None);
        assert_eq!(diagram, again);
        assert!(diagram.extent.iter().all(|side| *side > 0));
    }
}

#[test]
fn ranks_go_the_way_the_layout_says() {
    let line = |direction: &str| {
        let (diagram, _) = laid(&format!("flowchart {direction}\na --> b\n"), None);
        let (a, b) = (at(&diagram, "a"), at(&diagram, "b"));
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]].map(|value: f64| {
            if value.abs() < 0.5 {
                0.0
            } else {
                value.signum()
            }
        })
    };
    assert_eq!(line("TD"), [0.0, -1.0, 0.0]);
    assert_eq!(line("BT"), [0.0, 1.0, 0.0]);
    assert_eq!(line("LR"), [1.0, 0.0, 0.0]);
    assert_eq!(line("RL"), [-1.0, 0.0, 0.0]);
    let (mut diagram, _) = parse("a --> b\n");
    diagram.layout = tomori_protocol::diagram::Layout::Layered {
        direction: Rank::Away,
    };
    lay_out(&mut diagram, None);
    assert!(
        at(&diagram, "b")[2] < at(&diagram, "a")[2],
        "away is into the volume"
    );
    // An edge whose arrow points back flows from its to.
    let (diagram, _) = laid("a <-- b\n", None);
    assert!(at(&diagram, "b")[1] > at(&diagram, "a")[1]);
}

#[test]
fn a_wide_rank_folds_into_rows_in_depth() {
    let body: String = (0..12)
        .map(|index| format!("root --> leaf{index}\n"))
        .collect();
    let (diagram, _) = laid(&body, None);
    let depths: Vec<i16> = (0..12)
        .map(|index| diagram.nodes[diagram.node(&format!("leaf{index}")).unwrap()].at[2])
        .collect();
    let mut rows = depths.clone();
    rows.sort();
    rows.dedup();
    assert_eq!(rows.len(), 3, "{depths:?}");
    // Folded, the rank is a third as wide as it would be in one row.
    let width = f64::from(diagram.extent[0]);
    assert!(width < 12.0 * 120.0 / 2.0, "{width}");
}

#[test]
fn long_edges_pass_through_the_ranks_they_cross() {
    let (diagram, _) = laid(
        "a --> b\nb --> c\nc --> d\nd --> e\ne --> f\nf --> g\na --> g\ng --> a\n",
        None,
    );
    let long = &diagram.edges[6];
    assert_eq!(long.via.len(), WAYPOINTS, "thinned to the most an edge has");
    // From its from towards its to, ranked down.
    assert!(long.via[0][1] > long.via[3][1]);
    // The edge back up to a, reversed for ranking, has its waypoints from g up to a.
    let back = &diagram.edges[7];
    assert!(back.via.is_empty() || back.via[0][1] < back.via[back.via.len() - 1][1]);
}

#[test]
fn edges_between_one_pair_are_curved_apart() {
    let (diagram, _) = laid("a --> b\na --> b : again\nb --> a : back\n", None);
    assert!(diagram.edges[0].via.is_empty());
    let (one, two) = (diagram.edges[1].via[0], diagram.edges[2].via[0]);
    assert_ne!(one, two);
    // Curved out to either side.
    assert!(one[0].signum() != two[0].signum(), "{one:?} {two:?}");
    let (diagram, _) = laid("flowchart LR\na --> b\na --> b\n", None);
    assert_ne!(diagram.edges[1].via[0][1], 0);
}

#[test]
fn a_small_edit_to_a_layered_diagram_keeps_each_rank_in_order() {
    let (before, _) = laid(PIPELINE, None);
    let (after, _) = laid(
        &format!("{PIPELINE}hub --> speech\nspeech --> listener\n"),
        Some(&before),
    );
    let ids = ["headset", "listener", "hub", "stage", "backend", "opencode"];
    let was = relative(&before, &ids);
    let now = relative(&after, &ids);
    let pitch = 180.0;
    for (index, id) in ids.iter().enumerate() {
        let (was, now) = (was[index], now[index]);
        assert!(
            apart(was, now) < pitch,
            "{id} moved from {was:?} to {now:?}"
        );
    }
    // stage stood left of backend, and still does.
    assert_eq!(
        (at(&before, "stage")[0] < at(&before, "backend")[0]),
        (at(&after, "stage")[0] < at(&after, "backend")[0])
    );
}

#[test]
fn a_small_edit_to_a_force_diagram_moves_little() {
    let body = "a --- b\nb --- c\nc --- d\nd --- a\na --- e\ne --- f\nf --- c\n";
    let force = |body: &str, previous: Option<&Diagram>| {
        let mut diagram = parse(body).0;
        diagram.layout = tomori_protocol::diagram::Layout::Force;
        lay_out(&mut diagram, previous);
        diagram
    };
    let before = force(body, None);
    let after = force(&format!("{body}g --- a\n"), Some(&before));
    let ids = ["a", "b", "c", "d", "e", "f"];
    let was = relative(&before, &ids);
    let now = relative(&after, &ids);
    for (index, id) in ids.iter().enumerate() {
        let (was, now) = (was[index], now[index]);
        assert!(apart(was, now) < 90.0, "{id} moved from {was:?} to {now:?}");
    }
    // Laid out afresh from the same, it would be the same as before.
    assert_eq!(force(body, None), before);
    // No two nodes overlap.
    for (index, a) in after.nodes.iter().enumerate() {
        for b in &after.nodes[index + 1..] {
            let (a, b) = (&a.id, &b.id);
            assert!(apart(at(&after, a), at(&after, b)) >= 100.0, "{a} and {b}");
        }
    }
    // It is widest across, then tallest, and shallowest in depth.
    let spread = |axis: usize| {
        before
            .nodes
            .iter()
            .map(|node| f64::from(node.at[axis]).powi(2))
            .sum::<f64>()
    };
    let spreads = [spread(0), spread(1), spread(2)];
    assert!(
        spreads[0] >= spreads[1] && spreads[1] >= spreads[2],
        "{spreads:?}"
    );
}

#[test]
fn groups_stand_together_and_their_boxes_hold_their_members() {
    let (diagram, _) = laid(
        "subgraph scene [Scene]\nstep --> faces\nsubgraph inner\nshapes\nend\nend\nxr --> step\nfaces --> shapes\nxr --> other\nother --> faces\n",
        None,
    );
    for (index, group) in diagram.groups.iter().enumerate() {
        for node in diagram
            .nodes
            .iter()
            .filter(|node| node.group.map(usize::from) == Some(index))
        {
            for axis in 0..3 {
                let (node, group, at) = (
                    &node.id,
                    &group.id,
                    (group.low[axis], node.at[axis], group.high[axis]),
                );
                assert!(at.0 < at.1 && at.1 < at.2, "{node} in {group}");
            }
        }
    }
    // The outer group holds the inner one.
    let (outer, inner) = (&diagram.groups[0], &diagram.groups[1]);
    assert!(
        (0..3)
            .all(|axis| outer.low[axis] <= inner.low[axis] && inner.high[axis] <= outer.high[axis])
    );
    // A group of no nodes has an empty box at the middle; so does a diagram of nothing.
    let (diagram, _) = laid("subgraph empty\nend\na\n", None);
    assert_eq!(
        (diagram.groups[0].low, diagram.groups[0].high),
        ([0; 3], [0; 3])
    );
    let mut empty = Diagram::default();
    lay_out(&mut empty, None);
    assert_eq!(empty.extent, [0; 3]);
}

#[test]
fn force_keeps_a_group_together_and_apart_from_others() {
    let mut diagram = parse(
        "subgraph left\na --- b\nb --- c\nend\nsubgraph right\nd --- e\ne --- f\nend\nc --- d\nloose --- a\n",
    )
    .0;
    diagram.layout = tomori_protocol::diagram::Layout::Force;
    lay_out(&mut diagram, None);
    let within = apart(at(&diagram, "a"), at(&diagram, "c"));
    let between = apart(at(&diagram, "a"), at(&diagram, "f"));
    assert!(within < between, "{within} {between}");
}

#[test]
fn radial_comes_later_and_grid_lays_out_in_order() {
    let (mut diagram, _) = parse("a --> b\nb --> c\n");
    diagram.layout = tomori_protocol::diagram::Layout::Radial {
        hub: Some("a".into()),
    };
    let warnings = lay_out(&mut diagram, None);
    assert_eq!(
        warnings[0].text,
        "radial layout comes in a later version: laid out by force"
    );
    diagram.layout = tomori_protocol::diagram::Layout::Unknown;
    assert_eq!(lay_out(&mut diagram, None).len(), 1);
    let (mut diagram, _) = parse("a\nb\nc\nd\ne\n");
    diagram.layout = tomori_protocol::diagram::Layout::Grid;
    assert!(lay_out(&mut diagram, None).is_empty());
    let spots: Vec<[i16; 3]> = diagram.nodes.iter().map(|node| node.at).collect();
    // Three columns, of two rows.
    assert_eq!(spots[0][1], spots[2][1]);
    assert!(spots[3][1] < spots[0][1]);
    assert_eq!(spots[3][0], spots[0][0]);
}

#[test]
fn two_nodes_at_one_spot_part_and_far_points_are_held_in_range() {
    let graph = Graph {
        widths: vec![60.0; 2],
        groups: vec![None; 2],
        edges: vec![(0, 1)],
        written: vec![(0, 1)],
        ids: vec!["a".into(), "a".into()],
    };
    let spots = force::lay_out(&graph, &[Some([0.0; 3]), Some([0.0; 3])]);
    assert!(apart(spots[0], spots[1]) >= 120.0, "{spots:?}");
    assert_eq!(millimetres(1e9), i16::MAX);
    assert_eq!(normalised([0.0; 3]), None);
    assert_eq!(thinned(vec![[0.0; 3]; 3]).len(), 3);
}

#[test]
fn force_lays_out_one_node_parts_what_overlaps_and_turns_a_line_flat() {
    // One node, and nodes of no edges at all, which stand one more than the furthest apart.
    for body in ["a\n", "a\nb\nc --- d\n"] {
        let mut diagram = parse(body).0;
        diagram.layout = tomori_protocol::diagram::Layout::Force;
        lay_out(&mut diagram, None);
        assert!(diagram.extent.iter().all(|side| *side > 0));
    }
    let graph = Graph {
        widths: vec![60.0; 3],
        groups: vec![None; 3],
        edges: Vec::new(),
        written: Vec::new(),
        ids: vec!["a".into(), "b".into(), "c".into()],
    };
    // Overlapping, or at one spot, they are pushed apart until none overlaps.
    let mut spots = vec![[0.0; 3], [0.0; 3], [10.0, 0.0, 0.0]];
    force::separate(&graph, &mut spots);
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert!(apart(spots[a], spots[b]) >= 119.0, "{spots:?}");
    }
    // Along one line, they are turned to lie across, the first on the left.
    let mut line = vec![[0.0, 0.0, 300.0], [0.0, 0.0, 0.0], [0.0, 0.0, -300.0]];
    force::turned(&mut line);
    assert!(line[0][0] < -299.0 && line[2][0] > 299.0, "{line:?}");
    assert!(
        line.iter()
            .all(|spot| spot[1].abs() < 1e-6 && spot[2].abs() < 1e-6),
        "{line:?}"
    );
    // At one spot, they stay there.
    let mut one = vec![[5.0; 3]; 2];
    force::turned(&mut one);
    assert_eq!(one, [[0.0; 3]; 2]);
}

#[test]
fn a_layered_layout_laid_out_again_across_keeps_its_ranks_order() {
    let (before, _) = laid("flowchart LR\nhub --> a\nhub --> b\nhub --> c\n", None);
    let (after, _) = laid(
        "flowchart LR\nhub --> new\nhub --> a\nhub --> b\nhub --> c\n",
        Some(&before),
    );
    let y = |diagram: &Diagram, id: &str| at(diagram, id)[1];
    // Down the rank in the order they stood, the new one among them where its edge draws it.
    assert!(y(&after, "a") > y(&after, "b") && y(&after, "b") > y(&after, "c"));
}
