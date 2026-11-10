// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// A small deterministic generator, so that the properties below are checked over many clusters, the same on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-4)
}

/// An anchor 4 m ahead of the arrival, facing it.
fn ahead(name: &str) -> Anchor {
    Anchor::new(
        name,
        Pose::at([0.0, 1.4, -4.0]).facing_towards([0.0, 1.6, 0.0]),
    )
}

#[test]
fn an_anchor_s_name_and_extent_are_bounded() {
    assert_eq!(ahead("rack/a12.b_c-d").checked(), Ok(()));
    for name in ["", "rack a12", "farmyard:hen", &"a".repeat(NAME_BYTES + 1)] {
        assert!(
            ahead(name).checked().unwrap_err().contains("name"),
            "{name}"
        );
    }
    assert!(valid_name(&"a".repeat(NAME_BYTES)));
    for (width, height, depth) in [(0.2, 1.0, 1.0), (1.0, 12.5, 1.0), (1.0, 1.0, f32::NAN)] {
        let anchor = ahead("aisle").with_extent(width, height, depth);
        assert!(anchor.checked().unwrap_err().contains("extent"));
    }
    assert!(Extent::new(LEAST_EXTENT, MOST_EXTENT, 1.0).valid());
    assert!(ahead(HOME).is_home() && !ahead("aisle").is_home());
}

#[test]
fn a_named_anchor_gathers_about_its_audience_before_it_in_its_room_or_the_default() {
    let cluster = Cluster::new(&ahead("aisle"));
    assert!(close(cluster.audience(), [0.0, 1.4, -4.0 + AUDIENCE]));
    assert_eq!(cluster.extent(), Some(ROOM));
    assert_eq!(cluster.footprint(), WORKER);
    // One stands at the point, the next to its right as the audience sees it, then to its left.
    let slots = Cluster::new(&ahead("aisle").with_extent(6.0, 2.5, 4.0)).slots(3);
    assert!(close(slots[0].position, [0.0, 1.4, -4.0]));
    assert!(slots[1].position[0] > 0.9 && slots[2].position[0] < -0.9);
    assert!(slots.iter().all(|slot| slot.scale == 1.0 && !slot.more));
    // The arc curves about the audience: those beside the point stand nearer it.
    assert!(
        slots[1].position[2] > -4.0 && (slots[1].position[2] - slots[2].position[2]).abs() < 1e-4
    );
    // The home anchor gathers about the eyes at the arrival, its point kept for the agent.
    let home = Cluster::new(&ahead(HOME).with_extent(8.0, 3.0, 4.0));
    assert_eq!(home.audience(), EYES);
    let first = home.slots(1)[0].position;
    assert!(first[0].abs() >= KEPT, "{first:?}");
}

#[test]
fn home_without_room_is_today_s_arc_for_any_number() {
    let pose = Pose::at([0.0, 1.4, -2.3]).facing_towards(EYES);
    let cluster = Cluster::home(pose, None);
    let arrangement = cluster.arrange(64);
    assert_eq!(arrangement.step(), Step::Home);
    assert_eq!((arrangement.capacity(), arrangement.more()), (None, None));
    assert!(!arrangement.compact());
    for slot in 0..64 {
        let at = cluster.slot(&arrangement, slot);
        assert_eq!(
            at.position,
            home_arc(pose.position, [0.0, 0.0, 1.0], EYES, slot)
        );
        assert!(!at.more && at.scale == 1.0);
    }
    // Its first slot stands 55⅔° to the left of the agent, 2.3 m from the eyes, 0.25 m below the agent's middle.
    let first = home_arc(pose.position, [0.0, 0.0, 1.0], EYES, 0);
    assert!((first[0].atan2(-first[2]).to_degrees() + 40.0 * ARC_RADIUS / 2.3).abs() < 1e-3);
    assert!((first[1] - (1.4 - ROW_BELOW)).abs() < 1e-5);
    // Slots alternate sides, two columns and two rows to a tier.
    assert_eq!(lane(0), (-1.0, 0, 0));
    assert_eq!(lane(1), (1.0, 0, 0));
    assert_eq!(lane(4), (-1.0, 0, 1));
    assert_eq!(lane(32), (-1.0, 0, 2));
    assert!((column_angle(15).to_degrees() - 320.0).abs() < 1e-3);
    // Straight above the eyes, it faces them from the way it faces; facing nowhere, from ahead of home.
    let above = home_arc([0.0, 2.6, 0.0], [1.0, 0.0, 0.0], EYES, 0);
    assert!(above[0] < -1.0, "{above:?}");
    let nowhere = home_arc([0.0, 2.6, 0.0], [0.0; 3], EYES, 1);
    assert!(nowhere[2] < -1.0, "{nowhere:?}");
    // Never nearer the floor than its lowest.
    assert_eq!(
        home_arc([0.0, 0.2, -3.0], [0.0, 0.0, 1.0], EYES, 0)[1],
        LOWEST
    );
}

#[test]
fn home_with_room_keeps_today_s_arc_while_it_fits_then_fits_an_arc_then_compacts() {
    let pose = Pose::at([0.0, 1.4, -2.3]).facing_towards(EYES);
    let cluster = Cluster::home(pose, Some(Extent::new(9.0, 3.2, 3.4)));
    // A few workers stand where today's arc puts them.
    let few = cluster.arrange(2);
    assert_eq!(few.step(), Step::Home);
    assert_eq!(
        cluster.slot(&few, 1).position,
        home_arc(pose.position, cluster.facing(), EYES, 1)
    );
    // More than today's arc holds within the room: an arc fitted to it, a footprint apart.
    let fitted = (1..=64)
        .map(|span| cluster.arrange(span))
        .find(|arrangement| arrangement.step() != Step::Home)
        .unwrap();
    assert!(!fitted.compact() && fitted.more().is_none());
    assert_eq!(fitted.step(), Step::Arc);
    let held = fitted.capacity().unwrap();
    // Beyond what the arc holds, a grid; beyond that, compact; beyond that, the rest in one sign at its end.
    let grid = cluster.arrange(held + 1);
    assert_eq!((grid.step(), grid.compact()), (Step::Grid, false));
    let compact = cluster.arrange(grid.capacity().unwrap() + 1);
    assert!(compact.compact() && compact.more().is_none());
    let most = compact.capacity().unwrap();
    assert!(most > held);
    let over = cluster.arrange(most + 5);
    let more = over.more().unwrap();
    assert_eq!(more, most - 1);
    assert_eq!(over.capacity(), Some(more));
    let gathered = cluster.slot(&over, more + 3);
    assert!(gathered.more && gathered.scale == COMPACT);
    assert_eq!(gathered.position, cluster.slot(&over, more).position);
    assert!(!cluster.slot(&over, more - 1).more);
}

#[test]
fn an_aisle_seen_from_its_end_is_a_grid_back_along_its_depth() {
    let aisle = ahead("aisle").with_extent(2.2, 2.0, 8.0);
    let cluster = Cluster::new(&aisle);
    let arrangement = cluster.arrange(6);
    assert_eq!(arrangement.step(), Step::Grid);
    let slots = cluster.slots(6);
    // Further back along its depth, behind the point, half a column aside.
    assert!(slots.iter().any(|slot| slot.position[2] < -4.5));
    assert!(
        slots
            .iter()
            .any(|slot| (slot.position[0].abs() - WORKER.width / 2.0).abs() < 1e-4)
    );
    // A wide, shallow room is an arc.
    let shelf = Cluster::new(&ahead("shelf").with_extent(6.0, 1.0, 1.0));
    assert_eq!(shelf.arrange(3).step(), Step::Arc);
}

#[test]
fn a_room_too_small_for_one_keeps_one_where_the_first_stands_and_the_rest_in_its_sign() {
    let cupboard = Cluster::new(&ahead("cupboard").with_extent(0.3, 0.3, 0.3));
    let one = cupboard.arrange(1);
    assert!(one.compact() && one.more().is_none());
    assert!(close(cupboard.slot(&one, 0).position, [0.0, 1.4, -4.0]));
    let three = cupboard.arrange(3);
    assert_eq!(three.more(), Some(0));
    assert!(cupboard.slot(&three, 2).more && cupboard.slot(&three, 0).more);
    // At home, beside what the point keeps for the agent.
    let pose = Pose::at([0.0, 1.4, -2.3]).facing_towards(EYES);
    let home = Cluster::home(pose, Some(Extent::new(0.3, 0.3, 0.3)));
    let first = home.slots(1)[0].position;
    assert!((first[0] - (KEPT + WORKER.width * COMPACT / 2.0)).abs() < 1e-4);
    // A slot past an arrangement's places, as for one laid out for fewer, stands in its sign.
    let arrangement = home.arrange(1);
    assert!(home.slot(&arrangement, 4).more);
    // No span is one.
    assert_eq!(home.arrange(0), home.arrange(1));
}

#[test]
fn panels_and_the_agent_cluster_by_their_own_footprints() {
    let anchor = ahead("console").with_extent(4.0, 1.0, 1.0);
    let panels = Cluster::new(&anchor).with_footprint(PANEL);
    let slots = panels.slots(3);
    let apart = (slots[0].position[0] - slots[1].position[0])
        .hypot(slots[0].position[2] - slots[1].position[2]);
    assert!((apart - PANEL.width).abs() < 1e-3, "{apart}");
    let agent = Cluster::new(&anchor).with_footprint(AGENT).slots(2);
    let apart = (agent[0].position[0] - agent[1].position[0])
        .hypot(agent[0].position[2] - agent[1].position[2]);
    assert!((apart - AGENT.width).abs() < 1e-3, "{apart}");
    assert_eq!(
        WORKER.scaled(2.0),
        Footprint::new(2.1, ROW_APART * 2.0, 0.6, 0.3)
    );
}

#[test]
fn a_member_s_reach_holds_it_facing_whoever_it_gathers_about() {
    let cluster = Cluster::new(&ahead("aisle"));
    let slot = cluster.slots(1)[0];
    let (low, high) = cluster.reach(&slot);
    assert!(close(low, [-WORKER.width / 2.0, 1.4 - WORKER.base, -4.15]));
    assert!(close(
        high,
        [WORKER.width / 2.0, 1.4 - WORKER.base + WORKER.height, -3.85]
    ));
    // Turned a quarter, it reaches further along than across.
    let side = Cluster::new(&ahead("aisle")).with_audience([3.0, 1.4, -4.0]);
    let (low, high) = side.reach(&side.slots(1)[0]);
    assert!(high[2] - low[2] > high[0] - low[0]);
    // Gathered about a point straight above it, it faces along +Z, and arcs the way the anchor faces them from.
    let overhead =
        Cluster::new(&ahead("aisle").with_extent(6.0, 2.0, 2.0)).with_audience([0.0, 3.0, -4.0]);
    let (low, high) = overhead.reach(&overhead.slots(1)[0]);
    assert!(close(low, [-WORKER.width / 2.0, 1.4 - WORKER.base, -4.15]) && high[0] > low[0]);
    assert_eq!(overhead.slots(3).len(), 3);
}

#[test]
fn rows_stand_at_the_point_then_alternately_above_and_below() {
    assert_eq!(
        (0..5).map(row).collect::<Vec<_>>(),
        [0.0, 1.0, -1.0, 2.0, -2.0]
    );
}

/// A cluster of an anchor anywhere within reach, turned any way, of any room, gathered about whoever watches from
/// anywhere near, home or not.
fn any(random: &mut Random) -> Cluster {
    let position = [
        random.between(-15.0, 15.0),
        random.between(0.5, 3.0),
        random.between(-15.0, 15.0),
    ];
    let pose = Pose::upright(position, random.between(-3.2, 3.2));
    let extent = Extent::new(
        random.between(LEAST_EXTENT, MOST_EXTENT),
        random.between(LEAST_EXTENT, 6.0),
        random.between(LEAST_EXTENT, MOST_EXTENT),
    );
    let home = random.next() < 0.4;
    let anchor = Anchor::new(if home { HOME } else { "anchor" }, pose).with_extent(
        extent.width,
        extent.height,
        extent.depth,
    );
    let cluster = Cluster::new(&anchor);
    if home {
        cluster
    } else {
        let before = pose.carry(&Pose::at([
            random.between(-2.0, 2.0),
            0.2,
            random.between(1.0, 6.0),
        ]));
        cluster.with_audience(before.position)
    }
}

#[test]
fn every_cluster_keeps_its_members_apart_within_its_room() {
    let mut random = Random(0x5eed_c1a5_7e12);
    for _ in 0..300 {
        let cluster = any(&mut random);
        let extent = cluster.extent().unwrap();
        for span in [1, 2, 3, 5, 8, 13, 18, 30, 64] {
            let arrangement = cluster.arrange(span);
            let slots: Vec<Slot> = (0..span)
                .map(|slot| cluster.slot(&arrangement, slot))
                .filter(|slot| !slot.more)
                .collect();
            // Those beyond its own slots gather into one sign, only beyond what the compact step holds.
            assert_eq!(
                slots.len(),
                span.min(arrangement.capacity().unwrap_or(span))
            );
            for (index, slot) in slots.iter().enumerate() {
                // Within its room, except one alone in a room too small for it.
                assert!(
                    cluster.within(&extent, slot.position, slot.scale) || slots.len() == 1,
                    "{cluster:?} {span} {index}"
                );
                let footprint = cluster.footprint().scaled(slot.scale);
                for other in &slots[..index] {
                    let level = (slot.position[0] - other.position[0])
                        .hypot(slot.position[2] - other.position[2]);
                    let rise = (slot.position[1] - other.position[1]).abs();
                    assert!(
                        level >= footprint.width - 1e-3 || rise >= footprint.height - 1e-3,
                        "{cluster:?} {span} {index}: {level} {rise}"
                    );
                }
            }
        }
    }
}

/// Each member standing, by the order it arrived in, and where.
type Standing = Vec<(u32, [f32; 3])>;

#[test]
fn members_keep_their_slots_as_others_come_and_go_and_every_client_lays_them_out_alike() {
    let mut random = Random(0xc0ff_ee15_600d);
    for _ in 0..60 {
        let cluster = any(&mut random);
        // Arrivals take the lowest free slot; departures free theirs.
        let mut taken: Vec<Option<u32>> = Vec::new();
        let mut next = 0;
        let mut before: Option<(Arrangement, Standing)> = None;
        for _ in 0..120 {
            if random.next() < 0.6 || taken.iter().all(Option::is_none) {
                match taken.iter().position(Option::is_none) {
                    Some(free) => taken[free] = Some(next),
                    None => taken.push(Some(next)),
                }
                next += 1;
            } else {
                let live: Vec<usize> = (0..taken.len())
                    .filter(|slot| taken[*slot].is_some())
                    .collect();
                taken[live[(random.next() * live.len() as f32) as usize % live.len()]] = None;
            }
            let span = taken
                .iter()
                .rposition(Option::is_some)
                .map_or(0, |last| last + 1);
            let arrangement = cluster.arrange(span);
            let standing: Standing = taken
                .iter()
                .enumerate()
                .filter_map(|(slot, member)| {
                    member.map(|member| (member, cluster.slot(&arrangement, slot).position))
                })
                .collect();
            // Laid out again from scratch, as another client or the server would, it is the same to the bit.
            let again = cluster.arrange(span);
            assert_eq!(again, arrangement);
            if let Some((previous, stood)) = &before
                && *previous == arrangement
            {
                // Nobody who stayed moved.
                for (member, position) in stood {
                    if let Some((_, now)) = standing.iter().find(|(other, _)| other == member) {
                        assert_eq!(now, position);
                    }
                }
            }
            before = Some((arrangement, standing));
        }
    }
}
