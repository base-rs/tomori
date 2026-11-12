// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hand menu's tests: when it shows on the palm, its sections and rows, and choosing from it.

use super::*;
use tomori_protocol::Joint;

fn hand(wrist: Vec3, index: Vec3, little: Vec3) -> Hand {
    let mut joints = [None; crate::hands::JOINT_COUNT];
    for (place, position) in [
        (WRIST, wrist),
        (INDEX_KNUCKLE, index),
        (LITTLE_KNUCKLE, little),
    ] {
        joints[place] = Some(Joint {
            position: position.to_array(),
            radius: 0.01,
        });
    }
    Hand { joints }
}

/// A tracked hand on `side` with its palm up and its fingers forward: the index finger is on the thumb's side, left for a left hand and right for a right one.
fn hand_palm_up(side: Side) -> Hand {
    let thumb = -palm_sign(side) * 0.03;
    hand(
        Vec3::ZERO,
        Vec3::new(thumb, 0.0, -0.09),
        Vec3::new(-thumb, 0.0, -0.08),
    )
}

/// A controller held in the hand on `side`, low in front, its -Z forward and its +Y up in a neutral grip, turned by `turn`.
fn grip(side: Side, turn: Quat) -> controller::Pose {
    controller::Pose {
        position: [palm_sign(side) * -0.2, 1.1, -0.35],
        orientation: turn.to_array(),
    }
}

/// Rolling either hand outwards about the forward axis by 90° turns its palm up.
fn rolled_up(side: Side) -> Quat {
    Quat::from_rotation_z(palm_sign(side) * std::f32::consts::FRAC_PI_2)
}

/// The palm of the controller on `side` turned up, then tilted by `tilt`.
fn up_tilted(side: Side, tilt: Quat) -> Option<Palm> {
    Palm::from_controller(side, &grip(side, tilt * rolled_up(side)))
}

/// Whether `palm`, held for the dwell from hidden, shows.
fn shows(palm: Option<Palm>) -> bool {
    let start = Instant::now();
    let mut palm_up = PalmUp::default();
    palm_up.update(palm, start);
    palm_up.update(palm, start + HOLD)
}

const SIDES: [Side; 2] = [Side::Left, Side::Right];

#[test]
fn either_palm_turned_up_faces_up_from_joints_or_the_grip() {
    for side in SIDES {
        let palm = Palm::from_hand(side, &hand_palm_up(side)).unwrap();
        assert!(
            palm.normal.dot(Vec3::Y) > 0.99,
            "{side:?} {:?}",
            palm.normal
        );
        let palm = up_tilted(side, Quat::IDENTITY).unwrap();
        assert!(
            palm.normal.dot(Vec3::Y) > 0.999,
            "{side:?} {:?}",
            palm.normal
        );
        // A hand missing a joint has no palm.
        let mut missing = hand_palm_up(side);
        missing.joints[WRIST] = None;
        assert_eq!(Palm::from_hand(side, &missing), None);
    }
    let cue = wrist_cue(&up_tilted(Side::Right, Quat::IDENTITY).unwrap(), true);
    assert_eq!(cue.kind, Kind::Menu(MenuKind::ProblemCue));
    assert_eq!(cue.key, Key::Menu(MenuKey::WristCue));
    assert!(!cue.choosable);
}

#[test]
fn in_a_neutral_grip_either_palm_faces_inwards_along_opposite_signs_of_the_grip_x_axis() {
    // OpenXR's grip +X points away from the left palm and into the right one, so both palms face inwards, towards the body's middle, and 90° from up.
    let left = Palm::from_controller(Side::Left, &grip(Side::Left, Quat::IDENTITY)).unwrap();
    assert!(left.normal.dot(Vec3::X) > 0.999, "{:?}", left.normal);
    let right = Palm::from_controller(Side::Right, &grip(Side::Right, Quat::IDENTITY)).unwrap();
    assert!(right.normal.dot(-Vec3::X) > 0.999, "{:?}", right.normal);
    for palm in [left, right] {
        assert!((palm.angle_to_up().to_degrees() - 90.0).abs() < 0.01);
    }
    let degenerate = controller::Pose {
        position: [0.0; 3],
        orientation: [0.0; 4],
    };
    assert_eq!(Palm::from_controller(Side::Left, &degenerate), None);
}

#[test]
fn either_palm_up_shows_after_the_dwell() {
    for side in SIDES {
        // A neutral grip, palm inwards, does not show, however long it is held.
        let neutral = Palm::from_controller(side, &grip(side, Quat::IDENTITY));
        assert!(!shows(neutral), "{side:?}");
        // Straight up shows, from joints or the grip.
        assert!(shows(up_tilted(side, Quat::IDENTITY)), "{side:?}");
        assert!(
            shows(Palm::from_hand(side, &hand_palm_up(side))),
            "{side:?}"
        );
        // So do 40° towards the body, away from it, forwards and back.
        for tilt in [
            Quat::from_rotation_z(40f32.to_radians()),
            Quat::from_rotation_z(-40f32.to_radians()),
            Quat::from_rotation_x(40f32.to_radians()),
            Quat::from_rotation_x(-40f32.to_radians()),
        ] {
            let palm = up_tilted(side, tilt);
            let angle = palm.unwrap().angle_to_up().to_degrees();
            assert!((angle - 40.0).abs() < 0.01, "{side:?} {angle}");
            assert!(shows(palm), "{side:?} {tilt:?}");
        }
        // Palm down does not.
        assert!(!shows(Palm::from_controller(
            side,
            &grip(side, rolled_up(side).inverse())
        )));
    }
}

#[test]
fn either_palm_between_the_angles_neither_shows_nor_hides_and_past_them_hides() {
    let start = Instant::now();
    let tilt = |degrees: f32| Quat::from_rotation_z(degrees.to_radians());
    for side in SIDES {
        // 50° from up does not show from hidden.
        assert!(!shows(up_tilted(side, tilt(50.0))), "{side:?}");
        // But keeps it shown once it shows.
        let mut palm_up = PalmUp::default();
        palm_up.update(up_tilted(side, Quat::IDENTITY), start);
        assert!(palm_up.update(up_tilted(side, Quat::IDENTITY), start + HOLD));
        assert!(palm_up.update(up_tilted(side, tilt(50.0)), start + HOLD * 2));
        assert!(palm_up.update(up_tilted(side, tilt(-50.0)), start + HOLD * 3));
        // 70° hides it.
        assert!(!palm_up.update(up_tilted(side, tilt(70.0)), start + HOLD * 4));
    }
}

#[test]
fn either_palm_respects_the_dwell_so_a_hand_passing_through_does_not_show() {
    let start = Instant::now();
    let up = |side| up_tilted(side, Quat::IDENTITY);
    let away = |side| up_tilted(side, Quat::from_rotation_z(60f32.to_radians()));
    for side in SIDES {
        let mut palm_up = PalmUp::default();
        assert!(!palm_up.update(up(side), start));
        assert!(!palm_up.update(up(side), start + HOLD / 2));
        assert!(palm_up.update(up(side), start + HOLD));
        assert!(!palm_up.update(away(side), start + HOLD * 2));
        // Passing through the pose, turning away before the dwell, restarts it.
        assert!(!palm_up.update(up(side), start + HOLD * 3));
        assert!(!palm_up.update(away(side), start + HOLD * 3 + HOLD / 2));
        assert!(!palm_up.update(up(side), start + HOLD * 4));
        assert!(!palm_up.update(up(side), start + HOLD * 4 + HOLD / 2));
        assert!(palm_up.update(up(side), start + HOLD * 5));
        // Losing the hand hides it.
        assert!(!palm_up.update(None, start + HOLD * 6));
    }
}

fn texts(items: &[Item]) -> Vec<&str> {
    items.iter().map(|item| item.text.as_str()).collect()
}

#[test]
fn a_hand_or_controller_that_gives_no_direction_has_no_palm() {
    let flat = hand(Vec3::ZERO, Vec3::X, Vec3::X * 2.0);
    let lost = controller::Pose {
        position: [0.0; 3],
        orientation: [f32::NAN; 4],
    };
    for side in SIDES {
        assert_eq!(Palm::from_hand(side, &flat), None);
        for missing in [WRIST, INDEX_KNUCKLE, LITTLE_KNUCKLE] {
            let mut partial = hand(Vec3::ZERO, Vec3::X, Vec3::Z);
            partial.joints[missing] = None;
            assert_eq!(Palm::from_hand(side, &partial), None);
        }
        assert_eq!(Palm::from_controller(side, &lost), None);
    }
}

#[test]
fn sections_say_what_they_hold_in_every_state() {
    let space = SpaceInfo {
        title: "A space whose title is long enough to wrap onto more lines than the header shows"
            .into(),
        running: "fake in tmp".into(),
        speech: tomori_protocol::SpeechSetting::Full,
        ..Default::default()
    };
    let model = Model {
        in_space: true,
        role: ConnectionRole::Owner,
        pill: true,
        space: Some(space),
        feedback: tomori_client::feedback::Settings {
            mono: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let shown = items(Some(Section::Space), &model);
    let shown = texts(&shown);
    assert!(shown.contains(&"Full: switch to brief"));
    assert!(shown.contains(&"Status pill: on"));
    let sound = items(Some(Section::Sound), &model);
    assert!(texts(&sound).contains(&"Mono audio: on"));
    // Without a space, the Space section offers only to go back; without instructions, the Agent section says none.
    let away = Model {
        in_space: true,
        ..Default::default()
    };
    assert_eq!(texts(&items(Some(Section::Space), &away))[0], "< Back");
    assert_eq!(texts(&items(Some(Section::Agent), &away))[0], "< Back");
    let short = Model {
        in_space: true,
        instructions: Some(tomori_protocol::AgentInstructions {
            space: "Be brief.".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(texts(&items(Some(Section::Agent), &short)).contains(&"This space's: Be brief."));
    let layer = items(Some(Section::Layer(Layer::Space)), &short);
    assert_eq!(texts(&layer).last(), Some(&"Be brief."));
    for (state, said) in [
        (KnockState::Declined, "Your knock was declined"),
        (KnockState::Expired, "Nobody answered your knock"),
        (KnockState::Withdrawn, "Your knock was withdrawn"),
    ] {
        let knocked = Model {
            in_space: true,
            knock: Some(state),
            ..Default::default()
        };
        assert!(texts(&items(Some(Section::PeopleAndSecurity), &knocked)).contains(&said));
    }
}

#[test]
fn the_top_level_flashes_people_and_security_while_someone_knocks() {
    let mut model = Model::default();
    let top = items(None, &model);
    assert_eq!(texts(&top), ["People & security", "Sound"]);
    assert_eq!(top[0].kind, MenuKind::Section { flashing: false });
    model.knocks.push(KnockRequest {
        id: "knock-1".into(),
        name: "Vic".into(),
    });
    assert_eq!(
        items(None, &model)[0].kind,
        MenuKind::Section { flashing: true }
    );
}

#[test]
fn the_sound_section_steps_the_feedback_setting_in_and_out_of_a_space() {
    use tomori_client::feedback::{Ambience, Level, Settings};
    let mut model = Model {
        feedback: Settings {
            sounds: Level::Reduced,
            vibration: Level::Full,
            mono: false,
            ambience: Ambience::Medium,
        },
        ..Default::default()
    };
    for in_space in [false, true] {
        model.in_space = in_space;
        assert_eq!(
            texts(&items(Some(Section::Sound), &model)),
            [
                "< Back",
                "On this headset",
                "Sounds: reduced",
                "Vibration: full",
                "Reduced: requests, failures, knocks",
                "Mono audio: off",
                "Ambience: medium",
            ]
        );
    }
    let mut menu = HandMenu::default();
    assert_eq!(menu.choose(&MenuKey::Section(Section::Sound)), None);
    assert_eq!(menu.section, Some(Section::Sound));
    assert_eq!(menu.choose(&MenuKey::Sounds), Some(Choice::Sounds));
    assert_eq!(menu.choose(&MenuKey::Vibration), Some(Choice::Vibration));
    assert_eq!(menu.choose(&MenuKey::Mono), Some(Choice::Mono));
    assert_eq!(menu.choose(&MenuKey::Ambience), Some(Choice::Ambience));
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, None);
}

#[test]
fn people_and_security_holds_requests_apart_from_roles() {
    let owner = Model {
        name: "Ada".into(),
        role: ConnectionRole::Owner,
        in_space: true,
        participants: vec!["Ada".into(), "Bob".into()],
        visitors: 2,
        knocks: vec![KnockRequest {
            id: "knock-1".into(),
            name: "Vic".into(),
        }],
        ..Default::default()
    };
    let rows = items(Some(Section::PeopleAndSecurity), &owner);
    assert_eq!(
        texts(&rows),
        [
            "< Back",
            "Requests",
            "Let Vic (visitor) in",
            "Turn Vic (visitor) away",
            "Roles & security",
            "You: Ada (owner)",
            "Here: Ada, Bob",
            "Visitors watching: 2",
        ]
    );
    assert_eq!(rows[2].key, MenuKey::Admit("knock-1".into()));
    assert_eq!(rows[3].key, MenuKey::Decline("knock-1".into()));

    let visitor = Model {
        name: "Vic".into(),
        role: ConnectionRole::Visitor,
        in_space: true,
        can_knock: true,
        pairing: Some("Pair".into()),
        notice: Some("This server's identity has changed.".into()),
        knocks: owner.knocks.clone(),
        ..Default::default()
    };
    let rows = items(Some(Section::PeopleAndSecurity), &visitor);
    assert_eq!(
        texts(&rows),
        [
            "< Back",
            "Requests",
            "Knock to take part",
            "Pair",
            "Roles & security",
            "You: Vic (visitor)",
            "Here: nobody",
            "Visitors watching: 0",
            "This server's identity has changed.",
        ]
    );
    let waiting = Model {
        role: ConnectionRole::Visitor,
        knock: Some(KnockState::Pending),
        ..Default::default()
    };
    assert!(
        texts(&items(Some(Section::PeopleAndSecurity), &waiting))
            .contains(&"Knocked; waiting to be let in")
    );
    let idle = Model {
        role: ConnectionRole::Owner,
        ..Default::default()
    };
    assert!(texts(&items(Some(Section::PeopleAndSecurity), &idle)).contains(&"Nothing waiting"));
}

#[test]
fn a_participant_can_say_a_new_name_where_the_server_renames() {
    let mut model = Model {
        name: "Quest 3".into(),
        role: ConnectionRole::Collaborator,
        in_space: true,
        renames: true,
        ..Default::default()
    };
    let rows = items(Some(Section::PeopleAndSecurity), &model);
    let at = texts(&rows)
        .iter()
        .position(|text| *text == "You: Quest 3 (collaborator)")
        .unwrap();
    assert_eq!(rows[at + 1].key, MenuKey::Rename);
    assert_eq!(rows[at + 1].text, "Change your name");
    assert_eq!(rows[at + 1].kind, MenuKind::Button);
    // Chosen, the next press of X says it.
    model.naming = true;
    assert!(
        texts(&items(Some(Section::PeopleAndSecurity), &model)).contains(&"Hold X, say your name")
    );
    let mut menu = HandMenu::default();
    assert_eq!(menu.choose(&MenuKey::Rename), Some(Choice::Rename));
    // A server that does not rename offers nothing.
    model.renames = false;
    assert!(
        !items(Some(Section::PeopleAndSecurity), &model)
            .iter()
            .any(|item| item.key == MenuKey::Rename)
    );
}

#[test]
fn an_owner_renames_the_space_aloud_or_lets_it_follow_its_session_and_everyone_sees_who_named_it() {
    let mut model = in_space(ConnectionRole::Owner);
    let space = model.space.as_mut().unwrap();
    space.retitles = true;
    let rows = items(Some(Section::Space), &model);
    let at = rows
        .iter()
        .position(|item| item.key == MenuKey::RenameSpace)
        .unwrap();
    // Beneath what the space runs, before its modes.
    assert_eq!(rows[at - 1].text, "opencode in tomori");
    assert_eq!(
        (rows[at].text.as_str(), rows[at].kind),
        ("Rename this space", MenuKind::Button)
    );
    assert!(!rows.iter().any(|item| item.key == MenuKey::FollowSession));
    let mut menu = HandMenu::default();
    assert_eq!(
        menu.choose(&MenuKey::RenameSpace),
        Some(Choice::RenameSpace)
    );
    assert_eq!(
        menu.choose(&MenuKey::FollowSession),
        Some(Choice::FollowSession)
    );
    // Chosen, the next press of X says it.
    model.titling = true;
    assert!(texts(&items(Some(Section::Space), &model)).contains(&"Hold X, say the new title"));

    // Renamed, everyone reads who named it, and an owner can let it follow its session again.
    model.space.as_mut().unwrap().renamed_by = Some("Tomori".into());
    let rows = items(Some(Section::Space), &model);
    let texts = texts(&rows);
    let at = texts
        .iter()
        .position(|text| *text == "Renamed by Tomori")
        .unwrap();
    assert_eq!(rows[at + 2].key, MenuKey::FollowSession);
    assert_eq!(rows[at + 2].text, "Use its agent session's title");
    let mut model = in_space(ConnectionRole::Collaborator);
    model.space.as_mut().unwrap().renamed_by = Some("W".repeat(80));
    let rows = items(Some(Section::Space), &model);
    let shown = rows
        .iter()
        .find(|item| item.text.starts_with("Renamed by "))
        .unwrap();
    assert_eq!(shown.kind, MenuKind::Text);
    assert!(shown.text.ends_with('…') && shown.text.chars().count() <= WIDTH);
    assert!(
        !rows
            .iter()
            .any(|item| matches!(item.key, MenuKey::RenameSpace | MenuKey::FollowSession))
    );
}

#[test]
fn the_agent_section_shows_each_layer_in_a_space_and_opens_it() {
    let mut model = Model {
        role: ConnectionRole::Owner,
        ..Default::default()
    };
    // Only inside a space, where it applies.
    assert_eq!(texts(&items(None, &model)), ["People & security", "Sound"]);
    assert_eq!(
        texts(&items(Some(Section::Agent), &model)),
        ["People & security", "Sound"]
    );
    model.in_space = true;
    assert_eq!(
        texts(&items(None, &model)),
        ["Space", "Agent", "People & security", "Sound"]
    );
    assert_eq!(
        texts(&items(Some(Section::Agent), &model)),
        [
            "< Back",
            "Instructions",
            "This agent takes none from Tomori"
        ]
    );
    let tomori = "You are Tomori, the coordinator of a shared spatial workspace. ".repeat(12);
    model.instructions = Some(AgentInstructions {
        tomori: tomori.clone(),
        environment: String::new(),
        space: "Mind the docs.\nAnd more".into(),
        error: Some("The agent did not take Tomori's instructions: gone".into()),
    });
    let rows = items(Some(Section::Agent), &model);
    assert_eq!(
        texts(&rows),
        [
            "< Back",
            "Instructions",
            "Tomori's: You are Tomori, the…",
            "The environment's: none",
            "This space's: Mind the docs. And more",
            "Clear this space's",
            "The agent did not take Tomori's",
            "instructions: gone",
        ]
    );
    assert_eq!(rows[2].key, MenuKey::Layer(Layer::Tomori));
    assert_eq!(rows[3].kind, MenuKind::Text);
    assert_eq!(rows[5].key, MenuKey::ClearInstructions);
    // Only an owner can clear the space's own.
    let collaborator = Model {
        role: ConnectionRole::Collaborator,
        ..model.clone()
    };
    assert!(!texts(&items(Some(Section::Agent), &collaborator)).contains(&"Clear this space's"));

    // A layer opens in place, wrapped and cut to fit, and steps back to the section.
    let mut menu = HandMenu {
        section: Some(Section::Agent),
        ..Default::default()
    };
    assert_eq!(menu.choose(&MenuKey::Layer(Layer::Tomori)), None);
    let rows = items(menu.section, &model);
    assert_eq!(rows[1].text, "Tomori's");
    assert_eq!(rows.len(), 2 + LAYER_ROWS);
    assert!(rows.last().unwrap().text.ends_with('…'));
    assert!(rows.iter().all(|row| row.text.chars().count() <= WIDTH));
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, Some(Section::Agent));
    assert_eq!(
        menu.choose(&MenuKey::ClearInstructions),
        Some(Choice::ClearInstructions)
    );
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, None);
}

fn in_space(role: ConnectionRole) -> Model {
    Model {
        name: "Ada".into(),
        role,
        in_space: true,
        space: Some(SpaceInfo {
            title: "Fix the parser in the lexer before the release goes out".into(),
            running: "opencode in tomori".into(),
            missing_avatar: None,
            activity: "running".into(),
            modes: vec![
                (
                    "build".into(),
                    "build".into(),
                    Some("Makes changes.".into()),
                    true,
                ),
                (
                    "plan".into(),
                    "plan".into(),
                    Some("Plans without changing files.".into()),
                    false,
                ),
            ],
            mode: Some("build".into()),
            speech: tomori_protocol::SpeechSetting::Brief,
            environment: Some("Sky".into()),
            environments: Vec::new(),
            here: "2 here".into(),
            walls: None,
            deletion: None,
            renamed_by: None,
            retitles: false,
        }),
        people: vec![
            Person {
                name: "Ada".into(),
                colour: [0.7, 0.5, 1.0],
                you: true,
                away: false,
            },
            Person {
                name: "Bob".into(),
                colour: [0.4, 0.9, 0.5],
                you: false,
                away: true,
            },
        ],
        pill: true,
        ..Default::default()
    }
}

#[test]
fn the_space_section_names_the_avatar_this_build_lacks_on_a_row_of_its_own_within_the_width() {
    let mut model = in_space(ConnectionRole::Owner);
    // None, no row.
    assert!(
        !rows(Some(Section::Space), &model)
            .iter()
            .any(|row| row.contains("avatar"))
    );
    let space = model.space.as_mut().unwrap();
    space.running = format!("opencode in {}", "a-very-long-folder-name".repeat(3));
    space.missing_avatar = Some("river".into());
    let shown = rows(Some(Section::Space), &model);
    // Beneath what the space runs, which is cut to the width as ever, it is named in full.
    let running = shown
        .iter()
        .position(|row| row.starts_with("opencode in"))
        .unwrap();
    assert_eq!(crate::ui_text::width(&shown[running]), WIDTH);
    assert_eq!(shown[running + 1], "Unknown avatar: river");
    // An identifier as long as any is cut, its words kept, within the width.
    for long in ["r".repeat(tomori_protocol::AVATAR_BYTES), "川".repeat(42)] {
        model.space.as_mut().unwrap().missing_avatar = Some(long.clone());
        let shown = rows(Some(Section::Space), &model);
        let row = &shown[running + 1];
        assert!(row.starts_with("Unknown avatar: "), "{row}");
        assert!(row.ends_with('…'), "{row}");
        assert!(crate::ui_text::width(row) <= WIDTH, "{row}");
    }
}

#[test]
fn the_header_says_where_you_are_and_what_the_agent_does() {
    let model = in_space(ConnectionRole::Owner);
    let rows = items(None, &model);
    assert_eq!(
        texts(&rows),
        [
            "Fix the parser in the lexer before the",
            "release goes out",
            "running | build | brief | 2 here",
            "Space",
            "Agent",
            "People & security",
            "Sound",
        ]
    );
    assert_eq!(rows[0].kind, MenuKind::Heading);
    assert!(!matches!(
        rows[2].kind,
        MenuKind::Button | MenuKind::Section { .. }
    ));
    // Outside a space there is none.
    assert_eq!(
        texts(&items(None, &Model::default())),
        ["People & security", "Sound"]
    );
}

#[test]
fn the_space_section_switches_modes_speech_and_this_headsets_settings() {
    let mut model = in_space(ConnectionRole::Collaborator);
    let rows = items(Some(Section::Space), &model);
    assert_eq!(
        texts(&rows)[3..],
        [
            "< Back",
            "opencode in tomori",
            "Mode",
            "* build (now)",
            "  plan",
            "  Makes changes.",
            "Speech, for everyone",
            "Brief: switch to full",
            "Environment, for everyone",
            "Sky",
            "On this headset",
            "Spoken replies: on",
            "Status pill: on",
            "Leave space",
        ]
    );
    assert!(
        rows.iter()
            .any(|row| row.key == MenuKey::Mode("plan".into()))
    );
    // Beneath the modes is the description of the current one, or of the one pointed at, in the same row.
    model.pointed = Some(MenuKey::Mode("plan".into()));
    let shown = texts(&items(Some(Section::Space), &model))
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    let plan = shown.iter().position(|text| text == "  plan").unwrap();
    assert_eq!(shown[plan + 1], "  Plans without changing files.");
    assert_eq!(shown.len(), rows.len());
    let mut menu = HandMenu::default();
    assert_eq!(
        menu.choose(&MenuKey::Mode("plan".into())),
        Some(Choice::Mode("plan".into()))
    );
    assert_eq!(menu.choose(&MenuKey::Speech), Some(Choice::Speech));
    assert_eq!(menu.choose(&MenuKey::Mute), Some(Choice::Mute));
    assert_eq!(menu.choose(&MenuKey::Pill), Some(Choice::Pill));
    model.muted = true;
    model.pill = false;
    let shown = texts(&items(Some(Section::Space), &model))
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    assert!(shown.contains(&"Spoken replies: muted".to_owned()));
    assert!(shown.contains(&"Status pill: off".to_owned()));
    // Where the environment allows travel, this headset can turn it off, beneath the pill.
    assert!(!rows.iter().any(|row| row.key == MenuKey::Travel));
    for (travel, said) in [(true, "Travel: on"), (false, "Travel: off, room scale")] {
        model.travel = Some(travel);
        let rows = items(Some(Section::Space), &model);
        let pill = rows
            .iter()
            .position(|row| row.key == MenuKey::Pill)
            .unwrap();
        assert_eq!(rows[pill + 1].key, MenuKey::Travel);
        assert_eq!(rows[pill + 1].text, said);
        assert_eq!(rows[pill + 2].key, MenuKey::Leave);
    }
    assert_eq!(menu.choose(&MenuKey::Travel), Some(Choice::Travel));
    // While the agent is summoned away from its home, a participant can send it home, before this headset's own
    // settings; and not otherwise.
    assert!(!rows.iter().any(|row| row.key == MenuKey::AgentHome));
    model.agent_away = true;
    let rows = items(Some(Section::Space), &model);
    let home = rows
        .iter()
        .position(|row| row.key == MenuKey::AgentHome)
        .expect("offered");
    assert_eq!(rows[home].text, "Send Tomori home");
    assert_eq!(rows[home - 1].text, "Tomori, for everyone");
    assert_eq!(rows[home + 1].text, "On this headset");
    assert_eq!(menu.choose(&MenuKey::AgentHome), Some(Choice::AgentHome));
    model.agent_away = false;
    // On a server that says how the agent moves: anyone reads it, an owner steps it, and how long it waits, quiet,
    // before going home while it follows; and Call Tomori here, where it is offered, before sending it home.
    use tomori_protocol::{AgentMovement, Movement};
    let tomori = |model: &Model| -> Vec<(MenuKey, MenuKind, String)> {
        let rows = items(Some(Section::Space), model);
        let heading = rows
            .iter()
            .position(|row| row.text == "Tomori, for everyone")
            .expect("headed");
        rows[heading + 1..]
            .iter()
            .take_while(|row| row.text != "On this headset")
            .map(|row| (row.key.clone(), row.kind, row.text.clone()))
            .collect()
    };
    model.movement = Some((AgentMovement::default(), false));
    assert_eq!(
        tomori(&model)
            .iter()
            .map(|(_, kind, text)| (*kind, text.as_str()))
            .collect::<Vec<_>>(),
        [(MenuKind::Text, "Moves: only when asked")]
    );
    model.movement = Some((AgentMovement::new(Movement::Follow, Some(90)), true));
    model.calls = true;
    model.agent_away = true;
    model.pins = Some(true);
    let named = tomori(&model);
    assert_eq!(
        named
            .iter()
            .map(|(key, _, text)| (key.clone(), text.as_str()))
            .collect::<Vec<_>>(),
        [
            (MenuKey::AgentMovement, "Moves: follows the speaker"),
            (MenuKey::AgentReturn, "Goes home after 90 s quiet"),
            (MenuKey::CallAgent, "Call Tomori here"),
            (MenuKey::AgentStay, "Tomori: stay here"),
            (MenuKey::AgentHome, "Send Tomori home"),
        ]
    );
    // Pinned, it offers to follow again in its place.
    model.pins = Some(false);
    let named = tomori(&model);
    assert_eq!(
        (named[3].0.clone(), named[3].2.as_str()),
        (MenuKey::AgentFollow, "Tomori: follow again")
    );
    assert_eq!(menu.choose(&MenuKey::AgentStay), Some(Choice::AgentStay));
    assert_eq!(
        menu.choose(&MenuKey::AgentFollow),
        Some(Choice::AgentFollow)
    );
    // Beside them, it offers to walk with them, and, while it does, to stop.
    model.walks = Some(true);
    let named = tomori(&model);
    assert_eq!(
        (named[4].0.clone(), named[4].2.as_str()),
        (MenuKey::AgentWalk, "Tomori: walk with me")
    );
    assert_eq!(named[5].0, MenuKey::AgentHome);
    model.walks = Some(false);
    assert_eq!(tomori(&model)[4].2.as_str(), "Tomori: stop walking with me");
    assert_eq!(menu.choose(&MenuKey::AgentWalk), Some(Choice::AgentWalk));
    assert_eq!(
        menu.choose(&MenuKey::AgentWalkEnd),
        Some(Choice::AgentWalkEnd)
    );
    model.pins = None;
    model.walks = None;
    for (delay, said) in [
        (Some(30), "Goes home after 30 s quiet"),
        (Some(300), "Goes home after 5 min quiet"),
        (None, "Goes home: never"),
    ] {
        model.movement = Some((AgentMovement::new(Movement::Follow, delay), false));
        let named = tomori(&model);
        assert_eq!((named[1].1, named[1].2.as_str()), (MenuKind::Text, said));
    }
    model.movement = Some((AgentMovement::new(Movement::Home, None), true));
    assert_eq!(tomori(&model)[0].2, "Moves: stays home");
    assert_eq!(menu.choose(&MenuKey::CallAgent), Some(Choice::CallAgent));
    assert_eq!(
        menu.choose(&MenuKey::AgentMovement),
        Some(Choice::AgentMovement)
    );
    assert_eq!(
        menu.choose(&MenuKey::AgentReturn),
        Some(Choice::AgentReturn)
    );
    model.movement = None;
    model.calls = false;
    model.agent_away = false;
    // An owner adds walls, and removes the nearest added one where there is one, before this headset's settings.
    assert!(!rows.iter().any(|row| row.key == MenuKey::AddWall));
    for (removable, keys) in [
        (false, vec![MenuKey::AddWall]),
        (true, vec![MenuKey::AddWall, MenuKey::RemoveWall]),
    ] {
        model.space.as_mut().unwrap().walls = Some(removable);
        let rows = items(Some(Section::Space), &model);
        let heading = rows
            .iter()
            .position(|row| row.text == "Projection walls, for everyone")
            .unwrap();
        let shown: Vec<MenuKey> = rows[heading + 1..=heading + keys.len()]
            .iter()
            .map(|row| row.key.clone())
            .collect();
        assert_eq!(shown, keys);
        assert_eq!(rows[heading + keys.len() + 1].text, "On this headset");
    }
    assert_eq!(menu.choose(&MenuKey::AddWall), Some(Choice::AddWall));
    assert_eq!(menu.choose(&MenuKey::RemoveWall), Some(Choice::RemoveWall));
    // A visitor reads the mode and speech, and cannot switch them.
    let visitor = in_space(ConnectionRole::Visitor);
    let rows = items(Some(Section::Space), &visitor);
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row.key, MenuKey::Mode(_) | MenuKey::Speech))
    );
    assert!(texts(&rows).contains(&"* build (now)"));
    assert!(texts(&rows).contains(&"Brief"));
}

#[test]
fn only_an_owner_is_offered_the_space_s_environments() {
    let mut owner = in_space(ConnectionRole::Owner);
    // A server that changes environments offers its environments to an owner, the current one marked.
    let space = owner.space.as_mut().unwrap();
    space.environments = vec![
        ("sky".into(), "Sky".into(), true),
        ("minimal".into(), "Grid".into(), false),
    ];
    let rows = items(Some(Section::Space), &owner);
    let shown = texts(&rows);
    let heading = shown
        .iter()
        .position(|text| *text == "Environment, for everyone")
        .unwrap();
    assert_eq!(
        shown[heading..heading + 4],
        [
            "Environment, for everyone",
            "* Sky (now)",
            "  Grid",
            "On this headset"
        ]
    );
    // Only the others can be chosen.
    assert_eq!(rows[heading + 1].kind, MenuKind::Text);
    assert_eq!(
        rows[heading + 2].key,
        MenuKey::Environment("minimal".into())
    );
    let mut menu = HandMenu::default();
    assert_eq!(
        menu.choose(&MenuKey::Environment("minimal".into())),
        Some(Choice::Environment("minimal".into()))
    );
    // Anyone else reads the environment and chooses none.
    let collaborator = in_space(ConnectionRole::Collaborator);
    let rows = items(Some(Section::Space), &collaborator);
    assert!(texts(&rows).contains(&"Sky"));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row.key, MenuKey::Environment(_)))
    );
    // An older server names no environment, so none is shown.
    let mut older = in_space(ConnectionRole::Owner);
    older.space.as_mut().unwrap().environment = None;
    assert!(!texts(&items(Some(Section::Space), &older)).contains(&"Environment, for everyone"));
}

#[test]
fn leaving_from_the_menu_asks_first() {
    let model = in_space(ConnectionRole::Owner);
    let mut menu = HandMenu::default();
    menu.choose(&MenuKey::Section(Section::Space));
    assert_eq!(menu.choose(&MenuKey::Leave), None);
    let rows = items(menu.section, &model);
    assert_eq!(
        texts(&rows),
        [
            "Leave this space?",
            "Fix the parser in the lexer before the",
            "release goes out",
            "Leave",
            "Stay",
        ]
    );
    // Stay steps back to the Space section without leaving.
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, Some(Section::Space));
    menu.choose(&MenuKey::Leave);
    assert_eq!(menu.choose(&MenuKey::ConfirmLeave), Some(Choice::Leave));
    assert_eq!(menu.section, None);
}

/// An owner in a space on a server that deletes spaces, and its sessions when `sessions`.
fn deletable(sessions: bool) -> Model {
    let mut model = in_space(ConnectionRole::Owner);
    model.space.as_mut().unwrap().deletion = Some(sessions);
    model
}

#[test]
fn deleting_from_the_menu_asks_first_and_again_for_the_session() {
    let model = deletable(true);
    let mut menu = HandMenu::default();
    menu.toggle_pin();
    let space = items(menu.section, &model);
    // Last, apart from the rest of the space's rows.
    assert_eq!(space.last().unwrap().key, MenuKey::Delete);
    assert_eq!(space.last().unwrap().text, "Delete this space");
    assert_eq!(menu.choose(&MenuKey::Delete), None);
    assert_eq!(menu.section, Some(Section::Delete { session: false }));
    let kept = items(menu.section, &model);
    assert_eq!(
        texts(&kept),
        [
            "Delete this space?",
            "Fix the parser in the lexer before the",
            "release goes out",
            "Delete it, for everyone",
            "Everyone here returns to the lobby.",
            "Its agent session is kept.",
            "Also delete its agent session: no",
            "Keep this space",
        ]
    );
    // The last row, where the ray was as the rows changed, keeps the space, and the one that deletes is well above it.
    assert_eq!(kept.last().unwrap().key, MenuKey::Back);
    assert_eq!(kept[3].key, MenuKey::ConfirmDelete);
    // Turning the session's deletion on says so, and moves no row.
    assert_eq!(menu.choose(&MenuKey::DeleteSession), None);
    assert_eq!(menu.section, Some(Section::Delete { session: true }));
    let both = items(menu.section, &model);
    assert_eq!(both.len(), kept.len());
    assert_eq!(both[5].text, "Its agent session is deleted too.");
    assert_eq!(both[6].text, "Also delete its agent session: yes");
    menu.choose(&MenuKey::DeleteSession);
    assert_eq!(menu.section, Some(Section::Delete { session: false }));
    // Keeping it steps back to the Space section, and asking again starts with the session kept.
    menu.choose(&MenuKey::DeleteSession);
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, Some(Section::Space));
    menu.choose(&MenuKey::Delete);
    assert_eq!(menu.section, Some(Section::Delete { session: false }));

    // With the session, deleting asks again, and stepping back from that keeps the session.
    menu.choose(&MenuKey::DeleteSession);
    assert_eq!(menu.choose(&MenuKey::ConfirmDelete), None);
    assert_eq!(menu.section, Some(Section::DeleteSession));
    let again = items(menu.section, &model);
    assert_eq!(
        texts(&again),
        [
            "Delete its agent session too?",
            "Its conversation, and its workers'",
            "sessions, are lost for good. This cannot",
            "be undone.",
            "Delete the space and its session",
            "",
            "Back, keeping the session",
        ]
    );
    assert_eq!(again.last().unwrap().key, MenuKey::Back);
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, Some(Section::Delete { session: false }));
    menu.choose(&MenuKey::DeleteSession);
    menu.choose(&MenuKey::ConfirmDelete);
    assert_eq!(
        menu.choose(&MenuKey::ConfirmDeleteSession),
        Some(Choice::Delete { session: true })
    );
    // The menu closes, and lets go, as on leaving.
    assert_eq!(menu.section, None);
    assert!(!menu.pinned());

    // Without the session, deleting needs no second asking.
    menu.choose(&MenuKey::Section(Section::Space));
    menu.choose(&MenuKey::Delete);
    assert_eq!(
        menu.choose(&MenuKey::ConfirmDelete),
        Some(Choice::Delete { session: false })
    );
    assert_eq!(menu.section, None);

    // A choice left from rows no longer shown deletes nothing.
    for section in [
        None,
        Some(Section::Space),
        Some(Section::Delete { session: false }),
    ] {
        menu.section = section;
        assert_eq!(menu.choose(&MenuKey::ConfirmDeleteSession), None);
        assert_eq!(menu.section, section);
    }
    menu.section = Some(Section::Space);
    assert_eq!(menu.choose(&MenuKey::ConfirmDelete), None);
    assert_eq!(menu.choose(&MenuKey::DeleteSession), None);
    assert_eq!(menu.section, Some(Section::Space));
}

#[test]
fn only_owners_are_asked_whether_to_delete_a_space() {
    let sections = [
        Some(Section::Delete { session: false }),
        Some(Section::Delete { session: true }),
        Some(Section::DeleteSession),
    ];
    // Anyone else, or an owner on an older server, is offered nothing, and the rows that ask show the top level instead.
    for model in [
        in_space(ConnectionRole::Owner),
        {
            let mut model = deletable(true);
            model.role = ConnectionRole::Collaborator;
            model
        },
        {
            let mut model = deletable(true);
            model.role = ConnectionRole::Visitor;
            model
        },
        {
            let mut model = deletable(true);
            model.role = ConnectionRole::Unknown;
            model
        },
    ] {
        assert!(
            !items(Some(Section::Space), &model)
                .iter()
                .any(|row| row.key == MenuKey::Delete),
            "{:?}",
            model.role
        );
        for section in sections {
            assert_eq!(items(section, &model), items(None, &model));
        }
    }
    // Where the session cannot be deleted, the space can, and the session is kept without asking.
    let model = deletable(false);
    let rows = items(Some(Section::Delete { session: false }), &model);
    assert!(!rows.iter().any(|row| row.key == MenuKey::DeleteSession));
    assert!(texts(&rows).contains(&"Its agent session is kept."));
    assert_eq!(rows.last().unwrap().key, MenuKey::Back);
    for section in &sections[1..] {
        assert_eq!(items(*section, &model), items(None, &model));
    }
    // Outside a space, there is nothing to delete.
    let mut lobby = deletable(true);
    lobby.in_space = false;
    assert_eq!(
        items(Some(Section::Delete { session: false }), &lobby),
        items(None, &lobby)
    );
}

#[test]
fn y_pins_the_menu_open_at_the_space_section_until_y_or_b() {
    let start = Instant::now();
    let mut menu = HandMenu::default();
    assert!(!menu.update(None, start));
    menu.toggle_pin();
    assert!(menu.pinned());
    assert_eq!(menu.section, Some(Section::Space));
    // Shown whatever the hand's pose, or with no hand at all.
    assert!(menu.update(None, start));
    menu.toggle_pin();
    assert!(!menu.update(None, start));
    menu.toggle_pin();
    assert!(menu.unpin());
    assert!(!menu.unpin());
    assert!(!menu.update(None, start));
}

#[test]
fn people_have_swatches_with_you_and_away_marked() {
    let model = in_space(ConnectionRole::Owner);
    let rows = items(Some(Section::PeopleAndSecurity), &model);
    let people: Vec<_> = rows
        .iter()
        .filter(|row| matches!(row.kind, MenuKind::Person(_)))
        .collect();
    assert_eq!(people.len(), 2);
    assert_eq!(people[0].text, "You (Ada)");
    assert_eq!(people[0].kind, MenuKind::Person([0.7, 0.5, 1.0]));
    assert_eq!(people[1].text, "Bob, away");
    // The console's text marks what can be chosen.
    let shown = text(Some(Section::Space), &model);
    assert!(shown.contains(&"[Leave space]".to_owned()));
    assert!(shown.contains(&"MODE".to_owned()));
    assert!(text(Some(Section::PeopleAndSecurity), &model).contains(&"  Bob, away".to_owned()));
}

#[test]
fn an_owner_opens_pairing_for_a_new_device_and_sees_it_count_down() {
    let mut owner = Model {
        name: "Ada".into(),
        role: ConnectionRole::Owner,
        pairing_window: Some(None),
        ..Default::default()
    };
    let rows = |model: &Model| items(Some(Section::PeopleAndSecurity), model);
    let closed = rows(&owner);
    assert_eq!(
        texts(&closed)[..3],
        ["< Back", "Requests", "Pair a new device"]
    );
    assert_eq!(closed[2].key, MenuKey::PairingWindow);
    assert_eq!(closed[2].kind, MenuKind::Button);
    owner.pairing_window = Some(Some(102));
    let open = rows(&owner);
    assert_eq!(texts(&open)[2], "Pairing open, 1:42 left: close it");
    // Counting down changes what the row says, never how many rows there are.
    assert_eq!(open.len(), closed.len());
    owner.pairing_window = Some(Some(0));
    assert_eq!(texts(&rows(&owner))[2], "Pairing open: close it");
    assert!(
        text(Some(Section::PeopleAndSecurity), &owner)
            .contains(&"[Pairing open: close it]".to_owned())
    );
    // Offered to nobody whose server does not say, nor to anyone but an owner, whose model has none.
    owner.pairing_window = None;
    assert!(
        rows(&owner)
            .iter()
            .all(|row| row.key != MenuKey::PairingWindow)
    );
    let mut menu = HandMenu::default();
    assert_eq!(
        menu.choose(&MenuKey::PairingWindow),
        Some(Choice::PairingWindow)
    );
}

#[test]
fn choosing_opens_sections_and_asks_for_actions() {
    let mut menu = HandMenu::default();
    assert_eq!(
        menu.choose(&MenuKey::Section(Section::PeopleAndSecurity)),
        None
    );
    assert_eq!(menu.section, Some(Section::PeopleAndSecurity));
    assert_eq!(menu.choose(&MenuKey::Knock), Some(Choice::Knock));
    assert_eq!(
        menu.choose(&MenuKey::Admit("k".into())),
        Some(Choice::Admit("k".into()))
    );
    assert_eq!(
        menu.choose(&MenuKey::Decline("k".into())),
        Some(Choice::Decline("k".into()))
    );
    assert_eq!(menu.choose(&MenuKey::Pair), Some(Choice::Pair));
    assert_eq!(menu.choose(&MenuKey::Text(3)), None);
    assert_eq!(menu.choose(&MenuKey::Back), None);
    assert_eq!(menu.section, None);
}

#[test]
fn the_rows_stand_above_the_wrist_and_only_buttons_are_chosen() {
    let palm = Palm {
        wrist: Vec3::new(-0.2, 1.1, -0.35),
        normal: Vec3::Y,
    };
    let menu = HandMenu {
        section: Some(Section::PeopleAndSecurity),
        ..Default::default()
    };
    let model = Model {
        role: ConnectionRole::Owner,
        ..Default::default()
    };
    let objects = menu.objects(&palm, &model, Vec3::new(0.0, 1.6, 0.0));
    assert!(
        objects
            .iter()
            .all(|object| object.position.y > palm.wrist.y)
    );
    // Rows go down the menu in order.
    assert!(
        objects
            .windows(2)
            .all(|pair| pair[0].position.y > pair[1].position.y)
    );
    let chosen: Vec<bool> = objects.iter().map(|object| object.choosable).collect();
    assert!(chosen[0], "Back");
    assert!(!chosen[1], "Requests heading");
    let cue = cue(&palm);
    assert!(!cue.choosable);
    assert_eq!(cue.kind, Kind::Menu(MenuKind::Cue));
}

#[test]
fn the_menu_lies_on_a_plane_facing_the_head() {
    let palm = Palm {
        wrist: Vec3::new(-0.15, 1.0, -0.3),
        normal: Vec3::new(0.2, 0.8, 0.4).normalize(),
    };
    let head = Vec3::new(0.1, 1.65, 0.05);
    let model = Model {
        role: ConnectionRole::Owner,
        ..Default::default()
    };
    let menu = HandMenu {
        section: Some(Section::PeopleAndSecurity),
        ..Default::default()
    };
    let objects = menu.objects(&palm, &model, head);
    let middle = objects[objects.len() / 2].position;
    let towards = (head - middle).normalize();
    // Rows run down the plane, across the line of sight.
    let down = (objects[1].position - objects[0].position).normalize();
    assert!(down.dot(towards).abs() < 0.06, "{down:?} {towards:?}");
    assert!(objects.iter().all(|object| object.label.tilt));
    // Every row is a rectangle of one panel, whose plane holds its marker and its label's anchor.
    let plane = objects[0].panel.unwrap().plane;
    for (index, object) in objects.iter().enumerate() {
        let area = object.panel.unwrap();
        assert_eq!(area.plane, plane);
        assert_eq!(area.rect, row_rect(index));
        for point in [object.position, object.label.anchor] {
            assert!((point - plane.origin).dot(plane.normal()).abs() < 1e-5);
            assert!(area.rect.contains(area.local(point)), "{index}");
        }
    }
}

/// The rows tile the panel: each a row tall, from left of its marker to [`PANEL`], sharing its edges with its neighbours.
#[test]
fn the_rows_rectangles_tile_the_panel() {
    for index in 0..20 {
        let rect = row_rect(index);
        assert!((rect.max.y - rect.min.y - ROW).abs() < 1e-6);
        assert_eq!(rect.max.x, PANEL);
        assert!(rect.contains(Vec2::new(0.0, -ROW * index as f32)));
        assert_eq!(rect.min.y, row_rect(index + 1).max.y);
    }
}

/// An owner's Space section, whose buttons and rows that are only read alternate, with a mode whose name fills its row, and no mode described.
fn alternating() -> (HandMenu, Model) {
    let mut model = in_space(ConnectionRole::Owner);
    let space = model.space.as_mut().unwrap();
    for mode in &mut space.modes {
        mode.2 = None;
    }
    space
        .modes
        .push(("long".into(), "x".repeat(2 * WIDTH), None, false));
    let menu = HandMenu {
        section: Some(Section::Space),
        ..Default::default()
    };
    (menu, model)
}

/// Pointing at any row of any section, a mode most of all, moves no row: what is pointed at changes what the rows say, never where they are, so that the ray stays on the row it points at. The menu grew by the description of a mode pointed at, which raised every row by as many, and the next frame the ray met another.
#[test]
fn pointing_at_a_row_moves_no_row() {
    let (_, mut model) = alternating();
    let space = model.space.as_mut().unwrap();
    space.modes[0].2 = Some("Makes changes.".into());
    space.modes[1].2 = Some("Plans without changing files. ".repeat(4));
    space.environments = vec![
        ("sky".into(), "Sky".into(), true),
        ("minimal".into(), "Grid".into(), false),
    ];
    let palm = Palm {
        wrist: Vec3::new(-0.15, 1.05, -0.35),
        normal: Vec3::Y,
    };
    let head = Vec3::new(0.0, 1.6, 0.0);
    let layout = |menu: &HandMenu, model: &Model| {
        menu.objects(&palm, model, head)
            .into_iter()
            .map(|object| (object.key, object.position, object.panel))
            .collect::<Vec<_>>()
    };
    for section in [None, Some(Section::Space), Some(Section::Sound)] {
        let menu = HandMenu {
            section,
            ..Default::default()
        };
        model.pointed = None;
        let still = layout(&menu, &model);
        for item in items(section, &model) {
            let key = item.key;
            model.pointed = Some(key.clone());
            assert_eq!(layout(&menu, &model), still, "{section:?}: {key:?}");
        }
    }
    // The description shown is the one pointed at, and otherwise the current mode's.
    model.pointed = Some(MenuKey::Mode("plan".into()));
    assert!(
        rows(Some(Section::Space), &model)
            .contains(&"  Plans without changing files. Plans".to_owned())
    );
    model.pointed = Some(MenuKey::Mode("long".into()));
    let long = rows(Some(Section::Space), &model);
    assert!(
        !long
            .iter()
            .any(|row| row.contains("Makes") || row.contains("Plans")),
        "{long:?}"
    );
    model.pointed = Some(MenuKey::Speech);
    assert!(rows(Some(Section::Space), &model).contains(&"  Makes changes.".to_owned()));
    // With no modes there is no description.
    model.space.as_mut().unwrap().modes.clear();
    assert!(
        !rows(Some(Section::Space), &model)
            .iter()
            .any(|row| row.is_empty())
    );
}

/// The menu of [`alternating`] held palm up, at chest height, with the head where its panel tilts `tilt` degrees back to face it.
fn tilted_back(tilt: f32) -> Vec<Object> {
    let (menu, model) = alternating();
    let palm = Palm {
        wrist: Vec3::new(-0.15, 1.05, -0.35),
        normal: Vec3::Y,
    };
    // The panel faces the head from its middle, which is half its height up the tilted panel from above the wrist.
    let half = (0.04 + ROW * items(menu.section, &model).len() as f32) / 2.0;
    let (sin, cos) = tilt.to_radians().sin_cos();
    let (normal, up) = (Vec3::new(0.0, sin, cos), Vec3::new(0.0, cos, -sin));
    let middle = palm.wrist + palm.normal * 0.05 + up * half;
    let objects = menu.objects(&palm, &model, middle + normal * 0.45);
    let held = objects[0].panel.unwrap().plane.normal();
    let measured = held.y.asin().to_degrees();
    assert!((measured - tilt).abs() < 2.0, "{tilt}: {measured}");
    objects
}

/// Rays from the head, and from a right hand straight ahead of the panel, below it, above it, beside it, and steeply up from beneath it, at the point `at` of `plane`.
fn rays_at(plane: &Plane, head: Vec3, at: Vec2) -> Vec<crate::pointing::Ray> {
    let point = plane.at(at);
    let (right, up, normal) = (plane.right, plane.up, plane.normal());
    [
        head,
        point + normal * 0.5,
        point + normal * 0.35 + right * 0.25 - up * 0.3,
        point + normal * 0.3 + up * 0.25,
        point + normal * 0.2 - right * 0.3,
        point + normal * 0.12 - up * 0.45,
    ]
    .into_iter()
    .map(|origin| crate::pointing::Ray::new(origin, point - origin).unwrap())
    .collect()
}

/// A ray aimed anywhere at a row meets that row, and none of its neighbours, whichever way the menu tilts and from wherever it is aimed: across the whole of a long row to the panel's right edge, and on either side of the edges between rows. A row that is only read is met by nothing.
#[test]
fn a_ray_aimed_at_a_row_meets_that_row_at_any_tilt() {
    for tilt in [0.0, 30.0, 60.0] {
        let objects = tilted_back(tilt);
        let scene = crate::arrival::Scene {
            objects: objects.clone(),
            ..Default::default()
        };
        let targets = scene.targets();
        let plane = objects[0].panel.unwrap().plane;
        let (sin, cos) = f32::to_radians(tilt).sin_cos();
        let head = plane.at(Vec2::new(0.0, -0.2)) + Vec3::new(0.0, sin, cos) * 0.45;
        // The row of the long mode reaches the panel's right edge.
        let long = objects
            .iter()
            .position(|object| object.key == Key::Menu(MenuKey::Mode("long".into())))
            .unwrap();
        assert_eq!(objects[long].label.lines[0].chars().count(), WIDTH);
        let mut met = 0;
        for (index, object) in objects.iter().enumerate() {
            let rect = row_rect(index);
            let (top, bottom) = (rect.max.y, rect.min.y);
            let middle = (top + bottom) / 2.0;
            let inside = [
                Vec2::new(0.0, middle),
                rect.centre(),
                Vec2::new(rect.min.x + 0.001, top - 0.0002),
                Vec2::new(PANEL - 0.001, bottom + 0.0002),
                Vec2::new(PANEL - 0.001, middle),
            ];
            let expected = object.choosable.then(|| object.key.clone());
            for at in inside {
                for ray in rays_at(&plane, head, at) {
                    let hit = crate::pointing::nearest(&ray, &targets).map(|hit| hit.key);
                    assert_eq!(hit, expected, "{tilt}° row {index} at {at} from {ray:?}");
                    met += usize::from(hit.is_some());
                }
            }
            // Past either end of the row, nothing is met.
            for at in [
                Vec2::new(PANEL + 0.002, middle),
                Vec2::new(rect.min.x - 0.002, middle),
            ] {
                for ray in rays_at(&plane, head, at) {
                    assert_eq!(crate::pointing::nearest(&ray, &targets), None, "{at}");
                }
            }
        }
        assert!(met > 100, "{met}");
    }
}

#[test]
fn the_middle_of_the_head_stays_put_as_the_head_turns() {
    let neck = Vec3::new(0.1, 1.5, 0.05);
    let level = head_turned(neck, 0.0, 0.0);
    let eyes = level.transform_point3(Vec3::ZERO);
    let centre = head_centre(level);
    // Level with the eyes, and behind them.
    assert!((centre.y - eyes.y).abs() < 1e-5);
    assert!((centre - eyes).dot(Vec3::Z) > 0.07);
    for (yaw, pitch) in [(-45.0, 0.0), (45.0, 0.0), (30.0, -40.0), (-60.0, 20.0)] {
        let head = head_turned(neck, yaw, pitch);
        // The eyes swing by centimetres...
        assert!(head.transform_point3(Vec3::ZERO).distance(eyes) > 0.02);
        // ...but the middle of the head does not.
        assert!(head_centre(head).distance(centre) < 1e-5, "{yaw} {pitch}");
    }
}

#[test]
fn the_menu_stays_on_the_wrist_as_the_head_turns() {
    let palm = Palm {
        wrist: Vec3::new(-0.15, 1.2, -0.3),
        normal: Vec3::new(0.2, 0.6, 0.6).normalize(),
    };
    let model = Model {
        role: ConnectionRole::Owner,
        ..Default::default()
    };
    let menu = HandMenu {
        section: Some(Section::PeopleAndSecurity),
        ..Default::default()
    };
    let neck = Vec3::new(0.0, 1.5, 0.05);
    let place = |yaw: f32| {
        menu.objects(&palm, &model, head_centre(head_turned(neck, yaw, -30.0)))
            .into_iter()
            .map(|object| object.position)
            .collect::<Vec<_>>()
    };
    let ahead = place(0.0);
    for yaw in [-45.0, 45.0] {
        for (turned, ahead) in place(yaw).iter().zip(&ahead) {
            assert!(turned.distance(*ahead) < 1e-5, "{yaw}: {turned} {ahead}");
        }
    }
}

#[test]
fn showing_follows_the_palm() {
    let start = Instant::now();
    let mut menu = HandMenu::default();
    let palm = Palm {
        wrist: Vec3::new(0.0, 1.0, -0.3),
        normal: Vec3::new(0.0, 0.6, 0.3).normalize(),
    };
    assert!(!menu.update(Some(palm), start));
    assert!(menu.update(Some(palm), start + HOLD));
    assert!(!menu.update(None, start + HOLD * 2));
    for role in [
        ConnectionRole::Collaborator,
        ConnectionRole::Pairing,
        ConnectionRole::Unknown,
    ] {
        assert!(!role_label(role).is_empty());
    }
}
#[test]
fn every_row_fits_the_menu_and_draws_what_the_transcript_draws() {
    let long = "Bartholomew-Fitzgerald-Montgomery-Smythe";
    let mut model = in_space(ConnectionRole::Owner);
    model.name = format!("José {long}");
    model.participants = vec![long.into(); 4];
    model.people.clear();
    model.pairing = Some(format!("Pairing code {}", "9".repeat(60)));
    model.knocks = vec![KnockRequest {
        id: "knock-1".into(),
        name: long.repeat(2),
    }];
    model.notice = Some("A security notice long enough to be cut. ".repeat(4));
    let space = model.space.as_mut().unwrap();
    space.deletion = Some(true);
    space.title = format!("{} {}", "word".repeat(15), "Łódź ".repeat(20));
    space.activity = "running a very long tool call".into();
    space.here = "12 here, 3 away".into();
    space.modes[1].1 = long.repeat(3);
    space.modes[1].2 = Some("A description ".repeat(30));
    space.environments = vec![
        ("sky".into(), "Sky".into(), true),
        ("long".into(), long.repeat(2), false),
    ];
    model.pointed = Some(MenuKey::Mode("plan".into()));
    model.instructions = Some(AgentInstructions {
        tomori: "x".repeat(400),
        environment: "Short.".into(),
        space: "word ".repeat(400),
        error: Some("why ".repeat(200)),
    });
    let sections = [
        None,
        Some(Section::Space),
        Some(Section::Leave),
        Some(Section::Delete { session: false }),
        Some(Section::Delete { session: true }),
        Some(Section::DeleteSession),
        Some(Section::Agent),
        Some(Section::Layer(Layer::Tomori)),
        Some(Section::Layer(Layer::Space)),
        Some(Section::PeopleAndSecurity),
        Some(Section::Sound),
    ];
    // Japanese takes two cells a character, and every row still fits, counted in cells.
    let mut japanese = model.clone();
    japanese.name = "山田太郎".repeat(5);
    japanese.participants = vec!["佐藤花子".repeat(6); 2];
    let space = japanese.space.as_mut().unwrap();
    space.title = "日本語の会話のスペース。".repeat(8);
    space.modes[1].2 = Some("説明文です。".repeat(30));
    // And so does Latin mixed with Japanese, and emoji, which no font has, drawn `?` in one cell.
    let mut mixed = model.clone();
    mixed.name = "Bob 佐藤 🎤 José".repeat(4);
    mixed.participants = vec!["Ada 🎧 山田".repeat(5); 3];
    let space = mixed.space.as_mut().unwrap();
    space.title = "Tomoriは日本語 🎉 mixed text".repeat(6);
    space.modes[1].2 = Some("Fix the パーサー bug 🐛 ".repeat(20));
    for model in [&model, &japanese, &mixed] {
        for section in sections {
            let rows = items(section, model);
            for row in &rows {
                assert!(
                    crate::ui_text::width(&row.text) <= WIDTH,
                    "{section:?}: {}",
                    row.text
                );
                let cells: Vec<_> = drawn(&row.text).map(|ch| (ch, ())).collect();
                assert!(crate::ui_text::whole(&cells), "{section:?}: {}", row.text);
            }
        }
    }
    assert!(
        texts(&items(Some(Section::Space), &japanese))
            .iter()
            .any(|row| row.contains("日本語の会話"))
    );
    assert!(
        texts(&items(Some(Section::Space), &mixed))
            .iter()
            .any(|row| row.contains("Tomoriは日本語 ? mixed"))
    );
    // The top level marks a title cut at two rows; the Space section goes on with the rest of it.
    let top = items(None, &model);
    assert_eq!(top[1].text.chars().last(), Some('…'));
    assert!(!top[0].text.contains('?'));
    let space = texts(&items(Some(Section::Space), &model))
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    assert!(!space[1].ends_with('…'));
    assert!(space.iter().any(|row| row.contains("Łódź")), "{space:?}");
    // A word longer than a row is broken across rows rather than dropped.
    let layer = texts(&items(Some(Section::Layer(Layer::Tomori)), &model))
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    assert_eq!(layer[2], "x".repeat(WIDTH));
    assert_eq!(layer.len(), 2 + 10);
    // The description of the mode pointed at, the error and the notice are bounded in rows too, and marked where cut.
    let people = texts(&items(Some(Section::PeopleAndSecurity), &model))
        .iter()
        .map(|text| text.to_string())
        .collect::<Vec<_>>();
    assert!(people.last().unwrap().ends_with('…'));
    let pointed = items(Some(Section::Space), &model);
    assert!(pointed.iter().any(|row| row.text.ends_with('…')));
    // Its rows are kept whatever is pointed at, empty beneath the current mode's shorter description.
    model.pointed = None;
    let current = items(Some(Section::Space), &model);
    assert_eq!(current.len(), pointed.len());
    let after = |rows: &[Item]| {
        let last = rows
            .iter()
            .rposition(|row| matches!(row.key, MenuKey::Mode(_)))
            .unwrap();
        rows[last + 1..last + 1 + DESCRIPTION_ROWS]
            .iter()
            .map(|row| row.text.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        after(&pointed)
            .iter()
            .all(|row| row.starts_with("  ") && row.contains("description"))
    );
    assert_eq!(after(&current), ["  Makes changes.", "", "", ""]);
}

/// The rows of `section` for `model`, as text.
fn rows(section: Option<Section>, model: &Model) -> Vec<String> {
    texts(&items(section, model))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// Whether `rows` has one that starts with `start`, cuts what follows with "…" and ends with `end`, filling the menu's width.
fn cut_between(rows: &[String], start: &str, end: &str) -> bool {
    rows.iter().any(|row| {
        row.starts_with(start) && row.ends_with(&format!("…{end}")) && row.chars().count() == WIDTH
    })
}

/// The text of the row of `section` chosen by `key`.
fn text_of(section: Option<Section>, model: &Model, key: &MenuKey) -> String {
    items(section, model)
        .into_iter()
        .find(|item| &item.key == key)
        .expect("a row with the key")
        .text
}

#[test]
fn a_long_name_title_or_label_is_cut_and_the_rest_of_its_row_kept() {
    // Names, modes and environments of up to 64 bytes are allowed, and spoken.
    let long = "Bartholomew Fitzgerald-Montgomery of the Very Long Name";
    let mut model = in_space(ConnectionRole::Owner);
    model.name = long.into();
    model.knocks = vec![KnockRequest {
        id: "knock-1".into(),
        name: long.into(),
    }];
    model.people[0].name = long.into();
    model.people[1].name = long.into();
    model.people.push(Person {
        name: long.into(),
        colour: [1.0; 3],
        you: false,
        away: false,
    });
    model.pairing = Some(long.into());
    // The role, "away", "in" and the action words survive.
    let people = rows(Some(Section::PeopleAndSecurity), &model);
    // A knock's visitor stays marked as one, however long their name.
    assert!(
        cut_between(&people, "Let Bartholomew", " (visitor) in"),
        "{people:?}"
    );
    assert!(
        cut_between(&people, "Turn Bartholomew", " (visitor) away"),
        "{people:?}"
    );
    assert!(
        cut_between(&people, "You: Bartholomew", " (owner)"),
        "{people:?}"
    );
    assert!(cut_between(&people, "You (Bartholomew", ")"), "{people:?}");
    assert!(cut_between(&people, "Bartholomew", ", away"), "{people:?}");
    // A row that is only a name, such as a pairing label or someone here, is cut to the width.
    let whole = clip(long, WIDTH);
    assert_eq!(whole.chars().count(), WIDTH);
    assert_eq!(
        text_of(Some(Section::PeopleAndSecurity), &model, &MenuKey::Pair),
        whole
    );
    assert_eq!(people.iter().filter(|row| **row == whole).count(), 2);
    // A visitor's own row keeps its role too.
    model.role = ConnectionRole::Visitor;
    let visitor = rows(Some(Section::PeopleAndSecurity), &model);
    assert!(
        cut_between(&visitor, "You: Bartholomew", " (visitor)"),
        "{visitor:?}"
    );

    // In the Space section, a long mode keeps "(now)", and the title, environment and status are cut or wrapped to fit, keeping the rest.
    let mut model = in_space(ConnectionRole::Owner);
    let space = model.space.as_mut().unwrap();
    space.title = format!("{long} {long}");
    space.running = format!("opencode in {long}");
    space.mode = Some(long.into());
    space.modes[0].1 = long.into();
    space.modes[1].1 = long.into();
    space.here = "12 here, 3 away".into();
    space.environment = Some(long.into());
    space.environments = vec![
        ("current".into(), long.into(), true),
        ("other".into(), long.into(), false),
    ];
    let rows_of_space = rows(Some(Section::Space), &model);
    let now = format!("* {} (now)", clip(long, WIDTH - 8));
    let other = format!("  {}", clip(long, WIDTH - 2));
    assert!(
        cut_between(std::slice::from_ref(&now), "* Bartholomew", " (now)"),
        "{now}"
    );
    assert!(
        cut_between(std::slice::from_ref(&other), "  Bartholomew", ""),
        "{other}"
    );
    let space_row = |key: MenuKey| text_of(Some(Section::Space), &model, &key);
    assert_eq!(space_row(MenuKey::Mode("build".into())), now);
    assert_eq!(space_row(MenuKey::Mode("plan".into())), other);
    assert_eq!(space_row(MenuKey::Environment("other".into())), other);
    // The current environment is read, not chosen, after the mode's rows.
    assert_eq!(rows_of_space.iter().filter(|row| **row == now).count(), 2);
    assert!(
        rows_of_space
            .iter()
            .any(|row| row.starts_with("opencode in Bartholomew") && row.ends_with('…')),
        "{rows_of_space:?}"
    );
    // The status keeps the activity, "brief" and how many are here, and cuts the mode.
    assert_eq!(rows_of_space[2], "running | Bar… | brief | 12 here, 3 away");
    model.space.as_mut().unwrap().speech = tomori_protocol::SpeechSetting::Full;
    assert!(cut_between(
        &rows(Some(Section::Space), &model),
        "running | Bartholomew",
        " | 12 here, 3 away"
    ));
    // The title wraps at words, on the top level's two rows and the Space section's after them, before the space's running agent.
    let heading = items(Some(Section::Space), &model);
    assert_eq!(heading[0].kind, MenuKind::Heading);
    assert_eq!(heading[3].key, MenuKey::Back);
    let running = rows_of_space
        .iter()
        .position(|row| row.starts_with("opencode"))
        .unwrap();
    let title = [&rows_of_space[..2], &rows_of_space[4..running]].concat();
    assert!(title.iter().all(|row| !row.contains('…')), "{title:?}");
    assert_eq!(title.join(" "), format!("{long} {long}"));
    assert_eq!(rows_of_space[running].chars().count(), WIDTH);
    // A long mode no one can switch to keeps its label.
    model.space.as_mut().unwrap().modes.clear();
    let modeless = rows(Some(Section::Space), &model);
    assert!(
        cut_between(&modeless, "Mode: Bartholomew", ""),
        "{modeless:?}"
    );
    // Without environments to change to, the environment's name is cut.
    model.space.as_mut().unwrap().environments.clear();
    let fixed = rows(Some(Section::Space), &model);
    assert!(cut_between(&fixed, "Bartholomew", ""), "{fixed:?}");
}

#[test]
fn everyone_here_is_shown_when_the_server_gives_only_names() {
    let mut model = in_space(ConnectionRole::Owner);
    model.people.clear();
    model.participants = [
        "Ada Lovelace",
        "Bob",
        "Charles Babbage",
        "Dorothy",
        "Edsger",
    ]
    .map(String::from)
    .to_vec();
    let people = rows(Some(Section::PeopleAndSecurity), &model);
    let start = people
        .iter()
        .position(|row| row.starts_with("Here: "))
        .unwrap();
    let here = people[start..start + 2].join(" ");
    assert_eq!(
        here,
        "Here: Ada Lovelace, Bob, Charles Babbage, Dorothy, Edsger"
    );
    assert!(people.iter().all(|row| !row.contains('…')), "{people:?}");
}

#[test]
fn the_rows_fill_the_panel_at_the_menus_font() {
    // A row of `WIDTH` cells fits in the panel, with its background's margin after it, and one more would not.
    let reach = |cells: usize| 2.0 * MARKER + ((cells * CELL_WIDTH) as f32 + 2.0) * PIXEL;
    assert!(reach(WIDTH) <= PANEL);
    assert!(reach(WIDTH + 1) > PANEL);
    assert_eq!(WIDTH, 40);
}
