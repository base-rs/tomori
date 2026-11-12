// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn kinds_and_marks_are_read_in_any_case_and_named_as_written() {
    for (word, kind) in [
        ("object", Kind::Object),
        ("ANCHOR", Kind::Anchor),
        ("wall", Kind::Wall),
        ("Panel", Kind::Panel),
        ("worker", Kind::Worker),
        ("person", Kind::Person),
        ("participant", Kind::Person),
        ("agent", Kind::Agent),
    ] {
        assert_eq!(Kind::parse(word), Some(kind));
        assert_eq!(Kind::parse(kind.name()), Some(kind));
    }
    assert_eq!(Kind::parse("rack"), None);
    assert_eq!(Kind::Unknown.name(), "thing");
    for (word, mark) in [
        ("ring", Mark::Ring),
        ("Halo", Mark::Halo),
        ("outline", Mark::Outline),
        ("frame", Mark::Outline),
        ("none", Mark::Unmarked),
    ] {
        assert_eq!(Mark::parse(word), Some(mark));
    }
    assert_eq!(Mark::parse("glow"), None);
    // By its kind: a ring about an object or an anchor, a frame on a panel or a wall, a halo above anything else.
    for (kind, mark) in [
        (Kind::Object, Mark::Ring),
        (Kind::Anchor, Mark::Ring),
        (Kind::Panel, Mark::Outline),
        (Kind::Wall, Mark::Outline),
        (Kind::Worker, Mark::Halo),
        (Kind::Person, Mark::Halo),
        (Kind::Agent, Mark::Halo),
        (Kind::Unknown, Mark::Halo),
    ] {
        assert_eq!(Mark::ByKind.on(kind), mark);
    }
    assert_eq!(Mark::Unmarked.on(Kind::Object), Mark::Unmarked);
}

#[test]
fn a_reference_is_kept_as_written_cleaned_and_cut() {
    let long = Reference::new(Kind::Person, &format!("{}é", "a".repeat(NAME_BYTES - 1)));
    assert_eq!(long.name.len(), NAME_BYTES - 1);
    let lined = Reference::new(Kind::Panel, "Trial\nchecklist");
    assert_eq!(lined.to_string(), "panel Trial checklist");
}

#[test]
fn a_target_has_a_compact_wire_format_and_reads_newer_values_as_defaults() {
    let mut fire = Target::new(
        "fire",
        Reference::new(Kind::Object, "farmyard:campsite/bonfire"),
    );
    fire.label = "The bonfire".into();
    fire.state = State::Here;
    fire.at = Some([1.0, 0.5, -3.0]);
    fire.radius = 0.5;
    assert_eq!(
        serde_json::to_value(&fire).unwrap(),
        serde_json::json!({
            "id": "fire",
            "reference": {"kind": "object", "name": "farmyard:campsite/bonfire"},
            "label": "The bonfire",
            "state": "here",
            "at": [1.0, 0.5, -3.0],
            "radius": 0.5
        })
    );
    let newer: Target = serde_json::from_value(serde_json::json!({
        "id": "x",
        "reference": {"kind": "hologram"},
        "mark": "sparkle",
        "state": "flickering",
        "newer": true
    }))
    .unwrap();
    assert_eq!(newer.reference.kind, Kind::Unknown);
    assert_eq!(newer.mark, Mark::ByKind);
    assert_eq!(newer.state, State::Unknown);
    assert_eq!(newer.radius, DEFAULT_RADIUS);
    let unmarked = Target {
        mark: Mark::Unmarked,
        ..newer
    };
    assert_eq!(serde_json::to_value(&unmarked).unwrap()["mark"], "none");
}

#[test]
fn a_target_is_drawn_only_where_it_is_here_and_within_reach_and_bounded_whatever_was_sent() {
    let mut target = Target::new("t", Reference::new(Kind::Anchor, "home"));
    target.at = Some([0.0; 3]);
    assert_eq!(target.drawn_at(), None);
    target.state = State::Absent;
    assert_eq!(target.drawn_at(), None);
    target.state = State::Here;
    assert_eq!(target.drawn_at(), Some([0.0; 3]));
    target.at = Some([REACH + 1.0, 0.0, 0.0]);
    assert_eq!(target.drawn_at(), None);
    target.at = None;
    assert_eq!(target.drawn_at(), None);
    // Its label cut, its radius held, and a frame of no orientation, or of sides not finite, dropped.
    target.label = "l".repeat(LABEL_CHARS + 5);
    target.radius = 99.0;
    target.frame = Some(Frame {
        orientation: [0.0, 0.0, 0.0, 2.0],
        size: [20.0, -1.0],
    });
    let bounded = target.bounded();
    assert_eq!(bounded.label.chars().count(), LABEL_CHARS + 1);
    assert_eq!(bounded.radius, RADIUS.1);
    assert_eq!(
        bounded.frame,
        Some(Frame {
            orientation: [0.0, 0.0, 0.0, 1.0],
            size: [10.0, 0.0],
        })
    );
    target.radius = f32::NAN;
    target.frame = Some(Frame {
        orientation: [0.0; 4],
        size: [1.0, 1.0],
    });
    let bounded = target.bounded();
    assert_eq!((bounded.radius, bounded.frame), (DEFAULT_RADIUS, None));
    target.frame = Some(Frame {
        orientation: [0.0, 0.0, 0.0, 1.0],
        size: [f32::INFINITY, 1.0],
    });
    assert_eq!(target.bounded().frame, None);
}
