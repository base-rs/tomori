// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::diagram::{Edge, Node};

/// A diagram of `nodes` at their points, in millimetres, joined by `edges`, each from, to and which way.
fn drawn(nodes: &[[i16; 3]], edges: &[(u16, u16, Arrow)]) -> Diagram {
    let mut diagram = Diagram::default();
    diagram.nodes = nodes
        .iter()
        .enumerate()
        .map(|(index, at)| {
            let mut node = Node::new(format!("n{index}"), "N");
            node.at = *at;
            node
        })
        .collect();
    diagram.edges = edges
        .iter()
        .map(|(from, to, direction)| {
            let mut edge = Edge::new(*from, *to);
            edge.direction = *direction;
            edge
        })
        .collect();
    diagram
}

/// Run `springs` on for `seconds`, seen from in front.
fn run(springs: &mut Springs, seconds: f32) {
    for _ in 0..(seconds / STEP).round() as usize {
        springs.step(STEP, Vec3::new(0.0, 0.0, 2_000.0));
    }
}

/// A hub, `n0`, fed by `n1` and `n2`, feeding `n3` and `n4`, joined both ways to `n5`, and `n6` and `n7` apart.
fn hub() -> Springs {
    Springs::new(&drawn(
        &[
            [0, 0, 0],
            [-100, 200, 0],
            [100, 200, 0],
            [-100, -200, 0],
            [100, -200, 0],
            [250, 0, 0],
            [-300, 0, 0],
            [300, 300, 0],
        ],
        &[
            (1, 0, Arrow::Forward),
            (0, 2, Arrow::Back),
            (0, 3, Arrow::Forward),
            (4, 0, Arrow::Back),
            (0, 5, Arrow::Both),
            (6, 7, Arrow::None),
        ],
    ))
}

#[test]
fn a_node_held_fans_what_feeds_it_left_what_it_feeds_right_and_the_rest_falls_back() {
    let mut springs = hub();
    assert_eq!(
        springs.neighbours(0),
        [
            (1, Side::In),
            (2, Side::In),
            (3, Side::Out),
            (4, Side::Out),
            (5, Side::Both)
        ]
    );
    // Pulled 40 cm towards its holder, it is where the hand holds it, and its neighbours follow it.
    let target = Vec3::new(0.0, 0.0, 400.0);
    springs.hold(0, 0, target);
    run(&mut springs, 3.0);
    let at = springs.positions();
    assert_eq!(at[0], target);
    assert_eq!(springs.held(0), Some(0));
    for incoming in [1, 2] {
        assert!(
            at[incoming].x < -FAN_MM * 0.6,
            "{incoming} {:?}",
            at[incoming]
        );
        assert!(at[incoming].z > 250.0, "{incoming} {:?}", at[incoming]);
    }
    for outgoing in [3, 4] {
        assert!(
            at[outgoing].x > FAN_MM * 0.6,
            "{outgoing} {:?}",
            at[outgoing]
        );
    }
    // In the order they stood, the higher above the lower.
    assert!(at[1].y > at[2].y && at[3].y > at[4].y);
    assert!(at[5].y > FAN_MM * 0.6, "{:?}", at[5]);
    // Those not joined to it fall back, away from its holder.
    for apart in [6, 7] {
        assert!(at[apart].z < -100.0, "{apart} {:?}", at[apart]);
    }
    // It comes to rest, and sleeps once nothing is held.
    assert!(!springs.asleep());
    springs.let_go(0, true);
    run(&mut springs, 3.0);
    assert!(springs.asleep());
}

#[test]
fn let_go_outside_a_node_springs_home_and_inside_it_stays_pinned_until_unpinned() {
    let mut springs = hub();
    let home = springs.positions().to_vec();
    springs.hold(0, 0, Vec3::new(0.0, 0.0, 600.0));
    run(&mut springs, 1.0);
    springs.let_go(0, false);
    assert!(!springs.done());
    run(&mut springs, 3.0);
    assert!(springs.done());
    for (at, home) in springs.positions().iter().zip(&home) {
        assert!(at.distance(*home) <= 0.5, "{at} {home}");
    }
    // Let go within it, it stays where it was let go, the rest settling about it, until unpinned.
    let pin = Vec3::new(120.0, 80.0, 0.0);
    springs.hold(1, 6, pin);
    run(&mut springs, 0.5);
    springs.let_go(1, true);
    run(&mut springs, 5.0);
    assert!(springs.pinned(6) && springs.any_pinned());
    assert_eq!(springs.positions()[6], pin);
    assert!(springs.asleep() && !springs.done());
    // Taken up again, it is held, not pinned; let go outside and unpinned, everything is home.
    springs.hold(0, 6, pin);
    assert!(!springs.pinned(6));
    springs.let_go(0, true);
    springs.unpin();
    assert!(!springs.any_pinned());
    run(&mut springs, 3.0);
    assert!(springs.done());
}

#[test]
fn two_hands_hold_two_nodes_and_one_node_passes_between_them() {
    let mut springs = hub();
    springs.hold(0, 3, Vec3::new(-300.0, -300.0, 100.0));
    springs.hold(1, 7, Vec3::new(400.0, 400.0, 100.0));
    run(&mut springs, 1.0);
    assert_eq!(springs.positions()[3], Vec3::new(-300.0, -300.0, 100.0));
    assert_eq!(springs.positions()[7], Vec3::new(400.0, 400.0, 100.0));
    // The left hand takes the right's node, which then holds nothing.
    springs.hold(1, 3, Vec3::ZERO);
    assert_eq!((springs.held(0), springs.held(1)), (None, Some(3)));
    // Moving or letting go of a hand holding nothing does nothing, nor holding a node it does not have.
    springs.move_to(0, Vec3::ONE);
    springs.let_go(0, true);
    springs.hold(0, 99, Vec3::ZERO);
    assert_eq!(springs.held(0), None);
    assert!(!springs.any_pinned());
}

#[test]
fn it_is_bounded_whatever_it_is_given() {
    // Two nodes in one place part, whichever way.
    let mut springs = Springs::new(&drawn(
        &[[0, 0, 0], [0, 0, 0], [0, 0, 10]],
        &[(0, 1, Arrow::Forward)],
    ));
    springs.hold(0, 2, Vec3::new(0.0, 0.0, 10.0));
    run(&mut springs, 1.0);
    let at = springs.positions();
    assert!(at.iter().all(|at| at.is_finite()));
    assert!(at[0].distance(at[1]) > APART_MM * 0.5, "{at:?}");
    // A long frame runs at most a few steps, and nothing for one not finite, or for a viewer not finite.
    let mut long = hub();
    long.hold(0, 0, Vec3::new(0.0, 0.0, 400.0));
    long.step(10.0, Vec3::new(0.0, 0.0, 2_000.0));
    let mut few = hub();
    few.hold(0, 0, Vec3::new(0.0, 0.0, 400.0));
    for _ in 0..MOST_STEPS {
        few.step(STEP, Vec3::new(0.0, 0.0, 2_000.0));
    }
    assert_eq!(long.positions(), few.positions());
    long.step(f32::NAN, Vec3::NAN);
    assert_eq!(long.positions(), few.positions());
    // A far pull is held to the fastest a node moves.
    let mut far = hub();
    far.hold(0, 0, Vec3::new(0.0, 0.0, 25_000.0));
    far.step(STEP, Vec3::new(0.0, 0.0, 30_000.0));
    let moved = far.positions()[1].distance(Vec3::new(-100.0, 200.0, 0.0));
    assert!(moved <= FASTEST * STEP + 1e-3, "{moved}");
    // Two edges between one pair, one each way, fan it both ways; two alike, once; one both ways, either end.
    let pairs = Springs::new(&drawn(
        &[[0, 0, 0], [100, 0, 0], [0, 100, 0]],
        &[
            (0, 1, Arrow::Forward),
            (1, 0, Arrow::Forward),
            (0, 2, Arrow::Forward),
            (0, 2, Arrow::Forward),
            (2, 0, Arrow::None),
        ],
    ));
    assert_eq!(pairs.neighbours(0), [(1, Side::Both), (2, Side::Both)]);
    assert_eq!(pairs.neighbours(2), [(0, Side::Both)]);
    // An edge to a node it does not have is no spring.
    let mut odd = drawn(&[[0, 0, 0]], &[]);
    odd.edges.push(Edge::new(0, 5));
    assert!(Springs::new(&odd).neighbours(0).is_empty());
}

#[test]
fn the_path_between_two_nodes_is_the_shortest_by_edges_either_way() {
    let diagram = drawn(
        &[[0, 0, 0]; 6],
        &[
            (0, 1, Arrow::Forward),
            (1, 2, Arrow::Forward),
            (2, 3, Arrow::Forward),
            (3, 0, Arrow::Back),
            (4, 0, Arrow::Forward),
        ],
    );
    assert_eq!(path(&diagram, 0, 2), Some((vec![0, 1, 2], vec![0, 1])));
    // Against an edge's way, and the shorter of two.
    assert_eq!(path(&diagram, 4, 3), Some((vec![4, 0, 3], vec![4, 3])));
    assert_eq!(path(&diagram, 2, 2), Some((vec![2], vec![])));
    assert_eq!(path(&diagram, 0, 5), None);
    assert_eq!(path(&diagram, 0, 9), None);
}
