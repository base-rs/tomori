// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::backdrop::{BANKS, RIM};
use crate::layout::{FENCE, Footprint, POOL};

/// How far `point` is beyond the fence, on the ground, or nothing inside it.
fn beyond_fence(point: Vec2) -> f32 {
    (FENCE.low - point)
        .max(point - FENCE.high)
        .max(Vec2::ZERO)
        .length()
}

/// The edges of the trail's band, either side of each point of its middle.
fn band() -> Vec<Vec2> {
    (0..centreline().len())
        .flat_map(|index| {
            let (point, _, outward) = frame(index);
            [point + outward * HALF, point - outward * HALF]
        })
        .collect()
}

/// The solids of the yard and the trail, the plants' and the pines'.
fn solids() -> Vec<tomori_environment::sightlines::Solid> {
    use tomori_environment::sightlines::Solid;
    let mut solids: Vec<Solid> = layout::solids()
        .into_iter()
        .chain(super::solids())
        .map(|(low, high)| Solid::new(low, high))
        .collect();
    solids.extend(backdrop::plants().iter().map(backdrop::Plant::solid));
    solids.extend(crate::props::pines().into_iter().map(|(base, height)| {
        let reach = height * 0.32;
        Solid::new(
            (base - Vec3::new(reach, 0.0, reach)).to_array(),
            (base + Vec3::new(reach, height, reach)).to_array(),
        )
    }));
    solids
}

#[test]
fn the_trail_loops_round_the_outside_of_the_fence_past_the_river_on_the_level_meadow() {
    let line = centreline();
    assert_eq!(line.len(), WAYPOINTS.len() * STEPS);
    assert_eq!(line[0], WAYPOINTS[0]);
    assert_eq!(line, centreline());
    // A loop of about 60 m, closed, its points about a metre apart, turning gently enough that its band
    // never folds: each way along it within 40° of the last.
    let steps: Vec<f32> = (0..line.len())
        .map(|index| line[index].distance(line[(index + 1) % line.len()]))
        .collect();
    let length: f32 = steps.iter().sum();
    assert!((55.0..65.0).contains(&length), "{length}");
    assert!(
        steps.iter().all(|step| (0.5..1.25).contains(step)),
        "{steps:?}"
    );
    for index in 0..line.len() {
        let (_, along, outward) = frame(index);
        let (_, next, _) = frame(index + 1);
        assert!(along.dot(next) > 40f32.to_radians().cos(), "{index}");
        assert!((outward.length() - 1.0).abs() < 1e-4 && outward.dot(along).abs() < 1e-4);
    }
    // All of it within its reach of the backdrop's middle, so on the level meadow inside the foothills' first ring, and
    // outside the fence, its inner edge at least 0.9 m from it, its posts and seat too.
    const { assert!(REACH <= backdrop::RINGS[0]) };
    for edge in band() {
        assert!(edge.distance(layout::BACKDROP_CENTRE) < REACH, "{edge}");
        assert_eq!(backdrop::ground(edge), 0.0);
    }
    for (middle, radius) in clearings() {
        assert!(
            middle.distance(layout::BACKDROP_CENTRE) + radius < REACH,
            "{middle}"
        );
        assert!(beyond_fence(middle) > radius + 0.5, "{middle}");
    }
    for point in &line {
        assert!(beyond_fence(*point) > HALF + 0.9, "{point}");
    }
    for (low, high) in super::solids() {
        let footprint = Footprint::new([low[0], low[2]], [high[0], high[2]]);
        assert!(!FENCE.grown(0.5).overlaps(&footprint), "{footprint:?}");
    }
    // It goes along the river's bank, the river within 7 m of its middle for 12 m and more of it, but never onto the
    // bank, which stands above the water, nor the pool's rim, and stays clear of the pines and the bush.
    let pines = crate::props::pines();
    let plants = backdrop::plants();
    let mut by_river = 0.0;
    for (index, point) in line.iter().enumerate() {
        let river = layout::nearest_river(*point).1;
        assert!(river > BANKS[3] + HALF, "{point} {river}");
        if river < 7.0 {
            by_river += steps[index];
        }
        assert!(point.distance(Vec2::new(POOL.x, POOL.z)) > RIM[3] + HALF);
        for (base, height) in &pines {
            assert!(Vec2::new(base.x, base.z).distance(*point) > height * 0.32 + HALF + 0.3);
        }
        for plant in &plants {
            let at = Vec2::new(plant.at.x, plant.at.z);
            assert!(
                at.distance(*point) > plant.size().0 + HALF + 0.3,
                "{plant:?}"
            );
        }
    }
    assert!(by_river > 12.0, "{by_river}");
}

#[test]
fn the_waterfall_and_the_pool_are_seen_from_the_lookout_and_the_river_from_the_riverbank() {
    use tomori_environment::sightlines::EYES;
    let solids = solids();
    for ((name, _, at), sights) in VIEWPOINTS.iter().zip([
        vec![layout::RIVER[6], layout::RIVER[7]],
        vec![layout::WATERFALL, layout::WATERFALL_FOOT, POOL],
    ]) {
        let (point, _, _) = frame(*at);
        let eyes = Vec3::new(point.x, HEIGHT + EYES, point.y);
        for sight in sights {
            assert!(
                !solids
                    .iter()
                    .any(|solid| solid.blocks(eyes.to_array(), sight.to_array())),
                "{name} {sight}"
            );
            for step in 1..100 {
                let between = eyes.lerp(sight, step as f32 / 100.0);
                if between.distance(sight) < 1.2 {
                    break;
                }
                let ground = backdrop::ground(Vec2::new(between.x, between.z));
                assert!(ground < between.y - 0.05, "{name} {sight} {between}");
            }
        }
    }
}

#[test]
fn the_way_out_goes_from_the_yard_s_earth_between_the_gate_s_posts_to_the_loop() {
    let [inside, outside] = GATEWAY;
    assert_eq!(outside, centreline()[0]);
    // It begins on the yard's earth, where participants land, its whole width.
    for corner in [inside - Vec2::X * HALF, inside + Vec2::X * HALF] {
        assert!(
            layout::WALKABLE
                .iter()
                .any(|walkable| walkable.contains(corner)),
            "{corner}"
        );
    }
    // Between the gate's posts, clear of them, straight out to the loop's first point, beyond the fence.
    let post = 0.08;
    assert!(inside.x - HALF > layout::GATE[0] + post && inside.x + HALF < layout::GATE[1] - post);
    assert!(inside.y < FENCE.high.y && outside.y > FENCE.high.y + HALF);
    assert_eq!(inside.x, outside.x);
    // Its outline is a rectangle the yard's earth and the loop's band both overlap.
    let outlines = outlines();
    assert_eq!(outlines.len(), STRETCHES + 1);
    assert_eq!(outlines[STRETCHES].len(), 4);
}

#[test]
fn the_hens_stay_in_the_yard_away_from_the_gate_even_startled() {
    use std::sync::Arc;

    use crate::flock::{AREA, Flock, Intent, seed};
    // Where they keep to is inside the fence, a sprint's length and more short of the gate.
    let sprint = 1.3;
    let inside = FENCE.grown(-0.1);
    assert!(inside.contains(AREA.low) && inside.contains(AREA.high));
    assert!(AREA.high.y + sprint < FENCE.high.y - 1.0);
    // Nor do any wander out through it, as two participants on the trail beyond it point at each in turn, startling
    // them.
    let chicken = Arc::new(crate::chicken::tests::chicken());
    for space in ["space-1", "space-2"] {
        let mut flock = Flock::new(seed(space), chicken.clone(), 0);
        let count = flock.hens().len();
        let mut sprints = 0;
        let mut now = 0;
        while now < 10 * 60_000 {
            if now % 7_000 == 0 {
                let hen = (now / 7_000) as usize % count;
                for participant in [1, 2] {
                    flock.ask(
                        participant,
                        Intent::Notice {
                            notice: hen,
                            toward: GATEWAY[1].to_array(),
                        },
                        now,
                    );
                }
            }
            flock.update(now);
            for (index, hen) in flock.hens().iter().enumerate() {
                if matches!(hen.behaviour.doing, crate::behaviour::Doing::Sprint { .. }) {
                    sprints += 1;
                }
                let at = flock.position(index, now);
                let point = Vec2::new(at.x, at.z);
                assert!(inside.contains(point), "{space} {index} {point}");
                assert!(point.y < FENCE.high.y - 1.0, "{space} {index} {point}");
            }
            now += 50;
        }
        assert!(sprints > 0, "{space}");
    }
}

#[test]
fn five_exercise_stations_stand_beside_the_trail_each_on_its_clearing_with_its_exercise() {
    let line = centreline();
    let labels: Vec<&str> = STATIONS.iter().map(|station| station.label).collect();
    assert_eq!(
        labels,
        [
            "10 squats",
            "Shoulder and neck stretches, 30 seconds",
            "20 calf raises",
            "10 lunges each side",
            "Arm circles and a deep breath, 30 seconds",
        ]
    );
    let places = places();
    for (index, station) in STATIONS.iter().enumerate() {
        assert_eq!(station.name, format!("station-{}", index + 1));
        // Nothing on the floor, where a headset makes it awkward, nor leaning on anything, which is not there.
        assert!(!station.label.contains("push"));
        // Its plaque names it, then says its exercise in a few short lines.
        assert_eq!(station.plaque[0], format!("Station {}", index + 1));
        assert!(station.plaque.iter().all(|line| line.chars().count() <= 18));
        let words: String = station.plaque[1..].join(" ");
        assert!(
            station
                .label
                .starts_with(words.trim().trim_end_matches(" s")),
            "{words}"
        );
        // Its clearing joins the trail beside it, and its post stands at the clearing's edge, off the trail, facing it.
        let (point, _, outward) = frame(station.at);
        let clearing = station.clearing();
        assert!(clearing.distance(point) < HALF + CLEARING - 0.2);
        assert!((clearing - point).dot(outward) * station.side > 0.0);
        let post = station.post();
        assert!(post.distance(clearing) <= CLEARING + 0.05);
        for middle in &line {
            assert!(
                middle.distance(post) > HALF + PLAQUE.x * 0.5 + 0.1,
                "{index}"
            );
        }
        assert!(station.faces().dot(clearing - post) > 0.0);
        // Its anchor stands beside its clearing, opposite its post, a little above head height, facing the clearing,
        // within its radius of it.
        let place = places
            .iter()
            .find(|place| place.name == station.name)
            .unwrap();
        assert_eq!(place.kind, "station");
        assert_eq!(place.label, station.label);
        let at = Vec2::new(place.at.x, place.at.z);
        assert!(at.distance(clearing) < STATION_RADIUS);
        assert!(at.distance(post) > 1.8);
        assert_eq!(Vec2::new(place.towards.x, place.towards.z), clearing);
        assert!((1.8..2.2).contains(&place.at.y));
    }
    assert!((1.5..=2.0).contains(&STATION_RADIUS));
    // In order along the trail, spread round it, each at least 7 m from the last, and one by the lookout over the pool.
    let along = |at: usize| -> f32 {
        (0..at)
            .map(|index| line[index].distance(line[index + 1]))
            .sum()
    };
    for pair in STATIONS.windows(2) {
        assert!(pair[1].at > pair[0].at);
        assert!(along(pair[1].at) - along(pair[0].at) >= 7.0);
    }
    let lookout = places
        .iter()
        .find(|place| place.name == "pool-lookout")
        .unwrap();
    assert!(
        STATIONS
            .iter()
            .any(
                |station| Vec2::new(lookout.at.x, lookout.at.z).distance(station.clearing()) < 5.0
            )
    );
}

#[test]
fn its_posts_and_seat_keep_the_backdrop_s_views_open_and_its_places_have_room() {
    use tomori_environment::sightlines::Solid;
    let views = backdrop::views();
    for (low, high) in super::solids() {
        let solid = Solid::new(low, high);
        assert!(
            views.iter().all(|(from, to)| !solid.blocks(*from, *to)),
            "{low:?} {high:?}"
        );
    }
    // The seat lies beyond the trail at the lookout, along it, low enough to sit on.
    let (middle, along, length, radius) = seat();
    let (_, way, _) = frame(LOOKOUT);
    assert!(along.dot(way) > 0.99);
    assert!((1.2..2.0).contains(&length) && (0.35..0.55).contains(&(radius * 2.0 + 0.04)));
    for point in centreline() {
        assert!(point.distance(Vec2::new(middle.x, middle.z)) > HALF + radius + 0.1);
    }
    // Every named place is a name an anchor may have, the viewpoints then the stations, each above the trail, facing
    // level, and clear of every solid by more than a worker's reach.
    let places = places();
    let names: Vec<&str> = places.iter().map(|place| place.name).collect();
    assert_eq!(
        names,
        [
            "riverbank",
            "pool-lookout",
            "station-1",
            "station-2",
            "station-3",
            "station-4",
            "station-5"
        ]
    );
    let solids = solids();
    for place in &places {
        assert!(place.label.chars().count() <= 80);
        assert_eq!(place.at.y, place.towards.y);
        assert!(place.at.y > HEIGHT + 1.4);
        assert!(place.at.distance(place.towards) > 0.5);
        // Clear of every solid by 0.4 m about it, or a metre and more above it, as a station's above the seat.
        for solid in &solids {
            let low = Vec2::new(solid.low[0], solid.low[2]);
            let high = Vec2::new(solid.high[0], solid.high[2]);
            let at = Vec2::new(place.at.x, place.at.z);
            let apart = (low - at).max(at - high).max(Vec2::ZERO).length();
            assert!(
                apart >= 0.4 || place.at.y >= solid.high[1] + 1.0,
                "{} {solid:?}",
                place.name
            );
        }
    }
}

#[test]
fn a_pair_of_pads_joins_the_yard_and_the_lookout_off_the_way() {
    let [yard, lookout] = pads();
    // The yard's on the earth participants land on, its whole disc, away from where they arrive and the way out.
    let pad = 0.5;
    for corner in [
        Vec2::new(-pad, -pad),
        Vec2::new(pad, pad),
        Vec2::new(-pad, pad),
        Vec2::new(pad, -pad),
    ] {
        let at = Vec2::new(yard.centre.x, yard.centre.z) + corner;
        assert!(
            layout::WALKABLE
                .iter()
                .any(|walkable| walkable.contains(at)),
            "{at}"
        );
    }
    assert!(
        Vec2::new(yard.centre.x, yard.centre.z)
            .distance(Vec2::new(layout::ARRIVAL.x, layout::ARRIVAL.z))
            > 2.0
    );
    assert!((yard.centre.x - GATEWAY[0].x).abs() > HALF + pad + 0.5);
    // The lookout's in its bay, beside the trail, so that nobody walking along it steps onto it.
    let bay = bay();
    let at = Vec2::new(lookout.centre.x, lookout.centre.z);
    assert!(at.distance(bay) + pad <= 0.75);
    for point in centreline() {
        assert!(point.distance(at) >= HALF + pad, "{point}");
    }
    assert!(clearings().contains(&(bay, 0.75)));
    // Both at the trail's height, within 20 m of the yard's origin on each axis, named, facing level.
    for pad in [yard, lookout] {
        assert_eq!(pad.centre.y, HEIGHT);
        assert!(pad.centre.abs().max_element() < 20.0);
        assert!(!pad.name.is_empty() && pad.facing.y == 0.0 && pad.facing.length() > 0.1);
    }
    // Arriving at the lookout faces the pool.
    let towards = Vec2::new(POOL.x, POOL.z) - at;
    assert!(
        Vec2::new(lookout.facing.x, lookout.facing.z)
            .normalize()
            .dot(towards.normalize())
            > 0.99
    );
}

#[test]
fn the_trail_is_one_model_drawn_above_the_meadow_and_beyond_the_fence_the_same_every_time() {
    let model = model();
    assert_eq!(model.node("trail"), Some(0));
    let primitive = &model.meshes[0].primitives[0];
    assert_eq!(
        primitive.positions,
        super::model().meshes[0].primitives[0].positions
    );
    for position in &primitive.positions {
        // Above the meadow, which lies just below the yard's earth, so that nothing fights it.
        assert!(position.y > -0.006, "{position}");
        let at = Vec2::new(position.x, position.z);
        assert!(
            beyond_fence(at) > 0.0 || at.y >= FENCE.high.y - 1e-4,
            "{position}"
        );
        assert!(
            at.distance(layout::BACKDROP_CENTRE) < REACH + 0.2,
            "{position}"
        );
    }
    // Its track is lifted above the meadow, its crests above the track, and its posts reach their height.
    let highest = primitive
        .positions
        .iter()
        .map(|position| position.y)
        .fold(0.0, f32::max);
    assert!((highest - (POST_HEIGHT + 0.05)).abs() < 0.01, "{highest}");
    const { assert!(LIFT > 0.02 && EDGE > 0.02) };
    // Each plaque's text stands just proud of its board, facing its clearing, at the board's height.
    for (plaque, station) in plaques().iter().zip(&STATIONS) {
        let face = plaque.transform.transform_point3(Vec3::ZERO);
        let normal = plaque.transform.transform_vector3(Vec3::Z);
        let faces = station.faces();
        assert!(Vec2::new(normal.x, normal.z).dot(faces) > 0.999);
        assert_eq!(face.y, PLAQUE_HEIGHT);
        assert!(
            (Vec2::new(face.x, face.z).distance(station.post()) - (POST * 0.5 + 0.05)).abs() < 1e-4
        );
        assert_eq!(plaque.lines, station.plaque);
    }
}
