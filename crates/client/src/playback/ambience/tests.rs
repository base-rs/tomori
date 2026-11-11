// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

fn close(a: Position, b: Position) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
}

#[test]
fn a_slot_has_a_name_and_a_sound_or_none() {
    let tone: &'static [u8] = b"tone";
    let embedded = Slot::embedded("tui-1", tone);
    assert_eq!(embedded.name(), "tui-1");
    assert!(!embedded.is_missing());
    assert_eq!(embedded.load().as_deref(), Some(tone));
    let loaded = Slot::new("river", || Some(vec![1, 2, 3]));
    assert_eq!(loaded.load(), Some(vec![1, 2, 3]));
    let missing = Slot::missing("ruru-1");
    assert!(missing.is_missing());
    assert_eq!(missing.load(), None);
    assert_eq!(
        format!("{missing:?}"),
        r#"Slot { name: "ruru-1", missing: true }"#
    );
    assert_eq!(gain(0.0), 1.0);
    assert!((gain(-20.0) - 0.1).abs() < 1e-6);
}

#[test]
fn a_path_is_heard_from_its_point_nearest_the_listener() {
    let river = Path::new(&[[-10.0, 0.0, -5.0], [10.0, 0.0, -5.0], [10.0, 0.0, -25.0]]);
    assert_eq!(river.points().len(), 3);
    // Beside the first stretch, from straight across; past its ends, from its ends; and nearer the second, from it.
    assert!(close(river.nearest([2.0, 1.6, 0.0]), [2.0, 0.0, -5.0]));
    assert!(close(river.nearest([-30.0, 0.0, 0.0]), [-10.0, 0.0, -5.0]));
    assert!(close(river.nearest([14.0, 0.0, -20.0]), [10.0, 0.0, -20.0]));
    // A point is heard from itself, and a path of none from the origin.
    let waterfall = Path::new(&[[3.0, 2.0, -40.0]]);
    assert_eq!(waterfall.nearest([0.0; 3]), [3.0, 2.0, -40.0]);
    assert_eq!(Path::new(&[]).points(), [[0.0; 3]]);
    // A stretch of no length is its point.
    let still = Path::new(&[[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    assert_eq!(still.nearest([5.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
    // It keeps its first points, as many as it holds.
    let long: Vec<Position> = (0..20).map(|x| [x as f32, 0.0, 0.0]).collect();
    let kept = Path::new(&long);
    assert_eq!(kept.points().len(), PATH_POINTS);
    assert_eq!(kept.points()[PATH_POINTS - 1], [15.0, 0.0, 0.0]);
    let moved = river.carried(&|[x, y, z]| [x + 1.0, y, z]);
    assert_eq!(moved.points()[0], [-9.0, 0.0, -5.0]);
}

#[test]
fn an_ambience_lists_its_slots_and_is_carried_whole() {
    let ambience = Ambience::new()
        .with_bed(Bed::new(Slot::missing("wind")).gain(-6.0))
        .with_loop(
            Loop::along(
                Slot::missing("river"),
                &[[0.0, 0.0, -10.0], [5.0, 0.0, -10.0]],
            )
            .radius(40.0)
            .gain(-3.0),
        )
        .with_loop(Loop::at(Slot::missing("waterfall"), [2.0, 4.0, -30.0]))
        .with_call(
            Call::new(vec![Slot::missing("tui-1"), Slot::missing("tui-2")])
                .every(Duration::from_secs(8), Duration::from_secs(4))
                .around(&[[1.0, 6.0, -12.0]], 5.0)
                .pitch(1.1, 0.9)
                .gain(-3.0, -9.0),
        )
        .with_call(Call::new(Vec::new()).around(&[], -1.0));
    assert_eq!(
        ambience.slots().map(Slot::name).collect::<Vec<_>>(),
        ["wind", "river", "waterfall", "tui-1", "tui-2"]
    );
    assert_eq!(ambience.beds[0].gain, -6.0);
    assert_eq!(
        (ambience.loops[0].radius, ambience.loops[0].gain),
        (40.0, -3.0)
    );
    assert_eq!(
        (ambience.loops[1].radius, ambience.loops[1].gain),
        (30.0, 0.0)
    );
    // Ranges whose ends are given the wrong way round are as narrow as the first end.
    let call = &ambience.calls[0];
    assert_eq!(call.every, [Duration::from_secs(8); 2]);
    assert_eq!(call.pitch, [1.1, 1.1]);
    assert_eq!(call.gain, [-3.0, -3.0]);
    assert_eq!(
        (ambience.calls[1].around.clone(), ambience.calls[1].spread),
        (vec![[0.0; 3]], 0.0)
    );
    // Carried, every place moves, and nothing else changes.
    let carried = ambience.carried(|[x, y, z]| [-z, y, x]);
    assert_eq!(carried.loops[0].path.points()[1], [10.0, 0.0, 5.0]);
    assert_eq!(carried.loops[1].path.points()[0], [30.0, 4.0, 2.0]);
    assert_eq!(carried.calls[0].around, [[12.0, 6.0, 1.0]]);
    assert_eq!(carried.calls[0].spread, 5.0);
    assert_eq!(carried.beds[0].gain, -6.0);
    assert_eq!(carried.slots().count(), 5);
}

#[test]
fn random_numbers_are_the_same_for_the_same_seed_and_within_their_ranges() {
    let draw = |seed| {
        let mut random = Random::new(seed);
        (0..100).map(|_| random.next_u64()).collect::<Vec<_>>()
    };
    assert_eq!(draw(7), draw(7));
    assert_ne!(draw(7), draw(8));
    let mut random = Random::new(1);
    for _ in 0..10_000 {
        let unit = random.unit();
        assert!((0.0..1.0).contains(&unit));
        assert!((2.0..=3.0).contains(&random.between(2.0, 3.0)));
        assert!(random.below(3) < 3);
    }
    assert_eq!(random.below(1), 0);
}

fn tui() -> Call {
    Call::new(vec![
        Slot::missing("tui-1"),
        Slot::missing("tui-2"),
        Slot::missing("tui-3"),
    ])
    .every(Duration::from_secs(10), Duration::from_secs(30))
    .around(&[[-10.0, 5.0, -20.0], [10.0, 7.0, -20.0]], 4.0)
    .pitch(0.9, 1.1)
    .gain(-6.0, 0.0)
}

fn ruru() -> Call {
    Call::new(vec![Slot::missing("ruru-1")])
        .every(Duration::from_secs(60), Duration::from_secs(120))
}

/// Every call heard over `length`, checked each second.
fn hear(calls: &[Call], seed: u64, length: u64) -> Vec<(Duration, Due)> {
    let mut scheduler = Scheduler::new(calls, seed, Duration::ZERO);
    let mut heard = Vec::new();
    for second in 0..=length {
        let now = Duration::from_secs(second);
        while let Some(due) = scheduler.due(calls, now) {
            heard.push((now, due));
        }
    }
    heard
}

#[test]
fn calls_are_heard_at_intervals_within_their_range_from_places_pitches_and_gains_within_theirs() {
    let calls = [tui(), ruru()];
    let heard = hear(&calls, 42, 3_600);
    let tuis: Vec<_> = heard.iter().filter(|(_, due)| due.call == 0).collect();
    let rurus: Vec<_> = heard.iter().filter(|(_, due)| due.call == 1).collect();
    // The first within its longest interval, and each after within its range of the last.
    assert!(tuis[0].0 <= Duration::from_secs(30));
    assert!(rurus[0].0 <= Duration::from_secs(120));
    for pair in tuis.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!(
            (Duration::from_secs(10)..=Duration::from_secs(31)).contains(&gap),
            "{gap:?}"
        );
    }
    for pair in rurus.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!((Duration::from_secs(60)..=Duration::from_secs(121)).contains(&gap));
    }
    // About an hour's worth: three or four minutes apart on average for the tūī, a minute and a half for the ruru.
    assert!((110..=360).contains(&tuis.len()), "{}", tuis.len());
    assert!((29..=60).contains(&rurus.len()), "{}", rurus.len());
    for (_, due) in &tuis {
        // Within 4 m, across the ground, of one of its trees, at its height.
        let near = [[-10.0, 5.0, -20.0], [10.0, 7.0, -20.0]]
            .iter()
            .any(|tree: &Position| {
                let (dx, dz) = (due.position[0] - tree[0], due.position[2] - tree[2]);
                due.position[1] == tree[1] && (dx * dx + dz * dz).sqrt() <= 4.0 + 1e-4
            });
        assert!(near, "{:?}", due.position);
        assert!((0.9..=1.1).contains(&due.pitch));
        assert!((gain(-6.0) - 1e-6..=1.0 + 1e-6).contains(&due.gain));
        assert!(due.variant < 3);
    }
    // Every variant and both trees are heard, and the pitch varies, so that it never sounds like a loop.
    for variant in 0..3 {
        assert!(tuis.iter().any(|(_, due)| due.variant == variant));
    }
    assert!(tuis.iter().any(|(_, due)| due.position[0] < 0.0));
    assert!(tuis.iter().any(|(_, due)| due.position[0] > 0.0));
    assert!(
        tuis.iter().any(|(_, due)| due.pitch < 0.95)
            && tuis.iter().any(|(_, due)| due.pitch > 1.05)
    );
    // No variant twice running; a call of one is always that one, from the origin.
    assert!(
        tuis.windows(2)
            .all(|pair| pair[0].1.variant != pair[1].1.variant)
    );
    assert!(
        rurus
            .iter()
            .all(|(_, due)| due.variant == 0 && due.position == [0.0; 3])
    );
    assert!(
        rurus
            .iter()
            .all(|(_, due)| due.pitch == 1.0 && due.gain == 1.0)
    );
}

#[test]
fn the_same_seed_hears_the_same_calls_and_another_seed_others() {
    let calls = [tui(), ruru()];
    assert_eq!(hear(&calls, 9, 600), hear(&calls, 9, 600));
    assert_ne!(hear(&calls, 9, 600), hear(&calls, 10, 600));
}

#[test]
fn calls_held_back_are_heard_once_and_then_after_their_interval() {
    let calls = [tui()];
    let mut scheduler = Scheduler::new(&calls, 3, Duration::ZERO);
    assert_eq!(scheduler.next().len(), 1);
    // Nothing due yet.
    assert_eq!(scheduler.due(&calls, Duration::ZERO), None);
    // An hour later, as after ambience was off, it is heard once, not once for each interval missed.
    let late = Duration::from_secs(3_600);
    assert!(scheduler.due(&calls, late).is_some());
    assert_eq!(scheduler.due(&calls, late), None);
    assert!(scheduler.next()[0] >= late + Duration::from_secs(10));
    // A call around nowhere, as its fields allow, is heard from the origin.
    let nowhere = [Call {
        around: Vec::new(),
        ..tui()
    }];
    let mut scheduler = Scheduler::new(&nowhere, 3, Duration::ZERO);
    let due = scheduler.due(&nowhere, late).unwrap();
    assert_eq!(due.position[1], 0.0);
}

/// The farmyard's ruru: usually four or five calls in a row, sometimes one, 1.5 to 2.5 s apart, its two calls 1.25 s
/// long, the runs 120 to 300 s apart.
fn calling_ruru() -> Call {
    Call::new(vec![Slot::missing("ruru-1"), Slot::missing("ruru-2")])
        .every(Duration::from_secs(120), Duration::from_secs(300))
        .around(&[[-10.0, 6.0, -20.0], [10.0, 6.0, -20.0]], 5.0)
        .pitch(0.98, 1.02)
        .gain(13.0, 17.0)
        .run(
            Run::weighted(&[15.0, 5.0, 15.0, 25.0, 25.0, 15.0])
                .gap(Duration::from_millis(1_500), Duration::from_millis(2_500)),
        )
}

const RURU: Duration = Duration::from_millis(1_250);

/// Every call heard, each at the moment it is due, until `until`, with each variant `RURU` long.
fn hear_exactly(calls: &[Call], seed: u64, until: Duration) -> Vec<(Duration, Due)> {
    let lengths = calls
        .iter()
        .map(|call| vec![RURU; call.variants.len()])
        .collect();
    let mut scheduler = Scheduler::new(calls, seed, Duration::ZERO).with_lengths(lengths);
    let mut heard = Vec::new();
    loop {
        let now = scheduler.next().iter().copied().min().unwrap();
        if now > until {
            return heard;
        }
        while let Some(due) = scheduler.due(calls, now) {
            heard.push((now, due));
        }
    }
}

/// When a call heard at `at` as `due` ends.
fn end(at: Duration, due: &Due) -> Duration {
    at + RURU.div_f64(f64::from(due.pitch))
}

/// The calls heard, in runs: each heard within its run's longest gap of the last's end.
fn runs(heard: &[(Duration, Due)], gap: Duration) -> Vec<Vec<(Duration, Due)>> {
    let mut runs: Vec<Vec<(Duration, Due)>> = Vec::new();
    for (at, due) in heard {
        match runs.last_mut() {
            Some(run) if *at <= end(run[run.len() - 1].0, &run[run.len() - 1].1) + gap => {
                run.push((*at, *due))
            }
            _ => runs.push(vec![(*at, *due)]),
        }
    }
    runs
}

#[test]
fn a_run_is_as_long_as_its_weights_say() {
    let calls = [calling_ruru()];
    let heard = hear_exactly(&calls, 7, Duration::from_secs(500_000));
    let runs = runs(&heard, Duration::from_millis(2_500));
    assert!(runs.len() > 2_000, "{}", runs.len());
    let mut counts = [0usize; 8];
    for run in &runs {
        counts[run.len().min(7)] += 1;
    }
    // Never none or more than six, and each as often as its weight, within two in a hundred.
    assert_eq!((counts[0], counts[7]), (0, 0));
    for (length, weight) in [
        (1, 15.0),
        (2, 5.0),
        (3, 15.0),
        (4, 25.0),
        (5, 25.0),
        (6, 15.0),
    ] {
        let share = 100.0 * counts[length] as f64 / runs.len() as f64;
        assert!(
            (share - weight).abs() < 2.0,
            "{length}: {share}% for {weight}"
        );
    }
}

#[test]
fn a_run_is_one_bird_calling_within_its_gap_and_the_next_comes_after_its_end() {
    let calls = [calling_ruru()];
    let heard = hear_exactly(&calls, 11, Duration::from_secs(100_000));
    let runs = runs(&heard, Duration::from_millis(2_500));
    let close = |a: Duration, b: Duration| a.abs_diff(b) < Duration::from_micros(10);
    for run in &runs {
        let first = run[0].1;
        for pair in run.windows(2) {
            let ((at, due), (next, then)) = (pair[0], pair[1]);
            // From the end of one to the start of the next, within its gap.
            let gap = next - end(at, &due);
            assert!(
                (Duration::from_millis(1_500)..=Duration::from_millis(2_500)).contains(&gap),
                "{gap:?}"
            );
            // The two calls alternating, the same bird, at about its pitch and gain, within the call's ranges.
            assert_eq!(then.variant, 1 - due.variant);
            assert_eq!(then.position, first.position);
            assert!((then.pitch / first.pitch - 1.0).abs() <= 0.01 + 1e-6);
            assert!((0.98..=1.02).contains(&then.pitch));
            let decibels = 20.0 * then.gain.log10();
            assert!((decibels - 20.0 * first.gain.log10()).abs() <= 1.0 + 1e-3);
            assert!((13.0 - 1e-3..=17.0 + 1e-3).contains(&decibels));
        }
    }
    // Each run 120 to 300 s after the last's end, from somewhere else in the bush.
    for pair in runs.windows(2) {
        let last = pair[0][pair[0].len() - 1];
        let rest = pair[1][0].0 - end(last.0, &last.1);
        assert!(
            (Duration::from_secs(120)..=Duration::from_secs(300)).contains(&rest)
                || close(rest, Duration::from_secs(300)),
            "{rest:?}"
        );
    }
    assert!(
        runs.windows(2)
            .any(|pair| pair[0][0].1.position != pair[1][0].1.position)
    );
    assert_eq!(
        heard,
        hear_exactly(&calls, 11, Duration::from_secs(100_000))
    );
}

#[test]
fn a_run_held_back_past_its_gap_begins_again_and_one_heard_late_goes_on() {
    let calls = [calling_ruru().run(
        Run::weighted(&[0.0, 0.0, 0.0, 1.0]).gap(Duration::from_secs(2), Duration::from_secs(2)),
    )];
    let lengths = vec![vec![RURU; 2]];
    let mut scheduler = Scheduler::new(&calls, 5, Duration::ZERO).with_lengths(lengths);
    let start = scheduler.next()[0];
    let first = scheduler.due(&calls, start).unwrap();
    let second = end(start, &first) + Duration::from_secs(2);
    assert_eq!(scheduler.next()[0], second);
    // Heard a second late, as under a busy thread, the run goes on, from the same bird, its next gap from this end.
    let late = second + Duration::from_secs(1);
    let next = scheduler.due(&calls, late).unwrap();
    assert_eq!(next.position, first.position);
    assert_eq!(
        scheduler.next()[0],
        end(late, &next) + Duration::from_secs(2)
    );
    // An hour late, as after ambience was off, the run begins again, a whole run of four from another bird.
    let resumed = Duration::from_secs(3_600);
    let heard: Vec<Due> = (0..4)
        .map(|_| {
            let at = scheduler.next()[0].max(resumed);
            scheduler.due(&calls, at).unwrap()
        })
        .collect();
    assert!(heard.iter().all(|due| due.position == heard[0].position));
    assert_ne!(heard[0].position, first.position);
    assert!(scheduler.next()[0] >= resumed + Duration::from_secs(120));
}

#[test]
fn a_run_is_declared_within_its_bounds_and_a_call_without_one_is_heard_as_before() {
    let run = Run::weighted(&[1.0])
        .gap(Duration::from_secs(3), Duration::from_secs(1))
        .vary(-1.0, -2.0)
        .order(Order::Random);
    assert_eq!(run.gap, [Duration::from_secs(3); 2]);
    assert_eq!(run.vary, [0.0, 0.0]);
    assert_eq!(run.order, Order::Random);
    let default = Run::weighted(&[]);
    assert_eq!(
        default.gap,
        [Duration::from_secs(1), Duration::from_secs(2)]
    );
    assert_eq!(
        (default.vary, default.order),
        ([0.01, 1.0], Order::Alternate)
    );
    assert_eq!(tui().run, None);
    // Weights that are not finite or below 0 are never chosen, nor those beyond its most calls, and none is one call.
    for (weights, length) in [
        (vec![f32::NAN, -1.0, f32::INFINITY, 2.0], 4),
        (
            (0..RUN_CALLS + 4)
                .map(|index| if index < RUN_CALLS { 0.0 } else { 1.0 })
                .collect(),
            1,
        ),
        (Vec::new(), 1),
    ] {
        let calls = [calling_ruru().run(Run::weighted(&weights))];
        let runs = runs(
            &hear_exactly(&calls, 3, Duration::from_secs(20_000)),
            Duration::from_secs(2),
        );
        assert!(runs.len() > 50);
        assert!(runs.iter().all(|run| run.len() == length), "{weights:?}");
    }
    // Taken at random, its variants are any, the last again too, and without its lengths a gap counts from its start.
    let calls = [calling_ruru().run(
        Run::weighted(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0])
            .gap(Duration::from_secs(1), Duration::from_secs(1))
            .order(Order::Random),
    )];
    let mut scheduler = Scheduler::new(&calls, 2, Duration::ZERO);
    let mut at = scheduler.next()[0];
    let mut heard = Vec::new();
    for _ in 0..40 {
        heard.push(scheduler.due(&calls, at).unwrap().variant);
        let next = scheduler.next()[0];
        if heard.len() % 8 != 0 {
            assert_eq!(next - at, Duration::from_secs(1));
        }
        at = next;
    }
    assert!(heard.windows(2).any(|pair| pair[0] == pair[1]));
    assert!(heard.windows(2).any(|pair| pair[0] != pair[1]));
    // A call without a run is heard exactly as it was before runs, its lengths known or not.
    let calls = [tui(), ruru()];
    let mut known = Scheduler::new(&calls, 42, Duration::ZERO).with_lengths(vec![
        vec![Duration::from_secs(5); 3],
        vec![Duration::from_secs(1)],
    ]);
    let without = hear(&calls, 42, 3_600);
    for (at, due) in &without {
        assert_eq!(known.due(&calls, *at), Some(*due));
    }
    let first: Vec<_> = without
        .iter()
        .take(3)
        .map(|(at, due)| (at.as_secs(), due.call, due.variant, due.pitch, due.gain))
        .collect();
    assert_eq!(first, BEFORE_RUNS);
}

/// The first calls of `tui` and `ruru` from seed 42, as the scheduler gave them before calls had runs.
const BEFORE_RUNS: [(u64, usize, usize, f32, f32); 3] = [
    (20, 1, 0, 1.0, 1.0),
    (23, 0, 0, 1.033_032, 0.576_807),
    (46, 0, 1, 0.914_610_74, 0.758_481_3),
];
