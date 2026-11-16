// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard submits its models' materials to the render world, each piece painted with its primitive's, its ferns'
//! and plaques' with theirs, and its models are drawn through them exactly as they were uploaded, until the world says
//! otherwise.
use super::culling::{quest_eyes, viewpoints, with_hens};
use super::*;
use tomori_render::materials::{self, Pipeline};

/// Every model the farmyard draws, the hens among them.
fn models(farmyard: &Farmyard) -> Vec<&Drawn> {
    let backdrop = &farmyard.backdrop;
    [
        &farmyard.yard,
        &farmyard.coop,
        &farmyard.campsite,
        &farmyard.beach,
        &farmyard.sticks,
        &farmyard.logs,
        &farmyard.stones,
    ]
    .into_iter()
    .chain([&backdrop.trail, &backdrop.terrace, &backdrop.foothills])
    .chain(&backdrop.plants)
    .chain([&backdrop.stones, &backdrop.rock])
    .chain(&farmyard.chickens)
    .collect()
}

#[test]
fn the_farmyard_s_models_are_drawn_through_the_world_s_materials_as_they_were_uploaded() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (192, 160);
        let mut farmyard = with_hens(&device, &queue, size.0, size.1);
        let (_, eyes, facing) = viewpoints()[0];
        let clips = quest_eyes(eyes, facing);
        let picture =
            |farmyard: &Farmyard| capture_clip(&device, &queue, farmyard, 4, clips[0], size);
        // Before: without a world, against the eyes' frustum, by every lamp.
        farmyard.cull(&queue, &Views::new(&clips));
        let before = picture(&farmyard);
        // Through the world, by every lamp too, so that only its materials could change what is drawn.
        let mut world = World::new();
        let root = world.add_root();
        world.set_views(&clips);
        farmyard.set_lamps_listed(false);
        let frame = |farmyard: &mut Farmyard, world: &mut World| {
            farmyard.submit(world, root);
            world.cull();
            farmyard.cull(&queue, &Views::of(world, root));
            picture(farmyard)
        };
        let after = frame(&mut farmyard, &mut world);
        assert_eq!(
            before, after,
            "drawn through the world's materials, it changed"
        );
        // Every piece is painted with a material the model pipeline draws, just as it was uploaded: nothing rewritten.
        let mut pieces = 0;
        for drawn in models(&farmyard) {
            let item = drawn.item.expect("each is submitted");
            assert_eq!(drawn.painted.len(), drawn.materials.len());
            assert_eq!(drawn.repainted, 0);
            for (instance, material) in drawn.batch.materials(&drawn.model).enumerate() {
                let painted = world.instance_material(item, instance);
                assert_eq!(painted, material.map(|material| &drawn.materials[material]));
                if let Some(painted) = painted {
                    assert_eq!(materials::pipeline(painted), Some(Pipeline::Models));
                    assert!(painted.lamps && painted.hazed);
                }
                pieces += 1;
            }
        }
        assert!(pieces > 300, "{pieces}");
        // The ferns by their own pipeline, lit by their own lamps; the plaques unlit, their text blended.
        let ferns = world
            .instance_material(farmyard.ferns.item.unwrap(), 0)
            .unwrap();
        assert_eq!(materials::pipeline(ferns), Some(Pipeline::Custom(0)));
        assert!(ferns.lamps);
        let plaques = world
            .instance_material(farmyard.plaques.item.unwrap(), 0)
            .unwrap();
        assert_eq!(materials::pipeline(plaques), Some(Pipeline::Surfaces));
        assert_eq!(plaques.alpha, Alpha::Blend);
        // The coop's paint, as the world says it, is drawn so.
        let paint = farmyard.coop.painted[0];
        let kept = *world.material(paint).unwrap();
        world.material_mut(paint).unwrap().base_color = glam::Vec4::new(0.0, 0.0, 1.0, 1.0);
        let painted = frame(&mut farmyard, &mut world);
        assert_eq!(farmyard.coop.repainted, 1);
        assert_ne!(painted, after);
        // What no model draws, it leaves as it was.
        world.material_mut(paint).unwrap().shading = Shading::Own;
        assert_eq!(frame(&mut farmyard, &mut world), painted);
        assert_eq!(farmyard.coop.repainted, 1);
        // As it was, it is drawn as it was.
        *world.material_mut(paint).unwrap() = kept;
        assert_eq!(frame(&mut farmyard, &mut world), after);
        assert_eq!(farmyard.coop.repainted, 2);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
