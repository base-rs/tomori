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
    // A kind is a short slug, an object a reference, and Tomori's own names are its own; a label is cut, not refused.
    let rack = ahead("rack/a12")
        .with_kind("rack")
        .with_label("Rack A12, alerting: CPU throttled")
        .with_object("rack/a12");
    assert_eq!(rack.checked(), Ok(()));
    assert_eq!(
        (rack.kind.as_deref(), rack.object.as_deref(), rack.placed),
        (Some("rack"), Some("rack/a12"), false)
    );
    for kind in ["", "a rack", "rack/a", &"k".repeat(KIND_BYTES + 1)] {
        let refused = ahead("aisle").with_kind(kind).checked().unwrap_err();
        assert!(refused.contains("kind"), "{kind:?}");
    }
    assert!(
        ahead("aisle")
            .with_kind("k".repeat(KIND_BYTES))
            .checked()
            .is_ok()
    );
    let object = ahead("aisle").with_object("hen marigold").checked();
    assert!(object.unwrap_err().contains("object"));
    assert!(
        ahead("tomori/wall/1")
            .checked()
            .unwrap_err()
            .contains("Tomori's own")
    );
    assert!(ahead("aisle").with_label("x".repeat(500)).checked().is_ok());
    // Sent without what it does not say, and read so from an older server.
    let bare = serde_json::to_value(ahead("aisle")).unwrap();
    for absent in ["kind", "label", "object", "placed"] {
        assert!(bare.get(absent).is_none(), "{absent}");
    }
    let mut placed = rack.clone();
    placed.placed = true;
    let json = serde_json::to_value(&placed).unwrap();
    assert_eq!(
        (json["kind"].as_str(), json["placed"].as_bool()),
        (Some("rack"), Some(true))
    );
    assert_eq!(serde_json::from_value::<Anchor>(json).unwrap(), placed);
}

#[test]
fn a_cluster_steps_back_only_once_more_than_its_hysteresis_have_left() {
    let cluster = Cluster::new(&ahead(HOME).with_extent(4.2, 1.6, 2.0));
    let compact = (1..64)
        .find(|span| cluster.arrange(*span).compact())
        .unwrap();
    // Laid out for more at once.
    assert_eq!(Cluster::settle(0, 3), 3);
    assert_eq!(Cluster::settle(compact - 1, compact), compact);
    // One coming and going at the step's bound lays it out again only once.
    let mut laid = compact - 1;
    let mut changes = 0;
    for span in [compact, compact - 1, compact, compact - 1, compact] {
        let next = Cluster::settle(laid, span);
        if cluster.arrange(next) != cluster.arrange(laid) {
            changes += 1;
        }
        laid = next;
    }
    assert_eq!(changes, 1);
    // Leaving by more than it keeps room for, it steps back, laid out for one more than stand.
    let back = Cluster::settle(laid, compact - HYSTERESIS - 1);
    assert_eq!(back, compact - 1);
    assert!(!cluster.arrange(back).compact());
    // Emptied, laid out for nobody but what it keeps room for.
    assert_eq!(Cluster::settle(back, 0), HYSTERESIS);
}

#[test]
fn a_layout_names_each_worker_s_slot_and_is_read_alike_both_ways() {
    let layout = Layout::new("rack/a12", 4)
        .with_seat("ses_1", 0)
        .with_seat("ses_2", 3);
    assert_eq!(
        (layout.slot("ses_1"), layout.slot("ses_2")),
        (Some(0), Some(3))
    );
    assert_eq!(layout.slot("ses_9"), None);
    let json = serde_json::to_value(&layout).unwrap();
    assert_eq!(json["seats"][1]["worker"], "ses_2");
    assert_eq!(serde_json::from_value::<Layout>(json).unwrap(), layout);
    // Nobody seated, none is written, and a layout of a newer server's with more is still read.
    let empty = serde_json::to_value(Layout::new("home", 0)).unwrap();
    assert!(empty.get("seats").is_none());
    let newer: Layout = serde_json::from_value(
        serde_json::json!({"anchor": "home", "span": 2, "seats": [], "also": true}),
    )
    .unwrap();
    assert_eq!(newer, Layout::new("home", 2));
    assert_eq!(Layout::default().anchor, "");
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

/// The home anchor 4 m ahead of the arrival, facing it, standing in a ring.
fn ringed() -> Anchor {
    Anchor::new(
        HOME,
        Pose::at([0.0, 1.5, -4.0]).facing_towards([0.0, 1.6, 0.0]),
    )
    .with_formation(Formation::Ring)
}

/// How far `position` stands from the point of [`ringed`] about the vertical, and its angle there from the way to the
/// arrival, round to the arrival's right.
fn about(position: [f32; 3]) -> (f32, f32) {
    let (x, z) = (position[0], position[2] + 4.0);
    (x.hypot(z), x.atan2(z))
}

#[test]
fn a_ring_stands_evenly_about_the_agent_the_sides_first_clear_of_it() {
    let cluster = Cluster::new(&ringed());
    assert_eq!(
        (cluster.formation(), cluster.radius(), cluster.bounded()),
        (Formation::Ring, Some(RING), true)
    );
    // The swarm's room and half a sign's reach, turned any way, fit within the ring.
    assert!(KEPT + WORKER.width.hypot(WORKER.depth) / 2.0 <= RING);
    let few = cluster.arrange(2);
    assert_eq!(
        (few.step(), few.compact(), few.more()),
        (Step::Ring, false, None)
    );
    // Seven to a row, three rows without room: eighteen at full size, each in a slot of its own.
    assert_eq!(few.capacity(), Some(21));
    let eighteen = cluster.slots(18);
    assert!(eighteen.iter().all(|slot| slot.scale == 1.0 && !slot.more));
    for slot in &eighteen {
        let (radius, _) = about(slot.position);
        assert!((radius - RING).abs() < 1e-4, "{slot:?}");
    }
    // The first two at the agent's right and left, as the arrival sees them, the same way round, nearer before further.
    let (right, left) = (eighteen[0].position, eighteen[1].position);
    assert!(right[0] > 1.0 && (right[0] + left[0]).abs() < 1e-4);
    assert!((right[2] - left[2]).abs() < 1e-4 && right[2] > -4.0);
    // Its first row, a little below the agent's middle, seven evenly about it, one straight behind and none before it.
    let mut row: Vec<f32> = eighteen[..7]
        .iter()
        .map(|slot| {
            assert!((slot.position[1] - (1.5 - ROW_BELOW)).abs() < 1e-5);
            about(slot.position).1
        })
        .collect();
    row.sort_by(f32::total_cmp);
    let step = std::f32::consts::TAU / 7.0;
    for pair in row.windows(2) {
        assert!((pair[1] - pair[0] - step).abs() < 1e-4, "{row:?}");
    }
    assert!(row.iter().all(|angle| angle.abs() >= step / 2.0 - 1e-4));
    assert!(
        row.iter()
            .any(|angle| (angle.abs() - std::f32::consts::PI).abs() < 1e-4)
    );
    // In order, the sides first.
    let side = |slot: &Slot| about(slot.position).1.cos().abs();
    for pair in eighteen[..7].windows(2) {
        assert!(side(&pair[0]) <= side(&pair[1]) + 1e-4);
    }
    // The row above stands a footprint higher, each place straight above the one below, as today's arc's rows do.
    let above = eighteen[7];
    assert!((above.position[1] - (1.5 - ROW_BELOW + ROW_APART)).abs() < 1e-5);
    assert!(
        (above.position[0] - right[0]).abs() < 1e-5 && (above.position[2] - right[2]).abs() < 1e-5
    );
    // Each reaches as far, turned any way, as it is seen from every side.
    let (low, high) = cluster.reach(&eighteen[0]);
    let half = WORKER.width.hypot(WORKER.depth) / 2.0;
    assert!(
        (high[0] - low[0] - 2.0 * half).abs() < 1e-4
            && (high[2] - low[2] - 2.0 * half).abs() < 1e-4
    );
    // Beyond its rows, compact, then the rest gathered into one sign at its last slot.
    let compact = cluster.arrange(22);
    assert!(compact.compact() && compact.more().is_none() && compact.step() == Step::Ring);
    let most = compact.capacity().unwrap();
    assert_eq!(most, 33);
    let over = cluster.arrange(most + 4);
    assert_eq!(over.more(), Some(most - 1));
    assert!(cluster.slot(&over, most + 2).more);
}

#[test]
fn a_ring_in_room_rises_as_its_height_allows_and_where_it_holds_none_the_room_s_steps_stand() {
    // Room for one row: seven at full size, then eleven compact, then the sign.
    let low = Cluster::new(&ringed().with_extent(4.0, 1.0, 4.0));
    assert_eq!(low.arrange(7).capacity(), Some(7));
    let compact = low.arrange(8);
    assert!(compact.compact() && compact.capacity() == Some(11));
    assert_eq!(low.arrange(12).more(), Some(10));
    // Room for four rows: twenty-eight at full size.
    let tall = Cluster::new(&ringed().with_extent(4.0, 6.0, 4.0));
    assert_eq!(tall.arrange(28).capacity(), Some(28));
    assert!(!tall.arrange(28).compact());
    // A slim room that holds one place, straight behind the agent, at full size and compact alike: the rest gather
    // into the sign there, at full size.
    let slim = Cluster::new(&ringed().with_extent(1.6, 1.0, 4.0));
    let two = slim.arrange(2);
    assert_eq!(
        (two.step(), two.compact(), two.more()),
        (Step::Ring, false, Some(0))
    );
    assert!((about(slim.slot(&two, 1).position).1.abs() - std::f32::consts::PI).abs() < 1e-4);
    // A room too narrow for the ring stands its members as the room's steps do, the ring still where it is drawn.
    let narrow = Cluster::new(&ringed().with_extent(1.0, 2.0, 8.0));
    assert_ne!(narrow.arrange(3).step(), Step::Ring);
    assert_eq!(narrow.radius(), Some(RING));
    // A ring elsewhere stands about its anchor's point, kept for whatever the anchor stands by.
    let pod = Anchor::new("pod", Pose::at([2.0, 1.2, -6.0])).with_formation(Formation::Ring);
    let pod = Cluster::new(&pod.with_extent(4.0, 2.0, 4.0));
    for slot in pod.slots(5) {
        let away = (slot.position[0] - 2.0).hypot(slot.position[2] + 6.0);
        assert!((away - RING).abs() < 1e-4, "{slot:?}");
    }
}

#[test]
fn a_ring_s_sweep_leaves_the_side_away_from_its_audience_clear() {
    // Two thirds of the ring, about the way to the arrival: the side behind the agent stays clear.
    let anchor = ringed().with_sweep(240.0);
    assert_eq!(anchor.checked(), Ok(()));
    let cluster = Cluster::new(&anchor);
    assert!((cluster.sweep() - 240f32.to_radians()).abs() < 1e-6);
    let towards = cluster.towards();
    assert!(towards[1] > 0.99, "{towards:?}");
    let arrangement = cluster.arrange(1);
    let held = arrangement.capacity().unwrap();
    // Four of the seven places in each row, those before and beside the agent, are within 120° of the way to the
    // arrival; those behind it, and the one straight behind, are not.
    assert_eq!(held, 12);
    for slot in cluster.slots(held) {
        let (_, angle) = about(slot.position);
        assert!(angle.abs() <= 120f32.to_radians() + 1e-4, "{slot:?}");
    }
    // The whole ring, or one out of bounds, takes every place; a sweep is bounded.
    assert_eq!(
        Cluster::new(&ringed().with_sweep(360.0)),
        Cluster::new(&ringed())
    );
    assert_eq!(
        Cluster::new(&ringed().with_sweep(30.0)),
        Cluster::new(&ringed())
    );
    for sweep in [30.0, 400.0, f32::NAN] {
        let refused = ringed().with_sweep(sweep).checked().unwrap_err();
        assert!(refused.contains("sweep"), "{sweep}");
    }
    assert!(valid_sweep(LEAST_SWEEP) && valid_sweep(MOST_SWEEP));
    // Sent only where it is given.
    assert!(
        serde_json::to_value(ringed())
            .unwrap()
            .get("sweep")
            .is_none()
    );
    assert_eq!(serde_json::to_value(&anchor).unwrap()["sweep"], 240.0);
    // Gathered about a point straight above it, its sweep is about the way it faces.
    let overhead = Cluster::new(&anchor).with_audience([0.0, 3.0, -4.0]);
    let facing = overhead.towards();
    assert!(facing[1] > 0.99, "{facing:?}");
}

#[test]
fn a_formation_is_sent_only_where_it_is_not_the_arc_and_one_unknown_is_laid_out_as_the_arc() {
    let arc = serde_json::to_value(ahead(HOME)).unwrap();
    assert!(arc.get("formation").is_none());
    let ring = serde_json::to_value(ringed()).unwrap();
    assert_eq!(ring["formation"], "ring");
    assert_eq!(serde_json::from_value::<Anchor>(ring).unwrap(), ringed());
    // From an older server, the arc; from a newer, one this build does not know, laid out as the arc.
    let mut newer = serde_json::to_value(ahead(HOME)).unwrap();
    newer["formation"] = "spiral".into();
    let newer: Anchor = serde_json::from_value(newer).unwrap();
    assert_eq!(newer.formation, Formation::Unknown);
    let pose = Pose::at([0.0, 1.4, -4.0]).facing_towards([0.0, 1.6, 0.0]);
    assert_eq!(Cluster::new(&newer), Cluster::home(pose, None));
    assert_eq!(
        Cluster::home(pose, None).with_formation(Formation::Unknown),
        Cluster::home(pose, None)
    );
    assert!(Formation::default().is_arc() && !Formation::Ring.is_arc());
    // The arc's home, as Sky's, is today's arc, holding any number, and no ring is drawn for it.
    let sky = Cluster::new(&ahead(HOME));
    assert_eq!((sky.radius(), sky.bounded()), (None, false));
    assert_eq!(sky.arrange(18).step(), Step::Home);
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

/// A cluster as [`any`] makes, standing in a ring, about its point, half of them in part of it.
fn any_ring(random: &mut Random) -> Cluster {
    let cluster = any(random).with_formation(Formation::Ring);
    if random.next() < 0.5 {
        cluster.with_sweep(random.between(LEAST_SWEEP, MOST_SWEEP))
    } else {
        cluster
    }
}

/// The clusters the properties below are checked over: of every formation, each from a seed of its own.
type Make = fn(&mut Random) -> Cluster;

#[test]
fn every_cluster_keeps_its_members_apart_within_its_room() {
    for (seed, make) in [
        (0x5eed_c1a5_7e12, any as Make),
        (0x5eed_c1a5_7e12 ^ 0x7269_6e67, any_ring),
    ] {
        let mut random = Random(seed);
        for _ in 0..300 {
            let cluster = make(&mut random);
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
}

/// Each member standing, by the order it arrived in, and where.
type Standing = Vec<(u32, [f32; 3])>;

#[test]
fn members_keep_their_slots_as_others_come_and_go_and_every_client_lays_them_out_alike() {
    for (seed, make) in [
        (0xc0ff_ee15_600d, any as Make),
        (0xc0ff_ee15_600d ^ 0x7269_6e67, any_ring),
    ] {
        let mut random = Random(seed);
        for _ in 0..60 {
            let cluster = make(&mut random);
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
}

#[test]
fn gaps_close_only_where_nobody_moves_who_would_not_anyway_and_never_keep_a_cluster_compact() {
    for (seed, make) in [
        (0x9a95_c105_e5ed, any as Make),
        (0x9a95_c105_e5ed ^ 0x7269_6e67, any_ring),
    ] {
        let mut random = Random(seed);
        for _ in 0..60 {
            let cluster = make(&mut random);
            let mut members: Vec<usize> = Vec::new();
            let mut held: Vec<usize> = Vec::new();
            let mut standing = cluster.arrange(0);
            for _ in 0..160 {
                // Some leave, slowly, holding their slots a while; others arrive in the lowest free slot, or, as a child
                // beside its parent may, further along.
                held.retain(|_| random.next() < 0.5);
                let arrives = random.next() < 0.55 || members.is_empty();
                if arrives {
                    let free = |slot: &usize| !members.contains(slot) && !held.contains(slot);
                    let lowest = (0..).find(free).unwrap();
                    let slot = if random.next() < 0.2 {
                        (lowest + 1 + (random.next() * 6.0) as usize..)
                            .find(free)
                            .unwrap()
                    } else {
                        lowest
                    };
                    members.push(slot);
                } else {
                    let index = (random.next() * members.len() as f32) as usize % members.len();
                    held.push(members.swap_remove(index));
                }
                let before = members.clone();
                let changed = cluster.reslot(&mut members, &held, &standing);
                assert_eq!(changed, before != members);
                // Each in a slot of its own, none of them one still held.
                for (index, slot) in members.iter().enumerate() {
                    assert!(!held.contains(slot) && !members[..index].contains(slot));
                }
                let span = members.iter().max().map_or(0, |slot| slot + 1);
                let now = cluster.arrange(span);
                // It stands as its members would in the lowest free slots, so that gaps never keep it compact.
                let lowest: Vec<usize> = (0..)
                    .filter(|slot| !held.contains(slot))
                    .take(members.len())
                    .collect();
                assert_eq!(
                    now,
                    cluster.arrange(lowest.last().map_or(0, |slot| slot + 1))
                );
                // Nobody is gathered into the sign beside a slot of its own that is free.
                if let Some(more) = now.more()
                    && members.iter().any(|slot| *slot >= more)
                {
                    assert!((0..more).all(|slot| members.contains(&slot) || held.contains(&slot)));
                }
                // Where it stands as it stood, only those that were gathered moved, besides one just arriving.
                if now == standing {
                    let stayed = before.len() - usize::from(arrives);
                    for (was, is) in before[..stayed].iter().zip(&members) {
                        assert!(
                            was == is || standing.more().is_some_and(|more| *was >= more),
                            "{cluster:?} {before:?} {members:?} {held:?} {standing:?}"
                        );
                    }
                }
                standing = now;
            }
        }
    }
}

#[test]
fn seated_with_hysteresis_gaps_close_only_as_the_cluster_passes_a_step_it_would_pass_anyway() {
    for (seed, make) in [
        (0x5ea7_ed0f_f1ce, any as Make),
        (0x5ea7_ed0f_f1ce ^ 0x7269_6e67, any_ring),
    ] {
        let mut random = Random(seed);
        for _ in 0..60 {
            let cluster = make(&mut random);
            let mut members: Vec<usize> = Vec::new();
            let mut held: Vec<usize> = Vec::new();
            let mut laid = 0;
            let mut layouts = 0;
            for _ in 0..160 {
                held.retain(|_| random.next() < 0.5);
                let arrives = random.next() < 0.55 || members.is_empty();
                if arrives {
                    let free = |slot: &usize| !members.contains(slot) && !held.contains(slot);
                    let lowest = (0..).find(free).unwrap();
                    let slot = if random.next() < 0.2 {
                        (lowest + 1 + (random.next() * 6.0) as usize..)
                            .find(free)
                            .unwrap()
                    } else {
                        lowest
                    };
                    members.push(slot);
                } else {
                    let index = (random.next() * members.len() as f32) as usize % members.len();
                    held.push(members.swap_remove(index));
                }
                let standing = cluster.arrange(laid);
                let before = members.clone();
                let next = cluster.reseat(&mut members, &held, laid);
                // Each in a slot of its own, none of them one still held, all within what it is laid out for.
                for (index, slot) in members.iter().enumerate() {
                    assert!(!held.contains(slot) && !members[..index].contains(slot));
                    assert!(*slot < next);
                }
                let now = cluster.arrange(next);
                // It stands as it stood, or as its members would in the lowest free slots, with its hysteresis.
                let packed = (0..)
                    .filter(|slot| !held.contains(slot))
                    .take(members.len())
                    .last()
                    .map_or(0, |slot| slot + 1);
                assert!(
                    now == standing || now == cluster.arrange(Cluster::settle(laid, packed)),
                    "{laid} {next} {packed}"
                );
                // Nobody is gathered into the sign beside a slot of its own that is free.
                if let Some(more) = now.more()
                    && members.iter().any(|slot| *slot >= more)
                {
                    assert!((0..more).all(|slot| members.contains(&slot) || held.contains(&slot)));
                }
                // Where it stands as it stood, only those that were gathered moved, besides one just arriving.
                if now == standing {
                    let stayed = before.len() - usize::from(arrives);
                    for (was, is) in before[..stayed].iter().zip(&members) {
                        assert!(was == is || standing.more().is_some_and(|more| *was >= more));
                    }
                } else {
                    layouts += 1;
                }
                laid = next;
            }
            // It is laid out again far less often than members come and go.
            assert!(layouts < 80, "{layouts}");
        }
    }
}
