// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's farmyard, its client half: the scenery Tomori's clients draw, an evening sky behind the yard and the coop,
//! built in code and drawn with `tomori-assets`, with the chickens, and the New Zealand backdrop beyond the fence, its
//! foothills, bush, river, waterfall and ranges; the client side that shows the chickens from what
//! the farmyard's server side shares; its ambience, the wind, bush, river, waterfall and birds beyond the fence; and the
//! teleport profile, which a client registers with [`register`]. What its server and clients share is
//! `tomori-environment-farmyard`. See `design/farmyard.md`.
pub mod ambience;
pub mod client;
pub mod scenery;

pub use tomori_environment_farmyard::{ID, LABEL, LICENSE, VERSION, chicken, coop, layout, trail};

/// The heading the chicken's licence notice, [`LICENSE`], is shown under with a build's other bundled assets'.
pub const LICENSE_TITLE: &str = "The farmyard's chicken, by Hinoki Maple Williams (CC BY 4.0)";

/// Where participants may travel in the farmyard: teleporting onto the yard's ground, clear of what stands in it, and
/// along the trail round the outside of the fence, its band and its clearings, which join the yard's earth through the
/// gate at the same height; turning in snaps; and between the yard and the lookout at the trail's far end by a pair of
/// teleport pads. See Tomori's `design/locomotion.md` and `tomori-environment-farmyard`'s `context/trail.md`.
pub fn locomotion() -> tomori_scene::locomotion::Locomotion {
    use tomori_scene::locomotion::{Locomotion, Pad, Surface};
    let mut surfaces: Vec<Surface> = layout::walkable()
        .into_iter()
        .map(|(centre, size)| Surface::rectangle(centre, size))
        .collect();
    surfaces.extend(trail::outlines().into_iter().map(|outline| {
        Surface::polygon(outline, trail::HEIGHT).expect("the trail's outlines are simple")
    }));
    surfaces.extend(trail::clearings().into_iter().map(|(middle, radius)| {
        Surface::disc(glam::Vec3::new(middle.x, trail::HEIGHT, middle.y), radius)
    }));
    let pads = trail::pads().map(|pad| Pad::at(pad.centre).named(pad.name).facing(pad.facing));
    Locomotion::teleport(surfaces).with_pads([pads])
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
    fn the_trail_is_walkable_end_to_end_joining_the_yard_through_the_gate_without_a_gap_or_a_step()
    {
        use glam::{Vec2, Vec3};
        use tomori_scene::locomotion::{BACKSTEP, Locomotion};
        use tomori_scene::pointing::Ray;

        use super::{layout, trail};
        let profile = super::locomotion();
        let Locomotion::Teleport { surfaces, .. } = &profile else {
            unreachable!("the farmyard teleports");
        };
        // The yard's rectangles, the trail's stretches and its way out, and its clearings, all at the yard's height.
        assert_eq!(
            surfaces.len(),
            layout::WALKABLE.len() + trail::outlines().len() + trail::clearings().len()
        );
        assert!(surfaces.iter().all(|surface| surface.height() == 0.0));
        let on = |point: Vec2| {
            surfaces
                .iter()
                .any(|surface| surface.contains(Vec3::new(point.x, 0.0, point.y)))
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
        // Each station's clearing too, all of it.
        for station in &trail::STATIONS {
            for step in 0..12 {
                let yaw = step as f32 / 12.0 * std::f32::consts::TAU;
                let edge =
                    station.clearing() + Vec2::new(yaw.sin(), yaw.cos()) * (trail::CLEARING - 0.01);
                assert!(on(edge), "{}", station.name);
            }
        }
        // Stepping back along it, facing back the way one came, steps the whole way everywhere, through the gate and from
        // one stretch to the next, so that nothing steps or stops there.
        let mut points = vec![(inside, outside)];
        points.extend((0..line.len()).map(|index| (line[index], line[(index + 1) % line.len()])));
        for (from, to) in points {
            let along = (to - from).normalize();
            for step in 0..4 {
                let at = from.lerp(to, step as f32 / 4.0);
                let head = Vec3::new(at.x, 1.6, at.y);
                let facing = Vec3::new(-along.x, 0.0, -along.y);
                let stepped = profile.backstep(head, facing, 0.0).unwrap();
                assert!((stepped.length() - BACKSTEP).abs() < 1e-4, "{at} {stepped}");
            }
        }
        // Pointing out through the gate from the yard lands on the trail beyond it, at the yard's height.
        let eyes = Vec3::new(0.0, 1.6, -1.0);
        let ray = Ray::new(eyes, Vec3::new(0.0, 0.0, 3.5) - eyes).unwrap();
        let landing = profile.landing(&ray).unwrap();
        assert!(
            landing.y == 0.0 && landing.z > layout::FENCE.high.y,
            "{landing}"
        );
        // And the pads, the yard's and the lookout's, each stand on what is declared, so that both may be used.
        assert_eq!(profile.pads().count(), 1);
    }
}
