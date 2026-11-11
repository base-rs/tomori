// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// A square of ground `half` from `centre` each way, at its height, as two triangles.
fn square(centre: Vec3, half: f32) -> [[Vec3; 3]; 2] {
    let at = |x: f32, z: f32| centre + Vec3::new(x * half, 0.0, z * half);
    [
        [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0)],
        [at(-1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)],
    ]
}

/// The area of `mesh`'s triangles, seen from above.
fn area(mesh: &Navmesh) -> f32 {
    mesh.triangles()
        .iter()
        .map(|triangle| doubled_area(triangle).abs() * 0.5)
        .sum()
}

#[test]
fn ground_is_kept_where_it_is_drawn_at_its_height_and_steep_ground_is_left_out() {
    // A floor at 0.3 m, a ramp rising at 20° beside it, a wall standing up and a slope of 40°.
    let mut ground: Vec<[Vec3; 3]> = square(Vec3::new(0.0, 0.3, 0.0), 2.0).to_vec();
    let rise = 20f32.to_radians().tan();
    ground.push([
        Vec3::new(2.0, 0.3, -2.0),
        Vec3::new(4.0, 0.3 + 2.0 * rise, -2.0),
        Vec3::new(2.0, 0.3, 2.0),
    ]);
    ground.push([
        Vec3::new(-2.0, 0.0, -3.0),
        Vec3::new(2.0, 0.0, -3.0),
        Vec3::new(2.0, 2.0, -3.0),
    ]);
    let steep = 40f32.to_radians().tan();
    ground.push([
        Vec3::new(-6.0, 0.0, -2.0),
        Vec3::new(-6.0, 0.0, 2.0),
        Vec3::new(-4.0, 2.0 * steep, 0.0),
    ]);
    let mesh = Navmesh::derive(ground, &[]);
    assert_eq!(mesh.triangles().len(), 3);
    assert_eq!(mesh.height_at(Vec2::new(0.5, 0.5)), Some(0.3));
    let up = mesh.height_at(Vec2::new(3.0, -1.5)).unwrap();
    assert!((up - (0.3 + rise)).abs() < 1e-5, "{up}");
    assert_eq!(mesh.height_at(Vec2::new(-5.0, 0.0)), None);
    // Its edges are on it, and a hair beyond them is not.
    assert_eq!(mesh.height_at(Vec2::new(-2.0, 2.0)), Some(0.3));
    assert_eq!(mesh.height_at(Vec2::new(-2.001, 0.0)), None);
    // Nonsense is left out; so is a triangle with no area.
    let mesh = Navmesh::derive(
        [
            [Vec3::NAN, Vec3::X, Vec3::Z],
            [Vec3::ZERO, Vec3::X, Vec3::X * 2.0],
        ],
        &[],
    );
    assert!(mesh.is_empty() && mesh.bounds().is_none());
    assert_eq!(mesh.height_at(Vec2::ZERO), None);
    assert_eq!(mesh.meets(Vec3::Y, Vec3::NEG_Y, f32::INFINITY, false), None);
}

#[test]
fn an_obstacle_cuts_its_footprint_grown_by_its_clearance_out_of_the_ground() {
    let floor = square(Vec3::ZERO, 5.0);
    let crate_ = Obstacle::cuboid(Vec3::new(1.0, 0.0, -1.5), Vec3::new(2.0, 0.8, -0.5));
    let mesh = Navmesh::derive(floor, std::slice::from_ref(&crate_));
    // As the box grown by a quarter of a metre on every side.
    let cut = 1.5 * 1.5;
    let left = area(&mesh);
    assert!((left - (100.0 - cut)).abs() < 1e-3, "{left}");
    for x in -50..=50 {
        for z in -50..=50 {
            let point = Vec2::new(x as f32, z as f32) * 0.1;
            // None of these points is on the cut's edge, at 0.75 or 2.25 along X, or −1.75 or −0.25 along Z.
            let grown = point.x > 0.75 && point.x < 2.25 && point.y > -1.75 && point.y < -0.25;
            assert_eq!(mesh.height_at(point).is_none(), grown, "{point}");
            assert_eq!(crate_.keeps(point), grown, "{point}");
        }
    }
    // A clearance of its own; none at all for nonsense.
    let close = Navmesh::derive(floor, &[crate_.clone().with_clearance(0.05)]);
    assert!((area(&close) - (100.0 - 1.1 * 1.1)).abs() < 1e-3);
    assert_eq!(crate_.clone().with_clearance(f32::NAN).clearance(), 0.0);
    // A triangle's footprint is cut mitred, but no further than its box grown by its clearance.
    let wedge = Obstacle::footprint(
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(0.0, 0.5),
        ],
        0.0,
        1.0,
    );
    assert_eq!(wedge.hull().len(), 3);
    assert!(wedge.keeps(Vec2::new(4.2, 0.0)) && !wedge.keeps(Vec2::new(4.3, 0.0)));
    assert!(wedge.keeps(Vec2::new(-0.2, 0.7)) && !wedge.keeps(Vec2::new(2.0, 0.6)));
    // A post given as one corner is cut about as a square.
    let post = Obstacle::footprint([Vec2::new(-3.0, 3.0)], 0.0, 1.0);
    assert!(post.keeps(Vec2::new(-2.8, 3.2)) && !post.keeps(Vec2::new(-2.7, 3.0)));
    // Corners within the hull are left out of it, and so is nonsense.
    let hull = Obstacle::footprint(
        [
            Vec2::ZERO,
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 0.2),
            Vec2::NAN,
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, 1.0),
        ],
        0.0,
        1.0,
    );
    assert_eq!(hull.hull().len(), 4);
    assert!(!Obstacle::footprint([], 0.0, 1.0).keeps(Vec2::ZERO));
}

#[test]
fn an_obstacle_cuts_only_ground_whose_room_it_reaches_into() {
    let floor = square(Vec3::ZERO, 5.0);
    // A bough above the room, a block buried beneath the ground, a lintel within the room, and a block standing on it.
    let bough = Obstacle::cuboid(Vec3::new(-1.0, 2.5, -1.0), Vec3::new(1.0, 3.0, 1.0));
    let buried = Obstacle::cuboid(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, -0.1, 1.0));
    let lintel = Obstacle::cuboid(Vec3::new(3.0, 1.8, -1.0), Vec3::new(4.0, 2.0, 1.0));
    let block = Obstacle::cuboid(Vec3::new(-4.0, 0.0, -1.0), Vec3::new(-3.0, 0.4, 1.0));
    let mesh = Navmesh::derive(floor, &[bough, buried, lintel, block]);
    assert_eq!(mesh.height_at(Vec2::ZERO), Some(0.0));
    assert_eq!(mesh.height_at(Vec2::new(3.5, 0.0)), None);
    assert_eq!(mesh.height_at(Vec2::new(-3.5, 0.0)), None);
    // A floor above the block, as a deck over it, is not cut.
    let deck = square(Vec3::new(-3.5, 0.5, 0.0), 1.0);
    let block = Obstacle::cuboid(Vec3::new(-4.0, 0.0, -1.0), Vec3::new(-3.0, 0.4, 1.0));
    let mesh = Navmesh::derive(deck, &[block]);
    assert_eq!(mesh.height_at(Vec2::new(-3.5, 0.0)), Some(0.5));
}

#[test]
fn the_highest_floor_is_where_one_stands_and_a_ray_meets_the_first_from_above() {
    // A meadow, and a track 8 mm above it across its middle.
    let mut ground: Vec<[Vec3; 3]> = square(Vec3::ZERO, 10.0).to_vec();
    ground.extend([
        [
            Vec3::new(-10.0, 0.008, -0.6),
            Vec3::new(10.0, 0.008, -0.6),
            Vec3::new(10.0, 0.008, 0.6),
        ],
        [
            Vec3::new(-10.0, 0.008, -0.6),
            Vec3::new(10.0, 0.008, 0.6),
            Vec3::new(-10.0, 0.008, 0.6),
        ],
    ]);
    let mesh = Navmesh::derive(ground, &[]);
    assert_eq!(mesh.height_at(Vec2::new(3.0, 0.0)), Some(0.008));
    let mut heights: Vec<f32> = mesh.heights(Vec2::new(3.0, 0.0)).collect();
    heights.sort_by(f32::total_cmp);
    assert_eq!(heights, [0.0, 0.008]);
    assert_eq!(mesh.height_at(Vec2::new(3.0, 2.0)), Some(0.0));
    // Straight down onto the track, slanting onto the meadow beyond it from far away, and up from beneath, which only
    // a ray that may meet either side meets.
    let down = mesh.meets(Vec3::new(3.0, 1.6, 0.0), Vec3::NEG_Y, f32::INFINITY, false);
    assert!((down.unwrap() - (1.6 - 0.008)).abs() < 1e-6);
    let origin = Vec3::new(-9.5, 1.6, 9.5);
    let towards = Vec3::new(8.0, 0.0, -8.0) - origin;
    let distance = mesh.meets(origin, towards, f32::INFINITY, false).unwrap();
    assert!((origin + towards * distance).distance(Vec3::new(8.0, 0.0, -8.0)) < 1e-4);
    assert_eq!(mesh.meets(origin, towards, distance * 0.9, false), None);
    let below = Vec3::new(3.0, -1.0, 0.0);
    assert_eq!(mesh.meets(below, Vec3::Y, f32::INFINITY, false), None);
    let up = mesh.meets(below, Vec3::Y, f32::INFINITY, true).unwrap();
    assert!((up - 1.0).abs() < 1e-6);
    // A ray that goes nowhere near it, one that starts beyond it going away, one along its side and one that is
    // nonsense meet nothing.
    assert_eq!(
        mesh.meets(Vec3::new(30.0, 1.0, 0.0), Vec3::NEG_Y, f32::INFINITY, false),
        None
    );
    assert_eq!(
        mesh.meets(
            Vec3::new(11.0, 1.0, 0.0),
            Vec3::new(1.0, -0.1, 0.0),
            f32::INFINITY,
            false
        ),
        None
    );
    assert_eq!(
        mesh.meets(Vec3::new(0.0, 0.5, 0.0), Vec3::X, f32::INFINITY, true),
        None
    );
    assert_eq!(
        mesh.meets(Vec3::NAN, Vec3::NEG_Y, f32::INFINITY, false),
        None
    );
    // Every way round, across many cells, from near and far, a ray meets first what the nearest of all the
    // triangles is, as found by trying every one.
    for step in 0..72 {
        let yaw = step as f32 / 72.0 * std::f32::consts::TAU;
        let reach = 1.0 + (step % 9) as f32;
        let target = Vec3::new(yaw.sin() * reach, 0.0, yaw.cos() * reach + 0.3);
        let origin = Vec3::new(-yaw.sin() * 2.0, 1.6, -yaw.cos() * 2.0);
        let direction = target - origin;
        let nearest = mesh
            .triangles()
            .iter()
            .filter_map(|triangle| hit(triangle, origin, direction, false))
            .filter(|distance| *distance >= 0.0)
            .reduce(f32::min);
        assert_eq!(
            mesh.meets(origin, direction, f32::INFINITY, false),
            nearest,
            "{yaw}"
        );
        assert!(nearest.is_some());
    }
}

#[test]
fn triangles_given_as_walkable_are_kept_as_they_are_and_a_wide_mesh_has_coarser_cells() {
    let steep = [
        Vec3::ZERO,
        Vec3::new(1.0, 1.5, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let mesh = Navmesh::from_triangles([steep, [Vec3::ZERO, Vec3::Y, Vec3::X]]);
    assert_eq!(mesh.triangles(), [steep]);
    assert!(format!("{mesh:?}").contains("triangles: 1"));
    let wide = Navmesh::from_triangles(square(Vec3::ZERO, 900.0));
    assert!(wide.cell > CELL && wide.columns * wide.rows <= MOST_CELLS as usize);
    assert_eq!(wide.height_at(Vec2::new(899.0, -899.0)), Some(0.0));
    let (low, high) = wide.bounds().unwrap();
    assert_eq!(
        (low, high),
        (Vec3::new(-900.0, 0.0, -900.0), Vec3::new(900.0, 0.0, 900.0))
    );
}

#[test]
fn many_obstacles_leave_the_ground_between_them_whole() {
    // A yard of 10 by 12 m, with a grid of thirty posts, cut into pieces which together cover all of it but them.
    let floor = square(Vec3::new(0.0, 0.0, 0.0), 6.0);
    let posts: Vec<Obstacle> = (0..30)
        .map(|index| {
            let at = Vec2::new(
                (index % 6) as f32 * 2.0 - 5.0,
                (index / 6) as f32 * 2.0 - 4.0,
            );
            Obstacle::cuboid(
                Vec3::new(at.x - 0.1, 0.0, at.y - 0.1),
                Vec3::new(at.x + 0.1, 1.0, at.y + 0.1),
            )
        })
        .collect();
    let mesh = Navmesh::derive(floor, &posts);
    let cut = 30.0 * 0.7 * 0.7;
    let left = area(&mesh);
    assert!((left - (144.0 - cut)).abs() < 1e-2, "{left}");
    for x in -59..=59 {
        for z in -59..=59 {
            let point = Vec2::new(x as f32, z as f32) * 0.1 + 0.013;
            let kept = posts.iter().any(|post| post.keeps(point));
            assert_eq!(mesh.height_at(point).is_some(), !kept, "{point}");
        }
    }
}

#[test]
fn rays_run_through_every_cell_to_the_grid_s_edges_and_only_so_many_triangles_are_kept() {
    // A floor exactly 10 m square, its cells a metre each, with a deck 2 m up in its corner, so that a level ray at 1 m
    // is within the box about it all the way across, and meets nothing, either way along X or Z.
    let mut ground: Vec<[Vec3; 3]> = square(Vec3::ZERO, 5.0).to_vec();
    ground.push([
        Vec3::new(4.0, 2.0, 4.0),
        Vec3::new(5.0, 2.0, 4.0),
        Vec3::new(5.0, 2.0, 5.0),
    ]);
    // An obstacle with no corners cuts nothing.
    let nothing = Obstacle::footprint([Vec2::NAN], 0.0, 1.0);
    assert!(nothing.hull().is_empty());
    let mesh = Navmesh::derive(ground, &[nothing]);
    assert_eq!((mesh.columns, mesh.rows), (10, 10));
    let origin = Vec3::new(0.2, 1.0, 0.3);
    for way in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
        assert_eq!(mesh.meets(origin, way, f32::INFINITY, true), None, "{way}");
    }
    // Shallow, from far along each axis, it crosses every cell before it meets the floor.
    for (from, to) in [
        (Vec3::new(4.9, 1.6, 0.5), Vec3::new(-4.9, 0.0, 0.5)),
        (Vec3::new(0.5, 1.6, 4.9), Vec3::new(0.5, 0.0, -4.9)),
    ] {
        let distance = mesh.meets(from, to - from, f32::INFINITY, false).unwrap();
        assert!((distance - 1.0).abs() < 1e-4, "{distance}");
    }
    // An obstacle's height.
    let block = Obstacle::cuboid(Vec3::new(1.0, 0.5, 1.0), Vec3::ZERO);
    assert_eq!((block.bottom(), block.top()), (0.0, 0.5));
    // Ground beyond the most triangles kept is dropped, whether drawn or cut.
    let tiny = |index: usize| {
        let at = Vec3::new((index % 300) as f32, 0.0, (index / 300) as f32);
        [at, at + Vec3::X * 0.5, at + Vec3::Z * 0.5]
    };
    let many = Navmesh::derive((0..MOST_TRIANGLES + 10).map(tiny), &[]);
    assert_eq!(many.triangles().len(), MOST_TRIANGLES);
    let squares = (0..MOST_TRIANGLES / 2 + 10).flat_map(|index| {
        let at = Vec3::new((index % 300) as f32, 0.0, (index / 300) as f32);
        square(at, 0.4)
    });
    let post =
        Obstacle::cuboid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.1, 1.0, 0.1)).with_clearance(0.0);
    let cut = Navmesh::derive(squares, &[post]);
    assert_eq!(cut.triangles().len(), MOST_TRIANGLES);
}
