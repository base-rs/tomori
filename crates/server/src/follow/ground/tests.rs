// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use glam::Vec3;
use tomori_environment::navmesh::Obstacle;

use super::*;

/// A yard's floor, 30 m across about the origin.
pub(crate) const YARD: f32 = 15.0;

/// A terrace 4 m above the yard, from 4 to 10 m along X and −10 to −4 m along Z, reached only by teleport: its top is
/// ground, and what stands beneath it, its rock, cuts the yard's floor there.
pub(crate) const TERRACE: ([f32; 2], [f32; 2], f32) = ([4.0, -10.0], [10.0, -4.0], 4.0);

/// A deck 2.5 m above the yard, from −10 to −4 m along X and Z, standing on posts, with the yard's floor beneath it.
pub(crate) const DECK: ([f32; 2], [f32; 2], f32) = ([-10.0, -10.0], [-4.0, -4.0], 2.5);

/// The two triangles of the square from `low` to `high` at `height`.
fn square(low: [f32; 2], high: [f32; 2], height: f32) -> [[Vec3; 3]; 2] {
    let corner = |x: f32, z: f32| Vec3::new(x, height, z);
    let (a, b, c, d) = (
        corner(low[0], low[1]),
        corner(high[0], low[1]),
        corner(high[0], high[1]),
        corner(low[0], high[1]),
    );
    [[a, b, c], [a, c, d]]
}

/// The walkable ground of a yard with a terrace above it, reached only by teleport, and a deck over it, as its
/// environment would derive it.
pub(crate) fn levels() -> Navmesh {
    let (low, high, top) = TERRACE;
    let rock = Obstacle::cuboid(
        Vec3::new(low[0], 0.0, low[1]),
        Vec3::new(high[0], top, high[1]),
    );
    let mut ground = square([-YARD; 2], [YARD; 2], 0.0).to_vec();
    ground.extend(square(low, high, top));
    ground.extend(square(DECK.0, DECK.1, DECK.2));
    Navmesh::derive(ground, &[rock])
}

/// [`levels`] in a space, uncarried.
pub(crate) fn terrace() -> Ground {
    Ground::new(Arc::new(levels()), None)
}

fn sorted(mut floors: Vec<f32>) -> Vec<f32> {
    floors.sort_by(f32::total_cmp);
    floors
}

#[test]
fn its_floors_are_the_yard_the_terrace_and_the_deck_over_the_yard() {
    let ground = terrace();
    assert_eq!(ground.floors(1.0, 0.5), [0.0]);
    assert_eq!(ground.floors(7.3, -6.6), [4.0]);
    assert_eq!(sorted(ground.floors(-7.3, -6.6)), [0.0, 2.5]);
    assert!(ground.floors(20.0, 0.0).is_empty());
    // On each level, the floor there on it; none on a level it does not have there.
    assert_eq!(ground.level_at(-7.0, -7.0, 2.5), Some(2.5));
    assert_eq!(ground.level_at(-7.0, -7.0, 0.3), Some(0.0));
    assert_eq!(ground.level_at(7.0, -7.0, 0.0), None);
    assert_eq!(ground.level_at(12.0, -7.0, 4.0), None);
    // Beyond the terrace's edge, over the yard or the gap its rock cuts in it, another level; beyond the yard's edge,
    // over nothing near any other, none, but for someone on the terrace's level.
    assert!(ground.off_level(12.0, -7.0, 4.0));
    assert!(ground.off_level(10.1, -7.0, 4.0));
    assert!(!ground.off_level(16.0, 0.0, 0.0));
    assert!(!ground.off_level(25.0, 0.0, 0.0));
    assert!(ground.off_level(15.5, 0.0, 4.0));
}

#[test]
fn someone_stands_on_the_highest_floor_beneath_their_eyes_or_beside_them_off_an_edge() {
    let ground = terrace();
    assert_eq!(ground.underfoot([7.0, 5.6, -7.0]), Some(4.0));
    assert_eq!(ground.underfoot([0.0, 1.6, 0.0]), Some(0.0));
    // Seated, or crouching: still their floor; beneath the deck, the yard; on it, the deck.
    assert_eq!(ground.underfoot([0.0, 0.5, 0.0]), Some(0.0));
    assert_eq!(ground.underfoot([-7.0, 1.6, -7.0]), Some(0.0));
    assert_eq!(ground.underfoot([-7.0, 4.1, -7.0]), Some(2.5));
    // Standing in their room a little beyond the terrace's edge, over the yard: on the terrace still; beside its rock in
    // the yard, the yard; a step beyond the yard's edge, the yard; far beyond it, or flying over nothing, nowhere.
    assert_eq!(ground.underfoot([10.4, 5.6, -7.0]), Some(4.0));
    assert_eq!(ground.underfoot([10.4, 1.6, -7.0]), Some(0.0));
    assert_eq!(ground.underfoot([15.6, 1.6, 0.0]), Some(0.0));
    assert_eq!(ground.underfoot([20.0, 1.6, 0.0]), None);
    // Too far below a floor to stand on it, it is not theirs.
    assert_eq!(ground.underfoot([0.0, 0.2, 0.0]), None);
}

#[test]
fn the_agent_stands_clear_of_floors_above_and_below_and_never_inside_a_level() {
    let ground = terrace();
    assert!(ground.clear_of([0.0, 1.7, 0.0]));
    assert!(ground.clear_of([7.0, 5.7, -7.0]));
    // Too near the yard, in it or beneath it.
    assert!(!ground.clear_of([0.0, 0.3, 0.0]));
    assert!(!ground.clear_of([0.0, -0.3, 0.0]));
    assert!(!ground.clear_of([0.0, -2.0, 0.0]));
    // Inside the terrace's rock, or beside its side, within its reach.
    assert!(!ground.clear_of([7.0, 2.0, -7.0]));
    assert!(!ground.clear_of([10.2, 2.0, -7.0]));
    assert!(ground.clear_of([10.4, 2.0, -7.0]));
    // Beneath the deck, with room above and below; too near it, or its edge.
    assert!(ground.clear_of([-7.0, 1.2, -7.0]));
    assert!(!ground.clear_of([-7.0, 2.2, -7.0]));
    assert!(!ground.clear_of([-3.8, 2.7, -7.0]));
    assert!(ground.clear_of([-3.6, 2.7, -7.0]));
}

#[test]
fn a_way_through_a_floor_or_below_a_level_is_not_open() {
    let ground = terrace();
    assert!(ground.crosses([7.0, 3.0, -7.0], [7.0, 5.0, -7.0]));
    assert!(ground.crosses([0.0, 1.0, 0.0], [0.0, -1.0, 0.0]));
    assert!(!ground.crosses([0.0, 1.0, 0.0], [1.0, 1.0, 0.0]));
    assert!(!ground.crosses([0.0, 1.0, 0.0], [0.0, 1.0, 0.0]));
    // Straight up from the yard onto the terrace, it passes through its side; over it, or out over the yard, it is
    // open; straight down from it to the yard below, it passes inside it.
    assert!(!ground.open([0.0, 1.7, 0.0], [7.0, 5.7, -7.0]));
    assert!(ground.open([7.0, 5.7, -7.0], [12.0, 5.7, -7.0]));
    assert!(!ground.open([7.0, 5.7, -7.0], [12.0, 1.7, -7.0]));
    // Up through the deck, never; beneath it, yes.
    assert!(!ground.open([-7.0, 1.5, -7.0], [-7.0, 4.0, -7.0]));
    assert!(ground.open([0.0, 1.5, 0.0], [-7.0, 1.5, -7.0]));
    // Stuck in the yard, it may rise out of it, but not sink further.
    assert!(ground.open([0.0, 0.1, 0.0], [0.0, 1.7, 0.0]));
    assert!(!ground.open([0.0, 0.1, 0.0], [0.0, -1.0, 0.0]));
    // Rising out, it must stay clear once it is.
    assert!(!ground.open([-7.0, 0.1, -7.0], [-7.0, 4.0, -7.0]));
}

#[test]
fn carried_into_a_space_it_answers_in_the_space_s_coordinates() {
    // Arriving on the terrace, facing +X: the terrace's middle is the space's origin, and its floor the space's floor.
    let arrival = Pose::at([7.0, 4.0, -7.0]).facing_towards([8.0, 4.0, -7.0]);
    let carriage = Pose::carriage(&arrival);
    let carried = Ground::new(Arc::new(levels()), Some(carriage));
    assert_eq!(carried.floors(0.3, 0.1), [0.0]);
    let ground = terrace();
    // Every point of the environment's own, carried, has the floors it had there, carried as high.
    for (x, z) in [
        (7.0, -7.0),
        (0.0, 0.0),
        (-7.0, -7.0),
        (12.0, -7.0),
        (20.0, 3.0),
    ] {
        let [sx, _, sz] = carriage.carry(&Pose::at([x, 0.0, z])).position;
        let expected: Vec<f32> = sorted(ground.floors(x, z))
            .into_iter()
            .map(|floor| floor - 4.0)
            .collect();
        let found = sorted(carried.floors(sx, sz));
        assert_eq!(found.len(), expected.len(), "{x} {z}");
        for (found, expected) in found.iter().zip(&expected) {
            assert!(
                (found - expected).abs() < 1e-4,
                "{x} {z}: {found} {expected}"
            );
        }
    }
    // Its floors are crossed where they are carried to.
    let [sx, _, sz] = carriage.carry(&Pose::at([0.0, 0.0, 0.0])).position;
    assert!(carried.crosses([sx, -3.0, sz], [sx, -5.0, sz]));
    assert!(!carried.crosses([sx, -3.0, sz], [sx, -3.5, sz]));
}
