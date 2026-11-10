// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::{Align, Grid, Join, Pose, WallKind, WallShape};

fn wall(id: &str, pose: Pose, (columns, rows): (u32, u32), scale: f32) -> Wall {
    Wall::new(id, WallKind::Added, pose)
        .with_grid(Grid::new(columns, rows).unwrap())
        .with_scale(scale)
}

/// A two by two wall and a big one by one snapped to its right, their tops aligned, grouped as the server sends them.
fn pair() -> (Wall, Wall) {
    let two = wall(
        "wall-1",
        Pose::upright_facing([-0.4, 1.5, -2.5], [-0.4, 1.5, 0.0]),
        (2, 2),
        0.95,
    );
    let mut big = wall("wall-2", two.pose, (1, 1), 1.9);
    big.pose = group::attached_pose(&two, &big, Edge::Right, Align::Start).unwrap();
    (
        two.with_group(
            Some("wall-1".into()),
            vec![Join::new(Edge::Right, "wall-2")],
        ),
        big.with_group(Some("wall-1".into()), vec![Join::new(Edge::Left, "wall-1")]),
    )
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// The quads of `wall`'s board drawn in `colour`, in metres from its pose, as left, top, right and bottom.
fn quads(wall: &Wall, walls: &[&Wall], colour: [f32; 4]) -> Vec<[f32; 4]> {
    let unit = PIXEL * wall.scale();
    let sheets = sheets(wall, Mat4::IDENTITY, walls);
    sheets[0]
        .canvas
        .as_deref()
        .unwrap()
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Fill {
                x,
                y,
                width,
                height,
                color,
            } if color == colour => Some([
                x * unit,
                -y * unit,
                (x + width) * unit,
                -(y + height) * unit,
            ]),
            _ => None,
        })
        .collect()
}

#[test]
fn a_group_s_boards_meet_at_the_seam_their_outer_edges_even_and_borders_only_where_none_meets_them()
{
    let (two, big) = pair();
    let walls = [&two, &big];
    let (own, other) = (board(&two, &walls), board(&big, &walls));
    let (u, y, _, _) = group::on_surface(&two.pose, None, &big.pose);
    let other = other.shifted(u, y);
    // Edge to edge at the seam, halfway across the gap between their slots, without overlapping.
    assert!(near(own.right, other.left));
    assert!(near(own.right - two.extent().right, big.gap() / 2.0));
    // Their tops even, at the larger wall's margin, as their outer sides are.
    assert!(near(own.top, other.top));
    assert!(near(own.left, two.extent().left - big.margin(Edge::Left)));
    // A border where nothing meets them: all round, but along the seam, where the two meet; below the big one, the
    // two by two's right edge still shows its border, down to its own bottom.
    let border = wall_border(&two);
    let lines = quads(&two, &walls, border);
    let upright = |line: &&[f32; 4]| line[2] - line[0] < 0.01;
    let right: Vec<&[f32; 4]> = lines
        .iter()
        .filter(upright)
        .filter(|line| near(line[2], own.right))
        .collect();
    assert_eq!(right.len(), 1, "{lines:?}");
    assert!(
        near(right[0][1], other.bottom) && near(right[0][3], own.bottom),
        "{right:?}"
    );
    assert!(lines.iter().any(|line| near(line[0], own.left)));
    assert!(lines.iter().any(|line| near(line[1], own.top)));
    let lines = quads(&big, &walls, border);
    let unit = board(&big, &walls);
    assert!(
        !lines
            .iter()
            .filter(upright)
            .any(|line| near(line[0], unit.left)),
        "{lines:?}"
    );
    // Its slots and its board are drawn as quads; the label once, on the top left wall.
    assert_eq!(quads(&two, &walls, SLOT).len(), 4);
    assert_eq!(quads(&big, &walls, SLOT).len(), 1);
    assert!(labelled(&two, &walls) && !labelled(&big, &walls));
    assert_eq!(sheets(&two, Mat4::IDENTITY, &walls).len(), 2);
    assert_eq!(sheets(&big, Mat4::IDENTITY, &walls).len(), 1);
    let label = sheets(&two, Mat4::IDENTITY, &walls)[1].clone();
    assert_eq!(label.canvas.unwrap().plain(), "WALL");
    // A wall the server says is joined to one that no longer stands beside it, as one carried away, draws its edge.
    let mut away = big.clone();
    away.pose.position[0] += 1.0;
    let lines = quads(&two, &[&two, &away], border);
    assert!(
        lines
            .iter()
            .filter(upright)
            .any(|line| near(line[2], board(&two, &[&two, &away]).right))
    );
    // A wall of a group alone is labelled.
    assert!(labelled(&two, &[&two]));
}

#[test]
fn a_curved_group_is_labelled_once_below_its_bottom_left_wall() {
    let radius = 2.4;
    let first = wall(
        "wall-1",
        Pose::upright_facing([0.0, 1.4, -radius], [0.0, 1.4, 0.0]),
        (2, 1),
        0.8,
    )
    .with_shape(WallShape::Arc { radius });
    let mut second = wall("wall-2", first.pose, (2, 1), 0.8).with_shape(WallShape::Arc { radius });
    second.pose = group::attached_pose(&first, &second, Edge::Left, Align::Start).unwrap();
    let mut third = wall("wall-3", first.pose, (1, 1), 0.8).with_shape(WallShape::Arc { radius });
    third.pose = group::attached_pose(&first, &third, Edge::Top, Align::Start).unwrap();
    let in_group = |wall: Wall| wall.with_group(Some("wall-1".into()), Vec::new());
    let (first, second, third) = (in_group(first), in_group(second), in_group(third));
    let walls = [&first, &second, &third];
    assert!(labelled(&second, &walls));
    assert!(!labelled(&first, &walls) && !labelled(&third, &walls));
}

#[test]
fn an_edge_s_border_is_what_no_wall_covers() {
    assert_eq!(uncovered((0.0, 1.0), &[]), [(0.0, 1.0)]);
    assert_eq!(
        uncovered((0.0, 1.0), &[(0.6, 0.8), (-1.0, 0.2), (0.7, 0.9)]),
        [(0.2, 0.6), (0.9, 1.0)]
    );
    assert!(uncovered((0.0, 1.0), &[(-1.0, 2.0)]).is_empty());
    assert!(uncovered((0.0, 1.0), &[(0.0, 0.9995)]).is_empty());
    assert_eq!(uncovered((0.0, 1.0), &[(2.0, 3.0)]), [(0.0, 1.0)]);
}
