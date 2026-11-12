// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::{Grid, Projection};

const SKY: Walls<'static> = Walls {
    environment: "sky",
    tomori: Pose {
        position: [2.0, 1.5, -2.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    },
    declared: &[],
    status: crate::stage::Status::Beneath,
};

/// Eyes at home, looking along -Z.
const EYES: [f32; 3] = [0.0, 1.6, 0.0];

fn head() -> Pose {
    Pose::upright_facing(EYES, [0.0, 1.6, 1.0])
}

fn wall(id: &str, (columns, rows): (u32, u32), scale: f32) -> Planned {
    Planned {
        id: id.into(),
        layout: Layout {
            grid: Grid::new(columns, rows).unwrap(),
            scale,
            aspect: None,
            radius: None,
            tilt: None,
            content_scale: None,
        },
        width: None,
        sized: true,
        attach: None,
        context: Vec::new(),
    }
}

fn attached(mut planned: Planned, to: &str, edge: Edge, align: Align) -> Planned {
    planned.attach = Some((to.into(), edge, align));
    planned
}

fn filling_edge(mut planned: Planned) -> Planned {
    planned.sized = false;
    planned
}

/// In front of home, level, a little below the eyes, at each distance the hub tries, as `presence` puts them.
fn middles(extent: group::Extent, radius: Option<f32>) -> Vec<Pose> {
    crate::presence::layout_middles(&head(), false, extent.right - extent.left, radius)
}

fn view(stage: &Stage) -> Projection {
    stage.view("s", &SKY, 0)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 2e-3
}

/// How far `wall`'s middle is from home, level.
fn level(wall: &Wall) -> f32 {
    let [x, _, z] = wall.pose.position;
    x.hypot(z)
}

#[test]
fn a_two_by_two_and_a_big_one_beside_it_are_built_as_one_group_in_front_of_whoever_asked() {
    let mut stage = Stage::default();
    let planned = [
        wall("summary", (2, 2), 0.9),
        filling_edge(attached(
            wall("chart", (1, 1), 1.0),
            "summary",
            Edge::Right,
            Align::Start,
        )),
    ];
    let ids = stage.compose(&SKY, &planned, &middles, EYES).unwrap();
    let projection = view(&stage);
    let (two, big) = (
        projection.wall(&ids[0]).unwrap(),
        projection.wall(&ids[1]).unwrap(),
    );
    // One group, joined, the big one as tall as the two by two, its slots a gutter from the other's.
    assert_eq!(
        (two.group.as_ref(), big.group.as_ref()),
        (Some(&ids[0]), Some(&ids[0]))
    );
    assert_eq!(two.joined.len(), 1);
    let (u, y, _, _) = group::on_surface(&two.pose, None, &big.pose);
    let (own, other) = (two.extent(), big.extent().shifted(u, y));
    assert!(
        near(other.top, own.top) && near(other.bottom, own.bottom),
        "{own:?} {other:?}"
    );
    assert!((big.scale() - 1.91).abs() < 1e-4);
    // Its middle straight ahead of home, a little below the eyes, facing home, far enough to be seen whole.
    let middle = (own.left + other.right) / 2.0;
    let centre = group::along(&two.pose, None, middle, (own.top + own.bottom) / 2.0);
    assert!(
        near(centre.position[0], 0.0) && near(centre.position[1], 1.5),
        "{centre:?}"
    );
    let width = other.right - own.left;
    assert!(
        near(-centre.position[2], (width / 2.0 / 0.577).max(1.5)),
        "{centre:?}"
    );
    assert!(two.pose.orientation[1].abs() < 1e-4);
}

#[test]
fn a_curved_layout_curves_about_whoever_it_is_for_and_a_wide_one_comes_nearer_to_be_read() {
    let mut stage = Stage::default();
    let mut first = wall("left", (2, 1), 0.8);
    first.layout.radius = Some(2.2);
    let mut second = attached(
        wall("right", (2, 1), 0.8),
        "left",
        Edge::Right,
        Align::Middle,
    );
    second.layout.radius = Some(2.2);
    let ids = stage
        .compose(&SKY, &[first, second], &middles, EYES)
        .unwrap();
    let projection = view(&stage);
    for id in &ids {
        let wall = projection.wall(id).unwrap();
        // Each faces home, its arc's line through the eyes.
        let ahead = wall.pose;
        let (_, _, depth, _) = group::on_surface(
            &ahead,
            Some(2.2),
            &Pose::upright_facing([EYES[0], ahead.position[1], EYES[2]], ahead.position),
        );
        assert!(near(depth, 2.2), "{depth}");
        assert_eq!(wall.radius(), Some(2.2));
    }
    // Eight small slots across are too far to read at the edges from where they would be seen whole, or from 2 m, so
    // the wall comes nearer, to 1.6 m.
    let mut stage = Stage::default();
    let ids = stage
        .compose(&SKY, &[wall("strip", (8, 1), 0.5)], &middles, EYES)
        .unwrap();
    let strip = view(&stage).wall(&ids[0]).unwrap().clone();
    assert!(near(level(&strip), 1.6), "{}", level(&strip));
    assert!(readable(&strip, EYES));
    // Three such strips side by side are too far to read at their ends wherever they stand, and nothing is built.
    let mut stage = Stage::default();
    let strips = [
        wall("a", (8, 1), 0.5),
        attached(wall("b", (8, 1), 0.5), "a", Edge::Right, Align::Start),
        attached(wall("c", (8, 1), 0.5), "a", Edge::Left, Align::Start),
    ];
    let refused = stage.compose(&SKY, &strips, &middles, EYES).unwrap_err();
    assert!(refused.contains("too far to read"), "{refused}");
    assert!(stage.is_empty());
}

#[test]
fn a_layout_is_checked_whole_and_refused_whole() {
    let mut stage = Stage::default();
    let refused = |stage: &mut Stage, planned: &[Planned]| {
        stage.compose(&SKY, planned, &middles, EYES).unwrap_err()
    };
    assert_eq!(refused(&mut stage, &[]), "A layout has at least one wall.");
    let many: Vec<Planned> = (0..7)
        .map(|index| {
            let planned = wall(&format!("w{index}"), (1, 1), 1.0);
            if index == 0 {
                planned
            } else {
                attached(
                    planned,
                    &format!("w{}", index - 1),
                    Edge::Right,
                    Align::Start,
                )
            }
        })
        .collect();
    assert!(refused(&mut stage, &many).starts_with("A layout has at most 6 walls"));
    let first = wall("a", (1, 1), 1.0);
    assert!(
        refused(
            &mut stage,
            &[attached(first.clone(), "b", Edge::Left, Align::Start)]
        )
        .starts_with("A layout's first wall is attached to nothing")
    );
    assert_eq!(
        refused(&mut stage, &[first.clone(), first.clone()]),
        "Two walls of the layout are called \"a\"."
    );
    assert_eq!(
        refused(&mut stage, &[first.clone(), wall("b", (1, 1), 1.0)]),
        "Say which earlier wall \"b\" is attached to, and across which edge."
    );
    assert_eq!(
        refused(
            &mut stage,
            &[
                first.clone(),
                attached(wall("b", (1, 1), 1.0), "c", Edge::Left, Align::Start)
            ]
        ),
        "\"b\" is attached to \"c\", which is not an earlier wall of the layout."
    );
    let mut curved = attached(wall("b", (1, 1), 1.0), "a", Edge::Left, Align::Start);
    curved.layout.radius = Some(3.0);
    assert_eq!(
        refused(&mut stage, &[first.clone(), curved]),
        "A layout's walls are all flat, or all curved at the same radius."
    );
    let mut wide = wall("a", (1, 1), 1.0);
    wide.width = Some(f32::NAN);
    assert_eq!(
        refused(&mut stage, &[wide]),
        "A width is a number of metres."
    );
    // Two walls attached to the same place overlap.
    let same = |id: &str| attached(wall(id, (1, 1), 1.0), "a", Edge::Right, Align::Start);
    assert_eq!(
        refused(&mut stage, &[first.clone(), same("b"), same("c")]),
        "Walls cannot overlap: move it beside the other, or make one smaller."
    );
    let unknown = attached(wall("b", (1, 1), 1.0), "a", Edge::Unknown, Align::Start);
    assert_eq!(
        refused(&mut stage, &[first.clone(), unknown]),
        "Tomori does not know that edge or alignment."
    );
    // With all but one of the walls an environment takes added already, two more are too many, and none is built.
    for index in 0..crate::stage::ADDED_WALLS - 1 {
        let at = [
            (index % 12) as f32 * 3.0 - 16.5,
            1.5,
            5.0 + (index / 12) as f32 * 2.5,
        ];
        stage
            .add_wall(&SKY, &Pose::upright_facing(at, EYES), None)
            .unwrap();
    }
    let before = view(&stage);
    let two = [
        first.clone(),
        attached(wall("b", (1, 1), 1.0), "a", Edge::Right, Align::Start),
    ];
    assert_eq!(
        refused(&mut stage, &two),
        "This environment has as many added walls as it can take."
    );
    assert_eq!(view(&stage), before);
}

#[test]
fn a_layout_given_a_width_is_that_wide_and_one_where_a_wall_stands_comes_nearer() {
    let mut stage = Stage::default();
    let mut planned = wall("wide", (3, 1), 1.0);
    planned.width = Some(3.0);
    // One below it without a size of its own is as wide as its slots.
    let below = filling_edge(attached(
        wall("below", (3, 1), 1.0),
        "wide",
        Edge::Bottom,
        Align::Middle,
    ));
    let ids = stage
        .compose(&SKY, &[planned, below], &middles, EYES)
        .unwrap();
    let shown = view(&stage).wall(&ids[0]).unwrap().clone();
    assert!((shown.size().0 - 3.0).abs() < 0.03, "{:?}", shown.size());
    let under = view(&stage).wall(&ids[1]).unwrap().clone();
    let (wide, narrow) = (shown.extent(), under.extent());
    assert!((wide.right - wide.left - (narrow.right - narrow.left)).abs() < 0.02);
    // A wall standing where the next layout's middle would first stand: it comes nearer, clear of it.
    let again = wall("again", (3, 1), 1.0);
    let alone = again
        .layout
        .apply(Wall::new("", WallKind::Added, Pose::default()));
    let at = middles(alone.extent(), None)[0];
    let first = at.position[0].hypot(at.position[2]);
    let mut stage = Stage::default();
    stage.add_wall(&SKY, &at, None).unwrap();
    let ids = stage
        .compose(&SKY, std::slice::from_ref(&again), &middles, EYES)
        .unwrap();
    let placed = view(&stage).wall(&ids[0]).unwrap().clone();
    assert!(level(&placed) < first - 0.1, "{} {first}", level(&placed));
    // Where it would overlap a wall at every distance, it is refused, saying so.
    let mut stage = Stage::default();
    for at in middles(alone.extent(), None) {
        stage.add_wall(&SKY, &at, None).unwrap();
    }
    assert_eq!(
        stage.compose(&SKY, &[again], &middles, EYES).unwrap_err(),
        "It would overlap a wall already here: stand it elsewhere, or move that wall first."
    );
}
