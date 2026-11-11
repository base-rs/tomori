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

use crate::{campsite, layout, props, trail};

/// What the farmyard draws as ground, which participants walk on: the yard's packed earth ([`props::ground`]), and the
/// trail's track from crest to crest and its clearings ([`trail::ground`]).
pub fn ground() -> Vec<[Vec3; 3]> {
    let mut ground = props::ground();
    ground.extend(trail::ground());
    ground
}

/// What stands on the farmyard's ground, which participants keep clear of: the yard's obstacles, the coop, its ramp,
/// the trough, the feeder, the hay, the notice board and the workbench, by [`layout::CLEARANCE`]; the fence, along its
/// posts, so that the earth stops [`layout::FENCE_CLEARANCE`] short of its line, and the gate's posts; and, beside the
/// trail, its stations' posts with their plaques, the lookout's seat and the waterfall rock, the plants, pines and tree
/// ferns' trunks and crowns beside it ([`trail::beside`], [`crate::ferns::solids`]) and the campsite's fire and seat,
/// each as tall as it stands, by [`layout::CLEARANCE`]. The ground ferns, decoration, are not among them, standing off
/// the ground by more than the clearance.
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
    obstacles.extend(
        trail::solids()
            .into_iter()
            .chain(trail::beside())
            .chain(campsite::solids())
            .map(|(low, high)| cuboid(Vec3::from(low), Vec3::from(high))),
    );
    obstacles
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
        assert_eq!(ground.len(), 2 + 3 * 2 * (line + 2) + 6 * 7);
        // None of it slopes more than the edges' crests, which rise 35 mm over 80 mm, and all of it is within the
        // crests' height of the earth.
        for [a, b, c] in &ground {
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
        // The yard's obstacles, the fence's five runs and the gate's two posts, and what stands beside the trail.
        let obstacles = obstacles();
        assert_eq!(
            obstacles.len(),
            layout::obstacles().len()
                + 5
                + 2
                + trail::solids().len()
                + trail::beside().len()
                + campsite::solids().len()
        );
    }
}
