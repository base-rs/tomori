// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A frame costs no allocation: once what is placed stops growing, giving the world its views, placing what moves,
//! culling, lighting each instance and choosing its level of detail allocate nothing, so that the indirection costs the headset's frame no time in
//! the allocator. Counted on the test's thread only, as the world uses no other, so that what the test harness
//! allocates on its own threads meanwhile is not.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use glam::{Mat4, Quat, Vec3};
use tomori_world::{Aabb, Item, Layer, Levels, Light, Mobility, World};

/// The system's allocator, counting what each thread allocates.
struct Counting;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // Without a thread's locals, as it ends, nothing is counted.
    let _ = ALLOCATED.try_with(|allocated| allocated.set(allocated.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// Both eyes at `middle`, turned `heading`.
fn eyes(middle: Vec3, heading: f32) -> [Mat4; 2] {
    let facing = Quat::from_rotation_y(heading);
    let projection = glam::camera::rh::proj::directx::perspective(
        std::f32::consts::FRAC_PI_2,
        1.0,
        0.05,
        2_000.0,
    );
    [-0.032, 0.032].map(|across| {
        projection
            * Mat4::from_rotation_translation(facing, middle + facing * Vec3::X * across).inverse()
    })
}

#[test]
fn a_frame_allocates_nothing_once_what_is_placed_stops_growing() {
    let mut world = World::new();
    let space = world.add_root();
    let scenery = world.add_root();
    // The scenery's static models, 600 instances, and its hens, which move.
    let static_item = world.add(Item::new(scenery, Layer::Opaque, Mobility::Static));
    world.place(
        static_item,
        (0..600).map(|index| {
            let at = Vec3::new(
                (index % 30) as f32 * 2.0 - 30.0,
                0.0,
                (index / 30) as f32 * 2.0 - 20.0,
            );
            Some(Aabb::new(at - 0.4, at + 0.4))
        }),
    );
    let hens = world.add(Item::new(scenery, Layer::Opaque, Mobility::Dynamic));
    // Each drawn coarser far away, as the eyes walk to and fro, so that levels change and fade.
    world.set_levels(static_item, Levels::new(&[0.01, 0.03, 0.1]));
    world.set_levels(hens, Levels::new(&[0.005, 0.02]));
    world.set_height(Some(1_760));
    let cubes = world.add(Item::new(space, Layer::Opaque, Mobility::Dynamic));
    let lamp = world.add_light(Light::new(
        scenery,
        Vec3::new(0.0, 2.0, -3.0),
        4.0,
        Vec3::ONE,
    ));
    // And a dozen lanterns about the hens, more than light any one of them.
    for index in 0..12 {
        world.add_light(Light::new(
            scenery,
            Vec3::new(index as f32 * 0.5 - 3.0, 1.0, -2.5),
            3.0,
            Vec3::splat(0.5),
        ));
    }
    let frame = |world: &mut World, step: usize| {
        let turned = step as f32 * 0.05;
        world.set_views(&eyes(
            Vec3::new(0.0, 1.6, (step as f32 * 0.1).sin() * 15.0),
            turned,
        ));
        world.set_transform(scenery, Mat4::from_translation(Vec3::new(0.0, 0.0, turned)));
        world.place(
            hens,
            (0..5).map(|hen| {
                let at = Vec3::new(hen as f32 + turned.sin(), 0.2, -2.0);
                Some(Aabb::new(at - 0.2, at + 0.2))
            }),
        );
        world.place(
            cubes,
            (0..200).map(|cube| {
                let at = Vec3::new((cube as f32 * 0.37 + turned).sin() * 2.0, 1.5, -1.0);
                Some(Aabb::new(at - 0.01, at + 0.01))
            }),
        );
        world.light_mut(lamp).unwrap().intensity = 1.0 + turned.sin() * 0.1;
        world.cull();
        world.seen(static_item).iter().filter(|seen| **seen).count()
            + world.lit(static_item).map(<[_]>::len).sum::<usize>()
            + world
                .lods(static_item)
                .iter()
                .filter(|lod| lod.fading())
                .count()
    };
    // The first frames grow what the world keeps, and build the hierarchy.
    for step in 0..3 {
        frame(&mut world, step);
    }
    // Facing them, the hens are lit by as many lanterns as light one.
    frame(&mut world, 0);
    assert_eq!(
        world.lit(hens).map(<[_]>::len).max(),
        Some(tomori_world::LIGHTS_PER_INSTANCE)
    );
    let before = ALLOCATED.with(Cell::get);
    let mut seen = 0;
    for step in 3..200 {
        seen += frame(&mut world, step);
    }
    assert_eq!(ALLOCATED.with(Cell::get) - before, 0);
    assert!(seen > 0);
}
