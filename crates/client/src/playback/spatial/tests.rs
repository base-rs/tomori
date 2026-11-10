// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

const RATE: u32 = 48_000;
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn listener() -> Listener {
    Listener {
        position: [0.0, 1.6, 0.0],
        orientation: IDENTITY,
    }
}

/// Turned `degrees` to the left about the vertical axis.
fn turned(degrees: f32) -> [f32; 4] {
    let half = degrees.to_radians() / 2.0;
    [0.0, half.sin(), 0.0, half.cos()]
}

fn at(x: f32, z: f32) -> Position {
    [x, 1.6, z]
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_source_on_the_left_is_louder_and_earlier_on_the_left() {
    let left = place(&listener(), at(-2.0, 0.0), PERSON_FLOOR, RATE, false);
    assert!(left.gain[0] > left.gain[1]);
    assert_eq!(left.delay[0], 0.0);
    assert!(close(left.delay[1], INTERAURAL_DELAY * RATE as f32));
    // The far ear is shadowed, the near one not.
    assert!(left.lowpass[1] < 1.0);
    assert_eq!(left.lowpass[0], 1.0);
    let right = place(&listener(), at(2.0, 0.0), PERSON_FLOOR, RATE, false);
    assert!(close(right.gain[0], left.gain[1]) && close(right.gain[1], left.gain[0]));
    assert!(close(right.delay[0], left.delay[1]) && close(right.delay[1], left.delay[0]));
}

#[test]
fn the_far_ear_never_falls_below_its_floor_and_the_centre_is_unity() {
    let side = place(&listener(), at(2.0, 0.0), PERSON_FLOOR, RATE, false);
    assert!(close(side.gain[0] / side.gain[1], FAR_EAR));
    // Straight ahead at the reference distance: both ears at unity, with nothing delayed or filtered.
    let ahead = place(
        &listener(),
        at(0.0, -REFERENCE_DISTANCE),
        PERSON_FLOOR,
        RATE,
        false,
    );
    assert!(close(ahead.gain[0], 1.0) && close(ahead.gain[1], 1.0));
    assert_eq!(ahead.delay, [0.0, 0.0]);
    assert_eq!(ahead.lowpass, [1.0, 1.0]);
    // Equal power across every direction.
    for degrees in (0..360).step_by(15) {
        let angle = (degrees as f32).to_radians();
        let placed = place(
            &listener(),
            at(2.0 * angle.sin(), -2.0 * angle.cos()),
            PERSON_FLOOR,
            RATE,
            false,
        );
        let power = placed.gain[0].powi(2) + placed.gain[1].powi(2);
        assert!(close(power, 2.0), "{degrees}°: {power}");
        assert!(placed.gain.iter().all(|ear| *ear >= FAR_EAR - 1e-4));
    }
}

#[test]
fn distance_changes_loudness_within_its_limits() {
    let loudness = |distance: f32, floor: f32| {
        place(&listener(), at(0.0, -distance), floor, RATE, true).gain[0]
    };
    assert!(close(loudness(REFERENCE_DISTANCE, PERSON_FLOOR), 1.0));
    assert!(close(loudness(4.0, PERSON_FLOOR), 0.5));
    // No louder than 12 dB above the reference, however near, so that a voice in the same place as the listener is not deafening.
    assert_eq!(NEAREST, 0.5);
    assert!(close(loudness(1.0, PERSON_FLOOR), 2.0));
    assert!(close(loudness(NEAREST, PERSON_FLOOR), 4.0));
    assert!(close(loudness(0.1, PERSON_FLOOR), 4.0));
    // Never quieter than the floor: 12 dB for people, 6 dB for the agent.
    assert!(close(loudness(100.0, PERSON_FLOOR), 0.25));
    assert!(close(loudness(100.0, AGENT_FLOOR), 0.5));
    // At the listener's own head, centred at the nearest loudness.
    let inside = place(&listener(), listener().position, PERSON_FLOOR, RATE, false);
    assert_eq!(inside, Placement::CENTRE.scaled(4.0));
}

#[test]
fn a_source_nearer_than_the_nearest_distance_blends_towards_the_middle_of_the_head() {
    let side = |x: f32, z: f32| place(&listener(), at(x, z), PERSON_FLOOR, RATE, false);
    // At the nearest distance and beyond, fully to the side.
    let edge = side(NEAREST, 0.0);
    assert!(close(edge.gain[0] / edge.gain[1], FAR_EAR));
    assert!(close(edge.delay[0], INTERAURAL_DELAY * RATE as f32));
    // Half as near, half as far to the side, with half the far ear's delay, still at equal power.
    let half = side(NEAREST / 2.0, 0.0);
    assert!(close(
        half.gain[0],
        Placement::panned(0.5).scaled(4.0).gain[0]
    ));
    assert!(close(half.delay[0], INTERAURAL_DELAY * RATE as f32 / 2.0));
    assert!(close(
        half.gain[0].powi(2) + half.gain[1].powi(2),
        2.0 * 16.0
    ));
    assert!(half.lowpass[0] > edge.lowpass[0]);
    // A source 1 cm to the left and one 1 cm to the right sound almost alike, rather than flipping from ear to ear as it passes through the head.
    let (left, right) = (side(-0.01, 0.0), side(0.01, 0.0));
    for placed in [left, right] {
        assert!((placed.gain[0] - placed.gain[1]).abs() < 0.1, "{placed:?}");
        assert!(placed.delay.iter().all(|delay| *delay < 1.0));
    }
    // So do one just ahead and one just behind: a source behind is dulled only as far as it is outside the nearest distance.
    let (ahead, behind) = (side(0.0, -0.01), side(0.0, 0.01));
    assert_eq!(ahead.lowpass, [1.0, 1.0]);
    assert!(
        behind.lowpass.iter().all(|lowpass| *lowpass > 0.95),
        "{behind:?}"
    );
    assert!(side(0.0, 2.0).lowpass[0] < behind.lowpass[0]);
}

#[test]
fn turning_the_head_moves_the_sound() {
    // The agent ahead; the head turned 90° to the left hears it on the right.
    let mut turned_left = listener();
    turned_left.orientation = turned(90.0);
    let agent = place(&turned_left, at(0.0, -2.0), AGENT_FLOOR, RATE, false);
    assert!(agent.gain[1] > agent.gain[0]);
    assert!(close(agent.gain[0] / agent.gain[1], FAR_EAR));
    assert!(close(pan(&turned_left, at(0.0, -2.0)), 1.0));
    assert!(close(pan(&listener(), at(0.0, -2.0)), 0.0));
    assert_eq!(pan(&listener(), listener().position), 0.0);
    // An unnormalized or degenerate orientation is handled.
    let mut scaled = listener();
    scaled.orientation = turned(90.0).map(|component| component * 3.0);
    assert!(close(pan(&scaled, at(0.0, -2.0)), 1.0));
    let mut degenerate = listener();
    degenerate.orientation = [0.0; 4];
    assert!(close(pan(&degenerate, at(2.0, 0.0)), 1.0));
}

#[test]
fn a_source_behind_is_duller_than_one_in_front() {
    let front = place(&listener(), at(0.0, -2.0), PERSON_FLOOR, RATE, false);
    let behind = place(&listener(), at(0.0, 2.0), PERSON_FLOOR, RATE, false);
    assert_eq!(front.gain, behind.gain);
    assert!(behind.lowpass[0] < 1.0 && behind.lowpass[1] < 1.0);
    assert_eq!(front.lowpass, [1.0, 1.0]);
}

#[test]
fn mono_pans_nothing_but_keeps_distance() {
    let mono = place(&listener(), at(-4.0, 0.0), PERSON_FLOOR, RATE, true);
    assert_eq!(mono.gain[0], mono.gain[1]);
    assert!(close(mono.gain[0], 0.5));
    assert_eq!(mono.delay, [0.0, 0.0]);
    assert_eq!(mono.lowpass, [1.0, 1.0]);
}

#[test]
fn the_scene_finds_sources_and_placements_glide() {
    let scene = Scene {
        listener: Some(listener()),
        sources: vec![(7, at(1.0, 0.0))],
        mono: false,
    };
    assert_eq!(scene.position(7), Some(at(1.0, 0.0)));
    assert_eq!(scene.position(8), None);
    let half = Placement::CENTRE.lerp(&Placement::panned(1.0), 0.5);
    assert!(close(
        half.gain[1],
        (1.0 + Placement::panned(1.0).gain[1]) / 2.0
    ));
    assert!(delay_capacity(RATE) as f32 >= INTERAURAL_DELAY * RATE as f32 + 1.0);
}

/// Render `input` through new ears at `placement`, returning the left and right channels.
fn render(ears: &mut Ears, input: &[f32], placement: Placement) -> (Vec<f32>, Vec<f32>) {
    let mut output = vec![0.0; input.len() * 2];
    ears.render(input, placement, (1.0, 1.0), &mut output);
    (
        output.iter().step_by(2).copied().collect(),
        output.iter().skip(1).step_by(2).copied().collect(),
    )
}

#[test]
fn rendering_delays_the_far_ear_by_the_interaural_delay() {
    let placement = place(&listener(), at(-2.0, 0.0), PERSON_FLOOR, RATE, false);
    let mut ears = Ears::new(RATE);
    assert_eq!(ears.placement(), None);
    let mut impulse = vec![0.0; 64];
    impulse[0] = 1.0;
    let (left, right) = render(&mut ears, &impulse, placement);
    assert_eq!(ears.placement(), Some(placement));
    let peak = |channel: &[f32]| {
        channel
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0
    };
    assert_eq!(peak(&left), 0);
    // 0.6 ms at 48 kHz is 28.8 samples, between two taps.
    let delayed = peak(&right);
    assert!((28..=30).contains(&delayed), "{delayed}");
    let energy = |channel: &[f32]| channel.iter().map(|sample| sample * sample).sum::<f32>();
    assert!(energy(&left) > energy(&right));
}

#[test]
fn moving_a_source_glides_without_a_step() {
    let mut ears = Ears::new(RATE);
    let steady = vec![1_000.0; 480];
    render(
        &mut ears,
        &steady,
        place(&listener(), at(-2.0, 0.0), PERSON_FLOOR, RATE, false),
    );
    // Jumping from the left to the right sweeps across, about two thirds of the way in 30 ms, in 10 ms callbacks.
    let target = place(&listener(), at(2.0, 0.0), PERSON_FLOOR, RATE, false);
    let mut previous = None;
    for call in 0..20 {
        let (left, right) = render(&mut ears, &steady, target);
        for channel in [&left, &right] {
            let largest = channel
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0, f32::max);
            assert!(largest < 10.0, "a step of {largest} in call {call}");
        }
        // The far ear's delay moves slowly enough not to chirp: at most a few samples per callback.
        let delay = ears.placement().unwrap().delay;
        if let Some(previous) = previous {
            let [a, b]: [f32; 2] = previous;
            assert!((delay[0] - a).abs() < 8.0 && (delay[1] - b).abs() < 8.0);
        }
        previous = Some(delay);
        if call == 1 {
            assert!(left[479] > right[479]);
        }
        if call == 19 {
            assert!(left[479] < right[479]);
        }
    }
}

#[test]
fn the_agent_speaking_as_it_streams_past_close_by_glides_without_a_step() {
    let mut ears = Ears::new(RATE);
    let steady = vec![1_000.0; 480];
    // Its voice streams from 3 m to the left to 3 m to the right at 6 m/s, passing 0.3 m in front, within the nearest
    // distance, a new place each 10 ms callback, as a client places it each frame while it follows someone.
    render(
        &mut ears,
        &steady,
        place(&listener(), at(-3.0, -0.3), AGENT_FLOOR, RATE, false),
    );
    let mut previous: Option<[f32; 2]> = None;
    for call in 0..100 {
        let x = -3.0 + 6.0 * call as f32 / 100.0;
        let placement = place(&listener(), at(x, -0.3), AGENT_FLOOR, RATE, false);
        let (left, right) = render(&mut ears, &steady, placement);
        for channel in [&left, &right] {
            let largest = channel
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0, f32::max);
            assert!(largest < 10.0, "a step of {largest} in call {call}");
            // Never silent, nor louder than its nearest, 12 dB above unity, in the nearer ear.
            assert!(
                channel
                    .iter()
                    .all(|sample| sample.abs() < 1_000.0 * 4.0 * 1.42)
            );
            assert!(
                channel[479].abs() > 100.0,
                "{} in call {call}",
                channel[479]
            );
        }
        let delay = ears.placement().unwrap().delay;
        if let Some([a, b]) = previous {
            let [c, d] = delay;
            assert!((c - a).abs() < 8.0 && (d - b).abs() < 8.0, "call {call}");
        }
        previous = Some(delay);
    }
}

#[test]
fn mono_rendering_glides_its_gain() {
    let mut ears = Ears::new(RATE);
    let mut output = vec![0.0; 4];
    ears.render_mono(&[1.0; 4], Placement::CENTRE, (1.0, 1.0), &mut output);
    assert_eq!(output, [1.0; 4]);
    // Gliding towards silence: quieter each sample, but far from silent after 4 samples.
    let mut output = vec![0.0; 4];
    ears.render_mono(
        &[1.0; 4],
        Placement::CENTRE.scaled(0.0),
        (1.0, 1.0),
        &mut output,
    );
    assert!(output.windows(2).all(|pair| pair[1] < pair[0]));
    assert!(output[3] > 0.99);
    // A first render goes straight to its placement, with the gain gliding.
    let mut fresh = Ears::new(RATE);
    let mut output = vec![0.0; 4];
    fresh.render_mono(&[1.0; 4], Placement::CENTRE, (1.0, 0.5), &mut output);
    assert_eq!(output, [0.875, 0.75, 0.625, 0.5]);
}

#[test]
fn a_source_at_no_finite_distance_is_centred_at_full_gain() {
    for source in [at(f32::INFINITY, 0.0), at(f32::NAN, 0.0)] {
        let placement = place(&listener(), source, PERSON_FLOOR, RATE, false);
        assert_eq!(placement, Placement::CENTRE);
    }
}
