// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

fn placed(session: &str, at: &str, pose: Option<Pose>) -> Placement {
    Placement {
        session: session.to_owned(),
        at: at.to_owned(),
        pose,
        beside: None,
        by: None,
    }
}

#[test]
fn placements_are_kept_one_for_each_worker_at_most_so_many_the_oldest_forgotten_first() {
    let mut placements = Vec::new();
    keep(&mut placements, placed("a", "rack/a12", None));
    keep(&mut placements, placed("b", "rack/a12", None));
    keep(&mut placements, placed("a", "aisle-3", None));
    let at: Vec<(&str, &str)> = placements
        .iter()
        .map(|placement| (placement.session.as_str(), placement.at.as_str()))
        .collect();
    assert_eq!(at, [("b", "rack/a12"), ("a", "aisle-3")]);
    for index in 0..PLACEMENTS + 3 {
        keep(
            &mut placements,
            placed(&format!("w{index}"), "rack/a12", None),
        );
    }
    assert_eq!(placements.len(), PLACEMENTS);
    assert_eq!(placements[0].session, "w3");
    // Kept as written, and read back so; what is not said is not written.
    let json = serde_json::to_value(&placements[0]).unwrap();
    assert!(json.get("pose").is_none() && json.get("by").is_none());
    assert_eq!(
        serde_json::from_value::<Placement>(json).unwrap(),
        placements[0]
    );
}

#[test]
fn the_server_s_own_clusters_stand_beside_a_wall_or_a_person_shared_by_those_placed_together() {
    let wall = wall_cluster("added-1", 3);
    assert_eq!(wall, "tomori/wall/added-1");
    // A wall's identity no anchor's name may hold is named by its place.
    assert_eq!(wall_cluster("a wall", 3), "tomori/wall/3");
    let beside = Pose::at([1.5, 1.4, -2.0]);
    let (first, at) = near_cluster(&[], &beside);
    assert_eq!((first.as_str(), at), ("tomori/near/1.5_-2.0", beside));
    let mut placements = vec![
        Placement {
            beside: Some("Ada".into()),
            ..placed("a", &first, Some(at))
        },
        Placement {
            beside: Some("added wall 1".into()),
            ..placed("b", &wall, Some(Pose::at([4.0, 1.4, -3.0])))
        },
        placed("c", "rack/a12", None),
    ];
    // Another placed beside the same person, a little off, shares the first's.
    let (shared, pose) = near_cluster(&placements, &Pose::at([1.8, 1.4, -2.2]));
    assert_eq!((shared.as_str(), pose), (first.as_str(), beside));
    placements.push(placed("d", &shared, Some(pose)));
    // Far off, one of its own.
    let (other, _) = near_cluster(&placements, &Pose::at([6.0, 1.4, -2.0]));
    assert_ne!(other, first);
    let made = own(&placements);
    let names: Vec<&str> = made.iter().map(|anchor| anchor.name.as_str()).collect();
    assert_eq!(names, [first.as_str(), wall.as_str()]);
    assert_eq!(
        (made[0].kind.as_deref(), made[0].label.as_deref()),
        (Some("person"), Some("Beside Ada"))
    );
    assert_eq!(
        (made[1].kind.as_deref(), made[1].label.as_deref()),
        (Some("wall"), Some("Beside added wall 1"))
    );
    // One without a pose of its own, or what it stands beside, is labelled by nothing.
    let bare = own(&[placed("e", "tomori/near/0.0_0.0", Some(Pose::HOME))]);
    assert_eq!(bare[0].label, None);
}

#[test]
fn the_server_s_own_cluster_by_an_object_stands_before_it_towards_whoever_asked_facing_them() {
    assert_eq!(object_cluster("lamp/1", 2), "tomori/object/lamp/1");
    assert_eq!(object_cluster("a lamp", 2), "tomori/object/2");
    // A metre before it, towards them, at their eye height, facing them.
    let pose = before_object([0.0, 0.5, -4.0], [3.0, 1.6, 0.0]);
    let [x, y, z] = pose.position;
    assert!((x - 0.6).abs() < 1e-5 && (y - 1.6).abs() < 1e-5 && (z + 3.2).abs() < 1e-5);
    let facing = tomori_protocol::Pose::upright_facing(pose.position, [3.0, 1.6, 0.0]);
    assert!((pose.yaw() - facing.yaw()).abs() < 1e-4);
    // Asked from where it stands, before it along +Z.
    let pose = before_object([1.0, 0.5, 1.0], [1.0, 1.7, 1.0]);
    assert_eq!(pose.position, [1.0, 1.7, 2.0]);
    let ahead = Pose::upright_facing([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    assert!((pose.yaw() - ahead.yaw()).abs() < 1e-4);
    let own = own(&[Placement {
        beside: Some("yard:lamp/1".into()),
        ..placed("f", "tomori/object/lamp/1", Some(pose))
    }]);
    assert_eq!(own[0].kind.as_deref(), Some("object"));
}

#[test]
fn the_agent_places_each_worker_and_moves_itself_within_its_limits() {
    let start = Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    let mut rates = Rates::default();
    // A worker's first placement is free of its own limit; then once a minute, three times in ten.
    assert_eq!(rates.place("a", start), Ok(()));
    assert_eq!(rates.place("a", at(1)), Ok(()));
    assert!(
        rates
            .place("a", at(30))
            .unwrap_err()
            .contains("last minute")
    );
    assert_eq!(rates.place("a", at(61)), Ok(()));
    assert_eq!(rates.place("a", at(122)), Ok(()));
    assert!(
        rates
            .place("a", at(200))
            .unwrap_err()
            .contains("three times")
    );
    assert_eq!(rates.place("a", at(602)), Ok(()));
    // Forgotten, as it is removed, its next is its first.
    rates.forget("a");
    assert_eq!(rates.place("a", at(603)), Ok(()));
    // The space's, twelve a minute together.
    let mut rates = Rates::default();
    for index in 0..12 {
        assert_eq!(rates.place(&format!("w{index}"), start), Ok(()));
    }
    assert!(rates.place("w12", start).unwrap_err().contains("twelve"));
    assert_eq!(rates.place("w12", at(60)), Ok(()));
    // Its own moves, once every 20 seconds and six in ten minutes.
    let mut rates = Rates::default();
    assert_eq!(rates.move_agent(start), Ok(()));
    assert!(rates.move_agent(at(10)).unwrap_err().contains("20 seconds"));
    for step in 1..6 {
        assert_eq!(rates.move_agent(at(step * 20)), Ok(()));
    }
    assert!(rates.move_agent(at(140)).is_err());
    assert_eq!(rates.move_agent(at(600)), Ok(()));
}
