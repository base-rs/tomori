// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::Pose;
use tomori_protocol::cluster::HOME;

fn worker(id: &str, parent: Option<&str>) -> tomori_protocol::Worker {
    tomori_protocol::Worker {
        id: id.to_owned(),
        parent: parent.map(str::to_owned),
        title: format!("Title {id}"),
        state: tomori_protocol::WorkerState::Running,
    }
}

/// Home, in a room that compacts after a few, and a rack.
fn anchors() -> Vec<Anchor> {
    let home = Anchor::new(
        HOME,
        Pose::at([0.0, 1.4, -4.0]).facing_towards([0.0, 1.6, 0.0]),
    )
    .with_extent(4.2, 1.6, 2.0);
    let rack = Anchor::new(
        "rack/a12",
        Pose::at([4.0, 1.4, -6.0]).facing_towards([0.0, 1.4, 0.0]),
    );
    vec![home, rack]
}

fn seated(ids: &[&str], at: &'static str) -> Vec<(Member, &'static str)> {
    ids.iter()
        .map(|id| (Member::Worker((*id).to_owned()), at))
        .collect()
}

fn slots(seating: &Seating, name: &str) -> Vec<(String, usize)> {
    seating
        .members(name)
        .iter()
        .map(|(Member::Worker(id), slot)| (id.clone(), *slot))
        .collect()
}

#[test]
fn members_take_the_lowest_free_slot_in_order_of_arrival_and_keep_it_while_they_stay() {
    let anchors = anchors();
    let mut seating = Seating::default();
    assert!(seating.seat(&anchors, &seated(&["a", "b", "c"], HOME)));
    assert_eq!(
        slots(&seating, HOME),
        [("a".into(), 0), ("b".into(), 1), ("c".into(), 2)]
    );
    // The same again changes nothing.
    assert!(!seating.seat(&anchors, &seated(&["a", "b", "c"], HOME)));
    // One leaving leaves its gap, which the next to arrive fills; nobody else moves.
    assert!(seating.seat(&anchors, &seated(&["a", "c"], HOME)));
    assert_eq!(slots(&seating, HOME), [("a".into(), 0), ("c".into(), 2)]);
    assert!(seating.seat(&anchors, &seated(&["a", "c", "d"], HOME)));
    assert_eq!(
        slots(&seating, HOME),
        [("a".into(), 0), ("c".into(), 2), ("d".into(), 1)]
    );
    // Moving to another anchor, it leaves home and takes the rack's first slot.
    let mut wanted = seated(&["a", "d"], HOME);
    wanted.push((Member::Worker("c".into()), "rack/a12"));
    assert!(seating.seat(&anchors, &wanted));
    assert_eq!(slots(&seating, "rack/a12"), [("c".into(), 0)]);
    assert_eq!(seating.laid("rack/a12"), 1);
    // An anchor that goes takes its cluster with it; one not among the anchors seats nobody.
    let home_only = &anchors[..1];
    assert!(seating.seat(home_only, &wanted));
    assert!(seating.members("rack/a12").is_empty());
    assert_eq!(seating.laid("nowhere"), 0);
    // The layouts sent say where each worker is, in the order of their slots, and the span kept a slot higher as they
    // leave.
    assert_eq!(seating.laid(HOME), 3);
    assert_eq!(
        seating.layouts(),
        [Layout::new(HOME, 3).with_seat("a", 0).with_seat("d", 1)]
    );
    // Everyone gone, nothing is laid out.
    assert!(seating.seat(&anchors, &[]));
    assert!(seating.layouts().is_empty());
}

#[test]
fn a_cluster_at_a_step_s_bound_lays_out_again_once_as_a_member_comes_and_goes() {
    let anchors = anchors();
    let cluster = Cluster::new(&anchors[0]);
    let compact = (1..64)
        .find(|span| cluster.arrange(*span).compact())
        .unwrap();
    let ids: Vec<String> = (0..compact).map(|index| format!("w{index:02}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut seating = Seating::default();
    seating.seat(&anchors, &seated(&ids[..compact - 1], HOME));
    let full = |seating: &Seating| !cluster.arrange(seating.laid(HOME)).compact();
    assert!(full(&seating));
    // One more compacts it; it leaving and coming back keeps it compact, and nobody moves.
    seating.seat(&anchors, &seated(&ids, HOME));
    assert!(!full(&seating));
    let standing = slots(&seating, HOME);
    for _ in 0..3 {
        seating.seat(&anchors, &seated(&ids[..compact - 1], HOME));
        assert!(!full(&seating));
        seating.seat(&anchors, &seated(&ids, HOME));
        assert_eq!(slots(&seating, HOME), standing);
    }
    // Two fewer, it steps back to full size.
    seating.seat(&anchors, &seated(&ids[..compact - 2], HOME));
    assert!(full(&seating));
}

#[test]
fn those_beyond_what_a_cluster_holds_are_gathered_into_its_sign() {
    let anchors = anchors();
    let cluster = Cluster::new(&anchors[0]);
    let more = (1..64)
        .find(|span| cluster.arrange(*span).more().is_some())
        .unwrap();
    let ids: Vec<String> = (0..more + 2).map(|index| format!("w{index:02}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut seating = Seating::default();
    seating.seat(&anchors, &seated(&ids, HOME));
    let gathered = seating.gathered(&anchors[0]);
    let first = cluster.arrange(seating.laid(HOME)).more().unwrap();
    assert_eq!(gathered.len(), ids.len() - first);
    assert_eq!(gathered[0], &Member::Worker(ids[first].to_owned()));
    // The rack gathers nobody.
    assert!(seating.gathered(&anchors[1]).is_empty());
}

#[test]
fn a_worker_stands_where_it_was_placed_or_with_whoever_delegated_to_it_or_else_at_home() {
    let anchors = anchors();
    let workers = [
        worker("a", None),
        worker("b", Some("a")),
        worker("c", Some("b")),
        worker("d", None),
        // A cycle ends.
        worker("x", Some("y")),
        worker("y", Some("x")),
        // A parent not listed.
        worker("e", Some("gone")),
    ];
    let placed = |session: &str| match session {
        "a" => Some("rack/a12"),
        "d" => Some("aisle-9"),
        _ => None,
    };
    let wanted = super::workers(&workers, &anchors, placed);
    let at: Vec<&str> = wanted.iter().map(|(_, at)| *at).collect();
    // Its children follow it, theirs too; one placed at an anchor not here stands at home.
    assert_eq!(
        at,
        ["rack/a12", "rack/a12", "rack/a12", HOME, HOME, HOME, HOME]
    );
    assert_eq!(wanted[2].0, Member::Worker("c".into()));
}

#[test]
fn the_home_anchor_stands_about_the_workers_anchor_or_else_the_agent_s_home_in_its_room() {
    let pose = Pose::at([1.0, 1.4, -3.0]);
    let room = Anchor::new(HOME, pose).with_extent(5.0, 2.0, 2.0);
    let anchors = tomori_protocol::Anchors::new()
        .with_workers(pose)
        .with_cluster(room.clone());
    assert_eq!(home(&anchors), room);
    let agent = Pose::at([0.0, 1.5, -2.0]);
    let bare = home(&tomori_protocol::Anchors::new().with_agent(agent));
    assert_eq!((bare.pose, bare.extent), (agent, None));
    assert_eq!(
        home(&tomori_protocol::Anchors::new()).pose,
        crate::hub::agent_home(&tomori_protocol::Anchors::new())
    );
}
