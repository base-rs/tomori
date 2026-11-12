// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A frame costs no allocation: once what is placed stops growing, giving the world its views, placing what moves and
//! culling allocate nothing, so that the indirection costs the headset's frame no time in the allocator.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use glam::{Mat4, Quat, Vec3};
use tomori_world::{Aabb, Item, Layer, Light, Mobility, World};

/// The system's allocator, counting what it allocates.
struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(1, Ordering::Relaxed);
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
    let cubes = world.add(Item::new(space, Layer::Opaque, Mobility::Dynamic));
    let lamp = world.add_light(Light::new(
        scenery,
        Vec3::new(0.0, 2.0, -3.0),
        4.0,
        Vec3::ONE,
    ));
    let frame = |world: &mut World, step: usize| {
        let turned = step as f32 * 0.05;
        world.set_views(&eyes(Vec3::new(0.0, 1.6, 0.0), turned));
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
    };
    // The first frames grow what the world keeps, and build the hierarchy.
    for step in 0..3 {
        frame(&mut world, step);
    }
    let before = ALLOCATED.load(Ordering::Relaxed);
    let mut seen = 0;
    for step in 3..200 {
        seen += frame(&mut world, step);
    }
    assert_eq!(ALLOCATED.load(Ordering::Relaxed) - before, 0);
    assert!(seen > 0);
}
