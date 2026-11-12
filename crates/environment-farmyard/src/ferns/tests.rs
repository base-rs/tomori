// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_environment::navmesh::{Navmesh, Obstacle};
use tomori_environment::sightlines::{EYES, Spot, noticed, readable, seen};

use super::*;

/// The ferns near the trail, but their koru.
fn standing_ferns() -> Vec<Fern> {
    ferns()
        .into_iter()
        .filter(|fern| fern.kind != Kind::Koru)
        .collect()
}

/// Whether `point` is inside the polygon through `corners`.
fn inside(corners: &[Vec2], point: Vec2) -> bool {
    let mut inside = false;
    for index in 0..corners.len() {
        let (a, b) = (corners[index], corners[(index + 1) % corners.len()]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
        {
            inside = !inside;
        }
    }
    inside
}

/// Whether `point` is on a surface participants walk or land on: the ground the yard and the trail draw
/// ([`crate::walkable::ground`]), before what stands on it is taken away, so that a fern standing on it is found there
/// rather than taken away with it; the way out through the gate; the clearings, as the discs they are drawn within; and
/// the places to stand about the campsite's fire.
fn walked(point: Vec2) -> bool {
    static DRAWN: OnceLock<Navmesh> = OnceLock::new();
    DRAWN
        .get_or_init(|| Navmesh::from_triangles(crate::walkable::ground()))
        .height_at(point)
        .is_some()
        || trail::way_out()
            .iter()
            .any(|(outline, _)| inside(outline, point))
        || trail::clearings()
            .into_iter()
            .chain(campsite::standing())
            .any(|(middle, radius)| point.distance(middle) <= radius)
}

/// Points about `middle`, `radius` from it, every 22.5°, and `middle` itself.
fn about(middle: Vec2, radius: f32) -> Vec<Vec2> {
    (0..16)
        .map(|step| middle + way(step as f32 / 16.0 * TAU) * radius)
        .chain([middle])
        .collect()
}

#[test]
fn the_ferns_are_the_same_every_time_tree_ferns_and_ground_ferns_of_every_kind_with_koru() {
    let ferns = ferns();
    assert_eq!(ferns, placed());
    let of = |kind: Kind| -> Vec<Fern> {
        ferns
            .iter()
            .filter(|fern| fern.kind == kind)
            .copied()
            .collect()
    };
    // Every kind, none beyond its most, each within its sizes, turned and swaying its own way.
    for (kind, most, [least, largest]) in KINDS {
        let of = of(kind);
        assert!(of.len() >= 2 && of.len() <= most, "{kind:?} {}", of.len());
        assert!(
            of.iter()
                .all(|fern| (least..=largest).contains(&fern.scale)),
            "{kind:?}"
        );
        assert!(of.windows(2).all(|pair| pair[0].turn != pair[1].turn
            && pair[0].phase != pair[1].phase
            && (0.0..TAU).contains(&pair[0].turn)));
    }
    let trees = ferns.iter().filter(|fern| fern.kind.is_tree()).count();
    let ground = ferns
        .iter()
        .filter(|fern| Kind::GROUND.contains(&fern.kind))
        .count();
    assert!((20..=TREE_FERNS).contains(&trees), "{trees}");
    assert!((90..=GROUND_FERNS).contains(&ground), "{ground}");
    // Two koru at the heart of every other tree fern, among its fronds' bases, and one at a few ground ferns'.
    let koru = of(Kind::Koru);
    let hearts: Vec<(Vec3, f32)> = ferns
        .iter()
        .filter(|fern| fern.kind != Kind::Koru)
        .map(|fern| {
            (
                fern.transform().transform_point3(fern.kind.heart()),
                fern.scale,
            )
        })
        .collect();
    for one in &koru {
        assert!(
            hearts
                .iter()
                .any(|(heart, scale)| heart.distance(one.at) <= 0.1 * scale + 1e-4),
            "{one:?}"
        );
    }
    assert!(koru.len() >= trees + 2, "{}", koru.len());
}

#[test]
fn every_fern_stands_on_the_ground_off_where_anyone_stands_or_walks_and_out_of_the_water() {
    let pool = Vec2::new(POOL.x, POOL.z);
    for fern in standing_ferns() {
        let at = Vec2::new(fern.at.x, fern.at.z);
        let extent = fern.kind.extent();
        // On the ground as drawn there, a little into it.
        assert!((fern.at.y - (foot(at) - 0.02)).abs() < 1e-4, "{fern:?}");
        // Out of the water and off its banks' slopes.
        assert!(layout::nearest_river(at).1 > BANKS[1], "{fern:?}");
        assert!(at.distance(pool) > RIM[1], "{fern:?}");
        // A ground fern's fronds, and a tree fern's trunk, beyond the fence and off everywhere participants stand or
        // walk, and a ground fern within 5 m of where they do, to be seen.
        let (reach, height) = fern.size();
        let footing = if fern.kind.is_tree() {
            extent.trunk.0 * fern.scale
        } else {
            reach
        };
        for point in about(at, footing) {
            assert!(!walked(point), "{fern:?} {point}");
            assert!(!FENCE.contains(point), "{fern:?} {point}");
        }
        if fern.kind.is_tree() {
            // Its crown hangs over where anyone walks only high overhead, and over the fence by no more than half a
            // metre.
            let low = fern.solids()[1].low[1];
            if about(at, reach).into_iter().any(walked) {
                assert!(low >= OVERHEAD, "{fern:?}");
            }
            assert!(FENCE.distance(at) >= reach - 0.5, "{fern:?}");
            assert!((2.0..7.0).contains(&height), "{fern:?}");
        } else {
            assert!(from_standing(at) <= 5.0, "{fern:?}");
            assert!((0.2..1.2).contains(&height), "{fern:?}");
        }
    }
}

#[test]
fn the_tree_ferns_are_solid_to_the_walkable_ground_and_no_fern_cuts_any_of_it() {
    // Each tree fern beside the trail, its trunk and its crown, is among what the walkable ground is derived less, as
    // the plants and pines beside it are, by the yard's clearance; most of them are beside it.
    let obstacles = crate::walkable::obstacles();
    let beside = trail::beside();
    let declared = |solid: &Solid| {
        Obstacle::cuboid(solid.low.into(), solid.high.into()).with_clearance(layout::CLEARANCE)
    };
    let trees = super::solids();
    let near: Vec<&Solid> = trees
        .iter()
        .filter(|solid| beside.contains(&(solid.low, solid.high)))
        .collect();
    assert!(
        near.len() * 4 >= trees.len() * 3,
        "{} of {}",
        near.len(),
        trees.len()
    );
    for solid in near {
        assert!(obstacles.contains(&declared(solid)), "{solid:?}");
    }
    // None of them cuts any of the ground the yard and the trail draw: each trunk stands off it by more than the
    // clearance, and each crown reaches over it only high enough overhead to leave its room clear. So the tree ferns'
    // solids keep the agent and panels clear without taking ground from anyone.
    let every: Vec<Obstacle> = trees.iter().map(declared).collect();
    let area = |mesh: &Navmesh| -> f32 {
        mesh.triangles()
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).y.abs() * 0.5)
            .sum()
    };
    let ground = crate::walkable::ground();
    let (bare, with) = (
        area(&Navmesh::derive(ground.clone(), &[])),
        area(&Navmesh::derive(ground, &every)),
    );
    assert!((bare - with).abs() < 1e-3, "{bare} {with}");
    // A ground fern is decoration, not solid: its fronds reach no nearer the walkable ground, which the tree ferns take
    // none of, than the clearance that keeps anyone standing on it from what stands about it, so nobody standing
    // anywhere participants may stands in one.
    for fern in standing_ferns()
        .iter()
        .filter(|fern| Kind::GROUND.contains(&fern.kind))
    {
        let (reach, _) = fern.size();
        for point in about(Vec2::new(fern.at.x, fern.at.z), reach + layout::CLEARANCE) {
            assert_eq!(crate::walkable::height_at(point), None, "{fern:?} {point}");
        }
    }
    // And no fern's foot is anywhere participants walk or land.
    for fern in ferns() {
        assert_eq!(
            crate::walkable::height_at(Vec2::new(fern.at.x, fern.at.z)),
            None,
            "{fern:?}"
        );
    }
}

#[test]
fn the_ferns_keep_the_views_the_status_wall_the_agent_and_the_anchors_rooms_open() {
    let solids: Vec<Solid> = standing_ferns().iter().flat_map(Fern::solids).collect();
    // Neither a tree fern nor a ground fern stands in a view the backdrop or the trail keeps open.
    for (from, to) in sights() {
        for solid in &solids {
            assert!(!solid.blocks(from, to), "{from:?} {to:?} {solid:?}");
        }
    }
    // From where participants arrive, the status wall on the notice board is noticed and the agent at its home seen and
    // its wall beneath it read, with the ferns as with the yard alone.
    let yard: Vec<Solid> = layout::solids()
        .into_iter()
        .map(|(low, high)| Solid::new(low, high))
        .chain(solids.iter().copied())
        .collect();
    let arrival = Spot::facing(
        layout::ARRIVAL.to_array(),
        layout::ARRIVAL_FACING.to_array(),
    );
    let board = Spot::facing(
        layout::NOTICEBOARD.to_array(),
        layout::NOTICEBOARD_FACING.to_array(),
    );
    assert_eq!(noticed(&arrival, &board, layout::ARRIVAL.y, &yard), Ok(()));
    let agent =
        tomori_protocol::Pose::at(layout::AGENT_HOME.to_array()).facing_towards(arrival.eyes());
    let wall = tomori_protocol::entourage::wall_beneath(&agent, layout::ARRIVAL.y);
    assert_eq!(
        seen(&arrival, &Spot::new(agent.position, agent.yaw()), &yard),
        Ok(())
    );
    assert_eq!(
        readable(&arrival, &Spot::new(wall.position, wall.yaw()), &yard),
        Ok(())
    );
    // One to eighteen workers stand at each of the trail's and the campsite's anchors clear of the tree ferns, which
    // keep 0.4 m from each anchor's point, or a metre below it.
    let trees = super::solids();
    assert!(trees.len() < solids.len());
    for cluster in clusters() {
        assert_eq!(
            tomori_environment::sightlines::fits(&cluster, trail::HEIGHT, &trees),
            Ok(())
        );
    }
    // And wherever anyone stands on the trail, the agent finds most ways round them clear of the tree ferns, or as
    // many as it found before them.
    let kept = kept();
    let with = [kept.clone(), trees.clone()].concat();
    for at in along_the_trail() {
        let (before, after) = (taken(at, &kept), taken(at, &with));
        let clear = |taken: u64| WAYS - taken.count_ones();
        assert!(clear(after) >= CLEAR_WAYS.min(clear(before)), "{at}");
    }
}

/// Every refusal of [`judged`], and a place it refuses a fern of a kind for it, as `(kind, point, reason)`, found by
/// trying every half metre about the trail, with the ferns already placed.
fn refusals() -> Vec<(Kind, Vec2, &'static str)> {
    let mut context = Context::new();
    let placed = ferns();
    for fern in placed.iter().filter(|fern| fern.kind.is_tree()) {
        let solids = fern.solids();
        for (at, before) in &mut context.standing {
            *before |= taken(*at, &solids);
        }
    }
    let mut found: Vec<(Kind, Vec2, &'static str)> = Vec::new();
    for kind in [Kind::Ponga, Kind::Wheki, Kind::Mamaku, Kind::CrownFern] {
        for x in -36..=36 {
            for z in -56..=20 {
                let point = Vec2::new(x as f32, z as f32) * 0.5;
                if let Err(reason) = judged(kind, 1.0, point, &context, &placed)
                    && !found.iter().any(|(_, _, seen)| *seen == reason)
                {
                    found.push((kind, point, reason));
                }
            }
        }
    }
    found
}

#[test]
fn every_rule_refuses_a_fern_somewhere() {
    let mut reasons: Vec<&str> = refusals().iter().map(|(_, _, reason)| *reason).collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            "agent",
            "anchor",
            "crowded",
            "fence or standing",
            "overhead",
            "rooms",
            "sights",
            "steep",
            "water, cliff or board",
            "what is there"
        ]
    );
    // And a fern fits where it was placed, as it was.
    let context = Context::new();
    let first = ferns()[0];
    let point = Vec2::new(first.at.x, first.at.z);
    let fern = fits(first.kind, first.scale, point, &context, &[]).unwrap();
    assert_eq!((fern.at, fern.scale), (first.at, first.scale));
}

/// Where participants' eyes may be: over the yard's walkable ground, at every corner of it, along the trail and its
/// clearings, about the campsite's fire and on the waterfall rock.
fn eyes() -> Vec<Vec3> {
    let yard = crate::walkable::navmesh()
        .triangles()
        .iter()
        .flatten()
        .map(|corner| Vec2::new(corner.x, corner.z))
        .filter(|corner| FENCE.contains(*corner));
    let campsite = campsite::standing()
        .into_iter()
        .flat_map(|(middle, radius)| about(middle, radius));
    let rock = layout::WATERFALL_ROCK;
    let terrace = crate::walkable::navmesh()
        .triangles()
        .iter()
        .flatten()
        .filter(|corner| corner.y > layout::TERRACE - 0.1)
        .map(|corner| *corner + Vec3::Y * EYES);
    yard.chain(along_the_trail())
        .chain(campsite)
        .map(|at| Vec3::new(at.x, EYES, at.y))
        .chain([rock + Vec3::Y * EYES])
        .chain(terrace)
        .collect()
}

#[test]
fn each_fern_is_many_long_lacy_fronds_swaying_at_their_tips_and_ponga_s_silver_beneath() {
    let brightness = |color: Vec3| color.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    for (kind, detail) in drawn() {
        let points = mesh(kind, detail);
        assert_eq!(points, mesh(kind, detail));
        assert_eq!(points.len() % 3, 0);
        let extent = kind.extent();
        for triangle in points.chunks(3) {
            // Each face flat, its normal its upper side's, the way its corners wind.
            let [a, b, c] = [0, 1, 2].map(|index| triangle[index].position);
            let normal = (b - a).cross(c - a).normalize();
            assert!(
                triangle
                    .iter()
                    .all(|point| point.normal == triangle[0].normal)
            );
            assert!(
                normal.dot(triangle[0].normal) > 0.999,
                "{kind:?} {triangle:?}"
            );
            for point in triangle {
                // Within its extent, drawn near, or within a few centimetres of it, drawn far with fewer segments, and
                // swaying no more than its kind may, a tree fern's trunk and the heart a ground fern springs from hardly
                // at all.
                let out = Vec2::new(point.position.x, point.position.z).length();
                let within = if detail == Detail::Near { 1e-4 } else { 0.05 };
                assert!(out <= extent.crown.0 + within, "{kind:?} {point:?}");
                assert!(
                    point.position.y <= extent.crown.2 + within,
                    "{kind:?} {point:?}"
                );
                assert!(
                    (0.0..=TREE_SWAY).contains(&point.sway),
                    "{kind:?} {point:?}"
                );
                if out < 0.1 && (kind.is_tree() || point.position.y < 0.05) {
                    assert!(point.sway < 0.005, "{kind:?} {point:?}");
                }
            }
        }
        if kind.is_tree() {
            // Its trunk slender, its crown a wide umbrella of green fronds, above its skirt of brown, dead ones, more
            // than twice as wide as it is deep, its fronds and skirt hanging below its heart, their tips swaying most.
            assert!(extent.trunk.0 < 0.35 && extent.trunk.1 == kind.heart().y);
            let (reach, low, _) = extent.crown;
            let fronds = points
                .iter()
                .filter(|point| point.color.y > point.color.x * 1.5)
                .map(|point| point.position.y);
            let (lowest, highest) = fronds.fold((f32::INFINITY, 0.0f32), |(low, high), y| {
                (low.min(y), high.max(y))
            });
            assert!(reach > highest - lowest, "{kind:?} {lowest} {highest}");
            assert!(low < kind.heart().y - 0.5, "{kind:?} {extent:?}");
            let tips = points.iter().filter(|point| {
                Vec2::new(point.position.x, point.position.z).length() > reach * 0.9
            });
            assert!(tips.into_iter().all(|point| point.sway > TREE_SWAY * 0.5));
        }
    }
    // Drawn far, each has half its triangles or fewer.
    for kind in Kind::TREES.iter().chain(&Kind::GROUND) {
        let [near, far] = [Detail::Near, Detail::Far].map(|detail| mesh(*kind, detail).len());
        assert!(far * 2 <= near + 3, "{kind:?} {near} {far}");
    }
    // Ponga's fronds are silver beneath, twice as bright as above and more; every other fern's a little paler beneath.
    for kind in Kind::TREES.iter().chain(&Kind::GROUND) {
        let fronds: Vec<Point> = mesh(*kind, Detail::Near)
            .into_iter()
            .filter(|point| point.sway > 0.02)
            .collect();
        let above: f32 = fronds.iter().map(|point| brightness(point.color)).sum();
        let beneath: f32 = fronds.iter().map(|point| brightness(point.underside)).sum();
        let silver = if *kind == Kind::Ponga { 2.0 } else { 1.0 };
        assert!(beneath > above * silver, "{kind:?} {above} {beneath}");
    }
    // A koru curls over from its stalk, out and down and back in, beneath its top.
    let koru = mesh(Kind::Koru, Detail::Near);
    let top = koru
        .iter()
        .map(|point| point.position.y)
        .fold(0.0, f32::max);
    assert!(koru.iter().any(|point| point.position.x > 0.08));
    assert!(
        koru.iter()
            .any(|point| point.position.x > 0.03 && point.position.y < top - 0.06)
    );
    // A polygon of fewer than three corners, once the repeated are dropped, is nothing.
    let mut builder = Builder::new(tree_sway, 1.0);
    builder.polygon(
        &[Vec3::ZERO, Vec3::ZERO, Vec3::X],
        Vec3::Y,
        (Vec3::ONE, Vec3::ONE),
    );
    assert!(builder.points.is_empty());
}

#[test]
fn the_ferns_are_modest_for_the_quest() {
    // Each mesh, drawn near and far: a tree fern a few hundred triangles near and a couple of hundred far, a ground fern
    // about ninety near and thirty far, and a koru eighteen, one draw for each with all its instances.
    let meshes: Vec<(Kind, Detail, usize)> = drawn()
        .into_iter()
        .map(|(kind, detail)| (kind, detail, mesh(kind, detail).len() / 3))
        .collect();
    assert_eq!(
        meshes,
        [
            (Kind::Ponga, Detail::Near, 491),
            (Kind::Ponga, Detail::Far, 214),
            (Kind::Mamaku, Detail::Near, 442),
            (Kind::Mamaku, Detail::Far, 193),
            (Kind::Wheki, Detail::Near, 371),
            (Kind::Wheki, Detail::Far, 147),
            (Kind::CrownFern, Detail::Near, 99),
            (Kind::CrownFern, Detail::Far, 35),
            (Kind::Kiokio, Detail::Near, 90),
            (Kind::Kiokio, Detail::Far, 28),
            (Kind::HenAndChickens, Detail::Near, 90),
            (Kind::HenAndChickens, Detail::Far, 28),
            (Kind::Koru, Detail::Near, 18),
        ],
        "the counts the documentation gives"
    );
    // A fern within its distance of the eyes is drawn near, and beyond it far.
    let fern = ferns()[0];
    let heart = fern.transform().transform_point3(fern.kind.heart());
    assert_eq!(fern.detail(heart + Vec3::X * (NEAR - 0.1)), Detail::Near);
    assert_eq!(fern.detail(heart + Vec3::X * (NEAR + 0.1)), Detail::Far);
    // From anywhere participants stand, every fern, the bush's tree ferns among them, each in its detail there, is
    // within the budget, though all of them near would be well over it.
    let most = eyes()
        .into_iter()
        .map(|eyes| triangles(Some(eyes)))
        .max()
        .unwrap();
    assert_eq!(most, 22_796, "the count the documentation gives");
    assert!(most <= BUDGET);
    assert_eq!(triangles(Some(layout::ARRIVAL + Vec3::Y * EYES)), 17_431);
    assert!(triangles(None) > BUDGET * 3 / 2);
    assert_eq!((ferns().len(), bush().len()), (152, 40));
}
