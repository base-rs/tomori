// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_environment::{OBJECT_LIMIT, Object};

/// The objects `described`, kept as the server keeps them, where they stand.
fn kept(described: Vec<Object>) -> Objects {
    crate::channel::objects(described, Some)
}

/// A head at `position`, facing -z.
fn head(position: [f32; 3]) -> Pose {
    Pose {
        position,
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// The identities of `objects` at `indices`.
fn ids<'a>(objects: &'a Objects, indices: &[usize]) -> Vec<&'a str> {
    indices
        .iter()
        .map(|index| objects[*index].id.as_str())
        .collect()
}

/// A side's objects, described in an order that helps nobody: a head stands at the origin facing -z.
fn yard() -> Objects {
    kept(vec![
        Object::new("far-behind", "a far shed").at([0.0, 0.0, 20.0]),
        Object::new("unplaced", "the weather"),
        Object::new("in-view-far", "a tree ahead").at([2.0, 0.0, -12.0]),
        Object::new("near-behind", "a bucket behind").at([0.0, 0.0, 2.0]),
        Object::new("looked-at", "a gate straight ahead").at([0.1, 0.0, -8.0]),
        Object::new("in-view-near", "a hen ahead").at([-3.0, 0.0, -4.0]),
        Object::new("pointed", "a lantern behind").at([0.0, 0.0, 25.0]),
        Object::new("recent", "a post behind").at([10.0, 0.0, 8.0]),
        Object::new("last", "a stone behind").at([5.0, 0.0, 15.0]),
    ])
}

#[test]
fn objects_rank_pointed_looked_at_around_recent_then_in_the_side_s_order() {
    let objects = yard();
    // What was pointed at or noted and is described no longer ranks nothing.
    let focus = vec!["gone".to_owned(), "pointed".to_owned()];
    let recent = VecDeque::from(vec!["gone".to_owned(), "recent".to_owned()]);
    let ranked = rank(
        &objects,
        &focus,
        &recent,
        Some(&head([0.0; 3])),
        AMBIENT_LIMIT,
    );
    assert_eq!(
        ids(&objects, &ranked),
        [
            "pointed",
            "looked-at",
            "near-behind",
            "in-view-near",
            "in-view-far",
            "recent",
            "far-behind",
            "unplaced",
            "last",
        ]
    );
    // Without a head, all but what was pointed at and lately noted keep the side's order.
    let ranked = rank(&objects, &focus, &recent, None, AMBIENT_LIMIT);
    assert_eq!(
        ids(&objects, &ranked),
        [
            "pointed",
            "recent",
            "far-behind",
            "unplaced",
            "in-view-far",
            "near-behind",
            "looked-at",
            "in-view-near",
            "last",
        ]
    );
    // Cut to the limit, the first stay first.
    let ranked = rank(&objects, &focus, &recent, Some(&head([0.0; 3])), 3);
    assert_eq!(
        ids(&objects, &ranked),
        ["pointed", "looked-at", "near-behind"]
    );
    assert!(rank(&objects, &focus, &recent, None, 0).is_empty());
}

#[test]
fn a_head_turned_looks_at_and_sees_what_it_faces() {
    let objects = yard();
    // Turned to face +z: the bucket is looked at, the stone, the shed and the lantern ahead, the gate behind.
    let turned = Pose {
        position: [0.0; 3],
        orientation: [0.0, 1.0, 0.0, 0.0],
    };
    let ranked = rank(&objects, &[], &VecDeque::new(), Some(&turned), 4);
    assert_eq!(
        ids(&objects, &ranked),
        ["near-behind", "last", "far-behind", "pointed"]
    );
    // An object where the head is is near, but looked at by nobody.
    let objects = kept(vec![
        Object::new("other", "something").at([0.0, 0.0, -20.0]),
        Object::new("here", "a hat").at([0.0; 3]),
    ]);
    let ranked = rank(&objects, &[], &VecDeque::new(), Some(&head([0.0; 3])), 2);
    assert_eq!(ids(&objects, &ranked), ["other", "here"]);
}

/// However many objects a side describes, the list is the first [`AMBIENT_LIMIT`] as they rank, wherever the side
/// put them, the pointed at and those around ahead of those it describes first.
#[test]
fn the_list_is_bounded_and_ranks_the_whole_of_what_is_described() {
    let mut described: Vec<Object> = (0..OBJECT_LIMIT - 2)
        .map(|index| Object::new(format!("far-{index}"), "far away").at([0.0, 0.0, 19.0]))
        .collect();
    described.push(Object::new("hen", "a hen just ahead").at([0.0, 0.0, -1.0]));
    described.push(Object::new("clock", "a world clock"));
    let objects = kept(described);
    assert_eq!(objects.len(), OBJECT_LIMIT);
    let ranked = rank(
        &objects,
        &["clock".to_owned()],
        &VecDeque::new(),
        Some(&head([0.0; 3])),
        AMBIENT_LIMIT,
    );
    assert_eq!(ranked.len(), AMBIENT_LIMIT);
    let named = ids(&objects, &ranked);
    assert_eq!(&named[..3], ["clock", "hen", "far-0"]);
    assert_eq!(
        named[AMBIENT_LIMIT - 1],
        format!("far-{}", AMBIENT_LIMIT - 3)
    );
}

#[test]
fn the_list_stays_as_ranked_throughout_a_turn() {
    let mut ambient = Ambient::default();
    let objects = yard();
    let at = head([0.0; 3]);
    ambient.focus("ada", ["pointed".to_owned()]);
    let first: Vec<String> = ids(
        &objects,
        &ambient.listed(&objects, 4, Some("ada"), Some(&at)),
    )
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(first[..2], ["pointed", "looked-at"]);
    // In the same turn, the side describes them again where they moved, without one, and someone points elsewhere and
    // turns away: the list is as it was, without what has gone.
    let moved = kept(vec![
        Object::new("last", "a stone right here").at([0.0, 0.0, -0.5]),
        Object::new("recent", "a post").at([10.0, 0.0, 10.0]),
        Object::new("in-view-near", "a hen").at([-3.0, 0.0, -4.0]),
        Object::new("in-view-far", "a tree").at([2.0, 0.0, -12.0]),
        Object::new("near-behind", "a bucket").at([0.0, 0.0, 2.0]),
        Object::new("unplaced", "the weather"),
        Object::new("far-behind", "a far shed").at([0.0, 0.0, 20.0]),
        Object::new("pointed", "a lantern").at([0.0, 0.0, 25.0]),
    ]);
    ambient.focus("ada", ["last".to_owned()]);
    let elsewhere = head([0.0, 0.0, 19.0]);
    let again = ids(
        &moved,
        &ambient.listed(&moved, 4, Some("ada"), Some(&elsewhere)),
    );
    let expected: Vec<&str> = first
        .iter()
        .map(String::as_str)
        .filter(|id| *id != "looked-at")
        .collect();
    assert_eq!(again, expected);
    // The next turn ranks it again.
    let next = ids(
        &moved,
        &ambient.listed(&moved, 5, Some("ada"), Some(&elsewhere)),
    );
    assert_eq!(next[..3], ["last", "near-behind", "far-behind"]);
    // A list ranked when nothing was described is ranked again once something is.
    let mut empty = Ambient::default();
    empty.focus("ada", ["last".to_owned()]);
    assert!(
        empty
            .listed(&Objects::default(), 1, Some("ada"), None)
            .is_empty()
    );
    assert_eq!(
        ids(&moved, &empty.listed(&moved, 1, Some("ada"), None))[0],
        "last"
    );
}

#[test]
fn what_was_pointed_at_and_noted_is_bounded_newest_first() {
    let mut ambient = Ambient::default();
    ambient.focus(
        "ada",
        ["a", "b", "a", "c", "d", "e"]
            .into_iter()
            .map(str::to_owned),
    );
    assert_eq!(ambient.pointed(Some("ada")), ["a", "b", "c", "d"]);
    assert_eq!(ambient.recent, ["a", "b", "c", "d"]);
    for index in 0..RECENT_LIMIT {
        ambient.noted(&format!("n{index}"));
    }
    ambient.noted("n3");
    assert_eq!(ambient.recent.len(), RECENT_LIMIT);
    assert_eq!(ambient.recent[0], "n3");
    assert_eq!(ambient.recent[1], format!("n{}", RECENT_LIMIT - 1));
    assert!(!ambient.recent.contains(&"a".to_owned()));
    // Her words pointing at nothing point at nothing; nobody's point at nothing.
    ambient.focus("ada", Vec::new());
    assert!(ambient.pointed(Some("ada")).is_empty());
    assert!(ambient.pointed(None).is_empty());
    // Each speaker's pointing is their own, as many as the bound, the earliest to speak forgotten first.
    for index in 0..SPEAKERS_LIMIT {
        ambient.focus(&format!("p{index}"), [format!("o{index}")]);
    }
    assert_eq!(ambient.focus.len(), SPEAKERS_LIMIT);
    assert!(ambient.pointed(Some("ada")).is_empty());
    assert_eq!(ambient.pointed(Some("p0")), ["o0"]);
    ambient.focus("p0", ["again".to_owned()]);
    assert_eq!(ambient.focus.len(), SPEAKERS_LIMIT);
    assert_eq!(ambient.pointed(Some("p0")), ["again"]);
    ambient.focus("bob", Vec::new());
    assert!(ambient.pointed(Some("p1")).is_empty());
}

/// Each speaker's turn ranks what they pointed at, never what someone else pointed at meanwhile.
#[test]
fn a_turn_ranks_what_its_own_speaker_pointed_at() {
    let objects = yard();
    let mut ambient = Ambient::default();
    ambient.focus("ada", ["pointed".to_owned()]);
    ambient.focus("bob", ["last".to_owned()]);
    let at = head([0.0; 3]);
    assert_eq!(
        ids(
            &objects,
            &ambient.listed(&objects, 1, Some("ada"), Some(&at))
        )[0],
        "pointed"
    );
    assert_eq!(
        ids(
            &objects,
            &ambient.listed(&objects, 2, Some("bob"), Some(&at))
        )[0],
        "last"
    );
    // Nobody's turn ranks nobody's pointing first.
    assert_eq!(
        ids(&objects, &ambient.listed(&objects, 3, None, Some(&at)))[0],
        "looked-at"
    );
}

/// Of objects about as near where the head faces, within a degree, the one nearer the head is looked at.
#[test]
fn of_objects_looked_at_alike_the_nearer_is_looked_at() {
    let objects = kept(vec![
        Object::new("far-dead-ahead", "a far post").at([0.0, 0.0, -20.0]),
        Object::new("near-just-off", "a near post").at([0.05, 0.0, -5.0]),
        Object::new("nearer-well-off", "a nearer post").at([0.4, 0.0, -4.0]),
    ]);
    let ranked = rank(&objects, &[], &VecDeque::new(), Some(&head([0.0; 3])), 1);
    assert_eq!(ids(&objects, &ranked), ["near-just-off"]);
    // Described the other way about, the same.
    let objects = kept(vec![
        Object::new("near-just-off", "a near post").at([0.05, 0.0, -5.0]),
        Object::new("far-dead-ahead", "a far post").at([0.0, 0.0, -20.0]),
    ]);
    let ranked = rank(&objects, &[], &VecDeque::new(), Some(&head([0.0; 3])), 1);
    assert_eq!(ids(&objects, &ranked), ["near-just-off"]);
}
