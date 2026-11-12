// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Travel's tests: the surfaces, pads and targets an environment declares, teleporting, flying and the rig.

use super::*;

const HEAD: Vec3 = Vec3::new(0.3, 1.7, -0.2);
const PLACE: Place = Some(("space-1", Environment::new("floor")));

fn floors() -> Locomotion {
    Locomotion::teleport(vec![
        Surface::rectangle(Vec3::ZERO, Vec2::new(20.0, 20.0)),
        // A balcony, 3 m up, to the right.
        Surface::rectangle(Vec3::new(4.0, 3.0, 0.0), Vec2::new(2.0, 4.0)),
        Surface::disc(Vec3::new(-6.0, 1.0, 0.0), 1.0),
    ])
}

fn ray(origin: Vec3, towards: Vec3) -> Ray {
    Ray::new(origin, towards - origin).unwrap()
}

fn close(a: Vec3, b: Vec3) -> bool {
    a.distance(b) < 1e-4
}

#[test]
fn surfaces_hold_what_is_above_them() {
    let rectangle = Surface::rectangle(Vec3::new(1.0, 2.0, 3.0), Vec2::new(-2.0, 4.0));
    assert_eq!(rectangle.height(), 2.0);
    assert!(rectangle.contains(Vec3::new(1.9, 0.0, 4.9)));
    assert!(!rectangle.contains(Vec3::new(2.1, 2.0, 3.0)));
    assert!(!rectangle.contains(Vec3::new(1.0, 2.0, 5.1)));
    let disc = Surface::disc(Vec3::new(0.0, 1.0, 0.0), -2.0);
    assert_eq!(disc.height(), 1.0);
    assert!(disc.contains(Vec3::new(1.4, 9.0, 1.4)));
    assert!(!disc.contains(Vec3::new(1.5, 1.0, 1.5)));
}

#[test]
fn a_ray_lands_on_the_first_surface_it_meets_from_above() {
    let floors = floors();
    let eye = Vec3::new(0.0, 1.5, 0.0);
    // On the floor ahead.
    let landing = floors
        .landing(&ray(eye, Vec3::new(0.0, 0.0, -3.0)))
        .unwrap();
    assert!(close(landing, Vec3::new(0.0, 0.0, -3.0)));
    // On the disc, which stands above the floor beneath it.
    let landing = floors
        .landing(&ray(eye, Vec3::new(-6.0, 1.0, 0.5)))
        .unwrap();
    assert!(close(landing, Vec3::new(-6.0, 1.0, 0.5)));
    // The balcony, above the eye, cannot be reached from below; the floor beyond it can.
    let landing = floors.landing(&ray(eye, Vec3::new(5.0, 0.0, 0.0))).unwrap();
    assert_eq!(landing.y, 0.0);
    // From the balcony, it hides the floor beneath it, and the floor beyond it is reached.
    let up = Vec3::new(4.0, 4.6, 0.0);
    let landing = floors.landing(&ray(up, Vec3::new(4.5, 0.0, 0.5))).unwrap();
    assert!(close(
        landing.with_y(0.0),
        Vec3::new(4.0, 0.0, 0.0).lerp(Vec3::new(4.5, 0.0, 0.5), 1.6 / 4.6)
    ));
    assert_eq!(landing.y, 3.0);
    let landing = floors.landing(&ray(up, Vec3::new(8.0, 0.0, 0.0))).unwrap();
    assert!(close(landing, Vec3::new(8.0, 0.0, 0.0)));
    // Just above the balcony, it is too near to land on, and still hides the floor beneath it.
    let low = Vec3::new(4.0, 3.1, 0.0);
    assert_eq!(floors.landing(&ray(low, Vec3::new(4.1, 3.0, 0.0))), None);
}

#[test]
fn some_rays_land_nowhere() {
    let floors = floors();
    let eye = Vec3::new(0.0, 1.5, 0.0);
    // Level, upwards, off every surface, too near and too far.
    assert_eq!(floors.landing(&ray(eye, Vec3::new(0.0, 1.5, -1.0))), None);
    assert_eq!(floors.landing(&ray(eye, Vec3::new(0.0, 3.0, -1.0))), None);
    assert_eq!(floors.landing(&ray(eye, Vec3::new(0.0, 0.0, -11.0))), None);
    assert_eq!(
        floors.landing(&ray(Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.0, 0.0, -0.01))),
        None
    );
    let wide = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 100.0)]);
    assert!(
        wide.landing(&ray(eye, Vec3::new(0.0, 0.0, -14.0)))
            .is_some()
    );
    assert_eq!(wide.landing(&ray(eye, Vec3::new(0.0, 0.0, -16.0))), None);
    // From high up, straight down is within reach.
    assert!(
        wide.landing(&ray(Vec3::new(0.0, 30.0, 0.0), Vec3::new(1.0, 0.0, 0.0)))
            .is_some()
    );
    assert_eq!(Locomotion::None.landing(&ray(eye, Vec3::ZERO)), None);
}

#[test]
fn profiles_say_whether_they_travel_and_fly() {
    assert!(!Locomotion::default().travels());
    assert_eq!(Locomotion::default().ceiling(), None);
    assert!(floors().travels());
    assert_eq!(floors().ceiling(), None);
    let fly = Locomotion::fly(vec![Surface::disc(Vec3::ZERO, 5.0)], 10.0);
    assert!(fly.travels());
    assert_eq!(fly.ceiling(), Some(10.0));
    // Flying lands on its surfaces as teleporting does.
    let down = ray(Vec3::new(0.0, 8.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
    assert!(close(fly.landing(&down).unwrap(), Vec3::new(1.0, 0.0, 0.0)));
}

#[test]
fn teleports_keep_the_head_s_height_and_turns_keep_the_head_in_place() {
    let mut rig = Rig::default();
    let landing = Vec3::new(4.0, 3.0, 1.0);
    rig.teleport(landing, rig.matrix().transform_point3(HEAD));
    let head = rig.matrix().transform_point3(HEAD);
    assert!(close(head, landing + Vec3::Y * HEAD.y));
    assert_eq!(rig.floor(), 3.0);
    rig.turn(SNAP_TURN, head);
    assert!(close(rig.matrix().transform_point3(HEAD), head));
    assert_eq!(rig.floor(), 3.0);
    // The room's forward, -Z, turned 30° to the left.
    let forward = rig.matrix().transform_vector3(-Vec3::Z);
    assert!(close(
        forward,
        Vec3::new(-0.5, 0.0, -(3.0_f32.sqrt()) / 2.0)
    ));
    // Back down on the floor, at the same height above it.
    rig.teleport(
        Vec3::new(-2.0, 0.0, 0.0),
        rig.matrix().transform_point3(HEAD),
    );
    assert!((rig.matrix().transform_point3(HEAD).y - HEAD.y).abs() < 1e-5);
    // Turning all the way round stays within a turn.
    for _ in 0..13 {
        rig.turn(SNAP_TURN, Vec3::ZERO);
    }
    assert!(rig.yaw.abs() <= std::f32::consts::PI + 1e-5);
}

#[test]
fn rising_stops_at_the_ceiling() {
    let mut rig = Rig::default();
    rig.ascend(ASCENT_SPEED * 2.0, 10.0);
    assert!((rig.floor() - 1.2).abs() < 1e-5);
    rig.ascend(100.0, 10.0);
    assert_eq!(rig.floor(), 10.0);
    // A ceiling below where the floor already is keeps it there.
    rig.teleport(Vec3::new(0.0, 12.0, 0.0), Vec3::ZERO);
    rig.ascend(1.0, 10.0);
    assert_eq!(rig.floor(), 12.0);
}

fn input(locomotion: &Locomotion, stick: Vec2, ray: Option<Ray>) -> Input<'_> {
    Input {
        place: PLACE,
        locomotion,
        stick,
        ray,
        head: Some(HEAD),
        facing: Some(Vec3::NEG_Z),
        active: true,
        chosen: None,
        dt: 0.5,
    }
}

#[test]
fn forward_aims_and_releasing_it_lands_once() {
    let floors = floors();
    let mut travel = Travel::default();
    let aim = Some(ray(Vec3::new(0.3, 1.2, -0.4), Vec3::new(0.0, 0.0, -4.0)));
    let outcome = travel.update(input(&floors, Vec2::new(0.1, 0.9), aim));
    assert!(outcome.aiming && !outcome.teleported);
    assert!(close(outcome.landing.unwrap(), Vec3::new(0.0, 0.0, -4.0)));
    assert_eq!(travel.rig(), Rig::default(), "aiming does not move");
    let outcome = travel.update(input(&floors, Vec2::ZERO, aim));
    assert!(outcome.teleported && !outcome.aiming);
    assert_eq!(outcome.landing, None);
    let head = travel.rig().matrix().transform_point3(HEAD);
    assert!(close(head, Vec3::new(0.0, HEAD.y, -4.0)));
    // Released, it does nothing more.
    let outcome = travel.update(input(&floors, Vec2::ZERO, aim));
    assert_eq!(outcome, Outcome::default());
}

#[test]
fn releasing_with_nowhere_to_land_or_after_losing_the_ray_does_nothing() {
    let floors = floors();
    let mut travel = Travel::default();
    let up = Some(ray(HEAD, HEAD + Vec3::Y));
    let outcome = travel.update(input(&floors, Vec2::Y, up));
    assert!(outcome.aiming);
    assert_eq!(outcome.landing, None);
    assert!(!travel.update(input(&floors, Vec2::ZERO, up)).teleported);
    // Aimed, then released as the ray is lost.
    let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
    assert!(
        travel
            .update(input(&floors, Vec2::Y, aim))
            .landing
            .is_some()
    );
    assert!(!travel.update(input(&floors, Vec2::ZERO, None)).teleported);
    // Aimed, then the ray is lost while the stick is held.
    let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
    assert!(
        travel
            .update(input(&floors, Vec2::Y, aim))
            .landing
            .is_some()
    );
    assert!(!travel.update(input(&floors, Vec2::Y, None)).aiming);
    assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
    // Aimed, then the hand menu shows.
    travel.update(input(&floors, Vec2::Y, aim));
    let inactive = Input {
        active: false,
        ..input(&floors, Vec2::Y, aim)
    };
    assert_eq!(travel.update(inactive), Outcome::default());
    assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
    // Aimed, then focus is lost.
    travel.update(input(&floors, Vec2::Y, aim));
    travel.cancel();
    assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
    // Without a head, a release lands nowhere.
    travel.update(input(&floors, Vec2::Y, aim));
    let headless = Input {
        head: None,
        ..input(&floors, Vec2::ZERO, aim)
    };
    assert!(!travel.update(headless).teleported);
    assert_eq!(travel.rig(), Rig::default());
}

#[test]
fn left_and_right_turn_once_per_push() {
    let floors = floors();
    let mut travel = Travel::default();
    let outcome = travel.update(input(&floors, Vec2::new(0.9, 0.2), None));
    assert!(outcome.turned);
    let right = travel.rig();
    // Turning right turns the room's forward to the right.
    assert!(right.matrix().transform_vector3(-Vec3::Z).x > 0.4);
    assert!(close(right.matrix().transform_point3(HEAD), HEAD));
    // Held, or eased off but not back near the middle, it does not turn again.
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.9, 0.0), None))
            .turned
    );
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.4, 0.0), None))
            .turned
    );
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.9, 0.0), None))
            .turned
    );
    travel.update(input(&floors, Vec2::ZERO, None));
    assert!(
        travel
            .update(input(&floors, Vec2::new(-0.9, 0.0), None))
            .turned
    );
    assert!((travel.rig().yaw).abs() < 1e-5, "back where it was");
    // A push that is more forward than sideways aims instead.
    travel.update(input(&floors, Vec2::ZERO, None));
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.7, 0.8), None))
            .turned
    );
    // Without a head, a push turns nothing, and still counts as the push.
    travel.update(input(&floors, Vec2::ZERO, None));
    let headless = Input {
        head: None,
        ..input(&floors, Vec2::X, None)
    };
    assert!(!travel.update(headless).turned);
    assert!(!travel.update(input(&floors, Vec2::X, None)).turned);
    // After focus returns, the stick must come back to the middle first.
    travel.update(input(&floors, Vec2::ZERO, None));
    travel.cancel();
    assert!(!travel.update(input(&floors, Vec2::X, None)).turned);
    // A stick that reads nonsense is in the middle.
    travel.update(input(&floors, Vec2::NAN, None));
    assert!(travel.update(input(&floors, Vec2::X, None)).turned);
}

#[test]
fn back_rises_only_where_the_environment_flies_and_once_held() {
    let floors = floors();
    let mut travel = Travel::default();
    travel.update(input(&floors, Vec2::NEG_Y, None));
    travel.update(input(&floors, Vec2::NEG_Y, None));
    assert_eq!(travel.rig().floor(), 0.0);
    travel.update(input(&floors, Vec2::ZERO, None));
    let fly = Locomotion::fly(Vec::new(), 1.0);
    let mut travel = Travel::default();
    // Held past the hold, it rises from then on: 0.2 s of the first half second.
    travel.update(input(&fly, Vec2::NEG_Y, None));
    assert!((travel.rig().floor() - 0.12).abs() < 1e-5);
    travel.update(input(&fly, Vec2::NEG_Y, None));
    assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
    // A frame without a sensible length, or a push more sideways, rises nothing.
    let nonsense = Input {
        dt: f32::NAN,
        ..input(&fly, Vec2::NEG_Y, None)
    };
    travel.update(nonsense);
    assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
    travel.update(input(&fly, Vec2::ZERO, None));
    assert!(
        travel
            .update(input(&fly, Vec2::new(0.9, -0.7), None))
            .turned
    );
    assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
    travel.update(input(&fly, Vec2::ZERO, None));
    for _ in 0..10 {
        travel.update(input(&fly, Vec2::NEG_Y, None));
    }
    assert_eq!(travel.rig().floor(), 1.0);
    // Released after rising, it steps nowhere.
    let before = travel.rig();
    assert!(!travel.update(input(&fly, Vec2::ZERO, None)).stepped);
    assert_eq!(travel.rig(), before);
}

#[test]
fn heads_face_the_way_they_look_levelled() {
    assert!(close(facing(Quat::IDENTITY).unwrap(), Vec3::NEG_Z));
    let left = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    assert!(close(facing(left).unwrap(), Vec3::NEG_X));
    // Looking down, or up, a little, it is still forward.
    let down = Quat::from_rotation_x(-1.2);
    assert!(close(facing(down).unwrap(), Vec3::NEG_Z));
    // Straight down or up, the top of the head says which way is forward.
    let floor = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    assert!(close(facing(floor).unwrap(), Vec3::NEG_Z));
    let sky = left * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    assert!(close(facing(sky).unwrap(), Vec3::NEG_X));
    // Nonsense faces nowhere.
    assert_eq!(facing(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)), None);
    assert_eq!(facing(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)), None);
}

#[test]
fn a_backstep_goes_straight_back_as_far_as_the_floor_goes() {
    let head = Vec3::new(0.0, 1.7, 0.0);
    // A small floor, its back edge 0.42 m behind the head: it steps as far as the floor goes, in 5 cm strides.
    let small = Locomotion::teleport(vec![Surface::rectangle(
        Vec3::new(0.0, 0.0, -0.79),
        Vec2::new(2.0, 2.42),
    )]);
    let step = small.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
    assert!(close(step, Vec3::new(0.0, 0.0, 0.4)), "{step}");
    // Facing anywhere, it steps away from it, horizontally, however the face tilts.
    let wide = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 5.0)]);
    let step = wide.backstep(head, Vec3::new(1.0, -3.0, 0.0), 0.0).unwrap();
    assert!(close(step, Vec3::new(-BACKSTEP, 0.0, 0.0)), "{step}");
    // Only along the surface stood on: a ledge above the floor does not stop a step on the floor, as nothing
    // collides yet, and from the ledge, the floor below is not stepped down onto.
    let ledged = Locomotion::teleport(vec![
        Surface::disc(Vec3::ZERO, 5.0),
        Surface::rectangle(Vec3::new(0.0, 0.5, 0.6), Vec2::new(2.0, 0.4)),
    ]);
    let step = ledged.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
    assert!(close(step, Vec3::new(0.0, 0.0, BACKSTEP)), "{step}");
    let on_ledge = Vec3::new(0.0, 2.2, 0.45);
    assert_eq!(ledged.backstep(on_ledge, Vec3::Z, 0.5), None);
    // Too close to the edge, it steps nowhere: 0.15 m is less than is worth a step.
    let near = Locomotion::teleport(vec![Surface::rectangle(
        Vec3::new(0.0, 0.0, -0.84),
        Vec2::new(2.0, 2.0),
    )]);
    assert_eq!(near.backstep(head, Vec3::NEG_Z, 0.0), None);
    // Not over a gap: the floor beyond one is not reached.
    let gap = Locomotion::teleport(vec![
        Surface::rectangle(Vec3::new(0.0, 0.0, -1.0), Vec2::new(2.0, 2.64)),
        Surface::rectangle(Vec3::new(0.0, 0.0, 1.5), Vec2::new(2.0, 2.0)),
    ]);
    let step = gap.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
    assert!(close(step, Vec3::new(0.0, 0.0, 0.3)), "{step}");
    // Flying, the whole way, floor or not; with no travel, or no way it faces, nowhere.
    let fly = Locomotion::fly(Vec::new(), 10.0);
    let step = fly.backstep(head, Vec3::NEG_Z, 3.0).unwrap();
    assert!(close(step, Vec3::new(0.0, 0.0, BACKSTEP)));
    assert_eq!(Locomotion::None.backstep(head, Vec3::NEG_Z, 0.0), None);
    assert_eq!(wide.backstep(head, Vec3::Y, 0.0), None);
}

#[test]
fn back_steps_back_once_as_the_stick_returns_to_the_middle() {
    let floors = floors();
    let mut travel = Travel::default();
    // Pushed back, nothing moves until the stick returns, however long it is held.
    for _ in 0..4 {
        let outcome = travel.update(input(&floors, Vec2::new(0.1, -0.9), None));
        assert!(!outcome.stepped && !outcome.turned && !outcome.aiming);
    }
    // Eased off, or rolled to the side or forward, it still steps back, and only on the middle.
    for rolled in [
        Vec2::new(0.0, -0.3),
        Vec2::new(0.9, 0.0),
        Vec2::new(0.0, 0.9),
    ] {
        let outcome = travel.update(input(&floors, rolled, None));
        assert!(
            !outcome.stepped && !outcome.turned && !outcome.aiming,
            "{rolled}"
        );
    }
    assert_eq!(travel.rig(), Rig::default());
    let outcome = travel.update(input(&floors, Vec2::ZERO, None));
    assert!(outcome.stepped && !outcome.teleported);
    assert_eq!(outcome.blink, 1.0, "it blinks as a teleport does");
    assert!(close(
        travel.rig().matrix().transform_point3(HEAD),
        HEAD + Vec3::Z * BACKSTEP
    ));
    assert_eq!(travel.rig().floor(), 0.0);
    assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
    // Short of the threshold, or sideways first, it steps nowhere.
    travel.update(input(&floors, Vec2::new(0.0, -0.65), None));
    assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
    travel.update(input(&floors, Vec2::new(0.9, -0.5), None));
    travel.update(input(&floors, Vec2::new(0.0, -0.9), None));
    assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
    // Facing left, after the turn, it steps to the right; without a head or a way it faces, nowhere.
    let before = travel.rig().matrix().transform_point3(HEAD);
    let left = Input {
        facing: Some(Vec3::NEG_X),
        ..input(&floors, Vec2::NEG_Y, None)
    };
    travel.update(left);
    travel.update(Input {
        stick: Vec2::ZERO,
        ..left
    });
    assert!(close(
        travel.rig().matrix().transform_point3(HEAD),
        before + Vec3::X * BACKSTEP
    ));
    for lacking in [
        Input {
            head: None,
            ..input(&floors, Vec2::ZERO, None)
        },
        Input {
            facing: None,
            ..input(&floors, Vec2::ZERO, None)
        },
    ] {
        travel.update(input(&floors, Vec2::NEG_Y, None));
        assert!(!travel.update(lacking).stepped);
    }
    // Cancelled, as when the hand menu shows, or where the floor ends right behind, it steps nowhere.
    travel.update(input(&floors, Vec2::NEG_Y, None));
    travel.cancel();
    assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
    let edge = Locomotion::teleport(vec![Surface::disc(Vec3::new(0.3, 0.0, -0.3), 0.15)]);
    let mut travel = Travel::default();
    travel.update(input(&edge, Vec2::NEG_Y, None));
    let outcome = travel.update(input(&edge, Vec2::ZERO, None));
    assert!(!outcome.stepped);
    assert_eq!(outcome.blink, 0.0, "nowhere to step, it does not blink");
    // With no travel, nowhere.
    let none = Locomotion::None;
    travel.update(input(&none, Vec2::NEG_Y, None));
    assert_eq!(
        travel.update(input(&none, Vec2::ZERO, None)),
        Outcome::default()
    );
}

#[test]
fn where_the_environment_flies_a_tap_back_steps_back_and_a_hold_rises() {
    let fly = Locomotion::fly(Vec::new(), 10.0);
    let mut travel = Travel::default();
    let tap = |stick: Vec2| Input {
        dt: HOLD / 3.0,
        ..input(&fly, stick, None)
    };
    travel.update(tap(Vec2::NEG_Y));
    travel.update(tap(Vec2::NEG_Y));
    let outcome = travel.update(tap(Vec2::ZERO));
    assert!(outcome.stepped);
    assert_eq!(travel.rig().floor(), 0.0);
    assert!(close(
        travel.rig().matrix().transform_point3(HEAD),
        HEAD + Vec3::Z * BACKSTEP
    ));
    // Held past the hold, it rises rather than stepping back.
    for _ in 0..4 {
        assert!(!travel.update(tap(Vec2::NEG_Y)).stepped);
    }
    assert!((travel.rig().floor() - ASCENT_SPEED * HOLD / 3.0).abs() < 1e-5);
    assert!(!travel.update(tap(Vec2::ZERO)).stepped);
    // Rolled sideways before the hold is up, it rises nothing once it is, and still steps nowhere.
    let floor = travel.rig().floor();
    travel.update(tap(Vec2::NEG_Y));
    for _ in 0..3 {
        assert!(!travel.update(tap(Vec2::new(0.5, -0.1))).turned);
    }
    assert_eq!(travel.rig().floor(), floor);
    assert!(!travel.update(tap(Vec2::ZERO)).stepped);
}

/// Aim a teleport at the floor 4 m ahead, as the reviewer's cases start.
fn aimed(floors: &Locomotion, travel: &mut Travel) -> Option<Ray> {
    let aim = Some(ray(Vec3::new(0.3, 1.2, -0.4), Vec3::new(0.0, 0.0, -4.0)));
    let outcome = travel.update(input(floors, Vec2::new(0.0, 0.9), aim));
    assert!(outcome.aiming && outcome.landing.is_some());
    aim
}

#[test]
fn easing_off_while_aiming_does_not_land_until_the_stick_is_back_at_the_middle() {
    let floors = floors();
    let mut travel = Travel::default();
    let aim = aimed(&floors, &mut travel);
    // Eased from 0.9 to 0.55, below where a push starts, and further, it still aims.
    for eased in [0.55, 0.3, 0.21] {
        let outcome = travel.update(input(&floors, Vec2::new(0.0, eased), aim));
        assert!(outcome.aiming && !outcome.teleported, "{eased}");
    }
    assert_eq!(travel.rig(), Rig::default());
    // Back at the middle, it lands, once.
    let outcome = travel.update(input(&floors, Vec2::new(0.1, 0.1), aim));
    assert!(outcome.teleported && !outcome.aiming);
    assert!(close(
        travel.rig().matrix().transform_point3(HEAD),
        Vec3::new(0.0, HEAD.y, -4.0)
    ));
    assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
}

#[test]
fn rolling_the_thumb_sideways_while_aiming_neither_lands_nor_turns() {
    let floors = floors();
    let mut travel = Travel::default();
    let aim = aimed(&floors, &mut travel);
    // The reviewer's roll, from forward to (0.8, 0.5), then all the way to the side and back.
    for rolled in [
        Vec2::new(0.8, 0.5),
        Vec2::new(0.95, 0.0),
        Vec2::new(-0.9, 0.1),
        Vec2::new(0.0, -0.9),
    ] {
        let outcome = travel.update(input(&floors, rolled, aim));
        assert!(outcome.aiming, "{rolled}");
        assert!(!outcome.teleported && !outcome.turned, "{rolled}");
    }
    assert_eq!(travel.rig(), Rig::default());
    let outcome = travel.update(input(&floors, Vec2::ZERO, aim));
    assert!(outcome.teleported && !outcome.turned);
    assert_eq!(travel.rig().yaw, 0.0);
}

#[test]
fn whichever_axis_passes_its_threshold_first_owns_the_push_until_the_middle() {
    let floors = floors();
    let mut travel = Travel::default();
    let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
    // Short of each threshold, nothing starts: forward needs 0.7, sideways 0.6.
    for short in [
        Vec2::new(0.0, 0.65),
        Vec2::new(0.55, 0.0),
        Vec2::new(-0.55, 0.3),
    ] {
        assert_eq!(
            travel.update(input(&floors, short, aim)),
            Outcome::default(),
            "{short}"
        );
    }
    // Sideways first turns, and rolling forward then aims nothing.
    assert!(
        travel
            .update(input(&floors, Vec2::new(0.9, 0.1), aim))
            .turned
    );
    for rolled in [Vec2::new(0.5, 0.8), Vec2::new(0.0, 0.95)] {
        let outcome = travel.update(input(&floors, rolled, aim));
        assert!(!outcome.aiming && !outcome.turned, "{rolled}");
    }
    // Part of the way back is not the middle: neither a turn nor an aim starts.
    travel.update(input(&floors, Vec2::new(0.25, 0.0), aim));
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.9, 0.0), aim))
            .turned
    );
    travel.update(input(&floors, Vec2::new(0.15, 0.15), aim));
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.0, 0.9), aim))
            .aiming
    );
    // From the middle, the next push counts, whichever way it goes.
    travel.update(input(&floors, Vec2::new(0.1, -0.1), aim));
    assert!(
        travel
            .update(input(&floors, Vec2::new(0.0, 0.9), aim))
            .aiming
    );
    // Forward without a ray owns the push too, and aims nothing.
    travel.cancel();
    travel.update(input(&floors, Vec2::ZERO, None));
    let outcome = travel.update(input(&floors, Vec2::Y, None));
    assert!(!outcome.aiming);
    assert!(
        !travel
            .update(input(&floors, Vec2::new(0.9, 0.0), aim))
            .turned
    );
    assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
    // A stick that reads nonsense while aiming lands nothing.
    travel.update(input(&floors, Vec2::Y, aim));
    assert!(!travel.update(input(&floors, Vec2::NAN, aim)).teleported);
    // All that moved the rig was the one turn.
    assert!((travel.rig().yaw + SNAP_TURN).abs() < 1e-5);
    assert!(close(travel.rig().matrix().transform_point3(HEAD), HEAD));
}

#[test]
fn rising_owns_its_push_and_stops_as_the_stick_rolls_away() {
    let fly = Locomotion::fly(vec![Surface::disc(Vec3::ZERO, 20.0)], 10.0);
    let mut travel = Travel::default();
    let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
    // Back, short of its threshold, rises nothing; past it, held, it rises.
    travel.update(input(&fly, Vec2::new(0.0, -0.65), aim));
    assert_eq!(travel.rig().floor(), 0.0);
    travel.update(input(&fly, Vec2::ZERO, aim));
    travel.update(input(&fly, Vec2::NEG_Y, aim));
    assert!((travel.rig().floor() - 0.12).abs() < 1e-5);
    // Eased off, still held back, it keeps rising.
    travel.update(input(&fly, Vec2::new(0.0, -0.4), aim));
    assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
    // Rolled to the side or forward, it neither turns, aims nor rises.
    for rolled in [Vec2::new(0.9, -0.1), Vec2::new(0.1, 0.9)] {
        let outcome = travel.update(input(&fly, rolled, aim));
        assert!(!outcome.turned && !outcome.aiming, "{rolled}");
    }
    assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
    let outcome = travel.update(input(&fly, Vec2::ZERO, aim));
    assert!(!outcome.teleported && !outcome.stepped);
}

#[test]
fn a_teleport_darkens_the_view_and_it_comes_back_over_a_blink() {
    let floors = floors();
    let mut travel = Travel::default();
    let aim = aimed(&floors, &mut travel);
    let short = |stick: Vec2, active: bool| Input {
        dt: BLINK / 4.0,
        active,
        ..input(&floors, stick, aim)
    };
    assert_eq!(travel.update(short(Vec2::Y, true)).blink, 0.0);
    let landed = travel.update(short(Vec2::ZERO, true));
    assert!(landed.teleported);
    assert_eq!(landed.blink, 1.0);
    assert!((travel.update(short(Vec2::ZERO, true)).blink - 0.75).abs() < 1e-5);
    // It comes back while travel is inactive too, as while the hand menu shows, and nonsense time stands still.
    assert!((travel.update(short(Vec2::ZERO, false)).blink - 0.5).abs() < 1e-5);
    let nonsense = Input {
        dt: f32::NAN,
        ..short(Vec2::ZERO, true)
    };
    assert!((travel.update(nonsense).blink - 0.5).abs() < 1e-5);
    for _ in 0..3 {
        travel.update(short(Vec2::ZERO, true));
    }
    assert_eq!(travel.update(short(Vec2::ZERO, true)).blink, 0.0);
    // Turning does not darken it.
    assert_eq!(travel.update(short(Vec2::X, true)).blink, 0.0);
}

#[test]
fn the_rig_goes_home_on_leaving_changing_environment_or_without_travel() {
    let floors = floors();
    let mut travel = Travel::default();
    let turn = |travel: &mut Travel, input: Input| {
        travel.update(Input {
            stick: Vec2::ZERO,
            ..input
        });
        travel.update(input);
    };
    turn(&mut travel, input(&floors, Vec2::X, None));
    assert_ne!(travel.rig(), Rig::default());
    // Inactive, as while the hand menu shows, it stays where it is.
    travel.update(Input {
        active: false,
        ..input(&floors, Vec2::ZERO, None)
    });
    assert_ne!(travel.rig(), Rig::default());
    // Leaving the space.
    travel.update(Input {
        place: None,
        ..input(&floors, Vec2::ZERO, None)
    });
    assert_eq!(travel.rig(), Rig::default());
    // Another space in the same environment.
    turn(&mut travel, input(&floors, Vec2::X, None));
    assert_ne!(travel.rig(), Rig::default());
    assert_eq!(
        travel.settle(Some(("space-2", Environment::new("floor"))), &floors),
        Rig::default()
    );
    // Another environment.
    turn(&mut travel, input(&floors, Vec2::X, None));
    travel.update(Input {
        place: Some(("space-1", crate::test_support::SKY)),
        ..input(&floors, Vec2::ZERO, None)
    });
    assert_eq!(travel.rig(), Rig::default());
    // No travel in the same environment, and nothing moves then.
    turn(&mut travel, input(&floors, Vec2::X, None));
    let none = Locomotion::None;
    travel.update(Input {
        place: Some(("space-1", crate::test_support::SKY)),
        ..input(&none, Vec2::X, None)
    });
    assert_eq!(travel.rig(), Rig::default());
    assert_eq!(
        travel.update(Input {
            place: Some(("space-1", crate::test_support::SKY)),
            ..input(&none, Vec2::X, None)
        }),
        Outcome::default()
    );
}

#[test]
fn a_profile_declared_in_the_environment_s_coordinates_is_carried_into_the_space_s() {
    // A deck 4 m by 1 m along X at the environment's origin, and participants arriving 8 m along +X, raised 0.5 m,
    // facing its origin: in the space, the deck stands 8 m ahead, 0.5 m lower, and runs along Z.
    let deck = Locomotion::fly(
        vec![Surface::rectangle(
            Vec3::new(0.0, 0.5, 0.0),
            Vec2::new(4.0, 1.0),
        )],
        6.0,
    );
    let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
    let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
    let carried = deck.carried(&carriage);
    assert_eq!(carried.ceiling(), Some(5.5));
    let surface = &carried.surfaces()[0];
    assert!(surface.height().abs() < 1e-5);
    assert!(surface.contains(Vec3::new(0.0, 0.0, -9.9)));
    assert!(surface.contains(Vec3::new(0.4, 0.0, -8.0)));
    assert!(!surface.contains(Vec3::new(1.0, 0.0, -8.0)));
    // A ray from home lands on it there.
    let landing = carried
        .landing(&ray(Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.0, -7.0)))
        .expect("lands");
    assert!(
        landing.distance(Vec3::new(0.0, 0.0, -7.0)) < 1e-3,
        "{landing}"
    );
    // Unmoved, it is as declared; and none stays none.
    assert_eq!(deck.carried(&crate::anchors::Anchor::HOME), deck);
    assert_eq!(Locomotion::None.carried(&carriage), Locomotion::None);
    let discs = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 1.0)]).carried(&carriage);
    assert!(discs.surfaces()[0].contains(Vec3::new(0.0, 0.0, -8.5)));
}

/// Ground as an environment draws it: a yard 10 m square at its earth's height, a shed standing in it, and a ramp
/// rising 1 m over 4 m, about 14°, up to a deck 1 m up beyond the yard along −Z.
fn drawn_ground() -> Surface {
    let at = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let ground = [
        [at(-5.0, 0.0, 5.0), at(5.0, 0.0, 5.0), at(5.0, 0.0, -5.0)],
        [at(-5.0, 0.0, 5.0), at(5.0, 0.0, -5.0), at(-5.0, 0.0, -5.0)],
        [at(-1.0, 0.0, -5.0), at(1.0, 0.0, -5.0), at(1.0, 1.0, -9.0)],
        [at(-1.0, 0.0, -5.0), at(1.0, 1.0, -9.0), at(-1.0, 1.0, -9.0)],
        [at(-3.0, 1.0, -9.0), at(3.0, 1.0, -9.0), at(3.0, 1.0, -12.0)],
        [
            at(-3.0, 1.0, -9.0),
            at(3.0, 1.0, -12.0),
            at(-3.0, 1.0, -12.0),
        ],
    ];
    let shed = Obstacle::cuboid(Vec3::new(2.0, 0.0, 1.0), Vec3::new(4.0, 2.4, 3.0));
    Surface::mesh(Navmesh::derive(ground, &[shed]))
}

#[test]
fn ground_drawn_as_triangles_is_landed_on_at_the_height_it_is_drawn_and_stepped_back_along() {
    let ground = drawn_ground();
    assert_eq!(ground.height(), 0.0);
    assert_eq!(ground.navmesh().map(Navmesh::is_empty), Some(false));
    assert_eq!(Surface::disc(Vec3::ZERO, 1.0).navmesh(), None);
    assert_eq!(ground.height_at(Vec3::new(0.0, 5.0, 0.0)), Some(0.0));
    assert!((ground.height_at(Vec3::new(0.0, 0.0, -7.0)).unwrap() - 0.5).abs() < 1e-5);
    assert_eq!(ground.height_at(Vec3::new(3.0, 0.0, 2.0)), None);
    assert_eq!(Surface::disc(Vec3::Y, 1.0).height_at(Vec3::ZERO), Some(1.0));
    assert_eq!(Surface::disc(Vec3::Y, 1.0).height_at(Vec3::X * 2.0), None);
    let profile = Locomotion::teleport(vec![ground.clone()]);
    // Onto the yard, up the ramp and onto the deck, each where it is drawn; never into the shed.
    let eyes = Vec3::new(0.0, 1.6, 4.0);
    for (towards, height) in [
        (Vec3::new(-2.0, 0.0, 0.0), 0.0),
        (Vec3::new(0.0, 0.25, -6.0), 0.25),
        (Vec3::new(1.5, 1.0, -10.0), 1.0),
    ] {
        let landing = profile.landing(&ray(eyes, towards)).expect("lands");
        assert!(close(landing, towards), "{landing}");
        assert!((landing.y - height).abs() < 1e-5, "{landing}");
    }
    assert_eq!(profile.landing(&ray(eyes, Vec3::new(3.0, 0.0, 2.0))), None);
    // A backstep goes along the floor as far as it stays within a centimetre of the floor's height: across the
    // yard, but not off its edge, nor up the ramp.
    let head = Vec3::new(0.0, 1.6, 0.0);
    let back = profile.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
    assert!(close(back, Vec3::new(0.0, 0.0, BACKSTEP)));
    let edge = profile
        .backstep(Vec3::new(0.0, 1.6, 4.7), Vec3::NEG_Z, 0.0)
        .unwrap();
    assert!((edge.z - 0.3).abs() < 1e-4, "{edge}");
    let foot = Vec3::new(0.0, 1.6, -4.9);
    assert_eq!(profile.backstep(foot, Vec3::Z, 0.0), None);
    // A pad on it, at its height, may be used; one floating above it may not.
    let pads = profile.clone().with_pads([
        [
            Pad::at(Vec3::new(-3.0, 0.0, 3.0)),
            Pad::at(Vec3::new(0.0, 1.0, -11.0)),
        ],
        [
            Pad::at(Vec3::new(-3.0, 0.5, 3.0)),
            Pad::at(Vec3::new(0.0, 1.0, -11.0)),
        ],
    ]);
    assert_eq!(pads.pads().count(), 1);
    // It hides a pad beneath it from above, as a deck does.
    assert!(profile.surface_crossed(
        &ray(Vec3::new(0.0, 3.0, -10.0), Vec3::new(0.0, -1.0, -10.0)),
        5.0
    ));
    assert!(!profile.surface_crossed(
        &ray(Vec3::new(0.0, 3.0, -10.0), Vec3::new(0.0, 2.0, -10.0)),
        1.5
    ));
    assert!(profile.surface_crossed(
        &ray(Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
        5.0
    ));
    // Carried into a space whose arrival is 8 m along +X facing the origin, it stands 8 m ahead, turned.
    let street = tomori_protocol::Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0, 0.0, 0.0]);
    let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
    let carried = profile.carried(&carriage);
    let surface = &carried.surfaces()[0];
    assert_eq!(surface.navmesh(), ground.navmesh());
    let shed = carriage.place(Vec3::new(3.0, 0.0, 2.0));
    assert!(!surface.contains(shed) && surface.contains(carriage.place(Vec3::ZERO)));
    let deck = carriage.place(Vec3::new(1.5, 1.0, -10.0));
    let landing = carried
        .landing(&ray(Vec3::new(0.0, 1.6, 0.0), deck))
        .expect("lands");
    assert!(close(landing, deck), "{landing} {deck}");
    // A mesh with no ground holds nothing.
    let empty = Surface::mesh(Navmesh::from_triangles([]));
    assert_eq!(empty.height(), 0.0);
    assert!(!empty.contains(Vec3::ZERO));
}

/// An L-shaped deck 3 m up: 4 m square, to the right of home, its near right quarter cut away.
fn ell() -> Surface {
    let corners = [
        (2.0, -2.0),
        (6.0, -2.0),
        (6.0, 0.0),
        (4.0, 0.0),
        (4.0, 2.0),
        (2.0, 2.0),
    ];
    Surface::polygon(corners.map(|(x, z)| Vec2::new(x, z)), 3.0).unwrap()
}

/// A crescent at `height`, 4 m deep at its middle, 4 m ahead of home, its horns 8 m to either side.
fn crescent(height: f32) -> Surface {
    Surface::polygon(outline::crescent(), height).unwrap()
}

/// A curved deck at `height`: 2 m deep, a third of the way round, 6 m ahead of `centre` at its middle.
fn curve(centre: Vec3) -> Surface {
    let third = std::f32::consts::FRAC_PI_3;
    Surface::sector(centre, 6.0, 8.0, -third, third)
}

/// The point `radius` from `centre` in the direction `angle`, a turn about the vertical from −Z.
fn bearing(centre: Vec3, angle: f32, radius: f32) -> Vec3 {
    centre + radius * (Quat::from_rotation_y(angle) * Vec3::NEG_Z)
}

/// A grid of points at `height`, an eighth of a metre apart and `offset` from the whole eighths, from `low` to
/// `high` on X and Z.
fn grid(low: f32, high: f32, offset: f32, height: f32) -> impl Iterator<Item = Vec3> {
    let (low, high) = ((low * 8.0) as i32, (high * 8.0) as i32);
    (low..=high).flat_map(move |x| {
        (low..=high)
            .map(move |z| Vec3::new(x as f32 / 8.0 + offset, height, z as f32 / 8.0 + offset))
    })
}

#[test]
fn a_rectangle_declared_as_four_corners_is_that_rectangle() {
    let centre = Vec3::new(1.5, 2.0, -3.25);
    let size = Vec2::new(4.0, 2.5);
    let rectangle = Surface::rectangle(centre, size);
    let corners = [(-0.5, -4.5), (3.5, -4.5), (3.5, -2.0), (-0.5, -2.0)];
    let polygon = Surface::polygon(corners.map(|(x, z)| Vec2::new(x, z)), centre.y).unwrap();
    assert_eq!(polygon.height(), rectangle.height());
    // Every point of a grid, those on its edges and corners among them, is on both or neither.
    for point in grid(-6.0, 6.0, 0.0, centre.y) {
        assert_eq!(
            polygon.contains(point),
            rectangle.contains(point),
            "{point}"
        );
    }
    let held = |surface: &Surface| {
        grid(-6.0, 6.0, 0.0, centre.y)
            .filter(|point| surface.contains(*point))
            .count()
    };
    assert_eq!(held(&polygon), 33 * 21);
    assert_eq!(held(&rectangle), 33 * 21);
    // Rays land on both alike, and backsteps go as far along both.
    let rectangles = Locomotion::teleport(vec![rectangle]);
    let polygons = Locomotion::teleport(vec![polygon]);
    let eye = centre + Vec3::new(0.3, 2.0, 0.4);
    let mut landed = 0;
    for point in grid(-6.0, 6.0, 0.0625, centre.y) {
        let aim = ray(eye, point);
        assert_eq!(polygons.landing(&aim), rectangles.landing(&aim), "{point}");
        landed += usize::from(polygons.landing(&aim).is_some());
        for facing in [Vec3::X, Vec3::NEG_Z, Vec3::new(1.0, 0.0, 1.0)] {
            assert_eq!(
                polygons.backstep(point, facing, centre.y),
                rectangles.backstep(point, facing, centre.y),
                "{point} facing {facing}"
            );
        }
    }
    assert_eq!(landed, 32 * 20);
    // It steps back as far as the outline goes, as a rectangle's edge stops it.
    let step = polygons
        .backstep(Vec3::new(3.0, 3.7, -3.0), Vec3::NEG_X, centre.y)
        .unwrap();
    assert!(close(step, Vec3::new(0.5, 0.0, 0.0)), "{step}");
}

#[test]
fn a_ray_lands_on_a_polygon_and_a_sector_as_on_any_surface() {
    let floor = Surface::rectangle(Vec3::ZERO, Vec2::new(40.0, 40.0));
    let floors = Locomotion::teleport(vec![floor.clone(), ell(), curve(Vec3::new(0.0, 5.4, 0.0))]);
    // From above the deck, onto each of its arms.
    let above = Vec3::new(3.0, 4.6, 0.0);
    for arm in [Vec3::new(5.0, 3.0, -1.0), Vec3::new(3.0, 3.0, 1.5)] {
        assert!(close(floors.landing(&ray(above, arm)).unwrap(), arm));
    }
    // Through its notch, onto the floor beneath, which the deck hides elsewhere.
    let over = Vec3::new(5.0, 6.0, 1.0);
    let through = floors
        .landing(&ray(over, Vec3::new(5.2, 0.0, 1.2)))
        .unwrap();
    assert!(close(through, Vec3::new(5.2, 0.0, 1.2)));
    let hidden = floors
        .landing(&ray(above, Vec3::new(3.5, 0.0, 0.5)))
        .unwrap();
    assert_eq!(hidden.y, 3.0);
    assert!(floors.surface_crossed(&ray(above, Vec3::new(3.5, 0.0, 0.5)), 4.0));
    assert!(!floors.surface_crossed(&ray(over, Vec3::new(5.2, 0.0, 1.2)), 5.9));
    // From beneath, it is not met, and the floor beyond it is; just above it, it is too near, and still hides.
    let eye = Vec3::new(0.0, 1.5, 0.0);
    let beneath = floors.landing(&ray(eye, Vec3::new(3.0, 0.0, 0.0))).unwrap();
    assert_eq!(beneath.y, 0.0);
    let low = Vec3::new(3.0, 3.1, 0.0);
    assert_eq!(floors.landing(&ray(low, Vec3::new(3.1, 3.0, 0.0))), None);
    // The curved deck, from above it: onto it, through the hollow within it and past its ends onto the floor.
    let high = Vec3::new(0.0, 7.0, -5.0);
    let middle = Vec3::new(0.0, 5.4, -7.0);
    assert!(close(floors.landing(&ray(high, middle)).unwrap(), middle));
    let hollow = floors
        .landing(&ray(high, Vec3::new(0.0, 0.0, -3.0)))
        .unwrap();
    assert_eq!(hollow.y, 0.0);
    let past = floors
        .landing(&ray(Vec3::new(7.0, 7.0, 0.0), Vec3::new(7.0, 0.0, -1.0)))
        .unwrap();
    assert_eq!(past.y, 0.0);
    // Flying lands on them as teleporting does.
    let fly = Locomotion::fly(vec![floor, ell()], 6.0);
    let landing = fly.landing(&ray(above, Vec3::new(5.0, 3.0, -1.0))).unwrap();
    assert!(close(landing, Vec3::new(5.0, 3.0, -1.0)));
    // A backstep goes along the deck's arm, and stops at its notch.
    let step = floors
        .backstep(Vec3::new(4.5, 4.7, -1.0), Vec3::NEG_X, 3.0)
        .unwrap();
    assert!(close(step, Vec3::new(BACKSTEP, 0.0, 0.0)), "{step}");
    let step = floors
        .backstep(Vec3::new(3.0, 4.7, -0.3), Vec3::new(-1.0, 0.0, -1.0), 3.0)
        .unwrap();
    assert!(
        close(step, Vec3::new(0.3, 0.0, 0.3) * 2.0_f32.sqrt()),
        "{step}"
    );
}

#[test]
fn aiming_at_a_crescent_lands_on_it_and_not_in_its_hollow() {
    let dais = Locomotion::teleport(vec![crescent(0.5)]);
    let mut travel = Travel::default();
    let eye = Vec3::new(0.3, 1.2, -0.4);
    // The hollow, between home and the crescent, is nowhere to land.
    let hollow = Some(ray(eye, Vec3::new(0.0, 0.5, -3.0)));
    let outcome = travel.update(input(&dais, Vec2::Y, hollow));
    assert!(outcome.aiming);
    assert_eq!(outcome.landing, None);
    // Its middle, and one of its horns, are.
    for landing in [Vec3::new(0.0, 0.5, -6.0), Vec3::new(-7.4, 0.5, -1.0)] {
        let outcome = travel.update(input(&dais, Vec2::Y, Some(ray(eye, landing))));
        assert!(close(outcome.landing.unwrap(), landing), "{landing}");
    }
    let outcome = travel.update(input(&dais, Vec2::ZERO, hollow));
    assert!(outcome.teleported);
    let head = travel.rig().matrix().transform_point3(HEAD);
    assert!(close(head, Vec3::new(-7.4, HEAD.y + 0.5, -1.0)), "{head}");
    assert_eq!(travel.rig().floor(), 0.5);
}

#[test]
fn a_sector_is_the_part_of_a_ring_between_its_angles() {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
    let centre = Vec3::new(1.0, 2.0, -1.0);
    let sector = Surface::sector(centre, 2.0, 4.0, -FRAC_PI_4, FRAC_PI_2);
    assert_eq!(sector.height(), 2.0);
    // Ahead of its centre, to the left as far as −X, and half as far to the right; seen from above, at any height.
    for (angle, radius) in [
        (0.0, 3.0),
        (FRAC_PI_4, 3.0),
        (FRAC_PI_2 - 0.01, 3.0),
        (-FRAC_PI_4 + 0.01, 3.0),
        (0.3, 2.01),
        (0.3, 3.99),
    ] {
        let point = bearing(centre, angle, radius);
        assert!(sector.contains(point), "{angle} {radius}");
        assert!(sector.contains(point + Vec3::Y * 9.0), "{angle} {radius}");
    }
    assert!(sector.contains(centre + Vec3::new(-3.0, 0.0, -0.1)));
    assert!(sector.contains(centre + Vec3::new(2.0, 0.0, -2.1)));
    // Not within the ring, beyond it, past either end, or behind.
    for (angle, radius) in [
        (0.3, 1.9),
        (0.3, 4.1),
        (FRAC_PI_2 + 0.05, 3.0),
        (-FRAC_PI_4 - 0.05, 3.0),
        (PI, 3.0),
        (0.0, 0.0),
    ] {
        assert!(
            !sector.contains(bearing(centre, angle, radius)),
            "{angle} {radius}"
        );
    }
    // An end less than the start is a turn further round: this one is behind, a quarter turn about.
    let behind = Surface::sector(centre, 2.0, 4.0, 3.0 * FRAC_PI_4, -3.0 * FRAC_PI_4);
    for (angle, on) in [
        (PI, true),
        (PI - 0.7, true),
        (-PI + 0.7, true),
        (0.0, false),
        (FRAC_PI_2, false),
    ] {
        assert_eq!(behind.contains(bearing(centre, angle, 3.0)), on, "{angle}");
    }
    // A full turn, or more, is the whole ring, with its hole.
    for ring in [
        Surface::sector(centre, 2.0, 4.0, 0.0, TAU),
        Surface::sector(centre, 2.0, 4.0, 1.0, 9.0),
    ] {
        for step in 0..64 {
            let angle = step as f32 / 64.0 * TAU - PI;
            assert!(ring.contains(bearing(centre, angle, 3.0)), "{angle}");
            assert!(!ring.contains(bearing(centre, angle, 1.9)), "{angle}");
            assert!(!ring.contains(bearing(centre, angle, 4.1)), "{angle}");
        }
        assert!(!ring.contains(centre));
    }
    // Without an inner radius, it is a wedge of a disc, whose point is on it.
    let wedge = Surface::sector(centre, 0.0, 4.0, -FRAC_PI_4, FRAC_PI_4);
    assert!(wedge.contains(centre));
    assert!(wedge.contains(bearing(centre, 0.0, 0.1)));
    assert!(!wedge.contains(bearing(centre, PI, 0.1)));
    // Its radii are taken without their signs, and in order.
    assert_eq!(
        Surface::sector(centre, -4.0, 2.0, -FRAC_PI_4, FRAC_PI_2),
        sector
    );
    // Nonsense holds nothing, not even its centre.
    for nonsense in [
        Surface::sector(centre, f32::NAN, 4.0, 0.0, 1.0),
        Surface::sector(centre, 0.0, f32::NAN, 0.0, 1.0),
        Surface::sector(centre, 0.0, 4.0, f32::NAN, 1.0),
        Surface::sector(centre, 0.0, 4.0, 0.0, f32::NAN),
        Surface::sector(centre, 0.0, 4.0, 0.0, f32::INFINITY),
        Surface::sector(centre, 0.0, 4.0, f32::NEG_INFINITY, 0.0),
    ] {
        for point in grid(-6.0, 6.0, 0.0, 2.0) {
            assert!(!nonsense.contains(point), "{nonsense:?} {point}");
        }
    }
}

#[test]
fn a_sector_that_sweeps_nothing_holds_nothing() {
    use std::f32::consts::TAU;
    let centre = Vec3::new(1.0, 2.0, -1.0);
    for (from, to) in [
        (0.0, 0.0),
        (1.0, 1.0),
        (TAU, 0.0),
        (0.0, -TAU),
        (-2.0, -2.0),
    ] {
        for inner in [0.0, 2.0] {
            let none = Surface::sector(centre, inner, 4.0, from, to);
            // Not even along the line where it starts, nor its centre.
            for radius in [0.0, 1.0, 2.0, 3.0, 4.0] {
                for angle in [from, to, from.next_up(), from.next_down()] {
                    let point = bearing(centre, angle, radius);
                    assert!(!none.contains(point), "{from} {to} {inner} {point}");
                }
            }
            for point in grid(-6.0, 6.0, 0.0, 2.0) {
                assert!(!none.contains(point), "{from} {to} {point}");
            }
            assert!(none.sector_triangles(16).is_empty());
        }
    }
    // A sweep of a thousandth of a radian still holds what is within it.
    let sliver = Surface::sector(centre, 2.0, 4.0, 0.0, 1e-3);
    assert!(sliver.contains(bearing(centre, 5e-4, 3.0)));
}

#[test]
fn a_sector_s_triangles_follow_its_arcs_within_the_chord_error() {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let centre = Vec3::new(1.0, 2.0, -1.0);
    // Half a ring of 10 m, from −X round to +X through −Z, in 32 steps.
    let half = Surface::sector(centre, 6.0, 10.0, FRAC_PI_2, FRAC_PI_2 + PI);
    let floor = half.sector_triangles(32);
    assert_eq!(floor.len(), 64);
    // Every corner on one of its arcs, at its height, and the floor within it: each triangle's middle on it.
    for triangle in &floor {
        for corner in triangle {
            assert_eq!(corner.y, 2.0);
            let radius = (*corner - centre).length();
            assert!(
                (radius - 6.0).abs() < 1e-4 || (radius - 10.0).abs() < 1e-4,
                "{radius}"
            );
        }
        let middle = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
        assert!(half.contains(middle), "{middle}");
    }
    // The floor falls short of the outer arc, between corners, by the chord error, about 1.2 cm.
    let error = 10.0 * (1.0 - (PI / 64.0).cos());
    assert!((error - 0.012).abs() < 0.001, "{error}");
    let chord = (floor[0][1] + floor[0][2]) / 2.0;
    assert!(((chord - centre).length() - (10.0 - error)).abs() < 1e-4);
    // A wedge without an inner radius is one triangle a step, from its point; a whole ring closes.
    let wedge = Surface::sector(centre, 0.0, 4.0, 0.0, 1.0).sector_triangles(8);
    assert_eq!(wedge.len(), 8);
    assert!(wedge.iter().all(|triangle| triangle[0] == centre));
    let ring = Surface::sector(centre, 2.0, 4.0, 0.0, TAU).sector_triangles(12);
    let (first, last) = (ring[0][0], ring[23][2]);
    assert!(close(first, last), "{first} {last}");
    // Its steps are from 1 to the most there may be.
    assert_eq!(half.sector_triangles(0).len(), 2);
    assert_eq!(half.sector_triangles(usize::MAX).len(), 2 * MOST_SEGMENTS);
    // Carried, its floor is carried with it.
    let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
    let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
    let carried = half.carried(&carriage).sector_triangles(32);
    for (there, here) in carried.iter().flatten().zip(floor.iter().flatten()) {
        assert!(
            there.distance(carriage.place(*here)) < 1e-3,
            "{there} {here}"
        );
    }
    // The other shapes, and nonsense, have none.
    assert!(ell().sector_triangles(8).is_empty());
    assert!(Surface::disc(centre, 4.0).sector_triangles(8).is_empty());
    for nonsense in [
        Surface::sector(centre, f32::NAN, 4.0, 0.0, 1.0),
        Surface::sector(centre, 0.0, f32::NAN, 0.0, 1.0),
        Surface::sector(centre, 0.0, 4.0, f32::NAN, 1.0),
    ] {
        assert!(nonsense.sector_triangles(8).is_empty(), "{nonsense:?}");
    }
}

#[test]
fn a_rectangle_as_four_corners_carried_holds_the_same_up_to_rounding() {
    let centre = Vec3::new(1.5, 2.0, -3.25);
    let half = Vec2::new(2.0, 1.25);
    let rectangle = Surface::rectangle(centre, 2.0 * half);
    let corners = [(-0.5, -4.5), (3.5, -4.5), (3.5, -2.0), (-0.5, -2.0)];
    let polygon = Surface::polygon(corners.map(|(x, z)| Vec2::new(x, z)), centre.y).unwrap();
    // Near home, and nearly a kilometre away, turned.
    for (at, towards) in [
        ([8.0, 0.5, 0.0], [0.0, 0.5, 0.0]),
        ([900.0, -3.0, -700.0], [905.0, -3.0, -690.0]),
    ] {
        let carriage = crate::anchors::Anchor::carriage(
            &tomori_protocol::Pose::at(at).facing_towards(towards),
        )
        .unwrap();
        let (rectangles, polygons) = (rectangle.carried(&carriage), polygon.carried(&carriage));
        // Every point of a grid, but those within a millimetre of an edge, where rounding decides, is on both or
        // neither; and a millimetre within each edge's middle and corner is on both, a millimetre beyond on
        // neither.
        let mut held = 0;
        for point in grid(-6.0, 6.0, 0.0, centre.y) {
            let local = (point - centre).abs();
            if (local.x - half.x).abs() < 1e-3 || (local.z - half.y).abs() < 1e-3 {
                continue;
            }
            let there = carriage.place(point);
            assert_eq!(
                polygons.contains(there),
                rectangles.contains(there),
                "{point}"
            );
            held += usize::from(polygons.contains(there));
        }
        assert_eq!(held, 31 * 19);
        for (x, z) in [(1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            for direction in [Vec2::new(x, z), Vec2::new(-x, -z)] {
                let edge = centre + Vec3::new(direction.x * half.x, 0.0, direction.y * half.y);
                let outward = Vec3::new(direction.x, 0.0, direction.y) * 1e-3;
                for surface in [&rectangles, &polygons] {
                    assert!(surface.contains(carriage.place(edge - outward)), "{edge}");
                    assert!(!surface.contains(carriage.place(edge + outward)), "{edge}");
                }
            }
        }
    }
}

#[test]
fn a_polygon_is_declared_by_its_outline_or_refused() {
    let ell = ell();
    assert_eq!(ell.height(), 3.0);
    let outline = ell.outline().unwrap();
    assert_eq!(outline.points().len(), 6);
    assert_eq!(outline.triangles().count(), 4);
    // Seen from above, at any height; its edge and the corner that turns inwards are on it, its notch is not.
    assert!(ell.contains(Vec3::new(5.0, 9.0, -1.0)));
    assert!(ell.contains(Vec3::new(4.0, 3.0, 0.0)));
    assert!(ell.contains(Vec3::new(6.0, 3.0, -1.0)));
    assert!(!ell.contains(Vec3::new(5.0, 3.0, 1.0)));
    // The other shapes have no outline.
    assert_eq!(Surface::disc(Vec3::ZERO, 1.0).outline(), None);
    assert_eq!(curve(Vec3::ZERO).outline(), None);
    // An outline that is not one is refused, saying why, and a height that is nonsense is never met.
    let bow = [(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)].map(|(x, z)| Vec2::new(x, z));
    assert_eq!(
        Surface::polygon(bow, 0.0),
        Err(OutlineError::Crossing { edges: [0, 2] })
    );
    assert_eq!(
        Surface::polygon([Vec2::ZERO, Vec2::X], 0.0),
        Err(OutlineError::Count { points: 2 })
    );
    let adrift = Surface::polygon(outline.points().iter().copied(), f32::NAN).unwrap();
    let adrift = Locomotion::teleport(vec![adrift]);
    let above = Vec3::new(3.0, 4.6, 0.0);
    assert_eq!(adrift.landing(&ray(above, Vec3::new(5.0, 3.0, -1.0))), None);
}

#[test]
fn polygons_and_sectors_carried_into_a_space_hold_the_same_places() {
    let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
    let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
    let again = tomori_protocol::Pose::at([-3.0, -1.0, 2.0]).facing_towards([5.0, -1.0, 9.0]);
    let again = crate::anchors::Anchor::carriage(&again).unwrap();
    let ring = Surface::sector(
        Vec3::new(-2.0, 1.0, 1.0),
        1.0,
        3.0,
        2.0,
        2.0 + std::f32::consts::TAU,
    );
    for surface in [ell(), crescent(5.4), curve(Vec3::new(1.0, 2.0, -1.0)), ring] {
        // Carried once, and from there again.
        let once = surface.carried(&carriage);
        let twice = once.carried(&again);
        assert!((once.height() - (surface.height() - 0.5)).abs() < 1e-5);
        assert!((twice.height() - (surface.height() + 0.5)).abs() < 1e-5);
        // Its outline is as it was declared, and shared rather than copied.
        assert_eq!(twice.outline(), surface.outline());
        // Every point of a grid, but those within a millimetre of its edge, where rounding decides, is on it
        // wherever it is carried exactly where it was on it as declared.
        let settled = |point: &Vec3| {
            (0..16).all(|step| {
                let beside = bearing(*point, step as f32 * std::f32::consts::FRAC_PI_8, 1e-3);
                surface.contains(beside) == surface.contains(*point)
            })
        };
        let (mut on, mut off) = (0, 0);
        for point in grid(-10.0, 10.0, 0.0, surface.height()).filter(settled) {
            let held = surface.contains(point);
            let there = carriage.place(point);
            assert_eq!(once.contains(there), held, "{surface:?} {point}");
            assert_eq!(
                twice.contains(again.place(there)),
                held,
                "{surface:?} {point}"
            );
            on += usize::from(held);
            off += usize::from(!held);
        }
        assert!(on > 500 && off > 500, "{on} {off}");
        // Unmoved, it is as declared.
        assert_eq!(surface.carried(&crate::anchors::Anchor::HOME), surface);
    }
    // A profile carries them with the rest, and a ray lands on them there: on the L's arm, and not in its notch.
    let profile = Locomotion::fly(vec![ell(), crescent(5.4)], 8.0).carried(&carriage);
    assert_eq!(profile.ceiling(), Some(7.5));
    let arm = carriage.place(Vec3::new(5.0, 3.0, -1.0));
    let landing = profile
        .landing(&ray(arm + Vec3::new(0.5, 2.0, 0.5), arm))
        .expect("lands");
    assert!(landing.distance(arm) < 1e-3, "{landing}");
    let notch = carriage.place(Vec3::new(5.0, 3.0, 1.0));
    assert_eq!(
        profile.landing(&ray(notch + Vec3::new(0.2, 2.0, 0.2), notch)),
        None
    );
}

#[test]
fn pads_stand_on_polygons_and_sectors_as_on_any_surface() {
    let deck = Vec3::new(0.0, 5.4, 0.0);
    let pair = |pad: Pad| {
        Locomotion::teleport(vec![
            Surface::rectangle(Vec3::ZERO, Vec2::new(30.0, 30.0)),
            ell(),
            curve(deck),
        ])
        .with_pads([[Pad::at(Vec3::new(0.0, 0.0, 1.0)), pad]])
    };
    // On the L's arms, on its edge and on the corner that turns inwards, as a pad's centre on a rectangle's edge
    // is; and on the curved deck.
    for good in [
        Vec3::new(5.0, 3.0, -1.0),
        Vec3::new(3.0, 3.0, 1.5),
        Vec3::new(6.0, 3.0, -1.0),
        Vec3::new(4.0, 3.0, 0.0),
        Vec3::new(0.0, 5.4, -7.0),
        Vec3::new(-5.0, 5.4, -5.0),
    ] {
        assert_eq!(pair(Pad::at(good)).pads().count(), 1, "{good}");
    }
    // In the L's notch, above it, beside it, in the curved deck's hollow, past its end and behind it: the pair is
    // not used.
    for bad in [
        Vec3::new(5.0, 3.0, 1.0),
        Vec3::new(5.0, 3.5, -1.0),
        Vec3::new(7.0, 3.0, -1.0),
        Vec3::new(0.0, 5.4, -5.0),
        Vec3::new(-7.0, 5.4, 0.0),
        Vec3::new(0.0, 5.4, 7.0),
    ] {
        assert_eq!(pair(Pad::at(bad)).pads().count(), 0, "{bad}");
    }
}

/// A main floor and racks 4.5 m below it, with a pad on each, the racks' facing −X.
fn storeys() -> Locomotion {
    Locomotion::teleport(vec![
        Surface::rectangle(Vec3::ZERO, Vec2::new(20.0, 20.0)),
        Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::new(20.0, 20.0)),
    ])
    .with_pads([[
        Pad::at(Vec3::new(0.3, 0.0, -0.2)).named("Main floor"),
        Pad::at(Vec3::new(5.0, -4.5, 1.0))
            .named("Racks")
            .facing(Vec3::new(-2.0, 7.0, 0.0)),
    ]])
}

#[test]
fn pads_are_declared_where_they_are_and_may_say_more() {
    let pad = Pad::at(Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(pad.centre(), Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(pad.radius(), PAD_RADIUS);
    assert_eq!((pad.yaw(), pad.name()), (None, None));
    let pad = pad
        .with_radius(-0.8)
        .named("Deck")
        .facing(Vec3::new(1.0, -5.0, 0.0));
    assert_eq!(pad.radius(), 0.8);
    assert_eq!(pad.name(), Some("Deck"));
    // Facing +X, a quarter turn to the right of −Z, as the rig turns.
    let yaw = pad.yaw().unwrap();
    assert!(close(Quat::from_rotation_y(yaw) * Vec3::NEG_Z, Vec3::X));
    // Straight up or down faces the way they faced.
    assert_eq!(pad.clone().facing(Vec3::Y).yaw(), None);
    // Within its radius and a margin, seen from above.
    assert!(pad.reaches(Vec3::new(1.7, -9.0, 3.3), 0.0));
    assert!(!pad.reaches(Vec3::new(1.9, 2.0, 3.0), 0.0));
    assert!(pad.reaches(Vec3::new(1.9, 2.0, 3.0), 0.15));
}

#[test]
fn only_pads_that_stand_on_a_surface_within_bounds_are_used() {
    let floor = || vec![Surface::disc(Vec3::ZERO, crate::anchors::REACH + 10.0)];
    let pair = |a: Pad, b: Pad| Locomotion::teleport(floor()).with_pads([[a, b]]);
    let good = || Pad::at(Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(pair(good(), good()).pads().count(), 1);
    // Off every surface, above one, too wide, beyond the space's bounds, nonsense: the pair is not used.
    for bad in [
        Pad::at(Vec3::new(2.0, 0.5, 0.0)),
        Pad::at(Vec3::new(2.0, 0.0, 0.0)).with_radius(2.5),
        Pad::at(Vec3::new(2.0, 0.0, 0.0)).with_radius(0.0),
        Pad::at(Vec3::new(crate::anchors::REACH + 0.5, 0.0, 0.0)),
        Pad::at(Vec3::new(f32::NAN, 0.0, 0.0)),
        Pad {
            yaw: Some(f32::INFINITY),
            ..good()
        },
    ] {
        assert_eq!(pair(good(), bad.clone()).pads().count(), 0, "{bad:?}");
        assert_eq!(pair(bad.clone(), good()).pads().count(), 0, "{bad:?}");
    }
    // Flying, as teleporting; no more than eight pairs; and a profile of none has none.
    let many = Locomotion::fly(floor(), 5.0).with_pads((0..10).map(|_| [good(), good()]));
    assert_eq!(many.pads().count(), MOST_PAIRS);
    assert_eq!(
        Locomotion::None.with_pads([[good(), good()]]),
        Locomotion::None
    );
    assert_eq!(Locomotion::None.pads().count(), 0);
}

#[test]
fn pads_are_carried_into_the_space_with_the_profile() {
    let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
    let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
    for profile in [storeys(), Locomotion::fly(Vec::new(), 1.0)] {
        let declared: Vec<_> = profile.pads().cloned().collect();
        let carried = profile.carried(&carriage);
        for (pair, declared) in carried.pads().zip(&declared) {
            for (pad, declared) in pair.iter().zip(declared) {
                assert!(close(pad.centre(), carriage.place(declared.centre())));
                assert_eq!(pad.name(), declared.name());
            }
        }
    }
    let carried = storeys().carried(&carriage);
    let [floor, racks] = carried.pads().next().unwrap();
    assert!(close(floor.centre(), Vec3::new(0.2, -0.5, -7.7)));
    assert_eq!(floor.yaw(), None);
    // Facing −X in the environment, the way participants arrive facing, it faces −Z in the space.
    let facing = Quat::from_rotation_y(racks.yaw().unwrap()) * Vec3::NEG_Z;
    assert!(close(facing, Vec3::NEG_Z), "{facing}");
}

#[test]
fn standing_on_a_pad_takes_the_participant_to_its_partner_once_and_back_once_stepped_off() {
    let storeys = storeys();
    let mut travel = Travel::default();
    let short = |head: Vec3| Input {
        head: Some(head),
        dt: 0.125,
        ..input(&storeys, Vec2::ZERO, None)
    };
    let beside = HEAD + Vec3::X * 2.0;
    // Found on it first, as on arriving in the space, the pad is disarmed until stepped off.
    for _ in 0..5 {
        assert!(!travel.update(short(HEAD)).transported);
    }
    travel.update(short(beside));
    // Stood on for 0.4 s, it takes them; walking across it does not.
    travel.update(short(HEAD));
    travel.update(short(beside));
    travel.update(short(HEAD));
    for _ in 0..3 {
        assert!(!travel.update(short(HEAD)).transported);
    }
    let outcome = travel.update(short(HEAD));
    assert!(outcome.transported && !outcome.teleported);
    assert_eq!(outcome.blink, 1.0, "it blinks as a teleport does");
    // On the racks, at the same height above the floor, facing −X, where the racks' pad faces.
    let rig = travel.rig();
    let head = rig.matrix().transform_point3(HEAD);
    assert!(close(head, Vec3::new(5.0, HEAD.y - 4.5, 1.0)), "{head}");
    assert_eq!(rig.floor(), -4.5);
    assert!(close(
        rig.matrix().transform_vector3(Vec3::NEG_Z),
        Vec3::NEG_X
    ));
    // Standing there, and swaying within 15 cm of its edge, it does not take them back.
    let racks = |offset: Vec3| short(rig.matrix().transform_point3(HEAD + offset));
    for offset in [Vec3::ZERO, Vec3::Z * 0.6, Vec3::ZERO] {
        for _ in 0..4 {
            assert!(!travel.update(racks(offset)).transported, "{offset}");
        }
    }
    // Once stepped off, back on, it does, facing as they faced, since the main floor's pad says nothing.
    travel.update(racks(Vec3::Z * 0.7));
    for _ in 0..4 {
        assert!(!travel.update(racks(Vec3::ZERO)).transported);
    }
    let facing = travel.rig().matrix().transform_vector3(Vec3::NEG_Z);
    assert!(travel.update(racks(Vec3::ZERO)).transported);
    let head = travel.rig().matrix().transform_point3(HEAD);
    assert!(close(head, Vec3::new(0.3, HEAD.y, -0.2)), "{head}");
    assert!(close(
        travel.rig().matrix().transform_vector3(Vec3::NEG_Z),
        facing
    ));
}

#[test]
fn a_pad_chosen_with_the_ray_takes_the_participant_from_anywhere_while_travel_is_active() {
    let storeys = storeys();
    let racks = crate::pads::Way::Pad(crate::pads::Key { pair: 0, end: 1 });
    let mut travel = Travel::default();
    let away = HEAD + Vec3::new(-3.0, 0.0, 2.0);
    let chose = |chosen, active: bool, head: Option<Vec3>| Input {
        chosen,
        active,
        head,
        ..input(&storeys, Vec2::ZERO, None)
    };
    // Inactive, as while the hand menu shows, without a head, or a pad not among the profile's, it goes nowhere.
    for nowhere in [
        chose(Some(racks), false, Some(away)),
        chose(Some(racks), true, None),
        chose(
            Some(crate::pads::Way::Pad(crate::pads::Key { pair: 3, end: 0 })),
            true,
            Some(away),
        ),
    ] {
        assert!(!travel.update(nowhere).transported);
    }
    assert_eq!(travel.rig(), Rig::default());
    // Pointed at the racks' pad and chosen, it takes them to the main floor's, where they stand: back where they
    // stand on the floor, a step from where they were, without stepping onto it.
    let outcome = travel.update(chose(Some(racks), true, Some(away)));
    assert!(outcome.transported);
    let head = travel.rig().matrix().transform_point3(away);
    assert!(close(head, Vec3::new(0.3, HEAD.y, -0.2)), "{head}");
    // Arrived on it, it is disarmed: standing there takes them nowhere.
    for _ in 0..4 {
        assert!(!travel.update(chose(None, true, Some(head))).transported);
    }
    // A push aimed before it lands nothing after it, until the stick returns to the middle.
    let aim = Some(ray(head, Vec3::new(2.0, 0.0, -3.0)));
    travel.update(input(&storeys, Vec2::Y, aim));
    travel.update(Input {
        chosen: Some(crate::pads::Way::Pad(crate::pads::Key { pair: 0, end: 0 })),
        ..input(&storeys, Vec2::Y, aim)
    });
    assert!(!travel.update(input(&storeys, Vec2::ZERO, aim)).teleported);
}

#[test]
fn a_pad_stood_on_while_travel_was_inactive_or_before_joining_takes_nobody_until_stepped_off_and_on()
 {
    let storeys = storeys();
    let beside = HEAD + Vec3::X * 2.0;
    let at = |head: Vec3, active: bool| Input {
        head: Some(head),
        active,
        dt: 0.125,
        ..input(&storeys, Vec2::ZERO, None)
    };
    let mut travel = Travel::default();
    travel.update(at(beside, true));
    // Stood on for part of the dwell, the hand menu opens: however long it shows, and after, the pad takes nobody.
    for _ in 0..3 {
        travel.update(at(HEAD, true));
    }
    for _ in 0..8 {
        assert!(!travel.update(at(HEAD, false)).transported);
    }
    for _ in 0..8 {
        assert!(!travel.update(at(HEAD, true)).transported);
    }
    // Stepped off and on, it does.
    travel.update(at(beside, true));
    let taken = (0..6).filter(|_| travel.update(at(HEAD, true)).transported);
    assert_eq!(taken.count(), 1);
    // Leaving and joining again standing on it: the same.
    let racks = travel.rig().matrix().transform_point3(HEAD);
    travel.update(Input {
        place: None,
        ..at(racks, true)
    });
    for _ in 0..8 {
        assert!(!travel.update(at(HEAD, true)).transported);
    }
}

#[test]
fn a_pad_s_name_is_kept_short_and_plain() {
    let name = |name: &str| Pad::at(Vec3::ZERO).named(name).name().map(str::to_owned);
    assert_eq!(name("Racks"), Some("Racks".into()));
    assert_eq!(name("Ra\ncks\u{7}\t"), Some("Racks".into()));
    assert_eq!(name(&"x".repeat(100)), Some("x".repeat(LONGEST_NAME)));
    assert_eq!(name(&"あ".repeat(100)), Some("あ".repeat(LONGEST_NAME)));
    assert_eq!(name(""), None);
    assert_eq!(name("\n\u{0}"), None);
    // Carried, a name is shared rather than copied.
    let pad = Pad::at(Vec3::ZERO).named("Racks");
    let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
    let carried = pad.carried(&crate::anchors::Anchor::carriage(&street).unwrap());
    assert!(std::ptr::eq(
        pad.name().unwrap().as_ptr(),
        carried.name().unwrap().as_ptr()
    ));
}

/// A shore at the floor's height and a rock standing in the water beyond it, flat on top, 0.4 m above the shore.
fn rock() -> Target {
    Target::object(Vec3::new(0.0, -0.6, -6.0), 0.8, 1.0)
        .named("Rock")
        .facing(Vec3::NEG_Z)
}

fn shore() -> Vec<Surface> {
    vec![Surface::rectangle(
        Vec3::new(0.0, 0.0, 0.0),
        Vec2::new(6.0, 6.0),
    )]
}

#[test]
fn targets_are_declared_where_they_stand_and_may_say_more() {
    let rock = rock();
    assert_eq!(
        (rock.foot(), rock.radius(), rock.height()),
        (Vec3::new(0.0, -0.6, -6.0), 0.8, 1.0)
    );
    // Arriving on its top's middle, facing the way it says, called what it says.
    assert!(rock.arrival().distance(Vec3::new(0.0, 0.4, -6.0)) < 1e-6);
    assert!(rock.arrives_on_top());
    assert_eq!(rock.yaw(), Some(0.0));
    assert_eq!(rock.name(), Some("Rock"));
    let plain = Target::object(Vec3::ZERO, -0.5, 1.0);
    assert_eq!(
        (plain.radius(), plain.yaw(), plain.name()),
        (0.5, None, None)
    );
    assert_eq!(plain.clone().facing(Vec3::Y).yaw(), None);
    assert_eq!(plain.clone().named("\n").name(), None);
    assert!(plain.over(Vec3::new(0.4, 9.0, 0.2)) && !plain.over(Vec3::new(0.5, 0.0, 0.2)));
    // Arriving elsewhere, on the floor beside it.
    let beside = plain.arriving(Vec3::new(1.0, 0.0, 0.0));
    assert!(!beside.arrives_on_top());
    assert_eq!(beside.arrival(), Vec3::new(1.0, 0.0, 0.0));
}

#[test]
fn only_targets_that_arrive_on_a_floor_within_bounds_are_used() {
    let used = |target: Target| {
        Locomotion::teleport(shore())
            .with_targets([target])
            .targets()
            .count()
    };
    assert_eq!(used(rock()), 1, "its own top is its floor");
    assert_eq!(
        used(Target::object(Vec3::new(4.0, 0.0, 0.0), 0.3, 1.0).arriving(Vec3::new(2.5, 0.0, 0.0))),
        1,
        "on the shore beside it"
    );
    for bad in [
        Target::object(Vec3::new(4.0, 0.0, 0.0), 0.3, 1.0).arriving(Vec3::new(4.0, 0.0, 0.0)),
        rock().arriving(Vec3::new(0.0, 0.4, -7.0)),
        rock().arriving(Vec3::new(2.0, 0.2, 2.0)),
        Target::object(Vec3::new(0.0, 0.0, -6.0), 2.1, 1.0),
        Target::object(Vec3::new(0.0, 0.0, -6.0), 0.0, 1.0),
        Target::object(Vec3::new(0.0, 0.0, -6.0), 0.5, 0.0),
        Target::object(Vec3::new(0.0, 0.0, -6.0), 0.5, TALLEST_TARGET + 0.1),
        Target::object(Vec3::new(0.0, 0.0, -6.0), f32::NAN, 1.0),
        Target::object(Vec3::new(0.0, f32::INFINITY, -6.0), 0.5, 1.0),
        Target::object(Vec3::new(0.0, 0.0, -crate::anchors::REACH - 1.0), 0.5, 1.0),
        Target::object(Vec3::new(0.0, crate::anchors::REACH - 0.5, -6.0), 0.5, 1.0),
        rock().arriving(Vec3::new(0.0, 0.0, crate::anchors::REACH + 1.0)),
        Target {
            yaw: Some(f32::NAN),
            ..rock()
        },
        rock().within(0.0),
        rock().within(f32::NAN),
    ] {
        assert_eq!(used(bad.clone()), 0, "{bad:?}");
    }
    assert_eq!(used(rock().within(3.0)), 1, "pointed at from within 3 m");
    // A target on a terrace above takes a participant down onto the rock's top, which is a floor once the rock's own
    // target arrives on it and may be used; not onto a rock that may not be used, nor beside its top or above it.
    let down =
        |arrival: Vec3| Target::object(Vec3::new(3.0, 5.0, -6.0), 0.4, 0.8).arriving(arrival);
    let both = |rock: Target, down: Target| {
        Locomotion::teleport(shore())
            .with_targets([rock, down])
            .targets()
            .count()
    };
    let top = rock().arrival();
    assert_eq!(both(rock(), down(top + Vec3::X * 0.3)), 2);
    assert_eq!(
        both(rock().arriving(Vec3::new(2.5, 0.0, 0.0)), down(top)),
        1
    );
    assert_eq!(both(rock().within(-1.0), down(top)), 0);
    assert_eq!(both(rock(), down(top + Vec3::X)), 1);
    assert_eq!(both(rock(), down(top + Vec3::Y * 0.1)), 1);
    let many = Locomotion::fly(shore(), 5.0).with_targets((0..10).map(|_| rock()));
    assert_eq!(many.targets().count(), MOST_TARGETS);
    assert_eq!(Locomotion::None.with_targets([rock()]).targets().count(), 0);
}

#[test]
fn targets_are_carried_into_the_space_with_the_profile() {
    let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
    let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
    for profile in [
        Locomotion::teleport(shore()).with_targets([rock()]),
        Locomotion::fly(shore(), 4.0).with_targets([rock()]),
    ] {
        let carried = profile.carried(&carriage);
        let target = carried.targets().next().expect("still on its own top");
        assert!(target.foot().distance(carriage.place(rock().foot())) < 1e-5);
        assert!(target.arrival().distance(carriage.place(rock().arrival())) < 1e-5);
        assert!((target.yaw().unwrap() - carriage.yaw()).abs() < 1e-5);
        assert_eq!((target.radius(), target.height()), (0.8, 1.0));
    }
}

#[test]
fn a_target_chosen_with_the_ray_takes_the_participant_onto_it_facing_as_it_says() {
    let profile = Locomotion::teleport(shore()).with_targets([
        rock(),
        Target::object(Vec3::new(2.8, 0.0, 0.0), 0.2, 1.0).arriving(Vec3::new(2.0, 0.0, 0.0)),
    ]);
    let mut travel = Travel::default();
    let away = HEAD + Vec3::new(1.0, 0.0, 1.0);
    let chose = |chosen| Input {
        chosen,
        head: Some(away),
        facing: Some(Vec3::X),
        ..input(&profile, Vec2::ZERO, None)
    };
    // A target not among the profile's takes them nowhere.
    assert!(
        !travel
            .update(chose(Some(crate::pads::Way::Target(2))))
            .transported
    );
    let outcome = travel.update(chose(Some(crate::pads::Way::Target(0))));
    assert!(outcome.transported && outcome.blink == 1.0);
    let rig = travel.rig();
    assert!((rig.floor() - 0.4).abs() < 1e-5);
    let head = rig.matrix().transform_point3(away);
    assert!(close(head, Vec3::new(0.0, away.y + 0.4, -6.0)), "{head}");
    assert!(close(rig.matrix().transform_vector3(Vec3::X), Vec3::NEG_Z));
    // On its top, nothing is a surface to step back along, so a push back steps nowhere.
    let back = |stick| Input {
        head: Some(head),
        facing: Some(Vec3::NEG_Z),
        ..input(&profile, stick, None)
    };
    travel.update(back(Vec2::new(0.0, -1.0)));
    assert!(!travel.update(back(Vec2::ZERO)).stepped);
    // The other takes them back to the shore beside it, facing as they faced.
    let outcome = travel.update(Input {
        chosen: Some(crate::pads::Way::Target(1)),
        head: Some(head),
        facing: Some(Vec3::NEG_Z),
        ..input(&profile, Vec2::ZERO, None)
    });
    assert!(outcome.transported);
    let rig = travel.rig();
    assert_eq!(rig.floor(), 0.0);
    assert!(close(
        rig.matrix().transform_point3(away),
        Vec3::new(2.0, away.y, 0.0)
    ));
    assert!(close(rig.matrix().transform_vector3(Vec3::X), Vec3::NEG_Z));
}
