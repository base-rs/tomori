// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::smoothing::{
    CINEMATIC, LIGHT,
    traces::{Noise, angle, deviation, orientation},
};
use std::f32::consts::{FRAC_PI_2, PI};

const FRAME: f32 = 1.0 / 60.0;

fn head(position: Vec3, orientation: Quat) -> Placed {
    Placed {
        position,
        orientation,
    }
}

/// The filter's output, once a trace of `frames` heads has been put through it at 60 frames a second.
fn filtered(tuning: &Tuning, trace: &[Placed]) -> Vec<Placed> {
    let mut filter = Filter::new(trace[0], 0);
    trace[1..]
        .iter()
        .map(|head| filter.step(*head, FRAME, tuning))
        .collect()
}

/// A still head with tremor, as a hand-held device reports it: up to 3 mm in every direction, and 0.004 rad in yaw and pitch and 0.006 in roll.
fn tremor(frames: usize, seed: u64) -> Vec<Placed> {
    let mut noise = Noise::new(seed);
    (0..frames)
        .map(|_| {
            let position = Vec3::new(noise.next(), noise.next(), noise.next()) * 0.003;
            head(
                Vec3::new(0.0, 1.6, 0.0) + position,
                orientation(
                    noise.next() * 0.004,
                    noise.next() * 0.004,
                    noise.next() * 0.006,
                ),
            )
        })
        .collect()
}

/// The standard deviation of the position along X, and of the rotation vector's three axes, of a trace after its first second.
fn spread(trace: &[Placed]) -> [f32; 4] {
    let steady = &trace[60..];
    let rotation = |axis: usize| {
        deviation(
            &steady
                .iter()
                .map(|head| rotation_vector(head.orientation)[axis])
                .collect::<Vec<_>>(),
        )
    };
    [
        deviation(
            &steady
                .iter()
                .map(|head| head.position.x)
                .collect::<Vec<_>>(),
        ),
        rotation(0),
        rotation(1),
        rotation(2),
    ]
}

#[test]
fn a_still_heads_tremor_is_reduced_by_a_stated_factor_at_each_level() {
    let trace = tremor(1200, 7);
    let raw = spread(&trace);
    // The variance of the position, yaw and pitch falls by at least these factors, and the roll's by more.
    for (tuning, position, rotation, roll) in
        [(&LIGHT, 5.0, 8.0, 20.0), (&CINEMATIC, 15.0, 20.0, 100.0)]
    {
        let smoothed = spread(&filtered(tuning, &trace));
        let factor = |axis: usize| (raw[axis] / smoothed[axis]).powi(2);
        assert!(factor(0) >= position, "position {} < {position}", factor(0));
        assert!(factor(1) >= rotation, "pitch {} < {rotation}", factor(1));
        assert!(factor(2) >= rotation, "yaw {} < {rotation}", factor(2));
        assert!(factor(3) >= roll, "roll {} < {roll}", factor(3));
    }
}

/// How many seconds after a head moves or turns by `step`, instantly, the filter has covered nine tenths of it.
fn rise(tuning: &Tuning, step: Placed) -> f32 {
    let mut filter = Filter::new(head(Vec3::ZERO, Quat::IDENTITY), 0);
    for frame in 1..=600 {
        let output = filter.step(step, FRAME, tuning);
        let covered = if step.position != Vec3::ZERO {
            output.position.length() / step.position.length()
        } else {
            angle(Quat::IDENTITY, output.orientation) / angle(Quat::IDENTITY, step.orientation)
        };
        if covered >= 0.9 {
            return frame as f32 * FRAME;
        }
    }
    f32::INFINITY
}

#[test]
fn a_turn_or_a_step_is_followed_within_a_bounded_lag_at_each_level() {
    let turn = head(Vec3::ZERO, orientation(FRAC_PI_2 / 2.0, 0.0, 0.0));
    let step = head(Vec3::new(0.3, 0.0, 0.0), Quat::IDENTITY);
    let light = (rise(&LIGHT, turn), rise(&LIGHT, step));
    let cinematic = (rise(&CINEMATIC, turn), rise(&CINEMATIC, step));
    // A quarter turn of a head is nine tenths followed within a fifth of a second in the light level and within half a second in the cinematic, and a lean of 30 cm within as long.
    assert!(light.0 <= 0.2 && light.1 <= 0.3, "{light:?}");
    assert!(cinematic.0 <= 0.5 && cinematic.1 <= 0.75, "{cinematic:?}");
    // The cinematic level trails more.
    assert!(cinematic.0 > light.0 && cinematic.1 > light.1);
}

#[test]
fn a_steady_turn_trails_by_a_bounded_time() {
    // 90 degrees a second, for two seconds, from rest.
    let rate = FRAC_PI_2;
    let trace: Vec<Placed> = (0..=240)
        .map(|frame| {
            head(
                Vec3::ZERO,
                orientation(rate * frame as f32 * FRAME, 0.0, 0.0),
            )
        })
        .collect();
    for (tuning, bound) in [(&LIGHT, 0.08), (&CINEMATIC, 0.25)] {
        let output = filtered(tuning, &trace);
        let (last, filtered) = (trace[240], output[239]);
        let lag = angle(last.orientation, filtered.orientation) / rate;
        assert!(lag > 0.0 && lag <= bound, "{lag} > {bound}");
    }
}

#[test]
fn a_faster_turn_opens_the_filter_so_it_trails_less() {
    let lag = |rate: f32| {
        let trace: Vec<Placed> = (0..=120)
            .map(|frame| {
                head(
                    Vec3::ZERO,
                    orientation(rate * frame as f32 * FRAME, 0.0, 0.0),
                )
            })
            .collect();
        let output = filtered(&LIGHT, &trace);
        angle(trace[120].orientation, output[119].orientation) / rate
    };
    // The cutoff rises with speed, so a quick glance is followed in less time than a slow turn.
    assert!(
        lag(4.0 * PI) < lag(PI / 4.0) * 0.6,
        "{} {}",
        lag(4.0 * PI),
        lag(PI / 4.0)
    );
}

#[test]
fn rotations_stay_unit_and_take_the_shortest_path() {
    // A turn through half a revolution, whose quaternion changes sign half way, as one from a headset may at any time.
    let trace: Vec<Placed> = (0..300)
        .map(|frame| {
            let q = orientation(PI * 1.5 * frame as f32 / 300.0, 0.3, 0.0);
            head(Vec3::ZERO, if frame % 7 < 3 { -q } else { q })
        })
        .collect();
    for tuning in [&LIGHT, &CINEMATIC] {
        let output = filtered(tuning, &trace);
        let mut previous = trace[0].orientation;
        for (frame, head) in output.iter().enumerate() {
            assert!((head.orientation.length() - 1.0).abs() < 1e-5, "{frame}");
            // Never the long way round: a frame moves by about what the input moves, 1.5 turns a second at most.
            let moved = angle(previous, head.orientation);
            assert!(moved < 0.05, "{frame}: {moved}");
            previous = head.orientation;
        }
        // And arrives: within a quarter second of the end of the turn.
        let end = trace[299].orientation;
        assert!(
            angle(end, output[298].orientation) < 0.6,
            "{}",
            angle(end, output[298].orientation)
        );
    }
}

#[test]
fn input_that_is_not_quite_unit_is_renormalised() {
    let mut filter = Filter::new(head(Vec3::ZERO, Quat::IDENTITY), 0);
    let mut noise = Noise::new(3);
    for _ in 0..2000 {
        let q = orientation(noise.next(), noise.next() * 0.5, noise.next() * 0.2)
            * (1.0 + 1e-4 * noise.next());
        let output = filter.step(head(Vec3::ZERO, q.normalize()), FRAME, &LIGHT);
        assert!((output.orientation.length() - 1.0).abs() < 1e-5);
    }
}

#[test]
fn roll_is_damped_more_than_yaw_and_pitch() {
    // The same small rotation of the head, about the view axis and about up, and after the same time.
    let after = |rotation: Quat| {
        let mut filter = Filter::new(head(Vec3::ZERO, Quat::IDENTITY), 0);
        let mut output = filter.output();
        for _ in 0..15 {
            output = filter.step(head(Vec3::ZERO, rotation), FRAME, &LIGHT);
        }
        angle(Quat::IDENTITY, output.orientation) / angle(Quat::IDENTITY, rotation)
    };
    let (roll, yaw, pitch) = (
        after(orientation(0.0, 0.0, 0.1)),
        after(orientation(0.1, 0.0, 0.0)),
        after(orientation(0.0, 0.1, 0.0)),
    );
    assert!(
        roll < yaw * 0.9 && roll < pitch * 0.9,
        "{roll} {yaw} {pitch}"
    );
    assert!((yaw - pitch).abs() < 0.02);
    // But a deliberate tilt of the head is followed, not held level: nine tenths within a second, in the light level.
    let mut filter = Filter::new(head(Vec3::ZERO, Quat::IDENTITY), 0);
    let tilt = orientation(0.0, 0.0, 0.35);
    let mut output = filter.output();
    for _ in 0..60 {
        output = filter.step(head(Vec3::ZERO, tilt), FRAME, &LIGHT);
    }
    assert!(angle(Quat::IDENTITY, output.orientation) > 0.9 * 0.35);
}

#[test]
fn roll_is_the_rotation_about_the_view_axis_even_when_looking_up() {
    // Pitched 60 degrees up, a rotation about the view axis is still the roll, damped as ever.
    let looking = orientation(0.8, 1.0, 0.0);
    let after = |extra: Quat| {
        let mut filter = Filter::new(head(Vec3::ZERO, looking), 0);
        let mut output = filter.output();
        for _ in 0..15 {
            output = filter.step(head(Vec3::ZERO, looking * extra), FRAME, &LIGHT);
        }
        angle(looking, output.orientation) / angle(Quat::IDENTITY, extra)
    };
    assert!(after(Quat::from_rotation_z(0.1)) < after(Quat::from_rotation_y(0.1)) * 0.9);
}

#[test]
fn a_filter_starts_at_rest_where_it_is_put_and_does_not_move_for_no_time() {
    let start = head(Vec3::new(1.0, 2.0, 3.0), orientation(0.3, 0.2, 0.1));
    let filter = Filter::new(start, 4);
    assert_eq!((filter.output(), filter.epoch), (start, 4));
    // The same input forever stays where it is.
    let mut filter = Filter::new(start, 0);
    let output = (0..100)
        .map(|_| filter.step(start, FRAME, &CINEMATIC))
        .last()
        .unwrap();
    assert!(output.position.distance(start.position) < 1e-6);
    assert!(angle(output.orientation, start.orientation) < 1e-3);
}

#[test]
fn rotation_vectors_are_the_shortest_rotation() {
    assert_eq!(rotation_vector(Quat::IDENTITY), Vec3::ZERO);
    // A quaternion and its negative are one rotation.
    assert_eq!(rotation_vector(-Quat::IDENTITY), Vec3::ZERO);
    let q = Quat::from_axis_angle(Vec3::new(1.0, 2.0, 2.0).normalize(), 1.2);
    for sign in [1.0, -1.0] {
        let vector = rotation_vector(q * sign);
        assert!((vector.length() - 1.2).abs() < 1e-5);
        assert!(
            vector
                .normalize()
                .distance(Vec3::new(1.0, 2.0, 2.0).normalize())
                < 1e-5
        );
    }
    // A rotation of a very small angle, where the axis is unknowable, is its small vector.
    let tiny = Quat::from_xyzw(1e-7, 0.0, 0.0, 1.0);
    assert!((rotation_vector(tiny).x - 2e-7).abs() < 1e-9);
    // And the round trip, past a half turn, takes the other way.
    let past = Quat::from_rotation_y(PI * 1.5);
    assert!((rotation_vector(past).length() - PI / 2.0).abs() < 1e-5);
}
