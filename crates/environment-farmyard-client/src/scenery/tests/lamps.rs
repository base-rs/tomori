// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard lit by the few lamps the render world lists for each of its models' pieces and ferns looks as lit by
//! every lamp, but for a colour rounded apart here and there, from the census's viewpoints through a Quest 3's eyes and
//! by the campfire at its biggest, while most of what is drawn adds up fewer lamps. Set `TOMORI_FARMYARD_CENSUS` to
//! anything to print how many.
use std::f32::consts::PI;

use super::culling::{quest_eyes, viewpoints, with_hens};
use super::*;
use tomori_render::scenery::Stage;

/// How many lamps the pieces and ferns the eyes may see are lit by, of how many seen, with every lamp each.
fn listed(farmyard: &Farmyard, stage: &Stage) -> (usize, usize) {
    let (world, _) = stage.world();
    let backdrop = &farmyard.backdrop;
    let drawn = [
        &farmyard.yard,
        &farmyard.coop,
        &farmyard.campsite,
        &farmyard.beach,
    ]
    .into_iter()
    .chain([&backdrop.trail, &backdrop.terrace, &backdrop.foothills])
    .chain(&backdrop.plants)
    .chain([&backdrop.stones, &backdrop.rock])
    .filter_map(|drawn| drawn.item);
    drawn
        .chain(farmyard.ferns.item)
        .flat_map(|item| world.lit(item).zip(world.seen(item)))
        .filter(|(_, seen)| **seen)
        .fold((0, 0), |(lamps, seen), (lit, _)| {
            (lamps + lit.len(), seen + 1)
        })
}

#[test]
fn the_farmyard_lit_by_each_piece_s_own_lamps_looks_as_lit_by_every_lamp() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (448, 424);
        let mut farmyard = with_hens(&device, &queue, size.0, size.1);
        let mut stage = Stage::new();
        let (mut lamps, mut seen) = (0, 0);
        let mut compare = |farmyard: &mut Farmyard, name: &str, clips: [Mat4; 2]| {
            let pictures = |farmyard: &Farmyard| {
                clips.map(|clip| capture_clip(&device, &queue, farmyard, 4, clip, size))
            };
            farmyard.set_lamps_listed(false);
            stage.cull(farmyard, &queue, &clips, true);
            let every = pictures(farmyard);
            farmyard.set_lamps_listed(true);
            stage.cull(farmyard, &queue, &clips, true);
            let listed = pictures(farmyard);
            // Of the four lamps, those whose reach the eyes may see.
            let (world, root) = stage.world();
            let tabled = world.root_lights(root).len();
            assert!((1..=4).contains(&tabled), "{name}: {tabled}");
            for eye in 0..2 {
                assert!(
                    listed[eye]
                        .chunks(4)
                        .any(|pixel| pixel[..3] != listed[eye][..3]),
                    "{name}: eye {eye} sees nothing"
                );
                // The same pixels, but where the two loops round a colour apart: at most one pixel in 10,000, by a
                // step or so of 255, as on lavapipe.
                let changed: Vec<u8> = every[eye]
                    .chunks(4)
                    .zip(listed[eye].chunks(4))
                    .filter(|(a, b)| a != b)
                    .map(|(a, b)| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap())
                    .collect();
                assert!(
                    changed.len() * 10_000 <= (size.0 * size.1) as usize
                        && changed.iter().all(|by| *by <= 3),
                    "{name}: listing each piece's lamps changed eye {eye}: {changed:?}"
                );
            }
            let (some, of) = self::listed(farmyard, &stage);
            lamps += some;
            seen += of;
        };
        for (name, eyes, facing) in viewpoints() {
            for turn in [0.0, PI] {
                compare(&mut farmyard, name, quest_eyes(eyes, facing + turn));
            }
        }
        // By the campfire at its biggest, whose lamp reaches 7 m, flickering.
        farmyard.animate(
            &device,
            &queue,
            &crate::client::Drawn {
                chickens: Vec::new(),
                sticks: Vec::new(),
                logs: Vec::new(),
                fire: 1.0,
                stones: Vec::new(),
            },
        );
        let (seat, fire) = campsite_view();
        for at in [seat, seat + (fire - seat) * 0.5] {
            let heading = (-(fire - at).x).atan2(-(fire - at).z);
            compare(&mut farmyard, "Campfire", quest_eyes(at, heading));
        }
        if std::env::var_os("TOMORI_FARMYARD_CENSUS").is_some() {
            eprintln!("{seen} pieces and ferns seen, lit by {lamps} lamps");
        }
        // Each seen is lit by less than one of the four lamps on average, where every one added up all four.
        assert!(seen > 500 && lamps < seen, "{lamps} lamps over {seen}");
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn a_fern_in_reach_of_more_lamps_than_light_it_is_lit_by_the_brightest_those_at_the_cut_fading() {
    use tomori_render::world::{Light, World};
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (96, 96);
        let mut farmyard = with_hens(&device, &queue, size.0, size.1);
        // Eyes 3 m from the first fern, looking at it, and a dozen lamps in a row beside it, each further away.
        let heart = farmyard.ferns.hearts[0];
        let eyes = heart + Vec3::new(0.0, 0.6, 3.0);
        let clips = quest_eyes(eyes, 0.0);
        let mut world = World::new();
        let root = world.add_root();
        world.set_views(&clips);
        farmyard.submit(&mut world, root);
        for index in 0..12 {
            world.add_light(Light::new(
                root,
                heart + Vec3::new(1.0 + index as f32 * 0.3, 0.5, 0.0),
                8.0,
                Vec3::splat(0.2),
            ));
        }
        world.cull();
        farmyard.cull(&queue, &Views::of(&world, root));
        // Eight of them, the last fading.
        let [low, high, weights, more] = farmyard.ferns.lamps[0];
        assert_eq!(low.count_ones() + high.count_ones(), 8, "{low:b} {high:b}");
        assert!(
            weights == u32::MAX && more >> 24 < 255 && more >> 24 > 0,
            "{more:x}"
        );
        // Drawn so, it is drawn.
        let pictures = clips.map(|clip| capture_clip(&device, &queue, &farmyard, 4, clip, size));
        assert!(
            pictures[0]
                .chunks(4)
                .any(|pixel| pixel[..3] != pictures[0][..3])
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
