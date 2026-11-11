// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

const FRAME: f32 = 1.0 / 72.0;

fn at(position: Vec3) -> Anchor {
    Anchor {
        position,
        orientation: Quat::IDENTITY,
    }
}

/// `anchor` turned to `yaw` about the vertical.
fn turned(anchor: Anchor, yaw: f32) -> Anchor {
    Anchor {
        orientation: Quat::from_rotation_y(yaw),
        ..anchor
    }
}

/// The way `anchor` faces about the vertical, as `Pose::yaw` measures it.
fn yaw(anchor: &Anchor) -> f32 {
    let ahead = anchor.orientation * Vec3::Z;
    ahead.x.atan2(ahead.z)
}

/// A glide standing at `anchor`.
fn standing(anchor: Anchor) -> Glide {
    let mut glide = Glide::default();
    glide.step(anchor, None, false, false, 0.0);
    glide
}

/// The places `glide` passes through, frame by frame at `dt`, for `seconds`, going to `target`.
fn frames_at(
    glide: &mut Glide,
    target: Anchor,
    via: Option<Vec3>,
    stream: bool,
    seconds: f32,
    dt: f32,
) -> Vec<Anchor> {
    (0..(seconds / dt).round() as usize)
        .map(|_| glide.step(target, via, stream, false, dt))
        .collect()
}

/// The same, at 72 Hz.
fn frames(
    glide: &mut Glide,
    target: Anchor,
    via: Option<Vec3>,
    stream: bool,
    seconds: f32,
) -> Vec<Anchor> {
    frames_at(glide, target, via, stream, seconds, FRAME)
}

/// The velocity between each pair of frames `dt` apart.
fn velocities(path: &[Anchor], dt: f32) -> Vec<Vec3> {
    path.windows(2)
        .map(|pair| (pair[1].position - pair[0].position) / dt)
        .collect()
}

/// The most that `values`, a frame apart, change from one frame to the next, a second.
fn most_change<T: Copy>(values: &[T], change: impl Fn(T, T) -> f32) -> f32 {
    values
        .windows(2)
        .map(|pair| change(pair[0], pair[1]) / FRAME)
        .fold(0.0, f32::max)
}

/// The angular velocity between each pair of frames `dt` apart, about the vertical.
fn spins(path: &[Anchor], dt: f32) -> Vec<f32> {
    path.windows(2)
        .map(|pair| {
            let turn = pair[1].orientation * pair[0].orientation.inverse();
            rotation(turn).y / dt
        })
        .collect()
}

#[test]
fn it_appears_where_it_first_stands_and_where_it_is_told_to() {
    let mut glide = Glide::default();
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    assert_eq!(glide.step(home, None, false, false, 0.1), home);
    let yard = turned(at(Vec3::new(4.0, 1.4, -6.0)), 2.0);
    assert_eq!(glide.step(yard, None, false, true, 0.1), yard);
    // Appeared, it stays there, still, and no time moves nothing.
    assert_eq!(glide.step(yard, None, false, false, 0.1), yard);
    assert_eq!(glide.step(home, None, false, false, 0.0), yard);
}

#[test]
fn it_eases_in_cruises_at_its_speed_and_settles_without_overshooting() {
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let mut glide = standing(home);
    let there = at(Vec3::new(3.0, 1.4, -2.3));
    let frames = frames(&mut glide, there, None, false, 8.0);
    let speeds: Vec<f32> = velocities(&frames, FRAME)
        .iter()
        .map(|velocity| velocity.length())
        .collect();
    // Never faster than a glide, gaining speed gently, and never past the place, coming ever nearer.
    assert!(speeds.iter().all(|speed| *speed <= SPEED + 1e-3));
    assert!(speeds[0] < 0.1);
    assert!(speeds.iter().any(|speed| *speed > SPEED - 0.01));
    assert!(frames.iter().all(|frame| frame.position.x <= 3.0 + 1e-5));
    assert!(
        frames
            .windows(2)
            .all(|pair| pair[1].position.x >= pair[0].position.x)
    );
    // Its speed, and how fast that changes, rise and fall without a step, even as it sets off and as it stands still.
    let mut moving = vec![Vec3::ZERO];
    moving.extend(velocities(&frames, FRAME));
    moving.push(Vec3::ZERO);
    let pushes = velocities_of(&moving);
    assert!(most_change(&moving, Vec3::distance) < 2.0);
    assert!(most_change(&pushes, Vec3::distance) < 30.0);
    assert_eq!(frames.last().unwrap().position, there.position);
    // A stream is faster.
    let mut glide = standing(home);
    let far = at(Vec3::new(9.0, 1.4, -2.3));
    let streamed = frames_until(&mut glide, far, None, true);
    assert!(streamed < 3.0, "{streamed}");
}

/// How fast each of `velocities`, a frame apart, changes.
fn velocities_of(velocities: &[Vec3]) -> Vec<Vec3> {
    velocities
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) / FRAME)
        .collect()
}

/// How long `glide` takes to reach `target`, in seconds.
fn frames_until(glide: &mut Glide, target: Anchor, via: Option<Vec3>, stream: bool) -> f32 {
    let mut seconds = 0.0;
    while glide.step(target, via, stream, false, FRAME).position != target.position {
        seconds += FRAME;
        assert!(seconds < 60.0, "never arrived");
    }
    seconds
}

#[test]
fn it_goes_by_its_waypoint_and_turns_round_smoothly_when_sent_back() {
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let mut glide = standing(home);
    let there = at(Vec3::new(2.0, 1.4, -2.3));
    let over = Vec3::new(1.0, 2.6, -2.3);
    let path = frames(&mut glide, there, Some(over), false, 8.0);
    // It rises to the waypoint before coming down to its place.
    assert!(path.iter().any(|frame| frame.position.y > 2.5));
    assert_eq!(path.last().unwrap().position, there.position);
    // Sent back on its way, it slows and turns round rather than reversing at once.
    let mut glide = standing(home);
    frames(&mut glide, there, None, false, 1.5);
    let before = glide.step(there, None, false, false, FRAME);
    let back = frames(&mut glide, home, None, false, 0.2);
    assert!(back[0].position.x >= before.position.x - 1e-3);
    let path = frames(&mut glide, home, None, false, 8.0);
    assert_eq!(path.last().unwrap().position, home.position);
    // A waypoint that is not finite is ignored.
    let mut glide = standing(home);
    let path = frames(&mut glide, there, Some(Vec3::NAN), false, 8.0);
    assert!(path.iter().all(|frame| frame.position.y == 1.4));
}

#[test]
fn a_new_place_to_go_on_its_way_carries_on_from_how_it_moves() {
    // Gliding 4 m along X, it is sent 3 m along Z instead, then by a waypoint, then streamed far off, then back.
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let mut glide = standing(home);
    let mut path = vec![home];
    let goes = [
        (at(Vec3::new(4.0, 1.4, -2.3)), None, false, 1.3),
        (at(Vec3::new(0.0, 1.4, 0.7)), None, false, 1.1),
        (
            at(Vec3::new(-2.0, 1.4, -2.3)),
            Some(Vec3::new(-1.0, 2.6, -1.0)),
            false,
            2.0,
        ),
        (at(Vec3::new(-12.0, 1.4, -8.0)), None, true, 0.7),
        (at(Vec3::new(1.0, 1.4, -2.0)), None, false, 9.0),
    ];
    for (target, via, stream, seconds) in goes {
        path.extend(frames(&mut glide, target, via, stream, seconds));
    }
    // Its velocity never jumps, even in the frame it is sent elsewhere, nor, gliding, does how fast it changes.
    let velocities = velocities(&path, FRAME);
    let streaming = (4.4 / FRAME) as usize;
    let gliding = &velocities[..streaming];
    assert!(most_change(gliding, Vec3::distance) < 4.0);
    assert!(most_change(&velocities_of(gliding), Vec3::distance) < 60.0);
    assert!(most_change(&velocities, Vec3::distance) < 32.0);
    // Going back after streaming, it never goes faster than it streamed.
    assert!(
        velocities
            .iter()
            .all(|velocity| velocity.length() <= STREAM + 1e-2)
    );
    assert_eq!(path.last().unwrap().position, Vec3::new(1.0, 1.4, -2.0));
}

#[test]
fn it_turns_gently_by_the_shortest_arc_easing_in_and_out_without_overshooting() {
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let mut glide = standing(turned(home, 0.0));
    let to = turned(home, 1.5);
    let path = frames(&mut glide, to, None, false, 4.0);
    let turns = spins(&path, FRAME);
    // Easing in: barely turning at first, and never faster than its lead.
    assert!(turns[0].abs() < 0.1, "{}", turns[0]);
    assert!(turns.iter().all(|spin| *spin <= TURN_SPEED + 1e-3));
    assert!(turns.iter().any(|spin| *spin > TURN_SPEED * 0.75));
    // Its turn speeds up and slows without a step, never turning back, nor past the way it is to face.
    let mut turning = vec![0.0];
    turning.extend(&turns);
    turning.push(0.0);
    let difference = |a: f32, b: f32| (b - a).abs();
    assert!(most_change(&turning, difference) < 4.0);
    let pushes: Vec<f32> = turning
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) / FRAME)
        .collect();
    assert!(most_change(&pushes, difference) < 80.0);
    assert!(turns.iter().all(|spin| *spin >= -1e-4));
    assert!(path.iter().all(|frame| yaw(frame) <= 1.5 + 1e-4));
    // Most of the way round within a second and a half, and facing exactly that way within four.
    assert!((yaw(&path[(1.5 / FRAME) as usize]) - 1.5).abs() < 0.1);
    assert_eq!(path.last().unwrap().orientation, to.orientation);
    // From just short of facing behind on one side to just short of it on the other, it turns the 20° between,
    // through facing behind, never the 340° round the front.
    let mut glide = standing(turned(home, 170f32.to_radians()));
    let behind = turned(home, -170f32.to_radians());
    let path = frames(&mut glide, behind, None, false, 3.0);
    assert!(
        path.iter()
            .all(|frame| yaw(frame).abs() >= 170f32.to_radians() - 1e-3)
    );
    assert!(
        path.last()
            .unwrap()
            .orientation
            .dot(behind.orientation)
            .abs()
            > 1.0 - 1e-6
    );
    // A small turn, as it keeps facing someone who moves 5°, is eased and slight, never a twitch.
    let mut glide = standing(home);
    let slight = turned(home, 5f32.to_radians());
    let path = frames(&mut glide, slight, None, false, 2.0);
    let turns = spins(&path, FRAME);
    assert!(turns.iter().all(|spin| *spin < 0.2), "{turns:?}");
    assert!(yaw(&path[(0.25 / FRAME) as usize]).abs() > 0.01);
    assert!((yaw(&path[(0.25 / FRAME) as usize]) - 5f32.to_radians()).abs() > 0.02);
    assert_eq!(path.last().unwrap().orientation, slight.orientation);
}

#[test]
fn a_new_way_to_face_on_its_turn_carries_on_from_how_it_turns() {
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let mut glide = standing(home);
    let mut path = vec![home];
    // Halfway round one way, it is turned the other, then back again, and on.
    for (yaw, seconds) in [(2.0, 0.8), (-1.0, 0.6), (0.5, 0.3), (-2.5, 5.0)] {
        path.extend(frames(&mut glide, turned(home, yaw), None, false, seconds));
    }
    let mut turns = vec![0.0];
    turns.extend(spins(&path, FRAME));
    turns.push(0.0);
    let difference = |a: f32, b: f32| (b - a).abs();
    assert!(most_change(&turns, difference) < 8.0);
    let pushes: Vec<f32> = turns
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) / FRAME)
        .collect();
    assert!(most_change(&pushes, difference) < 150.0);
    assert!(turns.iter().all(|spin| spin.abs() <= TURN_SPEED + 1e-3));
    assert_eq!(
        path.last().unwrap().orientation,
        Quat::from_rotation_y(-2.5)
    );
}

#[test]
fn it_turns_to_where_it_goes_as_it_sets_off_and_to_face_them_as_it_arrives() {
    // Facing +Z, home's way, it goes 4 m along +X to stand facing −Z.
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let mut glide = standing(home);
    let there = turned(at(Vec3::new(4.0, 1.4, -2.3)), std::f32::consts::PI);
    let path = frames(&mut glide, there, None, false, 10.0);
    // On its way it faces where it goes, +X.
    let middle = path
        .iter()
        .find(|frame| frame.position.x > 2.0)
        .expect("halfway");
    assert!(
        (yaw(middle) - std::f32::consts::FRAC_PI_2).abs() < 0.15,
        "{}",
        yaw(middle)
    );
    // As it sets off, it turns that way after a moment, not at once.
    assert!(yaw(&path[(0.1 / FRAME) as usize]).abs() < 0.05);
    // It arrives facing the way it is to face, turned on round from where it went rather than back.
    assert_eq!(path.last().unwrap().position, there.position);
    assert!(
        path.last()
            .unwrap()
            .orientation
            .dot(there.orientation)
            .abs()
            > 1.0 - 1e-6
    );
    assert!(path.iter().all(|frame| {
        let yaw = yaw(frame).rem_euclid(std::f32::consts::TAU);
        yaw <= std::f32::consts::PI + 1e-3 || yaw >= std::f32::consts::TAU - 1e-3
    }));
    // Never faster than a gentle turn.
    assert!(
        spins(&path, FRAME)
            .iter()
            .all(|spin| spin.abs() <= TURN_SPEED + 1e-3)
    );
    // A short way, it sidesteps, facing as it is to face all the way.
    let mut glide = standing(home);
    let aside = at(Vec3::new(0.7, 1.4, -2.3));
    let path = frames(&mut glide, aside, None, false, 4.0);
    assert!(path.iter().all(|frame| frame.orientation == Quat::IDENTITY));
    assert_eq!(path.last().unwrap().position, aside.position);
    // Straight up, by a waypoint above it, it keeps facing as it faces.
    let mut glide = standing(home);
    let above = at(Vec3::new(0.0, 3.4, -2.3));
    let path = frames(&mut glide, above, None, false, 6.0);
    assert!(path.iter().all(|frame| frame.orientation == Quat::IDENTITY));
}

#[test]
fn it_moves_alike_at_any_frame_rate() {
    let home = at(Vec3::new(0.0, 1.4, -2.3));
    let there = turned(at(Vec3::new(3.0, 1.4, 0.5)), -2.0);
    let over = Vec3::new(1.5, 2.6, -1.0);
    let back = turned(at(Vec3::new(-1.0, 1.4, -3.0)), 1.0);
    let run = |hz: f32| {
        let dt = 1.0 / hz;
        let mut glide = standing(home);
        let mut path = frames_at(&mut glide, there, Some(over), false, 2.5, dt);
        path.extend(frames_at(&mut glide, back, None, false, 8.0, dt));
        // Sampled every 1/24 s, which both rates land on.
        let every = (hz / 24.0).round() as usize;
        path.into_iter()
            .skip(every - 1)
            .step_by(every)
            .collect::<Vec<_>>()
    };
    let (slow, fast) = (run(72.0), run(120.0));
    assert_eq!(slow.len(), fast.len());
    for (slow, fast) in slow.iter().zip(&fast) {
        assert!(
            slow.position.distance(fast.position) < 2e-3,
            "{slow:?} {fast:?}"
        );
        assert!(slow.orientation.angle_between(fast.orientation) < 0.01);
    }
    // Both arrive where the server stands it, facing as it faces.
    for path in [&slow, &fast] {
        assert_eq!(path.last().unwrap().position, back.position);
        assert_eq!(path.last().unwrap().orientation, back.orientation);
    }
}
