// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's yard and coop, in chunks, and its hens in their levels of detail, chosen for a Quest 3's eyes: each level simplified
//! within its error, its chunks' edges where they were; fewer triangles drawn from afar and never more; everything
//! within 4 m ahead of the eyes, 8 m at the corners of their view, drawn exactly as before; and, further, a few pixels changed, by little, through the haze. Set
//! `TOMORI_FARMYARD_CENSUS` to anything to print the triangles and pixels, and `TOMORI_FARMYARD_DETAIL_SIZE` to a size,
//! such as `1680x1760`, a Quest 3's eye's, to compare pictures of it rather than smaller ones.
use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;

use super::culling::{quest_eyes, with_hens};
use super::*;
use glam::Vec4;
use tomori_render::scenery::Stage;
use tomori_render::world::FADE_FRAMES;

/// The height of the eyes' images the levels are chosen for: a Quest 3's.
const QUEST: u32 = 1_760;

/// The heading, about the vertical from −Z, that looks from `from` towards `to`.
fn heading(from: Vec3, to: Vec3) -> f32 {
    let way = to - from;
    (-way.x).atan2(-way.z)
}

/// Where the eyes are and what they face: the arrival, facing the coop's corner; station 3, along the trail; seated at
/// the campsite, looking at the fire; seated on the upper terrace's flat rock, looking over the yard; and on the
/// waterfall rock, looking back at the yard, the furthest views of the farmyard there are.
fn views() -> [(&'static str, Vec3, f32); 5] {
    let arrival = layout::ARRIVAL + Vec3::Y * 1.6;
    let station = trail::STATIONS[2];
    let (_, along, _) = trail::frame(station.at);
    let clearing = Vec3::new(station.clearing().x, 1.6, station.clearing().y);
    let (seat, fire) = campsite_view();
    let (across, out) = tomori_environment_farmyard::terrace::SEAT;
    let flat = tomori_environment_farmyard::terrace::on_fall(across, out);
    let terrace = Vec3::new(flat.x, layout::TERRACE + 1.2, flat.y);
    let rock = layout::WATERFALL_ROCK + Vec3::Y * 1.6;
    [
        ("Arrival", arrival, heading(arrival, layout::ARRIVAL_FACING)),
        (
            "Trail, station 3",
            clearing,
            heading(clearing, clearing + Vec3::new(along.x, 0.0, along.y)),
        ),
        ("Campsite", seat, heading(seat, fire)),
        ("Upper terrace", terrace, heading(terrace, layout::ARRIVAL)),
        ("Waterfall rock", rock, heading(rock, layout::ARRIVAL)),
    ]
}

/// Cull the farmyard through `clips` until any level it changes has faded, and say what it then draws in each eye.
fn settled(
    stage: &mut Stage,
    farmyard: &mut Farmyard,
    queue: &wgpu::Queue,
    clips: &[Mat4],
) -> Tally {
    for _ in 0..FADE_FRAMES {
        stage.cull(farmyard, queue, clips, true);
    }
    stage.cull(farmyard, queue, clips, true)
}

/// The edges of `primitive` crossed by one triangle alone, by where their ends are, to the bit.
fn open(primitive: &tomori_assets::Primitive) -> HashSet<[[u32; 3]; 2]> {
    let mut edges: HashMap<[[u32; 3]; 2], usize> = HashMap::new();
    for triangle in primitive.indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|vertex| {
            primitive.positions[vertex as usize]
                .to_array()
                .map(f32::to_bits)
        });
        for (from, to) in [(a, b), (b, c), (c, a)] {
            *edges
                .entry(if from < to { [from, to] } else { [to, from] })
                .or_default() += 1;
        }
    }
    edges
        .into_iter()
        .filter(|(_, count)| *count == 1)
        .map(|(edge, _)| edge)
        .collect()
}

#[test]
fn each_level_of_the_yard_the_coop_and_a_hen_is_coarser_its_chunks_meeting_where_they_did() {
    let hen = tomori_environment_farmyard::chicken::Chicken::shared().unwrap();
    let models: Vec<(&str, Arc<Model>, Mat4, f32)> = vec![
        (
            "yard",
            Arc::new(props::model().chunked(CHUNK)),
            Mat4::IDENTITY,
            0.45,
        ),
        (
            "coop",
            Arc::new(coop::model().chunked(CHUNK)),
            coop::placement(),
            0.3,
        ),
        ("hen", hen.model.clone(), hen.placement, 0.55),
    ];
    let triangles = |model: &Model| -> usize {
        model
            .meshes
            .iter()
            .flat_map(|mesh| &mesh.primitives)
            .map(|primitive| primitive.indices.len() / 3)
            .sum()
    };
    for (name, model, placement, least) in models {
        let levels = coarser(&model, placement);
        // Kept, so that every instance shares them.
        assert!(Arc::ptr_eq(&levels, &coarser(&model, placement)));
        assert_eq!(levels.len(), DETAIL.len());
        let counts: Vec<usize> = std::iter::once(triangles(&model))
            .chain(levels.iter().map(triangles))
            .collect();
        assert!(
            counts.windows(2).all(|pair| pair[1] <= pair[0]),
            "{name}: {counts:?}"
        );
        let saved = 1.0 - counts[DETAIL.len()] as f32 / counts[0] as f32;
        assert!(saved >= least, "{name}: {counts:?}");
        for level in levels.iter() {
            for (own, coarse) in model
                .meshes
                .iter()
                .flat_map(|mesh| &mesh.primitives)
                .zip(level.meshes.iter().flat_map(|mesh| &mesh.primitives))
            {
                // Each chunk's open edges, where it meets the next, as they were.
                assert_eq!(open(coarse), open(own), "{name}");
            }
        }
        if std::env::var_os("TOMORI_FARMYARD_CENSUS").is_some() {
            eprintln!("{name}: {counts:?} triangles in its levels");
        }
    }
}

#[test]
fn from_afar_the_farmyard_draws_fewer_triangles_and_never_more() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut farmyard = with_hens(&device, &queue, 64, 64);
        let mut stage = Stage::new();
        stage.set_height(Some(QUEST));
        let print = std::env::var_os("TOMORI_FARMYARD_CENSUS").is_some();
        for (name, eyes, facing) in views() {
            let mut sums = [0; 2];
            for turn in 0..8 {
                let clips = quest_eyes(eyes, facing + turn as f32 * TAU / 8.0);
                farmyard.set_detailed(true);
                let detailed = settled(&mut stage, &mut farmyard, &queue, &clips);
                farmyard.set_detailed(false);
                let own = stage.cull(&mut farmyard, &queue, &clips, true);
                assert!(detailed.triangles <= own.triangles, "{name}, turned {turn}");
                assert_eq!((detailed.drawn, detailed.culled), (own.drawn, own.culled));
                if turn == 0 && print {
                    eprintln!(
                        "{name}, facing: {} triangles an eye in levels, {} in their own",
                        detailed.triangles, own.triangles
                    );
                }
                sums[0] += detailed.triangles;
                sums[1] += own.triangles;
            }
            if print {
                eprintln!(
                    "{name}, eight headings: {} triangles an eye in levels, {} in their own",
                    sums[0] / 8,
                    sums[1] / 8
                );
            }
            // From the upper terrace and the waterfall rock, where most is far away, a twentieth fewer at least.
            if name.starts_with("Upper") || name.starts_with("Waterfall") {
                assert!(sums[0] * 20 <= sums[1] * 19, "{name}: {sums:?}");
            }
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// `clip`, seeing no further than `far`, as the eyes' own see to 2 km, but for where its depth runs.
fn reaching(clip: Mat4, far: f32) -> Mat4 {
    let (near, depth) = (tomori_scene::depth::NEAR, clip.row(3));
    let mut rows = clip.transpose();
    rows.z_axis = depth * (far / (far - near)) - Vec4::W * (far * near / (far - near));
    rows.transpose()
}

#[test]
fn up_close_the_farmyard_is_drawn_as_it_was_and_further_away_changes_little_through_its_haze() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = std::env::var("TOMORI_FARMYARD_DETAIL_SIZE")
            .ok()
            .and_then(|size| {
                let (width, height) = size.split_once('x')?;
                Some((width.parse().ok()?, height.parse().ok()?))
            })
            .unwrap_or((448, 424));
        let mut farmyard = with_hens(&device, &queue, size.0, size.1);
        let mut stage = Stage::new();
        stage.set_height(Some(QUEST));
        let print = std::env::var_os("TOMORI_FARMYARD_CENSUS").is_some();
        let pixels = (size.0 * size.1) as usize;
        for (name, eyes, facing) in views() {
            for turn in [0.0, TAU / 4.0, TAU / 2.0] {
                let far = quest_eyes(eyes, facing + turn);
                // Within 4 m ahead, and so within 8 m at the corners of a Quest 3's view, where no level's error would
                // span a pixel, every pixel is as it was, to the bit.
                let near = far.map(|clip| reaching(clip, 4.0));
                for (clips, within) in [(near, true), (far, false)] {
                    settled(&mut stage, &mut farmyard, &queue, &clips);
                    let pictures = [true, false].map(|detailed| {
                        farmyard.set_detailed(detailed);
                        stage.cull(&mut farmyard, &queue, &clips, true);
                        clips.map(|clip| capture_clip(&device, &queue, &farmyard, 4, clip, size))
                    });
                    for eye in 0..2 {
                        let changed: Vec<u8> = pictures[0][eye]
                            .chunks(4)
                            .zip(pictures[1][eye].chunks(4))
                            .filter(|(a, b)| a != b)
                            .map(|(a, b)| {
                                a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap()
                            })
                            .collect();
                        if within {
                            assert!(
                                changed.is_empty(),
                                "{name}, turned {turn}: within 4 m, eye {eye} changed: {changed:?}"
                            );
                            continue;
                        }
                        // Further, where a coarser level is drawn, its edges move by at most a pixel of a Quest 3's, and
                        // the furthest ferns have fewer leaflets: at most two pixels in a hundred change, by 64 of 255 at
                        // most and, over the whole view, by a sixteenth of one of 255 on average, the haze having hidden
                        // much of them.
                        let total: usize = changed.iter().map(|by| usize::from(*by)).sum();
                        let mean = total as f32 / changed.len().max(1) as f32;
                        let most = changed.iter().copied().max().unwrap_or(0);
                        if print && eye == 0 {
                            eprintln!(
                                "{name}, turned {turn:.2}: {} of {pixels} pixels changed ({:.3}%), by {mean:.1} of 255 on average, {most} at most",
                                changed.len(),
                                changed.len() as f32 * 100.0 / pixels as f32
                            );
                        }
                        assert!(
                            changed.len() * 50 <= pixels && total * 16 <= pixels && most <= 64,
                            "{name}, turned {turn}: eye {eye}: {} changed by {mean} on average",
                            changed.len()
                        );
                    }
                }
            }
        }
        farmyard.set_detailed(true);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn a_fern_changing_its_detail_is_drawn_in_both_each_over_its_share_and_a_koru_fades_away() {
    use tomori_environment_farmyard::ferns::{self, Detail, Kind};
    let (_, meshes) = fern_vertices();
    let all = ferns::all();
    let lamps = vec![EVERY; all.len()];
    let tree = all
        .iter()
        .position(|fern| fern.kind == Kind::Ponga)
        .unwrap();
    let koru = all.iter().position(|fern| fern.kind == Kind::Koru).unwrap();
    let only = |index: usize, lod: Lod| {
        let mut details = vec![None; all.len()];
        details[index] = Some(lod);
        let (instances, draws) = fern_instances(&all, &details, &lamps, &meshes);
        let drawn: Vec<(Detail, [f32; 2])> = draws
            .iter()
            .flat_map(|(vertices, range)| {
                let ((_, detail), _) = meshes.iter().find(|(_, drawn)| drawn == vertices).unwrap();
                instances[range.start as usize..range.end as usize]
                    .iter()
                    .map(move |instance| (*detail, [instance.size[2], instance.size[3]]))
            })
            .collect();
        drawn
    };
    // Settled, in one detail, whole; changing, in both, the one it changes to over its share, the other over the rest.
    assert_eq!(only(tree, Lod::settled(2)), [(Detail::Distant, [0.0, 0.0])]);
    let changing = Lod {
        level: 1,
        from: 0,
        fade: 0.25,
    };
    assert_eq!(
        only(tree, changing),
        [(Detail::Near, [0.25, 2.0]), (Detail::Far, [0.25, 1.0])]
    );
    // A koru, drawn near alone, fades out where it would be drawn far, and is drawn no more.
    assert_eq!(only(koru, changing), [(Detail::Near, [0.25, 2.0])]);
    assert!(only(koru, Lod::settled(1)).is_empty());
    // Between two levels drawn alike, whole.
    let alike = Lod {
        level: 2,
        from: 1,
        fade: 0.5,
    };
    assert_eq!(only(koru, alike), []);
}

#[test]
fn half_way_between_two_details_each_pixel_of_a_fern_is_one_detail_s() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (448, 424);
        let mut farmyard = with_hens(&device, &queue, size.0, size.1);
        let (eye, target) = (ponga() + Vec3::new(4.0, -1.5, 4.0), ponga());
        let clip = tomori_scene::depth::perspective(1.2, size.0 as f32 / size.1 as f32)
            * glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
        farmyard.cull(&queue, &Views::new(&[clip]));
        let count = farmyard.ferns.ferns.len();
        let mut drawn = |lod: Lod| {
            farmyard.ferns.lods = Some(vec![lod; count]);
            farmyard.ferns.place(&queue, eye);
            capture_clip(&device, &queue, &farmyard, 4, clip, size)
        };
        let [near, far] = [0, 1].map(|level| drawn(Lod::settled(level)));
        let half = drawn(Lod {
            level: 1,
            from: 0,
            fade: 0.5,
        });
        let (mut either, mut differ) = (0, 0);
        for ((near, far), half) in near.chunks(4).zip(far.chunks(4)).zip(half.chunks(4)) {
            if near != far {
                differ += 1;
                either += usize::from(half == near || half == far);
            }
        }
        // Where the two details differ, every pixel is one or the other's, but along an edge, where a pixel's samples
        // are of both.
        assert!(differ > 500, "{differ}");
        assert!(either * 10 >= differ * 9, "{either} of {differ}");
        // Without a world to choose, each is drawn by its distance, the furthest distant, or far to compare.
        let above = Vec3::new(0.0, 60.0, 0.0);
        let levels = |farmyard: &mut Farmyard, detailed: bool| {
            farmyard.set_detailed(detailed);
            farmyard.cull(&queue, &Views::unculled(&[clip]));
            farmyard.ferns.place(&queue, above);
            let placed = farmyard.ferns.placed.lock().unwrap();
            // Of those drawn: a koru beyond its near detail is not.
            placed
                .0
                .iter()
                .flatten()
                .map(|lod| lod.level)
                .filter(|level| *level != u8::MAX)
                .max()
        };
        assert_eq!(levels(&mut farmyard, true), Some(2));
        assert_eq!(levels(&mut farmyard, false), Some(1));
        farmyard.set_detailed(true);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
