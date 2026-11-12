// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::Duration;

use super::*;

/// What the side is told, by zone and count.
#[derive(Default)]
struct Told(Vec<(String, u32)>);

impl Told {
    fn tell(&mut self) -> impl FnMut(Occupancy) -> bool + '_ {
        |occupancy| {
            self.0
                .push((occupancy.zone().to_owned(), occupancy.count()));
            true
        }
    }

    fn take(&mut self) -> Vec<(String, u32)> {
        std::mem::take(&mut self.0)
    }
}

fn told(zone: &str, count: u32) -> Vec<(String, u32)> {
    vec![(zone.to_owned(), count)]
}

fn head(participant: u32, x: f32) -> Head {
    Head {
        participant,
        at: [x, 1.6, 0.0],
    }
}

/// Where an anchor or object stands, by name.
type Finding = dyn Fn(&str) -> Option<[f32; 3]>;

fn nowhere() -> Centres<'static> {
    Centres::nowhere()
}

/// A console 2 m across from the origin, its band from the floor to 2.5 m up, entered at 2 m and left beyond 2.5 m.
fn console(dwell: Duration) -> Zones {
    let zone = Zone::cylinder("console", Around::point([0.0; 3]), 2.0, 0.0, 2.5).dwelling(dwell);
    Zones::declare(vec![zone], Some).unwrap()
}

#[test]
fn a_zone_is_told_as_it_is_declared_and_then_as_someone_enters_and_leaves_beyond_its_margin() {
    let mut zones = console(Duration::ZERO);
    let mut side = Told::default();
    let now = Instant::now();
    // Told at once, though nobody is there.
    zones.observe(now, &[head(1, 3.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 0));
    // Nothing more until someone enters, at its radius.
    zones.observe(now, &[head(1, 2.1)], nowhere(), side.tell());
    assert!(side.take().is_empty());
    zones.observe(now, &[head(1, 2.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    // Wandering at its edge, within its margin, tells nothing.
    for x in [2.4, 1.0, 2.5, 2.2] {
        zones.observe(now, &[head(1, x)], nowhere(), side.tell());
    }
    assert!(side.take().is_empty());
    assert_eq!(zones.count("console"), Some(1));
    // Beyond it, they have left.
    zones.observe(now, &[head(1, 2.6)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 0));
    // Coming back to the margin is not entering.
    zones.observe(now, &[head(1, 2.3)], nowhere(), side.tell());
    assert!(side.take().is_empty());
    // Out of its band, above it, they are not within it.
    zones.observe(
        now,
        &[Head {
            participant: 1,
            at: [0.0, 3.2, 0.0],
        }],
        nowhere(),
        side.tell(),
    );
    assert!(side.take().is_empty());
}

#[test]
fn a_change_between_nobody_and_somebody_waits_for_the_zone_s_dwell() {
    let dwell = Duration::from_secs(3);
    let mut zones = console(dwell);
    let mut side = Told::default();
    let start = Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    zones.observe(at(0), &[], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 0));
    // Passing through, for less than the dwell, tells nothing.
    zones.observe(at(1), &[head(1, 0.0)], nowhere(), side.tell());
    zones.observe(at(3), &[head(1, 0.0)], nowhere(), side.tell());
    zones.observe(at(4), &[], nowhere(), side.tell());
    assert!(side.take().is_empty());
    // Staying, it is told once the dwell has passed since they came.
    zones.observe(at(5), &[head(1, 0.0)], nowhere(), side.tell());
    zones.observe(at(7), &[head(1, 0.0)], nowhere(), side.tell());
    assert!(side.take().is_empty());
    zones.observe(at(8), &[head(1, 0.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    // A second participant arriving is told at once, as the count changes and it stays occupied.
    zones.observe(at(9), &[head(1, 0.0), head(2, 1.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 2));
    // One stepping out, with the other still there, at once too.
    zones.observe(at(10), &[head(2, 1.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    // Stepping out for a moment tells nothing; leaving for the dwell is told.
    zones.observe(at(11), &[], nowhere(), side.tell());
    zones.observe(at(12), &[head(2, 1.0)], nowhere(), side.tell());
    zones.observe(at(13), &[], nowhere(), side.tell());
    zones.observe(at(15), &[], nowhere(), side.tell());
    assert!(side.take().is_empty());
    zones.observe(at(16), &[], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 0));
}

#[test]
fn several_participants_are_counted_each_once_by_their_own_margins() {
    let mut zones = console(Duration::ZERO);
    let mut side = Told::default();
    let now = Instant::now();
    zones.observe(
        now,
        &[head(1, 0.0), head(2, 1.9), head(3, 2.4), head(1, 0.5)],
        nowhere(),
        side.tell(),
    );
    // Ada counted once, Ben within, Cy within its margin only and so not entering.
    assert_eq!(side.take(), told("console", 2));
    // Ben drifts into the margin, staying; Cy steps in.
    zones.observe(
        now,
        &[head(1, 0.0), head(2, 2.4), head(3, 2.0)],
        nowhere(),
        side.tell(),
    );
    assert_eq!(side.take(), told("console", 3));
    // Someone no longer among those looked at, as one gone or whose report went stale, is no longer within it.
    zones.observe(now, &[head(2, 2.4), head(3, 2.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 2));
}

#[test]
fn a_zone_follows_its_anchor_or_object_and_holds_nobody_where_it_is_not() {
    let zone = Zone::sphere("rack", Around::anchor("rack/a12"), 1.0);
    let hen = Zone::sphere("pip", Around::object("chicken-1"), 1.0);
    let mut zones = Zones::declare(vec![zone, hen], Some).unwrap();
    assert!(zones.around_anchors());
    let mut side = Told::default();
    let now = Instant::now();
    let racks: [&Finding; 3] = [
        &|name| (name == "rack/a12").then_some([4.0, 1.6, 0.0]),
        &|name| (name == "rack/a12").then_some([6.0, 1.6, 0.0]),
        &|name| (name == "rack/a12").then_some([4.2, 1.6, 0.0]),
    ];
    let hens: &Finding = &|id| (id == "chicken-1").then_some([9.0, 1.6, 0.0]);
    let rack_at = |index: usize| Centres {
        anchor: racks[index],
        object: hens,
    };
    zones.observe(now, &[head(1, 4.0)], rack_at(0), side.tell());
    assert_eq!(
        side.take(),
        vec![("rack".to_owned(), 1), ("pip".to_owned(), 0)]
    );
    // The rack moves away from them, beyond its margin.
    zones.observe(now, &[head(1, 4.0)], rack_at(1), side.tell());
    assert_eq!(side.take(), told("rack", 0));
    // Moved onto them, they are within it; gone, nobody is.
    zones.observe(now, &[head(1, 4.0)], rack_at(2), side.tell());
    assert_eq!(side.take(), told("rack", 1));
    zones.observe(now, &[head(1, 4.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("rack", 0));
    // Someone by the hen is within hers, wherever she is described.
    zones.observe(now, &[head(2, 9.0)], rack_at(0), side.tell());
    assert_eq!(side.take(), told("pip", 1));
    // A zone about a point is not about an anchor.
    assert!(!console(Duration::ZERO).around_anchors());
}

#[test]
fn declaring_again_tells_each_zone_as_it_is_and_forgets_the_rest() {
    let mut side = Told::default();
    let now = Instant::now();
    let mut zones = console(Duration::from_secs(30));
    zones.observe(now, &[head(1, 0.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    // Declared again, as a terminal changes, with another zone: each is told as it is, without waiting for its dwell.
    let well = Zone::sphere("well", Around::point([10.0, 1.6, 0.0]), 1.0);
    let console = Zone::cylinder("console", Around::point([0.0; 3]), 2.0, 0.0, 2.5)
        .dwelling(Duration::from_secs(30));
    let mut zones = Zones::declare(vec![well, console], Some).unwrap();
    zones.observe(now, &[head(1, 0.0)], nowhere(), side.tell());
    assert_eq!(
        side.take(),
        vec![("well".to_owned(), 0), ("console".to_owned(), 1)]
    );
    // Declaring none tells nothing.
    let mut zones = Zones::declare(Vec::new(), Some).unwrap();
    zones.observe(now, &[head(1, 0.0)], nowhere(), side.tell());
    assert!(side.take().is_empty());
}

#[test]
fn a_declaration_is_bounded_and_its_points_carried() {
    let refused =
        Zones::declare(vec![Zone::sphere("a", Around::point([0.0; 3]), 0.1)], Some).unwrap_err();
    assert!(refused.contains("radius"), "{refused}");
    let far = Zone::sphere("far", Around::point([30.0, 0.0, 0.0]), 1.0);
    let refused = Zones::declare(vec![far], |_| None).unwrap_err();
    assert_eq!(
        refused,
        "The zone \"far\" is around a point more than 20 m from home."
    );
    // Its point is where the carriage puts it.
    let well = Zone::sphere("well", Around::point([0.0; 3]), 1.0);
    let mut zones = Zones::declare(vec![well], |[x, y, z]| Some([x + 5.0, y, z])).unwrap();
    let mut side = Told::default();
    zones.observe(
        Instant::now(),
        &[Head {
            participant: 1,
            at: [5.0, 0.0, 0.0],
        }],
        nowhere(),
        side.tell(),
    );
    assert_eq!(side.take(), told("well", 1));
}

#[test]
fn only_the_first_heads_up_to_the_limit_are_counted() {
    let mut zones = console(Duration::ZERO);
    let mut side = Told::default();
    let heads: Vec<Head> = (0..PRESENCE_LIMIT as u32 + 8)
        .map(|participant| head(participant, 0.0))
        .collect();
    zones.observe(Instant::now(), &heads, nowhere(), side.tell());
    assert_eq!(side.take(), told("console", PRESENCE_LIMIT as u32));
}

#[test]
fn an_occupancy_the_side_could_not_be_told_is_told_at_a_later_look() {
    let mut zones = console(Duration::ZERO);
    let now = Instant::now();
    let mut refused = 0;
    zones.observe(now, &[], nowhere(), |_| {
        refused += 1;
        false
    });
    zones.observe(now, &[head(1, 0.0)], nowhere(), |_| {
        refused += 1;
        false
    });
    assert_eq!(refused, 2);
    let mut side = Told::default();
    zones.observe(now, &[head(1, 0.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    zones.observe(now, &[head(1, 0.0)], nowhere(), side.tell());
    assert!(side.take().is_empty());
}

#[test]
fn declaring_again_carries_who_is_within_a_zone_that_keeps_its_name() {
    let start = Instant::now();
    let mut declared = Declared::default();
    assert!(!declared.any());
    declared.declare(console(Duration::ZERO), start);
    assert!(declared.any());
    let mut side = Told::default();
    let zones = declared.current().unwrap();
    zones.observe(start, &[head(1, 1.0)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    // Drifting into the margin, they stay within it.
    zones.observe(start, &[head(1, 2.3)], nowhere(), side.tell());
    assert!(side.take().is_empty());
    // Declared again, as its terminal changes, the console keeps them within its margin, and is told as it is.
    let later = start + ZONE_INTERVAL;
    declared.declare(console(Duration::ZERO), later);
    let zones = declared.current().unwrap();
    zones.observe(later, &[head(1, 2.3)], nowhere(), side.tell());
    assert_eq!(side.take(), told("console", 1));
    // A zone of another name carries nobody: at 2.3 m they have not entered it.
    let well = Zone::cylinder("well", Around::point([0.0; 3]), 2.0, 0.0, 2.5);
    let again = later + ZONE_INTERVAL;
    declared.declare(Zones::declare(vec![well], Some).unwrap(), again);
    let zones = declared.current().unwrap();
    zones.observe(again, &[head(1, 2.3)], nowhere(), side.tell());
    assert_eq!(side.take(), told("well", 0));
}

#[test]
fn a_declaration_sooner_than_the_interval_waits_the_latest_replacing_the_one_before() {
    let start = Instant::now();
    let mut declared = Declared::default();
    let named = |name: &str| {
        Zones::declare(vec![Zone::sphere(name, Around::point([0.0; 3]), 1.0)], Some).unwrap()
    };
    declared.declare(named("first"), start);
    declared.declare(named("second"), start + Duration::from_millis(100));
    declared.declare(named("third"), start + Duration::from_millis(200));
    let mut side = Told::default();
    let look = |declared: &mut Declared, side: &mut Told, at: Instant| {
        declared.due(at);
        declared
            .current()
            .unwrap()
            .observe(at, &[], nowhere(), side.tell());
    };
    look(&mut declared, &mut side, start + Duration::from_millis(500));
    assert_eq!(side.take(), told("first", 0));
    // Once the interval has passed, the latest is taken, and told.
    look(&mut declared, &mut side, start + ZONE_INTERVAL);
    assert_eq!(side.take(), told("third", 0));
    look(&mut declared, &mut side, start + 3 * ZONE_INTERVAL);
    assert!(side.take().is_empty());
}
