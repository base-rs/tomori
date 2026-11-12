// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::Arc;

use super::*;
use crate::chicken::tests::chicken;

fn flock(space: &str) -> Flock {
    Flock::new(seed(space), Arc::new(chicken()), 0)
}

/// Run `flock` from `from` to `to` in steps of 50 ms, as the side does, calling `each` at every step.
fn run(flock: &mut Flock, (from, to): (u64, u64), mut each: impl FnMut(&Flock, u64)) {
    let mut now = from;
    while now < to {
        flock.update(now);
        each(flock, now);
        now += 50;
    }
}

#[test]
fn a_flock_is_the_same_for_the_same_space() {
    let one = flock("space-1");
    assert!((FEWEST..=MOST).contains(&one.hens().len()));
    assert_eq!(one.hens().iter().filter(|hen| hen.cone).count(), 1);
    assert_eq!(one.hens(), flock("space-1").hens());
    assert_ne!(one.hens(), flock("space-2").hens());
    assert!(one.hens().iter().all(|hen| (0.86..1.2).contains(&hen.pace)));
    // Each stands somewhere clear, at first.
    for hen in one.hens() {
        let at = Vec2::new(hen.behaviour.at[0], hen.behaviour.at[2]);
        assert!(Flock::clear(at), "{at}");
    }
    assert_eq!(one.state(5).clock, 5);
    assert_eq!(one.state(5).chickens, one.hens());
}

#[test]
fn chickens_wander_clear_of_the_yard_and_each_other_and_visit_the_coop_now_and_then() {
    for space in ["space-1", "space-2", "space-3", "space-4"] {
        let mut flock = flock(space);
        let mut roosted = false;
        let mut walked = false;
        let mut closest = f32::INFINITY;
        run(&mut flock, (0, 20 * 60_000), |flock, now| {
            for (index, hen) in flock.hens().iter().enumerate() {
                let here = flock.position(index, now);
                roosted |= hen.behaviour.doing == Doing::Roost;
                walked |= matches!(hen.behaviour.doing, Doing::Walk { .. });
                if here.y < layout::ON_GROUND && hen.behaviour.doing != Doing::Roost {
                    // On the ground, it is in the yard, clear of what stands there, but at the ramp's foot.
                    let point = Vec2::new(here.x, here.z);
                    assert!(AREA.grown(0.05).contains(point), "{space} {index} {point}");
                    let ramp = layout::ramp().grown(CLEARANCE + 0.3);
                    assert!(
                        ramp.contains(point)
                            || layout::obstacles()
                                .iter()
                                .all(|obstacle| !obstacle.grown(CLEARANCE - 0.1).contains(point)),
                        "{space} {index} {point}"
                    );
                    for other in (0..flock.hens().len()).filter(|other| *other != index) {
                        let there = flock.position(other, now);
                        if there.y < layout::ON_GROUND {
                            closest = closest.min(point.distance(Vec2::new(there.x, there.z)));
                        }
                    }
                }
            }
        });
        assert!(walked && roosted, "{space}");
        // Chickens pass near each other, but never through.
        assert!(closest > 0.15, "{space} {closest}");
    }
}

/// Each walk to `points` in turn, from `start`, each begun where the last stopped, as the flock begins them: where the
/// chicken stops after each.
fn walked(
    chicken: &Chicken,
    start: Behaviour,
    points: &[[f32; 3]],
) -> Vec<crate::behaviour::Placed> {
    let mut behaviour = start;
    let mut stops = Vec::new();
    for to in points {
        behaviour.doing = Doing::Walk { to: *to };
        let end = behaviour.ends(chicken, 1.0).unwrap();
        let placed = behaviour.place(chicken, 1.0, end);
        stops.push(placed);
        behaviour = Behaviour {
            since: end,
            at: placed.position.to_array(),
            heading: placed.heading,
            doing: Doing::Stand,
        };
    }
    stops
}

#[test]
fn a_hen_nests_inside_her_box_on_its_bottom_and_turns_round_in_it_clear_of_its_boards() {
    let chicken = chicken();
    let (start, end) = crate::chicken::seconds(crate::chicken::WALK);
    let standing = crate::chicken::tests::walking(&chicken, start);
    let walking: Vec<Vec3> = (0..=36)
        .flat_map(|frame| {
            crate::chicken::tests::walking(&chicken, start + (end - start) * frame as f32 / 36.0)
        })
        .collect();
    // `points` of a chicken at `at`, facing `heading`.
    let posed = |points: &[Vec3], at: Vec3, heading: f32| -> Vec<Vec3> {
        let turn = glam::Quat::from_rotation_y(heading);
        points.iter().map(|point| at + turn * *point).collect()
    };
    let turns: Vec<f32> = (0..36)
        .map(|step| step as f32 / 36.0 * std::f32::consts::TAU)
        .collect();
    let approach = [
        layout::RAMP_FOOT.x,
        0.0,
        layout::ramp().grown(CLEARANCE).high.y + 0.05,
    ];
    let door = (layout::DOOR - Vec3::Z * 0.15).to_array();
    let board = layout::NEST_BOARD;
    let mut checked = 0;
    for (roost, inside) in layout::NESTS.iter().enumerate() {
        let entry = ENTRIES[roost].unwrap();
        // From about the yard, facing every way, as she sets off for the coop.
        for (from, facing) in [
            (Vec2::new(0.0, -6.0), 0.0),
            (Vec2::new(-1.0, -4.0), 2.0),
            (Vec2::new(2.0, -8.5), 4.0),
            (Vec2::new(-3.6, -5.0), 1.0),
            (Vec2::new(-1.3, -6.6), 5.0),
        ] {
            let start = Behaviour {
                since: 0,
                at: [from.x, layout::underfoot(from), from.y],
                heading: facing,
                doing: Doing::Stand,
            };
            let points = [
                approach,
                layout::RAMP_FOOT.to_array(),
                door,
                entry,
                ROOSTS[roost],
                entry,
            ];
            let stops = walked(&chicken, start, &points);
            let (arrived, nested, left) = (stops[3], stops[4], stops[5]);
            // On her box's bottom, standing and turning round, and as her last steps in end, she is all inside it,
            // clear of its sides, back and lip, and above its bottom.
            assert_eq!(nested.position.y, layout::NEST_FLOOR);
            let inside_box = inside.grown(-0.005);
            let mut within = |point: &Vec3| {
                assert!(
                    inside_box.contains(Vec2::new(point.x, point.z))
                        && point.y >= layout::NEST_FLOOR - 0.002,
                    "{roost} {from} {point} {nested:?}"
                );
                checked += 1;
            };
            for heading in &turns {
                posed(&standing, nested.position, *heading)
                    .iter()
                    .for_each(&mut within);
            }
            posed(&walking, nested.position, nested.heading)
                .iter()
                .for_each(&mut within);
            // In front of it, on the coop's floor, as she turns to walk in and to walk out, she is clear of its lip.
            for stopped in [arrived, left] {
                assert_eq!(stopped.position.y, layout::COOP_FLOOR);
                for heading in &turns {
                    for point in posed(&standing, stopped.position, *heading) {
                        assert!(
                            point.z > inside.high.y + board + 0.005,
                            "{roost} {from} {point}"
                        );
                    }
                }
            }
        }
    }
    assert!(checked > 100_000, "{checked}");
}

#[test]
fn chickens_walk_round_the_workbench_never_on_it_nor_through_it() {
    let flock = flock("space-1");
    // Its pieces are among what they keep clear of; their part of the yard stops short of most of it, and where it
    // reaches the bench's end, by the left fence, they keep their clearance from it.
    let pieces = layout::workbench();
    assert!(
        pieces
            .iter()
            .all(|piece| layout::obstacles().contains(piece))
    );
    for piece in &pieces {
        for point in [piece.low, piece.high, piece.centre()] {
            assert!(!Flock::clear(point), "{point}");
        }
    }
    let end = Vec2::new(-4.3, -1.9);
    assert!(AREA.contains(end) && !Flock::clear(end));
    assert!(pieces[0].grown(CLEARANCE).contains(end));
    // In the yard before it they may stand, but walk neither onto it nor through it into its corner.
    let yard = Vec2::new(-3.8, -2.3);
    assert!(Flock::clear(yard));
    assert!(!flock.path_clear(0, yard, Vec2::new(-4.8, 1.4), 0));
    assert!(!flock.path_clear(0, yard, layout::on_workbench(2.0, -0.6), 0));
    // A chicken by it is said to be by the workbench.
    assert_eq!(near(Vec3::new(-3.9, 0.0, -2.2)), "by the workbench");
}

#[test]
fn a_chicken_pointed_at_is_confused_until_its_participant_looks_away() {
    let mut flock = flock("space-1");
    let notice = |chicken| Intent::Notice {
        notice: chicken,
        toward: [0.0, 0.0],
    };
    // At once, from the next moment, it turns to them.
    let (chicken, confused) = flock.ask(1, notice(0), 10_000).unwrap();
    assert_eq!((chicken, confused.since), (0, 10_000 + LEAD));
    assert_eq!(confused.doing, Doing::Confused { toward: [0.0, 0.0] });
    flock.update(10_000 + LEAD);
    // Again too soon, from the same participant, of it or another, nothing.
    assert!(flock.ask(1, notice(0), 11_000).is_none());
    assert!(flock.ask(1, notice(1), 11_000).is_none());
    // Looking away, it recovers; looking away again, nothing.
    let (_, recovering) = flock.ask(1, Intent::Away { away: 0 }, 12_000).unwrap();
    assert_eq!(recovering.doing, Doing::Recover);
    assert!(flock.ask(1, Intent::Away { away: 0 }, 12_100).is_none());
    // A chicken the flock does not have, or a place that is no place, nothing.
    assert!(flock.ask(2, notice(MOST + 1), 20_000).is_none());
    assert!(
        flock
            .ask(2, Intent::Away { away: MOST + 1 }, 20_000)
            .is_none()
    );
    assert!(
        flock
            .ask(
                2,
                Intent::Notice {
                    notice: 1,
                    toward: [f32::NAN, 0.0]
                },
                20_000
            )
            .is_none()
    );
    // Far away is taken as at the edge of reach.
    let (_, far) = flock
        .ask(
            2,
            Intent::Notice {
                notice: 1,
                toward: [1e9, -1e9],
            },
            20_000,
        )
        .unwrap();
    assert_eq!(
        far.doing,
        Doing::Confused {
            toward: [REACH, -REACH]
        }
    );
    // Held long enough, it is startled; and pointed at again, as it cannot be startled again so soon, it recovers of
    // its own accord when nobody has said they looked away for too long.
    let changes = flock.update(20_000 + LEAD + STARTLE);
    assert!(changes.iter().any(
        |(chicken, behaviour)| *chicken == 1 && matches!(behaviour.doing, Doing::Sprint { .. })
    ));
    flock.update(30_000);
    flock.update(40_000);
    let (_, again) = flock.ask(5, notice(1), 40_000).unwrap();
    assert!(matches!(again.doing, Doing::Confused { .. }));
    assert!(
        flock
            .update(40_000 + LEAD + STARTLE)
            .iter()
            .all(|(chicken, _)| *chicken != 1)
    );
    let changes = flock.update(40_000 + LEAD + HOLD);
    assert!(
        changes
            .iter()
            .any(|(chicken, behaviour)| *chicken == 1 && behaviour.doing == Doing::Recover),
        "{changes:?}"
    );
    // Another participant looking away from a chicken they do not point at changes nothing.
    assert!(flock.ask(3, Intent::Away { away: 1 }, 40_000).is_none());
}

#[test]
fn a_chicken_pointed_at_long_enough_or_by_two_is_startled_rarely() {
    let mut flock = flock("space-1");
    let notice = |chicken| Intent::Notice {
        notice: chicken,
        toward: [0.0, 0.0],
    };
    flock.ask(1, notice(0), 10_000).unwrap();
    assert!(
        flock.update(10_000 + STARTLE - 100).is_empty()
            || !flock.hens()[0]
                .behaviour
                .doing
                .eq(&Doing::Sprint { away: [0.0, 0.0] })
    );
    let changes = flock.update(10_000 + STARTLE + 100);
    let sprint = changes.iter().find(|(chicken, _)| *chicken == 0).unwrap().1;
    assert!(matches!(sprint.doing, Doing::Sprint { .. }));
    // Running, it pays no heed to pointing.
    assert!(flock.ask(2, notice(0), 15_000).is_none());
    // Two at once startle another at once, but not one startled lately.
    flock.ask(1, notice(1), 30_000).unwrap();
    flock.ask(2, notice(1), 30_000).unwrap();
    let changes = flock.update(30_000 + LEAD);
    assert!(changes.iter().any(
        |(chicken, behaviour)| *chicken == 1 && matches!(behaviour.doing, Doing::Sprint { .. })
    ));
    // Inspected, she says she was startled, and how long ago, and that nobody points at her now.
    let details = flock.inspect(&flock.reference(1), 32_000 + LEAD).unwrap();
    assert_eq!(details["startled_seconds_ago"], 2);
    assert_eq!(details["pointed_at_by"], 0);
    assert_eq!(details["doing"], "running off, startled");
    let mut flock = run_until_standing(flock, 0, 60_000);
    let now = 60_000;
    flock.ask(3, notice(0), now).unwrap();
    flock.ask(4, notice(0), now).unwrap();
    let changes = flock.update(now + LEAD);
    assert!(!changes.iter().any(
        |(chicken, behaviour)| *chicken == 0 && matches!(behaviour.doing, Doing::Sprint { .. })
    ));
    // Once a minute has passed since, it can be startled again.
    let later = 10_000 + STARTLE + 100 + UNSTARTLED;
    let changes = flock.update(later);
    assert!(
        changes
            .iter()
            .any(|(chicken, behaviour)| *chicken == 0
                && matches!(behaviour.doing, Doing::Sprint { .. })),
        "{changes:?}"
    );
}

/// The flock, moved on until chicken `index` is on the ground and doing nothing in particular, at `now`.
fn run_until_standing(mut flock: Flock, index: usize, now: u64) -> Flock {
    let mut time = now - 5_000;
    while time < now {
        flock.update(time);
        time += 50;
    }
    let _ = index;
    flock
}

#[test]
fn a_chicken_in_the_coop_or_on_its_ramp_cannot_be_pointed_at() {
    let mut flock = flock("space-1");
    let mut refused = 0;
    run(&mut flock, (0, 30 * 60_000), |_, _| {});
    // Somewhere in all that, chickens roosted; now point at each, while one is up there.
    let mut flock = Flock::new(seed("space-1"), Arc::new(chicken()), 0);
    let mut now = 0;
    while now < 30 * 60_000 {
        flock.update(now);
        for index in 0..flock.hens().len() {
            let up = flock.position(index, now).y >= layout::ON_GROUND
                || flock.hens()[index].behaviour.doing == Doing::Roost;
            if up
                && flock
                    .ask(
                        9,
                        Intent::Notice {
                            notice: index,
                            toward: [0.0, 0.0],
                        },
                        now,
                    )
                    .is_none()
            {
                refused += 1;
            }
        }
        now += 250;
    }
    assert!(refused > 0);
}

#[test]
fn what_a_chicken_is_doing_is_described_with_its_name_and_where() {
    let mut flock = flock("space-1");
    let described = flock.describe(0);
    assert_eq!(described.len(), flock.hens().len());
    assert_eq!(described[0].0, "chicken-0");
    assert_eq!(
        described[0].1,
        format!("hen/{}", flock.hens()[0].name.to_lowercase())
    );
    let coned = flock.hens().iter().position(|hen| hen.cone).unwrap();
    assert!(described[coned].2.contains("wearing a traffic cone"));
    assert!(described[0].2.starts_with(&flock.hens()[0].name));
    flock
        .ask(
            1,
            Intent::Notice {
                notice: 0,
                toward: [0.0, 0.0],
            },
            1_000,
        )
        .unwrap();
    assert!(flock.describe(2_000)[0].2.contains("puzzled"));
    // Each kind of doing has words, and so has each place.
    let mut seen = std::collections::BTreeSet::new();
    run(&mut flock, (2_000, 30 * 60_000), |flock, now| {
        for (_, _, description) in flock.describe(now) {
            seen.insert(
                description
                    .split(" a hen in the farmyard, ")
                    .nth(1)
                    .unwrap()
                    .split(" by ")
                    .next()
                    .unwrap()
                    .split(" near ")
                    .next()
                    .unwrap()
                    .to_owned(),
            );
        }
    });
    assert!(seen.len() >= 5, "{seen:?}");
    assert_eq!(near(Vec3::new(0.0, 0.0, 1.0)), "near the gate");
    assert_eq!(near(Vec3::new(3.7, 0.0, -10.0)), "by the hay");
    assert!((bearing(Vec2::ZERO, Vec2::X) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
}

#[test]
fn the_flock_is_sent_as_state_events_and_intents_its_clients_read() {
    let flock = flock("space-1");
    let state = flock.state(42);
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(serde_json::from_str::<State>(&json).unwrap(), state);
    let event = Event {
        clock: 43,
        chicken: 1,
        behaviour: flock.hens()[1].behaviour,
    };
    assert_eq!(
        serde_json::from_value::<Event>(serde_json::to_value(&event).unwrap()).unwrap(),
        event
    );
    assert_eq!(
        serde_json::from_str::<Intent>(r#"{"notice": 2, "toward": [1.0, -2.0]}"#).unwrap(),
        Intent::Notice {
            notice: 2,
            toward: [1.0, -2.0]
        }
    );
    assert_eq!(
        serde_json::from_str::<Intent>(r#"{"away": 2}"#).unwrap(),
        Intent::Away { away: 2 }
    );
    assert!(serde_json::from_str::<Intent>(r#"{"peck": 2}"#).is_err());
}

#[test]
fn each_hen_has_her_own_reference_and_is_inspected_by_it() {
    for space in ["space-1", "space-2", "space-3", "space-4"] {
        let flock = flock(space);
        let references: std::collections::BTreeSet<_> = (0..flock.hens().len())
            .map(|index| flock.reference(index))
            .collect();
        assert_eq!(references.len(), flock.hens().len(), "{space}");
        for reference in &references {
            assert!(
                tomori_environment::valid_reference(reference),
                "{reference}"
            );
        }
    }
    let mut flock = flock("space-1");
    let coned = flock.hens().iter().position(|hen| hen.cone).unwrap();
    let reference = flock.reference(coned);
    let details = flock.inspect(&reference, 0).unwrap();
    let hen = &flock.hens()[coned];
    assert_eq!(details["kind"], "hen");
    assert_eq!(details["name"], hen.name.as_str());
    assert_eq!(details["wearing"], "a traffic cone");
    assert_eq!(details["doing"], "standing");
    assert_eq!(details["position"].as_array().unwrap().len(), 3);
    assert_eq!(details["pointed_at_by"], 0);
    assert_eq!(details["startled_seconds_ago"], serde_json::Value::Null);
    assert!(details["roosts_on"].as_str().unwrap().starts_with("the "));
    assert!(!details["near"].as_str().unwrap().starts_with("by "));
    // Pointed at, she says so.
    flock
        .ask(
            1,
            Intent::Notice {
                notice: coned,
                toward: [0.0, 0.0],
            },
            1_000,
        )
        .unwrap();
    let details = flock.inspect(&reference, 2_000).unwrap();
    assert_eq!(details["pointed_at_by"], 1);
    assert!(details["doing"].as_str().unwrap().contains("puzzled"));
    // One without a cone wears nothing, and a name the flock does not have is nobody.
    let plain = (coned + 1) % flock.hens().len();
    assert_eq!(
        flock.inspect(&flock.reference(plain), 0).unwrap()["wearing"],
        serde_json::Value::Null
    );
    assert_eq!(flock.inspect("hen/nobody", 0), None);
    assert_eq!(flock.inspect("chicken-0", 0), None);
}

#[test]
fn the_agent_has_a_hen_on_the_ground_peck_a_bounded_number_of_times() {
    let mut flock = flock("space-1");
    let pip = flock.reference(0);
    let name = flock.hens()[0].name.clone();
    // Standing about at the start, she pecks, beginning a moment from now, for as many pecks as asked.
    let ((index, behaviour), summary) = flock.peck(&pip, 3, 1_000).unwrap();
    assert_eq!((index, behaviour.since), (0, 1_000 + LEAD));
    assert_eq!(
        behaviour.doing,
        Doing::Peck {
            until: 1_000 + LEAD + 3 * crate::behaviour::PECK_MILLIS
        }
    );
    assert_eq!(flock.hens()[0].behaviour, behaviour);
    assert_eq!(summary, format!("{name} pecks at the ground 3 times."));
    // Held within 1 and the most.
    let ((_, many), _) = flock.peck(&pip, 99, 2_000).unwrap();
    assert_eq!(
        many.doing,
        Doing::Peck {
            until: 2_000 + LEAD + PECKS * crate::behaviour::PECK_MILLIS
        }
    );
    let ((_, none), summary) = flock.peck(&pip, 0, 3_000).unwrap();
    assert_eq!(
        none.doing,
        Doing::Peck {
            until: 3_000 + LEAD + crate::behaviour::PECK_MILLIS
        }
    );
    assert!(summary.ends_with("1 times."));
    assert_eq!(
        flock.peck("hen/nobody", 1, 0),
        Err("There is no such hen in the yard now.".to_owned())
    );
    // Standing on the path's tallest stone, she is on the ground, and pecks.
    let stone = layout::stones()
        .into_iter()
        .max_by(|a, b| a.height.total_cmp(&b.height))
        .unwrap();
    assert!(stone.height > 0.01);
    let middle = stone.middle();
    flock.hens[0].behaviour = Behaviour {
        since: 0,
        at: [middle.x, stone.height, middle.y],
        heading: 0.0,
        doing: Doing::Stand,
    };
    assert_eq!(flock.position(0, 4_000).y, stone.height);
    assert!(flock.peck(&pip, 1, 4_000).is_ok());
    // Up in the coop, or running off, she cannot.
    let mut flock = Flock::new(seed("space-1"), Arc::new(chicken()), 0);
    let mut now = 0;
    let mut refused = 0;
    while now < 30 * 60_000 && refused == 0 {
        flock.update(now);
        for index in 0..flock.hens().len() {
            let up = flock.position(index, now).y >= layout::ON_GROUND
                || flock.hens()[index].behaviour.doing == Doing::Roost;
            if up {
                let reference = flock.reference(index);
                let why = flock.peck(&reference, 1, now).unwrap_err();
                assert!(why.contains("so she cannot peck now"), "{why}");
                refused += 1;
            }
        }
        now += 250;
    }
    assert!(refused > 0);
    let mut flock = flock_running();
    let reference = flock.reference(0);
    let why = flock.peck(&reference, 1, 10_000).unwrap_err();
    assert!(why.contains("running off, startled"), "{why}");
}

/// A flock whose first chicken was startled at 6 s, pointed at by two at once, and is running off.
fn flock_running() -> Flock {
    let mut flock = Flock::new(seed("space-1"), Arc::new(chicken()), 0);
    for by in [1, 2] {
        flock.ask(
            by,
            Intent::Notice {
                notice: 0,
                toward: [0.0, 0.0],
            },
            5_000,
        );
    }
    let mut now = 5_000;
    while !matches!(flock.hens()[0].behaviour.doing, Doing::Sprint { .. }) {
        flock.update(now);
        now += 50;
        assert!(now < 60_000, "never startled");
    }
    flock
}

#[test]
fn the_hens_on_the_ground_glance_up_as_someone_comes_to_the_coop() {
    let mut flock = flock("space-1");
    // At the start, every hen stands about on the ground: each glances up, to alternate sides, for a moment.
    let changes = flock.glance(1_000);
    assert_eq!(changes.len(), flock.hens().len());
    for (index, behaviour) in &changes {
        assert_eq!(behaviour.since, 1_000 + LEAD);
        let side = if index % 2 == 0 {
            crate::behaviour::LOOK
        } else {
            -crate::behaviour::LOOK
        };
        assert_eq!(
            behaviour.doing,
            Doing::Look {
                side,
                until: 1_000 + LEAD + GLANCE
            }
        );
    }
    // One up in the coop, and one running off, do not.
    flock.hens[1].behaviour = Behaviour {
        since: 0,
        at: [-2.8, layout::COOP_FLOOR, -8.4],
        heading: 0.0,
        doing: Doing::Stand,
    };
    let mut running = flock_running();
    running.hens[1].behaviour = flock.hens[1].behaviour;
    let glanced: Vec<usize> = running
        .glance(10_000)
        .into_iter()
        .map(|(index, _)| index)
        .collect();
    assert!(
        !glanced.contains(&0) && !glanced.contains(&1),
        "{glanced:?}"
    );
}
