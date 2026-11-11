// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's farmyard, its client half: the scenery Tomori's clients draw, an evening sky behind the yard and the coop,
//! built in code and drawn with `tomori-assets`, with the chickens, and the New Zealand backdrop beyond the fence, its
//! foothills, bush, river, waterfall and ranges; the client side that shows the chickens from what
//! the farmyard's server side shares; its ambience, the wind, bush, river, waterfall and birds beyond the fence; and the
//! teleport profile, which a client registers with [`register`]. What its server and clients share is
//! `tomori-environment-farmyard`. See `design/farmyard.md`.
pub mod ambience;
pub mod campsite;
pub mod client;
pub mod scenery;
pub mod stones;

pub use tomori_environment_farmyard::{ID, LABEL, LICENSE, VERSION, chicken, coop, layout, trail};

/// The heading the chicken's licence notice, [`LICENSE`], is shown under with a build's other bundled assets'.
pub const LICENSE_TITLE: &str = "The farmyard's chicken, by Hinoki Maple Williams (CC BY 4.0)";

/// Where participants may travel in the farmyard: teleporting onto the ground it draws, the yard's earth, the trail's
/// track and clearings round the outside of the fence and the beach's shingle down towards the river, clear of what
/// stands on it, as
/// `tomori-environment-farmyard`'s `walkable` derives it, at the heights it is drawn, so that whoever stands or lands
/// there, and the ring where they would, is on what is drawn; through the gate, on the way out declared by hand, and
/// about the campsite's fire; turning in snaps; between the yard and the lookout at the trail's far end by a pair of
/// teleport pads; and onto the waterfall rock, by pointing at it, and back to the lookout by pointing at its seat. See
/// Tomori's `design/locomotion.md` and `tomori-environment-farmyard`'s `context/trail.md`.
pub fn locomotion() -> tomori_scene::locomotion::Locomotion {
    use tomori_scene::locomotion::{Locomotion, Pad, Surface, Target};
    let mut surfaces = vec![Surface::mesh(
        tomori_environment_farmyard::walkable::navmesh().clone(),
    )];
    surfaces.extend(trail::way_out().into_iter().map(|(outline, height)| {
        Surface::polygon(outline, height).expect("the way out's outlines are simple")
    }));
    // About the campsite's fire, clear of its stones and seats, on its clearing as drawn.
    surfaces.extend(
        tomori_environment_farmyard::campsite::standing()
            .into_iter()
            .map(|(middle, radius)| {
                Surface::disc(
                    glam::Vec3::new(middle.x, trail::CLEARING_HEIGHT, middle.y),
                    radius,
                )
            }),
    );
    let pads = trail::pads().map(|pad| Pad::at(pad.centre).named(pad.name).facing(pad.facing));
    let targets = trail::targets().map(|target| {
        Target::object(target.foot, target.radius, target.height)
            .arriving(target.arrival)
            .named(target.name)
            .facing(target.facing)
    });
    Locomotion::teleport(surfaces)
        .with_pads([pads])
        .with_targets(targets)
}

/// Register the farmyard with a client's `sceneries`: its scenery, at the identity in `tomori-environment-farmyard`,
/// with its coop built in code, its teleport profile, its client side, which shows the chickens, and the licence of
/// the chicken it bundles. Tomori's own app and spectator do so; a build of another project's can too.
///
/// ```
/// let mut sceneries = tomori_render::scenery::Sceneries::empty();
/// tomori_environment_farmyard_client::register(&mut sceneries);
/// assert!(sceneries.contains(tomori_environment_farmyard_client::ID));
/// ```
pub fn register(sceneries: &mut tomori_render::scenery::Sceneries) {
    register_with(sceneries, coop::Coop::Built);
}

/// As [`register`], with `coop`, such as one modelled in Blender and bundled as an asset.
pub fn register_with(sceneries: &mut tomori_render::scenery::Sceneries, coop: coop::Coop) {
    registered(sceneries, coop, scenery::Clock::Running);
}

/// As [`register_with`], its water moved by `clock`.
pub(crate) fn registered(
    sceneries: &mut tomori_render::scenery::Sceneries,
    coop: coop::Coop,
    clock: scenery::Clock,
) {
    sceneries.register_versioned(ID, VERSION, move |target| {
        Box::new(scenery::Farmyard::with_clock(
            target,
            &coop,
            tomori_assets::Library::shared(),
            clock,
        ))
    });
    sceneries.set_locomotion(ID, locomotion());
    sceneries.set_client_side(ID, |space| {
        Box::new(client::FarmyardClient::new(
            space,
            chicken::Chicken::shared(),
            std::time::Instant::now(),
        ))
    });
    sceneries.add_license(LICENSE_TITLE, LICENSE);
    ambience::bundle();
    sceneries.set_ambience(ID, ambience::ambience());
    sceneries.add_license(ambience::LICENSE_TITLE, ambience::LICENSE);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_farmyard_registers_its_scenery_client_side_and_licence_at_its_identity_and_lets_participants_teleport()
     {
        use tomori_scene::environment::Environment;
        let mut sceneries = tomori_render::scenery::Sceneries::empty();
        super::register(&mut sceneries);
        assert!(sceneries.contains("farmyard"));
        assert!(
            sceneries
                .environments()
                .contains(&Environment::new(super::ID).with_version(super::VERSION))
        );
        let declared = sceneries.locomotion();
        assert_eq!(declared.len(), 1);
        assert_eq!(
            declared[0],
            (Environment::new("farmyard"), super::locomotion())
        );
        assert!(super::locomotion().travels());
        // Its client side, made for a space, shows nothing until the server side shares the flock.
        let client_sides = sceneries.client_sides();
        assert_eq!(client_sides[0].0.id(), super::ID);
        let mut client = (client_sides[0].1)("space-1");
        let output = client.update(tomori_scene::client_side::Input::new(
            Vec::new(),
            std::time::Instant::now(),
        ));
        assert!(output.drawn.is_none());
        // It carries the chicken's credit and licence, for the build to show with its own.
        assert_eq!(
            sceneries.licenses(),
            [
                (super::LICENSE_TITLE, super::LICENSE),
                (super::ambience::LICENSE_TITLE, super::ambience::LICENSE)
            ]
        );
        // Its ambience, at its identity, with every slot.
        let ambiences = sceneries.ambiences();
        assert_eq!(ambiences[0].0.id(), super::ID);
        assert_eq!(ambiences[0].1.slots().count(), super::ambience::SLOTS.len());
        assert!(super::LICENSE.contains("Hinoki Maple Williams"));
        assert!(super::LICENSE.contains("https://creativecommons.org/licenses/by/4.0/"));
    }

    #[test]
    fn the_trail_is_walkable_end_to_end_joining_the_yard_through_the_gate_without_a_gap_or_a_step_that_stops_anyone()
     {
        use glam::{Vec2, Vec3};
        use tomori_scene::locomotion::{BACKSTEP, Locomotion};
        use tomori_scene::pointing::Ray;

        use super::{layout, trail};
        let profile = super::locomotion();
        let Locomotion::Teleport { surfaces, .. } = &profile else {
            unreachable!("the farmyard teleports");
        };
        // The ground the farmyard draws, the trail's way out and the places to stand about the campsite's fire, all
        // within the centimetre a backstep goes up or down of the yard's earth, but the track's edges, which rise to
        // their crests.
        assert_eq!(
            surfaces.len(),
            1 + trail::way_out().len() + tomori_environment_farmyard::campsite::standing().len()
        );
        for surface in surfaces {
            match surface.navmesh() {
                Some(mesh) => {
                    assert!(mesh.triangles().iter().flatten().all(|corner| {
                        (-1e-6..=trail::LIFT + trail::EDGE + 1e-6).contains(&corner.y)
                    }))
                }
                None => assert!((0.0..0.009).contains(&surface.height())),
            }
        }
        let on = |point: Vec2| {
            surfaces
                .iter()
                .any(|surface| surface.contains(Vec3::new(point.x, 0.0, point.y)))
        };
        // Where whoever stands at `point` stands: on the highest surface there, as landing on it from above puts them.
        let floor = |point: Vec2| {
            surfaces
                .iter()
                .filter_map(|surface| surface.height_at(Vec3::new(point.x, 0.0, point.y)))
                .fold(f32::NEG_INFINITY, f32::max)
        };
        // From where participants arrive, out through the gate and round the whole loop back to it, every 5 cm, across
        // the whole width of the way out and the trail, there is somewhere to stand: no gap anywhere, nor at the gate.
        let line = trail::centreline();
        let arrival = Vec2::new(layout::ARRIVAL.x, layout::ARRIVAL.z);
        let [inside, outside] = trail::GATEWAY;
        let mut legs = vec![
            (arrival, inside, 0.0),
            (inside, outside, trail::HALF - 0.01),
        ];
        legs.extend((0..line.len()).map(|index| {
            (
                line[index],
                line[(index + 1) % line.len()],
                trail::HALF - 0.01,
            )
        }));
        let mut walked = 0.0;
        for (from, to, wide) in legs {
            let length = from.distance(to);
            let across = (to - from).normalize().perp();
            for step in 0..=(length / 0.05).ceil() as u32 {
                let point = from.lerp(to, (step as f32 * 0.05 / length).min(1.0));
                for offset in [-wide, -wide * 0.5, 0.0, wide * 0.5, wide] {
                    assert!(on(point + across * offset), "{point} {offset}");
                }
            }
            walked += length;
        }
        assert!(walked > 60.0, "{walked}");
        // Each station's clearing too, all of what it draws, nine-sided, to within a centimetre of its sides, but
        // within the clearance of its post, which stands at its edge, where nobody lands.
        let edge = trail::CLEARING * (std::f32::consts::PI / 9.0).cos() - 0.01;
        for station in &trail::STATIONS {
            let mut open = 0;
            for step in 0..36 {
                let yaw = step as f32 / 36.0 * std::f32::consts::TAU;
                let at = station.clearing() + Vec2::new(yaw.sin(), yaw.cos()) * edge;
                if kept(at) {
                    assert!(!on(at), "{} {at}", station.name);
                } else {
                    assert!(on(at), "{} {at}", station.name);
                    open += 1;
                }
            }
            assert!(open >= 24, "{} {open}", station.name);
            assert!(on(station.clearing()), "{}", station.name);
        }
        // Stepping back along it, either way, from where one stands, steps the whole way everywhere, out through the gate
        // and back in, from one stretch to the next, so that nothing stops there.
        let mut points = vec![(inside, outside), (outside, inside)];
        points.extend((0..line.len()).map(|index| (line[index], line[(index + 1) % line.len()])));
        for (from, to) in points {
            let along = (to - from).normalize();
            for step in 0..4 {
                let at = from.lerp(to, step as f32 / 4.0);
                let head = Vec3::new(at.x, 1.6, at.y);
                let facing = Vec3::new(-along.x, 0.0, -along.y);
                let stepped = profile.backstep(head, facing, floor(at)).unwrap();
                assert!((stepped.length() - BACKSTEP).abs() < 1e-4, "{at} {stepped}");
            }
        }
        // Pointing out through the gate from the yard lands on the trail beyond it, on its track.
        let eyes = Vec3::new(0.0, 1.6, -1.0);
        let ray = Ray::new(eyes, Vec3::new(0.0, 0.0, 3.5) - eyes).unwrap();
        let landing = profile.landing(&ray).unwrap();
        assert!(
            landing.y == trail::HEIGHT && landing.z > layout::FENCE.high.y,
            "{landing}"
        );
        // And the pads, the yard's and the lookout's, each stand on what is declared, so that both may be used.
        assert_eq!(profile.pads().count(), 1);
    }

    #[test]
    fn pointing_at_the_yard_s_open_ground_lands_there_beside_the_feeder_too() {
        use glam::{Vec2, Vec3};
        use tomori_scene::pointing::Ray;

        use super::layout;
        let profile = super::locomotion();
        // Every 10 cm of the yard's open ground, clear of what stands in it and of the fence, a teleport straight down
        // lands on its earth, where it is aimed.
        let kept: Vec<_> = layout::obstacles()
            .iter()
            .map(|obstacle| obstacle.grown(layout::CLEARANCE + 0.01))
            .collect();
        let yard = layout::FENCE.grown(-layout::FENCE_CLEARANCE - 0.01);
        let mut landed = 0;
        for column in 0..=(yard.size().x * 10.0) as u32 {
            for row in 0..=(yard.size().y * 10.0) as u32 {
                let point = yard.low + Vec2::new(column as f32, row as f32) * 0.1;
                if kept.iter().any(|footprint| footprint.contains(point)) {
                    continue;
                }
                let ray = Ray::new(Vec3::new(point.x, 1.6, point.y), -Vec3::Y).unwrap();
                let landing = profile.landing(&ray).expect("open ground is walkable");
                assert!(
                    landing.y < 0.009 && Vec2::new(landing.x, landing.z).distance(point) < 1e-4,
                    "{point}: {landing}"
                );
                landed += 1;
            }
        }
        assert!(landed > 7000, "{landed}");
        // Where the maintainer stood in a farmyard space, beside the feeder, pointing at the ground a metre or two
        // ahead, which had been left out of the yard's earth, lands there.
        let arrival = tomori_protocol::Pose::at(layout::ARRIVAL.to_array())
            .facing_towards(layout::ARRIVAL_FACING.to_array());
        let carriage = tomori_scene::anchors::Anchor::carriage(&arrival).unwrap();
        let carried = profile.carried(&carriage);
        let head = Vec3::new(3.97, 1.32, -4.82);
        let yaw = (-63f32).to_radians();
        let ahead = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        for distance in [1.0, 1.5, 2.0] {
            let aim = head.with_y(0.0) + ahead * distance;
            let landing = carried.landing(&Ray::new(head, aim - head).unwrap());
            assert!(
                landing.is_some_and(|landing| landing.distance(aim) < 1e-3),
                "{distance}: {landing:?}"
            );
        }
        let beside = carriage.unplace(head.with_y(0.0) + ahead);
        assert!((beside.x - layout::FEEDER.x).abs() < 0.5 && beside.z < layout::FEEDER.y - 1.0);
    }

    /// Where participants landed before the farmyard's ground was derived from what it draws, as November 2026's
    /// hand-laid surfaces had it (#353): the height of the highest floor at `point`, on the ground, or `None`. The
    /// yard's earth, 0.4 m inside the fence and beyond a quarter of a metre of each of the yard's obstacles; the loop's
    /// band, crest to crest, in four overlapping stretches; the way out; each clearing, a disc; and the places to stand
    /// about the campsite's fire.
    fn before() -> impl Fn(glam::Vec2) -> Option<f32> {
        use glam::{Vec2, Vec3};
        use tomori_scene::locomotion::Surface;

        use super::{layout, trail};
        let yard = layout::FENCE.grown(-layout::FENCE_CLEARANCE);
        let obstacles = layout::obstacles();
        let earth = move |point: Vec2| {
            yard.contains(point)
                && !obstacles.iter().any(|obstacle| {
                    let kept = obstacle.grown(layout::CLEARANCE);
                    point.cmpgt(kept.low).all() && point.cmplt(kept.high).all()
                })
        };
        let count = trail::centreline().len();
        let mut surfaces: Vec<Surface> = (0..4)
            .map(|stretch| {
                let sides: Vec<(Vec2, Vec2)> = (stretch * count / 4
                    ..=(stretch + 1) * count / 4 + 1)
                    .map(|index| {
                        let (middle, _, outward) = trail::frame(index);
                        (
                            middle + outward * trail::HALF,
                            middle - outward * trail::HALF,
                        )
                    })
                    .collect();
                let outline = sides
                    .iter()
                    .map(|(outer, _)| *outer)
                    .chain(sides.iter().rev().map(|(_, inner)| *inner));
                Surface::polygon(outline, trail::HEIGHT).unwrap()
            })
            .collect();
        surfaces.extend(
            trail::way_out()
                .into_iter()
                .map(|(outline, height)| Surface::polygon(outline, height).unwrap()),
        );
        let disc = |(middle, radius): (Vec2, f32)| {
            Surface::disc(
                Vec3::new(middle.x, trail::CLEARING_HEIGHT, middle.y),
                radius,
            )
        };
        surfaces.extend(trail::clearings().into_iter().map(disc));
        surfaces.extend(
            tomori_environment_farmyard::campsite::standing()
                .into_iter()
                .map(disc),
        );
        move |point| {
            surfaces
                .iter()
                .filter_map(|surface| surface.height_at(Vec3::new(point.x, 0.0, point.y)))
                .chain(earth(point).then_some(0.0))
                .reduce(f32::max)
        }
    }

    #[test]
    fn the_derived_ground_is_where_the_hand_laid_surfaces_were_but_what_stands_on_it_and_what_is_not_drawn()
     {
        use glam::{Vec2, Vec3};
        use tomori_scene::locomotion::Locomotion;

        use super::trail;
        let profile = super::locomotion();
        let Locomotion::Teleport { surfaces, .. } = &profile else {
            unreachable!("the farmyard teleports");
        };
        let after = |point: Vec2| {
            surfaces
                .iter()
                .filter_map(|surface| surface.height_at(Vec3::new(point.x, 0.0, point.y)))
                .reduce(f32::max)
        };
        let (before, groups) = (before(), obstacle_groups());
        let mut differences: std::collections::BTreeMap<&str, (u32, Vec<Vec2>)> =
            Default::default();
        let (mut both, mut neither) = (0, 0);
        for column in 0..=240 {
            for row in 0..=240 {
                let point =
                    Vec2::new(-12.0, -18.0) + Vec2::new(column as f32, row as f32) * 0.1 + 0.003;
                let middle = trail::from_middle(point);
                let drawn_clearing = trail::clearings().iter().any(|(centre, radius)| {
                    point.distance(*centre) <= radius * (std::f32::consts::PI / 9.0).cos()
                });
                let kept_by = groups
                    .iter()
                    .find(|(_, obstacles)| obstacles.iter().any(|obstacle| obstacle.keeps(point)))
                    .map(|(name, _)| *name);
                let (old, new) = (before(point), after(point));
                let same = match (old, new) {
                    (Some(old), Some(new)) => (old - new).abs() < 1e-3,
                    (old, new) => old.is_none() && new.is_none(),
                };
                if same {
                    neither += u32::from(old.is_none());
                    both += u32::from(old.is_some());
                    continue;
                }
                let lost = old.is_some() && new.is_none();
                let kind = [
                    (
                        "higher: up the track's edge",
                        old.is_some()
                            && new.is_some_and(|new| new > trail::HEIGHT)
                            && (trail::HALF - 0.08..=trail::HALF).contains(&middle),
                    ),
                    (kept_by.unwrap_or("lost: other"), lost && kept_by.is_some()),
                    (
                        "lost: undrawn clearing",
                        lost && kept_by.is_none() && !drawn_clearing && middle > trail::HALF,
                    ),
                ]
                .into_iter()
                .find(|(_, is)| *is)
                .map_or("other", |(kind, _)| kind);
                let entry = differences.entry(kind).or_default();
                entry.0 += 1;
                if entry.1.len() < 12 {
                    entry.1.push(point);
                }
            }
        }
        // Run with `--nocapture` for the table.
        println!("{both} points the same, {neither} walkable neither before nor after");
        for (kind, (count, points)) in &differences {
            println!("{kind}: {count}, such as {points:?}");
        }
        // The yard is as it was, and so is the rest of the trail, but three kinds of ground, each better: up the
        // track's edges, towards their crests, where the ring had been drawn beneath the edge, now on it; within the
        // clearance of a station's post and plaque, which participants had landed in, or on; and beyond what a
        // clearing draws, its nine sides within the disc it had been declared as. Nowhere is walkable that was not.
        assert!(both > 16000, "{both}");
        let kinds: Vec<&str> = differences.keys().copied().collect();
        assert!(
            kinds.iter().all(|kind| [
                "higher: up the track's edge",
                "lost: trail's posts, seat and rock",
                "lost: undrawn clearing"
            ]
            .contains(kind)),
            "{kinds:?}"
        );
        let count = |kind: &str| differences.get(kind).map_or(0, |(count, _)| *count);
        assert!(count("higher: up the track's edge") < 1000);
        assert!(count("lost: trail's posts, seat and rock") < 300);
        assert!(count("lost: undrawn clearing") < 60);
    }

    /// What the farmyard declares solid, in groups, each named as the ground lost to it is in the table of differences.
    fn obstacle_groups() -> Vec<(&'static str, Vec<tomori_scene::locomotion::Obstacle>)> {
        use tomori_scene::locomotion::Obstacle;
        let all = tomori_environment_farmyard::walkable::obstacles();
        let yard = tomori_environment_farmyard::layout::obstacles().len() + 7;
        let trail = tomori_environment_farmyard::trail::solids().len();
        let beside = tomori_environment_farmyard::trail::beside().len();
        let take = |from: usize, to: usize| -> Vec<Obstacle> {
            all[from..to].iter().filter(|o| low(o)).cloned().collect()
        };
        vec![
            ("lost: yard", take(0, yard)),
            (
                "lost: trail's posts, seat and rock",
                take(yard, yard + trail),
            ),
            ("lost: plants", take(yard + trail, yard + trail + beside)),
            ("lost: campsite", take(yard + trail + beside, all.len())),
        ]
    }

    /// Whether `obstacle` reaches into the room above the yard's and the trail's ground, as only what does takes ground
    /// from it: not a tree fern's crown high overhead, which participants walk beneath.
    fn low(obstacle: &tomori_scene::locomotion::Obstacle) -> bool {
        use tomori_environment_farmyard::trail;
        obstacle.bottom() < tomori_environment::navmesh::HEADROOM + trail::LIFT + trail::EDGE
    }

    /// Whether `point`, on the ground, is within the clearance of something the farmyard declares solid standing on the
    /// ground, where nobody lands.
    fn kept(point: glam::Vec2) -> bool {
        static OBSTACLES: std::sync::OnceLock<Vec<tomori_scene::locomotion::Obstacle>> =
            std::sync::OnceLock::new();
        OBSTACLES
            .get_or_init(tomori_environment_farmyard::walkable::obstacles)
            .iter()
            .any(|obstacle| low(obstacle) && obstacle.keeps(point))
    }

    /// How high the top of what `models` draw low down at `point` is, on the ground: the highest of their faces that
    /// face up, within 3 cm of the earth, over it, as the trail's track, its clearings and the yard's earth are, and
    /// none of what stands on them, such as a post or the seat.
    fn drawn(models: &[tomori_assets::Model], point: glam::Vec2) -> Option<f32> {
        use glam::Vec3;
        models
            .iter()
            .flat_map(|model| &model.meshes[0].primitives)
            .flat_map(|primitive| {
                primitive.indices.chunks(3).map(|triangle| {
                    [0, 1, 2].map(|corner| primitive.positions[triangle[corner] as usize])
                })
            })
            .filter_map(|[a, b, c]: [Vec3; 3]| {
                let normal = (b - a).cross(c - a);
                if normal.y <= 1e-9 {
                    return None;
                }
                // Where `point` is across the triangle, seen from above.
                let flat = |corner: Vec3| glam::Vec2::new(corner.x, corner.z);
                let (a2, b2, c2) = (flat(a), flat(b), flat(c));
                let area = (b2 - a2).perp_dot(c2 - a2);
                let u = (c2 - b2).perp_dot(point - b2) / area;
                let v = (a2 - c2).perp_dot(point - c2) / area;
                let w = 1.0 - u - v;
                let height = a.y * u + b.y * v + c.y * w;
                (u >= -1e-6 && v >= -1e-6 && w >= -1e-6 && height < 0.03).then_some(height)
            })
            .reduce(f32::max)
    }

    #[test]
    fn whoever_lands_on_the_trail_lands_on_its_track_as_drawn_and_so_does_the_ring_where_they_would()
     {
        use glam::{Vec2, Vec3};
        use tomori_scene::pointing::Ray;

        use super::{layout, trail};
        let profile = super::locomotion();
        let models = [trail::model(), tomori_environment_farmyard::props::model()];
        // Where a teleport straight down onto `point` lands, which is where the ring is drawn and the participant stands,
        // and the top of what is drawn there agree to a millimetre.
        let check = |point: Vec2| {
            let ray = Ray::new(Vec3::new(point.x, 2.0, point.y), -Vec3::Y).unwrap();
            let landing = profile.landing(&ray).expect("it is walkable");
            let top = drawn(&models, point).expect("something is drawn there");
            assert!((landing.y - top).abs() < 1e-3, "{point}: {landing} {top}");
            landing.y
        };
        // Along the loop, at every point of its line, across its track, short of its edges' slopes up to their crests.
        let line = trail::centreline();
        for index in 0..line.len() {
            let (point, _, outward) = trail::frame(index);
            for across in [-0.5, -0.25, 0.0, 0.25, 0.5] {
                assert_eq!(check(point + outward * across), trail::HEIGHT);
            }
        }
        // In each clearing, beyond the track's edge and its foot.
        for (middle, radius) in trail::clearings() {
            for step in 0..12 {
                let yaw = step as f32 / 12.0 * std::f32::consts::TAU;
                for reach in [0.0, radius * 0.5, radius * 0.8] {
                    let at = middle + Vec2::new(yaw.sin(), yaw.cos()) * reach;
                    let beyond = (0..line.len())
                        .map(|index| {
                            let (from, to) = (line[index], line[(index + 1) % line.len()]);
                            let t = ((at - from).dot(to - from) / from.distance_squared(to))
                                .clamp(0.0, 1.0);
                            at.distance(from.lerp(to, t))
                        })
                        .fold(f32::INFINITY, f32::min);
                    // Within the clearance of a station's post, which stands at its clearing's edge, nobody lands.
                    if kept(at) {
                        let ray = Ray::new(Vec3::new(at.x, 2.0, at.y), -Vec3::Y).unwrap();
                        assert_eq!(profile.landing(&ray), None, "{at}");
                    } else if beyond > trail::HALF + 0.2 {
                        assert_eq!(check(at), trail::CLEARING_HEIGHT, "{at}");
                    }
                }
            }
        }
        // Out through the gate, on the yard's earth and then on the way out's track; but over the path's first stone,
        // which stands a little proud of the earth and the ring stands taller than (see below), and either side of the
        // middle of the loop's inner edge, which is open only where the way out's middle meets it, rising to its crest
        // either side.
        let [inside, outside] = trail::GATEWAY;
        let stones = layout::stones();
        for step in 0..=26 {
            for across in [-0.5, 0.0, 0.5] {
                let at = inside.lerp(outside, step as f32 / 26.0) + Vec2::X * across;
                if (at.y - layout::FENCE.high.y).abs() < 0.01
                    || stones.iter().any(|stone| stone.height_at(at) > 0.0)
                    || (across != 0.0 && (at.y - (outside.y - trail::HALF)).abs() < 0.1)
                {
                    continue;
                }
                let height = check(at);
                let expected = if at.y < layout::FENCE.high.y {
                    trail::EARTH
                } else {
                    trail::HEIGHT
                };
                assert_eq!(height, expected, "{at}");
            }
        }
        // Each pad stands on what is drawn beneath it.
        for pad in trail::pads() {
            assert_eq!(check(Vec2::new(pad.centre.x, pad.centre.z)), pad.centre.y);
        }
        // The path's stones in the yard are walked over on its earth, so that a backstep crosses them, which they stand
        // too little proud of to hide the ring where a teleport lands, nor to feel underfoot.
        for stone in &stones {
            assert!(
                stone.height + 0.003 < tomori_render::graphics::LANDING_TOP,
                "{stone:?}"
            );
        }
    }

    #[test]
    fn the_water_is_never_walked_into_and_the_waterfall_rock_is_reached_only_by_pointing_at_it() {
        use glam::{Vec2, Vec3};
        use tomori_environment_farmyard::backdrop;
        use tomori_scene::locomotion::Locomotion;
        use tomori_scene::pointing::Ray;

        use super::{layout, trail};
        let profile = super::locomotion();
        let Locomotion::Teleport { surfaces, .. } = &profile else {
            unreachable!("the farmyard teleports");
        };
        let walkable = |point: Vec3| {
            surfaces
                .iter()
                .any(|surface| surface.contains(point.with_y(surface.height())))
        };
        // No surface reaches the river's water, its banks within the spread of its surface, or the pool and its rim.
        let line = layout::river_line(8);
        for (index, middle) in line.iter().enumerate() {
            let after = line[(index + 1).min(line.len() - 1)];
            let before = line[index.saturating_sub(1)];
            let across = Vec2::new(after.x - before.x, after.z - before.z)
                .normalize()
                .perp();
            for step in -12..=12 {
                let offset =
                    across * step as f32 / 12.0 * (layout::RIVER_HALF_WIDTH + backdrop::SPREAD);
                let point = Vec3::new(middle.x + offset.x, middle.y, middle.z + offset.y);
                assert!(!walkable(point), "{point}");
            }
        }
        let pool = layout::POOL;
        for step in 0..48 {
            let yaw = step as f32 / 48.0 * std::f32::consts::TAU;
            for reach in [0.0, 2.0, layout::POOL_RADIUS, backdrop::RIM[2]] {
                let point = pool + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * reach;
                assert!(!walkable(point), "{point}");
            }
        }
        // Two targets, used as the profile is carried into a space, where participants arrive at home, within its
        // bounds: the rock, arriving on its top, as high as it is drawn, facing the waterfall; and the lookout's seat,
        // arriving on the trail before it, facing the pool.
        let arrival = tomori_protocol::Pose::at(layout::ARRIVAL.to_array())
            .facing_towards(layout::ARRIVAL_FACING.to_array());
        let carriage = tomori_scene::anchors::Anchor::carriage(&arrival).unwrap();
        let carried = profile.carried(&carriage);
        assert_eq!(carried.pads().count(), 1);
        let targets: Vec<_> = carried.targets().collect();
        assert_eq!(targets.len(), 2);
        for target in &targets {
            let arrival = target.arrival();
            assert!(arrival.abs().max_element() < 19.9, "{arrival}");
        }
        let (rock, back) = (targets[0], targets[1]);
        assert_eq!(
            (rock.name(), back.name()),
            (Some("Waterfall rock"), Some("Pool lookout"))
        );
        let top = layout::WATERFALL_ROCK;
        let place = |at: Vec3| carriage.place(at);
        assert!(rock.arrives_on_top() && rock.arrival().distance(place(top)) < 1e-4);
        let drawn = backdrop::rock().meshes[0].primitives[0]
            .positions
            .iter()
            .map(|position| position.y)
            .fold(f32::MIN, f32::max);
        assert_eq!(drawn, top.y, "its top is drawn where one stands");
        let facing = |yaw: f32| Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
        let towards = |to: Vec3, from: Vec3| {
            ((place(to) - place(from)) * Vec3::new(1.0, 0.0, 1.0)).normalize()
        };
        assert!(facing(rock.yaw().unwrap()).dot(towards(layout::WATERFALL, top)) > 0.999);
        let [_, declared] = trail::targets();
        assert!(
            !back.arrives_on_top()
                && walkable(declared.arrival)
                && declared.arrival.y == trail::HEIGHT
        );
        assert!(back.arrival().distance(place(declared.arrival)) < 1e-4);
        assert!(facing(back.yaw().unwrap()).dot(towards(pool, declared.arrival)) > 0.999);
        // The rock's top is no surface, so that aiming a teleport at it from anywhere on the trail lands nowhere, on it or
        // in the pool about it, and stepping back on it steps nowhere, off it into the pool; nor is any pad there.
        assert!(!walkable(top));
        let line = trail::centreline();
        for point in &line {
            let eyes = Vec3::new(point.x, 1.6, point.y);
            for offset in [Vec3::ZERO, Vec3::X * 0.5, Vec3::Z * -0.5] {
                let ray = Ray::new(eyes, top + offset - eyes).unwrap();
                assert_eq!(profile.landing(&ray), None, "{point} {offset}");
            }
        }
        let eyes = top + Vec3::Y * 1.6;
        for facing in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
            assert_eq!(profile.backstep(eyes, facing, top.y), None);
        }
        // Nor is anyone kept there: a teleport aimed from it at the lookout's track lands on it.
        let (lookout, _, _) = trail::frame(trail::VIEWPOINTS[1].2);
        let track = Vec3::new(lookout.x, trail::HEIGHT, lookout.y);
        let landing = profile
            .landing(&Ray::new(eyes, track - eyes).unwrap())
            .unwrap();
        assert!(landing.distance(track) < 1e-3, "{landing}");
        for pair in profile.pads() {
            for pad in pair {
                assert!(pad.centre().distance(top) > 5.0);
            }
        }
        // From the lookout, the rock's side or top is within reach, and the ray to it crosses no surface before it;
        // and from the rock, the seat's.
        let lookout = track.with_y(1.6);
        for (from, target) in trail::targets()
            .into_iter()
            .zip([lookout, eyes])
            .map(|(target, from)| (from, target))
        {
            let to = target.foot + Vec3::Y * target.height * 0.6;
            assert!(
                Vec2::new(to.x - from.x, to.z - from.z).length()
                    < tomori_scene::locomotion::REACH - 2.0
            );
            let ray = Ray::new(from, to - from).unwrap();
            let crossed = (trail::HEIGHT - from.y) / ray.direction.y;
            if crossed > 0.0 && crossed < from.distance(to) {
                assert!(!walkable(ray.at(crossed)), "{from} {to}");
            }
        }
    }
}
