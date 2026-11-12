// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// The ribbon's quads that may glow, each as where it starts and ends, the ways it runs there and when each was laid.
fn segments(embers: &Embers) -> Vec<[EmberVertex; 2]> {
    embers
        .glowing()
        .filter(|quad| quad[0].kind() == Kind::Ribbon)
        .map(|quad| [quad[0], quad[2]])
        .collect()
}

/// A marshmallow burning on stick 1, moved from `from` by `step` each frame of `frame` seconds, `frames` times, from
/// `start`; the time of the last frame.
fn swing(
    embers: &mut Embers,
    (from, step): (Vec3, Vec3),
    (start, frame): (f64, f64),
    frames: usize,
) -> f64 {
    let mut now = start;
    for index in 0..=frames {
        now = start + frame * index as f64;
        embers.update(now, &[(1, from + step * index as f32)]);
    }
    now
}

#[test]
fn a_trail_glows_for_its_life_thin_and_yellow_white_then_wider_and_dull_red_and_then_is_gone() {
    // Brightest at once, fading to nothing at its life's end, and nothing after.
    assert_eq!(glow(0.0), 1.0);
    assert_eq!(glow(LIFE), 0.0);
    assert_eq!(glow(LIFE * 2.0), 0.0);
    let ages: Vec<f32> = (0..=12).map(|step| LIFE * step as f32 / 12.0).collect();
    for pair in ages.windows(2) {
        let (young, old) = (pair[0], pair[1]);
        assert!(glow(old) < glow(young), "{young} {old}");
        // Widening as it fades, from thin to a little wider.
        assert!(width(old) > width(young), "{young} {old}");
        // Cooling: ever redder, its green and blue falling against its red.
        let (young, old) = (colour(young), colour(old));
        assert!(old.y / old.x < young.y / young.x, "{young} {old}");
        assert!(old.z / old.x <= young.z / young.x, "{young} {old}");
    }
    assert_eq!(width(0.0), FRESH_WIDTH);
    assert_eq!(width(LIFE), FADED_WIDTH);
    const { assert!(FADED_WIDTH < FRESH_WIDTH * 3.0) };
    // Yellow-white when fresh, orange halfway, dull red at its end.
    let fresh = colour(0.0);
    assert!(fresh.min_element() > 0.6, "{fresh}");
    let halfway = colour(LIFE * 0.5);
    assert!(
        halfway.x > 0.9 && (0.3..0.6).contains(&halfway.y) && halfway.z < 0.2,
        "{halfway}"
    );
    let faded = colour(LIFE);
    assert!(faded.x < 0.7 && faded.y < 0.1, "{faded}");
    // Drawn as it is laid, a stroke glows while it is younger than its life and is gone after, its sparks and embers with
    // it, drawing nothing.
    let mut embers = Embers::new();
    let end = swing(
        &mut embers,
        (Vec3::ZERO, Vec3::X * 0.01),
        (10.0, 1.0 / 72.0),
        72,
    );
    assert!(embers.live() > 50, "{}", embers.live());
    let first = segments(&embers)[0][0].shape[2];
    embers.update(end + f64::from(LIFE) * 0.5, &[]);
    assert!(embers.live() > 0);
    // The oldest, laid a second before the last, has gone a second before the last.
    embers.update(end + f64::from(LIFE) - 0.5, &[]);
    assert!(
        segments(&embers)
            .iter()
            .all(|segment| segment[1].shape[2] > first + 0.4)
    );
    embers.update(end + f64::from(LIFE) + 0.01, &[]);
    assert_eq!(embers.live(), 0);
    assert_eq!(embers.drawn(), [0..0, 0..0]);
    assert_eq!(embers.glowing().count(), 0);
}

#[test]
fn points_are_laid_by_distance_joining_smoothly_when_slow_and_leaving_no_gaps_when_fast() {
    // Held still, it lays nothing but an ember now and then.
    let mut embers = Embers::new();
    swing(&mut embers, (Vec3::ONE, Vec3::ZERO), (0.0, 1.0 / 72.0), 144);
    assert!(segments(&embers).is_empty());
    assert!(embers.glowing().all(|quad| quad[0].kind() == Kind::Ember));
    assert!((2..=8).contains(&embers.live()), "{}", embers.live());
    // Slowly, a millimetre a frame, it lays a point each time it has moved its spacing, not each frame.
    let mut embers = Embers::new();
    swing(
        &mut embers,
        (Vec3::ZERO, Vec3::X * 0.001),
        (0.0, 1.0 / 72.0),
        120,
    );
    let slow = segments(&embers);
    assert!((9..=10).contains(&slow.len()), "{}", slow.len());
    for segment in &slow {
        let length = segment[0].at().distance(segment[1].at());
        assert!(
            (SPACING - 1e-4..SPACING + 1.1e-3).contains(&length),
            "{length}"
        );
    }
    // Fast, 30 cm a frame round a circle, more than 20 m a second, every frame's way is laid as segments no longer than
    // its share of the most a frame lays, each beginning where the last ended, the way it ended, with no gap.
    let mut embers = Embers::new();
    let mut now = 0.0;
    for frame in 0..24 {
        let turn = frame as f32 * 0.3;
        now = f64::from(frame) / 72.0;
        embers.update(now, &[(2, Vec3::new(turn.cos(), turn.sin(), 0.0))]);
    }
    let fast = segments(&embers);
    assert!(fast.len() >= 23 * STEPS, "{}", fast.len());
    for segment in &fast {
        let length = segment[0].at().distance(segment[1].at());
        assert!(length <= 0.3 / STEPS as f32 + 1e-4, "{length}");
    }
    for pair in fast.windows(2) {
        assert_eq!(pair[1][0].at, pair[0][1].at);
        assert_eq!(pair[1][0].way, pair[0][1].way);
        assert_eq!(pair[1][0].shape[2], pair[0][1].shape[2]);
    }
    // Each point laid when the marshmallow passed it, between the frames, the latest the last frame's.
    for pair in fast.windows(2) {
        assert!(pair[1][1].shape[2] > pair[0][1].shape[2]);
    }
    assert!((f64::from(fast.last().unwrap()[1].shape[2]) - now).abs() < 1e-5);
    // A swing that fast throws sparks along it, and drops embers.
    let sparks = embers
        .glowing()
        .filter(|quad| quad[0].kind() == Kind::Spark)
        .count();
    assert!(sparks >= 70, "{sparks}");
    // A jump farther than a frame's swing could carry it, as when the stick is taken into a hand, begins the trail anew
    // there, drawing nothing across it.
    let before = segments(&embers).len();
    embers.update(now + 0.014, &[(2, Vec3::new(5.0, 0.0, 0.0))]);
    embers.update(now + 0.028, &[(2, Vec3::new(5.0, 0.0, 0.005))]);
    assert_eq!(segments(&embers).len(), before);
    embers.update(now + 0.042, &[(2, Vec3::new(5.0, 0.0, 0.05))]);
    let after = segments(&embers);
    assert!(after.len() > before);
    assert!(
        after[before..]
            .iter()
            .all(|segment| segment[0].at[0] == 5.0)
    );
    // A stick that stops burning ends its trail; lit again, it begins anew where it is, joining nothing.
    embers.update(now + 0.056, &[]);
    embers.update(now + 0.07, &[(2, Vec3::new(5.0, 0.0, 0.3))]);
    embers.update(now + 0.084, &[(2, Vec3::new(5.0, 0.0, 0.32))]);
    let lit = segments(&embers);
    assert!(lit.len() > after.len());
    assert!((lit[after.len()][0].at[2] - 0.3).abs() < 1e-6);
    // Two at once lay a trail each, apart.
    let mut embers = Embers::new();
    for frame in 0..30 {
        let along = frame as f32 * 0.01;
        embers.update(
            f64::from(frame) / 72.0,
            &[
                (0, Vec3::new(along, 0.0, 0.0)),
                (3, Vec3::new(along, 1.0, 0.0)),
            ],
        );
    }
    let both = segments(&embers);
    assert!(
        both.iter()
            .all(|segment| segment[0].at[1] == segment[1].at[1])
    );
    assert!(both.iter().any(|segment| segment[0].at[1] == 1.0));
    assert!(both.iter().any(|segment| segment[0].at[1] == 0.0));
    // Nothing of a stick that is none, nor of a marshmallow nowhere.
    let mut embers = Embers::new();
    for frame in 0..30 {
        let along = frame as f32 * 0.01;
        embers.update(
            f64::from(frame) / 72.0,
            &[(STICKS, Vec3::X * along), (0, Vec3::NAN)],
        );
    }
    assert_eq!(embers.live(), 0);
}

#[test]
fn the_ring_holds_a_fixed_number_of_quads_overwriting_the_oldest_and_says_what_to_take_to_the_gpu_and_draw()
 {
    let mut embers = Embers::default();
    let capacity = embers.vertices().len();
    assert_eq!(capacity, QUADS * 6);
    // Swung fast for a second and a half, far more than the ring holds.
    let mut now = 0.0;
    let mut written = 0;
    for frame in 0..108 {
        let turn = frame as f32 * 0.25;
        now = f64::from(frame) / 72.0;
        embers.update(
            now,
            &(0..STICKS)
                .map(|index| (index, Vec3::new(turn.cos(), turn.sin(), index as f32)))
                .collect::<Vec<_>>(),
        );
        // What was written each frame is what it laid, in at most two ranges, never more than the ring.
        let ranges = embers.written();
        let count: usize = ranges.iter().map(|range| range.len()).sum();
        assert!(count <= QUADS * 6 && count.is_multiple_of(6));
        assert!(ranges.iter().all(|range| range.end <= QUADS * 6));
        written += count / 6;
        // And nothing more until it lays more.
        assert_eq!(embers.written(), [0..0, 0..0]);
        // At most every stick's steps a frame, with their sparks and embers.
        assert!(count / 6 <= STICKS * (STEPS + 6), "{count}");
    }
    assert!(written > QUADS, "{written}");
    // The ring is full, wrapping, drawn in two draws of all of it, the newest just before the oldest.
    assert_eq!(embers.live(), QUADS);
    let [first, second] = embers.drawn();
    assert_eq!(first.len() + second.len(), QUADS * 6);
    assert_eq!(first.end as usize, QUADS * 6);
    assert_eq!(second.start, 0);
    assert_eq!(second.end, first.start);
    assert_eq!(embers.vertices().len(), capacity);
    // Once all of it has faded, nothing is drawn; and the next stroke is drawn in one draw from where the ring is.
    embers.update(now + f64::from(LIFE) + 0.1, &[]);
    assert_eq!(embers.drawn(), [0..0, 0..0]);
    swing(
        &mut embers,
        (Vec3::ZERO, Vec3::X * 0.02),
        (now + 4.0, 1.0 / 72.0),
        10,
    );
    let [first, second] = embers.drawn();
    assert!(!first.is_empty());
    assert_eq!(first.len() + second.len(), embers.live() * 6);
    assert!(
        embers.live() >= 10 && embers.live() < 40,
        "{}",
        embers.live()
    );
    // A clock run back clears it.
    embers.update(1.0, &[]);
    assert_eq!(embers.live(), 0);
    embers.update(f64::NAN, &[(0, Vec3::ZERO)]);
    assert_eq!(embers.live(), 0);
}

#[test]
fn quads_are_stamped_by_the_shader_s_wrapping_clock_and_a_painted_path_ends_at_its_time() {
    let mut embers = Embers::new();
    // Painted across the clock's wrap, each point is stamped as the shader's clock said when it was laid.
    let path: Vec<Vec3> = (0..40).map(|step| Vec3::X * step as f32 * 0.02).collect();
    embers.paint(0, &path, 0.02, WRAP + 0.3);
    let laid = segments(&embers);
    assert!(laid.len() >= 39);
    let last = laid.last().unwrap()[1].shape[2];
    assert!((last - 0.3).abs() < 1e-4, "{last}");
    let first = laid[0][0].shape[2];
    assert!((f64::from(first) - (WRAP - 0.48)).abs() < 1e-3, "{first}");
    // Its points along the path, ending at its end.
    assert_eq!(laid.last().unwrap()[1].at(), *path.last().unwrap());
    // Nothing of a stick that is none.
    let mut embers = Embers::new();
    embers.paint(STICKS, &path, 0.02, 1.0);
    assert_eq!(embers.live(), 0);
}
