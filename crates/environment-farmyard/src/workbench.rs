// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The workbench in the yard's front left corner, the agent's home: a wooden bench curving through 100° about the
//! middle of its curve, its concave side facing out of the corner towards where participants arrive, on dark legs, with
//! a shelf below, a vice at its front, a plank, a box of nails, a mallet and shavings on its top; and behind it a curved
//! tool rack of weathered boards between two posts, capped, with a saw, a hammer, a wrench, chisels and a square hung on
//! pegs, and a lantern on the post at the gate's end. It is built with the yard, low-poly, in the yard's palette, laid
//! out by [`layout::WORKBENCH`] and the constants beside it, which its solids and footprints come from too, so that
//! what is drawn and what the agent, the workers and the chickens keep clear of agree. See the crate's
//! `context/workbench.md`.
use glam::{Quat, Vec2, Vec3};

use crate::layout::{self, on_workbench, workbench_facing, workbench_turn};
use crate::props::{lantern, palette};
use crate::shapes::Shapes;

/// How many flat pieces the bench's top, its shelf and the rack's cap are drawn in, and its legs' and the rack's
/// boards' thickness.
const PIECES: usize = 10;
const LEG: f32 = 0.07;
const BOARD: f32 = 0.04;

/// The point `radius` from the bench's middle, `turn` along its curve, `y` up.
fn at(radius: f32, turn: f32, y: f32) -> Vec3 {
    let point = on_workbench(radius, turn);
    Vec3::new(point.x, y, point.y)
}

/// The way that turns a thing's +Z out from the bench's middle, `turn` along its curve, towards the rack, and its +X
/// along the curve.
fn along(turn: f32) -> Quat {
    Quat::from_rotation_y(workbench_facing() + std::f32::consts::PI + turn)
}

/// A flat polygon through `corners`, lit from the side `outward` points to, whichever way they wind.
fn face(shapes: &mut Shapes, corners: &[Vec3], outward: Vec3, color: Vec3) {
    let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
    if normal.dot(outward) < 0.0 {
        let reversed: Vec<Vec3> = corners.iter().rev().copied().collect();
        shapes.polygon(&reversed, color);
    } else {
        shapes.polygon(corners, color);
    }
}

/// Which faces of a curved slab are drawn.
#[derive(Clone, Copy)]
struct Faces {
    top: bool,
    bottom: bool,
    inner: bool,
    outer: bool,
    ends: bool,
}

const EVERY: Faces = Faces {
    top: true,
    bottom: true,
    inner: true,
    outer: true,
    ends: true,
};

/// A slab following the bench's curve, from `inner` to `outer` about its middle and from `low` to `high`, in `pieces`
/// flat pieces, each its colour a little varied, as planks are.
fn slab(
    shapes: &mut Shapes,
    [inner, outer]: [f32; 2],
    [low, high]: [f32; 2],
    pieces: usize,
    faces: Faces,
    color: Vec3,
) {
    for piece in 0..pieces {
        let (from, to) = (
            workbench_turn(piece, pieces),
            workbench_turn(piece + 1, pieces),
        );
        let middle = (from + to) * 0.5;
        let out = along(middle) * Vec3::Z;
        let shade = color * if piece % 2 == 0 { 1.0 } else { 0.9 };
        let corner = |radius: f32, turn: f32, y: f32| at(radius, turn, y);
        if faces.top {
            face(
                shapes,
                &[
                    corner(inner, from, high),
                    corner(inner, to, high),
                    corner(outer, to, high),
                    corner(outer, from, high),
                ],
                Vec3::Y,
                shade,
            );
        }
        if faces.bottom {
            face(
                shapes,
                &[
                    corner(inner, from, low),
                    corner(inner, to, low),
                    corner(outer, to, low),
                    corner(outer, from, low),
                ],
                -Vec3::Y,
                shade * 0.8,
            );
        }
        for (radius, drawn, outward) in [(inner, faces.inner, -out), (outer, faces.outer, out)] {
            if drawn {
                face(
                    shapes,
                    &[
                        corner(radius, from, low),
                        corner(radius, to, low),
                        corner(radius, to, high),
                        corner(radius, from, high),
                    ],
                    outward,
                    shade * 0.92,
                );
            }
        }
        if faces.ends && (piece == 0 || piece == pieces - 1) {
            let (turn, outward) = if piece == 0 {
                (from, -(along(from) * Vec3::X))
            } else {
                (to, along(to) * Vec3::X)
            };
            face(
                shapes,
                &[
                    corner(inner, turn, low),
                    corner(outer, turn, low),
                    corner(outer, turn, high),
                    corner(inner, turn, high),
                ],
                outward,
                shade * 0.85,
            );
        }
    }
}

/// A box centred `radius` from the bench's middle, `turn` along its curve, `y` up, `half` its size across the curve,
/// up and out, turned with the curve there.
fn piece(shapes: &mut Shapes, radius: f32, turn: f32, y: f32, half: Vec3, color: Vec3) {
    shapes.cuboid(at(radius, turn, y), half, along(turn), color);
}

/// The bench: its top, its legs, the shelf below and the rails beneath its top, and the vice at its front.
fn bench(shapes: &mut Shapes) {
    let [inner, outer] = layout::WORKBENCH_TOP;
    let top = layout::WORKBENCH_HEIGHT;
    slab(
        shapes,
        [inner, outer],
        [top - 0.07, top],
        PIECES,
        EVERY,
        palette::WOOD,
    );
    // The shelf low down between the legs, and a rail under the top's front edge.
    let shelf = [inner + 0.08, outer - 0.08];
    slab(
        shapes,
        shelf,
        [0.2, 0.24],
        PIECES / 2,
        EVERY,
        palette::WOOD * 0.85,
    );
    slab(
        shapes,
        [inner + 0.04, inner + 0.07],
        [top - 0.17, top - 0.07],
        PIECES,
        Faces {
            bottom: false,
            ..EVERY
        },
        palette::DARK_WOOD,
    );
    // Four pairs of legs along the curve, set in from its ends.
    let span = layout::WORKBENCH_HALF_SPAN;
    let inset = LEG / inner;
    for turn in [-span + inset, -span / 3.0, span / 3.0, span - inset] {
        for radius in [inner + 0.07, outer - 0.07] {
            piece(
                shapes,
                radius,
                turn,
                (top - 0.07) * 0.5,
                Vec3::new(LEG * 0.5, (top - 0.07) * 0.5, LEG * 0.5),
                palette::DARK_WOOD,
            );
        }
    }
    // The vice at the front of its left half: a wooden jaw proud of the top's front edge, its top flush with the top,
    // closed on its screw by a bar.
    let vice = -span * 0.55;
    let jaw = inner - layout::VICE * 0.5;
    piece(
        shapes,
        jaw,
        vice,
        top - 0.08,
        Vec3::new(0.16, 0.08, layout::VICE * 0.5),
        palette::WOOD * 1.1,
    );
    piece(
        shapes,
        jaw,
        vice,
        top - 0.12,
        Vec3::new(0.015, 0.015, layout::VICE * 0.5),
        palette::METAL,
    );
    piece(
        shapes,
        inner - layout::VICE + 0.015,
        vice,
        top - 0.12,
        Vec3::new(0.12, 0.012, 0.012),
        palette::METAL * 1.2,
    );
}

/// What lies on the bench's top, no higher than [`layout::WORKBENCH_CLUTTER`]: a plank, a box of nails, a mallet and
/// curls of shavings.
fn clutter(shapes: &mut Shapes) {
    let [inner, outer] = layout::WORKBENCH_TOP;
    let top = layout::WORKBENCH_HEIGHT;
    let span = layout::WORKBENCH_HALF_SPAN;
    // A plank laid along the curve on its right half.
    piece(
        shapes,
        outer - 0.2,
        span * 0.45,
        top + 0.012,
        Vec3::new(0.4, 0.012, 0.07),
        palette::WEATHERED * 1.15,
    );
    // A box of nails, their heads grey within its rim.
    let nails = at(outer - 0.22, -span * 0.12, top + 0.03);
    let turn = along(-span * 0.12) * Quat::from_rotation_y(0.3);
    shapes.cuboid(
        nails,
        Vec3::new(0.09, 0.03, 0.06),
        turn,
        palette::DARK_WOOD * 1.2,
    );
    let rim = |x: f32, z: f32| nails + turn * Vec3::new(x, 0.031, z);
    shapes.polygon(
        &[
            rim(-0.075, 0.045),
            rim(0.075, 0.045),
            rim(0.075, -0.045),
            rim(-0.075, -0.045),
        ],
        palette::METAL * 1.3,
    );
    // A mallet lying on its side, its handle towards the front.
    let mallet = span * 0.12;
    piece(
        shapes,
        inner + 0.28,
        mallet,
        top + 0.03,
        Vec3::new(0.012, 0.012, 0.13),
        palette::WOOD * 1.25,
    );
    piece(
        shapes,
        inner + 0.44,
        mallet,
        top + 0.03,
        Vec3::new(0.08, 0.03, 0.045),
        palette::DARK_WOOD * 1.3,
    );
    // Curls of shavings, by the vice.
    for curl in 0..5u32 {
        let turn = -span * (0.4 + crate::props::scatter(curl * 3 + 900) * 0.3);
        let radius = inner + 0.08 + crate::props::scatter(curl * 3 + 901) * 0.35;
        shapes.disc(
            at(radius, turn, top + 0.004),
            0.03 + crate::props::scatter(curl * 3 + 902) * 0.02,
            5,
            palette::STRAW * 1.2,
        );
    }
}

/// The rack behind the bench: its boards between two posts, capped, with pegs and the tools hung on them.
fn rack(shapes: &mut Shapes, glass: &mut Shapes) {
    let [inner, outer] = layout::RACK;
    let middle = (inner + outer) * 0.5;
    let bottom = layout::WORKBENCH_HEIGHT + 0.02;
    let height = layout::RACK_HEIGHT;
    let span = layout::WORKBENCH_HALF_SPAN;
    let boards = [middle - BOARD * 0.5, middle + BOARD * 0.5];
    slab(
        shapes,
        boards,
        [bottom, height - 0.1],
        layout::RACK_BOARDS,
        Faces {
            bottom: false,
            top: false,
            ..EVERY
        },
        palette::WEATHERED,
    );
    // A cap along its top, and a rail along its foot.
    slab(
        shapes,
        [inner, outer],
        [height - 0.1, height - 0.06],
        layout::RACK_BOARDS,
        EVERY,
        palette::DARK_WOOD,
    );
    slab(
        shapes,
        [boards[0] - 0.02, boards[0]],
        [bottom, bottom + 0.06],
        layout::RACK_BOARDS,
        Faces {
            outer: false,
            ..EVERY
        },
        palette::DARK_WOOD,
    );
    // Its posts, at either end, from the ground.
    for end in [-1.0, 1.0] {
        let turn = end * (span - 0.04 / middle);
        piece(
            shapes,
            middle,
            turn,
            height * 0.5,
            Vec3::new(0.035, height * 0.5, 0.035),
            palette::DARK_WOOD,
        );
    }
    // The tools, hung flat on the boards' front on pegs, each where it is along the curve, as `(turn, height)`.
    let front = boards[0] - 0.008;
    let peg = |shapes: &mut Shapes, turn: f32, y: f32| {
        piece(
            shapes,
            front - 0.002,
            turn,
            y,
            Vec3::new(0.008, 0.008, 0.01),
            palette::DARK_WOOD * 0.8,
        );
    };
    let flat = |shapes: &mut Shapes, turn: f32, y: f32, half: [f32; 2], color: Vec3| {
        piece(
            shapes,
            front,
            turn,
            y,
            Vec3::new(half[0], half[1], 0.006),
            color,
        );
    };
    // A saw, its blade tapering from its handle.
    let saw = -span * 0.72;
    peg(shapes, saw, 1.62);
    let blade = |x: f32, y: f32| at(front - 0.004, saw + x / front, y);
    face(
        shapes,
        &[
            blade(-0.1, 1.58),
            blade(0.1, 1.58),
            blade(0.05, 1.08),
            blade(-0.04, 1.08),
        ],
        -(along(saw) * Vec3::Z),
        palette::METAL * 1.4,
    );
    flat(shapes, saw, 1.64, [0.06, 0.05], palette::WOOD * 1.2);
    // A hammer, its head up.
    let hammer = -span * 0.4;
    peg(shapes, hammer, 1.58);
    flat(shapes, hammer, 1.35, [0.014, 0.17], palette::WOOD * 1.3);
    flat(shapes, hammer, 1.53, [0.07, 0.025], palette::METAL);
    // A wrench.
    let wrench = -span * 0.12;
    peg(shapes, wrench, 1.6);
    flat(shapes, wrench, 1.42, [0.012, 0.15], palette::METAL * 1.2);
    flat(shapes, wrench, 1.58, [0.035, 0.025], palette::METAL * 1.2);
    // Three chisels, side by side.
    for chisel in 0..3 {
        let turn = span * (0.12 + chisel as f32 * 0.06);
        peg(shapes, turn, 1.56);
        flat(shapes, turn, 1.5, [0.012, 0.045], palette::DARK_WOOD * 1.4);
        flat(shapes, turn, 1.39, [0.008, 0.065], palette::METAL * 1.3);
    }
    // A square, its two arms.
    let square = span * 0.55;
    peg(shapes, square, 1.6);
    flat(shapes, square, 1.42, [0.012, 0.16], palette::METAL * 1.1);
    let foot = square + 0.09 / front;
    flat(shapes, foot, 1.27, [0.1, 0.012], palette::METAL * 1.1);
    // The lantern on the post at the gate's end, on a hook from it.
    let lamp = layout::workbench_lantern();
    let hook = at(middle, span - 0.04 / middle, lamp.y + 0.2);
    shapes.beam(hook, lamp + Vec3::Y * 0.2, 0.025, palette::DARK_WOOD);
    shapes.beam(
        lamp + Vec3::Y * 0.2,
        lamp + Vec3::Y * 0.15,
        0.012,
        palette::METAL,
    );
    lantern(shapes, glass, lamp);
}

/// Build the workbench into the yard's `shapes`, its lantern's glass into `glass`.
pub fn build(shapes: &mut Shapes, glass: &mut Shapes) {
    bench(shapes);
    clutter(shapes);
    rack(shapes, glass);
}

/// Where the middle of the workbench's top is, `[x, z]`, for words about what is near it.
pub fn middle() -> Vec2 {
    let [inner, outer] = layout::WORKBENCH_TOP;
    on_workbench((inner + outer) * 0.5, 0.0)
}

#[cfg(test)]
mod tests;
