// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's large models in chunks: each chunk about its cell, every triangle where it was, so that the
//! foothills' ground and the upper terrace still meet along every seam as they did; drawn as they were whole, from the
//! census's viewpoints through a Quest 3's eyes; and, with 28 more lanterns about the yard, 32 in all, most of what the
//! eyes may see reached by no more lamps than light one, each lighting it wholly, where the yard's ground, the trail
//! and the foothills, whole, were each reached by 10 to 27, drawn as by every lamp. Set `TOMORI_FARMYARD_CENSUS` to
//! anything to print how many lamps reach each piece.
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use super::culling::{chunked_with_hens, quest_eyes, viewpoints};
use super::*;
use tomori_render::scenery::Stage;
use tomori_render::world::{LIGHTS_PER_INSTANCE, Light, World};

/// Each triangle of `model`'s primitives, by material, each corner's position, normal and colour to the bit.
fn triangles(model: &Model) -> HashMap<Option<usize>, Vec<[[u32; 10]; 3]>> {
    let mut triangles: HashMap<Option<usize>, Vec<[[u32; 10]; 3]>> = HashMap::new();
    for primitive in model.meshes.iter().flat_map(|mesh| &mesh.primitives) {
        let corner = |vertex: u32| {
            let vertex = vertex as usize;
            let mut corner = [0; 10];
            let values = primitive.positions[vertex]
                .to_array()
                .into_iter()
                .chain(primitive.normals[vertex].to_array())
                .chain(primitive.colors[vertex].to_array());
            for (slot, value) in corner.iter_mut().zip(values) {
                *slot = value.to_bits();
            }
            corner
        };
        triangles.entry(primitive.material).or_default().extend(
            primitive
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|triangle| triangle.map(corner)),
        );
    }
    triangles
}

/// How many triangles of `triangles` have each edge, by where its corners are, to the bit, either way along it.
fn edges(triangles: &[[[u32; 10]; 3]]) -> HashMap<[[u32; 3]; 2], usize> {
    let mut edges = HashMap::new();
    for triangle in triangles {
        for index in 0..3 {
            let [from, to] = [triangle[index], triangle[(index + 1) % 3]]
                .map(|corner| [corner[0], corner[1], corner[2]]);
            *edges
                .entry(if from < to { [from, to] } else { [to, from] })
                .or_default() += 1;
        }
    }
    edges
}

#[test]
fn the_large_models_are_split_into_chunks_about_their_cells_every_triangle_where_it_was() {
    // The yard, its meadow and pines, the foothills, the trail and the upper terrace, each larger than a chunk; and the
    // rest, each smaller, whole.
    for (name, model, least) in [
        ("yard", props::model(), 20),
        ("foothills", backdrop::model(), 70),
        ("trail", trail::model(), 7),
        ("terrace", terrace::model(), 1),
        ("campsite", campsite::model(), 0),
        ("beach", beach::model(), 0),
        ("rock", backdrop::rock(), 0),
        ("coop", coop::model(), 0),
    ] {
        let chunked = model.chunked(CHUNK);
        let (before, after) = (
            model
                .meshes
                .iter()
                .map(|mesh| mesh.primitives.len())
                .sum::<usize>(),
            chunked
                .meshes
                .iter()
                .map(|mesh| mesh.primitives.len())
                .sum::<usize>(),
        );
        if least == 0 {
            assert_eq!(chunked, model, "{name}");
            continue;
        }
        assert!(after >= before + least, "{name}: {before} to {after}");
        // Each chunk within its cell but for the triangles reaching out of it, at most 3 m long, the foothills', or a
        // closed part of its own, as a rail of the fence along a side is.
        for primitive in chunked.meshes.iter().flat_map(|mesh| &mesh.primitives) {
            let (low, high) = primitive.bounds();
            let across = (high - low).x.max((high - low).z);
            assert!(
                across <= CHUNK + 3.0 || primitive.closed(),
                "{name}: {low} to {high}"
            );
        }
        // Every closed part whole, still closed.
        let closed = |model: &Model| {
            model
                .meshes
                .iter()
                .flat_map(|mesh| &mesh.primitives)
                .map(|primitive| {
                    primitive
                        .closed_triangles()
                        .iter()
                        .filter(|closed| **closed)
                        .count()
                })
                .sum::<usize>()
        };
        assert_eq!(closed(&chunked), closed(&model), "{name}");
        // Every triangle once, where it was, to the bit, of its material: so every seam between chunks, and between
        // the foothills' ground, the stream's bed and the cliffs about it, or the terrace's floor and its sides, is
        // where it was, shared as many times, and nothing open that was not.
        let (mut whole, mut split) = (triangles(&model), triangles(&chunked));
        assert_eq!(
            whole.keys().collect::<std::collections::BTreeSet<_>>(),
            split.keys().collect(),
            "{name}"
        );
        for (material, triangles) in &mut whole {
            let chunks = split.get_mut(material).unwrap();
            assert_eq!(edges(chunks), edges(triangles), "{name}");
            triangles.sort_unstable();
            chunks.sort_unstable();
            assert!(chunks == triangles, "{name}");
        }
    }
}

#[test]
fn the_farmyard_in_chunks_looks_as_it_did_whole() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (448, 424);
        let mut farmyards =
            [Some(CHUNK), None].map(|chunk| chunked_with_hens(&device, &queue, size, chunk));
        let mut stages = [Stage::new(), Stage::new()];
        let mut views: Vec<(String, [Mat4; 2])> = viewpoints()
            .into_iter()
            .flat_map(|(name, eyes, facing)| {
                [0.0, FRAC_PI_2, PI]
                    .map(|turn| (format!("{name} {turn}"), quest_eyes(eyes, facing + turn)))
            })
            .collect();
        let (seat, fire) = campsite_view();
        views.push((
            "Campfire".into(),
            quest_eyes(seat, (-(fire - seat).x).atan2(-(fire - seat).z)),
        ));
        let mut tallies = [Tally::default(); 2];
        for (name, clips) in views {
            let pictures: Vec<[Vec<u8>; 2]> = farmyards
                .iter_mut()
                .zip(&mut stages)
                .zip(&mut tallies)
                .map(|((farmyard, stage), tally)| {
                    *tally += stage.cull(farmyard, &queue, &clips, true);
                    clips.map(|clip| capture_clip(&device, &queue, farmyard, 4, clip, size))
                })
                .collect();
            for eye in 0..2 {
                // The same pixels, but where a GPU rounds a colour apart, at most one pixel in 10,000, by at most 2 of 255,
                // or, on a silhouette, a sample where two surfaces meet at one depth, which of them is drawn first decides.
                let changed: Vec<u8> = pictures[0][eye]
                    .chunks(4)
                    .zip(pictures[1][eye].chunks(4))
                    .filter(|(a, b)| a != b)
                    .map(|(a, b)| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap())
                    .collect();
                assert!(
                    changed.len() * 10_000 <= (size.0 * size.1) as usize
                        && changed.iter().filter(|by| **by > 2).count() <= 2,
                    "{name}: chunks changed eye {eye}: {changed:?}"
                );
            }
        }
        // It culls more and draws fewer triangles, in more draws.
        let [chunked, whole] = tallies;
        assert!(chunked.triangles < whole.triangles && chunked.draws > whole.draws);
        if std::env::var_os("TOMORI_FARMYARD_CENSUS").is_some() {
            eprintln!("in chunks: {chunked:?}; whole: {whole:?}");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// The 28 lanterns `scenery_frames` adds about the yard and beyond it, in rows 5 m apart across and 6 m deep, each
/// reaching 4 m.
fn lanterns(world: &mut World, root: RootId) {
    for index in 0..28 {
        let at = Vec3::new(
            -15.0 + (index % 7) as f32 * 5.0,
            2.0,
            -20.0 + (index / 7) as f32 * 6.0,
        );
        world.add_light(Light::new(root, at, 4.0, Vec3::new(1.0, 0.7, 0.35)));
    }
}

/// Of each piece of the farmyard's models and each fern that the eyes may see, as last culled in `world`: whether it is
/// a fern, how many of the lights the world lights `root` by reach it, as the world finds them, and the lights that
/// light it.
fn reached<'a>(
    farmyard: &Farmyard,
    world: &'a World,
    root: RootId,
) -> Vec<(bool, usize, &'a [tomori_render::world::Lit])> {
    let lights = world.root_lights(root);
    let reaching = |bounds: Option<Aabb>| {
        lights
            .iter()
            .filter(|light| {
                let reaches = |bounds: Aabb| {
                    let nearest = light.position.max(bounds.min).min(bounds.max);
                    nearest.distance(light.position) < light.reach * (1.0 + 1e-4) + 0.01
                };
                light.gives_light() && bounds.is_none_or(reaches)
            })
            .count()
    };
    let backdrop = &farmyard.backdrop;
    let mut reached = Vec::new();
    for drawn in [
        &farmyard.yard,
        &farmyard.coop,
        &farmyard.campsite,
        &farmyard.beach,
        &backdrop.trail,
        &backdrop.terrace,
        &backdrop.foothills,
        &backdrop.stones,
        &backdrop.rock,
    ]
    .into_iter()
    .chain(&farmyard.chickens)
    .chain(&backdrop.plants)
    {
        let Some(item) = drawn.item else { continue };
        for ((bounds, seen), lit) in drawn
            .batch
            .bounds()
            .zip(world.seen(item))
            .zip(world.lit(item))
        {
            if *seen {
                reached.push((false, reaching(bounds), lit));
            }
        }
    }
    let item = farmyard.ferns.item.expect("the ferns are submitted");
    for ((bounds, seen), lit) in farmyard
        .ferns
        .bounds
        .iter()
        .zip(world.seen(item))
        .zip(world.lit(item))
    {
        if *seen {
            reached.push((true, reaching(Some(*bounds)), lit));
        }
    }
    reached
}

#[test]
fn with_32_lamps_the_chunks_are_reached_by_few_more_than_light_one_and_look_as_lit_by_every_lamp() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (448, 424);
        let census = std::env::var_os("TOMORI_FARMYARD_CENSUS").is_some();
        let mut most = [0; 2];
        // The most pixels any view changed lit by its pieces' lamps, the most by more than 2 of 255, and by how much at
        // most, in chunks and whole.
        let mut worst = [(0, 0, 0); 2];
        for (chunk, farmyard) in [Some(CHUNK), None]
            .map(|chunk| (chunk, chunked_with_hens(&device, &queue, size, chunk)))
            .iter_mut()
        {
            let (mut pieces, mut over, mut lit) = (0, 0, 0);
            for (name, eyes, facing) in viewpoints() {
                for turn in [0.0, FRAC_PI_2, PI, -FRAC_PI_2] {
                    let clips = quest_eyes(eyes, facing + turn);
                    let mut world = World::new();
                    let root = world.add_root();
                    world.set_views(&clips);
                    farmyard.submit(&mut world, root);
                    lanterns(&mut world, root);
                    let mut pictures = [true, false].map(|listed| {
                        farmyard.set_lamps_listed(listed);
                        world.cull();
                        farmyard.cull(&queue, &Views::of(&world, root));
                        clips.map(|clip| capture_clip(&device, &queue, farmyard, 4, clip, size))
                    });
                    farmyard.set_lamps_listed(true);
                    world.cull();
                    farmyard.cull(&queue, &Views::of(&world, root));
                    for (fern, reaching, lights) in reached(farmyard, &world, root) {
                        pieces += 1;
                        lit += lights.len();
                        // Lit by each lamp that reaches it, wholly, or, of more, by the eight brightest on it.
                        assert_eq!(lights.len(), reaching.min(LIGHTS_PER_INSTANCE));
                        if reaching <= LIGHTS_PER_INSTANCE {
                            assert!(lights.iter().all(|lit| lit.weight == 1.0));
                        } else {
                            over += 1;
                        }
                        if !fern {
                            let most = &mut most[usize::from(chunk.is_none())];
                            *most = (*most).max(reaching);
                        }
                    }
                    let [listed, every] = &mut pictures;
                    for eye in 0..2 {
                        let changed: Vec<u8> = listed[eye]
                            .chunks(4)
                            .zip(every[eye].chunks(4))
                            .filter(|(a, b)| a != b)
                            .map(|(a, b)| {
                                a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap()
                            })
                            .collect();
                        if census {
                            eprintln!(
                                "{name} {turn}, eye {eye}: {} pixels changed, by at most {:?}",
                                changed.len(),
                                changed.iter().max()
                            );
                        }
                        let worst = &mut worst[usize::from(chunk.is_none())];
                        *worst = (
                            worst.0.max(changed.len()),
                            worst.1.max(changed.iter().filter(|by| **by > 2).count()),
                            worst.2.max(changed.iter().copied().max().unwrap_or(0)),
                        );
                    }
                }
            }
            if census {
                eprintln!(
                    "{chunk:?}: {pieces} pieces and ferns seen, lit by {lit} lamps, {over} reached by more than 8"
                );
            }
        }
        if census {
            eprintln!(
                "most lamps reaching a piece, in chunks and whole: {most:?}; pixels changed: {worst:?}"
            );
        }
        // Whole, the ground, trail and foothills were reached by up to 27, and most of a view drawn wrong. In chunks, a
        // piece by at most one more than light one, which, the dimmest on it left out, faded, changes next to nothing:
        // at most one pixel in 500 by at most 2 of 255, but for a few along the fence's rails, by at most 7.
        assert!(
            most[1] >= 20 && worst[1].0 * 2 > (size.0 * size.1) as usize,
            "{most:?} {worst:?}"
        );
        assert!(most[0] <= LIGHTS_PER_INSTANCE + 1, "{most:?}");
        assert!(
            worst[0].0 * 500 <= (size.0 * size.1) as usize && worst[0].1 <= 10 && worst[0].2 <= 7,
            "{worst:?}"
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
