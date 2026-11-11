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
