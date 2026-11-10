// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Walls and floating panels stand above the floor, however they are placed: by a participant carrying or dropping
//! them, by their handles, by the agent, snapped into a group, or as they were kept. See `design/projection.md`.
use super::*;
use tomori_protocol::{Align, Attach, Edge, wall as layout};

/// Sky, with its Tomori wall to one side and an environment wall of its own.
const SKY: Walls<'static> = Walls {
    environment: "sky",
    tomori: Pose {
        position: [2.0, 1.5, -2.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    },
    declared: &[crate::EnvironmentWall::new(Pose {
        position: [-4.0, 1.5, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    })],
    status: Status::Beneath,
};

/// Facing home from `z` ahead, at `x`, its middle at `y`.
fn at(x: f32, y: f32, z: f32) -> Pose {
    Pose::upright_facing([x, y, z], [x, y, 0.0])
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

fn shown(stage: &Stage, wall: &str) -> Wall {
    stage.shown_wall(&SKY, wall).expect("the wall is shown")
}

/// Whether `wall`'s lowest edge stands just its clearance above the floor.
fn on_the_floor(wall: &Wall) -> bool {
    (wall.lowest() - FLOOR - layout::FLOOR_CLEARANCE).abs() < 1e-3
}

/// Where the panel `panel` floats.
fn floating(stage: &Stage, panel: &str) -> Pose {
    match stage
        .view("s", &SKY, 0)
        .panel(panel)
        .expect("shown")
        .place
        .clone()
    {
        Place::Free { pose } => pose,
        place => panic!("{panel} is not floating: {place:?}"),
    }
}

#[test]
fn a_wall_added_or_moved_through_the_floor_stands_on_it_however_it_tilts_and_curves() {
    let mut stage = Stage::default();
    // Pushed through the floor while carried, as on the headset in the farmyard: centred 1.26 m under the ground.
    let sunk = at(0.0, -1.26, -2.0);
    let added = stage.add_wall(&SKY, &sunk, None).unwrap();
    let wall = shown(&stage, &added);
    assert!(on_the_floor(&wall), "{}", wall.lowest());
    // Raised straight up, where it was, facing as it faced.
    assert_eq!((wall.pose.position[0], wall.pose.position[2]), (0.0, -2.0));
    assert_eq!(wall.pose.orientation, sunk.orientation);
    // Above the floor, a wall stands where it is put.
    let high = at(3.0, 2.0, -3.0);
    let other = stage.add_wall(&SKY, &high, None).unwrap();
    assert_eq!(shown(&stage, &other).pose, high);
    // Carried down through the floor, as an older client may send it, it stops on it.
    let change = |pose: Pose| WallChange {
        pose: Some(pose),
        ..WallChange::default()
    };
    assert_eq!(
        stage.set_wall(&SKY, &other, change(at(3.0, -0.3, -3.0))),
        Ok(true)
    );
    assert!(on_the_floor(&shown(&stage, &other)));
    // Put down there again, nothing changes.
    assert_eq!(
        stage.set_wall(&SKY, &other, change(at(3.0, -0.3, -3.0))),
        Ok(false)
    );
    // Tilted, its lowest edge, nearer than upright, stands on the floor; so does a curved wall's, by its label.
    let tilted = WallChange {
        tilt: Some(Some(40.0)),
        ..change(at(3.0, -2.0, -3.0))
    };
    assert_eq!(stage.set_wall(&SKY, &other, tilted), Ok(true));
    let wall = shown(&stage, &other);
    assert_eq!(wall.tilt(), 40.0);
    assert!(on_the_floor(&wall));
    let curved = WallChange {
        radius: Some(Some(1.5)),
        tilt: Some(None),
        ..change(at(3.0, -2.0, -3.0))
    };
    assert_eq!(stage.set_wall(&SKY, &other, curved), Ok(true));
    let wall = shown(&stage, &other);
    assert_eq!(wall.radius(), Some(1.5));
    assert!(on_the_floor(&wall));
    // Grown by its handles about its middle, standing low, it rises rather than reaching through the floor.
    let low = stage.add_wall(&SKY, &at(-3.0, 0.3, -3.0), None).unwrap();
    assert!(on_the_floor(&shown(&stage, &low)));
    let grown = WallChange {
        grid: Some(Grid::from((3, 4))),
        scale: Some(1.5),
        ..WallChange::default()
    };
    assert_eq!(stage.set_wall(&SKY, &low, grown), Ok(true));
    let wall = shown(&stage, &low);
    assert_eq!(wall.grid(), Grid::from((3, 4)));
    assert!(on_the_floor(&wall));
    // Added tilted and curved by the agent, as `add_wall` and `create_layout` lay a wall out, it stands on the floor.
    for layout in [
        Layout {
            tilt: Some(-30.0),
            ..Layout::FLAT
        },
        Layout {
            radius: Some(2.0),
            ..Layout::FLAT
        },
    ] {
        let id = stage
            .add_wall(&SKY, &at(6.0, -0.5, -4.0), Some(layout))
            .unwrap();
        assert!(on_the_floor(&shown(&stage, &id)), "{layout:?}");
        stage.remove_wall(&SKY, &id).unwrap();
    }
    // Far beyond reach, a wall is still refused, as ever, rather than brought nearer.
    assert!(stage.add_wall(&SKY, &at(0.0, 1.5, -30.0), None).is_err());
}

#[test]
fn an_environment_s_own_wall_moved_through_the_floor_stands_on_it_for_the_space() {
    let mut stage = Stage::default();
    stage
        .lock_wall(&SKY, "environment-0", false, false)
        .unwrap();
    let moved = WallChange {
        pose: Some(at(-4.0, -1.0, -4.0)),
        ..WallChange::default()
    };
    assert_eq!(stage.set_wall(&SKY, "environment-0", moved), Ok(true));
    let wall = shown(&stage, "environment-0");
    assert!(on_the_floor(&wall));
    assert_eq!(stage.rooms["sky"].moved["environment-0"], wall.pose);
    // The status wall is never moved: it stands where its environment puts it.
    let carried = WallChange {
        pose: Some(at(2.0, -1.0, -2.0)),
        ..WallChange::default()
    };
    assert!(stage.set_wall(&SKY, TOMORI_WALL, carried).is_err());
}

#[test]
fn a_group_carried_or_snapped_through_the_floor_stands_on_it_as_a_whole() {
    let mut stage = Stage::default();
    let first = stage.add_wall(&SKY, &at(-1.0, 1.4, -2.4), None).unwrap();
    let second = stage.add_wall(&SKY, &at(2.0, 1.4, -2.4), None).unwrap();
    let below = Some(Attach::new(&first, Edge::Bottom, Align::Start));
    assert_eq!(
        stage.attach_wall(&SKY, &second, below, false, None),
        Ok(true)
    );
    let apart = |stage: &Stage| {
        let (a, b) = (shown(stage, &first), shown(stage, &second));
        [0, 1, 2].map(|axis| a.pose.position[axis] - b.pose.position[axis])
    };
    let before = apart(&stage);
    // The lower wall of the group reaches the floor first; carried down by the upper, the group stops on it.
    let carried = WallChange {
        pose: Some(at(-1.0, -0.5, -2.4)),
        ..WallChange::default()
    };
    assert_eq!(stage.set_wall(&SKY, &first, carried), Ok(true));
    let (upper, lower) = (shown(&stage, &first), shown(&stage, &second));
    assert!(on_the_floor(&lower), "{}", lower.lowest());
    assert!(upper.lowest() > lower.lowest());
    // It stays as it was laid out, joined.
    for (axis, apart) in apart(&stage).into_iter().enumerate() {
        assert!((apart - before[axis]).abs() < 1e-4, "{axis}");
    }
    assert_eq!(upper.group, lower.group);
    assert!(upper.group.is_some());
    // Taken out of its group and left through the floor, a wall stands on it alone.
    assert_eq!(
        stage.attach_wall(&SKY, &second, None, false, Some(at(2.0, -3.0, -2.4))),
        Ok(true)
    );
    let alone = shown(&stage, &second);
    assert!(alone.group.is_none() && on_the_floor(&alone));
    // Snapped beneath a wall standing low, the group it makes rises, as a whole, until its lowest wall clears it.
    let low = stage.add_wall(&SKY, &at(6.0, 0.4, -2.4), None).unwrap();
    let under = Some(Attach::new(&low, Edge::Bottom, Align::Start));
    assert_eq!(
        stage.attach_wall(&SKY, &second, under, false, None),
        Ok(true)
    );
    let (top, bottom) = (shown(&stage, &low), shown(&stage, &second));
    assert!(on_the_floor(&bottom));
    assert!(top.pose.position[1] > 0.4 + 0.1);
}

#[test]
fn a_panel_left_or_floated_below_the_floor_floats_above_it() {
    let mut stage = Stage::default();
    let panel = stage.project(&SKY, content("Dropped"), None, 0);
    // Let go below the floor, as an older client may leave it, it floats with its middle high enough to clear it.
    let low = at(0.5, -0.8, -1.5);
    assert_eq!(stage.place(&SKY, &panel, Target::Free(low)), Ok(true));
    let pose = floating(&stage, &panel);
    assert_eq!(pose.position, [0.5, FLOOR + layout::PANEL_LOWEST, -1.5]);
    assert_eq!(pose.orientation, low.orientation);
    // Left there again, nothing changes; left high, it floats where it is left.
    assert_eq!(stage.place(&SKY, &panel, Target::Free(low)), Ok(false));
    let high = at(0.5, 1.2, -1.5);
    assert_eq!(stage.place(&SKY, &panel, Target::Free(high)), Ok(true));
    assert_eq!(floating(&stage, &panel), high);
    // The agent's panels, floated where it says, beside or in front of someone sitting in a pit, or at an anchor.
    let free = stage.project_to(
        &SKY,
        content("Free"),
        &Destination::Free(at(1.0, -2.0, -2.0)),
        0,
    );
    assert_eq!(floating(&stage, &free).position[1], layout::PANEL_LOWEST);
    let pit = at(0.0, -1.5, 1.0);
    let beside = stage.project_to(
        &SKY,
        content("Beside"),
        &Destination::Beside {
            head: pit,
            solids: Vec::new(),
        },
        0,
    );
    assert_eq!(floating(&stage, &beside).position[1], layout::PANEL_LOWEST);
    let anchored = stage.project_to(
        &SKY,
        content("Anchored"),
        &Destination::Anchor {
            anchor: "pen".into(),
            pose: at(-1.0, -0.5, -3.0),
        },
        0,
    );
    let pose = floating(&stage, &anchored);
    assert_eq!(pose.position[1], layout::PANEL_LOWEST);
    // Still at its anchor, as it floats where its anchor put it.
    assert_eq!(
        stage.anchored("sky"),
        [(anchored.clone(), "pen".to_owned(), pose)]
    );
    // Moved by the agent below the floor, it floats above it.
    let moved = Destination::Free(at(2.0, -1.0, -2.0));
    assert_eq!(stage.relocate(&SKY, &free, &moved), Ok(true));
    assert_eq!(
        floating(&stage, &free).position,
        [2.0, layout::PANEL_LOWEST, -2.0]
    );
}

#[test]
fn panels_a_low_wall_gives_up_float_above_the_floor() {
    let mut stage = Stage::default();
    // A wall standing on the floor, its lowest row of slots near it.
    let layout = Layout {
        grid: Grid::from((2, 2)),
        ..Layout::FLAT
    };
    let wall = stage
        .add_wall(&SKY, &at(0.0, 0.0, -3.0), Some(layout))
        .unwrap();
    assert!(on_the_floor(&shown(&stage, &wall)));
    let low = stage.project_to(
        &SKY,
        content("Low"),
        &Destination::Wall {
            wall: wall.clone(),
            slot: Some(3),
        },
        0,
    );
    let slot = shown(&stage, &wall).slot_pose(3).unwrap();
    assert!(slot.position[1] < layout::PANEL_LOWEST);
    // Given fewer rows, the panel in its lowest row floats where its slot stood, above the floor.
    let fewer = WallChange {
        grid: Some(Grid::from((1, 1))),
        ..WallChange::default()
    };
    let other = stage.project_to(
        &SKY,
        content("Other"),
        &Destination::Wall {
            wall: wall.clone(),
            slot: Some(0),
        },
        0,
    );
    assert_eq!(stage.set_wall(&SKY, &wall, fewer), Ok(true));
    let pose = floating(&stage, &low);
    assert_eq!(pose.position[1], layout::PANEL_LOWEST);
    // Removed, the wall leaves its panels floating where their slots stood, above the floor.
    let grown = WallChange {
        grid: Some(Grid::from((2, 2))),
        ..WallChange::default()
    };
    assert_eq!(stage.set_wall(&SKY, &wall, grown), Ok(true));
    stage
        .place(&SKY, &low, Target::Slot(wall.clone(), 2))
        .unwrap();
    stage.remove_wall(&SKY, &wall).unwrap();
    assert_eq!(floating(&stage, &low).position[1], layout::PANEL_LOWEST);
    assert!(floating(&stage, &other).position[1] >= layout::PANEL_LOWEST);
}

/// A space's stage as a server wrote it before walls and panels were kept above the floor, in the farmyard: a wall an
/// owner added pushed through the floor and locked, a group of two below it, the environment's own wall moved below it,
/// and panels floating below it, one at an anchor.
const SUNK: &str = r#"{"version":1,"next":20,"rooms":{"sky":{"used":20,
"walls":[
 {"id":"wall-1","pose":{"position":[0.0,-1.26,-2.0],"orientation":[0.0,0.0,0.0,1.0]},"locked":true},
 {"id":"wall-2","pose":{"position":[4.0,-0.5,-2.0],"orientation":[0.0,0.0,0.0,1.0]}},
 {"id":"wall-3","pose":{"position":[4.0,-1.29,-2.0],"orientation":[0.0,0.0,0.0,1.0]},"attach":{"to":"wall-2","edge":"bottom","align":"start"}},
 {"id":"wall-4","pose":{"position":[-4.0,2.0,-2.0],"orientation":[0.0,0.0,0.0,1.0]}}],
"moved":{"environment-0":{"position":[-4.0,-2.0,-4.0],"orientation":[0.0,0.0,0.0,1.0]}},
"unlocked":["environment-0"],
"panels":[
 {"id":"panel-5","title":"Sunk","body":"","created":1,"placed":5,"place":{"kind":"free","pose":{"position":[1.0,-3.0,-1.0],"orientation":[0.0,0.0,0.0,1.0]}},"pinned":true},
 {"id":"panel-6","title":"At the pen","body":"","created":1,"placed":6,"place":{"kind":"free","pose":{"position":[1.0,-1.0,-1.0],"orientation":[0.0,0.0,0.0,1.0]}},"anchor":["pen",{"position":[1.0,-1.0,-1.0],"orientation":[0.0,0.0,0.0,1.0]}]},
 {"id":"panel-7","title":"Held","body":"","created":1,"placed":7,"place":{"kind":"free","pose":{"position":[1.0,1.4,-1.0],"orientation":[0.0,0.0,0.0,1.0]}}},
 {"id":"panel-8","title":"On the wall","body":"","created":1,"placed":8,"place":{"kind":"wall","wall":"wall-4","slot":0}}]
}}}"#;

#[test]
fn what_was_kept_through_the_floor_is_raised_above_it_once_on_reading_it() {
    let mut stage: Stage = serde_json::from_str(SUNK).unwrap();
    let before = stage.view("s", &SKY, 0);
    assert!(stage.ground(&SKY));
    let view = stage.view("s", &SKY, 0);
    let wall = |id: &str| view.wall(id).unwrap();
    // Every wall the space placed stands on or above the floor, a locked one too, since none was put there to stay.
    for id in ["wall-1", "wall-3", "environment-0"] {
        assert!(on_the_floor(wall(id)), "{id}: {}", wall(id).lowest());
    }
    assert!(wall("wall-1").is_locked());
    assert!(wall("wall-2").lowest() > wall("wall-3").lowest());
    // The group is raised as a whole, staying joined; a wall above the floor stays where it stood.
    let lift = |id: &str| wall(id).pose.position[1] - before.wall(id).unwrap().pose.position[1];
    assert!((lift("wall-2") - lift("wall-3")).abs() < 1e-4 && lift("wall-2") > 0.0);
    assert_eq!(wall("wall-2").group, wall("wall-3").group);
    assert_eq!(wall("wall-4").pose, before.wall("wall-4").unwrap().pose);
    // Floating panels float above it, the one at an anchor still at it; one held above it stays.
    assert_eq!(
        floating(&stage, "panel-5").position,
        [1.0, layout::PANEL_LOWEST, -1.0]
    );
    let pen = floating(&stage, "panel-6");
    assert_eq!(pen.position[1], layout::PANEL_LOWEST);
    assert_eq!(
        stage.anchored("sky"),
        [("panel-6".to_owned(), "pen".to_owned(), pen)]
    );
    assert_eq!(floating(&stage, "panel-7").position[1], 1.4);
    assert_eq!(
        stage.view("s", &SKY, 0).panel("panel-8").unwrap().place,
        Place::Wall {
            wall: "wall-4".into(),
            slot: 0
        }
    );
    // Raised once, it is kept so: read again, nothing more moves.
    let kept: Stage = serde_json::from_str(&serde_json::to_string(&stage).unwrap()).unwrap();
    assert_eq!(kept, stage);
    assert!(!stage.ground(&SKY));
    // An environment the space keeps nothing for has nothing to raise.
    assert!(!stage.ground(&Walls {
        environment: "farmyard",
        ..SKY
    }));
}
