// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What laying out walls' handles and bars costs a frame, as the participant turns their head among several walls and
//! a snapped group, and that they stand where they stood.
use super::*;
use tomori_protocol::{Align, Edge, Join, TOMORI_WALL};

const ADJUST: Rights = Rights {
    adjust: true,
    remove: false,
    snap: true,
    locks: true,
    lock: true,
};

/// Where the neck stands, which the head turns about.
const NECK: Vec3 = Vec3::new(0.0, 1.55, 0.0);

/// A wall of `columns` by `rows`, someone added, at `pose`, curved at `radius` or flat.
fn wall(id: &str, (columns, rows): (u32, u32), pose: Pose, radius: Option<f32>) -> Wall {
    let shape = radius.map_or(WallShape::Grid, |radius| WallShape::Arc { radius });
    Wall::new(id, WallKind::Added, pose)
        .with_grid(Grid::new(columns, rows).unwrap())
        .with_shape(shape)
}

/// `walls` snapped together as one group: the first at its pose, each after it attached to an earlier one as `attach`
/// says, by index.
fn group_of(walls: Vec<Wall>, attach: &[(usize, Edge, Align)]) -> Vec<Wall> {
    let mut placed: Vec<Wall> = Vec::new();
    for (index, mut wall) in walls.into_iter().enumerate() {
        if index > 0 {
            let (to, edge, align) = attach[index - 1];
            wall.pose = group::attached_pose(&placed[to], &wall, edge, align).unwrap();
        }
        placed.push(wall);
    }
    let first = placed[0].id.clone();
    let joined: Vec<Vec<Join>> = placed
        .iter()
        .map(|wall| {
            placed
                .iter()
                .filter(|other| other.id != wall.id)
                .filter_map(|other| {
                    group::joined(wall, other).map(|(edge, ..)| Join::new(edge, other.id.clone()))
                })
                .collect()
        })
        .collect();
    placed
        .into_iter()
        .zip(joined)
        .map(|(wall, joined)| wall.with_group(Some(first.clone()), joined))
        .collect()
}

/// Facing the neck from `at`.
fn facing_neck(at: [f32; 3]) -> Pose {
    Pose::upright_facing(at, NECK.to_array())
}

/// The downstream room: the Tomori wall, two walls of its own on either side, a group of three snapped above and beside
/// a wide wall ahead, and a curved pair behind and to the right.
fn room() -> Vec<Wall> {
    let mut walls = vec![
        Wall::new(
            TOMORI_WALL,
            WallKind::Tomori,
            Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
        )
        .with_shape(WallShape::Arc { radius: 2.3 }),
        wall("left", (3, 2), facing_neck([-2.6, 1.5, -0.6]), None),
        wall("right", (2, 2), facing_neck([2.4, 1.4, -1.2]), None),
    ];
    walls.extend(group_of(
        vec![
            wall("stack-1", (4, 1), facing_neck([0.0, 1.2, -3.2]), None),
            wall("stack-2", (1, 1), Pose::default(), None),
            wall("stack-3", (2, 1), Pose::default(), None),
        ],
        &[(0, Edge::Top, Align::Start), (0, Edge::Top, Align::End)],
    ));
    walls.extend(group_of(
        vec![
            wall("arc-1", (2, 2), facing_neck([1.8, 1.4, 2.2]), Some(3.0)),
            wall("arc-2", (2, 2), Pose::default(), Some(3.0)),
        ],
        &[(0, Edge::Right, Align::Start)],
    ));
    walls
}

/// The eyes, the head turned `yaw` radians from straight ahead about the neck.
fn eyes(yaw: f32) -> Vec3 {
    NECK + Quat::from_rotation_y(yaw) * Vec3::new(0.0, 0.1, -0.09)
}

/// The head's turn at frame `frame` of 72 a second: from side to side and back every two seconds, ±60°.
fn yaw(frame: usize) -> f32 {
    (frame as f32 / 144.0 * std::f32::consts::TAU).sin() * 60f32.to_radians()
}

fn input<'a>(projection: &'a Projection, viewer: Vec3, yaw: f32) -> Input<'a> {
    let hand = viewer + Vec3::new(0.2, -0.4, 0.0);
    Input {
        projection,
        // Pointing ahead of the face, over the walls.
        ray: Ray::new(hand, Quat::from_rotation_y(yaw) * Vec3::new(0.0, 0.6, -1.0)),
        pressed: false,
        released: false,
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        panel: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        free: false,
        viewer,
        rights: ADJUST,
        controls: Controls::Controllers,
        now: Instant::now(),
        dt: 1.0 / 72.0,
    }
}

fn frames(walls: &[Wall]) -> Vec<(&Wall, Mat4)> {
    walls
        .iter()
        .map(|wall| (wall, model(&wall.frame()).unwrap()))
        .collect()
}

/// Seen points the editor projected, and the time it took, over `frames` frames, the head turning as [`yaw`] says or
/// held still.
fn run(editor: &mut Editor, walls: &[Wall], count: usize, turning: bool) -> (u64, Duration) {
    let projection = Projection::new("s", "sky").with_walls(walls.to_vec());
    let drawn = frames(walls);
    SEEN.with(|seen| seen.set(0));
    let started = Instant::now();
    for frame in 0..count {
        let turn = if turning { yaw(frame) } else { 0.3 };
        let output = editor.update(input(&projection, eyes(turn), turn), &drawn);
        assert!(!output.sheets.is_empty());
    }
    (SEEN.with(std::cell::Cell::get), started.elapsed())
}

/// Every wall's bar and handles in `walls`, seen from `viewer`, written to a tenth of a millimetre.
fn layout(walls: &[Wall], viewer: Vec3) -> String {
    let all: Vec<&Wall> = walls.iter().collect();
    let mut written = String::new();
    for (wall, frame) in frames(walls) {
        let bar = group_bar(wall, frame, viewer, &all);
        let controls = LockControls::of(wall, ADJUST);
        let (points, size) = handle_points_among_with(wall, frame, viewer, &all, None, controls);
        written += &format!(
            "{} {:.4?} {:.4}",
            wall.id,
            bar.map(|bar| (bar.middle, bar.half)),
            size
        );
        for (handle, at) in points {
            written += &format!(" {handle:?} {at:.4}");
        }
        written += "\n";
    }
    written
}

/// Where the layouts are seen from: the head turned about the neck, near the group from below and to its side, and from
/// beside the curved pair, which folds as seen from there.
fn viewers() -> [Vec3; 8] {
    [
        eyes(-1.0),
        eyes(-0.3),
        eyes(0.0),
        eyes(0.4),
        eyes(1.0),
        Vec3::new(-1.0, 0.8, -2.4),
        Vec3::new(3.2, 1.6, 2.6),
        Vec3::new(0.9, 1.5, 1.2),
    ]
}

/// The numbers in `text`, and the rest of it with each number taken out.
fn numbers(text: &str) -> (String, Vec<f32>) {
    let (mut rest, mut numbers, mut number) = (String::new(), Vec::new(), String::new());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let starts = character.is_ascii_digit()
            || (character == '-' && characters.peek().is_some_and(char::is_ascii_digit));
        if !starts {
            rest.push(character);
            continue;
        }
        number.push(character);
        while let Some(next) = characters.next_if(|next| next.is_ascii_digit() || *next == '.') {
            number.push(next);
        }
        numbers.push(number.parse().unwrap());
        number.clear();
        rest.push('#');
    }
    (rest, numbers)
}

#[test]
fn handles_and_bars_stand_where_they_stood_before_their_layouts_were_kept_from_every_viewer() {
    // As laid out before each point was seen once and layouts were kept, written by `layout` then.
    let before: Vec<&str> = include_str!("layouts.txt").lines().collect();
    let walls = room();
    for (index, viewer) in viewers().into_iter().enumerate() {
        let now = layout(&walls, viewer);
        let then = &before[index * walls.len()..(index + 1) * walls.len()];
        for (now, then) in now.lines().zip(then) {
            let ((now_rest, now_numbers), (then_rest, then_numbers)) =
                (numbers(now), numbers(then));
            assert_eq!(now_rest, then_rest, "from {viewer}");
            for (now_number, then_number) in now_numbers.iter().zip(&then_numbers) {
                assert!(
                    (now_number - then_number).abs() <= 1e-3,
                    "from {viewer}: {now}\nwas {then}"
                );
            }
        }
    }
}

#[test]
fn turning_the_head_lays_out_handles_and_bars_every_few_frames_and_holding_it_still_never() {
    let walls = room();
    // Holding the wide wall of the group, its handles show too.
    let mut editor = Editor {
        wall: Some("stack-1".to_owned()),
        ..Editor::default()
    };
    let (first, _) = run(&mut editor, &walls, 1, false);
    assert!(first > 0, "the first frame lays them out");
    let (still, _) = run(&mut editor, &walls, 72, false);
    assert_eq!(still, 0, "held still, nothing is laid out again");
    let projection = Projection::new("s", "sky").with_walls(walls.clone());
    let drawn = frames(&walls);
    let mut laid = 0;
    SEEN.with(|seen| seen.set(0));
    for frame in 0..288 {
        let before = SEEN.with(std::cell::Cell::get);
        editor.update(input(&projection, eyes(yaw(frame)), yaw(frame)), &drawn);
        laid += usize::from(SEEN.with(std::cell::Cell::get) > before);
    }
    let seen = SEEN.with(std::cell::Cell::get);
    // Every frame saw 25,725 points before, of which the walls' bars 18,400.
    assert!(laid < 288 / 2, "laid out in {laid} of 288 frames");
    assert!(seen / 288 < 1_000, "{} points seen a frame", seen / 288);
}

#[test]
fn turning_the_head_shows_handles_and_bars_within_a_twentieth_of_a_degree_of_where_laid_out_afresh()
{
    let walls = room();
    let projection = Projection::new("s", "sky").with_walls(walls.clone());
    let drawn = frames(&walls);
    let mut kept = Editor {
        wall: Some("stack-1".to_owned()),
        ..Editor::default()
    };
    for frame in 0..144 {
        let (turn, viewer) = (yaw(frame), eyes(yaw(frame)));
        let mut afresh = Editor {
            wall: kept.wall.clone(),
            ..Editor::default()
        };
        let (shown, laid) = (
            kept.update(input(&projection, viewer, turn), &drawn),
            afresh.update(input(&projection, viewer, turn), &drawn),
        );
        assert_eq!(shown.sheets.len(), laid.sheets.len(), "frame {frame}");
        for (shown, laid) in shown.sheets.iter().zip(&laid.sheets) {
            let (shown, laid) = (shown.model.unwrap(), laid.model.unwrap());
            let (shown, laid) = (shown.w_axis.truncate(), laid.w_axis.truncate());
            // As seen: a few hundredths of a degree, a fiftieth of a handle.
            let off = shown.distance(laid) / laid.distance(viewer);
            assert!(
                off < 1e-3,
                "frame {frame}: {} degrees off",
                off.to_degrees()
            );
        }
    }
}

#[test]
fn a_bar_is_laid_out_again_as_its_wall_or_its_group_moves_or_the_eyes_move_a_centimetre() {
    let mut walls = room();
    let mut editor = Editor::default();
    let viewer = eyes(0.0);
    // The bar of the wall above the group's wide one, as the editor shows it, and as laid out afresh.
    let shown = |editor: &mut Editor, walls: &[Wall], viewer: Vec3| {
        let projection = Projection::new("s", "sky").with_walls(walls.to_vec());
        let drawn = frames(walls);
        SEEN.with(|seen| seen.set(0));
        editor.update(input(&projection, viewer, 0.0), &drawn);
        let seen = SEEN.with(std::cell::Cell::get);
        let all: Vec<&Wall> = walls.iter().collect();
        let (wall, frame) = drawn[4];
        let kept = editor.layouts.bar(wall, frame, viewer, &all).unwrap();
        (seen, kept, group_bar(wall, frame, viewer, &all).unwrap())
    };
    let (seen, kept, afresh) = shown(&mut editor, &walls, viewer);
    assert!(seen > 0);
    assert_eq!(kept, afresh);
    // The eyes moved less than a centimetre: as it was.
    let near = viewer + Vec3::new(0.006, 0.0, 0.0);
    let (seen, kept, afresh) = shown(&mut editor, &walls, near);
    assert_eq!(seen, 0);
    assert_ne!(kept, afresh, "laid out for where the eyes were");
    assert!(kept.middle.distance(afresh.middle) < 1e-4);
    // Further: laid out again, for where they are.
    let far = viewer + Vec3::new(0.02, 0.0, 0.0);
    let (seen, kept, afresh) = shown(&mut editor, &walls, far);
    assert!(seen > 0);
    assert_eq!(kept, afresh);
    // The wide wall below it lower, its slots about it: laid out again.
    let (moved, _, _) = shown(&mut editor, &walls, far);
    assert_eq!(moved, 0);
    walls[3].pose.position[1] -= 0.2;
    let (seen, kept, afresh) = shown(&mut editor, &walls, far);
    assert!(seen > 0);
    assert_eq!(kept, afresh);
    // Its own wall moved: laid out again.
    walls[4].pose.position[0] += 0.1;
    let (seen, kept, afresh) = shown(&mut editor, &walls, far);
    assert!(seen > 0);
    assert_eq!(kept, afresh);
    // A wall no longer shown is forgotten.
    let gone = walls.remove(1);
    let projection = Projection::new("s", "sky").with_walls(walls.clone());
    editor.update(input(&projection, far, 0.0), &frames(&walls));
    assert!(
        editor
            .layouts
            .kept
            .iter()
            .all(|kept| kept.wall.id != gone.id)
    );
    assert!(editor.layouts.kept.iter().all(|kept| !kept.used));
}
