// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::anchors::EYES;

const NOW: f64 = 1_800_000_000.0;

/// The default room, about the default home, from home's eyes, nothing else there.
fn open() -> Room {
    Room::open(Anchor::AGENT, EYES)
}

fn timer(name: &str, state: TimerState, left: Option<f64>) -> Timer {
    Timer::new(
        name,
        "every hour",
        "when present",
        left.map(|left| (NOW + left) as u64),
        state,
    )
}

fn named(count: usize) -> Vec<Timer> {
    (0..count)
        .map(|index| timer(&format!("t{index}"), TimerState::Waiting, Some(600.0)))
        .collect()
}

/// How large each dial is drawn, by its plate's first square, the dials' cubes in their order.
fn sizes(cubes: &[Cube]) -> Vec<f32> {
    cubes
        .iter()
        .filter(|cube| cube.color == [0.016, 0.03, 0.055])
        .step_by(9)
        .map(|cube| cube.scale[0])
        .collect()
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    Vec3::from_array(a).distance(Vec3::from_array(b)) < 1e-4
}

/// `dials` shown for `seconds`, in frames of a twentieth, about the default home, from home's eyes.
fn run(dials: &mut Dials, timers: &[Timer], seconds: f32) -> Vec<Cube> {
    let mut cubes = Vec::new();
    for frame in 0..(seconds * 20.0) as usize {
        cubes.clear();
        dials.update(
            timers,
            true,
            &open(),
            NOW + frame as f64 * 0.05,
            0.05,
            &mut cubes,
        );
    }
    cubes
}

#[test]
fn the_countdown_says_minutes_and_seconds_hours_days_and_when_it_is_due() {
    let left = |seconds: f64| countdown(Some(NOW as u64 + 1_000_000), NOW + 1_000_000.0 - seconds);
    assert_eq!(left(12.0 * 60.0 + 5.0).as_deref(), Some("12:05"));
    assert_eq!(left(59.0 * 60.0 + 59.0).as_deref(), Some("59:59"));
    // Rounded up to the second: never 0:00 before it is due.
    assert_eq!(left(0.2).as_deref(), Some("0:01"));
    assert_eq!(left(3600.0).as_deref(), Some("1h 00m"));
    assert_eq!(
        left(3.0 * 3600.0 + 5.0 * 60.0 + 59.0).as_deref(),
        Some("3h 05m")
    );
    assert_eq!(left(86_399.0).as_deref(), Some("23h 59m"));
    assert_eq!(left(86_400.0).as_deref(), Some("1d 0h"));
    assert_eq!(
        left(2.0 * 86_400.0 + 3.0 * 3600.0).as_deref(),
        Some("2d 3h")
    );
    // Due, or past it as it waits for the agent's turn, or a clock behind the server's by more than is left.
    assert_eq!(left(0.0).as_deref(), Some("due now"));
    assert_eq!(left(-90.0).as_deref(), Some("due now"));
    assert_eq!(countdown(Some(1), f64::NAN).as_deref(), Some("due now"));
    assert_eq!(countdown(None, NOW), None);
    // Far ahead, as from a server's nonsense, it stays bounded.
    assert_eq!(countdown(Some(u64::MAX), 0.0).unwrap(), "11574d 1h");
}

#[test]
fn the_pill_says_the_countdown_or_how_the_timer_stands() {
    let pill_of = |state, left| pill(&timer("t", state, left), NOW);
    assert_eq!(pill_of(TimerState::Waiting, Some(65.0)), "1:05");
    assert_eq!(pill_of(TimerState::Waiting, None), "—");
    assert_eq!(pill_of(TimerState::Unknown, Some(65.0)), "1:05");
    assert_eq!(pill_of(TimerState::Paused, Some(65.0)), "Paused 1:05");
    assert_eq!(pill_of(TimerState::Paused, None), "Paused");
    assert_eq!(pill_of(TimerState::Awaiting, Some(65.0)), "On arrival");
    assert_eq!(pill_of(TimerState::Fired, Some(65.0)), "Fired");
    // Each state its own colour.
    let colours: Vec<[f32; 3]> = [
        TimerState::Waiting,
        TimerState::Paused,
        TimerState::Awaiting,
        TimerState::Fired,
        TimerState::Unknown,
    ]
    .map(color)
    .to_vec();
    for (index, colour) in colours.iter().enumerate() {
        assert!(!colours[..index].contains(colour));
    }
}

#[test]
fn the_sector_fills_with_the_minutes_of_an_hour_smoothly_across_the_hour() {
    assert_eq!(fill(None, NOW, false), 1.0);
    assert_eq!(fill(Some(NOW as u64), NOW, false), 0.0);
    assert_eq!(fill(Some(NOW as u64 - 10), NOW, false), 0.0);
    assert!((fill(Some(NOW as u64 + 900), NOW, false) - 0.25).abs() < 1e-6);
    // Either side of the hour, nearly the same.
    let before = fill(Some(NOW as u64 + 3599), NOW, false);
    let after = fill(Some(NOW as u64 + 3601), NOW, false);
    assert!(after - before < 0.001);
    assert_eq!(fill(Some(NOW as u64 + 13 * 3600), NOW, false), 1.0);
}

#[test]
fn a_timer_set_seconds_ahead_counts_its_seconds_on_the_whole_face_and_its_pill() {
    // Its pill counts minutes and seconds, as 0:27, rounded up, never 0:00 before it is due.
    let next = NOW as u64 + 30;
    assert_eq!(countdown(Some(next), NOW).unwrap(), "0:30");
    assert_eq!(countdown(Some(next), NOW + 2.5).unwrap(), "0:28");
    assert_eq!(countdown(Some(next), NOW + 3.0).unwrap(), "0:27");
    assert_eq!(countdown(Some(next), NOW + 29.9).unwrap(), "0:01");
    assert_eq!(countdown(Some(next), NOW + 30.0).unwrap(), "due now");
    // First seen within a minute of its firing, its face counts a minute: half of it for 30 s, draining as they pass.
    assert!(within_a_minute(Some(next), NOW));
    assert!(within_a_minute(Some(NOW as u64 + 60), NOW));
    assert!(!within_a_minute(Some(NOW as u64 + 61), NOW));
    assert!(!within_a_minute(None, NOW));
    assert!((fill(Some(next), NOW, true) - 0.5).abs() < 1e-6);
    assert!((fill(Some(next), NOW + 15.0, true) - 0.25).abs() < 1e-6);
    assert_eq!(fill(Some(next), NOW + 30.0, true), 0.0);
    assert_eq!(fill(Some(NOW as u64 + 90), NOW, true), 1.0);
    // Otherwise the sliver of an hour.
    assert!(fill(Some(next), NOW, false) < 0.01);
}

#[test]
fn dials_stand_above_the_agent_s_reach_in_rows_of_four_centred_along_the_arc_about_the_audience() {
    let home = Anchor::AGENT;
    let room = open();
    assert!(places(0, &room).is_empty());
    let above = home.position.y + SWARM_REACH.y + CLEAR + RADIUS;
    // One, straight above the agent, its face clear above the swarm's halo and agents.
    let one = places(1, &room);
    assert_eq!(one.len(), 1);
    assert!(one[0].0.distance(Vec3::new(0.0, above, home.position.z)) < 1e-5);
    assert_eq!(one[0].1, 1.0);
    // Eight, two rows of four, the second above the first, each centred over the agent, every dial as far from the
    // audience as the agent and APART along the arc from the next.
    let eight = places(8, &room);
    assert_eq!(eight.len(), 8);
    let radius = |at: Vec3| (at.x - EYES.x).hypot(at.z - EYES.z);
    for (index, (at, scale)) in eight.iter().enumerate() {
        assert_eq!(*scale, 1.0);
        assert!((radius(*at) - radius(home.position)).abs() < 1e-4);
        let row = index / PER_ROW;
        assert!((at.y - (above + row as f32 * ROW_APART)).abs() < 1e-5);
    }
    for row in eight.chunks(PER_ROW) {
        let middle = (row[1].0 + row[2].0) / 2.0;
        assert!(middle.x.abs() < 1e-4, "centred over the agent: {middle}");
        for pair in row.windows(2) {
            let angle = Vec3::new(pair[0].0.x, 0.0, pair[0].0.z - EYES.z).angle_between(Vec3::new(
                pair[1].0.x,
                0.0,
                pair[1].0.z - EYES.z,
            ));
            assert!((angle * radius(home.position) - APART).abs() < 1e-3);
            // Left to right as the audience sees them.
            assert!(pair[0].0.x < pair[1].0.x);
        }
    }
    assert!(room.clear(&eight));
    // Five: a row of four, and one above, centred.
    let five = places(5, &room);
    assert!(five[4].0.x.abs() < 1e-4);
    // Never more than eight.
    assert_eq!(places(20, &room).len(), MOST);
    // Further than REACH, all of it larger in proportion.
    let far = Anchor {
        position: Vec3::new(0.0, 4.0, -6.4),
        ..home
    };
    let (at, scale) = places(1, &Room::open(far, EYES))[0];
    assert!((scale - 2.0).abs() < 1e-5);
    assert!((at.y - (4.0 + SWARM_REACH.y + CLEAR + RADIUS * 2.0)).abs() < 1e-5);
    // With the audience at home, the row runs across the way home faces.
    let turned = Anchor {
        position: Vec3::new(3.0, 1.4, 0.0),
        orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
    };
    let row = places(2, &Room::open(turned, Vec3::new(3.0, 1.6, 0.0)));
    assert!((row[0].0.x - 3.0).abs() < 1e-5 && (row[1].0.x - 3.0).abs() < 1e-5);
    assert!(((row[0].0.z - row[1].0.z).abs() - APART).abs() < 1e-4);
    // A larger avatar's reach lifts them.
    let tall = Room {
        agent: Vec3::splat(1.0),
        ..open()
    };
    assert!((places(1, &tall)[0].0.y - (home.position.y + 1.0 + CLEAR + RADIUS)).abs() < 1e-5);
}

#[test]
fn where_the_room_above_is_taken_they_stand_smaller_or_beneath_the_agent() {
    let home = Anchor::AGENT;
    let room = open();
    let rows = places(8, &room);
    // Something above the second row's place, and a ceiling over it: one row of eight, smaller.
    let ceiling = Room {
        ceiling: Some(rows[4].0.y),
        ..open()
    };
    let one_row = places(8, &ceiling);
    assert!(
        one_row
            .iter()
            .all(|(at, scale)| *scale == 0.7 && (at.y - one_row[0].0.y).abs() < 1e-5)
    );
    assert!(ceiling.clear(&one_row));
    // Nothing fits above, as under a ceiling just over the agent: beneath it, its signs clear below its reach, at their
    // own size where there is room down to the floor, or else at 0.7, or else at half.
    let low = |height: f32| {
        let home = Anchor {
            position: Vec3::new(0.0, height, -2.3),
            ..home
        };
        Room {
            ceiling: Some(height + 0.5),
            ..Room::open(home, EYES)
        }
    };
    let room = low(1.7);
    let beneath = places(3, &room);
    for (at, scale) in &beneath {
        let dial = Extent::dial(*at, *scale);
        assert!(dial.high <= 1.7 - SWARM_REACH.y - CLEAR + 1e-4);
        assert!(dial.low >= room.floor - 1e-4);
    }
    assert_eq!(beneath[0].1, 1.0);
    assert_eq!(places(3, &low(1.5))[0].1, 0.7);
    assert_eq!(places(3, &low(1.4))[0].1, 0.5);
    // Nowhere clear: as above, at their own size.
    let nowhere = Room {
        ceiling: Some(0.0),
        ..open()
    };
    assert_eq!(places(3, &nowhere), places(3, &open()));
    // Something in the way of the first row moves them on too.
    let worker = Extent::worker(rows[0].0, 1.0, EYES);
    let crowded = Room {
        extents: vec![worker],
        ..open()
    };
    assert!(!crowded.clear(&rows));
    let moved = places(8, &crowded);
    assert!(crowded.clear(&moved));
    assert_ne!(moved, rows);
    // Two dials in one place overlap one another.
    assert!(!open().clear(&[rows[0], rows[0]]));
    // Extents overlap only where they share room across and up.
    let a = Extent::dial(Vec3::ZERO, 1.0);
    assert!(a.overlaps(&Extent::dial(Vec3::X * 0.5, 1.0), 0.0));
    assert!(!a.overlaps(&Extent::dial(Vec3::X * 0.7, 1.0), 0.0));
    assert!(!a.overlaps(&Extent::dial(Vec3::Y * 0.6, 1.0), 0.0));
    // A worker far from whoever watches has its sign drawn larger.
    assert!(
        Extent::worker(Vec3::new(0.0, 1.4, -6.4), 1.0, EYES).radius
            > Extent::worker(rows[0].0, 1.0, EYES).radius * 1.9
    );
}

#[test]
fn a_dial_grows_in_pulses_as_it_fires_glides_as_others_go_and_shrinks_away() {
    let mut dials = Dials::default();
    let mut cubes = Vec::new();
    let three = [
        timer("a", TimerState::Waiting, Some(600.0)),
        timer("b", TimerState::Waiting, Some(600.0)),
        timer("c", TimerState::Waiting, Some(600.0)),
    ];
    dials.update(&three, true, &open(), NOW, 0.05, &mut cubes);
    assert!(dials.dials()[0].opacity < 0.2, "growing in");
    run(&mut dials, &three, 1.0);
    assert!(dials.dials().iter().all(|dial| dial.opacity == 1.0));
    let names: Vec<&str> = dials
        .dials()
        .iter()
        .map(|dial| dial.name.as_str())
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
    let size = |dials: &Dials, cubes: &[Cube], name: &str| {
        let index = dials
            .dials()
            .iter()
            .position(|dial| dial.name == name)
            .unwrap();
        sizes(cubes)[index]
    };
    let before = run(&mut dials, &three, 0.05);
    let resting = size(&dials, &before, "b");
    // "b" fires: it swells, once, and settles.
    let fired = [
        three[0].clone(),
        timer("b", TimerState::Fired, Some(3600.0)),
        three[2].clone(),
    ];
    let swelling = run(&mut dials, &fired, PULSE_SECONDS / 2.0);
    assert!(size(&dials, &swelling, "b") > resting * 1.2);
    let settled = run(&mut dials, &fired, PULSE_SECONDS);
    assert!((size(&dials, &settled, "b") - resting).abs() < 1e-5);
    // Still fired, it does not pulse again.
    let still = run(&mut dials, &fired, 0.3);
    assert!((size(&dials, &still, "b") - resting).abs() < 1e-5);
    // "a" is cancelled: it shrinks away, and the others glide to their places for two.
    let two = [fired[1].clone(), fired[2].clone()];
    run(&mut dials, &two, 0.2);
    assert_eq!(dials.dials().len(), 3);
    let leaving = dials.dials().last().unwrap();
    assert_eq!(leaving.name, "a");
    assert!(leaving.opacity < 1.0);
    run(&mut dials, &two, 2.0);
    let places = places(2, &open());
    let shown: Vec<Vec3> = dials
        .dials()
        .iter()
        .map(|dial| Vec3::from_array(dial.position))
        .collect();
    assert_eq!(shown, [places[0].0, places[1].0]);
    // Leaving the space and joining again within the fired minute, as on reconnecting, the firing does not pulse again.
    dials.clear();
    assert!(dials.dials().is_empty());
    let again = run(&mut dials, &fired, 0.6);
    assert!((size(&dials, &again, "b") - resting).abs() < 1e-5);
    // Its next firing does.
    let next = [
        fired[0].clone(),
        timer("b", TimerState::Waiting, Some(3600.0)),
        fired[2].clone(),
    ];
    run(&mut dials, &next, 0.2);
    let firing = [
        fired[0].clone(),
        timer("b", TimerState::Fired, Some(7200.0)),
        fired[2].clone(),
    ];
    let pulsing = run(&mut dials, &firing, PULSE_SECONDS / 2.0);
    assert!(size(&dials, &pulsing, "b") > resting * 1.2);
    // A firing learnt of only as the space's state is current again, as it fired while this client was disconnected,
    // does not pulse.
    run(&mut dials, &next, PULSE_SECONDS);
    let mut cubes = Vec::new();
    dials.update(&next, false, &open(), NOW, 0.05, &mut cubes);
    dials.update(&firing, true, &open(), NOW, 0.05, &mut cubes);
    let quiet = run(&mut dials, &firing, 0.5);
    assert!((size(&dials, &quiet, "b") - resting).abs() < 1e-5);
    // A one-off set again under the same name, with no firing ahead each time it fires, pulses each time.
    for _ in 0..2 {
        run(
            &mut dials,
            &[timer("once", TimerState::Waiting, Some(60.0))],
            0.2,
        );
        let once = run(
            &mut dials,
            &[timer("once", TimerState::Fired, None)],
            PULSE_SECONDS / 2.0,
        );
        assert!(sizes(&once)[0] > RADIUS * 1.75 * 1.2);
        run(
            &mut dials,
            &[timer("once", TimerState::Fired, None)],
            PULSE_SECONDS,
        );
    }
    // One first seen just fired, as by a client joining within the minute, does not pulse: it did not see it fire.
    let mut fresh = Dials::default();
    let first = run(&mut fresh, &[timer("x", TimerState::Fired, None)], 0.6);
    assert!((sizes(&first)[0] - RADIUS * 1.75).abs() < 1e-5);
}

#[test]
fn at_most_eight_dials_each_name_once_cut_to_their_bounds() {
    let mut timers = named(12);
    timers.insert(1, timer("t0", TimerState::Paused, None));
    timers.insert(2, timer("", TimerState::Waiting, None));
    timers[0].name = "x".repeat(60);
    timers[0].schedule = "y".repeat(200);
    let mut dials = Dials::default();
    let cubes = run(&mut dials, &timers, 1.0);
    assert_eq!(dials.dials().len(), MOST);
    assert!(cubes.len() <= MOST * 80);
    let first = &dials.dials()[0].clone();
    assert_eq!(first.name.chars().count(), NAME_CHARS);
    assert_eq!(first.schedule.chars().count(), WORDS_CHARS);
    assert!(dials.dials().iter().all(Dial::valid));
    assert!(cubes.iter().all(Cube::valid));
    // The name cut to its bounds is the same dial from frame to frame.
    let again = run(&mut dials, &timers, 0.05);
    assert_eq!(again.len(), cubes.len());
    // Bounds a spectator checks.
    let mut dial = first.clone();
    assert!(dial.valid());
    dial.scale = f32::NAN;
    assert!(!dial.valid());
    dial = first.clone();
    dial.name.clear();
    assert!(!dial.valid());
    dial = first.clone();
    dial.position[1] = 1000.0;
    assert!(!dial.valid());
    dial = first.clone();
    dial.opacity = 2.0;
    assert!(!dial.valid());
    dial = first.clone();
    dial.pill = "p".repeat(40);
    assert!(!dial.valid());
    // A serialised dial reads back the same, and one not pointed at says nothing of it.
    let encoded = serde_json::to_value(first).unwrap();
    assert!(encoded.get("pointed").is_none());
    assert_eq!(serde_json::from_value::<Dial>(encoded).unwrap(), *first);
}

#[test]
fn each_state_has_a_face_of_its_own_and_the_seconds_go_round_smoothly() {
    let face = |state, left| {
        let mut dials = Dials::default();
        run(&mut dials, &[timer("t", state, left)], 1.0)
    };
    let waiting = face(TimerState::Waiting, Some(630.0));
    let paused = face(TimerState::Paused, Some(600.0));
    let awaiting = face(TimerState::Awaiting, None);
    let fired = face(TimerState::Fired, Some(600.0));
    // Plate, sixty marks, then: a hub, a hand and the seconds' mote; two bars; a breathing hub; a hub and a hand.
    assert_eq!(waiting.len(), 9 + 60 + 3);
    assert_eq!(paused.len(), 9 + 60 + 2);
    assert_eq!(awaiting.len(), 9 + 60 + 1);
    assert_eq!(fired.len(), 9 + 60 + 2);
    // Waiting ten minutes, a sixth of the marks are lit, the rest dim.
    let lit = color(TimerState::Waiting);
    let marks = &waiting[9..69];
    assert!(marks[..10].iter().all(|mark| near(mark.color, lit)));
    assert!(marks[11..].iter().all(|mark| !near(mark.color, lit)));
    // Paused, grey; waiting for an arrival and fired, all lit.
    assert!(
        paused[9..69]
            .iter()
            .any(|mark| near(mark.color, color(TimerState::Paused)))
    );
    assert!(
        awaiting[9..69]
            .iter()
            .all(|mark| near(mark.color, color(TimerState::Awaiting)))
    );
    // The mote moves a little each frame, as the seconds go.
    let mut dials = Dials::default();
    let timers = [timer("t", TimerState::Waiting, Some(600.0))];
    let one = run(&mut dials, &timers, 0.05);
    let mut two = Vec::new();
    dials.update(&timers, true, &open(), NOW + 0.1, 0.05, &mut two);
    let (a, b) = (
        Vec3::from_array(one.last().unwrap().position),
        Vec3::from_array(two.last().unwrap().position),
    );
    assert!(a.distance(b) > 0.0 && a.distance(b) < 0.01);
    // First seen 30 s from its firing, its face counts a minute: half of it lit after the second it ran, its hand
    // counting the seconds, no mote.
    let mut short = Dials::default();
    let thirty = [timer("plank", TimerState::Waiting, Some(31.0))];
    let seconds = run(&mut short, &thirty, 1.0);
    assert_eq!(seconds.len(), 9 + 60 + 2);
    let marks = &seconds[9..69];
    assert!(marks[..29].iter().all(|mark| near(mark.color, lit)));
    assert!(marks[31..].iter().all(|mark| !near(mark.color, lit)));
    // Set again for longer, it counts the hour again, with its mote; and set again seconds ahead, a minute once more.
    let longer = [timer("plank", TimerState::Waiting, Some(600.0))];
    assert_eq!(run(&mut short, &longer, 0.05).len(), 9 + 60 + 3);
    assert_eq!(run(&mut short, &thirty, 0.05).len(), 9 + 60 + 2);
    // A full scene of cubes leaves the dials out rather than overflowing.
    let mut full = vec![waiting[0]; crate::delegation::MAX_CUBES];
    dials.update(&timers, true, &open(), NOW, 0.05, &mut full);
    assert_eq!(full.len(), crate::delegation::MAX_CUBES);
    // A home beyond the scene's bounds draws nothing there.
    let mut lost = Dials::default();
    let mut cubes = Vec::new();
    let beyond = Anchor {
        position: Vec3::new(500.0, 1.4, 0.0),
        ..Anchor::AGENT
    };
    lost.update(
        &timers,
        true,
        &Room::open(beyond, EYES),
        NOW,
        0.05,
        &mut cubes,
    );
    assert!(cubes.is_empty());
    // Nonsense time steps stand still.
    let mut cubes = Vec::new();
    dials.update(&timers, true, &open(), NOW, f32::NAN, &mut cubes);
    assert!(!cubes.is_empty());
}

#[test]
fn the_ray_points_at_the_nearest_live_dial_where_it_passes_nearest_its_middle() {
    let mut dials = Dials::default();
    let timers = named(3);
    run(&mut dials, &timers, 1.0);
    let middle = Vec3::from_array(dials.dials()[1].position);
    let ray = Ray::new(EYES, middle - EYES);
    let (distance, point) = dials.point(ray).unwrap();
    assert_eq!(dials.pointed(), Some("t1"));
    assert!(point.distance(middle) < 1e-3);
    assert!(distance < EYES.distance(middle));
    assert_eq!(dials.hit_point(), Some(point));
    // Along the row, through every dial, the nearest.
    let first = Vec3::from_array(dials.dials()[0].position);
    let along = first - (middle - first) * 3.0;
    dials.point(Ray::new(along, middle - first));
    assert_eq!(dials.pointed(), Some("t0"));
    // Within its pointing sphere, a little off.
    let off = middle + Vec3::X * (POINTING_RADIUS * 0.8);
    dials.point(Ray::new(EYES, off - EYES));
    assert_eq!(dials.pointed(), Some("t1"));
    // Above them all, or with nothing, none.
    assert!(dials.point(Ray::new(EYES, Vec3::Y)).is_none());
    assert_eq!(dials.pointed(), None);
    assert!(dials.point(None).is_none());
    assert_eq!(dials.hit_point(), None);
    // Pointed at, it is drawn paler and larger.
    dials.point(ray);
    let highlighted = run(&mut dials, &timers, 0.05);
    assert!(dials.dials()[1].pointed);
    let plain = {
        dials.point(None);
        run(&mut dials, &timers, 0.05)
    };
    assert!(sizes(&highlighted)[1] > sizes(&plain)[1] * 1.2);
    // Far off, its sphere grows with the distance, as a worker's does.
    let mut distant = Dials::default();
    let home = Anchor {
        position: Anchor::AGENT.position * 4.0,
        ..Anchor::AGENT
    };
    let mut cubes = Vec::new();
    for _ in 0..30 {
        distant.update(
            &timers[..1],
            true,
            &Room::open(home, EYES),
            NOW,
            0.05,
            &mut cubes,
        );
    }
    let centre = Vec3::from_array(distant.dials()[0].position);
    let beside = centre + Vec3::X * POINTING_RADIUS * 1.5;
    assert!(distant.point(Ray::new(EYES, beside - EYES)).is_some());
    // A leaving dial cannot be pointed at.
    run(&mut dials, &timers[..2], 0.1);
    let leaving = Vec3::from_array(dials.dials()[2].position);
    assert!(dials.point(Ray::new(EYES, leaving - EYES)).is_none());
}

#[test]
fn a_timer_set_again_says_its_new_words() {
    let mut dials = Dials::default();
    run(
        &mut dials,
        &[timer("t", TimerState::Waiting, Some(600.0))],
        0.1,
    );
    let mut again = timer("t", TimerState::Waiting, Some(600.0));
    again.schedule = "every 30 min".into();
    again.condition = "always".into();
    run(&mut dials, &[again], 0.1);
    assert_eq!(dials.dials()[0].schedule, "every 30 min");
    assert_eq!(dials.dials()[0].condition, "always");
}
