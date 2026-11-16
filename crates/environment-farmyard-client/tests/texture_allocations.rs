// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Laying the farmyard's textures costs a frame no allocation: once they have arrived and are laid, a frame of the
//! farmyard submitting to its render world, the world culling it and the farmyard drawing what is seen, its surfaces
//! laid, allocates nothing, in High as in Low. Counted on the test's thread only, as the farmyard culls on no other, so
//! that what the test harness or the GPU's driver allocates on its own threads meanwhile is not. See
//! `design/rendering.md`.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use glam::{Mat4, Quat, Vec3};
use tomori_assets::Library;
use tomori_environment_farmyard_client::coop::Coop;
use tomori_environment_farmyard_client::layout;
use tomori_environment_farmyard_client::scenery::{Clock, Farmyard};
use tomori_render::scenery::{Stage, Target};
use tomori_render::test_gpu;
use tomori_render::world::Tier;

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

/// Both eyes at the arrival, looking over the yard's earth.
fn eyes() -> [Mat4; 2] {
    let facing = Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.3);
    let middle = layout::ARRIVAL + Vec3::Y * 1.6;
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
fn a_frame_with_its_textures_laid_allocates_nothing() {
    test_gpu::with(|gpu| {
        let test_gpu::Gpu { device, queue, .. } = &gpu;
        let served: &'static Library = Box::leak(Box::default());
        served.add(tomori_environment_farmyard::textures::Generated);
        let target = Target::new(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            64,
            64,
            1,
            tomori_render::glyphs::SharedGlyphs::new(device),
        );
        let mut farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(0.0))
                .with_textures_from(served);
        let mut stage = Stage::new();
        let clips = eyes();
        for tier in [Tier::High, Tier::Low] {
            stage.set_tier(tier);
            // Its textures arriving, one a frame, and laid.
            for _ in 0..20 {
                stage.cull(&mut farmyard, queue, &clips, true);
            }
            let before = ALLOCATED.with(Cell::get);
            for _ in 0..100 {
                stage.cull(&mut farmyard, queue, &clips, true);
            }
            let allocated = ALLOCATED.with(Cell::get) - before;
            assert_eq!(
                allocated, 0,
                "{tier:?}: {allocated} allocations in 100 frames"
            );
        }
    });
}
