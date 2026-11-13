// Throwaway: where the farmyard's triangles go by distance and how large each is on screen.
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3, Vec4};
use tomori_assets::bounds::{Aabb, Frustum};
use tomori_assets::{Instance, Library, Model};
use tomori_environment_farmyard::{
    backdrop, beach, campsite, ferns, layout, props, terrace, trail,
};
use tomori_environment_farmyard_client::coop::Coop;
use tomori_environment_farmyard_client::scenery::CHUNK;
use tomori_scene::scene::headset::{Fov, View, view_projection};

const W: f32 = 1680.0;
const H: f32 = 1760.0;

fn eyes(middle: Vec3, heading: f32) -> [Mat4; 2] {
    let facing = Quat::from_rotation_y(heading) * Quat::from_rotation_x(-10f32.to_radians());
    let across = facing * Vec3::X * 0.032;
    let fov = |left: f32, right: f32| Fov {
        left: left.to_radians(),
        right: right.to_radians(),
        down: -50f32.to_radians(),
        up: 40f32.to_radians(),
    };
    [
        (middle - across, fov(-52.0, 41.0)),
        (middle + across, fov(-41.0, 52.0)),
    ]
    .map(|(position, fov)| {
        view_projection(&View {
            pose: tomori_scene::controller::Pose {
                position: position.to_array(),
                orientation: facing.to_array(),
            },
            fov,
        })
    })
}

fn heading(from: Vec3, to: Vec3) -> f32 {
    let way = to - from;
    (-way.x).atan2(-way.z)
}

struct Piece {
    kind: &'static str,
    bounds: Aabb,
    triangles: Vec<[Vec3; 3]>,
}

fn pieces_of(kind: &'static str, model: &Arc<Model>, transforms: &[Mat4], out: &mut Vec<Piece>) {
    for transform in transforms {
        let mut instance = Instance::new(model.clone());
        instance.transform = *transform;
        for draw in instance.draws() {
            for primitive in &model.meshes[draw.mesh].primitives {
                let world = if draw.joints.is_some() {
                    *transform
                } else {
                    draw.world
                };
                let triangles: Vec<[Vec3; 3]> = primitive
                    .indices
                    .chunks(3)
                    .map(|t| {
                        t.iter()
                            .map(|i| world.transform_point3(primitive.positions[*i as usize]))
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap()
                    })
                    .collect();
                if triangles.is_empty() {
                    continue;
                }
                let bounds = Aabb::from_points(triangles.iter().flatten().copied())
                    .grown(if draw.joints.is_some() { 0.3 } else { 0.0 });
                out.push(Piece {
                    kind,
                    bounds,
                    triangles,
                });
            }
        }
    }
}

fn report(name: &str, model: &Model) {
    let count = |m: &Model| {
        m.meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .map(|p| p.indices.len() / 3)
            .sum::<usize>()
    };
    let mut line = format!("{name:<12} {:>6}", count(model));
    for error in [0.005, 0.01, 0.02, 0.05, 0.1, 0.2] {
        let started = std::time::Instant::now();
        let (simple, strayed) = model.simplified(error);
        line += &format!(
            " | {error}: {:>6} ({:.3}, {:.0} ms)",
            count(&simple),
            strayed,
            started.elapsed().as_secs_f64() * 1e3
        );
    }
    println!("{line}");
}

fn main() {
    let chunked = |model: Model| Arc::new(model.chunked(CHUNK));
    if std::env::args().nth(1).as_deref() == Some("hen") {
        hen_debug();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("simplify") {
        report("yard", &chunked(props::model()));
        report(
            "coop",
            &chunked((*Coop::Built.model(Library::shared())).clone()),
        );
        report("campsite", &chunked(campsite::model()));
        report("beach", &chunked(beach::model()));
        report("trail", &chunked(trail::model()));
        report("terrace", &chunked(terrace::model()));
        report("foothills", &chunked(backdrop::model()));
        for kind in backdrop::Kind::ALL {
            if let Some(model) = kind.model() {
                report(&format!("{kind:?}"), &model);
            }
        }
        report("stone", &backdrop::stone());
        report("rock", &chunked(backdrop::rock()));
        let chicken = tomori_environment_farmyard::chicken::Chicken::shared().unwrap();
        for cm in [0.5f32, 1.0, 2.0, 3.0, 5.0, 8.0] {
            let (model, strayed) = chicken.model.simplified(cm / 100.0 / 0.0747);
            println!(
                "hen at {cm} cm: {} ({strayed})",
                model
                    .meshes
                    .iter()
                    .flat_map(|m| &m.primitives)
                    .map(|p| p.indices.len() / 3)
                    .sum::<usize>()
            );
        }
        report("hen", &chicken.model);
        return;
    }
    let mut pieces = Vec::new();
    let one = [Mat4::IDENTITY];
    pieces_of("chunks", &chunked(props::model()), &one, &mut pieces);
    pieces_of(
        "chunks",
        &chunked((*Coop::Built.model(Library::shared())).clone()),
        &[tomori_environment_farmyard::coop::placement()],
        &mut pieces,
    );
    pieces_of("chunks", &chunked(campsite::model()), &one, &mut pieces);
    pieces_of("chunks", &chunked(beach::model()), &one, &mut pieces);
    pieces_of("chunks", &chunked(trail::model()), &one, &mut pieces);
    pieces_of("chunks", &chunked(terrace::model()), &one, &mut pieces);
    pieces_of("foothills", &chunked(backdrop::model()), &one, &mut pieces);
    let placed = backdrop::plants();
    for kind in backdrop::Kind::ALL {
        let Some(model) = kind.model() else { continue };
        let transforms: Vec<Mat4> = placed
            .iter()
            .filter(|p| p.kind == kind)
            .map(|p| p.transform())
            .collect();
        pieces_of("bush", &Arc::new(model), &transforms, &mut pieces);
    }
    let stones: Vec<Mat4> = backdrop::stones()
        .iter()
        .chain(&terrace::stones())
        .map(backdrop::Stone::transform)
        .collect();
    pieces_of("rocks", &Arc::new(backdrop::stone()), &stones, &mut pieces);
    pieces_of("rocks", &chunked(backdrop::rock()), &one, &mut pieces);
    // Five hens about the yard.
    let chicken = tomori_environment_farmyard::chicken::Chicken::shared().unwrap();
    let hens: Vec<Mat4> = (0..5)
        .map(|i| {
            Mat4::from_translation(layout::ARRIVAL + Vec3::new(i as f32 - 2.0, 0.0, -4.0))
                * chicken.placement
        })
        .collect();
    pieces_of("hens", &chicken.model, &hens, &mut pieces);
    for fern in ferns::all() {
        let points = ferns::mesh(fern.kind, ferns::Detail::Near);
        let far = ferns::mesh(fern.kind, *fern.kind.details().last().unwrap());
        let t = fern.transform();
        let tri = |points: &[ferns::Point]| -> Vec<[Vec3; 3]> {
            points
                .chunks(3)
                .map(|c| [0, 1, 2].map(|i| t.transform_point3(c[i].position)))
                .collect()
        };
        let near = tri(&points);
        let far = tri(&far);
        let bounds = Aabb::from_points(near.iter().flatten().copied()).grown(0.3);
        let heart = fern.heart();
        pieces.push(Piece {
            kind: "ferns",
            bounds,
            triangles: near,
        });
        pieces.push(Piece {
            kind: "ferns far",
            bounds: Aabb::from_points([heart]),
            triangles: far,
        });
    }
    let ranges = backdrop::ranges().len() / 3;
    println!("ranges: {ranges} triangles, at the sky's depth");
    let total: usize = pieces
        .iter()
        .filter(|p| p.kind != "ferns far")
        .map(|p| p.triangles.len())
        .sum();
    println!("total {total}");

    let arrival = layout::ARRIVAL + Vec3::Y * 1.6;
    let station = trail::STATIONS[2];
    let (_, along, _) = trail::frame(station.at);
    let clearing = Vec3::new(station.clearing().x, 1.6, station.clearing().y);
    let (middle, seat_along, length, _) = trail::seat();
    let seat = middle + Vec3::new(seat_along.x, 0.0, seat_along.y) * length * 0.35 + Vec3::Y * 1.15;
    let fire = campsite::FIRE;
    let (sa, sc) = terrace::SEAT;
    let tseat = terrace::on_fall(sa, sc);
    let tseat = Vec3::new(tseat.x, layout::TERRACE + 1.2, tseat.y);
    let rock = layout::WATERFALL_ROCK + Vec3::Y * 1.6;
    let views = [
        ("arrival", arrival, heading(arrival, layout::ARRIVAL_FACING)),
        (
            "trail",
            clearing,
            heading(clearing, clearing + Vec3::new(along.x, 0.0, along.y)),
        ),
        (
            "campsite",
            seat,
            heading(seat, Vec3::new(fire.x - 0.5, 0.4, fire.z - 0.05)),
        ),
        ("terrace", tseat, heading(tseat, layout::ARRIVAL)),
        ("waterfall rock", rock, heading(rock, layout::ARRIVAL)),
    ];
    let kinds = ["chunks", "foothills", "bush", "rocks", "hens", "ferns"];
    let bands = [0.0, 10.0, 20.0, 50.0, 100.0, f32::INFINITY];
    for (name, eye, facing) in views {
        for (label, headings) in [
            ("facing", vec![facing]),
            (
                "8 headings",
                (0..8)
                    .map(|i| facing + i as f32 * std::f32::consts::TAU / 8.0)
                    .collect::<Vec<_>>(),
            ),
        ] {
            println!(
                "\n{name}, {label}: triangles submitted an eye (pieces seen), by distance of each triangle; median px area, share under 1 px"
            );
            let n = headings.len() as f32;
            let mut sum = 0.0;
            for kind in kinds {
                let mut counts = [0f32; 5];
                let mut areas: Vec<Vec<f32>> = vec![Vec::new(); 5];
                for h in &headings {
                    let clips = eyes(eye, *h);
                    let frustum = Frustum::enclosing(&clips);
                    for piece in pieces.iter().filter(|p| p.kind == kind) {
                        if !frustum.sees(&piece.bounds) {
                            continue;
                        }
                        // Ferns: near within 10 m of heart, far beyond: pick by the bounds' centre.
                        let tris = if kind == "ferns" {
                            let centre = (piece.bounds.min + piece.bounds.max) * 0.5;
                            if centre.distance(eye) > ferns::NEAR {
                                let i = pieces.iter().position(|q| std::ptr::eq(q, piece)).unwrap();
                                &pieces[i + 1].triangles
                            } else {
                                &piece.triangles
                            }
                        } else {
                            &piece.triangles
                        };
                        for t in tris {
                            let c = (t[0] + t[1] + t[2]) / 3.0;
                            let d = c.distance(eye);
                            let band = bands
                                .windows(2)
                                .position(|w| d >= w[0] && d < w[1])
                                .unwrap();
                            counts[band] += 1.0;
                            let p: Vec<Vec4> = t.iter().map(|v| clips[0] * v.extend(1.0)).collect();
                            if p.iter().all(|p| p.w > 0.01) {
                                let s: Vec<glam::Vec2> = p
                                    .iter()
                                    .map(|p| {
                                        glam::Vec2::new(
                                            (p.x / p.w + 1.0) * 0.5 * W,
                                            (p.y / p.w + 1.0) * 0.5 * H,
                                        )
                                    })
                                    .collect();
                                let inside = p.iter().any(|p| p.x.abs() <= p.w && p.y.abs() <= p.w);
                                if inside {
                                    let a = ((s[1] - s[0]).perp_dot(s[2] - s[0])).abs() * 0.5;
                                    areas[band].push(a);
                                }
                            }
                        }
                    }
                }
                let all: f32 = counts.iter().sum::<f32>() / n;
                sum += all;
                let cells: Vec<String> = (0..5)
                    .map(|b| {
                        let a = &mut areas[b];
                        a.sort_by(f32::total_cmp);
                        let median = a.get(a.len() / 2).copied().unwrap_or(0.0);
                        let small =
                            a.iter().filter(|x| **x < 1.0).count() as f32 / a.len().max(1) as f32;
                        format!(
                            "{:>6.0} ({:>6.1} px, {:>3.0}%)",
                            counts[b] / n,
                            median,
                            small * 100.0
                        )
                    })
                    .collect();
                println!(
                    "  {kind:<10} {all:>6.0}: <10 {} | 10-20 {} | 20-50 {} | 50-100 {} | >100 {}",
                    cells[0], cells[1], cells[2], cells[3], cells[4]
                );
            }
            println!("  all {sum:.0}");
        }
    }
}
#[allow(dead_code)]
fn hen_debug() {
    let chicken = tomori_environment_farmyard::chicken::Chicken::shared().unwrap();
    for mesh in &chicken.model.meshes {
        for p in &mesh.primitives {
            println!(
                "prim: {} verts {} tris, bounds {:?}, normals {} colors {} uvs {} joints {}",
                p.positions.len(),
                p.indices.len() / 3,
                p.bounds(),
                p.normals.len(),
                p.colors.len(),
                p.uvs.len(),
                p.joints.len()
            );
            let mut edges = std::collections::HashMap::new();
            let mut weld = std::collections::HashMap::new();
            let ids: Vec<u32> = p
                .positions
                .iter()
                .map(|q| {
                    let n = weld.len() as u32;
                    *weld.entry(q.to_array().map(f32::to_bits)).or_insert(n)
                })
                .collect();
            for t in p.indices.chunks(3) {
                for s in 0..3 {
                    let a = ids[t[s] as usize];
                    let b = ids[t[(s + 1) % 3] as usize];
                    *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
            let mut hist = std::collections::BTreeMap::new();
            for c in edges.values() {
                *hist.entry(*c).or_insert(0) += 1;
            }
            println!("welded {} points; edge counts {:?}", weld.len(), hist);
            let flat = p
                .indices
                .chunks(3)
                .filter(|t| {
                    p.normals[t[0] as usize] == p.normals[t[1] as usize]
                        && p.normals[t[1] as usize] == p.normals[t[2] as usize]
                })
                .count();
            let uvflat = p
                .indices
                .chunks(3)
                .filter(|t| {
                    p.uvs[t[0] as usize] == p.uvs[t[1] as usize]
                        && p.uvs[t[1] as usize] == p.uvs[t[2] as usize]
                })
                .count();
            println!(
                "flat normals {flat}, constant uvs {uvflat}; joints sample {:?} {:?}",
                p.joints[0], p.weights[0]
            );
        }
    }
    println!("placement {:?}", chicken.placement);
}
