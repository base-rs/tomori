// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where participants walk in the farmyard, derived from what it draws rather than laid out by hand: [`ground`], the
//! faces the yard and the trail draw as ground, less [`obstacles`], everything that stands on it, each grown by its
//! clearance, as [`navmesh`] derives it, once, at the heights it is drawn. So what participants land on is what is
//! drawn there, and nothing that stands in the yard or beside the trail can be landed on, however the layout changes.
//! The client crate's teleport profile declares it, with the way out through the gate ([`crate::trail::way_out`]) and
//! the places to stand about the campsite's fire ([`crate::campsite::standing`]). See Tomori's `design/locomotion.md`.
use glam::{Vec2, Vec3};
use tomori_environment::navmesh::{Navmesh, Obstacle};

use crate::{backdrop, beach, campsite, layout, props, terrace, trail};

/// What the farmyard draws as ground, which participants walk on: the yard's packed earth ([`props::ground`]), the
/// trail's track from crest to crest and its clearings ([`trail::ground`]), the beach's shingle down towards the
/// river ([`beach::ground`]), the upper terrace's floor, at the top of the falls ([`terrace::ground`]), a region of
/// its own, joined to none of the rest, and the campsite's clearing all the way round its fire ([`campsite::ground`]).
pub fn ground() -> Vec<[Vec3; 3]> {
    let mut ground = props::ground();
    ground.extend(trail::ground());
    ground.extend(beach::ground());
    ground.extend(terrace::ground());
    ground.extend(campsite::ground());
    ground
}

/// What stands on the farmyard's ground, which participants keep clear of: the yard's obstacles, the coop, its ramp,
/// the trough, the feeder, the hay, the notice board and the workbench, by [`layout::CLEARANCE`]; the fence, along its
/// posts, so that the earth stops [`layout::FENCE_CLEARANCE`] short of its line, and the gate's posts; and, beside the
/// trail, its stations' posts with their plaques, the lookout's seat and the waterfall rock, the plants, pines and tree
/// ferns' trunks and crowns beside it ([`beside`]), the campsite's ring of stones
/// about its fire, by its footprint ([`campsite::pit`]), so that participants walk round it, and what stands about it, and
/// the river's stones on the beach, and on the upper terrace its stones, flat rock, cairn and ferns ([`terrace::solids`]),
/// each as tall as it stands, by [`layout::CLEARANCE`]. The ground ferns beside the trail, decoration, are not among
/// them, standing off the ground by more than the clearance.
pub fn obstacles() -> Vec<Obstacle> {
    let cuboid =
        |low: Vec3, high: Vec3| Obstacle::cuboid(low, high).with_clearance(layout::CLEARANCE);
    let standing = |footprint: layout::Footprint, height: f32| {
        cuboid(
            Vec3::new(footprint.low.x, 0.0, footprint.low.y),
            Vec3::new(footprint.high.x, height, footprint.high.y),
        )
    };
    let yard = layout::obstacles();
    // How tall each of the yard's obstacles stands, in order, then the workbench's pieces, with its rack.
    let heights = [
        layout::COOP_RIDGE,
        layout::COOP_FLOOR,
        layout::TROUGH_HEIGHT,
        layout::FEEDER_HEIGHT,
        layout::HAY_HEIGHT,
        layout::NOTICEBOARD_HEIGHT,
    ];
    let mut obstacles: Vec<Obstacle> = yard
        .iter()
        .enumerate()
        .map(|(index, footprint)| {
            standing(
                *footprint,
                heights.get(index).copied().unwrap_or(layout::RACK_HEIGHT),
            )
        })
        .collect();
    // The fence's runs, along its posts, and the gate's posts.
    let fence = layout::FENCE;
    let post = layout::FENCE_POST * 0.5;
    let runs = [
        ([fence.low.x, fence.low.y], [fence.low.x, fence.high.y]),
        ([fence.high.x, fence.low.y], [fence.high.x, fence.high.y]),
        ([fence.low.x, fence.low.y], [fence.high.x, fence.low.y]),
        ([fence.low.x, fence.high.y], [layout::GATE[0], fence.high.y]),
        (
            [layout::GATE[1], fence.high.y],
            [fence.high.x, fence.high.y],
        ),
    ];
    obstacles.extend(runs.into_iter().map(|(from, to)| {
        let (from, to) = (Vec2::from(from), Vec2::from(to));
        let (low, high) = (from.min(to) - post, from.max(to) + post);
        Obstacle::cuboid(
            Vec3::new(low.x, 0.0, low.y),
            Vec3::new(high.x, layout::FENCE_HEIGHT, high.y),
        )
        .with_clearance(layout::FENCE_CLEARANCE - post)
    }));
    let gate = layout::GATE_POST * 0.5;
    obstacles.extend(layout::GATE.map(|x| {
        cuboid(
            Vec3::new(x - gate, 0.0, fence.high.y - gate),
            Vec3::new(x + gate, layout::GATE_HEIGHT, fence.high.y + gate),
        )
    }));
    obstacles.push(
        Obstacle::footprint(campsite::pit(), 0.0, campsite::FLAMES)
            .with_clearance(layout::CLEARANCE),
    );
    obstacles.extend(
        trail::solids()
            .into_iter()
            .chain(beside())
            .chain(campsite::furniture())
            .chain(beach::solids())
            .chain(terrace::solids())
            .map(|(low, high)| cuboid(Vec3::from(low), Vec3::from(high))),
    );
    obstacles
}

/// What stands beside the trail, as [`trail::beside`] says, but the bush's tree ferns by their trunks and crowns
/// ([`crate::ferns::bush`]), as the trail's own are, rather than the box about the whole of each plant that the agent
/// keeps clear of: so that ground beneath a crown hanging higher than anyone stands, as a ponga's over the campsite's
/// clearing, is walked.
pub fn beside() -> Vec<([f32; 3], [f32; 3])> {
    let beside = trail::beside();
    let plants: Vec<([f32; 3], [f32; 3])> = backdrop::plants()
        .iter()
        .filter(|plant| plant.kind == backdrop::Kind::Fern)
        .map(|plant| {
            let solid = plant.solid();
            (solid.low, solid.high)
        })
        .collect();
    let ferns = plants
        .iter()
        .zip(crate::ferns::bush())
        .filter(|(plant, _)| beside.contains(plant))
        .flat_map(|(_, fern)| fern.solids())
        .map(|solid| (solid.low, solid.high));
    beside
        .iter()
        .filter(|solid| !plants.contains(solid))
        .copied()
        .chain(ferns)
        .collect()
}

/// The farmyard's walkable ground: [`ground`] less [`obstacles`], derived once and kept, as the farmyard's layout and
/// models never change while it runs.
pub fn navmesh() -> &'static Navmesh {
    static NAVMESH: std::sync::OnceLock<Navmesh> = std::sync::OnceLock::new();
    NAVMESH.get_or_init(|| Navmesh::derive(ground(), &obstacles()))
}

/// Whether `point`, on the ground, X and Z, is walkable, and how high its floor is there.
pub fn height_at(point: Vec2) -> Option<f32> {
    navmesh().height_at(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ground_is_what_the_yard_and_the_trail_draw_as_ground_less_what_stands_on_it() {
        let ground = ground();
        // The yard's earth, two triangles, and the trail's track from crest to crest, three strips of two each between
        // each of its points, round the loop and out through the gate, and its clearings, each of nine sides.
        let line = trail::centreline().len();
        let shingle = beach::ground();
        let floor = terrace::ground();
        let camp = campsite::ground();
        assert_eq!(
            ground.len(),
            2 + 3 * 2 * (line + 2) + 6 * 7 + shingle.len() + floor.len() + camp.len()
        );
        // None of it slopes more than the edges' crests, which rise 35 mm over 80 mm, and all of it is within the
        // crests' height of the earth, but the beach's shingle, which slopes down towards the river, the upper
        // terrace's floor, level at its own height, above the waterfall, and the campsite's clearing, which rises a few
        // centimetres more towards the pool, gently.
        let below = ground.len() - shingle.len() - floor.len() - camp.len();
        assert_eq!(ground[ground.len() - camp.len()..], camp[..]);
        for [a, b, c] in &camp {
            let normal = (*b - *a).cross(*c - *a).normalize();
            assert!(normal.y.abs() > 0.99, "{a} {b} {c}");
            assert!(
                [a, b, c]
                    .iter()
                    .all(|corner| (0.0..0.06).contains(&corner.y))
            );
        }
        for [a, b, c] in &ground[..below] {
            let normal = (*b - *a).cross(*c - *a).normalize();
            assert!(normal.y.abs() > 0.9, "{a} {b} {c}");
            for corner in [a, b, c] {
                assert!(
                    (0.0..=trail::LIFT + trail::EDGE).contains(&corner.y),
                    "{corner}"
                );
            }
        }
        let mesh = navmesh();
        assert!(!mesh.is_empty() && mesh.triangles().len() < 4000);
        // Where participants arrive, on the earth; on the trail's track; in a clearing; and nowhere in the coop, on the
        // fence, beyond the yard's earth on the meadow or on the waterfall rock.
        assert_eq!(
            height_at(Vec2::new(layout::ARRIVAL.x, layout::ARRIVAL.z)),
            Some(0.0)
        );
        let (point, _, _) = trail::frame(8);
        assert_eq!(height_at(point), Some(trail::HEIGHT));
        let (clearing, _) = trail::clearings()[5];
        assert_eq!(height_at(clearing), Some(trail::CLEARING_HEIGHT));
        for nowhere in [
            layout::COOP.centre(),
            Vec2::new(layout::FENCE.high.x, -4.0),
            Vec2::new(7.0, 0.0),
            Vec2::new(layout::WATERFALL_ROCK.x, layout::WATERFALL_ROCK.z),
        ] {
            assert_eq!(height_at(nowhere), None, "{nowhere}");
        }
        // The yard's obstacles, the fence's five runs and the gate's two posts, the campsite's ring of stones, and what
        // stands beside the trail.
        let obstacles = obstacles();
        assert_eq!(
            obstacles.len(),
            layout::obstacles().len()
                + 5
                + 2
                + 1
                + trail::solids().len()
                + beside().len()
                + campsite::furniture().len()
                + beach::solids().len()
                + terrace::solids().len()
        );
        let terrace = ground.len() - camp.len();
        assert!(
            ground[terrace - floor.len()..terrace]
                .iter()
                .flatten()
                .all(|corner| corner.y == layout::TERRACE)
        );
        let floor = height_at(terrace::on_fall(26.0, 5.5)).unwrap();
        assert!((floor - layout::TERRACE).abs() < 1e-5, "{floor}");
        // The beach is walked on its shingle, as high as it is drawn.
        let on = beach::at(beach::NEAR + 1.0, 0.3);
        let walked = height_at(on).unwrap();
        assert!(
            (walked - beach::height(on).unwrap()).abs() < 1e-3,
            "{walked}"
        );
    }

    /// The middle of the campsite's fire, on the ground.
    fn fire() -> Vec2 {
        Vec2::new(campsite::FIRE.x, campsite::FIRE.z)
    }

    #[test]
    fn participants_walk_all_the_way_round_the_campsite_s_fire_but_never_into_it() {
        let fire = fire();
        // A ring about the fire, between its stones and what stands about it, walkable at every degree of the way round,
        // a step from one to the next, on the clearing as drawn, or the trail's track where it passes, joined to the
        // trail.
        let ring = |radius: f32| {
            (0..360).map(move |degree| {
                let turn = (degree as f32).to_radians();
                fire + Vec2::new(turn.sin(), turn.cos()) * radius
            })
        };
        for radius in [0.85, 0.92, 1.0] {
            for point in ring(radius) {
                let height = height_at(point);
                assert!(
                    height.is_some_and(|height| (0.0..0.06).contains(&height)),
                    "{radius} {point} {height:?}"
                );
            }
        }
        let (nearest, _, _) = (0..trail::centreline().len())
            .map(trail::frame)
            .min_by(|a, b| a.0.distance(fire).total_cmp(&b.0.distance(fire)))
            .unwrap();
        let towards = (nearest - fire).normalize();
        let way = nearest.distance(fire) - 0.95;
        for step in 0..=(way / 0.02) as usize {
            let point = fire + towards * (0.95 + step as f32 * 0.02);
            assert!(height_at(point).is_some(), "{point}");
        }
        // Never among its stones, let alone its flames, nor beyond its stones within the clearance kept from them.
        let stones = campsite::pit()
            .into_iter()
            .map(|corner| corner.distance(fire))
            .fold(0.0, f32::max);
        for radius in [0.0, 0.2, campsite::PIT, stones, campsite::PIT + 0.25] {
            assert!(
                ring(radius).all(|point| height_at(point).is_none()),
                "{radius}"
            );
        }
        // Nor on the seat across it, nor among the sticks leaning on it, nor beyond the clearing towards the pool, where
        // the far side of the backdrop would be beyond the far plane.
        let (seat, ..) = campsite::seat();
        assert_eq!(height_at(Vec2::new(seat.x, seat.z)), None);
        let pose = campsite::Pose::racked(0);
        assert_eq!(height_at(Vec2::new(pose.tip().x, pose.tip().z)), None);
        let away = (fire - layout::BACKDROP_CENTRE).normalize();
        let beyond =
            fire + away * (campsite::REACH - fire.distance(layout::BACKDROP_CENTRE) + 0.05);
        assert_eq!(height_at(beyond), None);
        let clearing = navmesh()
            .triangles()
            .iter()
            .flatten()
            .map(|corner| Vec2::new(corner.x, corner.z))
            .filter(|corner| corner.distance(fire) < campsite::CLEARING + 0.01)
            .filter(|corner| trail::from_middle(*corner) > trail::HALF + 0.2);
        for corner in clearing {
            let reach = corner.distance(layout::BACKDROP_CENTRE);
            assert!(reach < campsite::REACH + 1e-3, "{corner}");
        }
    }
}
