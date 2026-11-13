// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Levels of detail chosen by how many pixels each level's error spans, with hysteresis, alike for both eyes, and faded
//! from one to the next.
use glam::Quat;

use super::*;
use crate::view::tests::{both, looking};
use crate::{Item, Layer, Mobility, World};

/// A 90° view's density, 1,000 pixels tall: 500 pixels a unit at a unit away.
const DENSITY: f32 = 500.0;

#[test]
fn levels_keep_errors_that_grow_and_leave_out_the_rest() {
    let levels = Levels::new(&[0.01, 0.03, 0.1]);
    assert_eq!(levels.errors(), &[0.01, 0.03, 0.1]);
    assert_eq!((levels.len(), levels.is_empty()), (4, false));
    // At most three coarser; none after one that does not grow, is not positive or is not a number.
    assert_eq!(
        Levels::new(&[0.01, 0.02, 0.03, 0.04]).errors(),
        &[0.01, 0.02, 0.03]
    );
    assert_eq!(Levels::new(&[0.02, 0.01, 0.5]).errors(), &[0.02]);
    assert_eq!(Levels::new(&[0.0, 0.1]).errors(), &[] as &[f32]);
    assert_eq!(Levels::new(&[0.1, f32::NAN]).errors(), &[0.1]);
    assert_eq!(Levels::new(&[0.1, f32::INFINITY]).errors(), &[0.1]);
    assert!(Levels::default().is_empty() && Levels::default().len() == 1);
}

#[test]
fn the_coarsest_level_whose_error_spans_at_most_a_pixel_is_chosen_and_kept_a_quarter_further() {
    let levels = Levels::new(&[0.01, 0.03, 0.1]);
    // A level's error spans a pixel where `error × density / distance` is one: 5 m, 15 m and 50 m.
    for (distance, level) in [
        (0.0, 0),
        (1.0, 0),
        (4.99, 0),
        (5.0, 1),
        (14.9, 1),
        (15.0, 2),
        (49.9, 2),
        (50.0, 3),
        (2_000.0, 3),
        (f32::INFINITY, 3),
    ] {
        assert_eq!(
            levels.choose(distance, DENSITY, 0),
            level,
            "from its own at {distance} m"
        );
    }
    // Drawn coarser, it is drawn so until its error spans a quarter more, at 4 m for the first level, then at once the
    // finer that its distance asks.
    assert_eq!(levels.choose(4.01, DENSITY, 1), 1);
    assert_eq!(levels.choose(3.99, DENSITY, 1), 0);
    assert_eq!(levels.choose(12.01, DENSITY, 2), 2);
    assert_eq!(levels.choose(11.9, DENSITY, 2), 1);
    assert_eq!(levels.choose(40.1, DENSITY, 3), 3);
    assert_eq!(levels.choose(39.9, DENSITY, 3), 2);
    assert_eq!(levels.choose(1.0, DENSITY, 3), 0);
    // Coarser it goes as soon as it may.
    assert_eq!(levels.choose(60.0, DENSITY, 1), 3);
    // A level past the last is taken as the last; without levels, every instance is its own.
    assert_eq!(levels.choose(45.0, DENSITY, 9), 3);
    assert_eq!(Levels::default().choose(1e6, DENSITY, 0), 0);
    // More pixels, as on a larger image, spread the same errors further.
    assert_eq!(levels.choose(9.0, DENSITY * 2.0, 0), 0);
    assert_eq!(levels.choose(10.0, DENSITY * 2.0, 0), 1);
}

#[test]
fn a_change_of_level_fades_over_its_frames_and_none_other_begins_until_it_is_done() {
    let (mut lod, mut progress) = (Lod::default(), Progress::default());
    // First seen, it is drawn at once in what is chosen.
    step(&mut lod, &mut progress, 2);
    assert_eq!((lod, progress), (Lod::settled(2), Progress::Settled));
    step(&mut lod, &mut progress, 2);
    assert_eq!(lod, Lod::settled(2));
    // Then a change fades in over its frames, drawn in both meanwhile.
    let mut fades = Vec::new();
    step(&mut lod, &mut progress, 1);
    while lod.fading() {
        assert_eq!((lod.level, lod.from), (1, 2));
        fades.push(lod.fade);
        // Another level chosen meanwhile waits.
        step(&mut lod, &mut progress, 3);
    }
    assert_eq!(fades.len(), usize::from(FADE_FRAMES) - 1);
    assert!(fades.windows(2).all(|pair| pair[1] > pair[0]));
    assert!((fades[0] - 1.0 / f32::from(FADE_FRAMES)).abs() < 1e-6);
    assert_eq!((lod, progress), (Lod::settled(1), Progress::Settled));
    step(&mut lod, &mut progress, 3);
    assert_eq!((lod.level, lod.from), (3, 1));
}

#[test]
fn density_is_pixels_a_unit_spans_a_unit_away_whichever_way_the_eye_looks() {
    // A 90° view is two units tall a unit away.
    for heading in [0.0, 1.0, -2.5] {
        let clip = looking(Vec3::new(3.0, 1.6, -2.0), heading);
        assert!((density(clip, 1_000) - DENSITY).abs() < 0.01);
        // Moved, turned or scaled with its coordinates, as a root's are, it is the same, as a unit a unit away spans as
        // much whatever a unit is.
        let carried = clip * Mat4::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::X);
        assert!((density(carried, 1_000) - DENSITY).abs() < 0.01);
        let scaled = clip * Mat4::from_scale(Vec3::splat(2.0));
        assert!((density(scaled, 1_000) - DENSITY).abs() < 0.01);
    }
    // A view with no depth, or none that is a number, has none.
    assert_eq!(density(Mat4::IDENTITY, 1_000), 0.0);
    assert_eq!(density(Mat4::from_cols_array(&[f32::NAN; 16]), 1_000), 0.0);
    let mut endless = [0.0; 16];
    (endless[1], endless[3]) = (f32::INFINITY, 1.0);
    assert_eq!(density(Mat4::from_cols_array(&endless), 1_000), 0.0);
}

#[test]
fn eyes_measure_from_the_nearer_to_where_bounds_are_and_nothing_without_pixels() {
    let clips = both(Vec3::new(0.0, 1.6, 0.0), 0.0);
    let eyes = Eyes::new(&clips, Some(1_000));
    let off = Aabb::new(Vec3::new(1.0, 1.5, -10.1), Vec3::new(1.2, 1.7, -9.9));
    let (distance, density) = eyes.measure(Some(&off)).unwrap();
    let nearer = Vec3::new(0.032, 1.6, 0.0).distance(Vec3::new(1.0, 1.6, -9.9));
    assert!((distance - nearer).abs() < 1e-3, "{distance} {nearer}");
    assert!((density - DENSITY).abs() < 0.01);
    // Inside, it is no distance away.
    let about = Aabb::new(Vec3::splat(-5.0), Vec3::splat(5.0));
    assert_eq!(eyes.measure(Some(&about)).unwrap().0, 0.0);
    // Nothing to measure: no bounds, or none that are numbers, or empty; no eyes; no pixels.
    assert!(eyes.measure(None).is_none());
    assert!(eyes.measure(Some(&Aabb::EMPTY)).is_none());
    let unbounded = Aabb::new(Vec3::splat(f32::NEG_INFINITY), Vec3::ZERO);
    assert!(eyes.measure(Some(&unbounded)).is_none());
    assert!(Eyes::new(&clips, None).measure(Some(&off)).is_none());
    assert!(Eyes::new(&[], Some(1_000)).measure(Some(&off)).is_none());
    assert!(
        Eyes::new(&[Mat4::IDENTITY], Some(1_000))
            .measure(Some(&off))
            .is_none()
    );
}

/// A box 20 cm across, `distance` ahead of eyes at the origin looking along −Z, `aside` to their right.
fn ahead(distance: f32, aside: f32) -> Option<Aabb> {
    let middle = Vec3::new(aside, 1.6, -distance);
    Some(Aabb::new(middle - 0.1, middle + 0.1))
}

#[test]
fn each_instance_seen_is_drawn_in_one_level_for_both_eyes_faded_as_it_changes() {
    let mut world = World::new();
    let root = world.add_root();
    let item = world.add(Item::new(root, Layer::Opaque, Mobility::Static));
    let plain = world.add(Item::new(root, Layer::Opaque, Mobility::Static));
    world.set_levels(item, Levels::new(&[0.01, 0.03]));
    assert_eq!(world.levels(item).errors(), &[0.01, 0.03]);
    let placed = [ahead(2.0, 0.0), ahead(10.0, 0.0), ahead(30.0, 0.0), None];
    world.place(item, placed);
    world.place(plain, [ahead(30.0, 0.0)]);
    world.set_views(&both(Vec3::new(0.0, 1.6, 0.0), 0.0));
    // Without pixels to measure by, everything is its own.
    world.cull();
    assert_eq!(world.lods(item), &[Lod::settled(0); 4]);
    assert_eq!(world.height(), None);
    world.set_height(Some(1_000));
    assert_eq!(world.height(), Some(1_000));
    world.cull();
    // The first is near, the second's first level's error spans half a pixel, the third's second's, and the last may be
    // anywhere, so is its own; without levels, its own.
    let levels = |world: &World| {
        world
            .lods(item)
            .iter()
            .map(|lod| lod.level)
            .collect::<Vec<_>>()
    };
    assert_eq!(levels(&world), [0, 1, 2, 0]);
    assert!(world.lods(item).iter().all(|lod| !lod.fading()));
    assert_eq!(world.lods(plain), &[Lod::settled(0)]);
    // Walking towards the third, it fades to the first level once its second's error spans 1.25 pixels, at 12 m, over
    // its frames, and is then the first's.
    let walk = |world: &mut World, z: f32| {
        world.set_views(&both(Vec3::new(0.0, 1.6, z), 0.0));
        world.cull();
        world.lods(item)[2]
    };
    assert_eq!(walk(&mut world, -17.0).level, 2);
    let lod = walk(&mut world, -18.5);
    assert_eq!((lod.level, lod.from), (1, 2));
    for _ in 2..FADE_FRAMES {
        assert!(walk(&mut world, -18.5).fading());
    }
    assert_eq!(walk(&mut world, -18.5), Lod::settled(1));
    // Turned away, it is not seen, and seen again, far, it is drawn at once in what its distance asks.
    world.set_views(&both(Vec3::new(0.0, 1.6, -18.5), std::f32::consts::PI));
    world.cull();
    assert!(!world.seen(item)[2]);
    assert_eq!(walk(&mut world, 0.0), Lod::settled(2));
    // Placed again as many, each keeps its level; one more is chosen anew; levels set again choose anew.
    world.place(item, placed.into_iter().chain([ahead(60.0, 0.0)]));
    assert_eq!(world.lods(item)[2], Lod::settled(2));
    assert_eq!(walk(&mut world, 0.0), Lod::settled(2));
    assert_eq!(levels(&world)[4], 2);
    world.set_levels(item, Levels::new(&[0.05]));
    assert_eq!(world.lods(item), &[Lod::settled(0); 5]);
    world.cull();
    assert_eq!(levels(&world), [0, 0, 1, 0, 1]);
    // An item not here has none.
    assert!(world.remove(plain));
    assert!(world.lods(plain).is_empty() && world.levels(plain).is_empty());
    world.set_height(Some(0));
    world.cull();
    assert_eq!(levels(&world), [0; 5]);
}

#[test]
fn both_eyes_draw_an_instance_in_the_level_the_nearer_asks() {
    let mut world = World::new();
    let root = world.add_root();
    let item = world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic));
    world.set_levels(item, Levels::new(&[0.01]));
    world.set_height(Some(1_000));
    // To the right, 5 m away: from the left eye alone, 45 mm further, its error spans less than a pixel, and from the
    // right more.
    let eyes = both(Vec3::new(0.0, 1.6, 0.0), 0.0);
    let right = Vec3::new(0.032, 1.6, 0.0);
    let left = Vec3::new(-0.032, 1.6, 0.0);
    let at = Vec3::new(3.5, 1.6, -3.6);
    let bounds = Aabb::new(at - 0.001, at + 0.001);
    let spans = |eye: Vec3| 0.01 * DENSITY / eye.distance(at);
    assert!(spans(left) <= TOLERANCE && spans(right) > TOLERANCE);
    world.place(item, [Some(bounds)]);
    world.set_views(&eyes);
    world.cull();
    assert!(world.seen(item)[0]);
    assert_eq!(world.lods(item), &[Lod::settled(0)]);
    // Each eye alone would choose otherwise.
    for (eye, level) in [(eyes[1], 0), (eyes[0], 1)] {
        world.set_levels(item, Levels::new(&[0.01]));
        world.set_views(&[eye]);
        world.cull();
        assert_eq!(world.lods(item)[0].level, level);
    }
    // An inactive root's instances keep their levels.
    world.set_active(root, false);
    world.set_views(&eyes);
    world.cull();
    assert_eq!(world.lods(item)[0].level, 1);
}
