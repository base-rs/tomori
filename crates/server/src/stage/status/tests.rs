// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The status wall: the Tomori wall, or an environment's own wall in the role, its slots shared with the environment's
//! panels there, the full wall's order, status panels waiting for a slot, and what the space kept on the Tomori wall
//! and of its edits, as an old record of spaces has them.
use super::super::published::Change;
use super::super::*;
use crate::EnvironmentWall;
use tomori_protocol::{Grid, Place, Pose, TOMORI_WALL};

fn pose(x: f32, z: f32) -> Pose {
    Pose {
        position: [x, 1.45, z],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

fn content(title: &str) -> Content {
    Content {
        title: title.into(),
        body: format!("About {title}."),
        meta: Vec::new(),
        code: None,
        slug: None,
        image: None,
    }
}

/// A farm's notice board, two slots wide and one high, facing +Z.
fn board() -> [EnvironmentWall; 1] {
    [EnvironmentWall::new(pose(4.0, -4.8)).with_grid(Grid::from((2, 1)))]
}

/// A farmyard whose notice board, `board`, is its status wall.
fn yard(board: &[EnvironmentWall]) -> Walls<'_> {
    Walls {
        environment: "farmyard",
        tomori: default_stage(),
        declared: board,
        status: Status::Own(0),
    }
}

/// Sky, its status wall the space's own, beneath the agent's home.
const SKY: Walls<'static> = Walls {
    environment: "sky",
    tomori: Pose {
        position: [0.0, 0.52, -2.3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    },
    declared: &[],
    status: Status::Beneath,
};

const BOARD: &str = "environment-0";

/// The panels shown, by title, and where each is.
fn shown(stage: &Stage, walls: &Walls<'_>) -> Vec<(String, Place)> {
    let mut shown: Vec<(String, Place)> = stage
        .view("s", walls, 0)
        .panels
        .into_iter()
        .map(|panel| (panel.title, panel.place))
        .collect();
    shown.sort_by(|a, b| a.0.cmp(&b.0));
    shown
}

fn on(wall: &str, slot: u32) -> Place {
    Place::Wall {
        wall: wall.into(),
        slot,
    }
}

/// The identity of the panel titled `title` shown in `walls`' environment.
fn id(stage: &Stage, walls: &Walls<'_>, title: &str) -> String {
    stage
        .view("s", walls, 0)
        .panels
        .into_iter()
        .find(|panel| panel.title == title)
        .map(|panel| panel.id)
        .expect("shown")
}

#[test]
fn an_environment_s_own_wall_is_the_status_wall_its_notices_and_its_own_panels_sharing_its_slots() {
    let board = board();
    let walls = yard(&board);
    let mut stage = Stage::default();
    // The Tomori wall is not shown; the board is, locked, as the status wall.
    let view = stage.view("s", &walls, 0);
    assert_eq!(view.walls.len(), 1);
    assert_eq!(view.walls[0].id, BOARD);
    assert!(view.walls[0].status && view.walls[0].is_locked());
    assert_eq!(walls.status_wall(), BOARD);
    // A notice takes the board's free slot, as a status panel, and a panel put on the board by name the next.
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    let b = stage.project_to(
        &walls,
        content("B"),
        &Destination::Wall {
            wall: BOARD.into(),
            slot: None,
        },
        0,
    );
    assert_eq!(
        shown(&stage, &walls),
        [("A".into(), on(BOARD, 0)), ("B".into(), on(BOARD, 1))]
    );
    assert!(stage.is_status(&a) && !stage.is_status(&b));
    // Full, a notice takes the oldest status panel's slot nobody arranged, which leaves, as a full wall's oldest does.
    stage.project_notice(&walls, "workers", content("C"), 0);
    assert_eq!(
        shown(&stage, &walls),
        [("B".into(), on(BOARD, 1)), ("C".into(), on(BOARD, 0))]
    );
    assert!(stage.waiting(&walls).is_empty());
    // Arranged by a participant, a status panel stays; then the board's own nobody arranged leaves for a notice.
    let c = id(&stage, &walls, "C");
    assert_eq!(
        stage.place(&walls, &c, Target::Wall(BOARD.into())),
        Ok(true)
    );
    assert!(stage.is_status(&c));
    stage.project_notice(&walls, "pairing", content("D"), 0);
    assert_eq!(
        shown(&stage, &walls),
        [("C".into(), on(BOARD, 0)), ("D".into(), on(BOARD, 1))]
    );
    // Named as the status wall, a panel goes there as a status panel, in the slot named.
    let e = stage.project_to(
        &walls,
        content("E"),
        &Destination::Wall {
            wall: TOMORI_WALL.into(),
            slot: Some(1),
        },
        0,
    );
    assert!(stage.is_status(&e));
    assert_eq!(
        shown(&stage, &walls),
        [("C".into(), on(BOARD, 0)), ("E".into(), on(BOARD, 1))]
    );
    // Every slot arranged, a notice floats just in front of the board's right end, facing as it faces, never lost.
    assert_eq!(
        stage.place(&walls, &e, Target::Wall(BOARD.into())),
        Ok(true)
    );
    let at = stage.before_end(&walls, BOARD);
    let f = stage.project_notice(&walls, "layouts", content("F"), 0);
    assert_eq!(
        stage.view("s", &walls, 0).panel(&f).expect("shown").place,
        Place::Free { pose: at }
    );
    assert!((at.position[2] - (-4.8 + 0.25)).abs() < 1e-4, "{at:?}");
    assert!(at.position[0] > 4.0 && at.position[0] < 4.7, "{at:?}");
    assert_eq!(at.orientation, [0.0, 0.0, 0.0, 1.0]);
    assert!(!stage.is_status(&f));
    // Another notice floats a slot's width further to its right, rather than inside the first.
    let g = stage.project_notice(&walls, "moved", content("G"), 0);
    let Place::Free { pose: next } = stage.view("s", &walls, 0).panel(&g).expect("shown").place
    else {
        panic!("floats");
    };
    assert!(next.position[0] > at.position[0] + 0.5, "{next:?}");
    assert_eq!(
        (next.position[1], next.position[2], next.orientation),
        (at.position[1], at.position[2], at.orientation)
    );
    // The board stays locked: its layout changes for nobody, while notices go onto it.
    assert_eq!(stage.set_wall(&walls, BOARD, grid(1, 1)), Err(LOCKED));
    // A panel the agent moves to the status wall where it already is moves nothing.
    assert_eq!(
        stage.relocate(
            &walls,
            &e,
            &Destination::Wall {
                wall: TOMORI_WALL.into(),
                slot: None,
            },
        ),
        Ok(false)
    );
}

fn grid(columns: u32, rows: u32) -> WallChange {
    WallChange {
        grid: Some(Grid::from((columns, rows))),
        ..WallChange::default()
    }
}

#[test]
fn the_environment_s_own_panels_come_first_and_a_status_panel_they_displace_waits_for_a_slot() {
    let board = board();
    let walls = yard(&board);
    let mut stage = Stage::default();
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    let b = stage.project_notice(&walls, "workers", content("B"), 0);
    // The environment publishes onto its board, full of status panels: the oldest gives up its slot and waits.
    let publish = |stage: &mut Stage, id: &str| {
        stage.publish(
            &walls,
            Publishing::new(tomori_environment::Panel::text(id, id, "x").on_wall(0, None)).unwrap(),
            0,
        )
    };
    assert_eq!(publish(&mut stage, "p"), Ok(Change::Added));
    assert_eq!(stage.waiting(&walls), [(a.clone(), "A".to_owned())]);
    assert_eq!(publish(&mut stage, "q"), Ok(Change::Added));
    assert_eq!(
        stage.waiting(&walls),
        [(b.clone(), "B".to_owned()), (a.clone(), "A".to_owned())]
    );
    // Full of its own, the board refuses another.
    assert_eq!(publish(&mut stage, "r"), Err("That wall is full."));
    // A slot freed, the newest waiting takes it.
    let q = id(&stage, &walls, "q");
    stage.dismiss(&walls, &q).unwrap();
    assert_eq!(stage.waiting(&walls), [(a.clone(), "A".to_owned())]);
    assert!(
        shown(&stage, &walls)
            .iter()
            .any(|(title, place)| title == "B" && *place == on(BOARD, 1))
    );
    // Retracted, the environment's panel frees its slot too, which the hub then settles.
    assert!(stage.retract("farmyard", "p"));
    assert!(stage.settle(&walls));
    assert!(stage.waiting(&walls).is_empty());
    // A panel put on the board by hand, onto a board of status panels, takes the oldest's slot, which waits; and the
    // board's own a participant arranged leaves only once no status panel is there to wait.
    stage.project_to(
        &walls,
        content("Hay"),
        &Destination::Free(pose(0.0, 0.0)),
        0,
    );
    let hay = id(&stage, &walls, "Hay");
    assert_eq!(
        stage.place(&walls, &hay, Target::Wall(BOARD.into())),
        Ok(true)
    );
    assert!(!stage.is_status(&hay));
    assert_eq!(stage.waiting(&walls).len(), 1);
    let corn = stage.project_to(
        &walls,
        content("Corn"),
        &Destination::Free(pose(1.0, 0.0)),
        0,
    );
    assert_eq!(
        stage.place(&walls, &corn, Target::Wall(BOARD.into())),
        Ok(true)
    );
    assert_eq!(stage.waiting(&walls).len(), 2);
    let oats = stage.project_to(
        &walls,
        content("Oats"),
        &Destination::Free(pose(2.0, 0.0)),
        0,
    );
    assert_eq!(
        stage.place(&walls, &oats, Target::Wall(BOARD.into())),
        Ok(true)
    );
    assert_eq!(
        shown(&stage, &walls)
            .into_iter()
            .filter(|(_, place)| matches!(place, Place::Wall { .. }))
            .count(),
        2
    );
    assert_eq!(stage.waiting(&walls).len(), 2);
}

#[test]
fn the_environment_s_own_panel_takes_the_slot_it_names_or_rested_in_and_a_board_grown_again_seats_those_waiting()
 {
    let board = board();
    let walls = yard(&board);
    let mut stage = Stage::default();
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    let b = stage.project_notice(&walls, "workers", content("B"), 0);
    let publish = |stage: &mut Stage, id: &str, slot| {
        stage.publish(
            &walls,
            Publishing::new(tomori_environment::Panel::text(id, id, "x").on_wall(0, slot)).unwrap(),
            0,
        )
    };
    // Naming its slot 1, which B holds, it takes that slot, and B waits rather than A, the oldest.
    assert_eq!(publish(&mut stage, "p", Some(1)), Ok(Change::Added));
    assert_eq!(
        shown(&stage, &walls),
        [("A".into(), on(BOARD, 0)), ("p".into(), on(BOARD, 1))]
    );
    assert_eq!(stage.waiting(&walls), [(b.clone(), "B".to_owned())]);
    // Its side gone, its panel rests and B takes its slot again; published again, it takes back the slot it rested in.
    assert!(stage.rest("farmyard"));
    assert!(stage.settle(&walls));
    assert!(stage.waiting(&walls).is_empty());
    assert_eq!(publish(&mut stage, "p", None), Ok(Change::Added));
    assert_eq!(
        shown(&stage, &walls),
        [("A".into(), on(BOARD, 0)), ("p".into(), on(BOARD, 1))]
    );
    assert_eq!(stage.waiting(&walls), [(b.clone(), "B".to_owned())]);
    assert!(stage.is_status(&a));
    // Unlocked and grown, its new slot takes the status panel waiting.
    assert_eq!(stage.lock_wall(&walls, BOARD, false, false), Ok(true));
    assert_eq!(stage.set_wall(&walls, BOARD, grid(3, 1)), Ok(true));
    assert!(stage.waiting(&walls).is_empty());
    assert_eq!(
        stage.view("s", &walls, 0).panel(&b).unwrap().place,
        on(BOARD, 2)
    );
}

#[test]
fn only_a_status_panel_in_the_slot_named_on_the_status_wall_gives_way_and_notices_before_a_full_board_stand_side_by_side()
 {
    let declared = [
        board()[0],
        EnvironmentWall::new(pose(-4.0, -4.8)).with_grid(Grid::from((2, 1))),
    ];
    let walls = yard(&declared);
    let mut stage = Stage::default();
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    let publish = |stage: &mut Stage, id: &str, wall, slot| {
        stage.publish(
            &walls,
            Publishing::new(tomori_environment::Panel::text(id, id, "x").on_wall(wall, slot))
                .unwrap(),
            0,
        )
    };
    // Onto another wall of its own, or a free slot of the board, nothing waits.
    assert_eq!(publish(&mut stage, "o", 1, Some(0)), Ok(Change::Added));
    assert_eq!(publish(&mut stage, "p", 0, Some(1)), Ok(Change::Added));
    assert!(stage.waiting(&walls).is_empty());
    // Naming a slot one of the board's own holds, it goes where the full board makes room, the status panel waiting.
    assert_eq!(publish(&mut stage, "q", 0, Some(1)), Ok(Change::Added));
    assert_eq!(stage.waiting(&walls), [(a, "A".to_owned())]);
    let board: Vec<(String, Place)> = shown(&stage, &walls)
        .into_iter()
        .filter(|(_, place)| matches!(place, Place::Wall { wall, .. } if wall == BOARD))
        .collect();
    assert_eq!(
        board,
        [("p".into(), on(BOARD, 1)), ("q".into(), on(BOARD, 0))]
    );
    // Every slot its environment's, notices float before it side by side, six of them, and a seventh shares the first's
    // place.
    let floated: Vec<Pose> = (0..7)
        .map(|kind| {
            let id = stage.project_notice(&walls, &format!("k{kind}"), content("N"), 0);
            match stage
                .view("s", &walls, 0)
                .panel(&id)
                .expect("shown")
                .place
                .clone()
            {
                Place::Free { pose } => pose,
                place => panic!("{place:?}"),
            }
        })
        .collect();
    for (index, pose) in floated.iter().enumerate().take(6) {
        assert!(
            floated[..index].iter().all(|other| other != pose),
            "{index}"
        );
    }
    assert_eq!(floated[6], floated[0]);
}

#[test]
fn a_status_panel_keeps_its_kind_between_the_board_s_slots_and_is_the_board_s_once_pulled_off_and_put_back()
 {
    let board = board();
    let walls = yard(&board);
    let mut stage = Stage::default();
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    // Moved between the board's slots, it is still a status panel.
    assert_eq!(
        stage.place(&walls, &a, Target::Slot(BOARD.into(), 1)),
        Ok(true)
    );
    assert!(stage.is_status(&a));
    assert_eq!(shown(&stage, &walls), [("A".into(), on(BOARD, 1))]);
    // A panel of the board's put in its slot changes places with it, each keeping its kind.
    let b = stage.project_to(&walls, content("B"), &Destination::Free(pose(0.0, 0.0)), 0);
    assert_eq!(
        stage.place(&walls, &b, Target::Slot(BOARD.into(), 0)),
        Ok(true)
    );
    assert_eq!(
        stage.place(&walls, &b, Target::Slot(BOARD.into(), 1)),
        Ok(true)
    );
    assert_eq!(
        shown(&stage, &walls),
        [("A".into(), on(BOARD, 0)), ("B".into(), on(BOARD, 1))]
    );
    assert!(stage.is_status(&a) && !stage.is_status(&b));
    // Pulled off and put back, it is the board's.
    assert_eq!(
        stage.place(&walls, &a, Target::Free(pose(1.0, 0.0))),
        Ok(true)
    );
    assert!(!stage.is_status(&a));
    assert_eq!(
        stage.place(&walls, &a, Target::Wall(BOARD.into())),
        Ok(true)
    );
    assert!(!stage.is_status(&a));
    // On the Tomori wall, a panel put there by hand is a status panel, as it always was.
    let c = stage.project_to(&SKY, content("C"), &Destination::Free(pose(0.0, 0.0)), 0);
    assert_eq!(
        stage.place(&SKY, &c, Target::Wall(TOMORI_WALL.into())),
        Ok(true)
    );
    assert!(stage.is_status(&c));
}

#[test]
fn a_board_unlocked_and_made_smaller_keeps_its_status_panels_where_they_still_fit_and_floats_the_rest()
 {
    let board = board();
    let walls = yard(&board);
    let mut stage = Stage::default();
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    let b = stage.project_notice(&walls, "workers", content("B"), 0);
    assert_eq!(stage.lock_wall(&walls, BOARD, false, false), Ok(true));
    assert_eq!(stage.set_wall(&walls, BOARD, grid(1, 1)), Ok(true));
    // A keeps its column and row, and its kind; B, without a slot, floats where its slot stood, the environment's.
    assert!(stage.is_status(&a) && !stage.is_status(&b));
    let view = stage.view("s", &walls, 0);
    assert_eq!(view.panel(&a).unwrap().place, on(BOARD, 0));
    assert_eq!(view.walls[0].slots(), 1);
    assert_ne!(view.panel(&b).unwrap().place, on(BOARD, 0));
    // Put back as the environment has it, locked, two slots wide again.
    assert_eq!(stage.lock_wall(&walls, BOARD, true, true), Ok(true));
    assert!(stage.is_status(&a));
}

/// A space's stage as an older server wrote it, with six panels on the Tomori wall, the second put there by a
/// participant, its Tomori wall made four slots wide and tilted, and a panel of the farmyard's on its notice board.
const KEPT: &str = r#"{"version":1,"next":20,"tomori":[
{"id":"panel-1","title":"One","body":"1","meta":[],"created":100,"placed":1,"place":{"kind":"wall","wall":"tomori","slot":0}},
{"id":"panel-2","title":"Two","body":"2","meta":[],"created":100,"placed":2,"place":{"kind":"wall","wall":"tomori","slot":1},"pinned":true,"arranged":true},
{"id":"panel-3","title":"Three","body":"3","meta":[],"created":100,"placed":3,"place":{"kind":"wall","wall":"tomori","slot":2}},
{"id":"panel-4","title":"Four","body":"4","meta":[],"created":100,"placed":4,"place":{"kind":"wall","wall":"tomori","slot":3}},
{"id":"panel-5","title":"Five","body":"5","meta":[],"created":100,"placed":5,"place":{"kind":"wall","wall":"tomori","slot":4}},
{"id":"panel-6","title":"Six","body":"6","meta":[],"created":100,"placed":6,"place":{"kind":"wall","wall":"tomori","slot":5}}],
"rooms":{"farmyard":{"used":7,"panels":[{"id":"panel-7","title":"Hay prices","body":"x","meta":[],"created":100,"placed":7,"place":{"kind":"wall","wall":"environment-0","slot":1}}]}},
"tomori_layout":{"grid":[6,1],"scale":0.7,"radius":2.3,"tilt":-10.0}}"#;

#[test]
fn what_a_space_kept_on_the_tomori_wall_moves_into_the_board_s_free_slots_the_rest_waiting_and_comes_back()
 {
    let board = board();
    let walls = yard(&board);
    let mut stage: Stage = serde_json::from_str(KEPT).unwrap();
    // On first opening in the farmyard, the one a participant arranged takes the board's free slot; the rest wait,
    // newest first, kept.
    assert!(stage.settle(&walls));
    assert_eq!(
        shown(&stage, &walls),
        [
            ("Hay prices".into(), on(BOARD, 1)),
            ("Two".into(), on(BOARD, 0))
        ]
    );
    let waiting: Vec<String> = stage
        .waiting(&walls)
        .into_iter()
        .map(|(_, title)| title)
        .collect();
    assert_eq!(waiting, ["Six", "Five", "Four", "Three", "One"]);
    assert_eq!(stage.tomori.len(), 6);
    // Settled, it changes nothing again, nor across a restart, those waiting kept as they wait.
    assert!(!stage.settle(&walls));
    let mut stage: Stage = serde_json::from_str(&serde_json::to_string(&stage).unwrap()).unwrap();
    assert!(!stage.settle(&walls));
    assert_eq!(stage.waiting(&walls).len(), 5);
    assert_eq!(stage.tomori.len(), 6);
    // The board's own edits stand: the Tomori wall's do not apply to it.
    let view = stage.view("s", &walls, 0);
    assert_eq!((view.walls[0].slots(), view.walls[0].tilt()), (2, 0.0));
    // A slot freed, the newest waiting takes it.
    let hay = id(&stage, &walls, "Hay prices");
    stage.dismiss(&walls, &hay).unwrap();
    assert!(
        shown(&stage, &walls)
            .iter()
            .any(|(title, place)| title == "Six" && *place == on(BOARD, 1))
    );
    // Back in Sky, every one is on the Tomori wall, those that waited in the slots they had, the space's edits standing.
    assert!(stage.settle(&SKY));
    let view = stage.view("s", &SKY, 0);
    assert_eq!(view.panels.len(), 6);
    assert_eq!(view.panel("panel-1").unwrap().place, on(TOMORI_WALL, 0));
    assert_eq!(view.panel("panel-5").unwrap().place, on(TOMORI_WALL, 4));
    assert_eq!((view.walls[0].slots(), view.walls[0].tilt()), (6, -10.0));
    assert!(view.walls[0].status);
}

#[test]
fn the_tomori_wall_s_edits_stand_over_a_status_wall_its_environment_stands_and_as_far_as_they_can_beneath_the_agent()
 {
    let strip = Layout {
        grid: Grid::from((3, 1)),
        scale: 1.0,
        aspect: None,
        radius: None,
        tilt: None,
        content_scale: None,
    };
    let door = Walls {
        environment: "hall",
        tomori: pose(3.0, -1.0),
        declared: &[],
        status: Status::At(strip),
    };
    let mut stage = Stage::default();
    // Where the environment stands it, as it lays it out.
    let view = stage.view("s", &door, 0);
    assert_eq!(view.walls[0].id, TOMORI_WALL);
    assert_eq!(view.walls[0].pose, pose(3.0, -1.0));
    assert_eq!(
        (view.walls[0].grid(), view.walls[0].scale()),
        (strip.grid, 1.0)
    );
    // There, it takes a wall's changes, rows included, and the same as the environment's is kept as nothing.
    assert_eq!(stage.set_wall(&door, TOMORI_WALL, grid(3, 2)), Ok(true));
    assert_eq!(stage.view("s", &door, 0).walls[0].slots(), 6);
    assert_eq!(stage.set_wall(&door, TOMORI_WALL, grid(3, 1)), Ok(true));
    assert_eq!(stage.tomori_layout, None);
    // Beneath the agent, rows are refused, as ever; and an edit made where the environment stood it stands here as one
    // row of as many slots as fit, of its own size.
    assert!(stage.set_wall(&SKY, TOMORI_WALL, grid(2, 2)).is_err());
    assert_eq!(stage.set_wall(&door, TOMORI_WALL, grid(2, 2)), Ok(true));
    let beneath = &stage.view("s", &SKY, 0).walls[0];
    assert_eq!(beneath.grid(), Grid::from((4, 1)));
    assert_eq!(beneath.scale(), Layout::TOMORI.scale);
    // As tightly curved as a wall may be, eight wide slots stand as six of its own, curved as they were, which fits.
    let wide = Layout {
        grid: Grid::from((8, 1)),
        scale: 1.0,
        aspect: Some(2.0),
        radius: Some(1.0),
        tilt: Some(10.0),
        content_scale: None,
    };
    let beneath = wide.beneath();
    assert_eq!(
        (
            beneath.grid,
            beneath.scale,
            beneath.aspect,
            beneath.radius,
            beneath.tilt
        ),
        (
            Grid::from((6, 1)),
            Layout::TOMORI.scale,
            None,
            Some(1.0),
            Some(10.0)
        )
    );
    let wall = beneath.apply(tomori_protocol::Wall::new(
        TOMORI_WALL,
        tomori_protocol::WallKind::Tomori,
        Pose::default(),
    ));
    assert_eq!(Stage::fits(&wall, beneath), Ok(()));
}

#[test]
fn naming_a_wall_of_its_own_that_it_does_not_declare_leaves_the_status_wall_the_space_s_own() {
    let board = board();
    let walls = Walls {
        status: Status::Own(3),
        ..yard(&board)
    };
    assert_eq!(walls.status_wall(), TOMORI_WALL);
    assert!(!walls.shared());
    let mut stage = Stage::default();
    let notice = stage.project_notice(&walls, "walls", content("A"), 0);
    let view = stage.view("s", &walls, 0);
    assert_eq!(view.walls.len(), 2);
    assert!(view.walls[0].status && !view.walls[1].status);
    assert_eq!(view.panel(&notice).unwrap().place, on(TOMORI_WALL, 0));
}

#[test]
fn a_panel_named_onto_a_board_of_arranged_status_panels_or_the_environment_s_own_takes_the_oldest_s_slot()
 {
    let board = board();
    let walls = yard(&board);
    let onto = Destination::Wall {
        wall: BOARD.into(),
        slot: None,
    };
    // Every status panel there arranged by a participant: the oldest of them waits.
    let mut stage = Stage::default();
    let a = stage.project_notice(&walls, "walls", content("A"), 0);
    let b = stage.project_notice(&walls, "workers", content("B"), 0);
    for panel in [&a, &b] {
        assert_eq!(
            stage.place(&walls, panel, Target::Wall(BOARD.into())),
            Ok(true)
        );
    }
    let x = stage.project_to(&walls, content("X"), &onto, 0);
    assert_eq!(stage.waiting(&walls), [(a, "A".to_owned())]);
    assert!(!stage.is_status(&x));
    // With the status panel left arranged, the wall's own nobody arranged leaves for the next.
    let z = stage.project_to(&walls, content("Z"), &onto, 0);
    let mut titles: Vec<String> = shown(&stage, &walls)
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    titles.sort();
    assert_eq!(titles, ["B", "Z"]);
    assert!(stage.is_status(&b) && !stage.is_status(&z));
    // Every slot holding one its environment published: none is given up, and the panel goes to the status role,
    // floating just in front of the board, as a notice does.
    let mut stage = Stage::default();
    let publish = |stage: &mut Stage, id: &str| {
        stage.publish(
            &walls,
            Publishing::new(tomori_environment::Panel::text(id, id, "x").on_wall(0, None)).unwrap(),
            0,
        )
    };
    for id in ["p", "q"] {
        assert_eq!(publish(&mut stage, id), Ok(Change::Added));
    }
    let before = stage.before_end(&walls, BOARD);
    let y = stage.project_to(&walls, content("Y"), &onto, 0);
    assert_eq!(
        stage.view("s", &walls, 0).panel(&y).expect("shown").place,
        Place::Free { pose: before }
    );
    // Beside one it published, one of the board's own a participant arranged leaves for the next, rather than the
    // environment's.
    let mut stage = Stage::default();
    assert_eq!(publish(&mut stage, "p"), Ok(Change::Added));
    let hay = stage.project_to(
        &walls,
        content("Hay"),
        &Destination::Free(pose(0.0, 0.0)),
        0,
    );
    assert_eq!(
        stage.place(&walls, &hay, Target::Wall(BOARD.into())),
        Ok(true)
    );
    stage.project_to(&walls, content("W"), &onto, 0);
    let mut titles: Vec<String> = shown(&stage, &walls)
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    titles.sort();
    assert_eq!(titles, ["W", "p"]);
}

/// A farmyard's stage in which two status panels stand in the board's first slot, and a panel of its own in the other.
const CROWDED: &str = r#"{"version":1,"next":5,"tomori":[
{"id":"panel-1","title":"Older","body":"1","meta":[],"created":100,"placed":1,"place":{"kind":"wall","wall":"environment-0","slot":0}},
{"id":"panel-2","title":"Newer","body":"2","meta":[],"created":100,"placed":2,"place":{"kind":"wall","wall":"environment-0","slot":0}}],
"rooms":{"farmyard":{"used":3,"panels":[{"id":"panel-3","title":"Hay","body":"x","meta":[],"created":100,"placed":3,"place":{"kind":"wall","wall":"environment-0","slot":1}}]}}}"#;

#[test]
fn of_two_status_panels_in_one_slot_the_older_waits() {
    let board = board();
    let walls = yard(&board);
    let mut stage: Stage = serde_json::from_str(CROWDED).unwrap();
    assert!(stage.settle(&walls));
    assert_eq!(
        stage.waiting(&walls),
        [("panel-1".to_owned(), "Older".to_owned())]
    );
    assert_eq!(
        shown(&stage, &walls),
        [("Hay".into(), on(BOARD, 1)), ("Newer".into(), on(BOARD, 0))]
    );
    // Waiting, it takes the next slot that frees.
    stage.dismiss(&walls, "panel-3").unwrap();
    assert!(stage.waiting(&walls).is_empty());
}
