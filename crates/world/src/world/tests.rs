// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What is submitted to a world, culled against both eyes, moved, carried in other coordinates and lit.
use glam::Quat;

use super::*;
use crate::Frustum;
use crate::bvh::tests::scattered;
use crate::light::Falloff;
use crate::view::tests::both;

fn around(middle: Vec3, half: f32) -> Aabb {
    Aabb::new(middle - half, middle + half)
}

/// The eyes at the arrival, 1.6 m up at the origin, looking along −Z.
fn arrival() -> [Mat4; 2] {
    both(Vec3::new(0.0, 1.6, 0.0), 0.0)
}

#[test]
fn items_are_added_placed_and_removed_by_their_handles() {
    let mut world = World::default();
    let root = world.add_root();
    let item = Item::new(root, Layer::Translucent, Mobility::Dynamic)
        .with_mesh(MeshId(3))
        .with_material(MaterialId(7));
    let id = world.add(item);
    assert!(world.contains(id));
    assert_eq!(world.item(id), Some(&item));
    assert_eq!(
        (item.mesh, item.material, item.culled),
        (Some(MeshId(3)), Some(MaterialId(7)), true)
    );
    assert!(!item.unculled().culled);
    // Nothing is placed until it is, and what is placed is seen until it is culled.
    assert_eq!(world.instances(id), 0);
    assert!(!world.visible(id));
    world.place(id, [Some(around(Vec3::ZERO, 1.0)), None]);
    assert_eq!(world.instances(id), 2);
    assert_eq!(world.seen(id), [true, true]);
    // Removed, it is gone, and what takes its slot is not mistaken for it.
    assert!(world.remove(id));
    assert!(!world.remove(id));
    assert!(!world.contains(id) && world.item(id).is_none());
    assert_eq!(world.instances(id), 0);
    assert!(world.seen(id).is_empty());
    let next = world.add(item);
    assert_ne!(next, id);
    assert!(world.contains(next) && !world.contains(id));
    // The layers run in the order a frame draws them.
    assert!(Layer::Opaque < Layer::CutOut && Layer::CutOut < Layer::Sky);
    assert!(Layer::Sky < Layer::Translucent && Layer::Translucent < Layer::Overlay);
    assert_eq!(
        (Layer::default(), Mobility::default()),
        (Layer::Opaque, Mobility::Dynamic)
    );
}

#[test]
fn what_neither_eye_sees_is_culled_whether_it_is_static_or_dynamic() {
    let mut world = World::new();
    let root = world.add_root();
    let eyes = arrival();
    world.set_views(&eyes);
    assert_eq!(world.views().clips(), &eyes);
    let boxes = scattered(400, 9);
    let scenery = world.add(Item::new(root, Layer::Opaque, Mobility::Static));
    world.place(scenery, boxes.iter().copied().map(Some));
    let moving = world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic));
    world.place(moving, boxes.iter().copied().map(Some));
    world.cull();
    assert_eq!(world.static_instances(root), boxes.len());
    // Each, static through the hierarchy or dynamic one by one, is seen exactly when the eyes' frustum sees its box.
    let frustum = Frustum::enclosing(&eyes);
    let brute: Vec<bool> = boxes.iter().map(|bounds| frustum.sees(bounds)).collect();
    assert!(brute.iter().any(|seen| *seen) && brute.iter().any(|seen| !seen));
    assert_eq!(world.seen(scenery), brute);
    assert_eq!(world.seen(moving), brute);
    // Nothing either eye sees is culled.
    for (bounds, seen) in boxes.iter().zip(&brute) {
        if eyes.iter().any(|clip| Frustum::new(*clip).sees(bounds)) {
            assert!(seen, "{bounds:?}");
        }
    }
    let seen = brute.iter().filter(|seen| **seen).count();
    assert_eq!(
        world.tally(root),
        Tally {
            drawn: seen * 2,
            culled: (boxes.len() - seen) * 2,
            ..Tally::default()
        }
    );
    // An instance that may be anywhere, or is not all numbers, is always seen; an empty one never.
    let behind = around(Vec3::new(0.0, 1.6, 5.0), 0.5);
    for mobility in [Mobility::Static, Mobility::Dynamic] {
        let odd = world.add(Item::new(root, Layer::Opaque, mobility));
        world.place(
            odd,
            [
                None,
                Some(Aabb::new(Vec3::NAN, Vec3::ONE)),
                Some(Aabb::EMPTY),
                Some(behind),
            ],
        );
        world.cull();
        assert_eq!(world.seen(odd), [true, true, false, false], "{mobility:?}");
        // Without an instance that may be anywhere, one not all numbers is still seen beside one behind the eyes.
        world.place(
            odd,
            [
                Some(behind),
                Some(Aabb::new(Vec3::ONE, Vec3::splat(f32::NAN))),
            ],
        );
        world.cull();
        assert_eq!(world.seen(odd), [false, true], "{mobility:?}");
    }
    // A dynamic item whose instances are all behind the eyes is culled whole, and one that opts out never is.
    let away = world.add(Item::new(root, Layer::Opaque, Mobility::Dynamic));
    world.place(
        away,
        [Some(behind), Some(around(Vec3::new(1.0, 1.6, 6.0), 0.5))],
    );
    let kept = world.add(Item::new(root, Layer::Overlay, Mobility::Static).unculled());
    world.place(kept, [Some(behind)]);
    world.cull();
    assert_eq!(world.seen(away), [false, false]);
    assert!(!world.visible(away));
    assert_eq!(world.seen(kept), [true]);
    assert!(world.visible(kept));
    // Turned off to compare, culling sees everything.
    world.set_culling(false);
    world.cull();
    assert!(world.seen(scenery).iter().all(|seen| *seen));
    assert!(world.seen(away).iter().all(|seen| *seen));
    assert!(world.root_views(root).frustum().is_none());
    assert_eq!(world.root_views(root).clips(), &eyes);
    world.set_culling(true);
    world.cull();
    assert_eq!(world.seen(moving), brute);
}

#[test]
fn what_moves_is_seen_where_it_now_is() {
    let mut world = World::new();
    let root = world.add_root();
    world.set_views(&arrival());
    let ahead = around(Vec3::new(0.0, 1.5, -4.0), 0.3);
    let behind = around(Vec3::new(0.0, 1.5, 4.0), 0.3);
    for mobility in [Mobility::Dynamic, Mobility::Static] {
        let item = world.add(Item::new(root, Layer::Opaque, mobility));
        world.place(item, [Some(ahead), Some(behind)]);
        world.cull();
        assert_eq!(world.seen(item), [true, false], "{mobility:?}");
        // Moved, it is seen where it now is, a static one as its hierarchy is built again.
        world.place(item, [Some(behind), Some(ahead), Some(ahead)]);
        world.cull();
        assert_eq!(world.seen(item), [false, true, true], "{mobility:?}");
        // Removed, a static one leaves its hierarchy.
        let held = world.static_instances(root);
        world.remove(item);
        world.cull();
        assert_eq!(world.static_instances(root), 0);
        assert!(held == 0 || mobility == Mobility::Static);
    }
    // As the eyes turn about, what was behind them is seen and what was ahead is not.
    let item = world.add(Item::new(root, Layer::Opaque, Mobility::Static));
    world.place(item, [Some(ahead), Some(behind)]);
    world.set_views(&both(Vec3::new(0.0, 1.6, 0.0), std::f32::consts::PI));
    world.cull();
    assert_eq!(world.seen(item), [false, true]);
    // A desktop window's one view culls the same way.
    world.set_views(&arrival()[..1]);
    world.cull();
    assert_eq!(world.seen(item), [true, false]);
}

#[test]
fn each_root_is_culled_in_its_own_coordinates_while_it_is_active() {
    let mut world = World::new();
    let space = world.add_root();
    let scenery = world.add_root();
    let other = world.add_root();
    world.set_views(&arrival());
    // The scenery's coordinates are turned about and carried 2 m along −Z, so that its +Z is the space's −Z.
    let carriage = Mat4::from_rotation_translation(
        Quat::from_rotation_y(std::f32::consts::PI),
        Vec3::new(0.0, 0.0, -2.0),
    );
    world.set_transform(scenery, carriage);
    let at = around(Vec3::new(0.0, 1.5, 4.0), 0.3);
    let items = [space, scenery, other].map(|root| {
        let item = world.add(Item::new(root, Layer::Opaque, Mobility::Static));
        world.place(item, [Some(at)]);
        item
    });
    world.set_active(other, false);
    world.cull();
    // In the space it is behind the eyes; carried, it stands 6 m ahead of them; an inactive root is left as it was.
    assert_eq!(world.seen(items[0]), [false]);
    assert_eq!(world.seen(items[1]), [true]);
    assert_eq!(world.seen(items[2]), [true]);
    assert_eq!(world.static_instances(other), 0);
    for (clip, carried) in arrival().iter().zip(world.root_views(scenery).clips()) {
        assert_eq!(*carried, *clip * carriage);
    }
    // Active again, it is culled, its hierarchy built.
    world.set_active(other, true);
    world.cull();
    assert_eq!(world.seen(items[2]), [false]);
    assert_eq!(world.static_instances(other), 1);
    // Cleared, as when what drew them is dropped, its items and lights are gone, and the others' kept.
    let lamp = world.add_light(Light::new(other, Vec3::ZERO, 2.0, Vec3::ONE));
    let kept = world.add_light(Light::new(space, Vec3::ZERO, 2.0, Vec3::ONE));
    world.set_environment(other, Some(EnvironmentLight::default()));
    world.clear(other);
    assert!(!world.contains(items[2]) && world.light_mut(lamp).is_none());
    assert!(world.light_mut(kept).is_some());
    assert!(world.environment(other).is_none());
    assert!(world.contains(items[0]) && world.contains(items[1]));
    world.cull();
    assert_eq!(world.static_instances(other), 0);
    assert_eq!(world.tally(other), Tally::default());
}

#[test]
fn lights_are_kept_with_their_roots_and_culled_by_their_reach() {
    let mut world = World::new();
    let root = world.add_root();
    let resting = world.add_root();
    world.set_views(&arrival());
    let mut lantern = Light::new(
        root,
        Vec3::new(0.0, 2.0, -5.0),
        3.0,
        Vec3::new(1.0, 0.8, 0.5),
    );
    lantern.falloff = Falloff::Physical;
    lantern.mobility = Mobility::Static;
    let ahead = world.add_light(lantern);
    // One behind the eyes whose reach comes into their view, and one that reaches nowhere near it.
    let reaching = world.add_light(Light::new(root, Vec3::new(0.0, 1.6, 2.0), 4.0, Vec3::ONE));
    let far = world.add_light(Light::new(root, Vec3::new(0.0, 1.6, 30.0), 2.0, Vec3::ONE));
    let elsewhere = world.add_light(Light::new(
        resting,
        Vec3::new(0.0, 1.6, 30.0),
        2.0,
        Vec3::ONE,
    ));
    world.set_active(resting, false);
    world.cull();
    let seen: Vec<bool> = world.lights().map(|(_, seen)| seen).collect();
    assert_eq!(seen, [true, true, false, true]);
    assert_eq!(world.lights().next().unwrap().0, &lantern);
    assert_eq!(Falloff::default(), Falloff::Stylised);
    // A light changed, as a flame flickers, is culled where it now is; one removed is gone.
    world.light_mut(far).unwrap().position = Vec3::new(0.0, 1.6, -30.0);
    assert!(world.remove_light(reaching));
    assert!(!world.remove_light(reaching));
    world.cull();
    let seen: Vec<bool> = world.lights().map(|(_, seen)| seen).collect();
    assert_eq!(seen, [true, true, true]);
    assert!(world.light_mut(ahead).is_some() && world.light_mut(elsewhere).is_some());
    // An environment's own light.
    let evening = EnvironmentLight {
        sun_direction: Vec3::Y,
        haze_density: 0.002,
        ..EnvironmentLight::default()
    };
    assert!(world.environment(root).is_none());
    world.set_environment(root, Some(evening));
    assert_eq!(world.environment(root), Some(&evening));
}
