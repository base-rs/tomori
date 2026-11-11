// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use glam::{Vec2, Vec3};

use super::*;
use crate::chicken::tests::chicken;

fn at(doing: Doing) -> Behaviour {
    Behaviour {
        since: 1_000,
        at: [0.0, 0.0, -4.0],
        heading: 0.0,
        doing,
    }
}

#[test]
fn headings_and_turns_are_the_short_way_round() {
    assert!((heading(Vec2::Y) - 0.0).abs() < 1e-6);
    assert!((heading(Vec2::X) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert!(facing(std::f32::consts::FRAC_PI_2).distance(Vec2::X) < 1e-6);
    assert!((turn_between(3.0, -3.0) - (std::f32::consts::TAU - 6.0)).abs() < 1e-5);
    assert!((turned(0.0, 1.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
    assert!((turned(0.0, 1.0, 1.0, 9.0) - 1.0).abs() < 1e-6);
}

#[test]
fn a_walk_turns_then_goes_as_far_as_its_feet_carry_it_and_stops_with_them_down() {
    let chicken = chicken();
    let walk = at(Doing::Walk {
        to: [1.0, 0.0, -4.0],
    });
    let planned = walk.walk(&chicken, 1.0).unwrap();
    // A quarter turn first, at its rate.
    assert_eq!(
        planned.walks,
        1_000 + (std::f32::consts::FRAC_PI_2 / TURN * 1000.0).round() as u64
    );
    // It goes at least as far as asked, by whole steps, and no further than a step beyond.
    assert!(planned.distance >= 0.999 && planned.distance < 1.0 + chicken.stride());
    assert_eq!(walk.ends(&chicken, 1.0), Some(planned.stops));
    // Turning, it stays put; walking, it moves along; stopped, it is where its steps took it.
    let turning = walk.place(&chicken, 1.0, 1_200);
    assert_eq!(turning.position, Vec3::new(0.0, 0.0, -4.0));
    assert!(turning.heading > 0.0 && turning.heading < std::f32::consts::FRAC_PI_2);
    let midway = walk.place(&chicken, 1.0, (planned.walks + planned.stops) / 2);
    assert!(midway.position.x > 0.2 && midway.position.x < 0.8);
    let stopped = walk.place(&chicken, 1.0, planned.stops + 5_000);
    assert!((stopped.position.x - planned.distance).abs() < 1e-4);
    // A faster chicken stops sooner, as far.
    let quick = walk.walk(&chicken, 1.2).unwrap();
    assert!(quick.stops < planned.stops && quick.distance == planned.distance);
    // Walking nowhere, it stands where it is, facing as it did.
    let nowhere = at(Doing::Walk {
        to: [0.0, 0.0, -4.0],
    })
    .walk(&chicken, 1.0)
    .unwrap();
    assert_eq!(
        (nowhere.walks, nowhere.stops, nowhere.distance),
        (1_000, 1_000, 0.0)
    );
    // Up the coop's ramp, it rises as it goes, to the coop's floor.
    let up = Behaviour {
        at: layout::RAMP_FOOT.to_array(),
        ..at(Doing::Walk {
            to: (layout::DOOR - Vec3::Z * 0.15).to_array(),
        })
    };
    let climbing = up.place(&chicken, 1.0, up.ends(&chicken, 1.0).unwrap());
    assert!((climbing.position.y - layout::COOP_FLOOR).abs() < 1e-5);
    // Something else is no walk.
    assert_eq!(at(Doing::Stand).walk(&chicken, 1.0), None);
}

/// The soles of its feet as it stands, the lowest of its points, standing on the origin and facing +Z.
fn soles(chicken: &Chicken) -> Vec<Vec3> {
    let (start, _) = crate::chicken::seconds(crate::chicken::WALK);
    let standing = crate::chicken::tests::walking(chicken, start);
    let soles: Vec<Vec3> = standing
        .into_iter()
        .filter(|point| point.y < 0.001)
        .collect();
    assert!(soles.len() > 4, "{}", soles.len());
    soles
}

/// Where `point` of a chicken placed as `placed` is, as its controller draws it, but for its animation.
fn drawn(placed: &Placed, point: Vec3) -> Vec3 {
    glam::Mat4::from_rotation_translation(
        glam::Quat::from_rotation_y(placed.heading),
        placed.position,
    )
    .transform_point3(glam::Quat::from_rotation_x(placed.lean) * point)
}

#[test]
fn up_and_down_the_ramp_its_feet_stay_on_its_board_and_it_leans_with_it() {
    let chicken = chicken();
    let soles = soles(&chicken);
    let door = (layout::DOOR - Vec3::Z * 0.15).to_array();
    // Up from in front of its foot, to in at the door, and down again, from just off its line, as a chicken overshoots.
    for (from, to) in [
        ([layout::RAMP_FOOT.x, 0.0, layout::RAMP_FOOT.z + 0.4], door),
        (
            [layout::RAMP_FOOT.x + 0.03, 0.0, layout::RAMP_FOOT.z + 0.03],
            door,
        ),
        (door, [layout::RAMP_FOOT.x, 0.0, layout::RAMP_FOOT.z + 0.4]),
    ] {
        let walk = Behaviour {
            since: 0,
            at: from,
            heading: heading(Vec2::new(to[0] - from[0], to[2] - from[2])),
            doing: Doing::Walk { to },
        };
        let planned = walk.walk(&chicken, 1.0).unwrap();
        let (mut sunk, mut along, mut raised, mut steepest) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for now in (planned.walks..=planned.stops).step_by(20) {
            let placed = walk.place(&chicken, 1.0, now);
            steepest = steepest.max(placed.lean.abs());
            // Each sole is on whatever is beneath it: the board, the coop's floor or the ground.
            for sole in &soles {
                let at = drawn(&placed, *sole);
                let beneath = layout::underfoot(Vec2::new(at.x, at.z));
                sunk = sunk.max(beneath - at.y);
                // Along the board, a foot's length from its turns, on it; over them, resting on both sides.
                let turns = [layout::RAMP_FOOT.z, layout::DOOR.z];
                if turns
                    .iter()
                    .all(|turn| (placed.position.z - turn).abs() > 0.1)
                {
                    along = along.max((at.y - beneath).abs());
                } else {
                    raised = raised.max(at.y - beneath);
                }
            }
            // Leaning back up the ramp and forward down it, as steep as it is, and only on it.
            let climbing = to[1] > from[1];
            assert!(placed.lean <= 1e-6 || !climbing, "{now} {placed:?}");
            assert!(placed.lean >= -1e-6 || climbing, "{now} {placed:?}");
        }
        // Never into it by more than a couple of millimetres; along it, within them; and over its foot and its sill,
        // where a foot that does not bend rests on both sides of the turn, less than a centimetre above it.
        assert!(sunk < 0.002 && along < 0.002, "{from:?} {sunk} {along}");
        assert!(raised < 0.01, "{from:?} {raised}");
        let incline = (layout::COOP_FLOOR / (layout::RAMP_FOOT.z - layout::DOOR.z)).atan();
        assert!((steepest - incline).abs() < 1e-3, "{steepest} {incline}");
    }
    // Turned across the ramp, it stands level on it, as the board is level across.
    let middle = (layout::RAMP_FOOT + layout::DOOR) * 0.5;
    let (across, lean) = afoot(middle, std::f32::consts::FRAC_PI_2);
    assert!((across.y - layout::COOP_FLOOR * 0.5).abs() < 1e-5 && lean.abs() < 1e-6);
    // Off the ramp, it stands on the ground, upright.
    let (ground, lean) = afoot(Vec3::new(0.0, 0.3, -4.0), 0.0);
    assert_eq!((ground, lean), (Vec3::new(0.0, 0.0, -4.0), 0.0));
}

#[test]
fn standing_pecking_looking_and_being_confused_keep_it_where_it_is() {
    let chicken = chicken();
    for doing in [Doing::Stand, Doing::Roost, Doing::Recover] {
        let placed = at(doing).place(&chicken, 1.0, 5_000);
        assert_eq!(
            (placed.position, placed.heading, placed.lean),
            (Vec3::new(0.0, 0.0, -4.0), 0.0, 0.0)
        );
    }
    assert_eq!(at(Doing::Stand).ends(&chicken, 1.0), None);
    assert_eq!(
        at(Doing::Recover).ends(&chicken, 1.0),
        Some(1_000 + RECOVERING)
    );
    // Pecking, it leans down and up, a peck at a time, and stands straight once it has finished.
    let peck = at(Doing::Peck { until: 3_100 });
    assert_eq!(peck.ends(&chicken, 1.0), Some(3_100));
    assert!(peck.place(&chicken, 1.0, 1_000 + PECK_MILLIS / 4).lean > PECK * 0.9);
    assert_eq!(
        peck.place(&chicken, 1.0, 1_000 + PECK_MILLIS * 3 / 4).lean,
        0.0
    );
    // Looking, it turns to the side and back.
    let look = at(Doing::Look {
        side: LOOK,
        until: 3_000,
    });
    assert!((look.place(&chicken, 1.0, 2_000).heading - LOOK).abs() < 1e-4);
    assert!(look.place(&chicken, 1.0, 3_000).heading.abs() < 1e-4);
    // Confused, it turns to whoever points.
    let confused = at(Doing::Confused {
        toward: [5.0, -4.0],
    });
    assert!(
        (confused.place(&chicken, 1.0, 9_000).heading - std::f32::consts::FRAC_PI_2).abs() < 1e-4
    );
    assert_eq!(confused.ends(&chicken, 1.0), None);
}

#[test]
fn a_sprint_spins_round_and_runs_as_far_as_its_feet_carry_it() {
    let chicken = chicken();
    let sprint = at(Doing::Sprint { away: [0.0, 1.0] });
    let ends = sprint.ends(&chicken, 1.0).unwrap();
    assert_eq!(ends, 1_000 + 2_958);
    let done = sprint.place(&chicken, 1.0, ends + 1_000);
    assert!((done.position.z - (-4.0 + chicken.sprint.total())).abs() < 1e-4);
    assert!(chicken.sprint.total() > 1.0);
    // Facing the other way, it spins round first.
    let round = Behaviour {
        heading: std::f32::consts::PI,
        ..sprint
    };
    assert!(round.place(&chicken, 1.0, 1_100).heading.abs() > 1.0);
    assert!(
        round.place(&chicken, 1.0, 2_000).heading.abs() < 1e-4
            || (round.place(&chicken, 1.0, 2_000).heading.abs() - std::f32::consts::TAU).abs()
                < 1e-4
    );
}

#[test]
fn a_behaviour_is_sent_as_what_it_is_doing_beside_where() {
    let behaviour = at(Doing::Walk {
        to: [1.0, 0.0, -4.0],
    });
    let json = serde_json::to_value(behaviour).unwrap();
    assert_eq!(json["doing"], "walk");
    assert_eq!(json["to"], serde_json::json!([1.0, 0.0, -4.0]));
    assert_eq!(
        serde_json::from_value::<Behaviour>(json).unwrap(),
        behaviour
    );
}
