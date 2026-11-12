// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The chicken coop: a hen house raised on legs, painted barn red, with a pitched roof, a little door at the top of a
//! cleated ramp, a window in its side to peek in at the nest boxes and the perch, and a lantern by the door.
//!
//! It is a model of its own, in its own space: metres, the ground beneath the middle of its footprint at the origin, its
//! door facing +Z. The farmyard places it on [`layout::COOP`]. It is built here, in code, until a coop modelled in Blender
//! replaces it: any `.glb` with the same footprint, door and ramp can, through [`Coop::Asset`], loaded and checked as
//! every asset is. See the crate's `context/coop.md`.
use glam::{Quat, Vec3};
use tomori_assets::{AssetRef, Library, Limits, Mesh, Model, Node, Transform};

use crate::layout;
use crate::props::{GLASS, PLAIN, lantern, materials, palette};
use crate::shapes::Shapes;

/// The coop the farmyard draws.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Coop {
    /// The coop built here, in code.
    #[default]
    Built,
    /// A coop modelled elsewhere, loaded from the program's assets; the built one stands in if it cannot be.
    Asset(AssetRef),
}

impl Coop {
    /// The coop's model: the asset's, loaded from `library`, or the one built here if there is no asset or it cannot be
    /// loaded, saying why in the log.
    pub fn model(&self, library: &Library) -> std::sync::Arc<Model> {
        match self {
            Self::Asset(asset) => library.model(asset).unwrap_or_else(|error| {
                log::warn!(
                    "The farmyard's coop could not be loaded, so it is built instead: {error}"
                );
                std::sync::Arc::new(model())
            }),
            Self::Built => std::sync::Arc::new(model()),
        }
    }
}

/// Where the coop stands in the farmyard: its model's space, placed on [`layout::COOP`].
pub fn placement() -> glam::Mat4 {
    let centre = layout::COOP.centre();
    glam::Mat4::from_translation(Vec3::new(centre.x, 0.0, centre.y))
}

/// A point of the farmyard, in the coop's own space.
fn local(point: Vec3) -> Vec3 {
    placement().inverse().transform_point3(point)
}

/// How thick the coop's walls are, and the dark lining inside its far side, its back and its ceiling.
const WALL: f32 = 0.05;
const LINING: f32 = 0.012;
/// How far over the walls the roof reaches on every side.
const OVERHANG: f32 = 0.18;

/// Half the coop's footprint, across and from front to back, in its own space.
fn half() -> (f32, f32) {
    let size = layout::COOP.size();
    (size.x * 0.5, size.y * 0.5)
}

/// The roof's two pitched panels, each a box: its middle, half its size and how it is turned.
fn roof() -> [(Vec3, Vec3, Quat); 2] {
    let (half_x, half_z) = half();
    let (walls, ridge) = (layout::COOP_WALLS, layout::COOP_RIDGE);
    let pitch = ((ridge - walls) / half_x).atan();
    let run = (half_x + OVERHANG) / pitch.cos();
    [-1.0f32, 1.0].map(|side| {
        let middle = Vec3::new(
            side * (half_x + OVERHANG) * 0.5,
            (ridge + walls - OVERHANG * pitch.tan()) * 0.5,
            0.0,
        );
        (
            middle + Vec3::Y * 0.03,
            Vec3::new(run * 0.5, 0.03, half_z + OVERHANG),
            Quat::from_rotation_z(-side * pitch),
        )
    })
}

/// The perch, a pole from the lining of the far side to the near side, its ends on cleats on both: in front of the window,
/// towards the door, where the near side is a solid wall, above the door's head, so that a hen walking in passes under
/// it. Its two ends, at its middle, in the coop's space, and how thick it is.
fn perch() -> (Vec3, Vec3, f32) {
    let (half_x, _) = half();
    let (y, z) = (layout::COOP_FLOOR + 0.55, 0.5);
    (
        Vec3::new(-(half_x - WALL - LINING), y, z),
        Vec3::new(half_x - WALL, y, z),
        0.05,
    )
}

/// The lantern's bracket: an arm out from the front wall, beneath the roof, and the hook down from it to the lantern's
/// cap, each a beam, from and to, and how thick.
fn bracket() -> [(Vec3, Vec3, f32); 2] {
    let (_, half_z) = half();
    let hung = local(layout::LANTERN);
    let arm = hung.y + 0.21;
    [
        (
            Vec3::new(hung.x, arm, half_z),
            Vec3::new(hung.x, arm, hung.z + 0.015),
            0.03,
        ),
        (
            Vec3::new(hung.x, arm, hung.z),
            Vec3::new(hung.x, hung.y + 0.15, hung.z),
            0.015,
        ),
    ]
}

/// The ramp's board, its top running from the door's sill straight down to the ground at its foot, as
/// [`layout::ramp_height`] says, and its end cut to sit flat on the ground there: its side at its left edge, from the
/// sill down its top to its foot, back along the ground and up its underside to beneath the sill; the way across it to
/// its right edge; and how it slopes, +Z turned down it and +Y up out of its top.
fn ramp() -> ([Vec3; 4], Vec3, Quat) {
    let (_, half_z) = half();
    let foot = local(layout::RAMP_FOOT);
    let sill = Vec3::new(local(layout::DOOR).x, layout::COOP_FLOOR, half_z);
    let along = foot - sill;
    let turn = Quat::from_rotation_arc(Vec3::Z, along.normalize());
    let under = sill - turn * Vec3::Y * 0.04;
    let ground = under + along * (-under.y / along.y);
    let left = Vec3::X * layout::RAMP_WIDTH * -0.5;
    (
        [sill, foot, ground, under].map(|corner| corner + left),
        Vec3::X * layout::RAMP_WIDTH,
        turn,
    )
}

/// A nest box, against the back wall, about `inside`, its inside in the yard: its bottom, its back and sides, and a low
/// lip along its front, open above and in front for a hen to walk in over the lip, with straw on its bottom and a few
/// strands over its lip.
fn nest(shapes: &mut Shapes, inside: layout::Footprint) {
    let low = local(Vec3::new(inside.low.x, 0.0, inside.low.y));
    let high = local(Vec3::new(inside.high.x, 0.0, inside.high.y));
    let board = layout::NEST_BOARD;
    let (floor, bottom, lip, top) = (
        layout::COOP_FLOOR,
        layout::NEST_FLOOR,
        layout::NEST_LIP,
        layout::NEST_WALLS,
    );
    for (from, to, color) in [
        // Its bottom, beneath its boards.
        (
            Vec3::new(low.x - board, floor, low.z - board),
            Vec3::new(high.x + board, bottom, high.z + board),
            palette::DARK_WOOD,
        ),
        // Its back and its sides.
        (
            Vec3::new(low.x - board, bottom, low.z - board),
            Vec3::new(high.x + board, top, low.z),
            palette::WOOD,
        ),
        (
            Vec3::new(low.x - board, bottom, low.z),
            Vec3::new(low.x, top, high.z + board),
            palette::WOOD,
        ),
        (
            Vec3::new(high.x, bottom, low.z),
            Vec3::new(high.x + board, top, high.z + board),
            palette::WOOD,
        ),
        // Its lip, low, between its sides.
        (
            Vec3::new(low.x, bottom, high.z),
            Vec3::new(high.x, lip, high.z + board),
            palette::WOOD,
        ),
        // Straw on its bottom, which a hen in it stands in.
        (
            Vec3::new(low.x + 0.015, bottom, low.z + 0.015),
            Vec3::new(high.x - 0.015, bottom + 0.012, high.z - 0.01),
            palette::STRAW,
        ),
    ] {
        shapes.block(from, to, color);
    }
    let middle = (low.x + high.x) * 0.5;
    for (across, turn) in [(-0.08, 0.5), (0.1, -0.35)] {
        shapes.cuboid(
            Vec3::new(middle + across, lip + 0.004, high.z + board * 0.5),
            Vec3::new(0.006, 0.004, 0.035),
            Quat::from_rotation_y(turn),
            palette::STRAW,
        );
    }
}

/// The coop built in code.
pub fn model() -> Model {
    let mut shapes = Shapes::new();
    let mut glass = Shapes::new();
    let (half_x, half_z) = half();
    let (floor, walls, ridge) = (layout::COOP_FLOOR, layout::COOP_WALLS, layout::COOP_RIDGE);
    let wall = WALL;
    let door = local(layout::DOOR);
    let half_door = layout::DOOR_SIZE.x * 0.5;
    let door_top = floor + layout::DOOR_SIZE.y;
    // Legs, and the floor they hold up, into which they reach.
    for x in [-half_x + 0.08, half_x - 0.08] {
        for z in [-half_z + 0.08, half_z - 0.08] {
            shapes.block(
                Vec3::new(x - 0.06, 0.0, z - 0.06),
                Vec3::new(x + 0.06, floor - 0.03, z + 0.06),
                palette::DARK_WOOD,
            );
        }
    }
    shapes.block(
        Vec3::new(-half_x, floor - 0.06, -half_z),
        Vec3::new(half_x, floor, half_z),
        palette::WOOD,
    );
    // The front wall, around the door, and its gable.
    let front = half_z - wall;
    let panel = |shapes: &mut Shapes, x: [f32; 2], y: [f32; 2], z: [f32; 2]| {
        shapes.block(
            Vec3::new(x[0], y[0], z[0]),
            Vec3::new(x[1], y[1], z[1]),
            palette::RED,
        );
    };
    panel(
        &mut shapes,
        [-half_x, door.x - half_door],
        [floor, walls],
        [front, half_z],
    );
    panel(
        &mut shapes,
        [door.x + half_door, half_x],
        [floor, walls],
        [front, half_z],
    );
    panel(
        &mut shapes,
        [door.x - half_door, door.x + half_door],
        [door_top, walls],
        [front, half_z],
    );
    // The back wall.
    panel(
        &mut shapes,
        [-half_x, half_x],
        [floor, walls],
        [-half_z, -front],
    );
    // The far side, and the near side around its window.
    panel(
        &mut shapes,
        [-half_x, -half_x + wall],
        [floor, walls],
        [-half_z, half_z],
    );
    let window = layout::WINDOW.map(|[z, y]| (local(Vec3::new(0.0, y, z)).z, y));
    let side = [half_x - wall, half_x];
    panel(&mut shapes, side, [floor, window[0].1], [-half_z, half_z]);
    panel(&mut shapes, side, [window[1].1, walls], [-half_z, half_z]);
    panel(
        &mut shapes,
        side,
        [window[0].1, window[1].1],
        [-half_z, window[0].0],
    );
    panel(
        &mut shapes,
        side,
        [window[0].1, window[1].1],
        [window[1].0, half_z],
    );
    // The gables, front and back.
    for z in [half_z, -half_z] {
        let facing = z.signum();
        let corners = [
            Vec3::new(-half_x * facing, walls, z),
            Vec3::new(half_x * facing, walls, z),
            Vec3::new(0.0, ridge, z),
        ];
        shapes.polygon(&corners, palette::RED * 0.9);
    }
    // White trim at the corners, around the door and under the window.
    for x in [-half_x, half_x] {
        for z in [-half_z, half_z] {
            shapes.block(
                Vec3::new(x - 0.03, floor, z - 0.03),
                Vec3::new(x + 0.03, walls, z + 0.03),
                palette::WEATHERED,
            );
        }
    }
    shapes.block(
        Vec3::new(door.x - half_door - 0.04, door_top, half_z),
        Vec3::new(door.x + half_door + 0.04, door_top + 0.05, half_z + 0.02),
        palette::WEATHERED,
    );
    shapes.block(
        Vec3::new(half_x, window[0].1 - 0.04, window[0].0 - 0.04),
        Vec3::new(half_x + 0.04, window[0].1, window[1].0 + 0.04),
        palette::WEATHERED,
    );
    // The roof: two pitched panels overhanging every side.
    for (middle, half, turn) in roof() {
        shapes.cuboid(middle, half, turn, palette::ROOF);
    }
    shapes.beam(
        Vec3::new(0.0, ridge + 0.04, -half_z - OVERHANG),
        Vec3::new(0.0, ridge + 0.04, half_z + OVERHANG),
        0.07,
        palette::DARK_WOOD,
    );
    // Inside, seen through the window and the door, in the shade: a lining of dark boards, which the lantern outside
    // hardly reaches, straw on the floor, two nest boxes along the back, and a perch.
    let lining = LINING;
    let (inner_x, inner_z) = (half_x - wall - lining, half_z - wall - lining);
    for (low, high) in [
        (
            Vec3::new(-inner_x - lining, floor, -inner_z),
            Vec3::new(-inner_x, walls, inner_z),
        ),
        (
            Vec3::new(-inner_x, floor, -inner_z - lining),
            Vec3::new(inner_x, walls, -inner_z),
        ),
        (
            Vec3::new(-inner_x, walls - 0.02, -inner_z),
            Vec3::new(inner_x, walls, inner_z),
        ),
    ] {
        shapes.block(low, high, palette::SHADOW * 2.0);
    }
    // Loose straw, thin, over the floor, which a hen walking in stands in.
    shapes.block(
        Vec3::new(-half_x + wall, floor, -half_z + wall),
        Vec3::new(half_x - wall, floor + 0.008, half_z - wall),
        palette::STRAW * 0.45,
    );
    for inside in layout::NESTS {
        nest(&mut shapes, inside);
    }
    // The perch, on a cleat at either end.
    let (from, to, thickness) = perch();
    shapes.beam(from, to, thickness, palette::WOOD);
    for (end, inward) in [(from, 1.0f32), (to, -1.0)] {
        let under = end.y - thickness * 0.5;
        shapes.block(
            Vec3::new(end.x.min(end.x + inward * 0.06), under - 0.05, end.z - 0.04),
            Vec3::new(end.x.max(end.x + inward * 0.06), under, end.z + 0.04),
            palette::DARK_WOOD,
        );
    }
    // The door, swung open beside its opening, and the ramp down from its sill, with low cleats to grip.
    shapes.block(
        Vec3::new(door.x + half_door + 0.02, floor + 0.02, half_z + 0.01),
        Vec3::new(door.x + half_door + 0.32, door_top - 0.02, half_z + 0.04),
        palette::RED * 0.8,
    );
    let (board, across, turn) = ramp();
    shapes.prism(&board, across, palette::WOOD);
    let (sill, foot) = (board[0] + across * 0.5, board[1] + across * 0.5);
    let cleats = (sill.distance(foot) / 0.22) as u32;
    for cleat in 1..cleats {
        shapes.cuboid(
            sill.lerp(foot, cleat as f32 / cleats as f32) + turn * Vec3::Y * 0.008,
            Vec3::new(across.x * 0.5, 0.008, 0.015),
            turn,
            palette::DARK_WOOD,
        );
    }
    // The lantern by the door, hung from a bracket on the wall beneath the roof's overhang.
    let hung = local(layout::LANTERN);
    let [arm, hook] = bracket();
    shapes.block(
        Vec3::new(arm.0.x - 0.04, arm.0.y - 0.05, half_z),
        Vec3::new(arm.0.x + 0.04, arm.0.y + 0.05, half_z + 0.015),
        palette::DARK_WOOD,
    );
    for (from, to, thickness) in [arm, hook] {
        shapes.beam(from, to, thickness, palette::METAL);
    }
    lantern(&mut shapes, &mut glass, hung);
    Model::built(
        vec![Node::new("coop", Transform::IDENTITY).with_mesh(0)],
        vec![0],
        vec![Mesh::new(
            "coop",
            vec![shapes.primitive(PLAIN), glass.primitive(GLASS)],
        )],
        materials(),
        &Limits::default(),
    )
    .expect("the coop is built within the limits")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_assets::{Digest, Embedded};

    #[test]
    fn the_built_coop_fits_its_footprint_and_reaches_its_ramp() {
        let model = model();
        let (low, high) = model.meshes[0].primitives[0].bounds();
        let placed = |point: Vec3| placement().transform_point3(point);
        let (low, high) = (placed(low), placed(high));
        // Its roof overhangs, and its ramp reaches the ground in front of its door.
        assert!(low.x >= layout::COOP.low.x - 0.25 && high.x <= layout::COOP.high.x + 0.25);
        assert!((high.z - layout::RAMP_FOOT.z).abs() < 0.1);
        assert!(low.y.abs() < 1e-3 && (high.y - layout::COOP_RIDGE).abs() < 0.15);
        assert!(local(layout::DOOR).z > 0.0);
        // Low-poly, in 722 triangles, its lantern's glass among them.
        let triangles: usize = model.meshes[0]
            .primitives
            .iter()
            .map(|primitive| primitive.indices.len() / 3)
            .sum();
        assert_eq!(triangles, 722, "the count the documentation gives");
    }

    #[test]
    fn the_perch_runs_from_wall_to_wall_beside_the_window_above_the_door_s_head() {
        let (from, to, thickness) = perch();
        let (half_x, half_z) = half();
        // Its ends meet the far side's lining and the near side's wall, where they face in.
        assert_eq!(from.x, -(half_x - WALL - LINING));
        assert_eq!(to.x, half_x - WALL);
        assert_eq!((from.y, from.z), (to.y, to.z));
        // The near side is solid wall where it meets it, beside the window, not across its opening.
        let window = layout::WINDOW.map(|[z, y]| (local(Vec3::new(0.0, y, z)).z, y));
        // Towards the door from the window, by 5 cm, and short of the front wall's lining.
        let (near, far) = (to.z - thickness * 0.5, to.z + thickness * 0.5);
        assert!(near > window[1].0 + 0.05, "{to}");
        assert!(far < half_z - WALL - LINING);
        // High enough that a hen with her cone walks in beneath it, and below the walls' top.
        let under = from.y - thickness * 0.5;
        assert!(under > layout::COOP_FLOOR + layout::DOOR_SIZE.y + 0.1);
        assert!(under > layout::COOP_FLOOR + crate::chicken::HEIGHT + 0.1);
        assert!(from.y + thickness < layout::COOP_WALLS);
    }

    #[test]
    fn the_lantern_hangs_clear_of_the_roof_lighting_the_door() {
        // Every corner of the lantern and its bracket is beneath the roof's panels by 5 cm, or beyond them.
        let hung = local(layout::LANTERN);
        let mut corners: Vec<Vec3> = [-1.0f32, 1.0]
            .into_iter()
            .flat_map(|x| [-0.14f32, 0.15].map(move |y| (x, y)))
            .flat_map(|(x, y)| [-1.0f32, 1.0].map(move |z| hung + Vec3::new(x * 0.09, y, z * 0.09)))
            .collect();
        for (from, to, thickness) in bracket() {
            for end in [from, to] {
                corners.extend(
                    [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z]
                        .map(|side| end + side * thickness * 0.5),
                );
            }
        }
        for (middle, half, turn) in roof() {
            for corner in &corners {
                // How far it is beneath the panel, by more than 5 cm, or beyond its edges, if it is.
                let within = turn.inverse() * (*corner - middle);
                let clear = (-half.y - 0.05 - within.y)
                    .max(within.x.abs() - half.x)
                    .max(within.z.abs() - half.z);
                assert!(clear > 0.0, "{corner} {within}");
            }
        }
        // It hangs in front of the wall, beneath the overhang, beside the door and a little above its head.
        let (_, half_z) = half();
        assert!(hung.z - 0.09 > half_z && hung.z < half_z + OVERHANG);
        let head = layout::DOOR + Vec3::Y * layout::DOOR_SIZE.y;
        let distance = layout::LANTERN.distance(head);
        assert!(distance < 0.9, "{distance}");
        assert!(hung.y - 0.14 > head.y);
        // Its bracket's arm reaches it from the wall.
        let [arm, hook] = bracket();
        assert_eq!(arm.0.z, half_z);
        assert!((hook.1.y - (hung.y + 0.15)).abs() < 1e-6);
    }

    #[test]
    fn the_ramp_s_board_is_where_the_chickens_walk_and_sits_on_the_ground() {
        let (board, across, _) = ramp();
        // Its top, at both edges, from the sill to its foot, is the height the chickens walk at.
        for edge in [Vec3::ZERO, across] {
            for corner in &board[..2] {
                let at = placement().transform_point3(*corner + edge);
                assert!((at.y - layout::ramp_height(at.z)).abs() < 1e-5, "{at}");
            }
        }
        // Its foot is on the ground, cut flat along it, and its top meets the coop's floor at the sill.
        assert!(board[1].y.abs() < 1e-6 && board[2].y.abs() < 1e-6);
        assert!(board[2].z < board[1].z);
        assert_eq!(board[0].y, layout::COOP_FLOOR);
    }

    #[test]
    fn the_nest_boxes_stand_open_against_the_back_wall_inside() {
        let (half_x, half_z) = half();
        let board = layout::NEST_BOARD;
        for (index, inside) in layout::NESTS.iter().enumerate() {
            let low = local(Vec3::new(inside.low.x, 0.0, inside.low.y));
            let high = local(Vec3::new(inside.high.x, 0.0, inside.high.y));
            // Within the coop, against its back lining, clear of its sides and of the other box.
            assert!(low.x - board > -(half_x - WALL - LINING) && high.x + board < half_x - WALL);
            let back = low.z - board - -(half_z - WALL - LINING);
            assert!((0.0..0.01).contains(&back), "{back}");
            for other in &layout::NESTS[index + 1..] {
                assert!(!inside.grown(board).overlaps(&other.grown(board)));
            }
            // A hen walks in over its low lip and stands on its bottom, its sides and back taller than the lip.
            let middle = inside.centre();
            assert_eq!(layout::nest_height(middle), Some(layout::NEST_FLOOR));
            let lip = glam::Vec2::new(middle.x, inside.high.y + board * 0.5);
            assert_eq!(layout::nest_height(lip), Some(layout::NEST_LIP));
            assert_eq!(
                layout::underfoot(lip + glam::Vec2::Y * 0.05),
                layout::COOP_FLOOR
            );
            const {
                assert!(layout::NEST_FLOOR > layout::COOP_FLOOR);
                assert!(layout::NEST_LIP < layout::NEST_WALLS - 0.15);
            }
        }
        // The window looks in at them, and at the perch.
        let window = layout::WINDOW;
        assert!(layout::NESTS[1].high.y > window[0][0] && layout::NESTS[1].low.y < window[1][0]);
    }

    #[test]
    fn a_coop_from_an_asset_replaces_the_built_one_unless_it_cannot_be_loaded() {
        log::set_max_level(log::LevelFilter::Trace);
        let built = Coop::default().model(Library::shared());
        assert_eq!(*built, model());
        // Missing, the built one stands in.
        let library = Library::default();
        let missing = Coop::Asset(AssetRef::new(
            "farmyard",
            1,
            "coop.glb",
            Digest::of(b"coop"),
        ));
        assert_eq!(*missing.model(&library), model());
        // Present, it is the asset's.
        let bytes: &'static [u8] = Box::leak(crate::tests::cube_glb().into_boxed_slice());
        library.add(Embedded::new().with(bytes));
        let replaced = Coop::Asset(AssetRef::new("farmyard", 1, "coop.glb", Digest::of(bytes)))
            .model(&library);
        assert_eq!(replaced.meshes[0].primitives[0].positions.len(), 3);
    }
}
