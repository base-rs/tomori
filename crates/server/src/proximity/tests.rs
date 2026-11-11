// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::Pose;

/// The station `name` at `x` along the trail, telling the agent within 2 m.
fn station(name: &str, x: f32) -> Anchor {
    Anchor::new(name, Pose::at([x, 0.0, -10.0]))
        .with_kind("station")
        .with_label(format!("Station {}: 10 \"squats\"", &name[8..]))
        .notify_within(2.0)
}

/// Samuel, their head at `x` along the trail, 1.6 m up.
fn samuel(x: f32) -> Person<'static> {
    Person {
        key: "p1",
        name: "Samuel",
        at: [x, 1.6, -10.0],
    }
}

#[test]
fn coming_within_an_anchor_is_told_once_as_its_label_and_kind_say() {
    let mut proximity = Proximity::default();
    let start = Instant::now();
    let anchors = [station("station-1", 0.0), Anchor::new("pen", Pose::HOME)];
    assert!(
        proximity
            .observe(start, &[samuel(-5.0)], &anchors)
            .is_empty()
    );
    // Across the floor, whatever the heights.
    let notes = proximity.observe(start, &[samuel(-1.9)], &anchors);
    assert_eq!(
        notes,
        [(
            "p1/station-1".to_owned(),
            "Samuel came within 2 m of station-1 \"Station 1: 10 'squats'\" (kind station)"
                .to_owned()
        )]
    );
    assert_eq!(proximity.within("station-1"), ["Samuel"]);
    // Staying, or moving about inside, tells nothing more.
    for x in [-1.0, 0.0, 1.5] {
        assert!(proximity.observe(start, &[samuel(x)], &anchors).is_empty());
    }
}

#[test]
fn an_anchor_without_a_radius_tells_nothing() {
    let mut proximity = Proximity::default();
    let pen = Anchor::new("pen", Pose::HOME).with_kind("pen");
    let ada = Person {
        key: "p2",
        name: "Ada",
        at: [0.0, 1.6, 0.0],
    };
    assert!(proximity.observe(Instant::now(), &[ada], &[pen]).is_empty());
    assert!(proximity.within("pen").is_empty());
}

#[test]
fn someone_at_the_edge_stays_within_until_half_a_metre_beyond_it() {
    let mut proximity = Proximity::default();
    let start = Instant::now();
    let anchors = [station("station-1", 0.0)];
    assert_eq!(proximity.observe(start, &[samuel(2.0)], &anchors).len(), 1);
    // Wandering out to 2.5 m and back is still within: no note, though the cooldown has long passed.
    let later = start + COOLDOWN * 2;
    for x in [2.4, 2.5, 1.9, 2.5, 2.0] {
        assert!(proximity.observe(later, &[samuel(x)], &anchors).is_empty());
        assert_eq!(proximity.within("station-1"), ["Samuel"]);
    }
    // Beyond it, they have left; coming back is told again.
    assert!(
        proximity
            .observe(later, &[samuel(2.6)], &anchors)
            .is_empty()
    );
    assert!(proximity.within("station-1").is_empty());
    assert!(
        proximity
            .observe(later, &[samuel(2.1)], &anchors)
            .is_empty()
    );
    assert_eq!(proximity.observe(later, &[samuel(1.9)], &anchors).len(), 1);
}

#[test]
fn coming_back_within_the_cooldown_is_not_told_again() {
    let mut proximity = Proximity::default();
    let start = Instant::now();
    let anchors = [station("station-1", 0.0)];
    assert_eq!(proximity.observe(start, &[samuel(0.0)], &anchors).len(), 1);
    assert!(
        proximity
            .observe(start, &[samuel(5.0)], &anchors)
            .is_empty()
    );
    let soon = start + COOLDOWN - Duration::from_secs(1);
    assert!(proximity.observe(soon, &[samuel(0.0)], &anchors).is_empty());
    // Within it all the same.
    assert_eq!(proximity.within("station-1"), ["Samuel"]);
    assert!(proximity.observe(soon, &[samuel(5.0)], &anchors).is_empty());
    let after = start + COOLDOWN;
    assert_eq!(proximity.observe(after, &[samuel(0.0)], &anchors).len(), 1);
    // A different station is its own.
    let both = [station("station-1", 0.0), station("station-2", 3.0)];
    assert_eq!(
        proximity.observe(after, &[samuel(1.5)], &both),
        [(
            "p1/station-2".to_owned(),
            "Samuel came within 2 m of station-2 \"Station 2: 10 'squats'\" (kind station)"
                .to_owned()
        )]
    );
}

#[test]
fn each_participant_is_told_of_on_their_own() {
    let mut proximity = Proximity::default();
    let start = Instant::now();
    let anchors = [station("station-1", 0.0)];
    let ada = |x: f32| Person {
        key: "p2",
        name: "Ada",
        at: [x, 1.5, -10.0],
    };
    let notes = proximity.observe(start, &[samuel(0.0), ada(1.0)], &anchors);
    let keys: Vec<&str> = notes.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["p1/station-1", "p2/station-1"]);
    assert!(notes[1].1.starts_with("Ada came within 2 m of station-1"));
    assert_eq!(proximity.within("station-1"), ["Samuel", "Ada"]);
    // Ada leaves; Samuel stays, and is not told again as she comes back.
    assert!(
        proximity
            .observe(start, &[samuel(0.0), ada(9.0)], &anchors)
            .is_empty()
    );
    assert_eq!(proximity.within("station-1"), ["Samuel"]);
    // Someone gone from the space is no longer within, and coming back within the cooldown is not told.
    assert!(proximity.observe(start, &[], &anchors).is_empty());
    assert!(proximity.within("station-1").is_empty());
    assert!(
        proximity
            .observe(start, &[samuel(0.0)], &anchors)
            .is_empty()
    );
}

#[test]
fn an_anchor_placed_or_removed_while_someone_is_within_it() {
    let mut proximity = Proximity::default();
    let start = Instant::now();
    assert!(proximity.observe(start, &[samuel(0.0)], &[]).is_empty());
    // Placed where they stand: they are within it now, and told.
    let placed = [station("station-1", 0.5)];
    assert_eq!(proximity.observe(start, &[samuel(0.0)], &placed).len(), 1);
    // Removed, nobody is within it; placed again within the cooldown, they are within it, untold.
    assert!(proximity.observe(start, &[samuel(0.0)], &[]).is_empty());
    assert!(proximity.within("station-1").is_empty());
    assert!(proximity.observe(start, &[samuel(0.0)], &placed).is_empty());
    assert_eq!(proximity.within("station-1"), ["Samuel"]);
    // Moved off from them, beyond the hysteresis, they left it.
    let moved = [station("station-1", 3.0)];
    assert!(proximity.observe(start, &[samuel(0.0)], &moved).is_empty());
    assert!(proximity.within("station-1").is_empty());
    // And placed again after the cooldown, they are told again.
    let after = start + COOLDOWN;
    assert_eq!(proximity.observe(after, &[samuel(0.0)], &placed).len(), 1);
}
