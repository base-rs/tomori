// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The home cluster fitted to its room, its "+N more" sign, and the satellites that orbit the agent while it is away.
use super::*;
use crate::anchors::HomeArc;
use tomori_protocol::cluster::{COMPACT, Extent};

const EYES: Vec3 = crate::anchors::EYES;

fn node(id: &str, state: State) -> Node {
    Node {
        id: id.into(),
        parent_id: Some("root".into()),
        title: format!("Title {id}"),
        state,
    }
}

fn workers(count: usize) -> Workspace {
    Workspace {
        nodes: (0..count)
            .map(|i| node(&format!("w{i:02}"), State::Running))
            .collect(),
        ..Default::default()
    }
}

fn settle(scene: &mut Scene, workspace: &Workspace) {
    for _ in 0..240 {
        scene.update(Some("root"), workspace, 1.0 / 60.0);
    }
}

/// The room about the default home: 4.2 m across, 1.6 m up and 2 m deep, which holds two workers, then eight compact.
const ROOM: Extent = Extent::new(4.2, 1.6, 2.0);

/// The fewest workers that compact the home cluster in [`ROOM`], and that gather some into its "+N more" sign.
fn thresholds() -> (usize, usize) {
    let cluster = crate::anchors::cluster(Anchor::AGENT, EYES, Some(ROOM));
    let compact = (1..64)
        .find(|span| cluster.arrange(*span).compact())
        .unwrap();
    let more = (1..64)
        .find(|span| cluster.arrange(*span).more().is_some())
        .unwrap();
    (compact, more)
}

fn own(scene: &Scene) -> BTreeMap<String, ([f32; 3], f32)> {
    scene
        .stations()
        .iter()
        .filter(|station| station.more == 0)
        .map(|station| (station.id.clone(), (station.position, station.scale)))
        .collect()
}

#[test]
fn workers_stand_in_the_home_cluster_fitted_to_its_room_moving_only_as_it_compacts() {
    let (compact, more) = thresholds();
    assert!(1 < compact && compact < more, "{compact} {more}");
    let cluster = crate::anchors::cluster(Anchor::AGENT, EYES, Some(ROOM));
    let mut scene = Scene::new();
    scene.arrange(Anchor::AGENT, EYES, Some(ROOM), None);
    let mut before = BTreeMap::new();
    for count in 1..compact {
        settle(&mut scene, &workers(count));
        let standing = own(&scene);
        // Each in its slot of the fitted cluster, at full size; nobody who stood moved as another arrived.
        let layout = cluster.arrange(count);
        for (id, (position, scale)) in &standing {
            let slot = scene.slots[id];
            assert_eq!(*position, cluster.slot(&layout, slot).position, "{id}");
            assert_eq!(*scale, 1.0);
        }
        for (id, was) in &before {
            assert_eq!(
                standing.get(id),
                Some(was),
                "{id} moved as worker {count} arrived"
            );
        }
        before = standing;
    }
    // One more than it holds: every live worker is placed again, compact, and glides there.
    settle(&mut scene, &workers(compact));
    let standing = own(&scene);
    assert_eq!(standing.len(), compact);
    assert!(standing.values().all(|(_, scale)| *scale == COMPACT));
    assert!(
        before
            .iter()
            .any(|(id, (position, _))| standing[id].0 != *position)
    );
    // And back to full size as it holds them again.
    settle(&mut scene, &workers(compact - 1));
    assert!(own(&scene).values().all(|(_, scale)| *scale == 1.0));
}

#[test]
fn beyond_what_compact_holds_the_rest_gather_into_one_sign_listed_when_pointed_at() {
    let (_, more) = thresholds();
    let mut scene = Scene::new();
    scene.arrange(Anchor::AGENT, EYES, Some(ROOM), None);
    let count = more + 3;
    let w = workers(count);
    settle(&mut scene, &w);
    let cluster = crate::anchors::cluster(Anchor::AGENT, EYES, Some(ROOM));
    let first = cluster.arrange(count).more().unwrap();
    // Those in slots of their own, and one sign for the rest, where the first of them stands, saying how many.
    assert_eq!(own(&scene).len(), first);
    let signs: Vec<&Station> = scene.stations().iter().filter(|s| s.more > 0).collect();
    assert_eq!(signs.len(), 1);
    let sign = signs[0].clone();
    assert_eq!(sign.more, count - first);
    assert!(sign.title.is_empty() && sign.scale == COMPACT && sign.valid());
    // Pointed at, it lists them, highlights them all and names none of them; the ray ends among them.
    // From beneath it, where no other worker stands.
    let ray = Ray {
        origin: Vec3::from_array(sign.position) - Vec3::Y * 0.8,
        direction: Vec3::Y,
    };
    assert_eq!(scene.point(Some(ray)), None);
    assert!(scene.hit_point().is_some());
    scene.update(Some("root"), &w, 1.0 / 60.0);
    let listed = scene.stations().iter().find(|s| s.more > 0).unwrap();
    assert!(
        listed.title.contains(&format!("Title w{:02}", count - 1)),
        "{}",
        listed.title
    );
    assert!(listed.title.contains(" · "));
    let cubes = scene.cubes().len();
    // Pointed away, it says how many again.
    assert_eq!(scene.point(None), None);
    scene.update(Some("root"), &w, 1.0 / 60.0);
    assert!(
        scene
            .stations()
            .iter()
            .find(|s| s.more > 0)
            .unwrap()
            .title
            .is_empty()
    );
    assert_eq!(scene.cubes().len(), cubes);
    // As workers leave, the sign goes.
    settle(&mut scene, &workers(first));
    assert!(scene.stations().iter().all(|s| s.more == 0));
}

#[test]
fn a_registered_placement_places_each_worker_once_even_as_the_cluster_compacts() {
    struct Row;
    impl crate::anchors::WorkerPlacement for Row {
        fn place(&self, _origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            Vec3::new(worker.slot as f32 * 0.4, 1.4, -3.0)
        }
    }
    let (compact, _) = thresholds();
    let mut scene = Scene::new();
    scene.arrange(
        Anchor::AGENT,
        EYES,
        Some(ROOM),
        Some(std::sync::Arc::new(Row)),
    );
    settle(&mut scene, &workers(compact - 1));
    let before = own(&scene);
    settle(&mut scene, &workers(compact + 2));
    let after = own(&scene);
    for (id, was) in before {
        assert_eq!(after[&id], was);
    }
    assert!(after.values().all(|(_, scale)| *scale == 1.0));
}

/// The satellites' colours, in the order they are drawn.
fn colours(scene: &Scene) -> Vec<[f32; 3]> {
    scene.satellites().iter().map(|cube| cube.color).collect()
}

#[test]
fn satellites_orbit_the_agent_away_one_for_each_worker_working_in_its_state_s_colour() {
    let agent = Vec3::new(3.0, 1.5, -1.0);
    let mut scene = Scene::new();
    let mut w = workers(12);
    w.nodes[1].state = State::Waiting;
    w.nodes[2].state = State::Idle;
    w.nodes[3].state = State::Completed;
    // At home, none.
    settle(&mut scene, &w);
    assert!(scene.satellites().is_empty());
    // Away, at most eight, by slot, each the colour of its worker's state, orbiting it beyond its reach.
    scene.set_agent(Some((agent, 0.4)));
    scene.update(Some("root"), &w, 1.0 / 60.0);
    let shown = colours(&scene);
    assert_eq!(shown.len(), MOTES);
    assert_eq!(shown[0], palette(State::Running).to_array());
    assert_eq!(shown[1], palette(State::Waiting).to_array());
    for cube in scene.satellites() {
        let at = Vec3::from_array(cube.position);
        assert!(
            ((at - agent).with_y(0.0).length() - 0.55).abs() < 1e-3,
            "{at}"
        );
        assert_eq!(cube.scale, [MOTE_SIDE; 3]);
        assert!(
            scene
                .cubes()
                .iter()
                .any(|drawn| drawn.position == cube.position)
        );
    }
    // With fewer working, one for each.
    let few = Workspace {
        nodes: w.nodes[..5].to_vec(),
        ..Default::default()
    };
    settle(&mut scene, &few);
    assert_eq!(scene.satellites().len(), 3);
    // They orbit: a while later each stands elsewhere about it.
    let then = scene.satellites()[0].position;
    for _ in 0..60 {
        scene.update(Some("root"), &few, 1.0 / 60.0);
    }
    assert_ne!(scene.satellites()[0].position, then);
    // Back home, none; and an agent not finite is none.
    scene.set_agent(None);
    scene.update(Some("root"), &few, 1.0 / 60.0);
    assert!(scene.satellites().is_empty());
    scene.set_agent(Some((Vec3::NAN, 0.4)));
    scene.update(Some("root"), &few, 1.0 / 60.0);
    assert!(scene.satellites().is_empty());
}

#[test]
fn a_finished_worker_s_satellite_pulses_once_and_fades() {
    let mut scene = Scene::new();
    let mut w = workers(2);
    settle(&mut scene, &w);
    scene.set_agent(Some((Vec3::new(0.0, 1.4, -1.0), 0.3)));
    scene.update(Some("root"), &w, 1.0 / 60.0);
    assert_eq!(scene.satellites().len(), 2);
    w.nodes[0].state = State::Completed;
    let done = palette(State::Completed).to_array();
    let mut sizes = Vec::new();
    for _ in 0..120 {
        scene.update(Some("root"), &w, 1.0 / 60.0);
        sizes.push(
            scene
                .satellites()
                .iter()
                .find(|cube| cube.color == done)
                .map_or(0.0, |cube| cube.scale[0]),
        );
    }
    // It grows to nearly twice its size and back within half a second, once, then shrinks away and is gone by 1.5 s.
    let largest = sizes.iter().copied().fold(0.0, f32::max);
    assert!(largest > MOTE_SIDE * 1.8, "{largest}");
    let peak = sizes.iter().position(|size| *size == largest).unwrap();
    assert!(peak < 30, "{peak}");
    assert!(
        sizes[peak..]
            .windows(2)
            .all(|pair| pair[1] <= pair[0] + 1e-6)
    );
    assert_eq!(sizes[95], 0.0);
    // The other still works; the finished one gets no mote again.
    assert_eq!(scene.satellites().len(), 1);
    assert_eq!(colours(&scene), [palette(State::Running).to_array()]);
    // A worker that finished resumes: its mote comes back.
    w.nodes[0].state = State::Running;
    scene.update(Some("root"), &w, 1.0 / 60.0);
    assert_eq!(scene.satellites().len(), 2);
    // One that resumes mid-pulse is working again; one that stops goes.
    w.nodes[0].state = State::Error;
    scene.update(Some("root"), &w, 1.0 / 60.0);
    w.nodes[0].state = State::Waiting;
    scene.update(Some("root"), &w, 1.0 / 60.0);
    assert!(colours(&scene).contains(&palette(State::Waiting).to_array()));
    w.nodes[0].state = State::Stopped;
    scene.update(Some("root"), &w, 1.0 / 60.0);
    assert_eq!(scene.satellites().len(), 1);
    // Leaving the space's workers, its mote goes.
    settle(&mut scene, &workers(0));
    assert!(scene.satellites().is_empty());
    assert!((pulse(0.25) - 1.9).abs() < 1e-4 && pulse(PULSE_SECONDS) == 0.0);
}

#[test]
fn workers_at_home_are_pointed_at_from_afar_while_the_agent_is_away_and_its_satellites_never_are() {
    let mut scene = Scene::new();
    let w = workers(4);
    settle(&mut scene, &w);
    // Someone 6 m off, with the agent beside them.
    let eyes = Vec3::new(5.0, 1.6, 3.0);
    scene.set_agent(Some((eyes + Vec3::new(-1.0, 0.0, -1.0), 0.35)));
    scene.update(Some("root"), &w, 1.0 / 60.0);
    let worker = scene.workers["w00"].anchor;
    assert!(eyes.distance(worker) > 6.0);
    // Passing beneath it, where no other worker stands.
    let aside = Vec3::NEG_Y;
    let ray = |offset: f32| Ray {
        origin: eyes,
        direction: (worker + aside * offset - eyes).normalize(),
    };
    assert_eq!(scene.point(Some(ray(0.45))), Some("w00"));
    assert_eq!(scene.point(Some(ray(1.2))), None);
    // Straight at a satellite, from beneath it, nothing.
    let mote = Vec3::from_array(scene.satellites()[0].position);
    let at_mote = Ray {
        origin: mote - Vec3::Y * 0.5,
        direction: Vec3::Y,
    };
    assert_eq!(scene.point(Some(at_mote)), None);
}

#[test]
fn gaps_close_so_that_nobody_is_gathered_or_compact_beside_room_left_by_those_who_went() {
    let (compact, more) = thresholds();
    let mut scene = Scene::new();
    scene.arrange(Anchor::AGENT, EYES, Some(ROOM), None);
    let all = workers(more + 3);
    settle(&mut scene, &all);
    // The last worker gathered into the sign takes the first slot of its own that frees, alone moving.
    let gone = "w00";
    let before = own(&scene);
    let first = scene
        .workers
        .iter()
        .filter(|(_, worker)| worker.more)
        .max_by_key(|(_, worker)| worker.slot)
        .map(|(id, _)| id.clone())
        .unwrap();
    assert_eq!(scene.workers[gone].slot, 0);
    let fewer = Workspace {
        nodes: all.nodes[1..].to_vec(),
        ..Default::default()
    };
    // With the agent away, its satellites keep to their workers' slots, the first eight.
    scene.set_agent(Some((Vec3::new(3.0, 1.5, -1.0), 0.4)));
    settle(&mut scene, &fewer);
    assert_eq!(scene.satellites().len(), MOTES);
    assert!(
        scene
            .motes
            .iter()
            .all(|mote| mote.slot == scene.slots[&mote.id])
    );
    assert!(
        scene
            .motes
            .iter()
            .any(|mote| mote.id == first && mote.slot == 0)
    );
    let after = own(&scene);
    assert!(!scene.workers[&first].more);
    assert_eq!(scene.slots[&first], 0);
    for (id, was) in &before {
        if id != gone {
            assert_eq!(after.get(id), Some(was), "{id}");
        }
    }
    // All but the last gone, it stands in a slot of its own, at full size, rather than alone in a "+1 more" sign.
    let last = all.nodes.last().unwrap().clone();
    let alone = Workspace {
        nodes: vec![last.clone()],
        ..Default::default()
    };
    settle(&mut scene, &alone);
    assert_eq!(scene.slots[&last.id], 0);
    assert!(!scene.workers[&last.id].more && scene.workers[&last.id].scale == 1.0);
    assert!(scene.stations().iter().all(|station| station.more == 0));
    // Children that would stand beside their parent further along take the gaps, so the cluster compacts only as it
    // fills.
    let mut scene = Scene::new();
    scene.arrange(Anchor::AGENT, EYES, Some(ROOM), None);
    let mut family = workers(compact - 2);
    let parent = family.nodes.last().unwrap().id.clone();
    let mut child = node("child", State::Running);
    child.parent_id = Some(parent);
    family.nodes.push(child);
    settle(&mut scene, &family);
    assert!(own(&scene).values().all(|(_, scale)| *scale == 1.0));
    let mut taken: Vec<usize> = scene.workers.values().map(|worker| worker.slot).collect();
    taken.sort_unstable();
    assert_eq!(taken, (0..compact - 1).collect::<Vec<_>>());
}

#[test]
fn one_arriving_far_along_takes_a_gap_rather_than_compact_the_cluster() {
    let (compact, _) = thresholds();
    let mut scene = Scene::new();
    scene.arrange(Anchor::AGENT, EYES, Some(ROOM), None);
    settle(&mut scene, &workers(1));
    // A worker given a slot well beyond those taken, as one beside its parent may be, takes the first gap.
    let late = node("late", State::Running);
    let far = compact + 4;
    let mut arriving = [(far, "late", &late)];
    scene.reslot(&mut arriving, &[]);
    assert_eq!((arriving[0].0, scene.slots["late"]), (1, 1));
    assert!(!scene.layout.compact());
}

/// A rack 5 m off to the right, facing home.
fn rack() -> tomori_protocol::cluster::Anchor {
    tomori_protocol::cluster::Anchor::new(
        "rack/a12",
        tomori_protocol::Pose::at([5.0, 1.4, -4.0]).facing_towards([0.0, 1.4, 0.0]),
    )
    .with_extent(3.0, 2.5, 1.0)
}

/// The server's layouts: `home` at home, from slot 0, `there` at the rack, from slot 0, each laid out for `span`.
fn layouts(home: &[&str], there: &[&str], span: u32) -> Vec<tomori_protocol::cluster::Layout> {
    use tomori_protocol::cluster::Layout;
    let seat = |name: &str, ids: &[&str]| {
        ids.iter()
            .enumerate()
            .fold(Layout::new(name, span), |layout, (slot, id)| {
                layout.with_seat(*id, slot as u32)
            })
    };
    vec![seat("home", home), seat("rack/a12", there)]
}

#[test]
fn workers_stand_where_the_server_seats_them_alike_on_every_client_and_glide_as_it_reseats_them() {
    use tomori_protocol::cluster::Cluster;
    let w = workers(4);
    let seats = layouts(&["w00", "w01"], &["w02", "w03"], 2);
    // Two clients, one seeing the workers arrive one by one and the other all at once, stand them alike.
    let mut one = Scene::new();
    let mut other = Scene::new();
    for count in 1..=4 {
        one.seat(&[rack()], &seats);
        settle(
            &mut one,
            &Workspace {
                nodes: w.nodes[..count].to_vec(),
                ..Default::default()
            },
        );
    }
    other.seat(&[rack()], &seats);
    settle(&mut other, &w);
    assert_eq!(own(&one), own(&other));
    // Those at the rack stand in its cluster's slots, laid out for the span the server keeps.
    let cluster = Cluster::new(&rack());
    let laid = cluster.arrange(2);
    for (id, slot) in [("w02", 0), ("w03", 1)] {
        assert_eq!(own(&one)[id].0, cluster.slot(&laid, slot).position, "{id}");
    }
    assert_eq!(one.slots["w03"], 1);
    // Reseated, only the worker whose seat changed moves, and it glides there.
    let before = own(&one);
    one.seat(&[rack()], &layouts(&["w00", "w01", "w03"], &["w02"], 2));
    one.update(Some("root"), &w, 1.0 / 60.0);
    let moving = own(&one);
    assert_ne!(moving["w03"].0, before["w03"].0);
    assert_ne!(
        moving["w03"].0,
        HomeArc::at(Anchor::AGENT, EYES, 2).to_array()
    );
    settle(&mut one, &w);
    settle(&mut one, &w);
    let after = own(&one);
    assert_eq!(
        after["w03"].0,
        HomeArc::at(Anchor::AGENT, EYES, 2).to_array()
    );
    for id in ["w00", "w01", "w02"] {
        assert_eq!(after[id], before[id], "{id}");
    }
    // Seated the same again, nothing changes.
    one.seat(&[rack()], &layouts(&["w00", "w01", "w03"], &["w02"], 2));
    settle(&mut one, &w);
    assert_eq!(own(&one), after);
    // The rack laid out for more, those there move to its new slots; nobody at home does.
    one.seat(&[rack()], &layouts(&["w00", "w01", "w03"], &["w02"], 12));
    settle(&mut one, &w);
    settle(&mut one, &w);
    let relaid = own(&one);
    assert_eq!(
        relaid["w02"].0,
        cluster.slot(&cluster.arrange(12), 0).position
    );
    for id in ["w00", "w01", "w03"] {
        assert_eq!(relaid[id], after[id], "{id}");
    }
}

#[test]
fn an_anchor_or_a_seat_out_of_bounds_is_left_out_and_its_workers_stand_at_home() {
    use tomori_protocol::cluster::{Anchor as Named, Layout};
    let w = workers(3);
    let far = Named::new("far", tomori_protocol::Pose::at([0.0, 1.4, -40.0]));
    let named = Named::new("bad name", tomori_protocol::Pose::at([0.0, 1.4, -4.0]));
    let cramped = Named::new("cramped", tomori_protocol::Pose::at([0.0, 1.4, -4.0]))
        .with_extent(0.1, 1.0, 1.0);
    let mut scene = Scene::new();
    scene.seat(
        &[far, named, cramped],
        &[
            Layout::new("home", 3)
                .with_seat("w00", 0)
                .with_seat("w01", 2)
                .with_seat("w02", 9999),
            Layout::new("far", 1).with_seat("w01", 0),
            Layout::new("nowhere", 1).with_seat("w02", 0),
        ],
    );
    settle(&mut scene, &w);
    let standing = own(&scene);
    // Home's seats stand; those at an anchor left out, or of a slot beyond what a client keeps, stand at home, in
    // slots this client gives them as no seat says otherwise.
    assert_eq!(
        standing["w00"].0,
        HomeArc::at(Anchor::AGENT, EYES, 0).to_array()
    );
    assert_eq!(scene.workers["w00"].at, None);
    assert_eq!(scene.workers["w01"].at, None);
    assert_eq!(scene.workers["w02"].at, None);
    assert!(standing.values().all(|(position, _)| position[2] > -10.0));
}

#[test]
fn each_cluster_gathers_its_own_more_sign_and_pointing_at_one_names_its_anchor() {
    use tomori_protocol::cluster::Cluster;
    let cluster = Cluster::new(&rack());
    let more = (1..64)
        .find(|span| cluster.arrange(*span).more().is_some())
        .unwrap();
    let count = more + 2;
    let w = workers(count);
    let ids: Vec<String> = w.nodes.iter().map(|node| node.id.clone()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut scene = Scene::new();
    scene.seat(&[rack()], &layouts(&[], &ids, count as u32));
    settle(&mut scene, &w);
    let signs: Vec<Station> = scene
        .stations()
        .iter()
        .filter(|station| station.more > 0)
        .cloned()
        .collect();
    assert_eq!(signs.len(), 1);
    let first = cluster.arrange(count).more().unwrap();
    assert_eq!(signs[0].more, count - first);
    assert_eq!(
        signs[0].position,
        cluster.slot(&cluster.arrange(count), first).position
    );
    // Pointed at from beneath, it names its anchor, lists its workers and highlights them, naming none of them.
    let ray = Ray {
        origin: Vec3::from_array(signs[0].position) - Vec3::Y * 0.8,
        direction: Vec3::Y,
    };
    assert_eq!(scene.point(Some(ray)), None);
    assert_eq!(scene.pointed_gathered(), Some("rack/a12"));
    scene.update(Some("root"), &w, 1.0 / 60.0);
    let listed = scene.stations().iter().find(|s| s.more > 0).unwrap();
    assert!(listed.title.contains(&format!("Title {}", ids[count - 1])));
    // Home's sign, pointed at, is named home; pointing away names none.
    assert_eq!(scene.point(None), None);
    assert_eq!(scene.pointed_gathered(), None);
    let (_, home_more) = thresholds();
    let w = workers(home_more + 2);
    let mut home = Scene::new();
    home.arrange(Anchor::AGENT, EYES, Some(ROOM), None);
    settle(&mut home, &w);
    let sign = home
        .stations()
        .iter()
        .find(|station| station.more > 0)
        .unwrap()
        .clone();
    home.point(Some(Ray {
        origin: Vec3::from_array(sign.position) - Vec3::Y * 0.8,
        direction: Vec3::Y,
    }));
    assert_eq!(home.pointed_gathered(), Some("home"));
}

#[test]
fn a_registered_placement_is_asked_again_only_for_a_worker_the_server_seats_anew_at_home() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted(AtomicUsize);
    impl crate::anchors::WorkerPlacement for Counted {
        fn place(&self, _origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            self.0.fetch_add(1, Ordering::Relaxed);
            Vec3::new(worker.slot as f32 * 0.4, 1.4, -3.0)
        }
    }
    let counted = std::sync::Arc::new(Counted(AtomicUsize::new(0)));
    let w = workers(3);
    let mut scene = Scene::new();
    scene.arrange(Anchor::AGENT, EYES, None, Some(counted.clone()));
    scene.seat(&[rack()], &layouts(&["w00", "w01"], &["w02"], 2));
    settle(&mut scene, &w);
    let asked = counted.0.load(Ordering::Relaxed);
    assert_eq!(asked, 2);
    // The one at the rack coming home is placed by it, with those already there.
    scene.seat(&[rack()], &layouts(&["w00", "w01", "w02"], &[], 3));
    settle(&mut scene, &w);
    settle(&mut scene, &w);
    assert_eq!(own(&scene)["w02"].0, [0.8, 1.4, -3.0]);
    assert!(counted.0.load(Ordering::Relaxed) > asked);
    // The rack's layout changing with nobody there asks nothing more.
    let asked = counted.0.load(Ordering::Relaxed);
    scene.seat(&[rack()], &layouts(&["w00", "w01", "w02"], &[], 4));
    settle(&mut scene, &w);
    assert_eq!(counted.0.load(Ordering::Relaxed), asked);
}

#[test]
fn a_worker_the_server_no_longer_seats_stays_where_it_stands_and_satellites_keep_to_their_slots() {
    let w = workers(3);
    let mut scene = Scene::new();
    scene.seat(&[rack()], &layouts(&["w00", "w01"], &["w02"], 2));
    settle(&mut scene, &w);
    // With the agent away, each satellite keeps to its worker's slot as the server seats it again.
    scene.set_agent(Some((Vec3::new(3.0, 1.5, -1.0), 0.4)));
    scene.update(Some("root"), &w, 1.0 / 60.0);
    assert_eq!(scene.satellites().len(), 3);
    let before = own(&scene);
    scene.seat(&[rack()], &[layouts(&[], &["w02", "w01"], 2).remove(1)]);
    settle(&mut scene, &w);
    assert!(
        scene
            .motes
            .iter()
            .all(|mote| mote.slot == scene.slots[&mote.id])
    );
    // The one no longer seated stays where it stood; the one seated at the rack goes there.
    let after = own(&scene);
    assert_eq!(after["w00"], before["w00"]);
    assert_eq!(scene.workers["w01"].at.as_deref(), Some("rack/a12"));
    assert_ne!(after["w01"], before["w01"]);
}
