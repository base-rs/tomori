// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The workers of a joined space, as its state lists them: their tree, their states and where the server seats them.

use super::*;
use tomori_protocol::{SpaceState, Worker as SpaceWorker, WorkerState};

const SPACE: &str = "space-1";

fn worker(id: &str, parent: Option<&str>, state: WorkerState) -> SpaceWorker {
    SpaceWorker {
        id: id.into(),
        parent: parent.map(Into::into),
        title: format!("{id} title"),
        state,
    }
}

fn space(workers: Vec<SpaceWorker>) -> SpaceState {
    SpaceState {
        id: SPACE.into(),
        workers,
        ..Default::default()
    }
}

fn stations(scene: &Scene) -> BTreeMap<&str, State> {
    scene
        .stations()
        .iter()
        .map(|station| (station.id.as_str(), station.state))
        .collect()
}

fn show(scene: &mut Scene, workers: &mut Workers, space: Option<(&SpaceState, bool)>) {
    for _ in 0..10 {
        let (root, workspace) = workers.update(space);
        scene.update(root, workspace, 1.0 / 60.0);
    }
}

#[test]
fn a_long_error_is_cut_on_a_character_boundary() {
    let mut state = space(vec![]);
    state.error = Some(format!("a{}", "é".repeat(2048)));
    let workspace = Workspace::from_space(&state, true);
    assert_eq!(workspace.error.as_ref().map(String::len), Some(4095));
    assert!(workspace.valid());
}

#[test]
fn a_space_without_workers_is_an_empty_trusted_root() {
    let state = space(vec![]);
    let mut workers = Workers::default();
    let (root, workspace) = workers.update(Some((&state, true)));
    assert_eq!(root, Some(SPACE));
    assert!(workspace.nodes.is_empty());
    assert!(workspace.valid());
    assert!(!workspace.stale && !workspace.truncated && workspace.error.is_none());
    let mut scene = Scene::new();
    show(&mut scene, &mut workers, Some((&state, true)));
    assert!(scene.stations().is_empty());
    assert!(scene.cubes().is_empty());
}

#[test]
fn a_top_level_worker_hangs_off_the_space() {
    let state = space(vec![worker("a", None, WorkerState::Running)]);
    let workspace = Workspace::from_space(&state, true);
    assert_eq!(workspace.nodes.len(), 1);
    assert_eq!(workspace.nodes[0].id, "a");
    assert_eq!(workspace.nodes[0].parent_id.as_deref(), Some(SPACE));
    assert_eq!(workspace.nodes[0].title, "a title");
    assert_eq!(workspace.nodes[0].state, State::Running);

    let mut scene = Scene::new();
    show(&mut scene, &mut Workers::default(), Some((&state, true)));
    assert_eq!(stations(&scene), BTreeMap::from([("a", State::Running)]));
    assert!(!scene.cubes().is_empty());
    assert!(scene.cubes().iter().all(Cube::valid));
}

#[test]
fn a_nested_worker_keeps_its_parent() {
    let state = space(vec![
        worker("a", None, WorkerState::Running),
        worker("b", Some("a"), WorkerState::Waiting),
    ]);
    let workspace = Workspace::from_space(&state, true);
    assert_eq!(workspace.nodes[1].parent_id.as_deref(), Some("a"));

    let mut scene = Scene::new();
    show(&mut scene, &mut Workers::default(), Some((&state, true)));
    assert_eq!(
        stations(&scene),
        BTreeMap::from([("a", State::Running), ("b", State::Waiting)])
    );
}

#[test]
fn every_worker_state_maps_to_its_station_state() {
    for (from, to) in [
        (WorkerState::Running, State::Running),
        (WorkerState::Waiting, State::Waiting),
        (WorkerState::Idle, State::Idle),
        (WorkerState::Completed, State::Completed),
        (WorkerState::Error, State::Error),
        (WorkerState::Stopped, State::Stopped),
        (WorkerState::Stalled, State::Stalled),
        (WorkerState::Unanswered, State::Unanswered),
        (WorkerState::Unknown, State::Unknown),
    ] {
        let workspace = Workspace::from_space(&space(vec![worker("a", None, from)]), true);
        assert_eq!(workspace.nodes[0].state, to);
    }
}

/// A stalled or unanswered worker is amber, deeper than a waiting one's, still at work, and drifts slowly. See
/// `design/events.md`.
#[test]
fn a_stalled_or_unanswered_worker_is_amber_and_still_at_work() {
    for state in [State::Stalled, State::Unanswered] {
        let colour = palette(state);
        assert_eq!(colour, Vec3::new(1.00, 0.55, 0.10));
        assert_ne!(colour, palette(State::Waiting));
        assert_ne!(colour, palette(State::Error));
        assert!(is_working(state) && !is_terminal(state));
        let offset = flock_offset(3, 0.5, 1.0, state);
        assert!(offset.is_finite() && offset.length() < 0.2);
    }
    assert!(!is_working(State::Completed));
}

#[test]
fn state_changes_reach_the_scene_and_removals_retire_workers() {
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    let running = space(vec![
        worker("a", None, WorkerState::Running),
        worker("b", Some("a"), WorkerState::Running),
    ]);
    show(&mut scene, &mut workers, Some((&running, true)));
    let completed = space(vec![
        worker("a", None, WorkerState::Running),
        worker("b", Some("a"), WorkerState::Completed),
    ]);
    show(&mut scene, &mut workers, Some((&completed, true)));
    assert_eq!(
        stations(&scene),
        BTreeMap::from([("a", State::Running), ("b", State::Completed)])
    );
    let failed = space(vec![worker("a", None, WorkerState::Error)]);
    show(&mut scene, &mut workers, Some((&failed, true)));
    assert_eq!(scene.workers["a"].state, State::Error);
    assert!(scene.workers["b"].retired.is_some());
}

#[test]
fn workers_are_pointed_at_where_they_stand_and_only_while_they_stay() {
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    let both = space(vec![
        worker("a", None, WorkerState::Running),
        worker("b", None, WorkerState::Running),
    ]);
    show(&mut scene, &mut workers, Some((&both, true)));
    let anchor = |scene: &Scene, id: &str| {
        Vec3::from_array(
            scene
                .stations()
                .iter()
                .find(|station| station.id == id)
                .unwrap()
                .position,
        )
    };
    let (a, b) = (anchor(&scene, "a"), anchor(&scene, "b"));
    let eye = Vec3::new(0.0, 1.6, 0.0);
    let at = |target: Vec3| Ray::new(eye, target - eye);
    // Each worker's flock, leader and sign.
    assert_eq!(scene.targets().len(), 6);
    // Pointing tests what the targets describe.
    // Including a ray through both, which points at the nearer.
    let through = Ray::new(a + (a - b), b - a);
    for ray in [at(a), at(b), at(Vec3::ZERO), at((a + b) / 2.0), through] {
        let expected = ray
            .and_then(|ray| crate::pointing::nearest(&ray, &scene.targets()))
            .map(|hit| hit.key);
        assert_eq!(scene.point(ray), expected.as_deref());
    }
    assert_eq!(scene.point(at(a)), Some("a"));
    // The ray ends where it passes nearest the worker's anchor.
    assert!(scene.hit_point().unwrap().distance(a) < 1e-4);
    assert_eq!(scene.point(at(b)), Some("b"));
    // Passing within the pointing radius of a worker's anchor still points at it; the floor is nothing.
    let aside = (b - eye).cross(Vec3::Y).normalize();
    assert_eq!(
        scene.point(at(b + aside * (POINTING_RADIUS * 0.8))),
        Some("b")
    );
    assert!(scene.hit_point().unwrap().distance(b) < POINTING_RADIUS * 0.8);
    assert_eq!(scene.point(Ray::new(eye, Vec3::NEG_Y)), None);
    assert_eq!(scene.point(None), None);
    assert_eq!(scene.hit_point(), None);

    // The worker pointed at is drawn larger, and stays valid geometry.
    let size = |scene: &Scene| scene.cubes().iter().map(|cube| cube.scale[0]).sum::<f32>();
    show(&mut scene, &mut workers, Some((&both, true)));
    let plain = size(&scene);
    scene.point(at(a));
    show(&mut scene, &mut workers, Some((&both, true)));
    let pointed = size(&scene);
    assert!(pointed > plain * 1.05, "{pointed} {plain}");
    assert!(scene.cubes().iter().all(Cube::valid));

    // A worker that leaves can no longer be pointed at.
    let only_b = space(vec![worker("b", None, WorkerState::Running)]);
    show(&mut scene, &mut workers, Some((&only_b, true)));
    assert!(scene.workers["a"].retired.is_some());
    assert_eq!(scene.targets().len(), 3);
    assert_ne!(scene.point(at(a)), Some("a"));
}

#[test]
fn a_worker_pointed_at_by_its_flock_leader_or_sign_highlights_all_three_for_either_hand() {
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    let both = space(vec![
        worker("a", None, WorkerState::Running),
        worker("b", None, WorkerState::Running),
    ]);
    show(&mut scene, &mut workers, Some((&both, true)));
    let station = |scene: &Scene, id: &str| {
        scene
            .stations()
            .iter()
            .find(|station| station.id == id)
            .unwrap()
            .clone()
    };
    let highlighted = |scene: &Scene| ["a", "b"].map(|id| station(scene, id).highlighted);
    assert_eq!(highlighted(&scene), [false, false]);
    let eye = Vec3::new(0.0, 1.6, 0.0);
    let at = |target: Vec3| Ray::new(eye, target - eye);
    let a = station(&scene, "a");
    let (anchor, drawn) = (Vec3::from_array(a.position), a.drawn());
    // Its flock, its leader half way up to its sign, and its sign's middle each point at it, the ray ending there.
    for (target, part) in [
        (anchor, "flock"),
        (anchor + Vec3::Y * (SIGN_LIFT * drawn / 2.0), "leader"),
        (
            anchor + Vec3::Y * ((SIGN_LIFT + SIGN_TOP) * drawn / 2.0),
            "sign",
        ),
    ] {
        assert_eq!(scene.point(at(target)), Some("a"), "{part}");
        assert!(
            scene.hit_point().unwrap().distance(target) < SIGN_HALF * drawn,
            "{part}"
        );
        show(&mut scene, &mut workers, Some((&both, true)));
        // Its sign and leader are drawn highlighted with its flock; the other's are not.
        assert_eq!(highlighted(&scene), [true, false], "{part}");
    }
    // Above its tallest sign, nothing.
    assert_eq!(
        scene.point(at(anchor + Vec3::Y * (SIGN_TOP * drawn + 0.3))),
        None
    );
    show(&mut scene, &mut workers, Some((&both, true)));
    assert_eq!(highlighted(&scene), [false, false]);
    // Each hand highlights what it points at, and only that.
    let b = Vec3::from_array(station(&scene, "b").position);
    scene.point_with(Side::Right, at(anchor));
    scene.point_with(Side::Left, at(b));
    show(&mut scene, &mut workers, Some((&both, true)));
    assert_eq!(highlighted(&scene), [true, true]);
    scene.point_with(Side::Right, None);
    show(&mut scene, &mut workers, Some((&both, true)));
    assert_eq!(highlighted(&scene), [false, true]);
}

#[test]
fn leaving_the_space_clears_the_workers() {
    let state = space(vec![worker("a", None, WorkerState::Running)]);
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    show(&mut scene, &mut workers, Some((&state, true)));
    assert!(!scene.stations().is_empty());
    let (root, workspace) = workers.update(None);
    assert_eq!(root, None);
    assert!(workspace.nodes.is_empty());
    show(&mut scene, &mut workers, None);
    assert!(scene.stations().is_empty());
    assert!(scene.cubes().is_empty());
}

#[test]
fn joining_another_space_is_a_new_root() {
    let first = space(vec![worker("a", None, WorkerState::Running)]);
    let second = SpaceState {
        id: "space-2".into(),
        ..space(vec![worker("c", None, WorkerState::Idle)])
    };
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    show(&mut scene, &mut workers, Some((&first, true)));
    show(&mut scene, &mut workers, Some((&second, true)));
    assert_eq!(workers.update(Some((&second, true))).0, Some("space-2"));
    assert_eq!(stations(&scene), BTreeMap::from([("c", State::Idle)]));
}

#[test]
fn a_lost_connection_is_stale_and_a_space_error_is_partial() {
    let state = space(vec![worker("a", None, WorkerState::Running)]);
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    show(&mut scene, &mut workers, Some((&state, true)));
    assert!(workers.update(Some((&state, false))).1.stale);
    show(&mut scene, &mut workers, Some((&state, false)));
    assert_eq!(stations(&scene), BTreeMap::from([("a", State::Unknown)]));

    // With the backend lost, an empty worker list is no evidence that the worker finished.
    let errored = SpaceState {
        error: Some("é".repeat(3000)),
        ..space(vec![])
    };
    let (_, workspace) = workers.update(Some((&errored, true)));
    assert!(workspace.valid());
    assert!(workspace.error.as_ref().unwrap().len() <= 4096);
    show(&mut scene, &mut workers, Some((&errored, true)));
    assert!(scene.workers["a"].retired.is_none());
}

#[test]
fn worker_freshness_is_independent_of_an_unrelated_space_error() {
    let mut state = space(vec![worker("a", None, WorkerState::Completed)]);
    state.error = Some("unrelated agent error".into());
    state.workers_authoritative = Some(true);
    let mut workers = Workers::default();
    let mut scene = Scene::new();
    show(&mut scene, &mut workers, Some((&state, true)));
    assert!(scene.workers.contains_key("a"));

    // A partial list does not prove that the absent worker was deleted.
    state.workers.clear();
    state.workers_authoritative = Some(false);
    show(&mut scene, &mut workers, Some((&state, true)));
    assert!(scene.workers["a"].retired.is_none());
    assert_eq!(
        workers.update(Some((&state, true))).1.error.as_deref(),
        Some("unrelated agent error")
    );

    // A later authoritative worker update may retire it even while the
    // unrelated error remains visible; freshness alone changed here.
    state.workers_authoritative = Some(true);
    show(&mut scene, &mut workers, Some((&state, true)));
    assert!(scene.workers["a"].retired.is_some());
    assert_eq!(
        workers.update(Some((&state, true))).1.error.as_deref(),
        Some("unrelated agent error")
    );
}

#[test]
fn out_of_contract_workers_are_omitted_as_truncated() {
    let mut list: Vec<_> = (0..MAX_NODES + 3)
        .map(|i| worker(&format!("w{i}"), None, WorkerState::Running))
        .collect();
    list.insert(0, worker("", None, WorkerState::Running));
    let long = "p".repeat(MAX_ID_BYTES + 1);
    list.insert(1, worker("bad-parent", Some(&long), WorkerState::Running));
    list.last_mut().unwrap().title = "t".repeat(400);
    let workspace = Workspace::from_space(&space(list.clone()), true);
    assert_eq!(workspace.nodes.len(), MAX_NODES);
    // The server lists the oldest first; the newest are kept.
    assert_eq!(workspace.nodes[0].id, "w3");
    assert_eq!(
        workspace.nodes[MAX_NODES - 1].id,
        format!("w{}", MAX_NODES + 2)
    );
    assert_eq!(workspace.nodes[MAX_NODES - 1].title.chars().count(), 160);
    assert!(workspace.truncated);
    assert!(workspace.valid());

    let mut scene = Scene::new();
    show(
        &mut scene,
        &mut Workers::default(),
        Some((&space(list), true)),
    );
    assert_eq!(scene.stations().len(), MAX_NODES);
}

#[test]
fn a_parent_cycle_is_rejected_by_validation() {
    let state = space(vec![
        worker("a", Some("b"), WorkerState::Running),
        worker("b", Some("a"), WorkerState::Running),
    ]);
    assert!(!Workspace::from_space(&state, true).valid());
    let mut scene = Scene::new();
    show(&mut scene, &mut Workers::default(), Some((&state, true)));
    assert!(scene.stations().is_empty());
}

#[test]
fn a_worker_whose_parent_is_missing_keeps_that_parent() {
    let state = space(vec![worker("b", Some("gone"), WorkerState::Running)]);
    let workspace = Workspace::from_space(&state, true);
    assert!(workspace.valid());
    assert_eq!(workspace.nodes[0].parent_id.as_deref(), Some("gone"));
}

#[test]
fn timers_stand_as_dials_above_the_agent_among_the_workers_and_the_nearer_is_pointed_at() {
    use tomori_protocol::{Timer, TimerState};
    const NOW: f64 = 1_800_000_000.0;
    let timers: Vec<Timer> = (0..8)
        .map(|index| {
            Timer::new(
                format!("t{index}"),
                "every hour",
                "when present",
                Some(NOW as u64 + 600),
                TimerState::Waiting,
            )
        })
        .collect();
    // Many workers, and eight timers: the dials stand where they would with none, clear of every worker.
    let mut state = space(
        (0..MAX_NODES)
            .map(|index| worker(&format!("w{index:02}"), None, WorkerState::Running))
            .collect(),
    );
    state.timers = timers.clone();
    let mut scene = Scene::new();
    let mut workers = Workers::default();
    let reach = crate::dials::SWARM_REACH;
    scene.set_timers(&state.timers, Anchor::AGENT, reach, NOW);
    scene.set_timers(&state.timers, Anchor::AGENT, reach, NOW);
    show(&mut scene, &mut workers, Some((&state, true)));
    for _ in 0..40 {
        let (root, workspace) = workers.update(Some((&state, true)));
        scene.update(root, workspace, 0.05);
    }
    assert_eq!(scene.dials().len(), 8);
    let places = crate::dials::places(8, &scene.room);
    for (dial, (place, _)) in scene.dials().iter().zip(&places) {
        assert_eq!(Vec3::from_array(dial.position), *place);
    }
    assert!(scene.cubes().len() <= MAX_CUBES);
    assert!(scene.cubes().len() > MAX_NODES * PARTICLES);
    for station in scene.stations() {
        for (place, _) in &places {
            assert!(Vec3::from_array(station.position).distance(*place) > 0.75);
        }
    }
    // The dial the ray meets, nearer than any worker, is pointed at, and the ray ends on it.
    let first = places[0].0;
    let eyes = crate::anchors::EYES;
    assert_eq!(scene.point(Ray::new(eyes, first - eyes)), None);
    assert_eq!(scene.pointed_timer(), Some("t0"));
    assert!(scene.hit_point().unwrap().distance(first) < 1e-3);
    // A worker nearer along the same ray is pointed at instead.
    let (id, nearer) = scene
        .workers
        .iter()
        .map(|(id, worker)| (id.clone(), worker.anchor))
        .next()
        .unwrap();
    let behind = nearer + (nearer - eyes).normalize() * 0.5;
    let mut close = Scene::new();
    close
        .workers
        .insert(id.clone(), scene.workers.remove(&id).unwrap());
    close.dials = std::mem::take(&mut scene.dials);
    close.dials.update(
        &timers[..1],
        true,
        &crate::dials::Room::open(
            Anchor {
                position: behind - Vec3::Y * (reach.y + crate::dials::CLEAR + crate::dials::RADIUS),
                ..Anchor::AGENT
            },
            eyes,
        ),
        NOW,
        1.0,
        &mut Vec::new(),
    );
    for _ in 0..40 {
        close.dials.update(
            &timers[..1],
            true,
            &crate::dials::Room::open(
                Anchor {
                    position: behind
                        - Vec3::Y * (reach.y + crate::dials::CLEAR + crate::dials::RADIUS),
                    ..Anchor::AGENT
                },
                eyes,
            ),
            NOW,
            0.05,
            &mut Vec::new(),
        );
    }
    assert_eq!(
        close.point(Ray::new(eyes, nearer - eyes)),
        Some(id.as_str())
    );
    assert_eq!(close.pointed_timer(), None);
    // Nothing pointed at, neither.
    assert_eq!(scene.point(None), None);
    assert_eq!(scene.pointed_timer(), None);
    // An avatar's reach as it grows is taken at once, and as it shrinks only past SETTLE, so that an animated one
    // does not move the dials back and forth.
    let grown = reach + Vec3::Y * 0.2;
    scene.set_timers(&timers, Anchor::AGENT, grown, NOW);
    assert_eq!(scene.room.agent, grown);
    scene.set_timers(&timers, Anchor::AGENT, grown - Vec3::Y * 0.1, NOW);
    assert_eq!(scene.room.agent, grown);
    scene.set_timers(&timers, Anchor::AGENT, reach, NOW);
    assert_eq!(scene.room.agent, grown);
    scene.set_timers(&timers, Anchor::AGENT, reach - Vec3::Y * 0.1, NOW);
    assert_eq!(scene.room.agent, reach - Vec3::Y * 0.1);
    scene.set_timers(&timers, Anchor::AGENT, reach, NOW);
    // Outside a space, or in another, the dials are forgotten.
    let (root, workspace) = workers.update(None);
    scene.update(root, workspace, 0.05);
    assert!(scene.dials().is_empty());
    scene.set_timers(&timers, Anchor::AGENT, reach, NOW);
    let other = SpaceState {
        id: "space-2".into(),
        timers: timers[..1].to_vec(),
        ..Default::default()
    };
    let (root, workspace) = workers.update(Some((&other, true)));
    scene.update(root, workspace, 0.05);
    assert_eq!(scene.dials().len(), 8);
    assert!(scene.dials()[0].opacity < 0.5, "growing in afresh");
}

/// The farmyard's home, its eyes at the arrival, and its ring, all but straight behind the agent.
const FARMYARD_HOME: Vec3 = Vec3::new(-2.5, 1.5, -0.84);
const FARMYARD_EYES: Vec3 = Vec3::new(0.6, 1.6, -2.4);

/// The dials and the workers in `scene`, with `workers` workers and eight timers, laid out by `arrange`, stand clear
/// of one another and their signs, and within the room's ceiling and floor.
fn clear_of_every_worker(
    arrange: impl Fn(&mut Scene),
    home: Anchor,
    eyes: Vec3,
    walls: &[tomori_protocol::Wall],
) -> Vec<Vec3> {
    use tomori_protocol::{Timer, TimerState};
    const NOW: f64 = 1_800_000_000.0;
    let timers: Vec<Timer> = (0..8)
        .map(|index| {
            Timer::new(
                format!("t{index}"),
                "every hour",
                "when present",
                Some(NOW as u64 + 600),
                TimerState::Waiting,
            )
        })
        .collect();
    let mut stood = Vec::new();
    for count in [0, 8, MAX_NODES] {
        let mut state = space(
            (0..count)
                .map(|index| worker(&format!("w{index:02}"), None, WorkerState::Running))
                .collect(),
        );
        state.timers = timers.clone();
        let mut scene = Scene::new();
        arrange(&mut scene);
        let mut workers = Workers::default();
        for _ in 0..60 {
            scene.set_timers(&state.timers, home, crate::dials::SWARM_REACH, NOW);
            scene.set_walls(walls);
            let (root, workspace) = workers.update(Some((&state, true)));
            scene.update(root, workspace, 0.05);
        }
        assert_eq!(scene.dials().len(), 8);
        let dials: Vec<crate::dials::Extent> = scene
            .dials()
            .iter()
            .map(|dial| crate::dials::Extent::dial(Vec3::from_array(dial.position), dial.scale))
            .collect();
        for (index, dial) in dials.iter().enumerate() {
            assert!(dial.low >= scene.room.floor - 1e-4);
            assert!(
                scene
                    .room
                    .ceiling
                    .is_none_or(|ceiling| dial.high <= ceiling + 1e-4)
            );
            // Clear of the agent's reach, above or below it.
            assert!(
                dial.low >= home.position.y + crate::dials::SWARM_REACH.y
                    || dial.high <= home.position.y - crate::dials::SWARM_REACH.y
            );
            for other in &dials[..index] {
                assert!(!dial.overlaps(other, 0.0));
            }
            for slot in crate::dials::Extent::walls(walls) {
                assert!(
                    !dial.overlaps(&slot, 0.0),
                    "{count} workers: a wall's slot at {} and a dial at {}",
                    slot.centre,
                    dial.centre
                );
            }
            for (id, worker) in &scene.workers {
                let extent = crate::dials::Extent::worker(worker.place, worker.scale, eyes);
                assert!(
                    !dial.overlaps(&extent, 0.0),
                    "{count} workers: {id} at {} and a dial at {}",
                    worker.place,
                    dial.centre
                );
            }
        }
        stood = dials.iter().map(|dial| dial.centre).collect();
    }
    stood
}

#[test]
fn on_today_s_arc_the_dials_stand_above_the_agent_clear_of_every_worker() {
    let stood = clear_of_every_worker(|_| {}, Anchor::AGENT, crate::anchors::EYES, &[]);
    assert!(stood.iter().all(|at| at.y > Anchor::AGENT.position.y));
    // With the status wall beneath the agent, where the server stands it by default.
    let stood = clear_of_every_worker(
        |_| {},
        Anchor::AGENT,
        crate::anchors::EYES,
        &[status_wall(Anchor::AGENT.position, crate::anchors::EYES)],
    );
    assert!(stood.iter().all(|at| at.y > Anchor::AGENT.position.y));
}

#[test]
fn in_a_home_with_room_the_dials_stand_clear_of_the_arc_grid_and_compact_steps() {
    let home = Anchor::AGENT;
    let stood = clear_of_every_worker(
        |scene| {
            scene.arrange(
                home,
                crate::anchors::EYES,
                Some(tomori_protocol::cluster::Extent::new(4.2, 1.6, 2.0)),
                None,
            );
        },
        home,
        crate::anchors::EYES,
        &[status_wall(home.position, crate::anchors::EYES)],
    );
    assert_eq!(stood.len(), 8);
}

/// The space's own status wall, the Tomori wall, where the server stands it by default: 0.88 m beneath the agent's
/// home `at`, curving about `eyes`.
fn status_wall(at: Vec3, eyes: Vec3) -> tomori_protocol::Wall {
    let middle = at - Vec3::Y * 0.88;
    tomori_protocol::Wall::new(
        tomori_protocol::TOMORI_WALL,
        tomori_protocol::WallKind::Tomori,
        tomori_protocol::Pose::upright_facing(middle.to_array(), [eyes.x, middle.y, eyes.z]),
    )
    .with_shape(tomori_protocol::WallShape::Arc { radius: 2.3 })
}

fn farmyard() -> (Anchor, impl Fn(&mut Scene)) {
    let pose = tomori_protocol::Pose::at(FARMYARD_HOME.to_array())
        .facing_towards(FARMYARD_EYES.to_array());
    let home = Anchor::from_pose(&pose).unwrap();
    (home, move |scene: &mut Scene| {
        scene.arrange_as(
            home,
            FARMYARD_EYES,
            None,
            (tomori_protocol::cluster::Formation::Ring, Some(300.0)),
            None,
        );
    })
}

#[test]
fn in_the_farmyard_s_ring_the_dials_stand_beneath_the_agent_within_its_height() {
    // Its status wall is its notice board, one of its own walls, across the yard.
    let board = tomori_protocol::Wall::new(
        "noticeboard",
        tomori_protocol::WallKind::Environment,
        tomori_protocol::Pose::upright_facing([-4.0, 1.5, -6.0], FARMYARD_EYES.to_array()),
    )
    .with_status(true);
    let (home, arrange) = farmyard();
    let stood = clear_of_every_worker(arrange, home, FARMYARD_EYES, &[board]);
    // Above the agent the ring keeps nothing over 2.5 m, so the dials stand beneath it.
    assert!(stood.iter().all(|at| at.y < FARMYARD_HOME.y));
}

#[test]
fn in_a_ring_over_the_default_status_wall_the_dials_stand_clear_of_its_slots() {
    let (home, arrange) = farmyard();
    let wall = status_wall(FARMYARD_HOME, FARMYARD_EYES);
    let stood = clear_of_every_worker(arrange, home, FARMYARD_EYES, std::slice::from_ref(&wall));
    assert_eq!(stood.len(), 8);
    // And at the default home's height, lower over its floor.
    let low = Vec3::new(FARMYARD_HOME.x, 1.4, FARMYARD_HOME.z);
    let pose = tomori_protocol::Pose::at(low.to_array()).facing_towards(FARMYARD_EYES.to_array());
    let home = Anchor::from_pose(&pose).unwrap();
    clear_of_every_worker(
        |scene| {
            scene.arrange_as(
                home,
                FARMYARD_EYES,
                None,
                (tomori_protocol::cluster::Formation::Ring, Some(300.0)),
                None,
            );
        },
        home,
        FARMYARD_EYES,
        &[status_wall(low, FARMYARD_EYES)],
    );
}
