// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::{
    traces::{Noise, Run, angle, arrivals, deviation, orientation, pose, report, tilt},
    *,
};
use std::f32::consts::FRAC_PI_2;

const SOURCE: u32 = 4;
const FPS: f64 = 60.0;

fn at(x: f32) -> Vec3 {
    Vec3::new(x, 1.6, 0.0)
}

/// A head walking along X at one metre a second, facing ahead.
fn walking(second: f64) -> Placed {
    Placed {
        position: at(second as f32),
        orientation: Quat::IDENTITY,
    }
}

/// A head at `(x, 1.6, 0)` facing `yaw`.
fn facing(x: f32, yaw: f32) -> Placed {
    Placed {
        position: at(x),
        orientation: orientation(yaw, 0.0, 0.0),
    }
}

fn xs(frames: &[(f64, View)]) -> Vec<f32> {
    frames
        .iter()
        .map(|(_, view)| view.head.position.x)
        .collect()
}

/// How far the head moves from one frame to the next.
fn steps(frames: &[(f64, View)]) -> Vec<f32> {
    xs(frames)
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect()
}

/// A run of `mode` in which `source` walks along X for `until` seconds, its reports arriving as `latency` and `jitter` say, drawn from the second up to `until`.
fn walk(mode: Smoothing, (latency, jitter): (f64, f64), until: f64) -> Vec<(f64, View)> {
    let mut run = Run::new(mode);
    run.queue(arrivals(walking, (0.0, until), (latency, jitter), 11));
    run.play(SOURCE, (1.0, until), FPS)
}

#[test]
fn the_levels_are_named_cycled_and_light_by_default() {
    assert_eq!(Smoothing::default(), Smoothing::Light);
    assert_eq!(
        Smoothing::EVERY.map(Smoothing::name),
        ["off", "light", "cinematic"]
    );
    for level in Smoothing::EVERY {
        assert_eq!(Smoothing::named(level.name()), Some(level));
        assert_eq!(level.next().next().next(), level);
    }
    assert_eq!(Smoothing::named("heavy"), None);
    assert_eq!(Smoothing::Off.next(), Smoothing::Light);
    assert_eq!(Smoothing::Light.next(), Smoothing::Cinematic);
    assert_eq!(Smoothing::Cinematic.next(), Smoothing::Off);
}

#[test]
fn each_level_is_stronger_than_the_one_before() {
    assert!(Smoothing::Off.tuning().is_none());
    let (light, cinematic) = (
        Smoothing::Light.tuning().unwrap(),
        Smoothing::Cinematic.tuning().unwrap(),
    );
    assert!(cinematic.delay_intervals > light.delay_intervals);
    assert!(cinematic.min_delay > light.min_delay && cinematic.max_delay > light.max_delay);
    assert!(cinematic.catch_up > light.catch_up);
    assert!(cinematic.position.min_cutoff < light.position.min_cutoff);
    assert!(cinematic.orientation.min_cutoff < light.orientation.min_cutoff);
    assert!(cinematic.position.beta < light.position.beta);
    assert!(cinematic.roll < light.roll);
    // Roll is damped more than yaw and pitch, and the delay bounded and above a frame, at both.
    for tuning in [light, cinematic] {
        assert!(tuning.roll < 1.0);
        assert!(
            0.02 < tuning.min_delay
                && tuning.min_delay <= tuning.max_delay
                && tuning.max_delay <= 0.25
        );
        assert!(0.0 < tuning.min_rate && tuning.min_rate < 1.0 && 1.0 < tuning.max_rate);
    }
}

#[test]
fn interpolation_makes_the_motion_continuous_at_the_draw_rate() {
    let ideal = 1.0 / FPS as f32;
    let off = steps(&walk(Smoothing::Off, (0.02, 0.0), 6.0));
    // Off takes each report as it comes: the view stands still for two frames in three and then steps.
    assert!(off.iter().filter(|step| **step == 0.0).count() > off.len() / 2);
    assert!(off.iter().any(|step| *step > 2.0 * ideal));
    for level in [Smoothing::Light, Smoothing::Cinematic] {
        let frames = walk(level, (0.02, 0.0), 6.0);
        let moved = steps(&frames[120..]);
        // Moving every frame, by about as much each time, at the speed of the head.
        let (least, most) = moved
            .iter()
            .fold((f32::MAX, f32::MIN), |(least, most), step| {
                (least.min(*step), most.max(*step))
            });
        assert!(
            least > 0.5 * ideal && most < 1.5 * ideal,
            "{level:?}: {least} {most}"
        );
        let mean = moved.iter().sum::<f32>() / moved.len() as f32;
        assert!((mean - ideal).abs() < 0.05 * ideal, "{level:?}: {mean}");
    }
}

#[test]
fn the_view_trails_the_head_by_a_bounded_delay() {
    // Reports arrive 20 ms after they are sent; the head is at one metre a second, so metres are seconds.
    for (level, bound) in [(Smoothing::Light, 0.25), (Smoothing::Cinematic, 0.45)] {
        let frames = walk(level, (0.02, 0.0), 6.0);
        let (second, view) = frames.last().unwrap();
        let lag = *second as f32 - view.head.position.x;
        assert!(lag > 0.05 && lag < bound, "{level:?}: {lag}");
    }
    // The cinematic level trails more, as it filters more.
    let lag = |level| {
        let frames = walk(level, (0.02, 0.0), 6.0);
        let (second, view) = frames.last().unwrap();
        *second as f32 - view.head.position.x
    };
    assert!(lag(Smoothing::Cinematic) > lag(Smoothing::Light));
}

#[test]
fn reports_arriving_unevenly_are_played_evenly() {
    // 20 ms either way of a report every 50 ms, in the order they were sent.
    let ideal = 1.0 / FPS as f32;
    let spread = |level| {
        let frames = walk(level, (0.03, 0.02), 10.0);
        let moved = steps(&frames[120..]);
        (
            deviation(&moved),
            moved.iter().fold(0.0f32, |most, step| most.max(step.abs())),
        )
    };
    let (off, off_most) = spread(Smoothing::Off);
    for level in [Smoothing::Light, Smoothing::Cinematic] {
        let (deviation, most) = spread(level);
        assert!(deviation < 0.25 * off, "{level:?}: {deviation} {off}");
        assert!(
            most < 1.6 * ideal && off_most > 2.5 * ideal,
            "{level:?}: {most} {off_most}"
        );
    }
}

#[test]
fn a_gap_holds_the_view_and_is_caught_up_at_a_bounded_rate() {
    // Walking, then nothing for 0.6 s, then reports resume with the head 0.6 m on, and walking.
    let mut run = Run::new(Smoothing::Light);
    run.queue(arrivals(walking, (0.0, 2.0), (0.02, 0.0), 1));
    run.queue(arrivals(walking, (2.6, 6.0), (0.02, 0.0), 2));
    let frames = run.play(SOURCE, (1.0, 6.0), FPS);
    let held: Vec<f32> = frames
        .iter()
        .filter(|(second, _)| (2.35..2.6).contains(second))
        .map(|(_, view)| view.head.position.x)
        .collect();
    // Held: the last report, and no more, whatever the filter does about it.
    let last = 1.95_f32;
    assert!(held.iter().all(|x| *x <= last + 1e-3), "{held:?}");
    assert!(held.windows(2).all(|pair| (pair[1] - pair[0]).abs() < 1e-3));
    // Caught up at no more than the level's rate times the speed of the head, with no jump.
    let max_rate = LIGHT.max_rate as f32;
    let moved = steps(&frames);
    let most = moved.iter().fold(0.0f32, |most, step| most.max(step.abs()));
    assert!(most <= 1.2 * max_rate / FPS as f32, "{most}");
    // And it does catch up, to the usual delay behind the head.
    let (second, view) = frames.last().unwrap();
    let lag = *second as f32 - view.head.position.x;
    assert!(lag > 0.05 && lag < 0.25, "{lag}");
}

#[test]
fn a_gap_too_long_to_be_a_delay_is_a_cut_and_the_view_goes_to_where_the_head_is() {
    let mut run = Run::new(Smoothing::Light);
    run.queue(arrivals(walking, (0.0, 2.0), (0.02, 0.0), 1));
    // A silence of three seconds, in which the head has moved 2 m... less than a teleport, but too late to be walked.
    run.queue(arrivals(
        |second| walking(second - 1.0),
        (5.0, 8.0),
        (0.02, 0.0),
        2,
    ));
    let frames = run.play(SOURCE, (1.0, 8.0), FPS);
    // 5 s onwards the head is at (second - 1) m, resuming at 4 m, from 1.95 m: no frame is between, other than its filter's.
    let ahead = frames
        .iter()
        .position(|(_, view)| view.head.position.x > 3.0)
        .unwrap();
    let step = frames[ahead].1.head.position.x - frames[ahead - 1].1.head.position.x;
    assert!(step > 1.0, "{step}");
    assert!(frames[ahead].0 < 5.5, "{}", frames[ahead].0);
}

#[test]
fn a_teleport_snaps_and_never_glides_through_what_is_between() {
    let mut run = Run::new(Smoothing::Light);
    // Walking to 3 m, then, as a locomotion teleport does, 6 m further in one report.
    let teleporting = |second: f64| walking(if second < 3.0 { second } else { second + 6.0 });
    run.queue(arrivals(teleporting, (0.0, 6.0), (0.02, 0.0), 5));
    let frames = run.play(SOURCE, (1.0, 6.0), FPS);
    let x = xs(&frames);
    // Everything drawn is before the jump, or after it.
    assert!(
        x.iter().all(|x| *x < 3.1 || *x > 8.9),
        "{:?}",
        x.iter().find(|x| **x >= 3.1 && **x <= 8.9)
    );
    // The frame that jumps is the report itself, not the filter's catching up with it.
    let jump = x.windows(2).position(|pair| pair[1] > 8.0).unwrap();
    assert!((x[jump + 1] - 9.0).abs() < 0.06, "{}", x[jump + 1]);
    // And after, the view walks on smoothly.
    assert!(x[jump + 3..].windows(2).all(|pair| pair[1] > pair[0]));
}

#[test]
fn a_turn_is_not_a_teleport() {
    // A turn through half a revolution at the same place, in one report, is followed, not cut.
    let mut run = Run::new(Smoothing::Light);
    for report_at in 0..40 {
        let yaw = if report_at < 20 { 0.0 } else { 3.0 };
        run.arrive(
            SOURCE,
            report_at as f64 * 0.05,
            report(at(0.0), orientation(yaw, 0.0, 0.0)),
        );
    }
    assert_eq!(run.smoother.tracks[&SOURCE].cuts(), 0);
}

#[test]
fn following_someone_else_starts_at_where_they_are() {
    let mut run = Run::new(Smoothing::Cinematic);
    run.queue(arrivals(walking, (0.0, 4.0), (0.02, 0.0), 1));
    let before = run.play(SOURCE, (1.0, 4.0), FPS);
    // Another participant, 10 m away and facing the other way, reports for a moment.
    for count in 0..10 {
        run.arrive(
            9,
            4.0 + count as f64 * 0.05,
            report(at(10.0), orientation(2.0, 0.0, 0.0)),
        );
    }
    let view = run.smoother.view(Some(9), run.at(4.5)).unwrap();
    assert_eq!(view.head.position, at(10.0));
    assert!(angle(view.head.orientation, orientation(2.0, 0.0, 0.0)) < 1e-4);
    // And back again to the first, from where they are now, not from where the view left them.
    run.arrive(SOURCE, 4.6, report(at(4.6), Quat::IDENTITY));
    let view = run.smoother.view(Some(SOURCE), run.at(4.6)).unwrap();
    assert!(
        view.head.position.distance(at(4.6)) < 0.5,
        "{}",
        view.head.position
    );
    assert!(before.last().unwrap().1.head.position.x < view.head.position.x);
    // With nobody followed there is no view, and no one is the one last seen.
    assert!(run.smoother.view(None, run.at(5.0)).is_none());
    assert_eq!(run.smoother.viewing, None);
    // A participant never seen has none.
    assert!(run.smoother.view(Some(77), run.at(5.0)).is_none());
}

#[test]
fn a_lost_connection_cuts_to_where_the_participants_are_after_it() {
    let after_losing = |lose: bool| {
        let mut run = Run::new(Smoothing::Light);
        run.queue(arrivals(walking, (0.0, 2.0), (0.02, 0.0), 1));
        run.play(SOURCE, (1.0, 2.0), FPS);
        if lose {
            run.smoother.interrupt();
        }
        // They are 0.4 m on after 0.5 s: walked, if the connection had held.
        run.queue(arrivals(
            |second| walking(second - 0.1),
            (2.5, 4.0),
            (0.02, 0.0),
            3,
        ));
        let frames = run.play(SOURCE, (2.0, 3.0), FPS);
        let most = steps(&frames)
            .iter()
            .fold(0.0f32, |most, step| most.max(step.abs()));
        (most, run.smoother.tracks[&SOURCE].cuts())
    };
    let (kept, cuts) = after_losing(false);
    assert_eq!(cuts, 0);
    assert!(kept < 2.0 / FPS as f32);
    let (lost, cuts) = after_losing(true);
    assert_eq!(cuts, 1);
    assert!(lost > 0.3, "{lost}");
}

#[test]
fn off_is_the_latest_report_exactly() {
    let mut run = Run::new(Smoothing::Off);
    assert_eq!(run.smoother.mode(), Smoothing::Off);
    // Not even a unit orientation, as it came from the network, nor a position that a teleport would cut.
    let left = Pose {
        position: [0.1, 0.2, 0.3],
        orientation: [0.0, 0.5, 0.0, 0.5],
    };
    let hands = Some([None, None]);
    let latest = Presence {
        head: Pose {
            position: [1.0, 2.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 2.0],
        },
        left: Some(left),
        right: None,
        hands,
        hidden: false,
    };
    run.arrive(SOURCE, 0.0, report(at(0.0), Quat::IDENTITY));
    run.arrive(SOURCE, 0.05, report(at(50.0), Quat::IDENTITY));
    run.arrive(SOURCE, 0.1, latest);
    for second in [0.1, 0.11, 0.2, 5.0] {
        let view = run.smoother.view(Some(SOURCE), run.at(second)).unwrap();
        assert_eq!(view.head.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            view.head.orientation,
            Quat::from_array([0.0, 0.0, 0.0, 2.0])
        );
        assert_eq!(
            (view.left, view.right, view.hands),
            (Some(left), None, hands)
        );
    }
    assert_eq!(run.smoother.latest(SOURCE), Some(&latest));
    // Turned on, it starts from the latest report, and turned off again it is the latest report.
    run.smoother.set_mode(Smoothing::Light);
    let view = run.smoother.view(Some(SOURCE), run.at(5.1)).unwrap();
    assert!(view.head.position.distance(Vec3::new(1.0, 2.0, 3.0)) < 60.0);
    assert!((view.head.orientation.length() - 1.0).abs() < 1e-5);
    run.smoother.set_mode(Smoothing::Off);
    let view = run.smoother.view(Some(SOURCE), run.at(5.2)).unwrap();
    assert_eq!(
        view.head.orientation,
        Quat::from_array([0.0, 0.0, 0.0, 2.0])
    );
}

#[test]
fn reports_that_cannot_be_drawn_are_not_played_and_the_last_that_can_be_stays() {
    let mut run = Run::new(Smoothing::Light);
    let spoilt = |position: [f32; 3], orientation: [f32; 4]| Presence {
        head: Pose {
            position,
            orientation,
        },
        ..report(at(0.0), Quat::IDENTITY)
    };
    // Nothing usable yet: nothing to play, though the report is the latest, with whether they asked not to be drawn.
    run.arrive(
        SOURCE,
        0.0,
        spoilt([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
    );
    assert!(run.smoother.view(Some(SOURCE), run.at(0.1)).is_none());
    assert!(run.smoother.latest(SOURCE).is_some());
    run.arrive(SOURCE, 0.1, spoilt([0.0, 1.6, 0.0], [0.0, 0.0, 0.0, 0.0]));
    run.arrive(
        SOURCE,
        0.15,
        spoilt([0.0, 1.6, f32::INFINITY], [0.0, 0.0, 0.0, 1.0]),
    );
    assert!(run.smoother.view(Some(SOURCE), run.at(0.2)).is_none());
    // Then a good one, and bad ones among the good: the view holds and stays finite.
    run.arrive(SOURCE, 0.2, report(at(1.0), Quat::IDENTITY));
    run.arrive(SOURCE, 0.25, spoilt([0.0, 1.6, 0.0], [f32::NAN; 4]));
    for frame in 0..30 {
        let view = run
            .smoother
            .view(Some(SOURCE), run.at(0.3 + frame as f64 / FPS))
            .unwrap();
        assert_eq!(view.head.position, at(1.0));
    }
    // A controller that cannot be drawn is not drawn.
    run.arrive(
        SOURCE,
        0.3,
        Presence {
            left: Some(pose(Vec3::NAN, Quat::IDENTITY)),
            right: Some(pose(Vec3::ZERO, Quat::IDENTITY)),
            ..report(at(1.0), Quat::IDENTITY)
        },
    );
    let view = run.smoother.view(Some(SOURCE), run.at(1.0)).unwrap();
    assert_eq!(
        (view.left, view.right),
        (None, Some(pose(Vec3::ZERO, Quat::IDENTITY)))
    );
}

#[test]
fn controllers_are_played_with_the_head_and_hands_are_held() {
    let hand = Hand::default();
    let held = |hands| Presence {
        left: Some(pose(Vec3::new(0.0, 1.0, -0.5), Quat::IDENTITY)),
        right: Some(pose(Vec3::new(0.0, 1.0, 0.5), orientation(0.0, 0.0, 0.0))),
        hands,
        ..report(at(0.0), Quat::IDENTITY)
    };
    let mut run = Run::new(Smoothing::Light);
    run.arrive(SOURCE, 0.0, held(None));
    let moved = Presence {
        left: Some(pose(Vec3::new(0.0, 1.0, -1.5), Quat::IDENTITY)),
        right: Some(pose(Vec3::new(0.0, 1.0, 0.5), orientation(0.0, 0.0, 0.5))),
        hands: Some([Some(hand), None]),
        ..report(at(0.0), Quat::IDENTITY)
    };
    run.arrive(SOURCE, 0.05, moved);
    run.arrive(SOURCE, 0.1, moved);
    let mut seen = Vec::new();
    for frame in 0..30 {
        let second = frame as f64 / FPS;
        seen.push(run.smoother.view(Some(SOURCE), run.at(second)).unwrap());
    }
    // The left moves from one report to the next, and between, in a straight line.
    let lefts: Vec<f32> = seen
        .iter()
        .map(|view| view.left.unwrap().position[2])
        .collect();
    assert!(lefts.windows(2).all(|pair| pair[1] <= pair[0]) && lefts[29] == -1.5);
    assert!(lefts.iter().any(|z| *z < -0.6 && *z > -1.4));
    // The right turns, the short way, and stays a unit.
    let turned = seen
        .iter()
        .map(|view| Quat::from_array(view.right.unwrap().orientation))
        .collect::<Vec<_>>();
    assert!(turned.iter().all(|q| (q.length() - 1.0).abs() < 1e-5));
    assert!(angle(turned[29], orientation(0.0, 0.0, 0.5)) < 1e-4);
    // Hands are those of the report the view has reached.
    assert_eq!(seen[0].hands, None);
    assert_eq!(seen[29].hands, Some([Some(hand), None]));
}

#[test]
fn an_orientation_is_played_along_the_shortest_arc() {
    // 170 degrees to −170, which is 20 degrees, and the third report with the sign of its quaternion flipped.
    let mut run = Run::new(Smoothing::Light);
    let (from, to) = (170.0_f32.to_radians(), (-170.0_f32).to_radians());
    run.arrive(SOURCE, 0.0, report(at(0.0), orientation(from, 0.0, 0.0)));
    run.arrive(SOURCE, 0.05, report(at(0.0), orientation(to, 0.0, 0.0)));
    run.arrive(SOURCE, 0.1, report(at(0.0), -orientation(to, 0.0, 0.0)));
    let mut previous: Option<Quat> = None;
    for frame in 0..40 {
        let view = run
            .smoother
            .view(Some(SOURCE), run.at(frame as f64 / FPS))
            .unwrap();
        assert!((view.head.orientation.length() - 1.0).abs() < 1e-5);
        // Never further than the 20 degrees from the start, nor from the end: the short way.
        assert!(
            angle(view.head.orientation, orientation(from, 0.0, 0.0))
                <= 20.0_f32.to_radians() + 1e-3
        );
        if let Some(previous) = previous {
            assert!(angle(previous, view.head.orientation) < 0.1);
        }
        previous = Some(view.head.orientation);
    }
}

#[test]
fn a_still_head_s_tremor_is_reduced_end_to_end_by_a_stated_factor() {
    // A still head, with tremor of up to 3 mm and 0.004 rad (0.006 in roll), reported with jitter and drawn at 60 frames a second.
    let still = |level: Smoothing| {
        let mut noise = Noise::new(21);
        let mut run = Run::new(level);
        run.queue(arrivals(
            |_| Placed {
                position: at(0.0) + Vec3::new(noise.next(), noise.next(), noise.next()) * 0.003,
                orientation: orientation(
                    noise.next() * 0.004,
                    noise.next() * 0.004,
                    noise.next() * 0.006,
                ),
            },
            (0.0, 20.0),
            (0.03, 0.01),
            9,
        ));
        let frames = run.play(SOURCE, (2.0, 20.0), FPS);
        let x = xs(&frames);
        let rotation = |axis: usize| {
            deviation(
                &frames
                    .iter()
                    .map(|(_, view)| filter::rotation_vector(view.head.orientation)[axis])
                    .collect::<Vec<_>>(),
            )
        };
        [deviation(&x), rotation(0), rotation(1), rotation(2)]
    };
    let off = still(Smoothing::Off);
    for (level, position, rotation, roll) in [
        (Smoothing::Light, 4.0, 6.0, 15.0),
        (Smoothing::Cinematic, 9.0, 14.0, 80.0),
    ] {
        let smoothed = still(level);
        let factor = |axis: usize| (off[axis] / smoothed[axis]).powi(2);
        assert!(factor(0) >= position, "{level:?} position {}", factor(0));
        assert!(
            factor(1) >= rotation && factor(2) >= rotation,
            "{level:?} {} {}",
            factor(1),
            factor(2)
        );
        assert!(factor(3) >= roll, "{level:?} roll {}", factor(3));
    }
}

#[test]
fn a_deliberate_turn_is_followed_within_a_bounded_delay() {
    // Still, a quarter turn in half a second, and still, reported every 50 ms.
    let turning = |second: f64| {
        facing(
            0.0,
            FRAC_PI_2 * ((second - 2.0) / 0.5).clamp(0.0, 1.0) as f32,
        )
    };
    for (level, bound) in [(Smoothing::Light, 0.35), (Smoothing::Cinematic, 0.7)] {
        let mut run = Run::new(level);
        run.queue(arrivals(turning, (0.0, 6.0), (0.03, 0.01), 3));
        let frames = run.play(SOURCE, (1.0, 6.0), FPS);
        // The turn ends at 2.5 s; nine tenths of it is drawn by then plus the delay, and all of it within a degree some time after.
        let reached = |fraction: f32| {
            frames
                .iter()
                .find(|(_, view)| {
                    angle(Quat::IDENTITY, view.head.orientation) >= fraction * FRAC_PI_2
                })
                .map(|(second, _)| *second)
                .unwrap()
        };
        let delay = reached(0.9) - 2.45;
        assert!(delay > 0.0 && delay <= bound, "{level:?}: {delay}");
        let last = frames.last().unwrap().1.head.orientation;
        assert!(angle(last, orientation(FRAC_PI_2, 0.0, 0.0)) < 1.0_f32.to_radians());
    }
}

#[test]
fn a_turn_about_the_vertical_keeps_the_horizon_level_and_is_followed_in_time_at_any_pitch() {
    // Still, a quarter turn about the world's vertical in half a second, and still, at each pitch, as a viewer looking down at a wall of charts, or up at a ceiling, turns.
    for (level, bound) in [(Smoothing::Light, 0.35), (Smoothing::Cinematic, 0.7)] {
        for degrees in [0.0_f32, 30.0, 45.0, 70.0, -45.0] {
            let pitch = degrees.to_radians();
            let turning = |second: f64| Placed {
                position: at(0.0),
                orientation: orientation(
                    FRAC_PI_2 * ((second - 2.0) / 0.5).clamp(0.0, 1.0) as f32,
                    pitch,
                    0.0,
                ),
            };
            let mut run = Run::new(level);
            run.queue(arrivals(turning, (0.0, 6.0), (0.03, 0.01), 3));
            let frames = run.play(SOURCE, (1.0, 6.0), FPS);
            // The horizon stays within a degree, nothing of the turn being taken for a tilt.
            let worst = frames
                .iter()
                .map(|(_, view)| tilt(view.head.orientation).abs())
                .fold(0.0, f32::max);
            assert!(worst <= 1.0, "{level:?} at {degrees}: {worst}");
            // Nine tenths of the turn is drawn within the bound of the head's reaching it, whatever the pitch.
            let start = orientation(0.0, pitch, 0.0);
            let reached = frames
                .iter()
                .find(|(_, view)| angle(start, view.head.orientation) >= 0.9 * FRAC_PI_2)
                .map(|(second, _)| *second)
                .unwrap();
            let delay = reached - 2.45;
            assert!(
                delay > 0.0 && delay <= bound,
                "{level:?} at {degrees}: {delay}"
            );
        }
    }
}

#[test]
fn a_deliberate_tilt_of_the_head_still_arrives_damped() {
    // Still, then tilted by 20 degrees about the view axis in half a second while looking 30 degrees down: within a tenth of it some seconds on.
    for level in [Smoothing::Light, Smoothing::Cinematic] {
        let pitch = -30.0_f32.to_radians();
        let tilting = |second: f64| Placed {
            position: at(0.0),
            orientation: orientation(
                0.0,
                pitch,
                0.35 * ((second - 2.0) / 0.5).clamp(0.0, 1.0) as f32,
            ),
        };
        let mut run = Run::new(level);
        run.queue(arrivals(tilting, (0.0, 8.0), (0.03, 0.01), 3));
        let frames = run.play(SOURCE, (1.0, 8.0), FPS);
        let end = tilt(tilting(8.0).orientation);
        let (_, last) = frames.last().unwrap();
        assert!(
            (tilt(last.head.orientation) - end).abs() < 0.1 * end.abs(),
            "{level:?}"
        );
        // It lags the head's tilt by more than a turn would: at 2.6 s, a tenth of a second after it ends, short of nine tenths.
        let soon = frames.iter().find(|(second, _)| *second >= 2.6).unwrap();
        assert!(tilt(soon.1.head.orientation) < 0.9 * end, "{level:?}");
    }
}

#[test]
fn nothing_reported_while_hidden_is_drawn_once_they_let_others_see_them() {
    let hidden = |position: Vec3| Presence {
        hidden: true,
        ..report(position, Quat::IDENTITY)
    };
    let mut run = Run::new(Smoothing::Light);
    // Seen, then asking not to be drawn and moving to 0.8 m a report, then seen again 5 m back where they began.
    for count in 0..10 {
        run.arrive(SOURCE, count as f64 * 0.05, report(at(0.0), Quat::IDENTITY));
    }
    for count in 10..40 {
        let second = count as f64 * 0.05;
        run.arrive(SOURCE, second, hidden(at(count as f32 * 0.8)));
    }
    assert!(run.smoother.latest(SOURCE).unwrap().hidden);
    // Nothing of it is kept to be drawn.
    assert!(run.smoother.view(Some(SOURCE), run.at(2.0)).is_none());
    run.arrive(SOURCE, 2.0, report(at(-5.0), Quat::IDENTITY));
    run.arrive(SOURCE, 2.05, report(at(-5.0), Quat::IDENTITY));
    for frame in 0..30 {
        let second = 2.05 + frame as f64 / FPS;
        let view = run.smoother.view(Some(SOURCE), run.at(second)).unwrap();
        // Where they are, from the first frame, and never where they were while hidden.
        assert_eq!(view.head.position, at(-5.0), "{frame}");
    }
    assert!(!run.smoother.latest(SOURCE).unwrap().hidden);
}

#[test]
fn participants_who_have_left_are_forgotten_after_a_while_and_those_just_heard_from_are_not() {
    let mut run = Run::new(Smoothing::Light);
    for source in [1, 2, 3] {
        run.arrive(source, 0.0, report(at(0.0), Quat::IDENTITY));
    }
    // 4 reports only just before.
    run.arrive(4, 9.0, report(at(0.0), Quat::IDENTITY));
    let present = |source: u32| source == 1;
    // Not yet missed for long.
    run.smoother.forget_absent(present, run.at(4.0));
    assert_eq!(run.smoother.reports().count(), 4);
    // Later, only those named, and those heard from within the time, are kept.
    run.smoother.forget_absent(present, run.at(10.0));
    assert!(run.smoother.latest(1).is_some() && run.smoother.latest(4).is_some());
    assert!(run.smoother.latest(2).is_none() && run.smoother.latest(3).is_none());
    run.smoother.forget_absent(present, run.at(20.0));
    assert!(run.smoother.latest(4).is_none() && run.smoother.latest(1).is_some());
    // A name nobody has reported is no one.
    run.smoother
        .forget_absent(|source| source == 7, run.at(30.0));
    assert_eq!(run.smoother.reports().count(), 0);
}

#[test]
fn the_presence_of_each_participant_is_kept_whether_or_not_it_is_followed() {
    let mut run = Run::new(Smoothing::Light);
    assert!(run.smoother.latest(SOURCE).is_none());
    run.arrive(SOURCE, 0.0, report(at(1.0), Quat::IDENTITY));
    run.arrive(
        8,
        0.0,
        Presence {
            hidden: true,
            ..report(at(2.0), Quat::IDENTITY)
        },
    );
    assert_eq!(
        run.smoother.latest(SOURCE).unwrap().head.position,
        at(1.0).to_array()
    );
    assert!(run.smoother.latest(8).unwrap().hidden);
    assert_eq!(run.smoother.reports().count(), 2);
}
