// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The frame step's tests, headset and desktop alike; the grip, scrolling, timers, tracked hands and volumes have files
//! of their own.

use super::fixtures::*;
use super::*;
use crate::{conversation::LEAVE_HOLD, hand_menu::HOLD};
use std::time::Duration;
use tomori_client::{
    connection::Action,
    feedback::{Hand as Side, Haptic},
    playback::Earcon,
};
use tomori_protocol::ConnectionRole;

const FRAME: f32 = 1.0 / 72.0;

#[test]
fn a_home_ring_carried_with_the_room_carries_its_height_with_its_middle() {
    // A ring under a home floating 5.3 m up, carried as a rig standing 0.5 m up and turned a quarter carries it.
    let ring = crate::anchors::HomeRing {
        centre: Vec3::new(1.0, 5.3, -4.0),
        height: 3.9,
        ..crate::anchors::HomeRing::WHOLE
    };
    let rig = crate::anchors::Anchor {
        position: Vec3::new(2.0, 0.5, 0.0),
        orientation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
    };
    let carried = carried_ring(ring, |point| rig.place(point));
    assert!(carried.centre.distance(rig.place(ring.centre)) < 1e-5);
    assert!((carried.height - 4.4).abs() < 1e-5, "{carried:?}");
    let back = carried_ring(carried, |point| rig.unplace(point));
    assert!(back.centre.distance(ring.centre) < 1e-4);
    assert!((back.height - ring.height).abs() < 1e-5);
}

/// A head standing at home, looking ahead.
fn senses() -> Senses {
    let head = Mat4::from_translation(arrival::HOME);
    Senses {
        focused: true,
        head: Some(head),
        head_pose: Pose::from_transform(head),
        viewer: Some(arrival::HOME),
        space_head: head,
        trigger: Some(0.0),
        ..Default::default()
    }
}

/// A right palm `degrees` from facing straight up, held in front of the head.
fn palm_up(degrees: f32) -> Palm {
    let normal = glam::Quat::from_rotation_z(degrees.to_radians()) * Vec3::Y;
    Palm {
        wrist: arrival::HOME + Vec3::new(0.1, -0.35, -0.3),
        normal,
    }
}

/// A left palm `degrees` from facing straight up, held up in front of the head, as the right one in [`palm_up`] is.
fn left_palm_up(degrees: f32) -> Palm {
    Palm {
        wrist: arrival::HOME + Vec3::new(-0.06, -0.2, -0.45),
        ..palm_up(degrees)
    }
}

/// A ray from the eye through the middle of `shape`.
fn aim_at(shape: crate::pointing::Shape) -> Option<Ray> {
    Ray::new(arrival::HOME, shape.centre() - arrival::HOME)
}

/// The target with `key` in `scene`.
fn target(scene: &arrival::Scene, key: &arrival::Key) -> crate::pointing::Target<arrival::Key> {
    scene
        .targets()
        .into_iter()
        .find(|target| &target.key == key)
        .expect("the target")
}

fn earcons(feedback: &mut Feedback) -> Vec<Earcon> {
    feedback.take_earcons().map(|(earcon, _)| earcon).collect()
}

fn felt_on(feedback: &mut Feedback, side: Side, now: Instant) -> Vec<Haptic> {
    feedback
        .haptics(now + Duration::from_secs(1))
        .into_iter()
        .filter(|(hand, _)| *hand == side)
        .map(|(_, haptic)| haptic)
        .collect()
}

fn press(change: impl FnOnce(&mut Buttons)) -> Buttons {
    let mut buttons = Buttons::default();
    change(&mut buttons);
    buttons
}

/// What a fake client side was told each frame: how many updates, what was pointed at, and whether the head was known.
type Told = std::sync::Arc<std::sync::Mutex<Vec<(usize, Option<String>, bool)>>>;

/// A client side that picks whatever the ray meets a metre away, asks and sounds once a frame, and shows how many
/// frames it has had, keeping what it was told.
struct Fake {
    frames: usize,
    told: Told,
}

impl crate::client_side::ClientSide for Fake {
    fn pick(&self, _ray: &Ray) -> Option<crate::client_side::Picked> {
        Some(crate::client_side::Picked::new("thing-1", "the thing", 1.0))
    }

    fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
        self.frames += 1;
        self.told.lock().unwrap().push((
            input.updates.len(),
            input.pointed.map(str::to_owned),
            input.head.is_some(),
        ));
        crate::client_side::Output::new()
            .with_intent(tomori_protocol::Payload::new(serde_json::json!(
                self.frames
            )))
            .with_sound(
                crate::client_side::Sound::new(vec![0.0; 4], 48_000),
                [0.0, 1.0, -1.0],
            )
            .drawing(std::sync::Arc::new(self.frames))
    }
}

#[test]
fn an_environment_s_client_side_is_made_for_its_space_once_it_grows_in_and_presents_what_its_side_shares()
 {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tomori_client::Event;
    use tomori_protocol::Resource;
    use tomori_protocol::{ClientMessage, ServerMessage};
    let made = Arc::new(AtomicUsize::new(0));
    let told = Arc::new(Mutex::new(Vec::new()));
    let factory: crate::client_side::Factory = {
        let (made, told) = (made.clone(), told.clone());
        Arc::new(move |space: &str| {
            assert_eq!(space, SPACE);
            made.fetch_add(1, Ordering::Relaxed);
            Box::new(Fake {
                frames: 0,
                told: told.clone(),
            }) as Box<dyn crate::client_side::ClientSide>
        })
    };
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    assert!(conversation.client_side().is_none());
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let senses = Senses {
        ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
        ..senses()
    };
    conversation.apply(space(|_| {}));
    conversation.take_actions();
    assert_eq!(
        conversation
            .client_side()
            .map(|(environment, _)| environment),
        Some(crate::test_support::SKY)
    );
    // Not while the space's scenery grows in.
    let growing = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(growing.environment.is_none());
    let mut shown = None;
    for _ in 0..400 {
        shown = step
            .update(&senses, &mut conversation, &mut feedback, now, FRAME)
            .environment;
    }
    // Made once, it shows what it shows, sounds, and points at what the ray meets, which the participant attends to.
    assert_eq!(made.load(Ordering::Relaxed), 1);
    let frames = *shown.unwrap().downcast_ref::<usize>().unwrap();
    assert!(frames > 1);
    assert_eq!(
        told.lock().unwrap().last(),
        Some(&(0, Some("thing-1".to_owned()), true))
    );
    assert_eq!(feedback.take_ambient().count(), 1);
    conversation.talk(true);
    conversation.talk(false);
    let actions = conversation.take_actions();
    let Some(Action::BeginUtterance(utterance)) = actions.first() else {
        unreachable!("recording begins, not {actions:?}")
    };
    assert_eq!(utterance.attention, [Resource::object("thing-1")]);
    // Before the server sends the environment's state, its intents are not sent; after, they are, and it is told.
    assert!(!format!("{actions:?}").contains("EnvironmentIntent"));
    conversation.apply(Event::Message(ServerMessage::EnvironmentState {
        environment: "sky".into(),
        version: 1,
        state: tomori_protocol::Payload::new(serde_json::json!({})),
    }));
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(told.lock().unwrap().last().unwrap().0, 1);
    assert!(conversation.take_actions().iter().any(|action| matches!(
        action,
        Action::Send(ClientMessage::EnvironmentIntent { .. })
    )));
    // Leaving, it is gone; joining again, it is made anew.
    conversation.apply(tomori_client::Event::Message(
        tomori_protocol::ServerMessage::LEFT,
    ));
    assert!(
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
            .environment
            .is_none()
    );
    assert!(conversation.client_side().is_none());
    conversation.apply(space(|_| {}));
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    }
    assert_eq!(made.load(Ordering::Relaxed), 2);
    // An environment it has no client side for has none.
    conversation.set_client_sides(Vec::new());
    assert!(conversation.client_side().is_none());
    assert!(
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
            .environment
            .is_none()
    );
}

/// What a client side with a selectable object was told each frame: the trigger, B and the sticks.
pub(super) type Done =
    std::sync::Arc<std::sync::Mutex<Vec<(Option<crate::client_side::Select>, bool, Vec2, Vec2)>>>;

/// A client side whose lamp, a metre along any ray ahead, is selectable, or not, as the farmyard's chickens are not,
/// keeping what the participant did with it.
struct Lamp {
    done: Done,
    selectable: bool,
    /// How far along the ray it is.
    distance: f32,
}

impl crate::client_side::ClientSide for Lamp {
    fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
        let lamp = crate::client_side::Picked::new("lamp", "the lamp", self.distance);
        (ray.direction.z < 0.0).then(|| {
            if self.selectable {
                lamp.selectable()
            } else {
                lamp
            }
        })
    }

    fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
        self.done.lock().unwrap().push((
            input.select,
            input.back,
            input.left_stick,
            input.right_stick,
        ));
        crate::client_side::Output::new()
    }
}

/// Joined to [`SPACE`] with a [`Lamp`], selectable or not, for Sky, where `locomotion` lets the participant travel,
/// and what it is told.
pub(super) fn lamp(
    locomotion: crate::locomotion::Locomotion,
    selectable: bool,
) -> (Conversation, Done) {
    lamp_at(locomotion, selectable, 1.0)
}

/// As [`lamp`], with the lamp `distance` along any ray ahead.
fn lamp_at(
    locomotion: crate::locomotion::Locomotion,
    selectable: bool,
    distance: f32,
) -> (Conversation, Done) {
    let done = Done::default();
    let factory: crate::client_side::Factory = {
        let done = done.clone();
        std::sync::Arc::new(move |_: &str| {
            Box::new(Lamp {
                done: done.clone(),
                selectable,
                distance,
            }) as Box<dyn crate::client_side::ClientSide>
        })
    };
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    conversation.set_locomotion(vec![(crate::test_support::SKY, locomotion)]);
    conversation.apply(space(|_| {}));
    conversation.take_actions();
    (conversation, done)
}

/// `step` on until the space's scenery has grown in, as `senses` find it.
fn grow(step: &mut Step, conversation: &mut Conversation, senses: &Senses, now: Instant) {
    for _ in 0..400 {
        step.update(senses, conversation, &mut felt(), now, FRAME);
    }
}

/// In [`SPACE`] with a selectable [`Lamp`] grown in, pointing at it, with what it is told and `locomotion` for Sky.
fn lit(locomotion: crate::locomotion::Locomotion) -> (Conversation, Step, Senses, Done) {
    let (mut conversation, done) = lamp(locomotion, true);
    let mut step = Step::default();
    let senses = Senses {
        ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
        ..senses()
    };
    grow(&mut step, &mut conversation, &senses, Instant::now());
    done.lock().unwrap().clear();
    (conversation, step, senses, done)
}

/// The selects the lamp was told since this was last asked.
fn selects(done: &Done) -> Vec<crate::client_side::Select> {
    done.lock()
        .unwrap()
        .drain(..)
        .filter_map(|done| done.0)
        .collect()
}

#[test]
fn a_pull_that_outlives_its_client_side_is_nobody_s_until_it_is_let_go() {
    use crate::client_side::Select;
    let now = Instant::now();
    let to = |environment: &'static str| {
        move |conversation: &mut Conversation| {
            conversation.apply(space(|state| state.environment = environment.into()));
        }
    };
    type Change = Box<dyn Fn(&mut Conversation)>;
    let changes: [Vec<Change>; 2] = [
        // The space moves to an environment with no client side, and back.
        vec![Box::new(to("minimal")), Box::new(to("sky"))],
        // Another space.
        vec![Box::new(|conversation: &mut Conversation| {
            conversation.apply(space(|state| state.id = "space-2".into()));
        })],
    ];
    for change in changes {
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        senses.trigger = Some(0.3);
        assert!(
            !step
                .update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .attracting
        );
        // Changed mid-pull, its client side goes and is made anew, and the pull, held hard, never attracts halfway,
        // nor selects the new one's lamp.
        senses.trigger = Some(1.0);
        for change in &change {
            change(&mut conversation);
            for _ in 0..400 {
                let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
                assert!(!frame.attracting);
            }
        }
        assert!(step.client.is_some());
        assert!(selects(&done).is_empty());
        // Let go, the next pull on the lamp is the new client side's.
        senses.trigger = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        senses.trigger = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    }
    // A pull begun before there was a client side stays nobody's once it is made, as it began, drawing nothing in
    // a space.
    let (mut conversation, done) = lamp(Default::default(), true);
    let mut step = Step::default();
    let senses = Senses {
        ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
        trigger: Some(1.0),
        ..senses()
    };
    let mut attracting = Vec::new();
    for _ in 0..400 {
        attracting.push(
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .attracting,
        );
    }
    assert!(step.client.is_some());
    assert!(attracting.iter().all(|attracting| !*attracting));
    assert!(selects(&done).is_empty());
}

#[test]
fn b_goes_to_the_first_rung_that_wants_it_and_to_the_environment_only_when_none_does() {
    use crate::conversation::Back;
    use tomori_client::Event;
    use tomori_protocol::{Panel, Place, QueueEntry, QueueState, ServerMessage, SpeechSetting};
    /// A projected panel 0.6 m ahead, nearer than the lamp, so that the ray points at it.
    fn panel(conversation: &mut Conversation) {
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
        fixtures::show_projection(
            conversation,
            Vec::new(),
            vec![Panel::new(
                "panel-1",
                "Build status",
                "Green.",
                Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        ahead.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )],
        );
    }
    /// A wall of `kind` 0.6 m ahead, an empty slot of its top row where the ray meets it, nearer than the lamp, taken
    /// hold of with the grip: carried, if someone added it, or else held, its handles showing.
    fn wall(
        conversation: &mut Conversation,
        step: &mut Step,
        senses: &mut Senses,
        now: Instant,
        kind: tomori_protocol::WallKind,
    ) {
        let middle = arrival::HOME + Vec3::new(0.0, -0.3, -0.6);
        fixtures::show_editable_walls(
            conversation,
            vec![tomori_protocol::Wall::new(
                "wall-1",
                kind,
                tomori_protocol::Pose::upright_facing(
                    middle.to_array(),
                    [arrival::HOME.x, middle.y, arrival::HOME.z],
                ),
            )],
            Vec::new(),
        );
        grow(step, conversation, senses, now);
        senses.grip = Some(1.0);
        step.update(senses, conversation, &mut felt(), now, FRAME);
        if kind != tomori_protocol::WallKind::Added {
            senses.grip = Some(0.0);
            step.update(senses, conversation, &mut felt(), now, FRAME);
        }
        assert_eq!(step.projector.editing_wall(), Some("wall-1"));
    }
    fn failed(conversation: &mut Conversation, error: &str) {
        conversation.apply(Event::Message(ServerMessage::TranscriptFailed {
            utterance: "utterance-9".into(),
            error: error.into(),
        }));
    }
    type Setup = fn(&mut Conversation, &mut Step, &mut Senses, Instant);
    let rows: [(Back, Setup); 14] = [
        (Back::Carried, |conversation, step, senses, now| {
            panel(conversation);
            senses.grip = Some(1.0);
            grow(step, conversation, senses, now);
            assert!(step.projector.held().is_some());
        }),
        (Back::CarriedWall, |conversation, step, senses, now| {
            wall(
                conversation,
                step,
                senses,
                now,
                tomori_protocol::WallKind::Added,
            );
            assert_eq!(step.projector.carried_wall(), Some("wall-1"));
        }),
        (Back::HeldWall, |conversation, step, senses, now| {
            wall(
                conversation,
                step,
                senses,
                now,
                tomori_protocol::WallKind::Environment,
            );
            assert_eq!(step.projector.carried_wall(), None);
        }),
        (Back::Pinned, |conversation, step, _, now| {
            step.buttons(
                &press(|buttons| buttons.y = true),
                true,
                conversation,
                &mut felt(),
                now,
            );
            assert!(step.pinned());
        }),
        // Showing, with something to back out of: here a problem, which it dismisses ahead of the space.
        (Back::Wrist, |conversation, step, senses, now| {
            senses.right_palm = Some(palm_up(0.0));
            wrist_after(step, conversation, senses, now);
            assert!(step.wrist.state().active());
            failed(conversation, "The recogniser failed.");
        }),
        (Back::Modes, |conversation, _, _, _| {
            conversation.apply(space(|state| state.speech = SpeechSetting::Full));
            conversation.refresh();
        }),
        (Back::Queued, |conversation, _, _, _| {
            conversation.apply(space(|state| {
                state.queue = vec![QueueEntry {
                    id: "entry-1".into(),
                    source: 1,
                    speaker: "Ada".into(),
                    text: "Fix the parser.".into(),
                    state: QueueState::Queued,
                }]
            }));
            conversation.select_queued(1);
        }),
        (Back::Naming, |conversation, _, _, _| {
            conversation.apply(Event::Message(ServerMessage::Name { name: "Ada".into() }));
            conversation.choose_menu(crate::hand_menu::Choice::Rename);
        }),
        (Back::Unheard, |conversation, _, _, _| {
            failed(conversation, tomori_protocol::NO_SPEECH)
        }),
        (Back::Problem, |conversation, _, _, _| {
            failed(conversation, "The recogniser failed.")
        }),
        (Back::SpaceProblem, |conversation, _, _, _| {
            conversation.apply(space(|state| {
                state.error = Some("The agent stopped.".into())
            }));
        }),
        (Back::Panel, |conversation, step, senses, now| {
            panel(conversation);
            grow(step, conversation, senses, now);
        }),
        (Back::Environment, |_, _, _, _| {}),
        (Back::Nothing, |conversation, step, senses, now| {
            senses.ray = Ray::new(arrival::HOME, Vec3::Z);
            step.update(senses, conversation, &mut felt(), now, FRAME);
        }),
    ];
    for (rung, setup) in rows {
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        let now = Instant::now();
        setup(&mut conversation, &mut step, &mut senses, now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        // The rungs the conversation holds take B even while the lamp is pointed at.
        let pointed = step.client.as_ref().unwrap().focus.is_some();
        if matches!(
            rung,
            Back::Modes
                | Back::Queued
                | Back::Naming
                | Back::Unheard
                | Back::Problem
                | Back::SpaceProblem
                | Back::Environment
        ) {
            assert!(pointed, "{rung:?}");
        }
        done.lock().unwrap().clear();
        let b = |step: &mut Step, conversation: &mut Conversation, pressed, at| {
            step.buttons(
                &press(|buttons| {
                    buttons.back = pressed;
                    buttons.back_held = true;
                }),
                true,
                conversation,
                &mut felt(),
                at,
            );
        };
        b(&mut step, &mut conversation, true, now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.last_back, Some(rung));
        let told = done.lock().unwrap().iter().filter(|done| done.1).count();
        assert_eq!(told, usize::from(rung == Back::Environment), "{rung:?}");
        // Held, it leaves after a second, except on a panel or a wall carried, where it never does.
        b(&mut step, &mut conversation, false, now + LEAVE_HOLD / 2);
        assert!(conversation.joined_space().is_some(), "{rung:?}");
        b(&mut step, &mut conversation, false, now + LEAVE_HOLD);
        let left = conversation
            .take_actions()
            .contains(&Action::Send(tomori_protocol::ClientMessage::Leave));
        assert_eq!(
            left,
            !matches!(rung, Back::Carried | Back::CarriedWall),
            "{rung:?}"
        );
    }
    // The rows are in the order `Back` declares its rungs, which is B's order.
    assert!(rows.windows(2).all(|pair| pair[0].0 < pair[1].0));
    // Any two rungs at once, the later set up first, as what was open before: the earlier takes B, and the
    // environment is told nothing. Nothing is what is left when no rung wants B, so it pairs with none; and a panel
    // carried is the panel pointed at, is not taken up while the hand menu is pinned or the transcript shows, and
    // keeps the transcript away, so it pairs with none of those.
    for (index, (first, before)) in rows.iter().enumerate() {
        for (then, after) in &rows[index + 1..] {
            // A wall carried is let go as a panel carried is; no wall is taken hold of while the hand menu is
            // pinned or the transcript shows, as they take the ray; and each shows its own projection, which a
            // panel's replaces.
            if *then == Back::Nothing
                || (matches!(first, Back::Carried | Back::CarriedWall)
                    && matches!(
                        then,
                        Back::CarriedWall
                            | Back::HeldWall
                            | Back::Pinned
                            | Back::Wrist
                            | Back::Panel
                    ))
                || (*first == Back::HeldWall
                    && matches!(then, Back::Pinned | Back::Wrist | Back::Panel))
            {
                continue;
            }
            let (mut conversation, mut step, mut senses, done) = lit(Default::default());
            let now = Instant::now();
            after(&mut conversation, &mut step, &mut senses, now);
            before(&mut conversation, &mut step, &mut senses, now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            done.lock().unwrap().clear();
            step.buttons(
                &press(|buttons| buttons.back = true),
                true,
                &mut conversation,
                &mut felt(),
                now,
            );
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(step.last_back, Some(*first), "{first:?} before {then:?}");
            let told = done.lock().unwrap().iter().filter(|done| done.1).count();
            assert_eq!(told, 0, "{first:?} before {then:?}");
        }
    }
}

#[test]
fn a_trigger_pull_on_a_selectable_object_is_the_environment_s_and_never_attracts() {
    use crate::client_side::Select;
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    let now = Instant::now();
    let mut pulls = Vec::new();
    for trigger in [0.3, 1.0, 1.0, 0.0] {
        senses.trigger = Some(trigger);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        pulls.push(frame.attracting);
    }
    assert_eq!(pulls, [false; 4], "a pull on the lamp never attracts");
    let selects: Vec<_> = done
        .lock()
        .unwrap()
        .drain(..)
        .filter_map(|done| done.0)
        .collect();
    assert_eq!(
        selects,
        [
            Select::Pressed("lamp".into()),
            Select::Released("lamp".into())
        ]
    );
    // Pointing at nothing of the environment's, a pull selects nothing, and in a space draws the swarm nowhere.
    senses.ray = Ray::new(arrival::HOME, Vec3::Z);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    senses.trigger = Some(1.0);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(!frame.attracting);
    assert!(done.lock().unwrap().iter().all(|done| done.0.is_none()));
}

#[test]
fn b_is_the_environment_s_only_while_its_object_is_pointed_at_and_nothing_else_wants_it() {
    let (mut conversation, mut step, senses, done) = lit(Default::default());
    let now = Instant::now();
    let mut feedback = felt();
    let mut b = |step: &mut Step, conversation: &mut Conversation, pressed, held, at| {
        step.buttons(
            &press(|buttons| {
                buttons.back = pressed;
                buttons.back_held = held;
            }),
            true,
            conversation,
            &mut feedback,
            at,
        );
    };
    let backs = |done: &Done| done.lock().unwrap().drain(..).filter(|done| done.1).count();
    // Pointed at, B is the lamp's, once, without the notice that holding it leaves.
    b(&mut step, &mut conversation, true, false, now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(backs(&done), 1);
    assert_eq!(conversation.notice(), None);
    // A problem shown takes B first.
    conversation.apply(space(|state| {
        state.error = Some("The agent stopped.".into())
    }));
    b(&mut step, &mut conversation, true, false, now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(backs(&done), 0);
    // Losing focus forgets a press not yet told.
    b(&mut step, &mut conversation, true, false, now);
    step.blur(&mut conversation);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(backs(&done), 0);
    // Held, it still leaves, after a second, as ever.
    let later = now + Duration::from_secs(5);
    b(&mut step, &mut conversation, true, true, later);
    step.update(&senses, &mut conversation, &mut felt(), later, FRAME);
    assert_eq!(backs(&done), 1);
    b(
        &mut step,
        &mut conversation,
        false,
        true,
        later + LEAVE_HOLD / 2,
    );
    assert!(conversation.joined_space().is_some());
    b(
        &mut step,
        &mut conversation,
        false,
        true,
        later + LEAVE_HOLD,
    );
    assert!(
        conversation
            .take_actions()
            .contains(&Action::Send(tomori_protocol::ClientMessage::Leave))
    );
}

#[test]
fn b_pointing_elsewhere_keeps_its_meaning() {
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    let now = Instant::now();
    senses.ray = Ray::new(arrival::HOME, Vec3::Z);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    step.buttons(
        &press(|buttons| buttons.back = true),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(done.lock().unwrap().iter().all(|done| !done.1));
    assert_eq!(conversation.notice(), Some("Hold B to leave the space."));
}

/// The press and hold of A, as the controller reports it.
fn a_down() -> Buttons {
    press(|buttons| {
        buttons.a = true;
        buttons.a_held = true;
    })
}

/// A held, no new press.
fn a_held() -> Buttons {
    press(|buttons| buttons.a_held = true)
}

/// What A is heard to do, by the choosing cue, of a frame step with `feedback`.
fn chose(feedback: &mut Feedback) -> bool {
    earcons(feedback).contains(&Earcon::Choose)
}

#[test]
fn a_goes_to_the_first_rung_that_wants_it_and_to_the_environment_only_when_none_does() {
    use crate::hand_menu::MenuKey;
    use precedence::A;
    use tomori_protocol::{Approval, ApprovalOption, QueueEntry, QueueState, SpeechSetting};
    /// Whether what A would do, were it the rung's, was done.
    type Effect = Box<dyn Fn(&mut Conversation, &mut Feedback) -> bool>;
    type Setup = fn(&mut Conversation, &mut Step, &mut Senses, Instant) -> Effect;
    /// What is set up after the last frame before A, which a frame would undo.
    type Late = fn(&mut Conversation);
    fn nothing(_: &mut Conversation) {}
    fn request(conversation: &mut Conversation) {
        conversation.apply(space(|state| {
            state.approval = Some(Approval {
                id: "request-1".into(),
                title: "Bash".into(),
                text: "cargo test".into(),
                options: vec![ApprovalOption {
                    id: "once".into(),
                    label: "Allow once".into(),
                }],
            });
        }));
        conversation.take_actions();
    }
    /// The hand menu pinned with Y and shown, with the ray on the Mute row, or beside every row.
    fn menu(
        conversation: &mut Conversation,
        step: &mut Step,
        senses: &mut Senses,
        now: Instant,
        row: bool,
    ) {
        step.buttons(
            &press(|buttons| buttons.y = true),
            true,
            conversation,
            &mut felt(),
            now,
        );
        senses.left_palm = Some(left_palm_up(0.0));
        let scene = step
            .update(senses, conversation, &mut felt(), now, FRAME)
            .arrival
            .expect("the pinned menu")
            .scene;
        if row {
            let mute = arrival::Key::Menu(MenuKey::Mute);
            senses.ray = aim_at(target(&scene, &mute).shape);
            senses.beam = true;
            step.update(senses, conversation, &mut felt(), now, FRAME);
            assert_eq!(step.hands.right.highlighted, Some(mute));
        }
    }
    /// The participant pointing at the queue's choosable row `pod112`, as the frame before found it.
    fn on_row(conversation: &mut Conversation) {
        conversation.point_item(Some(("panel-2".into(), 1)));
    }
    /// Whether A chose the row pointed at, which the environment would be told.
    fn chose_row(actions: &[Action]) -> bool {
        actions.contains(&Action::Send(
            tomori_protocol::ClientMessage::ChoosePanelRow {
                panel: "panel-2".into(),
                row: "pod112".into(),
                revision: Some(6),
                key: None,
            },
        ))
    }
    /// The queue floating behind the lamp, which the ray meets first.
    fn queue(conversation: &mut Conversation) {
        show_queue(
            conversation,
            arrival::HOME + Vec3::new(0.0, 0.0, -2.0),
            arrival::HOME,
        );
    }
    let rows: [(A, &str, Setup, Late); 18] = [
        (
            A::MenuRow,
            "a hand menu row",
            |conversation, step, senses, now| {
                menu(conversation, step, senses, now, true);
                let muted = conversation.menu_model().muted;
                Box::new(move |conversation, _| conversation.menu_model().muted != muted)
            },
            nothing,
        ),
        (
            A::Menu,
            "the hand menu, off its rows",
            |conversation, step, senses, now| {
                // A request waiting, which A would decide were it not for the menu, which A chooses only in.
                request(conversation);
                conversation.settle_request();
                menu(conversation, step, senses, now, false);
                assert!(step.hands.right.highlighted.is_none());
                Box::new(|conversation, _| {
                    conversation.take_actions().is_empty() && !conversation.a_refused()
                })
            },
            nothing,
        ),
        (
            A::Transcript,
            "the transcript",
            |conversation, step, senses, now| {
                conversation.apply(space(|state| {
                    state.messages = [
                        (tomori_protocol::Role::User, "Fix the parser."),
                        (tomori_protocol::Role::Assistant, "The parser is fixed."),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (role, text))| tomori_protocol::Message {
                        id: format!("message-{index}"),
                        role,
                        text: text.into(),
                        completed: true,
                    })
                    .collect();
                }));
                senses.right_palm = Some(palm_up(0.0));
                wrist_after(step, conversation, senses, now);
                assert!(step.wrist.state().active());
                // The right stick moves the focus onto a turn, which A then expands, heard.
                step.buttons(
                    &press(|buttons| buttons.right_stick = Vec2::new(0.0, 1.0)),
                    true,
                    conversation,
                    &mut felt(),
                    now,
                );
                step.buttons(&Buttons::default(), true, conversation, &mut felt(), now);
                Box::new(|_, feedback| chose(feedback))
            },
            nothing,
        ),
        (
            A::Conversation,
            "the list of modes",
            |conversation, _, _, _| {
                conversation.apply(space(|state| state.speech = SpeechSetting::Full));
                conversation.refresh();
                Box::new(|conversation, feedback| {
                    chose(feedback)
                        && conversation.take_actions().contains(&Action::Send(
                            tomori_protocol::ClientMessage::SetSpeech {
                                speech: SpeechSetting::Brief,
                            },
                        ))
                })
            },
            nothing,
        ),
        (
            A::Conversation,
            "a request that has just appeared",
            |conversation, _, _, _| {
                request(conversation);
                Box::new(|conversation, feedback| {
                    conversation.a_refused()
                        && conversation.notice() == Some("Read the request before choosing.")
                        && !chose(feedback)
                        && conversation.take_actions().is_empty()
                })
            },
            nothing,
        ),
        (
            A::Conversation,
            "a request that has just appeared, before a choosable row",
            |conversation, _, _, _| {
                queue(conversation);
                request(conversation);
                Box::new(|conversation, feedback| {
                    conversation.a_refused()
                        && !chose(feedback)
                        && !chose_row(&conversation.take_actions())
                })
            },
            on_row,
        ),
        (
            A::Conversation,
            "a checklist item pointed at",
            |conversation, _, _, _| {
                // Behind the lamp, which the ray meets first.
                let behind = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
                show_checklist(conversation, behind, arrival::HOME);
                Box::new(|conversation, feedback| {
                    chose(feedback)
                        && conversation.take_actions()
                            == [Action::Send(
                                tomori_protocol::ClientMessage::CheckPanelItem {
                                    panel: "panel-1".into(),
                                    line: 2,
                                    checked: false,
                                    revision: Some(4),
                                },
                            )]
                })
            },
            |conversation| conversation.point_item(Some(("panel-1".into(), 2))),
        ),
        (
            A::Conversation,
            "a request to turn the right wrist to decide",
            |conversation, _, _, _| {
                conversation.decide_on_wrist();
                request(conversation);
                conversation.settle_request();
                Box::new(|conversation, feedback| {
                    conversation.a_refused()
                        && conversation.notice() == Some("Turn your right wrist to decide.")
                        && !chose(feedback)
                        && conversation.take_actions().is_empty()
                })
            },
            nothing,
        ),
        (
            A::Conversation,
            "a request to decide",
            |conversation, _, _, _| {
                request(conversation);
                conversation.settle_request();
                Box::new(|conversation, feedback| {
                    chose(feedback)
                        && conversation.take_actions()
                            == [Action::Send(tomori_protocol::ClientMessage::Decide {
                                request: "request-1".into(),
                                decision: tomori_protocol::Decision::AllowOnce,
                                option: None,
                            })]
                })
            },
            nothing,
        ),
        (
            A::Conversation,
            "a request to decide, before a choosable row",
            |conversation, _, _, _| {
                queue(conversation);
                request(conversation);
                conversation.settle_request();
                Box::new(|conversation, feedback| {
                    let actions = conversation.take_actions();
                    chose(feedback)
                        && actions
                            == [Action::Send(tomori_protocol::ClientMessage::Decide {
                                request: "request-1".into(),
                                decision: tomori_protocol::Decision::AllowOnce,
                                option: None,
                            })]
                })
            },
            on_row,
        ),
        (
            A::Conversation,
            "a question waiting, which refuses a choosable row",
            |conversation, _, _, _| {
                queue(conversation);
                conversation.apply(space(|state| {
                    state.projection = true;
                    state.choices = true;
                    state.question = Some(tomori_protocol::Question {
                        id: "question-1".into(),
                        title: "Which?".into(),
                        text: "Name it".into(),
                        field: "field-1".into(),
                        supported: true,
                    });
                }));
                conversation.settle_request();
                conversation.take_actions();
                Box::new(|conversation, feedback| {
                    conversation.a_refused()
                        && conversation.notice() == Some("Answer the request before choosing.")
                        && !chose(feedback)
                        && conversation.take_actions().is_empty()
                })
            },
            on_row,
        ),
        (
            A::Conversation,
            "a selected queue entry",
            |conversation, _, _, _| {
                conversation.apply(space(|state| {
                    state.queue = vec![QueueEntry {
                        id: "entry-1".into(),
                        source: 1,
                        speaker: "Ada".into(),
                        text: "Fix the parser.".into(),
                        state: QueueState::Queued,
                    }]
                }));
                conversation.select_queued(1);
                Box::new(|conversation, feedback| {
                    chose(feedback) && conversation.text().contains("cancelling, A undoes")
                })
            },
            nothing,
        ),
        (
            A::Conversation,
            "a selected queue entry, before a choosable row",
            |conversation, _, _, _| {
                queue(conversation);
                conversation.apply(space(|state| {
                    state.projection = true;
                    state.choices = true;
                    state.queue = vec![QueueEntry {
                        id: "entry-1".into(),
                        source: 1,
                        speaker: "Ada".into(),
                        text: "Fix the parser.".into(),
                        state: QueueState::Queued,
                    }]
                }));
                conversation.select_queued(1);
                conversation.take_actions();
                Box::new(|conversation, feedback| {
                    chose(feedback)
                        && conversation.text().contains("cancelling, A undoes")
                        && !chose_row(&conversation.take_actions())
                })
            },
            on_row,
        ),
        (
            A::Conversation,
            "a choosable row of an environment's list pointed at, after everything that decides",
            |conversation, _, _, _| {
                queue(conversation);
                Box::new(|conversation, feedback| {
                    chose(feedback)
                        && conversation.take_actions()
                            == [Action::Send(
                                tomori_protocol::ClientMessage::ChoosePanelRow {
                                    panel: "panel-2".into(),
                                    row: "pod112".into(),
                                    revision: Some(6),
                                    key: None,
                                },
                            )]
                })
            },
            on_row,
        ),
        (
            A::Conversation,
            "a teleport pad pointed at, the last of its meanings",
            |conversation, _, senses, _| {
                // Behind the head, where the lamp, which is ahead, does not pick; the pad takes A there, and, as
                // the frame before found it, would do before the lamp were it still in focus.
                conversation.set_locomotion(vec![(crate::test_support::SKY, pad_behind_home())]);
                senses.ray = Ray::new(arrival::HOME, Vec3::new(0.0, 0.3, 2.0) - arrival::HOME);
                Box::new(|conversation, feedback| {
                    chose(feedback) && conversation.take_actions().is_empty()
                })
            },
            nothing,
        ),
        (
            A::Tomori,
            "a wall in front of the lamp",
            |conversation, step, senses, now| {
                let middle = arrival::HOME + Vec3::new(0.0, -0.3, -0.6);
                fixtures::show_editable_walls(
                    conversation,
                    vec![tomori_protocol::Wall::new(
                        "wall-1",
                        tomori_protocol::WallKind::Added,
                        tomori_protocol::Pose::upright_facing(
                            middle.to_array(),
                            [arrival::HOME.x, middle.y, arrival::HOME.z],
                        ),
                    )],
                    Vec::new(),
                );
                grow(step, conversation, senses, now);
                Box::new(|conversation, feedback| {
                    conversation.take_actions().is_empty() && !chose(feedback)
                })
            },
            nothing,
        ),
        (
            A::Environment,
            "the lamp, with nothing before it",
            |_, _, _, _| Box::new(|_, feedback| !chose(feedback)),
            nothing,
        ),
        (
            A::Nothing,
            "the ray on nothing",
            |conversation, step, senses, now| {
                senses.ray = Ray::new(arrival::HOME, Vec3::Z);
                step.update(senses, conversation, &mut felt(), now, FRAME);
                Box::new(|conversation, feedback| {
                    conversation.take_actions().is_empty() && !chose(feedback)
                })
            },
            nothing,
        ),
    ];
    // The rows are in the order `A` declares its rungs, which is A's order.
    let position = |rung: A| A::ORDER.iter().position(|other| *other == rung).unwrap();
    assert!(
        rows.windows(2)
            .all(|pair| position(pair[0].0) <= position(pair[1].0))
    );
    for rung in A::ORDER {
        assert!(rows.iter().any(|row| row.0 == *rung), "{rung:?}");
    }
    for (rung, label, setup, late) in rows {
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        let now = Instant::now();
        let effect = setup(&mut conversation, &mut step, &mut senses, now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        late(&mut conversation);
        let focus = |step: &Step| step.client.as_ref().unwrap().focus.clone();
        // The rungs of the frame step take A while the ray is not the environment's, so the lamp is not in focus
        // there; forcing it in shows that A goes first to them, as it would on the frame the menu or the
        // transcript appears, before the frame step has found what is pointed at. Everywhere else the lamp is
        // pointed at, as the frame before found it.
        if matches!(rung, A::MenuRow | A::Menu | A::Transcript) || step.hands.right.pad.is_some() {
            step.client.as_mut().unwrap().focus = Some("lamp".into());
        }
        assert_eq!(
            focus(&step).as_deref(),
            (rung != A::Nothing).then_some("lamp"),
            "{label}"
        );
        assert_eq!(
            step.hands.right.aim == precedence::Aim::Tomori,
            matches!(rung, A::MenuRow | A::Menu | A::Transcript | A::Tomori),
            "{label}"
        );
        done.lock().unwrap().clear();
        // Pressed and held, A goes to the rung, and to the lamp only if no rung before it wants it.
        let mut feedback = felt();
        step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let wanted = effect(&mut conversation, &mut feedback);
        assert!(wanted, "{label}");
        // The environment's own is told that the lamp was pressed and, once A is let go, that it was let go.
        assert_eq!(
            selects(&done),
            if rung == A::Environment {
                vec![crate::client_side::Select::Pressed("lamp".into())]
            } else {
                Vec::new()
            },
            "{label}"
        );
        step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(selects(&done).is_empty(), "{label}");
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(
            selects(&done),
            if rung == A::Environment {
                vec![crate::client_side::Select::Released("lamp".into())]
            } else {
                Vec::new()
            },
            "{label}"
        );
    }
}

#[test]
fn a_selects_only_a_selectable_object_pointed_at_and_only_with_input() {
    use crate::client_side::Select;
    let now = Instant::now();
    // An object that is not selectable, as the farmyard's chickens are not, is not selected by A.
    let (mut conversation, done) = lamp(Default::default(), false);
    let mut step = Step::default();
    let senses = Senses {
        ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
        ..senses()
    };
    grow(&mut step, &mut conversation, &senses, now);
    assert_eq!(step.hands.right.aim, precedence::Aim::Environment);
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(selects(&done).is_empty());
    // Without a client side, A does what it always did, and nothing is told.
    let mut conversation = joined();
    let mut step = Step::default();
    grow(&mut step, &mut conversation, &senses, now);
    assert!(step.client.is_none());
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.a_press.on, None);
    // Without focus, a press selects nothing, and one held is let go.
    let (mut conversation, mut step, senses, done) = lit(Default::default());
    step.buttons(&a_down(), false, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(selects(&done).is_empty());
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    step.buttons(&a_held(), false, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Released("lamp".into())]);
    // Losing focus, as the platform reports it, lets go once, whether or not buttons are read between.
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    step.blur(&mut conversation);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Released("lamp".into())]);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(selects(&done).is_empty());
    // Pressed and let go within a frame, it is let go on the next.
    let click = press(|buttons| buttons.a = true);
    step.buttons(&click, true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Released("lamp".into())]);
}

#[test]
fn a_never_draws_the_swarm_and_the_trigger_and_a_select_one_at_a_time() {
    use crate::client_side::Select;
    let now = Instant::now();
    let lamp = || "lamp".to_owned();
    let frame =
        |step: &mut Step, conversation: &mut Conversation, senses: &Senses, buttons: Buttons| {
            step.buttons(&buttons, true, conversation, &mut felt(), now);
            step.update(senses, conversation, &mut felt(), now, FRAME)
                .attracting
        };
    // A pull of the trigger pressed on the lamp keeps the select: a press of A is ignored, and letting go of it,
    // or of the pull, tells nothing more than the pull's own release.
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    senses.trigger = Some(1.0);
    assert!(!frame(
        &mut step,
        &mut conversation,
        &senses,
        Buttons::default()
    ));
    assert_eq!(selects(&done), [Select::Pressed(lamp())]);
    assert!(!frame(&mut step, &mut conversation, &senses, a_down()));
    assert!(!frame(&mut step, &mut conversation, &senses, a_held()));
    assert!(!frame(
        &mut step,
        &mut conversation,
        &senses,
        Buttons::default()
    ));
    assert!(selects(&done).is_empty());
    senses.trigger = Some(0.0);
    frame(&mut step, &mut conversation, &senses, a_down());
    assert_eq!(selects(&done), [Select::Released(lamp())]);
    frame(&mut step, &mut conversation, &senses, Buttons::default());
    assert!(selects(&done).is_empty());
    // A press of A on the lamp keeps it: a pull of the trigger that begins meanwhile is nobody's, so that it
    // selects nothing, and, begun on the lamp, never attracts the swarm either, even once A is let go.
    senses.trigger = Some(0.0);
    frame(&mut step, &mut conversation, &senses, a_down());
    assert_eq!(selects(&done), [Select::Pressed(lamp())]);
    senses.trigger = Some(1.0);
    assert!(!frame(&mut step, &mut conversation, &senses, a_held()));
    assert!(!frame(
        &mut step,
        &mut conversation,
        &senses,
        Buttons::default()
    ));
    assert_eq!(selects(&done), [Select::Released(lamp())]);
    assert!(!frame(
        &mut step,
        &mut conversation,
        &senses,
        Buttons::default()
    ));
    senses.trigger = Some(0.0);
    frame(&mut step, &mut conversation, &senses, Buttons::default());
    senses.trigger = Some(1.0);
    frame(&mut step, &mut conversation, &senses, Buttons::default());
    assert_eq!(selects(&done), [Select::Pressed(lamp())]);
    senses.trigger = Some(0.0);
    frame(&mut step, &mut conversation, &senses, Buttons::default());
    assert_eq!(selects(&done), [Select::Released(lamp())]);
    // A pull that begins on nothing is nobody's, drawing nothing in a space, and a press of A across the lamp
    // selects it, as letting go of the pull does nothing to it.
    let away = Senses {
        ray: Ray::new(arrival::HOME, Vec3::Z),
        ..senses
    };
    frame(&mut step, &mut conversation, &away, Buttons::default());
    let mut pulling = away;
    pulling.trigger = Some(1.0);
    assert!(!frame(
        &mut step,
        &mut conversation,
        &pulling,
        Buttons::default()
    ));
    let mut across = senses;
    across.trigger = Some(1.0);
    frame(&mut step, &mut conversation, &across, Buttons::default());
    assert!(selects(&done).is_empty());
    frame(&mut step, &mut conversation, &across, a_down());
    assert_eq!(selects(&done), [Select::Pressed(lamp())]);
    assert!(!frame(&mut step, &mut conversation, &across, a_held()));
    across.trigger = Some(0.0);
    frame(&mut step, &mut conversation, &across, Buttons::default());
    assert_eq!(selects(&done), [Select::Released(lamp())]);
}

#[test]
fn a_press_that_outlives_its_client_side_is_let_go_without_telling_the_next() {
    use crate::client_side::Select;
    let now = Instant::now();
    let (mut conversation, mut step, senses, done) = lit(Default::default());
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    // The space moves to an environment with no client side, and back, with A held throughout.
    for environment in ["minimal", "sky"] {
        conversation.apply(space(|state| state.environment = environment.into()));
        for _ in 0..400 {
            step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        }
    }
    assert!(step.client.is_some());
    assert!(selects(&done).is_empty());
    // Let go, it tells the new client side nothing; the next press selects its lamp.
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(selects(&done).is_empty());
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
}

#[test]
fn a_press_waits_for_a_frame_that_is_drawn_and_is_told_once() {
    use crate::client_side::Select;
    let now = Instant::now();
    let lamp = || "lamp".to_owned();
    let (mut conversation, mut step, senses, done) = lit(Default::default());
    let read = |step: &mut Step, conversation: &mut Conversation, buttons: Buttons, focus| {
        step.buttons(&buttons, focus, conversation, &mut felt(), now)
    };
    let draw = |step: &mut Step, conversation: &mut Conversation| {
        step.update(&senses, conversation, &mut felt(), now, FRAME);
        selects(&done)
    };
    // The headset reads its buttons on frames it does not draw: a press, and A held after it, wait.
    read(&mut step, &mut conversation, a_down(), true);
    read(&mut step, &mut conversation, a_held(), true);
    read(&mut step, &mut conversation, a_held(), true);
    assert_eq!(
        draw(&mut step, &mut conversation),
        [Select::Pressed(lamp())]
    );
    assert!(draw(&mut step, &mut conversation).is_empty());
    read(&mut step, &mut conversation, Buttons::default(), true);
    assert_eq!(
        draw(&mut step, &mut conversation),
        [Select::Released(lamp())]
    );
    // Pressed and let go before the next frame drawn: pressed there, and let go on the one after.
    read(&mut step, &mut conversation, a_down(), true);
    read(&mut step, &mut conversation, Buttons::default(), true);
    assert_eq!(
        draw(&mut step, &mut conversation),
        [Select::Pressed(lamp())]
    );
    assert_eq!(
        draw(&mut step, &mut conversation),
        [Select::Released(lamp())]
    );
    assert!(draw(&mut step, &mut conversation).is_empty());
    // Pressed twice, it is still one select, as held and let go as it is.
    read(&mut step, &mut conversation, a_down(), true);
    read(&mut step, &mut conversation, a_down(), true);
    read(&mut step, &mut conversation, a_held(), true);
    assert_eq!(
        draw(&mut step, &mut conversation),
        [Select::Pressed(lamp())]
    );
    read(&mut step, &mut conversation, Buttons::default(), true);
    assert_eq!(
        draw(&mut step, &mut conversation),
        [Select::Released(lamp())]
    );
    // A press that waits is forgotten on losing focus, as the platform reports it or with `blur`.
    read(&mut step, &mut conversation, a_down(), true);
    read(&mut step, &mut conversation, a_held(), false);
    assert!(draw(&mut step, &mut conversation).is_empty());
    read(&mut step, &mut conversation, a_down(), true);
    step.blur(&mut conversation);
    assert!(draw(&mut step, &mut conversation).is_empty());
    // A held through a long skip, then drawn, is pressed once however many frames it was read on.
    for _ in 0..100 {
        read(&mut step, &mut conversation, a_held(), true);
    }
    assert!(draw(&mut step, &mut conversation).is_empty());
    read(&mut step, &mut conversation, Buttons::default(), true);
    assert!(draw(&mut step, &mut conversation).is_empty());
}

#[test]
fn a_pressed_in_the_frame_the_hand_menu_appears_selects_nothing() {
    let now = Instant::now();
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    // Y pins the menu, which shows as this frame is drawn, as A is pressed: the menu's, as the trigger's would be.
    step.buttons(
        &press(|buttons| {
            buttons.y = true;
            buttons.a = true;
            buttons.a_held = true;
        }),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    senses.left_palm = Some(left_palm_up(0.0));
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(step.menu_was_shown);
    assert!(selects(&done).is_empty());
    // Held, then let go, it tells nothing, and the press is not kept for when the menu goes.
    step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    senses.left_palm = None;
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(selects(&done).is_empty());
}

#[test]
fn a_pointing_at_something_of_tomori_s_in_front_of_a_selectable_object_is_tomori_s() {
    use tomori_protocol::{Card, CardKind, Checks, Panel, Place};
    let now = Instant::now();
    let ahead_of_home = Ray::new(arrival::HOME, Vec3::NEG_Z);
    // Each thing stands nearer than the lamp, 8 m along the ray, which the ray also meets, so that A goes to what is
    // nearer first, as the frame before found it. The checklist's item is real pointing, ticked by A, which the
    // conversation takes ahead of what else of Tomori's is pointed at.
    type Setup = fn(&mut Conversation, &mut Step, &mut Senses, Instant);
    let rows: [(&str, Setup); 5] = [
        ("a panel", |conversation, _, _, _| {
            fixtures::show_projection(
                conversation,
                Vec::new(),
                vec![Panel::new(
                    "panel-1",
                    "Build status",
                    "Green.",
                    Place::Free {
                        pose: tomori_protocol::Pose::upright_facing(
                            (arrival::HOME + Vec3::new(0.0, 0.0, -2.0)).to_array(),
                            arrival::HOME.to_array(),
                        ),
                    },
                )],
            );
        }),
        ("a checklist item", |conversation, step, senses, now| {
            let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
            fixtures::show_checklist(conversation, ahead, arrival::HOME);
            grow(step, conversation, senses, now);
            *senses = ray_on_item(step, conversation, ahead, "A  untick").expect("the item");
        }),
        ("a card", |conversation, _, _, _| {
            let at = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
            let (_, rotation, _) =
                crate::recording::facing(at, arrival::HOME, 1.0).to_scale_rotation_translation();
            fixtures::show_cards(
                conversation,
                vec![Card {
                    id: "github://o/r/issues/1".into(),
                    number: 1,
                    kind: CardKind::Issue,
                    title: "A card".into(),
                    author: "ada".into(),
                    labels: Vec::new(),
                    checks: Checks::None,
                    age: 0,
                    placement: Some(tomori_protocol::Pose {
                        position: at.to_array(),
                        orientation: rotation.to_array(),
                    }),
                }],
            );
        }),
        ("a worker", |conversation, step, senses, now| {
            conversation.apply(space(|state| {
                state.workers = vec![tomori_protocol::Worker {
                    id: "ses_a".into(),
                    parent: None,
                    title: "Review".into(),
                    state: tomori_protocol::WorkerState::Running,
                }];
            }));
            let worker = step
                .update(senses, conversation, &mut felt(), now, FRAME)
                .stations[0]
                .position;
            senses.ray = Ray::new(arrival::HOME, Vec3::from_array(worker) - arrival::HOME);
        }),
        ("a panel carried", |conversation, step, senses, now| {
            fixtures::show_projection(
                conversation,
                Vec::new(),
                vec![Panel::new(
                    "panel-1",
                    "Build status",
                    "Green.",
                    Place::Free {
                        pose: tomori_protocol::Pose::upright_facing(
                            (arrival::HOME + Vec3::new(0.0, 0.0, -2.0)).to_array(),
                            arrival::HOME.to_array(),
                        ),
                    },
                )],
            );
            senses.grip = Some(1.0);
            grow(step, conversation, senses, now);
            assert!(step.projector.held().is_some());
        }),
    ];
    for (label, setup) in rows {
        let (mut conversation, done) = lamp_at(Default::default(), true, 8.0);
        let mut step = Step::default();
        let mut senses = Senses {
            ray: ahead_of_home,
            beam: true,
            ..senses()
        };
        grow(&mut step, &mut conversation, &senses, now);
        setup(&mut conversation, &mut step, &mut senses, now);
        grow(&mut step, &mut conversation, &senses, now);
        assert_eq!(step.hands.right.aim, precedence::Aim::Tomori, "{label}");
        conversation.take_actions();
        done.lock().unwrap().clear();
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        // The environment is told nothing; A ticks the item pointed at, and does nothing else on the rest.
        assert!(selects(&done).is_empty(), "{label}");
        let tick = [Action::Send(
            tomori_protocol::ClientMessage::CheckPanelItem {
                panel: "panel-1".into(),
                line: 2,
                checked: false,
                revision: Some(4),
            },
        )];
        let actions = conversation.take_actions();
        if label == "a checklist item" {
            assert_eq!(actions, tick, "{label}");
        } else {
            assert!(actions.is_empty(), "{label}: {actions:?}");
        }
    }
}

/// The sticks a [`Lamp`] is told after `buttons`, `focused` or not, and a frame as `senses` find it.
fn sticks(
    step: &mut Step,
    conversation: &mut Conversation,
    senses: &Senses,
    done: &Done,
    buttons: Buttons,
    focused: bool,
) -> (Vec2, Vec2) {
    step.buttons(&buttons, focused, conversation, &mut felt(), Instant::now());
    step.update(senses, conversation, &mut felt(), Instant::now(), FRAME);
    let last = done.lock().unwrap().last().cloned().unwrap();
    (last.2, last.3)
}

fn both(left: Vec2, right: Vec2) -> Buttons {
    press(|buttons| {
        buttons.left_stick = left;
        buttons.right_stick = right;
    })
}

#[test]
fn the_left_stick_reaches_the_environment_only_where_it_travels_nowhere_from_the_start_of_a_push() {
    use crate::locomotion::{Locomotion, Surface};
    let push = both(Vec2::X, Vec2::ZERO);
    let rest = Buttons::default();
    // In an environment without travel, it is the environment's, but not without focus.
    let (mut conversation, mut step, senses, done) = lit(Locomotion::None);
    let told = |step: &mut Step, conversation: &mut Conversation, buttons, focused| {
        sticks(step, conversation, &senses, &done, buttons, focused).0
    };
    assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::X);
    assert_eq!(told(&mut step, &mut conversation, push, false), Vec2::ZERO);
    // Where the environment lets the participant travel, it is travel's, until travel is turned off here, from the
    // next push: a push begun while it travelled stays travel's until it returns to the middle.
    let teleport = Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]);
    let (mut conversation, mut step, senses, done) = lit(teleport);
    let told = |step: &mut Step, conversation: &mut Conversation, buttons, focused| {
        sticks(step, conversation, &senses, &done, buttons, focused).0
    };
    assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::ZERO);
    conversation.toggle_travel();
    assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::ZERO);
    told(&mut step, &mut conversation, rest, true);
    assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::X);
    // And the other way: a push begun as the environment's stays its own as travel is turned on again.
    conversation.toggle_travel();
    assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::X);
    assert_eq!(step.travel_stick, Vec2::ZERO);
    told(&mut step, &mut conversation, rest, true);
    assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::ZERO);
    // Losing focus forgets whose the push was.
    step.blur(&mut conversation);
    assert_eq!(step.left_stick, Push::Free);
    assert_eq!(
        Push::Other.update(Vec2::NAN, || Push::Environment),
        Push::Free
    );
}

#[test]
fn the_right_stick_reaches_the_environment_only_where_nothing_else_takes_it() {
    let push = both(Vec2::ZERO, Vec2::NEG_Y);
    let rest = Buttons::default();
    // With nothing carried and the transcript closed, it is the environment's, whether or not its objects are
    // selectable, as the farmyard's chickens are not; but not without focus.
    for selectable in [true, false] {
        let (mut conversation, done) = lamp(Default::default(), selectable);
        let mut step = Step::default();
        let senses = Senses {
            ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
            ..senses()
        };
        grow(&mut step, &mut conversation, &senses, Instant::now());
        let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
        assert_eq!(told.1, Vec2::NEG_Y);
        let told = sticks(&mut step, &mut conversation, &senses, &done, push, false);
        assert_eq!(told.1, Vec2::ZERO);
    }
    // While the right wrist's transcript shows, a push is its own.
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    senses.right_palm = Some(palm_up(0.0));
    let now = Instant::now();
    wrist_after(&mut step, &mut conversation, &senses, now);
    assert!(step.wrist.state().active());
    let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
    assert_eq!(told.1, Vec2::ZERO);
    assert_eq!(step.right_stick, Push::Other);
    // While a panel is carried, a push is the panel's.
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
    fixtures::show_projection(
        &mut conversation,
        Vec::new(),
        vec![tomori_protocol::Panel::new(
            "panel-1",
            "Build status",
            "Green.",
            tomori_protocol::Place::Free {
                pose: tomori_protocol::Pose::upright_facing(
                    ahead.to_array(),
                    arrival::HOME.to_array(),
                ),
            },
        )],
    );
    senses.grip = Some(1.0);
    grow(&mut step, &mut conversation, &senses, now);
    assert!(step.projector.held().is_some());
    sticks(&mut step, &mut conversation, &senses, &done, rest, true);
    let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
    assert_eq!(told.1, Vec2::ZERO);
    assert_eq!(step.right_stick, Push::Carried);
    // A push is the environment's only if nothing else takes it as it leaves the middle, until it returns there.
    assert_eq!(
        Push::Environment.update(Vec2::Y, || Push::Carried),
        Push::Environment
    );
}

/// A code panel of `lines` lines floating 0.6 m ahead, nearer than the lamp, so that the ray points at it.
fn code_ahead(conversation: &mut Conversation, lines: u32) {
    use tomori_protocol::{Code, Panel, Place, Pose};
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
    let body: Vec<String> = (1..=lines)
        .map(|line| format!("let x{line} = {line};"))
        .collect();
    fixtures::show_projection(
        conversation,
        Vec::new(),
        vec![
            Panel::new(
                "panel-1",
                "rust",
                body.join("\n"),
                Place::Free {
                    pose: Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array()),
                },
            )
            .with_code(Code::new("rust", lines)),
        ],
    );
}

#[test]
fn pointing_at_a_panel_the_sticks_are_tomori_s_and_never_the_environment_s() {
    use crate::locomotion::Locomotion;
    let push = both(Vec2::X, Vec2::NEG_Y);
    let rest = Buttons::default();
    let now = Instant::now();
    // In an environment without travel, whose client side would take both sticks, a panel in front of its lamp
    // takes the right stick: one with more than it shows scrolls with it, and one that shows everything takes it to
    // do nothing. The left stick follows the left ray, which is on nothing, so it is the environment's.
    for (lines, scrolls) in [(200, true), (1, false)] {
        let (mut conversation, mut step, senses, done) = lit(Locomotion::None);
        code_ahead(&mut conversation, lines);
        grow(&mut step, &mut conversation, &senses, now);
        assert_eq!(step.hands.right.aim, precedence::Aim::Tomori);
        assert_eq!(step.projector.scrollable("panel-1"), scrolls);
        let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
        assert_eq!(told, (Vec2::X, Vec2::ZERO), "{lines}");
        assert_eq!(step.left_stick, Push::Environment);
        if scrolls {
            assert_eq!(step.right_stick, Push::Scrolled("panel-1".into()));
            step.update(&senses, &mut conversation, &mut felt(), now, 0.5);
            assert!(step.projector.scroll("panel-1") > 0);
        } else {
            assert_eq!(step.right_stick, Push::Other);
        }
        // A push begun on the panel stays Tomori's as the ray moves off it, until it returns to the middle.
        let away = Senses {
            ray: Ray::new(arrival::HOME, Vec3::Z),
            ..senses
        };
        let told = sticks(&mut step, &mut conversation, &away, &done, push, true);
        assert_eq!(told, (Vec2::X, Vec2::ZERO), "{lines}");
        // Back to the middle with the ray on nothing of Tomori's, nor of the environment's, the next push is the
        // environment's.
        sticks(&mut step, &mut conversation, &away, &done, rest, true);
        assert_eq!(step.hands.right.aim, precedence::Aim::Nothing);
        let told = sticks(&mut step, &mut conversation, &away, &done, push, true);
        assert_eq!(told, (Vec2::X, Vec2::NEG_Y), "{lines}");
        // Carried in the right hand, the panel takes the right stick before anything else, and the left stick,
        // whose hand carries nothing, is the environment's still.
        sticks(&mut step, &mut conversation, &senses, &done, rest, true);
        let gripped = Senses {
            grip: Some(1.0),
            ..senses
        };
        step.update(&gripped, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.held(), Some("panel-1"));
        let told = sticks(&mut step, &mut conversation, &gripped, &done, push, true);
        assert_eq!(told, (Vec2::X, Vec2::ZERO), "{lines}");
        assert_eq!(step.right_stick, Push::Carried);
        assert_eq!(step.left_stick, Push::Environment);
    }
}

#[test]
fn a_trigger_pull_on_a_panel_in_front_of_a_selectable_object_is_tomori_s() {
    let (mut conversation, mut step, mut senses, done) = lit(Default::default());
    let now = Instant::now();
    code_ahead(&mut conversation, 1);
    grow(&mut step, &mut conversation, &senses, now);
    assert_eq!(step.hands.right.aim, precedence::Aim::Tomori);
    done.lock().unwrap().clear();
    // The pull attracts the swarm, as on any panel, and selects nothing behind it.
    let mut attracting = Vec::new();
    for trigger in [0.3, 1.0, 1.0, 0.0] {
        senses.trigger = Some(trigger);
        attracting.push(
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .attracting,
        );
    }
    assert_eq!(attracting, [false; 4]);
    assert!(selects(&done).is_empty());
    // Nor does a pull begun on the panel select the lamp once the panel has gone from in front of it.
    senses.trigger = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    fixtures::show_projection(&mut conversation, Vec::new(), Vec::new());
    for _ in 0..4 {
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    }
    assert_eq!(step.hands.right.aim, precedence::Aim::Environment);
    assert!(selects(&done).is_empty());
    // Let go, the next pull on the lamp is the environment's.
    senses.trigger = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    senses.trigger = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(
        selects(&done),
        [crate::client_side::Select::Pressed("lamp".into())]
    );
}

#[test]
fn without_a_client_side_the_right_stick_pages_no_result_card_in_a_space() {
    // In a space whose environment has no client side, a push is the cards', as ever, which never page there.
    let mut conversation = joined();
    let mut step = Step::default();
    let senses = senses();
    let now = Instant::now();
    grow(&mut step, &mut conversation, &senses, now);
    assert!(step.client.is_none());
    for direction in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
        step.buttons(
            &both(Vec2::ZERO, direction),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        assert_eq!(step.right_stick, Push::Other);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!((frame.card, frame.content), (0, 0));
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
    }
}

#[test]
fn joining_sets_the_room_on_the_spawn_point_the_server_chose_and_leaving_sends_it_home() {
    let mut conversation = lobby(ConnectionRole::Owner);
    let mut step = Step::default();
    conversation.apply(space(|_| {}));
    // Until the server says, and in an environment that declares none, the room is home.
    assert_eq!(step.rig(&conversation), Rig::default());
    let camp = tomori_protocol::Pose::upright([3.0, 0.0, -5.0], 2.0);
    conversation.apply(tomori_client::Event::Message(
        tomori_protocol::ServerMessage::Spawn {
            space: "space-1".into(),
            environment: "sky".into(),
            at: camp,
        },
    ));
    // Sky allows no travel, so the room stays at the campsite, frame after frame.
    assert_eq!(step.rig(&conversation), Rig::arriving(&camp));
    let now = Instant::now();
    step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.rig(&conversation), Rig::arriving(&camp));
    conversation.apply(tomori_client::Event::Message(
        tomori_protocol::ServerMessage::LEFT,
    ));
    assert_eq!(step.rig(&conversation), Rig::default());
}

#[test]
fn travel_waits_for_the_space_s_scenery_to_grow_in() {
    use crate::locomotion::{Locomotion, Surface};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(
        crate::test_support::SKY,
        Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
    )]);
    let mut step = Step::default();
    let now = Instant::now();
    let senses = Senses {
        ray: Ray::new(arrival::HOME, Vec3::new(0.0, -1.0, -1.0)),
        ..senses()
    };
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    conversation.apply(space(|_| {}));
    step.buttons(
        &press(|buttons| buttons.left_stick = Vec2::Y),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    let growing = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_ne!(
        growing.scenery,
        arrival_motion::Scenery {
            environment: crate::test_support::SKY,
            reveal: None
        }
    );
    assert_eq!(growing.landing, None);
    let mut landing = None;
    for _ in 0..400 {
        landing = step
            .update(&senses, &mut conversation, &mut felt(), now, FRAME)
            .landing;
    }
    // A push held from before it grew in aims nothing: the stick comes back to the middle first.
    assert_eq!(landing, None);
    for stick in [Vec2::ZERO, Vec2::Y] {
        step.buttons(
            &press(|buttons| buttons.left_stick = stick),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        landing = step
            .update(&senses, &mut conversation, &mut felt(), now, FRAME)
            .landing;
    }
    assert!(landing.is_some(), "once it has grown in");
}

#[test]
fn pointing_at_a_more_sign_as_words_begin_attends_to_the_workers_it_stands_for() {
    use tomori_protocol::Resource;
    use tomori_protocol::cluster::{Anchor as Named, HOME, Layout};
    let mut conversation = lobby(ConnectionRole::Owner);
    // A home of little room, and more workers than it holds even compact, seated by the server.
    let home =
        tomori_protocol::Pose::at(crate::swarm::CENTER.to_array()).facing_towards([0.0, 1.6, 0.0]);
    let count = 16;
    conversation.apply(space(|state| {
        state.workers = (0..count)
            .map(|index| tomori_protocol::Worker {
                id: format!("ses_{index:02}"),
                parent: None,
                title: format!("Look at {index}"),
                state: tomori_protocol::WorkerState::Running,
            })
            .collect();
        state.anchors = tomori_protocol::Anchors::new()
            .with_workers(home)
            .with_cluster(Named::new(HOME, home).with_extent(4.2, 1.6, 2.0));
        state.clusters = vec![(0..count).fold(Layout::new(HOME, count), |layout, slot| {
            layout.with_seat(format!("ses_{slot:02}"), slot)
        })];
    }));
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ..senses()
    };
    let mut sign = None;
    for _ in 0..400 {
        let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        sign = output
            .stations
            .iter()
            .find(|station| station.more > 0)
            .cloned();
    }
    let sign = Vec3::from_array(sign.expect("a sign for those it does not hold").position);
    // From beneath it, where no other worker stands.
    senses.ray = Ray::new(sign - Vec3::Y * 0.8, Vec3::Y);
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    conversation.take_actions();
    conversation.talk(true);
    conversation.talk(false);
    let actions = conversation.take_actions();
    let Some(Action::BeginUtterance(utterance)) = actions.first() else {
        unreachable!("recording begins, not {actions:?}")
    };
    assert_eq!(utterance.attention, [Resource::gathered(HOME)]);
}

#[test]
fn aiming_a_teleport_takes_the_ray_from_workers_and_the_hand_menu_or_focus_cancels_it() {
    use crate::locomotion::{Locomotion, Surface};
    use tomori_protocol::Resource;
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(
        crate::test_support::SKY,
        Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
    )]);
    conversation.apply(space(|state| {
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
    }));
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ..senses()
    };
    // Travel waits for the space's scenery to grow in.
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    }
    let worker = Vec3::from_array(
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
            .stations[0]
            .position,
    );
    // At the top of the sphere that points at it, nearly level.
    senses.ray = Ray::new(arrival::HOME, worker + Vec3::Y * 0.28 - arrival::HOME);
    let attending = |conversation: &mut Conversation| {
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        utterance.attention.clone()
    };
    let stick = |step: &mut Step, conversation: &mut Conversation, stick: Vec2| {
        step.buttons(
            &press(|buttons| buttons.left_stick = stick),
            true,
            conversation,
            &mut felt(),
            now,
        );
    };
    conversation.take_actions();
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(attending(&mut conversation), [Resource::worker("ses_a")]);
    // Aiming, the worker is no longer pointed at. The ray at it meets the floor only beyond travel's reach, so it has nowhere to land, and is drawn plain and short of it.
    stick(&mut step, &mut conversation, Vec2::Y);
    let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let beam = output
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert_eq!(output.landing, None);
    assert!(!beam.hit);
    let short = senses.ray.unwrap().at(crate::pointing::SPACE_RAY_LENGTH);
    assert!(beam.end.distance(short) < 1e-4);
    assert!(attending(&mut conversation).is_empty());
    // Down at the floor, it lands; with the hand menu showing, the aim is dropped and releasing does nothing.
    senses.ray = Ray::new(arrival::HOME, Vec3::new(0.0, -1.0, -1.0));
    assert!(
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
            .landing
            .is_some()
    );
    senses.left_palm = Some(left_palm_up(0.0));
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let shown = step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
    assert_eq!(shown.landing, None);
    senses.left_palm = None;
    stick(&mut step, &mut conversation, Vec2::ZERO);
    step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
    assert_eq!(step.rig(&conversation), Rig::default());
    // Losing focus drops it too.
    stick(&mut step, &mut conversation, Vec2::Y);
    step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
    step.blur(&mut conversation);
    step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
    assert_eq!(step.rig(&conversation), Rig::default());
    // Unfocused buttons hold the stick in the middle.
    step.buttons(
        &press(|buttons| buttons.left_stick = Vec2::Y),
        false,
        &mut conversation,
        &mut felt(),
        now,
    );
    assert_eq!(step.travel_stick, Vec2::ZERO);
}

/// A main floor and racks 4.5 m below it, with a pad 2 m ahead of home and another on the racks.
fn storeys() -> crate::locomotion::Locomotion {
    use crate::locomotion::{Locomotion, Pad, Surface};
    Locomotion::teleport(vec![
        Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0)),
        Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::splat(40.0)),
    ])
    .with_pads([[
        Pad::at(Vec3::new(0.0, 0.0, -2.0)).named("Main floor"),
        Pad::at(Vec3::new(3.0, -4.5, -2.0)).named("Racks"),
    ]])
}

/// The senses of a head at home, with the trigger at `trigger` and the ray from the head towards `towards`, in the
/// room, both placed with the rig the step settles for `conversation`, as the platform places them.
fn on_pads(step: &mut Step, conversation: &Conversation, towards: Vec3, trigger: f32) -> Senses {
    let rig = step.rig(conversation).matrix();
    let head = rig * Mat4::from_translation(arrival::HOME);
    Senses {
        beam: true,
        head: Some(head),
        head_pose: Pose::from_transform(head),
        viewer: Some(head.w_axis.truncate()),
        ray: Ray::new(
            rig.transform_point3(arrival::HOME),
            rig.transform_vector3(towards - arrival::HOME),
        ),
        trigger: Some(trigger),
        ..senses()
    }
}

#[test]
fn a_pad_pointed_at_is_highlighted_and_labelled_and_the_trigger_or_a_takes_the_participant_to_its_partner()
 {
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(crate::test_support::SKY, storeys())]);
    conversation.apply(space(|_| {}));
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let pad = Vec3::new(0.0, 0.2, -2.0);
    // Before the scenery has grown in, no pad is drawn or pointed at.
    let senses = on_pads(&mut step, &conversation, pad, 0.0);
    let growing = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(growing.pads.is_empty());
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    }
    // Pointed at, it is highlighted, its label says where it goes, the ray ends on its column, and it ticks.
    let senses = on_pads(&mut step, &conversation, Vec3::new(0.0, 1.6, -5.0), 0.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(frame.pads.len(), 2);
    assert!(frame.pads.iter().all(|pad| !pad.highlighted));
    let later = now + Duration::from_secs(5);
    felt_on(&mut feedback, Side::Right, later);
    let senses = on_pads(&mut step, &conversation, pad, 0.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, later, FRAME);
    assert!(frame.pads[0].highlighted && !frame.pads[1].highlighted);
    assert_eq!(frame.pads[0].up, Some(false));
    let labels: Vec<String> = frame
        .projection
        .iter()
        .map(|sheet| sheet.canvas.as_deref().unwrap().plain())
        .collect();
    assert!(
        labels.iter().any(|label| label.starts_with("Racks ↓")),
        "{labels:?}"
    );
    let beam = frame
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert!(beam.hit);
    assert!((beam.end.z + 1.5).abs() < 1e-3, "{}", beam.end);
    assert!(
        !felt_on(&mut feedback, Side::Right, later).is_empty(),
        "it ticks"
    );
    earcons(&mut feedback);
    // The trigger, pulled on it, takes the participant down to the racks, blinking, without attracting the swarm.
    let senses = on_pads(&mut step, &conversation, pad, 1.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(!frame.attracting);
    assert_eq!(frame.blink, 1.0);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    let rig = step.rig(&conversation);
    assert_eq!(rig.floor(), -4.5);
    let head = rig.matrix().transform_point3(arrival::HOME);
    assert!(
        head.distance(Vec3::new(3.0, 1.6 - 4.5, -2.0)) < 1e-4,
        "{head}"
    );
    // Held there, the pull neither attracts nor takes them anywhere else; they stand on the racks' pad, disarmed.
    for _ in 0..60 {
        let senses = on_pads(&mut step, &conversation, Vec3::new(0.0, 1.6, -5.0), 1.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(!frame.attracting && frame.blink < 1.0);
    }
    assert_eq!(step.rig(&conversation), rig);
    // Pointing down at the pad stood on, A takes them back up, clicking once.
    let feet = Vec3::new(0.0, 0.0, -0.05);
    let senses = on_pads(&mut step, &conversation, feet, 0.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(frame.pads[1].highlighted, "{:?}", frame.pads);
    assert!(frame.projection.iter().any(|sheet| {
        sheet
            .canvas
            .as_deref()
            .unwrap()
            .plain()
            .starts_with("Main floor ↑")
    }));
    earcons(&mut feedback);
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(frame.blink, 1.0);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    let head = step
        .rig(&conversation)
        .matrix()
        .transform_point3(arrival::HOME);
    assert!(head.distance(Vec3::new(0.0, 1.6, -2.0)) < 1e-4, "{head}");
    // A pointing at nothing then chooses no pad.
    let senses = on_pads(&mut step, &conversation, Vec3::new(5.0, 1.6, -2.0), 0.0);
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let rig = step.rig(&conversation);
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.rig(&conversation), rig);
}

#[test]
fn a_target_pointed_at_shows_where_it_takes_the_participant_and_the_trigger_or_a_takes_them_there()
{
    use crate::locomotion::{Locomotion, Surface, Target};
    let profile = Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))])
        .with_targets([
            Target::object(Vec3::new(0.0, -0.6, -6.0), 0.8, 1.0)
                .named("Rock")
                .facing(Vec3::X),
            Target::object(Vec3::new(2.0, 0.0, -1.0), 0.3, 1.0)
                .arriving(Vec3::new(1.0, 0.0, -1.0))
                .named("Shore"),
        ]);
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(crate::test_support::SKY, profile)]);
    conversation.apply(space(|_| {}));
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let rock = Vec3::new(0.0, 0.2, -5.2);
    // Before the scenery has grown in, nothing is pointed at and no ring is drawn.
    let senses = on_pads(&mut step, &conversation, rock, 0.0);
    let growing = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(growing.landing, None);
    let away = on_pads(&mut step, &conversation, Vec3::new(5.0, 1.6, -2.0), 0.0);
    for _ in 0..400 {
        step.update(&away, &mut conversation, &mut feedback, now, FRAME);
    }
    // Pointed at, the ring stands on its top, where it takes them, the label names it, the ray ends on it and it
    // ticks; Tomori draws no pad for it.
    let later = now + Duration::from_secs(5);
    felt_on(&mut feedback, Side::Right, later);
    let frame = step.update(&senses, &mut conversation, &mut feedback, later, FRAME);
    assert!(frame.pads.is_empty());
    assert!(frame.landing.unwrap().distance(Vec3::new(0.0, 0.4, -6.0)) < 1e-5);
    let labels: Vec<String> = frame
        .projection
        .iter()
        .map(|sheet| sheet.canvas.as_deref().unwrap().plain())
        .collect();
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("Rock\n") && label.contains("Trigger  go")),
        "{labels:?}"
    );
    let beam = frame
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert!(beam.hit && (beam.end.z + 5.2).abs() < 1e-3, "{}", beam.end);
    assert!(
        !felt_on(&mut feedback, Side::Right, later).is_empty(),
        "it ticks"
    );
    earcons(&mut feedback);
    // The trigger takes them onto its top, blinking and clicking, facing the way it says, without attracting.
    let senses = on_pads(&mut step, &conversation, rock, 1.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(!frame.attracting);
    assert_eq!(frame.blink, 1.0);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    let rig = step.rig(&conversation);
    assert!((rig.floor() - 0.4).abs() < 1e-5);
    let head = rig.matrix().transform_point3(arrival::HOME);
    assert!(head.distance(Vec3::new(0.0, 2.0, -6.0)) < 1e-4, "{head}");
    let facing = rig.matrix().transform_vector3(Vec3::NEG_Z);
    assert!(facing.distance(Vec3::X) < 1e-4, "{facing}");
    // Standing on it, pointing down at it points at nothing: it would take them nowhere.
    for _ in 0..3 {
        let senses = on_pads(&mut step, &conversation, Vec3::new(0.0, 0.0, -0.3), 0.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(frame.landing, None);
    }
    // A, pointing at the shore's stone, takes them back to the floor beside it, facing as they faced.
    let stone = rig
        .matrix()
        .inverse()
        .transform_point3(Vec3::new(2.0, 0.5, -1.0));
    let senses = on_pads(&mut step, &conversation, stone, 0.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(frame.landing, Some(Vec3::new(1.0, 0.0, -1.0)));
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(frame.blink, 1.0);
    let rig = step.rig(&conversation);
    assert_eq!(rig.floor(), 0.0);
    let head = rig.matrix().transform_point3(arrival::HOME);
    assert!(head.distance(Vec3::new(1.0, 1.6, -1.0)) < 1e-4, "{head}");
    assert!(
        rig.matrix()
            .transform_vector3(Vec3::NEG_Z)
            .distance(Vec3::X)
            < 1e-4
    );
}

#[test]
fn a_pad_gives_way_to_anything_nearer_and_to_aiming_and_is_not_there_without_travel() {
    use tomori_protocol::{Panel, Place};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(crate::test_support::SKY, storeys())]);
    // A panel floating between home and the pad.
    let between = arrival::HOME + Vec3::new(0.0, -0.6, -1.0);
    fixtures::show_projection(
        &mut conversation,
        Vec::new(),
        vec![Panel::new(
            "panel-1",
            "Build status",
            "Green.",
            Place::Free {
                pose: tomori_protocol::Pose::upright_facing(
                    between.to_array(),
                    arrival::HOME.to_array(),
                ),
            },
        )],
    );
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    // Through the panel's middle to the pad's column behind it.
    let pad = Vec3::new(0.0, 0.4, -2.0);
    let senses = on_pads(&mut step, &conversation, pad, 0.0);
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    }
    // Behind the panel, the pad is not pointed at, and the trigger is the panel's, attracting the swarm as ever.
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(frame.pads.iter().all(|pad| !pad.highlighted));
    assert_eq!(step.hands.right.aim, precedence::Aim::Tomori);
    let senses = Senses {
        attractor: Some(pad),
        ..on_pads(&mut step, &conversation, pad, 1.0)
    };
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(!frame.attracting);
    assert_eq!(step.rig(&conversation), Rig::default());
    // Aiming a teleport, the ray serves only it.
    // From the side, so that the panel is not in the way.
    let from = Vec3::new(2.0, 1.3, 0.0);
    let senses = Senses {
        ray: Ray::new(from, Vec3::new(0.0, 0.3, -2.0) - from),
        ..on_pads(&mut step, &conversation, pad, 0.0)
    };
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.hands.right.aim, precedence::Aim::Pad);
    step.buttons(
        &press(|buttons| buttons.left_stick = Vec2::Y),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(frame.landing.is_some());
    assert!(frame.pads.iter().all(|pad| !pad.highlighted));
    step.cancel_travel();
    // With travel turned off here, no pad is drawn or pointed at.
    conversation.toggle_travel();
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(frame.pads.is_empty());
    assert_ne!(step.hands.right.aim, precedence::Aim::Pad);
}

/// A profile with a pad on a disc 1 m beyond where `ray` meets something `nearer`, with its column 0.3 m up the ray
/// there, so that the ray would point at it but for what is nearer, on a disc too small for the ray to cross.
fn pad_behind(ray: &Ray, nearer: f32) -> crate::locomotion::Locomotion {
    use crate::locomotion::{Locomotion, Pad, Surface};
    let at = ray.at(nearer + 1.0);
    let base = Vec3::new(at.x, at.y - 0.3, at.z);
    // Its partner stands a little further along the ray, so that it is no nearer.
    let along = Vec3::new(ray.direction.x, 0.0, ray.direction.z).normalize_or_zero();
    Locomotion::teleport(vec![Surface::disc(base, 0.9)])
        .with_pads([[Pad::at(base), Pad::at(base + along * 0.4)]])
}

/// With `ray` meeting something `nearer`, which makes the aim `aim`, the pad behind it is not pointed at, though it
/// would be alone.
fn behind_it_no_pad(
    conversation: &mut Conversation,
    step: &mut Step,
    ray: Ray,
    nearer: f32,
    aim: precedence::Aim,
) {
    let profile = pad_behind(&ray, nearer);
    let (_, distance, _) = crate::pads::pick(&profile, &ray).expect("alone, it is pointed at");
    assert!(distance > nearer, "{distance} {nearer}");
    conversation.set_locomotion(vec![(crate::test_support::SKY, profile)]);
    let senses = Senses {
        beam: true,
        ray: Some(ray),
        ..senses()
    };
    let frame = step.update(&senses, conversation, &mut felt(), Instant::now(), FRAME);
    assert!(
        frame.pads.iter().all(|pad| !pad.highlighted),
        "{aim:?} {nearer}"
    );
    assert_eq!(step.hands.right.aim, aim);
}

#[test]
fn a_pad_is_not_pointed_at_behind_a_card_a_worker_an_environment_s_object_or_a_wall() {
    use precedence::Aim;
    use tomori_protocol::{Pose, Wall, WallKind};
    let now = Instant::now();
    let ahead = senses();
    let grown = |conversation: &mut Conversation| {
        let mut step = Step::default();
        grow(&mut step, conversation, &ahead, now);
        step
    };
    // A card, wherever it stands, which a ray through its middle meets.
    let mut conversation = lobby(ConnectionRole::Owner);
    fixtures::show_cards(
        &mut conversation,
        vec![tomori_protocol::Card {
            id: "github://o/r/issues/1".into(),
            number: 1,
            kind: tomori_protocol::CardKind::Issue,
            title: "A card".into(),
            author: "ada".into(),
            labels: Vec::new(),
            checks: tomori_protocol::Checks::Passing,
            age: 0,
            placement: None,
        }],
    );
    let mut step = grown(&mut conversation);
    let frame = step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
    let card = frame.cards[0].model.expect("the card").w_axis.truncate();
    let ray = Ray::new(arrival::HOME, card - arrival::HOME).unwrap();
    behind_it_no_pad(
        &mut conversation,
        &mut step,
        ray,
        card.distance(arrival::HOME),
        Aim::Tomori,
    );
    // A worker, met at the top of the sphere that points at it.
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| {
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
    }));
    let mut step = grown(&mut conversation);
    let worker = Vec3::from_array(
        step.update(&ahead, &mut conversation, &mut felt(), now, FRAME)
            .stations[0]
            .position,
    ) + Vec3::Y * 0.28;
    let ray = Ray::new(arrival::HOME, worker - arrival::HOME).unwrap();
    behind_it_no_pad(
        &mut conversation,
        &mut step,
        ray,
        worker.distance(arrival::HOME) - 0.3,
        Aim::Tomori,
    );
    // An environment's wall's board, where the participant may take hold of walls.
    let wall = Wall::new(
        "environment-0",
        WallKind::Environment,
        Pose::upright_facing([1.2, 1.5, -2.5], arrival::HOME.to_array()),
    );
    let board = Vec3::new(1.2, 1.5 + 0.42, -2.5);
    let mut conversation = lobby(ConnectionRole::Owner);
    fixtures::show_editable_walls(&mut conversation, vec![wall], Vec::new());
    let mut step = grown(&mut conversation);
    let ray = Ray::new(arrival::HOME, board - arrival::HOME).unwrap();
    behind_it_no_pad(
        &mut conversation,
        &mut step,
        ray,
        board.distance(arrival::HOME),
        Aim::Tomori,
    );
    // An environment's object, selectable or not, a metre away.
    for selectable in [true, false] {
        let (mut conversation, _) = lamp(Default::default(), selectable);
        let mut step = grown(&mut conversation);
        let ray = Ray::new(arrival::HOME, Vec3::new(0.0, -0.2, -1.0)).unwrap();
        behind_it_no_pad(&mut conversation, &mut step, ray, 1.0, Aim::Environment);
    }
}

/// A profile with a pad 2 m behind home, whose partner is on the racks below.
fn pad_behind_home() -> crate::locomotion::Locomotion {
    use crate::locomotion::{Locomotion, Pad, Surface};
    Locomotion::teleport(vec![
        Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0)),
        Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::splat(40.0)),
    ])
    .with_pads([[
        Pad::at(Vec3::new(0.0, 0.0, 2.0)),
        Pad::at(Vec3::new(3.0, -4.5, 2.0)),
    ]])
}

#[test]
fn a_pull_begun_elsewhere_that_slides_onto_a_pad_chooses_nothing_and_a_still_goes_while_the_environment_holds_a_select()
 {
    use crate::client_side::Select;
    let now = Instant::now();
    let at_pad = Ray::new(arrival::HOME, Vec3::new(0.0, 0.3, 2.0) - arrival::HOME);
    let at_nothing = Ray::new(arrival::HOME, Vec3::X);
    let aimed = |ray: Option<Ray>, trigger: f32| Senses {
        ray,
        trigger: Some(trigger),
        ..senses()
    };
    // A pull that begins on nothing attracts the swarm, and sliding onto the pad then, even pulled fully, chooses it
    // no more than letting go does.
    let (mut conversation, mut step, _, _) = lit(pad_behind_home());
    step.update(
        &aimed(at_nothing, 0.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    let frame = step.update(
        &aimed(at_nothing, 0.3),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert!(!frame.attracting);
    for trigger in [0.3, 1.0, 1.0, 0.0] {
        let frame = step.update(
            &aimed(at_pad, trigger),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        assert_eq!(
            frame.pads.iter().filter(|pad| pad.highlighted).count(),
            usize::from(trigger > 0.0 || frame.pads[0].highlighted)
        );
    }
    assert_eq!(step.rig(&conversation), Rig::default());
    // The environment's select, held from the lamp, and A on a pad slid onto: A goes, the select is pressed and let
    // go once, and the pull chooses no pad.
    let (mut conversation, mut step, _, done) = lit(pad_behind_home());
    let ahead = Ray::new(arrival::HOME, Vec3::NEG_Z);
    step.update(
        &aimed(ahead, 0.7),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    let frame = step.update(
        &aimed(at_pad, 0.7),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert!(frame.pads[0].highlighted && !frame.attracting);
    assert_eq!(step.rig(&conversation), Rig::default());
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.update(
        &aimed(at_pad, 0.7),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(step.rig(&conversation).floor(), -4.5);
    step.update(
        &aimed(at_pad, 0.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(
        selects(&done),
        [
            Select::Pressed("lamp".into()),
            Select::Released("lamp".into())
        ]
    );
}

#[test]
fn a_pull_on_a_row_or_a_pad_while_a_holds_the_environment_s_select_is_nobody_s() {
    use crate::client_side::Select;
    let now = Instant::now();
    // A row: behind home, where the lamp, which picks only rays ahead, is not.
    let (mut conversation, mut step, ahead, done) = lit(Default::default());
    let behind = arrival::HOME + Vec3::new(0.0, 0.0, 2.0);
    show_queue(&mut conversation, behind, arrival::HOME);
    let row = queue_rows(&mut step, &mut conversation, behind)[1];
    let pulled = Senses {
        trigger: Some(1.0),
        ..row
    };
    step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    conversation.take_actions();
    step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    for _ in 0..2 {
        step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    }
    // A let go while the pull lasts, the pull is still nobody's.
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Released("lamp".into())]);
    assert!(conversation.take_actions().is_empty());
    // Let go, and pulled again, it chooses the row.
    step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(
        conversation.take_actions(),
        [Action::Send(
            tomori_protocol::ClientMessage::ChoosePanelRow {
                panel: "panel-2".into(),
                row: "pod112".into(),
                revision: Some(6),
                key: None,
            }
        )]
    );
    // A pad: the same.
    let (mut conversation, mut step, ahead, done) = lit(pad_behind_home());
    let at_pad = Senses {
        ray: Ray::new(arrival::HOME, Vec3::new(0.0, 0.3, 2.0) - arrival::HOME),
        ..ahead
    };
    let pulled = Senses {
        trigger: Some(1.0),
        ..at_pad
    };
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    step.update(&at_pad, &mut conversation, &mut felt(), now, FRAME);
    step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.rig(&conversation), Rig::default());
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.update(&at_pad, &mut conversation, &mut felt(), now, FRAME);
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.rig(&conversation).floor(), -4.5);
}

#[test]
fn a_panel_behind_a_pad_is_not_pulled_off_by_the_grip() {
    use crate::locomotion::{Locomotion, Pad, Surface};
    use tomori_protocol::{Panel, Place};
    let now = Instant::now();
    // A low ray along the floor, to the right of the Tomori wall, which meets the pad's column 2 m ahead and a panel
    // 1 m behind it.
    let behind = Vec3::new(2.0, 0.4, -3.0);
    let ray = Ray::new(Vec3::new(2.0, 0.5, 0.0), Vec3::new(0.0, -0.05, -1.5));
    let held = |grip: f32| Senses {
        ray,
        grip: Some(grip),
        ..senses()
    };
    for pad in [false, true] {
        let mut conversation = lobby(ConnectionRole::Owner);
        fixtures::show_projection(
            &mut conversation,
            Vec::new(),
            vec![Panel::new(
                "panel-1",
                "Behind",
                "Hidden.",
                Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        behind.to_array(),
                        Vec3::new(2.0, 0.4, 0.0).to_array(),
                    ),
                },
            )],
        );
        let floor = Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0));
        let racks = Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::splat(40.0));
        let mut profile = Locomotion::teleport(vec![floor, racks]);
        if pad {
            profile = profile.with_pads([[
                Pad::at(Vec3::new(2.0, 0.0, -2.0)),
                Pad::at(Vec3::new(3.0, -4.5, -2.0)),
            ]]);
        }
        conversation.set_locomotion(vec![(crate::test_support::SKY, profile)]);
        let mut step = Step::default();
        let mut feedback = felt();
        grow(&mut step, &mut conversation, &held(0.0), now);
        let frame = step.update(&held(0.0), &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(frame.pads.first().is_some_and(|pad| pad.highlighted), pad);
        step.update(&held(1.0), &mut conversation, &mut feedback, now, FRAME);
        // Without the pad, the grip pulls the panel off; with it nearer, it takes nothing.
        assert_eq!(step.projector.held(), (!pad).then_some("panel-1"), "{pad}");
    }
}

#[test]
fn a_panel_carried_travels_in_the_hand_through_a_teleport_a_turn_and_a_backstep() {
    use crate::locomotion::{Locomotion, Surface};
    use tomori_protocol::{ClientMessage, Panel, Place};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(
        crate::test_support::SKY,
        Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
    )]);
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    fixtures::show_projection(
        &mut conversation,
        Vec::new(),
        vec![Panel::new(
            "panel-1",
            "Build status",
            "Green.",
            Place::Free {
                pose: tomori_protocol::Pose::upright_facing(
                    ahead.to_array(),
                    arrival::HOME.to_array(),
                ),
            },
        )],
    );
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    // The platform places the frame's poses with the rig, as it was settled before the frame.
    let placed = |step: &mut Step, conversation: &Conversation, ray: Vec3, grip: f32| {
        let rig = step.rig(conversation).matrix();
        let head = rig * Mat4::from_translation(arrival::HOME);
        Senses {
            beam: true,
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(head.w_axis.truncate()),
            ray: Ray::new(
                rig.transform_point3(arrival::HOME),
                rig.transform_vector3(ray),
            ),
            grip: Some(grip),
            ..senses()
        }
    };
    let stick = |step: &mut Step, conversation: &mut Conversation, stick: Vec2| {
        step.buttons(
            &press(|buttons| buttons.left_stick = stick),
            true,
            conversation,
            &mut felt(),
            now,
        );
    };
    let carried = |frame: &Frame<'_>| {
        frame
            .projection
            .iter()
            .find(|sheet| {
                sheet
                    .canvas
                    .as_deref()
                    .unwrap()
                    .plain()
                    .starts_with("Build")
            })
            .map(|sheet| sheet.model.unwrap().w_axis.truncate())
            .unwrap()
    };
    // Once the scenery has grown in, the panel is pulled off.
    for _ in 0..400 {
        let senses = placed(&mut step, &conversation, -Vec3::Z, 0.0);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    }
    let senses = placed(&mut step, &conversation, -Vec3::Z, 1.0);
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.projector.held(), Some("panel-1"));
    // Aimed at the floor, the teleport aims, and the panel stays in the hand.
    stick(&mut step, &mut conversation, Vec2::Y);
    let down = Vec3::new(0.0, -0.5, -1.0);
    let senses = placed(&mut step, &conversation, down, 1.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(frame.landing.is_some());
    let beam = frame
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert!(beam.hit && beam.end.distance(frame.landing.unwrap()) < 1e-4);
    assert!(carried(&frame).distance(senses.ray.unwrap().at(2.0)) < 1e-3);
    assert_eq!(step.projector.held(), Some("panel-1"));
    // Released, it lands, and the panel comes along: next frame it is at the same place in the hand.
    stick(&mut step, &mut conversation, Vec2::ZERO);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(frame.blink, 1.0);
    assert_ne!(step.rig(&conversation), Rig::default());
    let senses = placed(&mut step, &conversation, -Vec3::Z, 1.0);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert!(carried(&frame).distance(senses.ray.unwrap().at(2.0)) < 1e-3);
    assert!(carried(&frame).distance(ahead) > 1.0, "carried along");
    // Turned and stepped back, it stays in the hand too.
    for push in [Vec2::X, Vec2::NEG_Y] {
        let before = step.rig(&conversation);
        stick(&mut step, &mut conversation, push);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        stick(&mut step, &mut conversation, Vec2::ZERO);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        // A backstep darkens the view as a teleport does; a turn leaves it coming back.
        assert_eq!(frame.blink == 1.0, push == Vec2::NEG_Y, "{push}");
        assert_ne!(step.rig(&conversation), before, "{push}");
        let senses = placed(&mut step, &conversation, -Vec3::Z, 1.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(carried(&frame).distance(senses.ray.unwrap().at(2.0)) < 1e-3);
        assert_eq!(step.projector.held(), Some("panel-1"));
    }
    assert!(conversation.take_actions().is_empty(), "nothing put down");
    // Let go, it floats where the participant carried it to, for everyone.
    let senses = placed(&mut step, &conversation, -Vec3::Z, 0.0);
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let actions = conversation.take_actions();
    let [
        Action::Send(ClientMessage::PlacePanel {
            pose: Some(pose), ..
        }),
    ] = actions.as_slice()
    else {
        unreachable!("the panel is placed, not {actions:?}")
    };
    let left = Vec3::from_array(pose.position);
    assert!(left.distance(senses.ray.unwrap().at(2.0)) < 1e-3);
    assert!(left.distance(ahead) > 1.0);
}

use crate::sides::Side as Hand;

/// Where a hand's ray starts, in the participant's room, and which way it points.
type Aim = (Vec3, Vec3);

/// In a space in Sky whose environment teleports onto a floor 40 m across, or does not, showing `panels`, grown in.
fn travelling(
    locomotion: crate::locomotion::Locomotion,
    panels: Vec<tomori_protocol::Panel>,
) -> (Conversation, Step) {
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(crate::test_support::SKY, locomotion)]);
    fixtures::show_projection(&mut conversation, Vec::new(), panels);
    let mut step = Step::default();
    for _ in 0..400 {
        let senses = rays_placed(&mut step, &conversation, None, None, [0.0; 2]);
        step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            FRAME,
        );
    }
    (conversation, step)
}

fn floor() -> crate::locomotion::Locomotion {
    use crate::locomotion::{Locomotion, Surface};
    Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))])
}

/// The senses of a participant at home in their room with the rays `right` and `left` and the grips, right then
/// left, as the platform places them with the rig, the rays drawn as the headset draws them.
fn rays_placed(
    step: &mut Step,
    conversation: &Conversation,
    right: Option<Aim>,
    left: Option<Aim>,
    grips: [f32; 2],
) -> Senses {
    let rig = step.rig(conversation).matrix();
    let head = rig * Mat4::from_translation(arrival::HOME);
    let ray = |aim: Option<Aim>| {
        aim.and_then(|(origin, direction)| {
            Ray::new(
                rig.transform_point3(origin),
                rig.transform_vector3(direction),
            )
        })
    };
    Senses {
        beam: true,
        head: Some(head),
        head_pose: Pose::from_transform(head),
        viewer: Some(head.w_axis.truncate()),
        ray: ray(right),
        left_ray: ray(left),
        grip: Some(grips[0]),
        left_grip: Some(grips[1]),
        left_trigger: Some(0.0),
        ..senses()
    }
}

/// Which way the rig turns the room's ahead, along the floor.
fn rig_ahead(step: &mut Step, conversation: &Conversation) -> Vec3 {
    step.rig(conversation)
        .matrix()
        .transform_vector3(Vec3::NEG_Z)
}

/// What a frame drew that the tests of the sticks read.
struct Pushed {
    landing: Option<Vec3>,
    blink: f32,
    arrival: Option<Arrival>,
    projection: Vec<controller_ui::Sheet>,
}

/// Push the sticks, left then right, and step a frame with `senses`.
fn push_sticks(
    step: &mut Step,
    conversation: &mut Conversation,
    senses: &Senses,
    left: Vec2,
    right: Vec2,
) -> Pushed {
    step.buttons(
        &both(left, right),
        true,
        conversation,
        &mut felt(),
        Instant::now(),
    );
    let frame = step.update(senses, conversation, &mut felt(), Instant::now(), FRAME);
    Pushed {
        landing: frame.landing,
        blink: frame.blink,
        arrival: frame.arrival,
        projection: frame.projection,
    }
}

/// Whose `side`'s stick's push is.
fn stick_of(step: &Step, side: Hand) -> &Push {
    match side {
        Hand::Left => &step.left_stick,
        Hand::Right => &step.right_stick,
    }
}

/// Facing −Z after `turns` snap turns to the right, along the floor.
fn turned(turns: f32) -> Vec3 {
    let angle = turns * crate::locomotion::SNAP_TURN;
    Vec3::new(angle.sin(), 0.0, -angle.cos())
}

#[test]
fn either_stick_turns_30_degrees_once_a_push_where_its_ray_is_on_nothing() {
    let ahead: Aim = (arrival::HOME + Vec3::new(0.2, -0.4, -0.3), Vec3::NEG_Z);
    for side in Hand::BOTH {
        let (mut conversation, mut step) = travelling(floor(), Vec::new());
        let push = |stick: Vec2| match side {
            Hand::Left => (stick, Vec2::ZERO),
            Hand::Right => (Vec2::ZERO, stick),
        };
        let sense = |step: &mut Step, conversation: &Conversation| {
            rays_placed(step, conversation, Some(ahead), Some(ahead), [0.0; 2])
        };
        let senses = sense(&mut step, &conversation);
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::ZERO,
            Vec2::ZERO,
        );
        assert_eq!(step.hands[side].aim, precedence::Aim::Nothing, "{side:?}");
        // Pushed right and held, it turns 30° to the right once, about the head.
        for _ in 0..5 {
            let senses = sense(&mut step, &conversation);
            let (left, right) = push(Vec2::X);
            push_sticks(&mut step, &mut conversation, &senses, left, right);
        }
        assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
        let head = step
            .rig(&conversation)
            .matrix()
            .transform_point3(arrival::HOME);
        assert!(
            head.distance(arrival::HOME) < 1e-4,
            "{side:?}: about the head"
        );
        // Rolled forward and back again within the push, it turns no more and aims nothing.
        for stick in [Vec2::new(0.5, 0.9), Vec2::new(0.9, -0.9), Vec2::X] {
            let senses = sense(&mut step, &conversation);
            let (left, right) = push(stick);
            let frame = push_sticks(&mut step, &mut conversation, &senses, left, right);
            assert_eq!(frame.landing, None);
        }
        assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
        // Back to the middle, the next push turns again, and a push left turns back.
        for stick in [Vec2::ZERO, Vec2::X, Vec2::ZERO, Vec2::NEG_X, Vec2::ZERO] {
            let senses = sense(&mut step, &conversation);
            let (left, right) = push(stick);
            push_sticks(&mut step, &mut conversation, &senses, left, right);
        }
        assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
        // A push mostly forward, drifting a little sideways as it passes the threshold, aims rather than turns.
        for stick in [Vec2::new(0.3, 0.8), Vec2::new(0.6, 0.75)] {
            let senses = sense(&mut step, &conversation);
            let (left, right) = push(stick);
            push_sticks(&mut step, &mut conversation, &senses, left, right);
        }
        assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
        assert_eq!(step.travel_right, side == Hand::Right);
    }
}

#[test]
fn the_right_stick_aims_a_teleport_along_the_right_ray_and_lands_as_it_returns() {
    let (mut conversation, mut step) = travelling(floor(), Vec::new());
    // The right ray points down at the floor a metre and a half ahead; the left, level, at nothing.
    let right: Aim = (
        arrival::HOME + Vec3::new(0.2, -0.4, -0.3),
        Vec3::new(0.0, -1.2, -1.2),
    );
    let left: Aim = (arrival::HOME + Vec3::new(-0.2, -0.4, -0.3), Vec3::NEG_Z);
    let senses = rays_placed(&mut step, &conversation, Some(right), Some(left), [0.0; 2]);
    push_sticks(
        &mut step,
        &mut conversation,
        &senses,
        Vec2::ZERO,
        Vec2::ZERO,
    );
    let frame = push_sticks(&mut step, &mut conversation, &senses, Vec2::ZERO, Vec2::Y);
    let landing = frame.landing.expect("somewhere to land");
    assert!(
        landing.distance(Vec3::new(0.2, 0.0, -1.5)) < 1e-4,
        "{landing}"
    );
    // The right ray shows the landing; the left goes on pointing.
    let arrival = frame.arrival.unwrap();
    let beam = arrival.ray.unwrap();
    assert!(beam.hit && beam.end.distance(landing) < 1e-4);
    assert!(arrival.left_ray.is_none_or(|beam| !beam.hit));
    // Meanwhile a push of the left stick travels nowhere, and is nobody's until it returns.
    let frame = push_sticks(&mut step, &mut conversation, &senses, Vec2::X, Vec2::Y);
    assert_eq!(step.left_stick, Push::Other);
    assert!(frame.landing.is_some());
    assert!(rig_ahead(&mut step, &conversation).distance(Vec3::NEG_Z) < 1e-4);
    // Returned to the middle, it lands there, the head over the landing.
    let frame = push_sticks(
        &mut step,
        &mut conversation,
        &senses,
        Vec2::ZERO,
        Vec2::ZERO,
    );
    assert_eq!(frame.blink, 1.0);
    let head = step
        .rig(&conversation)
        .matrix()
        .transform_point3(arrival::HOME);
    assert!(head.distance(Vec3::new(0.2, 1.6, -1.5)) < 1e-4, "{head}");
    assert!(!step.travel_right);
    // A right stick lost while it aims lands nowhere.
    let senses = rays_placed(&mut step, &conversation, Some(right), Some(left), [0.0; 2]);
    push_sticks(&mut step, &mut conversation, &senses, Vec2::ZERO, Vec2::Y);
    step.cancel_travel_of(Hand::Left);
    assert!(
        step.travel_right,
        "the left stick's loss leaves the right's aim"
    );
    step.cancel_travel_of(Hand::Right);
    push_sticks(
        &mut step,
        &mut conversation,
        &senses,
        Vec2::ZERO,
        Vec2::ZERO,
    );
    let after = step
        .rig(&conversation)
        .matrix()
        .transform_point3(arrival::HOME);
    assert!(after.distance(head) < 1e-4, "{after}");
    // Where the environment allows no travel, or travel is turned off here, the right stick neither turns nor aims.
    for off in [false, true] {
        let (mut conversation, mut step) = if off {
            let (mut conversation, step) = travelling(floor(), Vec::new());
            conversation.toggle_travel();
            (conversation, step)
        } else {
            travelling(crate::locomotion::Locomotion::None, Vec::new())
        };
        let senses = rays_placed(&mut step, &conversation, Some(right), Some(left), [0.0; 2]);
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::ZERO,
            Vec2::ZERO,
        );
        for stick in [Vec2::X, Vec2::ZERO, Vec2::Y] {
            let frame = push_sticks(&mut step, &mut conversation, &senses, Vec2::ZERO, stick);
            assert_eq!(frame.landing, None, "{off}");
        }
        assert_eq!(step.right_stick, Push::Other, "{off}");
        assert_eq!(step.rig(&conversation), Rig::default(), "{off}");
    }
}

/// A code panel of 200 lines, more than it shows, floating at `at`, facing home.
fn long_panel(at: Vec3) -> tomori_protocol::Panel {
    use tomori_protocol::{Code, Panel, Place, Pose};
    let body: Vec<String> = (1..=200)
        .map(|line| format!("let x{line} = {line};"))
        .collect();
    Panel::new(
        "panel-1",
        "rust",
        body.join("\n"),
        Place::Free {
            pose: Pose::upright_facing(at.to_array(), arrival::HOME.with_y(at.y).to_array()),
        },
    )
    .with_code(Code::new("rust", 200))
}

#[test]
fn a_push_begun_on_a_panel_with_more_than_it_shows_scrolls_it_with_either_stick_as_the_ray_leaves()
{
    let at = arrival::HOME + Vec3::new(0.0, 0.0, -0.8);
    let on: Aim = (
        arrival::HOME + Vec3::new(0.0, -0.3, -0.2),
        Vec3::new(0.0, 0.3, -0.6),
    );
    let off: Aim = (
        arrival::HOME + Vec3::new(0.0, -0.3, -0.2),
        Vec3::new(1.0, 0.0, -0.2),
    );
    for side in Hand::BOTH {
        let (mut conversation, mut step) = travelling(floor(), vec![long_panel(at)]);
        let push = |stick: Vec2| match side {
            Hand::Left => (stick, Vec2::ZERO),
            Hand::Right => (Vec2::ZERO, stick),
        };
        let aims = |aim: Aim| match side {
            Hand::Left => (None, Some(aim)),
            Hand::Right => (Some(aim), None),
        };
        let (right, left) = aims(on);
        let senses = rays_placed(&mut step, &conversation, right, left, [0.0; 2]);
        for _ in 0..3 {
            push_sticks(
                &mut step,
                &mut conversation,
                &senses,
                Vec2::ZERO,
                Vec2::ZERO,
            );
        }
        assert_eq!(
            step.hands[side].panel.as_deref(),
            Some("panel-1"),
            "{side:?}"
        );
        assert!(step.hands[side].full, "{side:?}");
        // Pulled back, it scrolls the panel down, and neither steps back nor turns as it rolls sideways, even once
        // the ray has left the panel.
        let (left_stick, right_stick) = push(Vec2::NEG_Y);
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            left_stick,
            right_stick,
        );
        assert_eq!(
            stick_of(&step, side),
            &Push::Scrolled("panel-1".into()),
            "{side:?}"
        );
        let (right, left) = aims(off);
        let away = rays_placed(&mut step, &conversation, right, left, [0.0; 2]);
        let rolled = [Vec2::new(0.9, -0.3), Vec2::NEG_X];
        for stick in std::iter::repeat_n(Vec2::NEG_Y, 30).chain(rolled) {
            let (left_stick, right_stick) = push(stick);
            let frame = push_sticks(&mut step, &mut conversation, &away, left_stick, right_stick);
            assert_eq!(frame.landing, None);
        }
        assert!(step.projector.scroll("panel-1") > 0, "{side:?}");
        let frame = push_sticks(&mut step, &mut conversation, &away, Vec2::ZERO, Vec2::ZERO);
        assert_eq!(frame.blink, 0.0, "{side:?}: no backstep");
        assert_eq!(step.rig(&conversation), Rig::default(), "{side:?}");
        // With the ray off the panel, the next push turns.
        push_sticks(&mut step, &mut conversation, &away, Vec2::ZERO, Vec2::ZERO);
        let (left_stick, right_stick) = push(Vec2::X);
        push_sticks(&mut step, &mut conversation, &away, left_stick, right_stick);
        assert_eq!(stick_of(&step, side), &Push::Travel, "{side:?}");
        assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
    }
    // With both rays on it, one stick scrolls it at a time: the other's push meanwhile is nobody's.
    let (mut conversation, mut step) = travelling(floor(), vec![long_panel(at)]);
    let senses = rays_placed(&mut step, &conversation, Some(on), Some(on), [0.0; 2]);
    for _ in 0..3 {
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::ZERO,
            Vec2::ZERO,
        );
    }
    push_sticks(
        &mut step,
        &mut conversation,
        &senses,
        Vec2::NEG_Y,
        Vec2::ZERO,
    );
    push_sticks(
        &mut step,
        &mut conversation,
        &senses,
        Vec2::NEG_Y,
        Vec2::NEG_Y,
    );
    assert_eq!(step.left_stick, Push::Scrolled("panel-1".into()));
    assert_eq!(step.right_stick, Push::Other);
}

#[test]
fn a_hand_hanging_at_the_side_past_a_long_panel_captures_no_push_while_one_pointed_at_it_does() {
    // A long panel low beside the path; the left hand hangs, its ray pointing steeply down through the panel, as an
    // arm at the side does walking past it.
    let at = Vec3::new(0.0, 0.4, -0.5);
    let hand = arrival::HOME + Vec3::new(0.0, -0.5, 0.0);
    let hanging: Aim = (hand, (at - hand).normalize());
    assert!(
        -hanging.1.y > 0.8,
        "pointing well down, past where a ray hides"
    );
    let pointed: Aim = (
        Vec3::new(0.0, 0.7, 0.0),
        (at - Vec3::new(0.0, 0.7, 0.0)).normalize(),
    );
    assert!(
        -pointed.1.y < 0.7,
        "pointing a little down, where a ray shows"
    );
    // On the desktop, which draws no rays and so lowers none, the hanging ray does meet the panel.
    let (mut conversation, mut step) = travelling(floor(), vec![long_panel(at)]);
    let senses = Senses {
        beam: false,
        ..rays_placed(&mut step, &conversation, None, Some(hanging), [0.0; 2])
    };
    push_sticks(
        &mut step,
        &mut conversation,
        &senses,
        Vec2::ZERO,
        Vec2::ZERO,
    );
    assert_eq!(step.hands.left.panel.as_deref(), Some("panel-1"));
    // On the headset it hides, meets nothing, and a push of the left stick turns rather than scrolls.
    let (mut conversation, mut step) = travelling(floor(), vec![long_panel(at)]);
    for _ in 0..3 {
        let senses = rays_placed(&mut step, &conversation, None, Some(hanging), [0.0; 2]);
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::ZERO,
            Vec2::ZERO,
        );
    }
    assert!(step.hands.left.lowered && !step.hands.left.full);
    assert_eq!(step.hands.left.panel, None);
    let senses = rays_placed(&mut step, &conversation, None, Some(hanging), [0.0; 2]);
    push_sticks(&mut step, &mut conversation, &senses, Vec2::X, Vec2::ZERO);
    assert_eq!(step.left_stick, Push::Travel);
    assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
    assert_eq!(step.projector.scroll("panel-1"), 0);
    // Pointed at it deliberately, its ray shown in full on it, the next push scrolls it instead.
    let (mut conversation, mut step) = travelling(floor(), vec![long_panel(at)]);
    for _ in 0..3 {
        let senses = rays_placed(&mut step, &conversation, None, Some(pointed), [0.0; 2]);
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::ZERO,
            Vec2::ZERO,
        );
    }
    assert!(step.hands.left.full && !step.hands.left.lowered);
    assert_eq!(step.hands.left.panel.as_deref(), Some("panel-1"));
    let senses = rays_placed(&mut step, &conversation, None, Some(pointed), [0.0; 2]);
    for _ in 0..10 {
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::NEG_Y,
            Vec2::ZERO,
        );
    }
    assert_eq!(step.left_stick, Push::Scrolled("panel-1".into()));
    assert!(step.projector.scroll("panel-1") > 0);
    assert_eq!(step.rig(&conversation), Rig::default());
}

#[test]
fn each_stick_moves_what_its_own_hand_carries_while_the_other_travels_carrying_it_along() {
    let at = arrival::HOME + Vec3::new(-0.3, 0.0, -1.0);
    let hand = arrival::HOME + Vec3::new(-0.3, -0.3, -0.2);
    let left: Aim = (hand, (at - hand).normalize());
    let right: Aim = (
        arrival::HOME + Vec3::new(0.3, -0.3, -0.2),
        Vec3::new(0.3, 0.0, -1.0),
    );
    let panel = tomori_protocol::Panel::new(
        "panel-1",
        "Build status",
        "Green.",
        tomori_protocol::Place::Free {
            pose: tomori_protocol::Pose::upright_facing(
                at.to_array(),
                arrival::HOME.with_y(at.y).to_array(),
            ),
        },
    );
    let (mut conversation, mut step) = travelling(floor(), vec![panel]);
    let senses = |step: &mut Step, conversation: &Conversation, grip: f32| {
        rays_placed(step, conversation, Some(right), Some(left), [0.0, grip])
    };
    for grip in [0.0, 0.0, 1.0] {
        let senses = senses(&mut step, &conversation, grip);
        push_sticks(
            &mut step,
            &mut conversation,
            &senses,
            Vec2::ZERO,
            Vec2::ZERO,
        );
    }
    assert_eq!(step.projector.held_in(Hand::Left), Some("panel-1"));
    let carried = |frame: &Pushed| {
        frame
            .projection
            .iter()
            .find(|sheet| {
                sheet
                    .canvas
                    .as_deref()
                    .is_some_and(|canvas| canvas.plain().starts_with("Build"))
            })
            .and_then(|sheet| sheet.model)
            .map(|model| model.w_axis.truncate())
            .unwrap()
    };
    // The left stick pushes the panel the left hand carries further along the left ray, and travels nowhere.
    let reach = |step: &mut Step, conversation: &Conversation, frame: &Pushed| {
        let ray = senses(step, conversation, 1.0).left_ray.unwrap();
        carried(frame).distance(ray.origin)
    };
    let held = senses(&mut step, &conversation, 1.0);
    let frame = push_sticks(&mut step, &mut conversation, &held, Vec2::ZERO, Vec2::ZERO);
    let near = reach(&mut step, &conversation, &frame);
    let mut frame = frame;
    for _ in 0..10 {
        let senses = senses(&mut step, &conversation, 1.0);
        frame = push_sticks(&mut step, &mut conversation, &senses, Vec2::Y, Vec2::ZERO);
    }
    assert_eq!(step.left_stick, Push::Carried);
    assert_eq!(frame.landing, None);
    assert!(reach(&mut step, &conversation, &frame) > near + 0.1);
    let far = reach(&mut step, &conversation, &frame);
    // The right stick, its ray on nothing, turns the participant, and the panel stays in the left hand, as far
    // along its ray as it was.
    for stick in [Vec2::ZERO, Vec2::X, Vec2::ZERO] {
        let senses = senses(&mut step, &conversation, 1.0);
        push_sticks(&mut step, &mut conversation, &senses, Vec2::ZERO, stick);
    }
    assert!(rig_ahead(&mut step, &conversation).distance(turned(1.0)) < 1e-4);
    let senses_now = senses(&mut step, &conversation, 1.0);
    let frame = push_sticks(
        &mut step,
        &mut conversation,
        &senses_now,
        Vec2::ZERO,
        Vec2::ZERO,
    );
    assert_eq!(step.projector.held_in(Hand::Left), Some("panel-1"));
    let ray = senses_now.left_ray.unwrap();
    assert!(carried(&frame).distance(ray.at(far)) < 1e-2);
}

#[test]
fn the_trigger_held_while_a_wall_is_carried_attracts_no_swarm_and_lets_it_go_unsnapped() {
    use tomori_protocol::{Pose, Wall, WallKind};
    let wall = Wall::new(
        "wall-1",
        WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -2.0], arrival::HOME.to_array()),
    );
    let board = Vec3::new(0.0, 1.5 + 0.42, -2.0);
    let mut conversation = lobby(ConnectionRole::Owner);
    fixtures::show_editable_walls(&mut conversation, vec![wall], Vec::new());
    let mut step = Step::default();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ray: Ray::new(arrival::HOME, board - arrival::HOME),
        grip: Some(0.0),
        ..senses()
    };
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    }
    senses.grip = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.projector.carried_wall(), Some("wall-1"));
    // Pulled while it is carried, the trigger attracts nothing, and the hint says it is left where it is.
    senses.trigger = Some(1.0);
    senses.attractor = Some(board);
    for _ in 0..3 {
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(!frame.attracting);
        assert!(frame.projection.iter().any(|sheet| {
            sheet
                .canvas
                .as_deref()
                .is_some_and(|canvas| canvas.plain().contains("leave it here"))
        }));
    }
    // Let go, it is no longer carried, and the trigger attracts the swarm again.
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.projector.carried_wall(), None);
    senses.ray = Ray::new(arrival::HOME, Vec3::Z);
    senses.trigger = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    senses.trigger = Some(1.0);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(!frame.attracting);
    // A change to it the server refused, as made against a version since changed, the frame step takes, to draw
    // the wall as the server has it at once.
    conversation.apply(tomori_client::Event::Message(
        tomori_protocol::ServerMessage::Error {
            error: "That wall changed as you changed it; look again.".into(),
            space: None,
            panel: None,
            wall: Some("wall-1".into()),
        },
    ));
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(conversation.take_refused_wall(), None);
}

#[test]
fn a_wall_is_carried_with_the_grip_put_down_where_let_go_and_removed_by_an_owner_s_b_asked_twice_with_panels()
 {
    use tomori_protocol::{ClientMessage, Panel, Place, Pose, Wall, WallKind};
    let wall = Wall::new(
        "wall-1",
        WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -2.0], arrival::HOME.to_array()),
    );
    let panel = Panel::new(
        "panel-1",
        "Build",
        "Green.",
        Place::Wall {
            wall: "wall-1".into(),
            slot: 2,
        },
    );
    // Between the top row of slots and the label, on the board.
    let board = Vec3::new(0.0, 1.5 + 0.42, -2.0);
    let run = |role: ConnectionRole, editable: bool| {
        let mut conversation = lobby(role);
        if editable {
            fixtures::show_editable_walls(
                &mut conversation,
                vec![wall.clone()],
                vec![panel.clone()],
            );
        } else {
            fixtures::show_projection(&mut conversation, vec![wall.clone()], vec![panel.clone()]);
        }
        let mut step = Step::default();
        let start = Instant::now();
        let mut senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, board - arrival::HOME),
            grip: Some(0.0),
            ..senses()
        };
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
        }
        senses.grip = Some(1.0);
        let frame = step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
        let hint = frame
            .projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref().map(crate::canvas::Canvas::plain))
            .find(|text| text.contains("leave it here"));
        (conversation, step, senses, hint, start)
    };
    let b = |step: &mut Step, conversation: &mut Conversation, at: Instant| {
        for (back, held, after) in [(true, true, 0), (false, false, 100)] {
            step.buttons(
                &press(|buttons| {
                    buttons.back = back;
                    buttons.back_held = held;
                }),
                true,
                conversation,
                &mut felt(),
                at + Duration::from_millis(after),
            );
        }
    };
    let removals = |conversation: &mut Conversation| {
        conversation
            .take_actions()
            .into_iter()
            .filter(|action| matches!(action, Action::Send(ClientMessage::RemoveWall { .. })))
            .count()
    };
    // An owner carries it, and is told B removes it.
    let (mut conversation, mut step, mut senses, hint, start) = run(ConnectionRole::Owner, true);
    assert_eq!(step.projector.carried_wall(), Some("wall-1"));
    let hint = hint.expect("the hint shows while carrying");
    assert!(hint.contains("remove, for all"), "{hint}");
    // It holds a panel, so a first press of B only asks again, and a second removes it, for everyone.
    b(&mut step, &mut conversation, start);
    assert_eq!(removals(&mut conversation), 0);
    b(
        &mut step,
        &mut conversation,
        start + Duration::from_millis(500),
    );
    assert_eq!(removals(&mut conversation), 1);
    assert_eq!(step.projector.editing_wall(), None);
    // Carried elsewhere and let go, its new place is sent.
    let (mut conversation, mut step, _, _, start) = run(ConnectionRole::Owner, true);
    senses.ray = Ray::new(
        arrival::HOME,
        board + Vec3::new(-0.8, 0.0, 0.0) - arrival::HOME,
    );
    step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
    let sent = conversation.take_actions();
    assert!(
        sent.iter().any(|action| matches!(
            action,
            Action::Send(ClientMessage::SetWall { wall, pose: Some(pose), grid: None, .. })
                if wall == "wall-1" && pose.position[0] < -0.5
        )),
        "{sent:?}"
    );
    // Held, not carried, B lets go of it, and backs out of nothing else.
    assert_eq!(step.projector.editing_wall(), Some("wall-1"));
    b(&mut step, &mut conversation, start);
    assert_eq!(step.projector.editing_wall(), None);
    assert!(conversation.take_actions().is_empty());
    // A collaborator carries it, but B removes nothing, and their hint does not offer it.
    let (mut conversation, mut step, _, hint, start) = run(ConnectionRole::Collaborator, true);
    assert_eq!(step.projector.carried_wall(), Some("wall-1"));
    assert!(!hint.unwrap().contains("remove"));
    b(&mut step, &mut conversation, start);
    b(
        &mut step,
        &mut conversation,
        start + Duration::from_millis(500),
    );
    assert_eq!(removals(&mut conversation), 0);
    // A visitor, and anyone where the server takes no wall changes, takes no wall.
    let (_, step, _, _, _) = run(ConnectionRole::Visitor, true);
    assert_eq!(step.projector.editing_wall(), None);
    let (mut conversation, step, _, _, _) = run(ConnectionRole::Owner, false);
    assert_eq!(step.projector.editing_wall(), None);
    let change = crate::projection::editing::Change {
        wall: "wall-1".into(),
        grid: None,
        scale: Some(1.5),
        aspect: None,
        shape: None,
        pose: None,
        join: None,
        tilt: None,
        content_scale: None,
        version: Some(7),
    };
    assert!(!conversation.set_wall(change.clone()));
    assert!(!conversation.remove_wall_by_id("wall-1".into()));
    assert!(conversation.take_actions().is_empty());
    let mut conversation = lobby(ConnectionRole::Collaborator);
    fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], Vec::new());
    assert!(conversation.set_wall(change.clone()));
    assert!(!conversation.remove_wall_by_id("wall-1".into()));
    // Made against the wall as it was taken hold of, it says its version then, so that the server refuses it where
    // the wall changed since; one made against none, or a wall from an older server, at 0, says none.
    assert!(matches!(
        conversation.take_actions().as_slice(),
        [Action::Send(ClientMessage::SetWall {
            revision: Some(7),
            ..
        })]
    ));
    for version in [None, Some(0)] {
        assert!(conversation.set_wall(crate::projection::editing::Change {
            version,
            ..change.clone()
        }));
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Send(ClientMessage::SetWall { revision: None, .. })]
        ));
    }
    // A snap is sent as one, and so is taking a wall out of its group, where it was let go.
    use crate::projection::editing::Joining;
    let attach = tomori_protocol::Attach::new(
        "wall-2",
        tomori_protocol::Edge::Right,
        tomori_protocol::Align::Middle,
    );
    conversation.take_actions();
    let pose = tomori_protocol::Pose::default();
    assert!(conversation.set_wall(crate::projection::editing::Change {
        join: Some(Joining::Attach {
            attach: attach.clone(),
            alone: true,
        }),
        ..change.clone()
    }));
    assert!(conversation.set_wall(crate::projection::editing::Change {
        join: Some(Joining::Leave),
        pose: Some(pose),
        ..change.clone()
    }));
    assert_eq!(
        conversation.take_actions(),
        [
            Action::Send(ClientMessage::AttachWall {
                wall: "wall-1".into(),
                attach: Some(attach.clone()),
                alone: true,
                pose: None,
            }),
            Action::Send(ClientMessage::AttachWall {
                wall: "wall-1".into(),
                attach: None,
                alone: true,
                pose: Some(pose),
            }),
        ]
    );
    // A server that snaps no walls is sent neither.
    conversation.apply(fixtures::space(|state| {
        state.projection = true;
        state.walls = true;
        state.groups = false;
    }));
    assert!(!conversation.wall_rights().snap);
    assert!(!conversation.set_wall(crate::projection::editing::Change {
        join: Some(Joining::Leave),
        ..change
    }));
}

#[test]
fn an_owner_unlocks_a_locked_wall_with_its_padlock_for_everyone_while_others_see_no_padlock() {
    use crate::projection::editing::{LockChange, LockControls, handle_points_with};
    use tomori_protocol::{ClientMessage, Pose, Wall, WallKind};
    let wall = Wall::new(
        "environment-0",
        WallKind::Environment,
        Pose::upright_facing([0.0, 1.5, -2.0], arrival::HOME.to_array()),
    )
    .with_grid(tomori_protocol::Grid::from((2, 1)))
    .with_locked(true);
    let frame_of = crate::board::model(&wall.pose).unwrap();
    let lock = LockControls {
        padlock: true,
        reset: false,
    };
    let (points, _) = handle_points_with(&wall, frame_of, lock);
    let padlock = points[0].1;
    let run = |role: ConnectionRole, locks: bool| {
        let mut conversation = lobby(role);
        if locks {
            fixtures::show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
        } else {
            fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], Vec::new());
        }
        let mut step = Step::default();
        let start = Instant::now();
        let mut senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, padlock - arrival::HOME),
            grip: Some(0.0),
            ..senses()
        };
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
        }
        // Squeezed, its padlock, shown to an owner near it without taking hold of the wall, unlocks it.
        let mut hint = None;
        for grip in [0.0, 1.0, 0.0] {
            senses.grip = Some(grip);
            let frame = step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
            hint = hint.or_else(|| {
                frame
                    .projection
                    .iter()
                    .filter_map(|sheet| sheet.canvas.as_deref().map(crate::canvas::Canvas::plain))
                    .find(|text| text.contains("unlock") || text.contains("Locked"))
            });
        }
        (conversation.take_actions(), hint)
    };
    let unlocks = |sent: &[Action]| {
        sent.iter()
            .filter(|action| {
                matches!(
                    action,
                    Action::Send(ClientMessage::LockWall { wall, locked: false, reset: false })
                        if wall == "environment-0"
                )
            })
            .count()
    };
    let (sent, hint) = run(ConnectionRole::Owner, true);
    assert_eq!(unlocks(&sent), 1, "{sent:?}");
    let hint = hint.unwrap();
    assert!(hint.contains("Grip  unlock this wall"), "{hint}");
    // A collaborator sees no padlock, nor what the grip would do there, and sends nothing.
    let (sent, hint) = run(ConnectionRole::Collaborator, true);
    assert_eq!(unlocks(&sent), 0, "{sent:?}");
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(hint, None);
    // Where the server locks no walls, as an older one, there is no padlock to squeeze, and nothing is sent.
    let (sent, _) = run(ConnectionRole::Owner, false);
    assert_eq!(unlocks(&sent), 0, "{sent:?}");
    // Nor does anyone ask a server that locks no walls, and only an owner asks one that does.
    let mut conversation = lobby(ConnectionRole::Owner);
    fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], Vec::new());
    let change = LockChange {
        wall: "environment-0".into(),
        locked: false,
        reset: false,
    };
    assert!(!conversation.lock_wall(change.clone()));
    let mut conversation = lobby(ConnectionRole::Collaborator);
    fixtures::show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
    assert!(!conversation.lock_wall(change.clone()));
    assert!(conversation.take_actions().is_empty());
    let mut conversation = lobby(ConnectionRole::Owner);
    fixtures::show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
    assert!(conversation.lock_wall(change));
}

#[test]
fn b_throws_a_panel_away_only_while_it_is_carried_and_a_hold_then_neither_throws_nor_leaves() {
    use tomori_protocol::{ClientMessage, Panel, Place};
    let mut conversation = lobby(ConnectionRole::Owner);
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    fixtures::show_projection(
        &mut conversation,
        Vec::new(),
        vec![Panel::new(
            "panel-1",
            "Build status",
            "Green.",
            Place::Free {
                pose: tomori_protocol::Pose::upright_facing(
                    ahead.to_array(),
                    arrival::HOME.to_array(),
                ),
            },
        )],
    );
    let mut step = Step::default();
    let mut feedback = felt();
    let start = Instant::now();
    let mut senses = Senses {
        beam: true,
        ray: Ray::new(arrival::HOME, -Vec3::Z),
        grip: Some(0.0),
        ..senses()
    };
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut feedback, start, FRAME);
    }
    let hint = |frame: &Frame<'_>| {
        frame
            .projection
            .iter()
            .map(|sheet| sheet.canvas.as_deref().unwrap().plain())
            .find(|text| text.contains("throw away"))
    };
    let frame = |step: &mut Step, conversation: &mut Conversation, senses: &Senses, at| {
        let frame = step.update(senses, conversation, &mut felt(), at, FRAME);
        (
            hint(&frame),
            frame.projection.iter().any(|sheet| {
                sheet
                    .canvas
                    .as_deref()
                    .unwrap()
                    .plain()
                    .starts_with("Build")
            }),
        )
    };
    let b = |step: &mut Step, conversation: &mut Conversation, back: bool, held: bool, at| {
        step.buttons(
            &press(|buttons| {
                buttons.back = back;
                buttons.back_held = held;
            }),
            true,
            conversation,
            &mut felt(),
            at,
        );
    };
    // Pointed at, not carried, a press of B dismisses nothing, and no hint shows.
    let (shown, _) = frame(&mut step, &mut conversation, &senses, start);
    assert_eq!(shown, None);
    assert_eq!(
        step.update(&senses, &mut conversation, &mut felt(), start, FRAME)
            .presence
            .and_then(|presence| presence.carried.first().cloned()),
        None
    );
    b(&mut step, &mut conversation, true, true, start);
    b(
        &mut step,
        &mut conversation,
        false,
        false,
        start + Duration::from_millis(100),
    );
    assert!(conversation.take_actions().is_empty());
    assert_eq!(conversation.notice(), Some(crate::conversation::THROW_HOW));
    // Carried with the grip, the hint shows beside it.
    senses.grip = Some(1.0);
    frame(&mut step, &mut conversation, &senses, start);
    let (shown, _) = frame(&mut step, &mut conversation, &senses, start);
    assert_eq!(step.projector.held(), Some("panel-1"));
    // The server is told which panel is carried, so that the agent leaves it in the hand.
    let carrying = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
        step.update(senses, conversation, &mut felt(), start, FRAME)
            .presence
            .and_then(|presence| presence.carried.first().cloned())
    };
    assert_eq!(
        carrying(&mut step, &mut conversation, &senses).as_deref(),
        Some("panel-1")
    );
    let shown = shown.expect("the hint shows while carrying");
    assert!(shown.contains("throw away, for all"), "{shown}");
    assert!(shown.contains("leave it floating"), "{shown}");
    // Held while carrying, B neither throws the panel away nor leaves, however long, and letting it go then does
    // nothing.
    let now = start + Duration::from_secs(1);
    b(&mut step, &mut conversation, true, true, now);
    for held in [LEAVE_HOLD / 2, LEAVE_HOLD, LEAVE_HOLD * 2] {
        b(&mut step, &mut conversation, false, true, now + held);
        frame(&mut step, &mut conversation, &senses, now + held);
    }
    b(
        &mut step,
        &mut conversation,
        false,
        false,
        now + LEAVE_HOLD * 2,
    );
    assert!(conversation.take_actions().is_empty());
    assert!(conversation.joined_space().is_some(), "still in the space");
    assert_eq!(step.projector.held(), Some("panel-1"));
    // Let go of the grip before B, the panel is put down, not thrown away.
    let now = now + LEAVE_HOLD * 3;
    b(&mut step, &mut conversation, true, true, now);
    senses.grip = Some(0.0);
    frame(&mut step, &mut conversation, &senses, now);
    b(
        &mut step,
        &mut conversation,
        false,
        false,
        now + Duration::from_millis(50),
    );
    let actions = conversation.take_actions();
    assert!(
        matches!(
            actions.as_slice(),
            [Action::Send(ClientMessage::PlacePanel { .. })]
        ),
        "{actions:?}"
    );
    // Picked up again, a press of B, let go soon enough, throws it away for everyone at once.
    let now = now + Duration::from_secs(1);
    senses.grip = Some(1.0);
    frame(&mut step, &mut conversation, &senses, now + PENDING_OVER);
    assert_eq!(step.projector.held(), Some("panel-1"));
    let now = now + PENDING_OVER;
    b(&mut step, &mut conversation, true, true, now);
    assert!(
        conversation.take_actions().is_empty(),
        "only once B is let go"
    );
    b(
        &mut step,
        &mut conversation,
        false,
        false,
        now + Duration::from_millis(200),
    );
    let actions = conversation.take_actions();
    assert_eq!(
        actions,
        [Action::Send(ClientMessage::DismissPanel {
            panel: "panel-1".into()
        })]
    );
    assert_eq!(step.projector.held(), None);
    // It is neither drawn nor hinted at, nor put down as the grip is let go, while the server agrees.
    let (shown, drawn) = frame(&mut step, &mut conversation, &senses, now);
    assert_eq!((shown, drawn), (None, false));
    senses.grip = Some(0.0);
    frame(&mut step, &mut conversation, &senses, now);
    assert!(conversation.take_actions().is_empty());
    // Holding B, with nothing carried, leaves the space as ever.
    let now = now + Duration::from_secs(1);
    b(&mut step, &mut conversation, true, true, now);
    b(&mut step, &mut conversation, false, true, now + LEAVE_HOLD);
    assert!(conversation.joined_space().is_none(), "left");
}

/// Longer than a panel let go is shown where it was let go, so that the server's projection, which these tests do
/// not change, stands it again.
const PENDING_OVER: Duration = Duration::from_secs(4);

#[test]
fn a_panel_carried_takes_the_right_stick_and_its_click_from_the_transcript_and_replays() {
    use crate::locomotion::{Locomotion, Surface};
    use tomori_protocol::{ClientMessage, Panel, Place, ServerMessage};
    let mut conversation = conversed();
    conversation.set_locomotion(vec![(
        crate::test_support::SKY,
        Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
    )]);
    let messages = conversation.joined_space().unwrap().0.messages.clone();
    conversation.apply(space(|state| {
        state.messages = messages;
        state.projection = true;
        state.speech = tomori_protocol::SpeechSetting::Full;
    }));
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![Panel::new(
                    "panel-1",
                    "Build status",
                    "Green.",
                    Place::Free {
                        pose: tomori_protocol::Pose::upright_facing(
                            ahead.to_array(),
                            arrival::HOME.to_array(),
                        ),
                    },
                )]),
        ),
    }));
    conversation.take_actions();
    let mut step = Step::default();
    let mut feedback = felt();
    let start = Instant::now();
    let mut senses = Senses {
        ray: Ray::new(arrival::HOME, -Vec3::Z),
        right_palm: Some(palm_up(0.0)),
        grip: Some(0.0),
        ..senses()
    };
    // The transcript shows on the upturned palm, then goes as the palm turns away, and the panel is pulled off.
    assert!(
        wrist_after(&mut step, &mut conversation, &senses, start)
            .model
            .is_some()
    );
    senses.right_palm = None;
    for _ in 0..400 {
        step.update(
            &senses,
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
    }
    senses.grip = Some(1.0);
    step.update(
        &senses,
        &mut conversation,
        &mut feedback,
        start + HOLD,
        FRAME,
    );
    assert_eq!(step.projector.held(), Some("panel-1"));
    // Carried, the upturned palm shows no transcript, which would let the panel go.
    senses.right_palm = Some(palm_up(0.0));
    let wrist = wrist_after(&mut step, &mut conversation, &senses, start + HOLD * 2);
    assert!(wrist.model.is_none());
    assert_eq!(step.projector.held(), Some("panel-1"));
    let now = start + HOLD * 3;
    let carried = |frame: &Frame<'_>| {
        frame
            .projection
            .iter()
            .find(|sheet| {
                sheet
                    .canvas
                    .as_deref()
                    .unwrap()
                    .plain()
                    .starts_with("Build")
            })
            .map(|sheet| sheet.model.unwrap())
            .unwrap()
    };
    let buttons = |step: &mut Step, conversation: &mut Conversation, buttons: Buttons| {
        step.buttons(&buttons, true, conversation, &mut felt(), now);
    };
    // A flick of the right stick turns the panel a snap, with a tick, and moves neither the transcript's focus nor the
    // cards; the left stick turns the participant in the same frame.
    let focus = conversation.wrist_focus().cloned();
    let square = carried(&step.update(&senses, &mut conversation, &mut feedback, now, FRAME));
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| {
            buttons.right_stick = Vec2::X;
            buttons.left_stick = Vec2::NEG_X;
        }),
    );
    felt_on(&mut feedback, Side::Right, now);
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let turned = carried(&frame);
    assert!(
        (square
            .z_axis
            .truncate()
            .angle_between(turned.z_axis.truncate())
            - crate::manipulation::YAW_STEP)
            .abs()
            < 1e-3
    );
    assert!(!felt_on(&mut feedback, Side::Right, now).is_empty());
    assert_eq!(conversation.wrist_focus().cloned(), focus);
    assert_eq!(step.presentation_navigation.card, 0);
    assert!(
        step.rig(&conversation)
            .matrix()
            .transform_vector3(-Vec3::Z)
            .x
            < -0.4
    );
    // Its click turns it back square, and plays nothing again.
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.replay = true),
    );
    let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let straightened = carried(&frame);
    assert!(
        straightened
            .z_axis
            .truncate()
            .angle_between(square.z_axis.truncate())
            < 1e-3
    );
    assert!(conversation.take_actions().is_empty(), "no replay");
    // A push begun while carrying stays the panel's after it is let go, so the transcript then showing takes none of it.
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::NEG_Y),
    );
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.projector.held(), None);
    conversation.take_actions();
    assert!(
        wrist_after(&mut step, &mut conversation, &senses, now)
            .model
            .is_some()
    );
    for _ in 0..3 {
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::NEG_Y),
        );
    }
    assert_eq!(conversation.wrist_focus().cloned(), focus);
    // Back at the middle, the next push is the transcript's, and the click replays again.
    buttons(&mut step, &mut conversation, Buttons::default());
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::NEG_X),
    );
    assert_ne!(conversation.wrist_focus().cloned(), focus);
    buttons(&mut step, &mut conversation, Buttons::default());
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.replay = true),
    );
    assert!(matches!(
        conversation.take_actions().as_slice(),
        [Action::Send(ClientMessage::Replay { message: None })]
    ));
    // Losing focus forgets whose the push was.
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::X),
    );
    step.blur(&mut conversation);
    assert_eq!(step.right_stick, Push::Free);
    assert_eq!(Push::Other.update(Vec2::NAN, || Push::Carried), Push::Free);
}

#[test]
fn a_push_of_the_right_stick_pointing_at_a_panel_with_more_than_it_shows_scrolls_it_for_this_participant()
 {
    use tomori_protocol::{Code, Panel, Place};
    let mut conversation = joined();
    // Thirty lines of code in the Tomori wall's middle slot, more than a slot shows.
    let body: Vec<String> = (1..=30)
        .map(|line| format!("let x{line} = {line};"))
        .collect();
    show_projection(
        &mut conversation,
        Vec::new(),
        vec![
            Panel::new(
                "panel-1",
                "rust",
                body.join("\n"),
                Place::Wall {
                    wall: tomori_protocol::TOMORI_WALL.into(),
                    slot: 2,
                },
            )
            .with_code(Code::new("rust", 30)),
        ],
    );
    let wall = tomori_wall();
    let frame = Mat4::from_rotation_translation(
        glam::Quat::from_array(wall.pose.orientation),
        Vec3::from_array(wall.pose.position),
    );
    let slot = crate::projection::slot_frames(&wall, frame)[2]
        .w_axis
        .truncate();
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let on_panel = Senses {
        ray: Ray::new(arrival::HOME, slot - arrival::HOME),
        grip: Some(0.0),
        ..senses()
    };
    let away = Senses {
        ray: Ray::new(arrival::HOME, Vec3::Y),
        ..on_panel
    };
    let footer = |frame: &Frame<'_>| {
        frame
            .projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .find(|canvas| canvas.plain().starts_with("rust"))
            .map(|canvas| canvas.plain().lines().last().unwrap().trim_end().to_owned())
            .unwrap()
    };
    let buttons = |step: &mut Step, conversation: &mut Conversation, buttons: Buttons| {
        step.buttons(&buttons, true, conversation, &mut felt(), now);
    };
    // Pointed at, its label says the stick scrolls it.
    let frame = step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(footer(&frame), "↓ 14 more");
    assert!(
        frame
            .projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .any(|canvas| canvas.plain().contains("Stick ↕  scroll"))
    );
    conversation.take_actions();
    // Pulled back, the stick scrolls it down, faster the further it is pulled, paging no cards and sending nothing.
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::NEG_Y),
    );
    assert!(matches!(step.right_stick, Push::Scrolled(ref panel) if panel == "panel-1"));
    step.update(&on_panel, &mut conversation, &mut feedback, now, 0.2);
    assert_eq!(step.projector.scroll("panel-1"), 3);
    assert_eq!(step.presentation_navigation.card, 0);
    assert!(conversation.take_actions().is_empty());
    // The push stays the panel's while the ray wanders off it, until the stick comes back to the middle.
    step.update(&away, &mut conversation, &mut feedback, now, 10.0);
    assert_eq!(step.projector.scroll("panel-1"), 14);
    let frame = step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(footer(&frame), "↑ 14 above");
    buttons(&mut step, &mut conversation, Buttons::default());
    assert_eq!(step.right_stick, Push::Free);
    // Pushed forward, it scrolls back up.
    step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::Y),
    );
    step.update(&on_panel, &mut conversation, &mut feedback, now, 0.4);
    assert_eq!(step.projector.scroll("panel-1"), 8);
    buttons(&mut step, &mut conversation, Buttons::default());
    // A push begun pointing at nothing that scrolls is not the panel's, even once the ray comes back to it.
    step.update(&away, &mut conversation, &mut feedback, now, FRAME);
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::NEG_Y),
    );
    assert_eq!(step.right_stick, Push::Other);
    step.update(&on_panel, &mut conversation, &mut feedback, now, 1.0);
    assert_eq!(step.projector.scroll("panel-1"), 8);
    buttons(&mut step, &mut conversation, Buttons::default());
    // The wheel scrolls the panel pointed at, by its rows, at once.
    step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.scroll = -3.0),
    );
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.scroll = -2.0),
    );
    step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.projector.scroll("panel-1"), 3);
    // A push begun scrolling, then the grip pulling the panel off, scrolls it no further and does not push it.
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::Y),
    );
    let gripped = Senses {
        grip: Some(1.0),
        ..on_panel
    };
    step.update(&gripped, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.projector.held(), Some("panel-1"));
    let carried = |frame: &Frame<'_>| {
        frame
            .projection
            .iter()
            .find(|sheet| sheet.canvas.as_deref().unwrap().plain().starts_with("rust"))
            .map(|sheet| sheet.model.unwrap().w_axis.truncate())
            .unwrap()
    };
    // Long enough for it to glide into the slot it docks in, where it was.
    let before = carried(&step.update(&gripped, &mut conversation, &mut feedback, now, 0.5));
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| buttons.right_stick = Vec2::Y),
    );
    assert_eq!(step.right_stick, Push::Scrolled("panel-1".into()));
    let after = carried(&step.update(&gripped, &mut conversation, &mut feedback, now, 0.5));
    assert!(before.distance(after) < 1e-4);
    // Carried, a new push moves it, as ever, and the wheel scrolls nothing.
    buttons(&mut step, &mut conversation, Buttons::default());
    buttons(
        &mut step,
        &mut conversation,
        press(|buttons| {
            buttons.right_stick = Vec2::Y;
            buttons.scroll = 3.0;
        }),
    );
    assert_eq!(step.right_stick, Push::Carried);
    assert_eq!(step.wheel, None);
    let pushed = carried(&step.update(&gripped, &mut conversation, &mut feedback, now, 0.5));
    assert!(pushed.distance(arrival::HOME) > after.distance(arrival::HOME) + 0.1);
    // Losing focus forgets the scrolling push and the wheel.
    step.blur(&mut conversation);
    assert_eq!(
        (step.scroll_stick.clone(), step.wheel.clone()),
        (None, None)
    );
}

#[test]
fn the_stick_and_the_wheel_scroll_together_the_stick_s_panel_first() {
    let stick = ("panel-1".to_owned(), -1.0);
    let rows = crate::projection::SCROLL_SPEED * 0.5;
    // Both on one panel add; on two, the stick's, until it is let go; either alone, its own.
    assert_eq!(
        scrolled(Some(&stick), Some(("panel-1".into(), 2.0)), 0.5),
        Some(("panel-1".into(), rows + 2.0))
    );
    assert_eq!(
        scrolled(Some(&stick), Some(("panel-2".into(), 2.0)), 0.5),
        Some(("panel-1".into(), rows))
    );
    assert_eq!(
        scrolled(None, Some(("panel-2".into(), 2.0)), 0.5),
        Some(("panel-2".into(), 2.0))
    );
    assert_eq!(scrolled(None, None, 0.5), None);
}

#[test]
fn a_panel_carried_to_the_tomori_wall_shows_its_slot_and_is_put_in_it_for_everyone() {
    use tomori_protocol::{ClientMessage, Panel, Place};
    let mut conversation = lobby(ConnectionRole::Owner);
    let ahead = arrival::HOME + Vec3::new(-1.0, 0.0, -1.5);
    fixtures::show_projection(
        &mut conversation,
        Vec::new(),
        vec![Panel::new(
            "panel-1",
            "Build status",
            "Green.",
            Place::Free {
                pose: tomori_protocol::Pose::upright_facing(
                    ahead.to_array(),
                    arrival::HOME.to_array(),
                ),
            },
        )],
    );
    let wall = tomori_wall();
    let frame = crate::board::model(&wall.pose).unwrap();
    let slot = crate::projection::slot_frames(&wall, frame)[3];
    let mut step = Step::default();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ray: Ray::new(arrival::HOME, ahead - arrival::HOME),
        grip: Some(0.0),
        ..senses()
    };
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    senses.grip = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.projector.held(), Some("panel-1"));
    // Pointed at the wall's fourth slot, but carried well short of it, no outline shows, and the ray ends on the
    // panel, where it is carried.
    let middle = slot.w_axis.truncate();
    senses.ray = Ray::new(arrival::HOME, middle - arrival::HOME);
    let outline = slot * Mat4::from_translation(Vec3::Z * 0.01);
    let outlined = |output: &Frame<'_>| {
        output
            .projection
            .iter()
            .any(|sheet| sheet.model == Some(outline))
    };
    let beam = |output: &Frame<'_>| {
        output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap()
            .end
    };
    let carried = ahead.distance(arrival::HOME);
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(!outlined(&output));
    assert!((beam(&output).distance(arrival::HOME) - carried).abs() < 1e-3);
    // Pushed along the ray up to the wall with the right stick, its outline shows there, and the panel and the ray's
    // end are pulled into the slot.
    let push = (middle.distance(arrival::HOME) / carried).ln() / crate::manipulation::RATE;
    let stick = |right_stick: Vec2| Buttons {
        right_stick,
        ..Buttons::default()
    };
    step.buttons(&stick(Vec2::Y), true, &mut conversation, &mut felt(), now);
    step.update(&senses, &mut conversation, &mut felt(), now, push);
    step.buttons(
        &stick(Vec2::ZERO),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    let output = step.update(&senses, &mut conversation, &mut felt(), now, 1.0);
    assert!(outlined(&output));
    assert!(beam(&output).distance(middle + slot.z_axis.truncate().normalize() * 0.02) < 1e-3);
    // Let go, it is put in that slot, for everyone.
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(
        conversation.take_actions(),
        [Action::Send(ClientMessage::PlacePanel {
            panel: "panel-1".into(),
            wall: Some(tomori_protocol::TOMORI_WALL.into()),
            slot: Some(3),
            pose: None,
            size: None,
            revision: None,
        })]
    );
}

#[test]
fn pointed_at_on_a_checklist_item_as_x_is_pressed_the_words_name_its_line() {
    use tomori_protocol::{Panel, Place, Pose, Resource, ServerMessage};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.projection = true));
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![Panel::new(
                    "panel-1",
                    "Agenda",
                    "To cover:\n\n- [x] Panel ids\n- [ ] Pairing window",
                    Place::Free {
                        pose: Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array()),
                    },
                )]),
        ),
    }));
    conversation.take_actions();
    let mut step = Step::default();
    let now = Instant::now();
    // From the top of the panel down, what X held attends to.
    let mut named = Vec::new();
    for step_down in 0..60 {
        let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        if let Some(attended) = utterance.attention.first()
            && named.last() != Some(attended)
        {
            named.push(attended.clone());
        }
    }
    // The panel itself above its list, then each item by the line it begins on, then the panel again below.
    assert_eq!(
        named,
        [
            Resource::panel("panel-1"),
            Resource::panel_at("panel-1", Some(2)),
            Resource::panel_at("panel-1", Some(3)),
            Resource::panel("panel-1"),
        ]
    );
}

#[test]
fn pointing_at_a_frozen_chart_draws_a_readout_linked_to_it_over_another_frozen_chart() {
    use tomori_protocol::chart::{AxisLabel, TimeAxis};
    use tomori_protocol::{Panel, Place, Pose, ServerMessage};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.projection = true));
    let at = |x: f32| arrival::HOME + Vec3::new(x, 0.0, -2.0);
    let frozen = |id: &str, x: f32, from: &str, to: &str| {
        let mut chart = crate::chart::fixtures::admissions();
        let last = chart.points - 1;
        chart.labels = vec![AxisLabel::new(0, from), AxisLabel::new(last, to)];
        // The same half hour of 12 November 2026 in UTC, by which they link, the labels only shown.
        chart.time = Some(TimeAxis::Fixed {
            from_ms: 1_794_450_000_000,
            to_ms: 1_794_451_800_000,
        });
        Panel::new(
            id,
            id,
            chart.summary(),
            Place::Free {
                pose: Pose::upright_facing(at(x).to_array(), arrival::HOME.to_array()),
            },
        )
        .with_chart(chart)
    };
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![
                    frozen("us-east1", -0.6, "02:20", "02:50"),
                    frozen("eu-west1", 0.6, "02:20", "02:50"),
                ]),
        ),
    }));
    conversation.take_actions();
    let mut step = Step::default();
    let overlays = |frame: &Frame| {
        frame
            .projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .filter(|canvas| canvas.background[3] == 0.0 && !canvas.labels.is_empty())
            .map(|canvas| crate::ui_text::plain(&canvas.labels[0].cells))
            .collect::<Vec<_>>()
    };
    // Through the middle of the first chart's plot, a little below its middle: the same time on both.
    let pointing = Senses {
        beam: true,
        ray: Ray::new(
            arrival::HOME,
            at(-0.6) + Vec3::new(0.0, -20.0 * crate::projection::PIXEL, 0.0) - arrival::HOME,
        ),
        ..senses()
    };
    let mut frame = step.update(
        &pointing,
        &mut conversation,
        &mut felt(),
        Instant::now(),
        FRAME,
    );
    for _ in 0..3 {
        frame = step.update(
            &pointing,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            FRAME,
        );
    }
    let read = overlays(&frame);
    assert_eq!(read.len(), 2, "{read:?}");
    assert_eq!(read[0], read[1]);
    assert!(read[0].starts_with("02:3"), "{read:?}");
    // Pointing at nothing, neither.
    let away = Senses {
        beam: true,
        ray: Ray::new(arrival::HOME, Vec3::Y),
        ..senses()
    };
    let frame = step.update(&away, &mut conversation, &mut felt(), Instant::now(), FRAME);
    assert!(overlays(&frame).is_empty());
}

#[test]
fn words_begun_pointing_at_a_panel_s_chip_name_the_chip() {
    use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};
    use tomori_protocol::{Panel, Place, Pose, Resource, ServerMessage};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.projection = true));
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let panel = Panel::new(
        "panel-1",
        "Pods",
        "- pod-7",
        Place::Free {
            pose: Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array()),
        },
    )
    .with_context(vec![ContextChip::new(
        "region",
        "Region",
        ContextValue::Text {
            text: "EU-WEST1".into(),
        },
        ContextSetting::Fixed,
    )]);
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![panel.clone()]),
        ),
    }));
    conversation.take_actions();
    let mut step = Step::default();
    let (width, height) = crate::projection::fitted_size(&panel, crate::projection::Fit::Text);
    let (padding, pixel) = (crate::projection::PADDING, crate::projection::PIXEL);
    let at = |column: f32, row: f32| {
        ahead
            + Vec3::new(
                -width / 2.0
                    + (padding.0 as f32 + column * crate::ui_text::CELL_WIDTH as f32) * pixel,
                height / 2.0
                    - (padding.1 as f32 + row * crate::ui_text::CELL_HEIGHT as f32) * pixel,
                0.0,
            )
    };
    let mut named = |point: Vec3| {
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, point - arrival::HOME),
            ..senses()
        };
        step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            FRAME,
        );
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        utterance.attention.first().cloned()
    };
    assert_eq!(
        named(at(3.0, 1.5)),
        Some(Resource::panel_chip("panel-1", "region"))
    );
    assert_eq!(
        named(at(3.0, 0.5)),
        Some(Resource::panel_at("panel-1", None))
    );
}

#[test]
fn scanning_a_chart_s_plot_reads_it_over_the_chart_and_the_words_say_when_it_was_pointed_at() {
    use tomori_protocol::{Panel, Place, Pose, Resource, ServerMessage};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.projection = true));
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let chart = crate::chart::fixtures::admissions();
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![
                    Panel::new(
                        "panel-1",
                        "Root admissions",
                        chart.summary(),
                        Place::Free {
                            pose: Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array()),
                        },
                    )
                    .with_chart(chart),
                ]),
        ),
    }));
    conversation.take_actions();
    let mut step = Step::default();
    let now = Instant::now();
    // From the left of the floating chart to its right, a little below its middle, through its plot.
    let (width, _) = crate::projection::fitted_size(
        conversation
            .projection()
            .and_then(|projection| projection.panel("panel-1"))
            .unwrap(),
        crate::projection::Fit::Text,
    );
    let mut named = Vec::new();
    let mut read = Vec::new();
    for across in 0..=40 {
        let x = width * (across as f32 / 40.0 - 0.5) * 0.98;
        let at = ahead + Vec3::new(x, -20.0 * crate::projection::PIXEL, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        // Its readout over the chart, its time and value, in place of the label on the ray.
        let readout = frame
            .projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .find(|canvas| canvas.background[3] == 0.0 && !canvas.labels.is_empty())
            .map(|canvas| {
                canvas
                    .labels
                    .iter()
                    .map(|label| crate::ui_text::plain(&label.cells))
                    .collect::<Vec<_>>()
            });
        let labelled = frame
            .projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .any(|canvas| canvas.plain().starts_with("Root admissions") && canvas.rows.len() <= 4);
        assert_ne!(readout.is_some(), labelled);
        read.extend(readout);
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        named.extend(utterance.attention.first().cloned());
    }
    // Left of the plot, where its values are labelled, the panel alone; across it, each time later than the last.
    assert_eq!(named[0], Resource::panel("panel-1"));
    let acrosses: Vec<u16> = named
        .iter()
        .filter_map(|resource| {
            let Resource::Panel { across, .. } = resource else {
                unreachable!("only the panel is pointed at")
            };
            *across
        })
        .collect();
    assert!(acrosses.len() > 30, "{named:?}");
    assert!(
        acrosses.windows(2).all(|pair| pair[0] < pair[1]),
        "{acrosses:?}"
    );
    assert_eq!(
        named.iter().find(|resource| matches!(
            resource,
            Resource::Panel {
                across: Some(_),
                ..
            }
        )),
        Some(&Resource::chart_at("panel-1", acrosses[0]))
    );
    assert_eq!(read.len(), acrosses.len());
    // From about a quarter of a minute after its oldest point to about a quarter of a minute before now.
    assert_eq!(read[0][0], "-14 min 55 s");
    assert_eq!(read[read.len() - 1][0], "-16 s");
    assert!(
        read.iter()
            .all(|lines| lines[1].starts_with("admitted ") && lines[1].ends_with(" a minute"))
    );
}

/// The senses with the ray on each line of the queue [`show_queue`] floats at `ahead`, from the top down: the first
/// that meets each of its rows, `pod7`, `pod112` and `pod9`.
fn queue_rows(step: &mut Step, conversation: &mut Conversation, ahead: Vec3) -> [Senses; 3] {
    let mut found: [Option<Senses>; 3] = [None; 3];
    for step_down in 0..60 {
        let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        step.update(&senses, conversation, &mut felt(), Instant::now(), FRAME);
        if let Some((_, line)) = conversation.pointed_item()
            && found[*line as usize].is_none()
        {
            found[*line as usize] = Some(senses);
        }
    }
    found.map(|senses| senses.expect("each row"))
}

/// The widths of the outlines the queue's face in `frame` draws: the row pointed at's, or the row just chosen's.
fn queue_marks(frame: &Frame<'_>) -> Vec<f32> {
    let face = frame
        .projection
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref())
        .find(|canvas| canvas.plain().starts_with("Queue") && canvas.rows.len() > 4)
        .expect("the queue's face");
    [1.0, 2.0]
        .into_iter()
        .filter(|width| {
            face.shapes.iter().any(|shape| {
                matches!(shape, crate::shapes::Shape::Rect { outline: Some(outline), .. } if outline == width)
            })
        })
        .collect()
}

#[test]
fn a_choosable_row_pointed_at_ticks_as_it_is_entered_and_is_drawn_pointed_at_then_chosen() {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    for (role, marks) in [
        (ConnectionRole::Collaborator, true),
        (ConnectionRole::Visitor, false),
    ] {
        let mut conversation = lobby(role);
        show_queue(&mut conversation, ahead, arrival::HOME);
        let mut step = Step::default();
        let [unchoosable, choosable, other] = queue_rows(&mut step, &mut conversation, ahead);
        let now = Instant::now();
        let mut feedback = felt();
        // Whether a frame pointing as `senses` says ticked, and the queue's marks it drew.
        // Each a tenth of a second after the last, so that hovering ticks as often as it may.
        let frames = std::cell::Cell::new(0);
        let ticks = |senses: &Senses,
                     step: &mut Step,
                     conversation: &mut Conversation,
                     feedback: &mut Feedback| {
            frames.set(frames.get() + 1);
            let now = now + Duration::from_millis(100 * frames.get());
            let frame = step.update(senses, conversation, feedback, now, FRAME);
            // Pointing at the queue, the ray's label names it.
            assert!(label_hint(&frame.projection).is_some());
            let marks = queue_marks(&frame);
            (!felt_on(feedback, Side::Right, now).is_empty(), marks)
        };
        ticks(&unchoosable, &mut step, &mut conversation, &mut feedback);
        // Entering a choosable row ticks, for a participant who may choose it; staying on it does not; and the next
        // frame draws it pointed at.
        let (felt, _) = ticks(&choosable, &mut step, &mut conversation, &mut feedback);
        assert_eq!(felt, marks, "{role:?}");
        let (felt, drawn) = ticks(&choosable, &mut step, &mut conversation, &mut feedback);
        assert!(!felt);
        let pointed: &[f32] = if marks { &[1.0] } else { &[] };
        assert_eq!(drawn, pointed, "{role:?}");
        let (felt, _) = ticks(&other, &mut step, &mut conversation, &mut feedback);
        assert_eq!(felt, marks, "{role:?}");
        // Leaving a row for one not choosable, or for the panel's metadata above its rows, on the same panel, ticks
        // nothing; entering a choosable row again does.
        let above = {
            let ray = unchoosable.ray.expect("a ray");
            let at = ray.origin + ray.direction * (2.0 / -ray.direction.z) + Vec3::Y * 0.03;
            Senses {
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..unchoosable
            }
        };
        let (felt, _) = ticks(&unchoosable, &mut step, &mut conversation, &mut feedback);
        assert!(!felt, "{role:?}");
        let (felt, _) = ticks(&above, &mut step, &mut conversation, &mut feedback);
        assert!(!felt, "{role:?}");
        let (felt, _) = ticks(&choosable, &mut step, &mut conversation, &mut feedback);
        assert_eq!(felt, marks, "{role:?}");
        if !marks {
            continue;
        }
        // Chosen, it is drawn with the brighter, thicker outline, for as long as the label says so.
        step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
        ticks(&choosable, &mut step, &mut conversation, &mut feedback);
        let (_, drawn) = ticks(&choosable, &mut step, &mut conversation, &mut feedback);
        assert_eq!(drawn, [2.0]);
    }
}

#[test]
fn pointed_at_on_a_row_of_an_environment_s_list_the_words_name_the_row_by_its_id() {
    use tomori_protocol::Resource;
    let mut conversation = lobby(ConnectionRole::Owner);
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    show_queue(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let mut named: Vec<Resource> = Vec::new();
    for step_down in 0..60 {
        let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            FRAME,
        );
        conversation.talk(true);
        conversation.talk(false);
        // Recording begins, and ends, attending to what is pointed at.
        let attended: Vec<_> = conversation
            .take_actions()
            .into_iter()
            .filter_map(|action| match action {
                Action::BeginUtterance(utterance) => Some(utterance.attention),
                _ => None,
            })
            .collect();
        assert_eq!(attended.len(), 1);
        if let Some(attended) = attended[0].first()
            && named.last() != Some(attended)
        {
            named.push(attended.clone());
        }
    }
    assert_eq!(
        named,
        [
            Resource::panel("panel-2"),
            Resource::panel_row("panel-2", 0, "pod7"),
            Resource::panel_row("panel-2", 1, "pod112"),
            Resource::panel_row("panel-2", 2, "pod9"),
            Resource::panel("panel-2"),
        ]
    );
}

#[test]
fn a_row_of_a_list_waiting_for_its_layout_is_neither_named_nor_chosen() {
    use tomori_protocol::{Panel, Place, Pose};
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Collaborator);
    show_queue(&mut conversation, ahead, arrival::HOME);
    let queue = conversation
        .projection()
        .unwrap()
        .panel("panel-2")
        .unwrap()
        .clone();
    // Two other panels, beside it, and the queue, all changed together, the queue last.
    let shown = |version: u64| {
        let other = |index: usize| {
            let at = arrival::HOME + Vec3::new(3.0 + index as f32, 0.0, -2.0);
            Panel::new(
                format!("other-{index}"),
                format!("Other {index}"),
                format!("- at {version}"),
                Place::Free {
                    pose: Pose::upright_facing(at.to_array(), arrival::HOME.to_array()),
                },
            )
            .with_version(version)
        };
        tomori_client::Event::Message(tomori_protocol::ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(vec![tomori_wall()])
                    .with_panels(vec![
                        other(0),
                        other(1),
                        queue.clone().with_version(version),
                    ]),
            ),
        })
    };
    conversation.apply(shown(6));
    let mut step = Step::default();
    let row = queue_rows(&mut step, &mut conversation, ahead)[1];
    let now = Instant::now();
    step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    assert!(conversation.pointed_row().is_some());
    // Changed, the queue waits its turn behind the others: its row is not pointed at, named or chosen.
    conversation.apply(shown(7));
    step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(conversation.pointed_item(), None);
    assert_eq!(conversation.pointed_row(), None);
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    assert!(conversation.take_actions().is_empty());
    // Its turn come, it is chosen as it is now.
    for _ in 0..4 {
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    }
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    assert_eq!(
        conversation.take_actions(),
        [Action::Send(
            tomori_protocol::ClientMessage::ChoosePanelRow {
                panel: "panel-2".into(),
                row: "pod112".into(),
                revision: Some(7),
                key: None,
            }
        )]
    );
}

#[test]
fn a_or_the_trigger_on_a_choosable_row_chooses_it_once_for_its_environment() {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Collaborator);
    show_queue(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let rows = queue_rows(&mut step, &mut conversation, ahead);
    let [unchoosable, choosable, other] = rows;
    let now = Instant::now();
    let choose = |row: &str| {
        Action::Send(tomori_protocol::ClientMessage::ChoosePanelRow {
            panel: "panel-2".into(),
            row: row.into(),
            revision: Some(6),
            key: None,
        })
    };
    let hint = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
        let output = step.update(senses, conversation, &mut felt(), now, FRAME);
        label_hint(&output.projection)
    };
    // On a choosable row, the label says A chooses it; on one that is not, it says nothing of choosing.
    assert_eq!(
        hint(&mut step, &mut conversation, &choosable).as_deref(),
        Some("A  choose")
    );
    // A chooses it once, with the choosing cue, however many frames it is held, or skipped, before it is let go.
    let mut feedback = felt();
    step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
    assert!(chose(&mut feedback));
    assert_eq!(conversation.take_actions(), [choose("pod112")]);
    for _ in 0..3 {
        step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
    }
    step.update(&choosable, &mut conversation, &mut felt(), now, FRAME);
    assert!(conversation.take_actions().is_empty());
    // Just chosen, the label says so.
    assert_eq!(
        hint(&mut step, &mut conversation, &choosable).as_deref(),
        Some("✓  chosen")
    );
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    // The trigger, pulled on another choosable row, chooses it once pressed there.
    assert_eq!(
        hint(&mut step, &mut conversation, &other).as_deref(),
        Some("A  choose")
    );
    let mut feedback = felt();
    let pulled = Senses {
        trigger: Some(1.0),
        attractor: Some(arrival::HOME),
        ..other
    };
    step.update(&pulled, &mut conversation, &mut feedback, now, FRAME);
    assert!(chose(&mut feedback));
    assert_eq!(conversation.take_actions(), [choose("pod9")]);
    for _ in 0..3 {
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    }
    assert!(conversation.take_actions().is_empty());
    step.update(&other, &mut conversation, &mut felt(), now, FRAME);
    // On a row that may not be chosen, A does nothing new, nor does the trigger, as anywhere on a panel in a space.
    let shown = hint(&mut step, &mut conversation, &unchoosable);
    assert!(!shown.unwrap_or_default().contains("choose"));
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    assert!(conversation.take_actions().is_empty());
    let pulled = Senses {
        trigger: Some(1.0),
        attractor: Some(arrival::HOME),
        ..unchoosable
    };
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    assert!(conversation.take_actions().is_empty());
}

#[test]
fn the_trigger_on_a_choosable_row_decides_no_request_and_a_does_not_choose_while_one_waits() {
    use tomori_protocol::{Approval, ApprovalOption};
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Collaborator);
    show_queue(&mut conversation, ahead, arrival::HOME);
    conversation.apply(space(|state| {
        state.projection = true;
        state.choices = true;
        state.approval = Some(Approval {
            id: "request-1".into(),
            title: "Bash".into(),
            text: "cargo test".into(),
            options: vec![ApprovalOption {
                id: "once".into(),
                label: "Allow once".into(),
            }],
        });
    }));
    conversation.settle_request();
    conversation.take_actions();
    let mut step = Step::default();
    let row = queue_rows(&mut step, &mut conversation, ahead)[1];
    let now = Instant::now();
    step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    // The trigger never decides a request, so a pull on the row chooses it, and the request still waits.
    step.update(
        &Senses {
            trigger: Some(1.0),
            ..row
        },
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(
        conversation.take_actions(),
        [Action::Send(
            tomori_protocol::ClientMessage::ChoosePanelRow {
                panel: "panel-2".into(),
                row: "pod112".into(),
                revision: Some(6),
                key: None,
            }
        )]
    );
    assert!(conversation.is_choosing());
    step.update(&row, &mut conversation, &mut felt(), now, FRAME);
    // A, on the row, decides the request rather than choose the row.
    step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
    let actions = conversation.take_actions();
    assert!(matches!(
        actions[..],
        [Action::Send(tomori_protocol::ClientMessage::Decide { .. })]
    ));
}

#[test]
fn a_visitor_s_a_or_trigger_on_a_choosable_row_chooses_nothing() {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Visitor);
    show_queue(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let rows = queue_rows(&mut step, &mut conversation, ahead);
    let choosable = rows[1];
    let now = Instant::now();
    let output = step.update(&choosable, &mut conversation, &mut felt(), now, FRAME);
    assert!(
        !label_hint(&output.projection)
            .unwrap_or_default()
            .contains("choose")
    );
    let mut feedback = felt();
    step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
    assert!(!chose(&mut feedback));
    assert!(conversation.a_refused());
    assert!(conversation.take_actions().is_empty());
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    let pulled = Senses {
        trigger: Some(1.0),
        attractor: Some(arrival::HOME),
        ..choosable
    };
    step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
    assert!(conversation.take_actions().is_empty());
}

/// Where, scanning down the agenda [`show_checklist`] floats at `ahead`, the ray first meets a row whose label says
/// `hint`, and the label there.
fn ray_on_item(
    step: &mut Step,
    conversation: &mut Conversation,
    ahead: Vec3,
    hint: &str,
) -> Option<Senses> {
    (0..60).find_map(|step_down| {
        let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        let output = step.update(&senses, conversation, &mut felt(), Instant::now(), FRAME);
        (label_hint(&output.projection).as_deref() == Some(hint)).then_some(senses)
    })
}

#[test]
fn a_pointing_at_a_checklist_item_ticks_or_unticks_it_for_everyone_with_a_hint_on_the_label() {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Collaborator);
    show_checklist(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let tick = |line: u32, checked: bool, revision: u64| {
        Action::Send(tomori_protocol::ClientMessage::CheckPanelItem {
            panel: "panel-1".into(),
            line,
            checked,
            revision: Some(revision),
        })
    };
    // Above the list, on the panel's title, the label says nothing of ticking.
    let hint = (0..60).find_map(|step_down| {
        let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        label_hint(&output.projection)
    });
    assert!(!hint.expect("the label").contains("tick"));
    // A elsewhere on the panel does nothing new.
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert!(conversation.take_actions().is_empty());
    // On the ticked item, the label says A unticks it, and A does, for everyone, with the choosing cue.
    ray_on_item(&mut step, &mut conversation, ahead, "A  untick").expect("the ticked item");
    earcons(&mut feedback);
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    // It says the revision the server sent, which its own ticks shown at once do not change.
    assert_eq!(conversation.take_actions(), [tick(2, false, 4)]);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    // Shown at once, so the label on it now says A ticks it, and A does.
    let senses = ray_on_item(&mut step, &mut conversation, ahead, "A  tick").expect("unticked");
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert_eq!(conversation.take_actions(), [tick(2, true, 4)]);
    // On the item below, not ticked, A ticks it.
    ray_on_item(&mut step, &mut conversation, ahead, "A  tick").expect("the second item");
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert_eq!(conversation.take_actions(), [tick(3, true, 4)]);
    // The right wrist's transcript comes first: while it shows, A is its, and ticks nothing.
    let wrist = Senses {
        right_palm: Some(palm_up(0.0)),
        ..senses
    };
    step.update(&wrist, &mut conversation, &mut feedback, now, FRAME);
    step.update(&wrist, &mut conversation, &mut feedback, now + HOLD, FRAME);
    assert!(step.wrist.state().active());
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now + HOLD,
    );
    assert!(conversation.take_actions().is_empty());
    // Carrying the panel, A ticks nothing on it.
    let mut step = Step::default();
    let mut carrying =
        ray_on_item(&mut step, &mut conversation, ahead, "A  untick").expect("the item ticked");
    carrying.grip = Some(0.0);
    step.update(&carrying, &mut conversation, &mut feedback, now, FRAME);
    carrying.grip = Some(1.0);
    step.update(&carrying, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(step.projector.held(), Some("panel-1"));
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert!(conversation.take_actions().is_empty());
}

#[test]
fn a_visitor_s_label_offers_no_ticking_and_a_ticks_nothing() {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Visitor);
    show_checklist(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let mut feedback = felt();
    assert!(ray_on_item(&mut step, &mut conversation, ahead, "A  tick").is_none());
    assert!(ray_on_item(&mut step, &mut conversation, ahead, "A  untick").is_none());
    // Pointing down the list, A is refused, without the choosing cue.
    for step_down in 0..60 {
        let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
        let senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, at - arrival::HOME),
            ..senses()
        };
        let now = Instant::now();
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        earcons(&mut feedback);
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert!(conversation.take_actions().is_empty());
        assert!(!earcons(&mut feedback).contains(&Earcon::Choose));
    }
}

#[test]
fn signs_face_whoever_walks_about_them_and_a_worker_pointed_at_from_aside_is_attended_to() {
    use tomori_protocol::Resource;
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| {
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
    }));
    conversation.take_actions();
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ..senses()
    };
    for _ in 0..200 {
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    }
    let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    let worker = Vec3::from_array(output.stations[0].position);
    // Signs face this participant, not whoever the agent faces.
    assert_eq!(output.sign_facing, Some(arrival::HOME));
    // From in front, 45° to its left and 70° to its right, about the worker, as far from it as home is: the signs
    // turn to face them there, and the ray from there at the worker names it as the words begin.
    let from_home = (arrival::HOME - worker).with_y(0.0);
    for degrees in [0.0_f32, 45.0, -70.0] {
        let eyes = worker
            + glam::Quat::from_rotation_y(degrees.to_radians()) * from_home
            + Vec3::Y * (arrival::HOME.y - worker.y);
        let head = Mat4::from_translation(eyes);
        senses.head = Some(head);
        senses.head_pose = Pose::from_transform(head);
        senses.viewer = Some(eyes);
        senses.ray = Ray::new(eyes, worker - eyes);
        let mut facing = None;
        for _ in 0..72 {
            facing = step
                .update(&senses, &mut conversation, &mut feedback, now, FRAME)
                .sign_facing;
        }
        let facing = facing.expect("signs face the viewer");
        assert!(
            facing.distance(eyes)
                <= crate::sign_facing::DEAD_ZONE + crate::sign_facing::SETTLE + 1e-4,
            "{degrees}: {facing} {eyes}"
        );
        conversation.take_actions();
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        assert_eq!(
            utterance.attention,
            [Resource::worker("ses_a")],
            "{degrees}"
        );
    }
    // Without a viewer, as a spectator's, they face whoever the agent faces.
    senses.viewer = None;
    let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(output.sign_facing, None);
}

#[test]
fn the_ray_names_a_panel_and_the_grip_pulls_it_off_unless_a_card_is_nearer() {
    use tomori_protocol::{
        Card, CardKind, Cards, Checks, Panel, Place, Pose, Resource, ServerMessage,
    };
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| {
        state.cards = true;
        state.projection = true;
    }));
    let facing_home = |at: Vec3| Pose::upright_facing(at.to_array(), arrival::HOME.to_array());
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -3.0);
    let project = |conversation: &mut Conversation| {
        conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(vec![tomori_wall()])
                    .with_panels(vec![Panel::new(
                        "panel-1",
                        "Build status",
                        "Green.",
                        Place::Free {
                            pose: facing_home(ahead),
                        },
                    )]),
            ),
        }));
        conversation.take_actions();
    };
    project(&mut conversation);
    let mut step = Step::default();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ray: Ray::new(arrival::HOME, -Vec3::Z),
        ..senses()
    };
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    // The Tomori wall's six slots and its label, the panel, and the label on the ray naming it.
    let wall = tomori_protocol::WALL_SLOTS as usize + 1;
    assert_eq!(output.projection.len(), wall + 2);
    let label = output.projection[wall + 1]
        .canvas
        .as_deref()
        .unwrap()
        .plain();
    assert!(label.starts_with("Build status"), "{label}");
    let beam = output
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert!(beam.hit && beam.end.distance(ahead) < 1e-3);
    let attending = |conversation: &mut Conversation| {
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        utterance.attention.clone()
    };
    assert_eq!(attending(&mut conversation), [Resource::panel("panel-1")]);
    // A visitor's squeeze pulls nothing off.
    let role = |role| tomori_client::Event::Message(ServerMessage::Role { role });
    conversation.apply(role(ConnectionRole::Visitor));
    senses.grip = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.projector.held(), None);
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    conversation.apply(role(ConnectionRole::Owner));
    conversation.take_actions();
    // Squeezed, it is pulled off and carried; let go, it floats where it was left, for everyone.
    senses.grip = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.projector.held(), Some("panel-1"));
    senses.ray = Ray::new(arrival::HOME, Vec3::new(-0.5, 0.0, -1.0));
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    // No label on a panel carried, but the hint beside it, last, over everything else.
    assert_eq!(output.projection.len(), wall + 2);
    let hint = output.projection[wall + 1]
        .canvas
        .as_deref()
        .unwrap()
        .plain();
    assert!(hint.contains("throw away, for all"), "{hint}");
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(matches!(
        conversation.take_actions().as_slice(),
        [Action::Send(tomori_protocol::ClientMessage::PlacePanel { panel, wall: None, slot: None, pose: Some(_), .. })] if panel == "panel-1"
    ));
    // A card nearer on the ray is pointed at and picked up instead.
    project(&mut conversation);
    conversation.apply(tomori_client::Event::Message(ServerMessage::Cards {
        cards: Box::new(Cards {
            space: SPACE.into(),
            source: "o/r".into(),
            cards: vec![Card {
                id: "github://o/r/issues/1".into(),
                number: 1,
                kind: CardKind::Issue,
                title: "A card".into(),
                author: "ada".into(),
                labels: Vec::new(),
                checks: Checks::None,
                age: 0,
                placement: Some(facing_home(arrival::HOME - Vec3::Z * 1.5)),
            }],
            status: None,
        }),
    }));
    let mut step = Step::default();
    senses.ray = Ray::new(arrival::HOME, -Vec3::Z);
    step.update(
        &senses,
        &mut conversation,
        &mut felt(),
        now + crate::projection::PENDING,
        FRAME,
    );
    assert_eq!(
        attending(&mut conversation),
        [Resource::card("github://o/r/issues/1")]
    );
    senses.grip = Some(1.0);
    step.update(
        &senses,
        &mut conversation,
        &mut felt(),
        now + crate::projection::PENDING,
        FRAME,
    );
    assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
    assert_eq!(step.projector.held(), None);
}

#[test]
fn the_ray_points_at_the_nearer_of_a_card_and_a_worker_and_the_grip_carries_a_card() {
    use tomori_protocol::{Card, CardKind, Cards, Checks, Resource, ServerMessage};
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| {
        state.cards = true;
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
    }));
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        ..senses()
    };
    let mut frame = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
        step.update(senses, conversation, &mut feedback, now, FRAME)
            .stations
            .iter()
            .map(|station| Vec3::from_array(station.position))
            .collect::<Vec<_>>()
    };
    let worker = frame(&mut step, &mut conversation, &senses)[0];
    // Before the cards arrive, there are none to draw.
    assert!(
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
            .cards
            .is_empty()
    );
    // One card stands halfway to the worker, facing the eye, and another beyond it.
    let between = arrival::HOME.lerp(worker, 0.5);
    let beyond = arrival::HOME.lerp(worker, 1.5);
    let placed = |at: Vec3| {
        let model = crate::recording::facing(at, arrival::HOME, 1.0);
        let (_, rotation, _) = model.to_scale_rotation_translation();
        tomori_protocol::Pose {
            position: at.to_array(),
            orientation: rotation.to_array(),
        }
    };
    let card = |number: u64, at: Vec3| Card {
        id: format!("github://o/r/issues/{number}"),
        number,
        kind: CardKind::Issue,
        title: "A card".into(),
        author: "ada".into(),
        labels: Vec::new(),
        checks: Checks::None,
        age: 0,
        placement: Some(placed(at)),
    };
    let show = |conversation: &mut Conversation, at: Vec3| {
        conversation.apply(tomori_client::Event::Message(ServerMessage::Cards {
            cards: Box::new(Cards {
                space: SPACE.into(),
                source: "o/r".into(),
                cards: vec![card(1, at)],
                status: None,
            }),
        }));
    };
    show(&mut conversation, between);
    conversation.take_actions();
    senses.ray = Ray::new(arrival::HOME, worker - arrival::HOME);
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(output.cards.len(), 2, "the inbox and the card");
    let beam = output
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert!(beam.hit && beam.end.distance(between) < 1e-3);
    // What X begins attending to as it is pressed.
    let attending = |conversation: &mut Conversation| {
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        utterance.attention.clone()
    };
    let card_pointed = Resource::card("github://o/r/issues/1");
    assert_eq!(
        attending(&mut conversation),
        std::slice::from_ref(&card_pointed)
    );
    // Beyond the worker, the worker is pointed at.
    show(&mut conversation, beyond);
    frame(&mut step, &mut conversation, &senses);
    assert_eq!(attending(&mut conversation), [Resource::worker("ses_a")]);
    // And the ray ends on the worker, the nearer.
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let beam = output
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    let met = step.delegation_scene.hit_point().unwrap();
    assert!(beam.hit && beam.end.distance(met) < 1e-3);
    assert!(beam.end.distance(arrival::HOME) < beyond.distance(arrival::HOME));
    // Back between, the grip picks it up and, carried, it is what is pointed at, wherever the ray goes.
    show(&mut conversation, between);
    frame(&mut step, &mut conversation, &senses);
    senses.grip = Some(1.0);
    frame(&mut step, &mut conversation, &senses);
    assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
    // In an environment that allows travel, the left stick does not aim while a card is carried, which would put it down.
    conversation.set_locomotion(vec![(
        crate::test_support::SKY,
        crate::locomotion::Locomotion::teleport(vec![crate::locomotion::Surface::disc(
            Vec3::ZERO,
            20.0,
        )]),
    )]);
    // Once the scenery has grown in, where travel could otherwise aim.
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    }
    assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
    conversation.take_actions();
    step.buttons(
        &press(|buttons| buttons.left_stick = Vec2::Y),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(output.landing, None);
    assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
    assert!(conversation.take_actions().is_empty(), "nothing put down");
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    conversation.set_locomotion(Vec::new());
    senses.ray = Ray::new(arrival::HOME, Vec3::new(0.5, 0.0, -1.0));
    let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let beam = output
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.ray)
        .unwrap();
    assert!(
        beam.end
            .distance(senses.ray.unwrap().at(between.distance(arrival::HOME)))
            < 1e-3
    );
    assert_eq!(attending(&mut conversation), [card_pointed]);
    // Letting go puts it down there, for everyone in the space.
    senses.grip = Some(0.0);
    frame(&mut step, &mut conversation, &senses);
    let actions = conversation.take_actions();
    let [Action::Send(tomori_protocol::ClientMessage::PlaceCard { card, placement })] =
        actions.as_slice()
    else {
        unreachable!("one card put down, not {actions:?}");
    };
    assert_eq!(card, "github://o/r/issues/1");
    assert!(placement.is_some());
}

#[test]
fn labels_face_the_viewer_in_steps_of_five_centimetres() {
    let mut conversation = joined();
    let mut step = Step::default();
    let start = Instant::now();
    let swaying = Senses {
        left_palm: Some(left_palm_up(0.0)),
        viewer: Some(arrival::HOME + Vec3::new(0.012, -0.013, 0.031)),
        ..senses()
    };
    step.update(&swaying, &mut conversation, &mut felt(), start, FRAME);
    let frame = step.update(
        &swaying,
        &mut conversation,
        &mut felt(),
        start + HOLD,
        FRAME,
    );
    let viewer = frame.arrival.expect("the menu").scene.viewer.unwrap();
    assert!(
        viewer.abs_diff_eq(Vec3::new(0.0, 1.6, 0.05), 1e-6),
        "{viewer}"
    );
    // The desktop's viewer, at home, is already on the grid, so it is unchanged.
    assert_eq!((arrival::HOME * 20.0).round() / 20.0, arrival::HOME);
}

#[test]
fn a_stick_push_navigates_once_until_it_returns_and_never_in_a_space() {
    let mut conversation = Conversation::new("Ada");
    conversation.set_servers(
        ["10.0.0.1:8765", "10.0.0.2:8765", "10.0.0.3:8765"]
            .map(|address| {
                tomori_client::server_choice::Choice::new(
                    address,
                    address,
                    tomori_client::server_choice::Source::Discovered,
                )
            })
            .to_vec(),
    );
    let mut feedback = felt();
    let mut step = Step::default();
    let now = Instant::now();
    let down = press(|buttons| buttons.left_stick = Vec2::new(0.0, -1.0));
    let before = conversation.text();
    step.buttons(&down, true, &mut conversation, &mut feedback, now);
    let after = conversation.text();
    assert_ne!(before, after, "a push moves");
    step.buttons(&down, true, &mut conversation, &mut feedback, now);
    assert_eq!(conversation.text(), after, "holding it does not repeat");
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    step.buttons(
        &press(|buttons| buttons.left_stick = Vec2::new(0.0, 1.0)),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert_eq!(conversation.text(), before, "pushing back moves back");
    // Sideways, it moves between pages rather than rows, and all three servers are on one page.
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    for x in [1.0, -1.0] {
        step.buttons(
            &press(|buttons| buttons.left_stick = Vec2::new(x, 0.2)),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
    }
    assert_eq!(conversation.text(), before);
    let mut conversation = joined();
    let before = conversation.text();
    step.buttons(&down, true, &mut conversation, &mut feedback, now);
    assert_eq!(conversation.text(), before);
}

#[test]
fn holding_b_in_a_space_ramps_then_leaves_once_the_hold_is_long_enough() {
    let mut conversation = joined();
    let mut feedback = felt();
    let mut step = Step::default();
    let start = Instant::now();
    let held = press(|buttons| buttons.back_held = true);
    step.buttons(
        &press(|buttons| {
            buttons.back = true;
            buttons.back_held = true;
        }),
        true,
        &mut conversation,
        &mut feedback,
        start,
    );
    step.buttons(
        &held,
        true,
        &mut conversation,
        &mut feedback,
        start + LEAVE_HOLD / 2,
    );
    // The right controller ramps from light to firm as it is held.
    let ramp = felt_on(&mut feedback, Side::Right, start + LEAVE_HOLD / 2);
    assert!(
        matches!(ramp[..], [Haptic::Vibrate { amplitude, .. }] if (amplitude - 0.35).abs() < 0.01),
        "{ramp:?}"
    );
    assert!(conversation.joined_space().is_some());
    step.buttons(
        &held,
        true,
        &mut conversation,
        &mut feedback,
        start + LEAVE_HOLD,
    );
    assert!(earcons(&mut feedback).contains(&Earcon::Leave));
    assert!(
        conversation
            .take_actions()
            .iter()
            .any(|action| matches!(action, Action::Send(tomori_protocol::ClientMessage::Leave)))
    );
}

#[test]
fn letting_go_of_b_early_stays_and_stops_the_ramp() {
    let mut conversation = joined();
    let mut feedback = felt();
    let mut step = Step::default();
    let start = Instant::now();
    step.buttons(
        &press(|buttons| {
            buttons.back = true;
            buttons.back_held = true;
        }),
        true,
        &mut conversation,
        &mut feedback,
        start,
    );
    let half = start + LEAVE_HOLD / 2;
    step.buttons(
        &press(|buttons| buttons.back_held = true),
        true,
        &mut conversation,
        &mut feedback,
        half,
    );
    assert!(!felt_on(&mut feedback, Side::Right, half).is_empty());
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut feedback,
        half,
    );
    assert_eq!(felt_on(&mut feedback, Side::Right, half), [Haptic::Stop]);
    step.buttons(
        &press(|buttons| buttons.back_held = true),
        true,
        &mut conversation,
        &mut feedback,
        start + LEAVE_HOLD * 2,
    );
    assert!(conversation.joined_space().is_some());
}

#[test]
fn b_held_from_before_joining_never_leaves() {
    let mut conversation = lobby(ConnectionRole::Owner);
    let mut feedback = felt();
    let mut step = Step::default();
    let start = Instant::now();
    step.buttons(
        &press(|buttons| {
            buttons.back = true;
            buttons.back_held = true;
        }),
        true,
        &mut conversation,
        &mut feedback,
        start,
    );
    assert!(step.back_since.is_none());
    // Held on into a space, it never leaves it.
    let mut conversation = joined();
    for seconds in 1..4 {
        step.buttons(
            &press(|buttons| buttons.back_held = true),
            true,
            &mut conversation,
            &mut feedback,
            start + Duration::from_secs(seconds),
        );
    }
    assert!(conversation.joined_space().is_some());
}

#[test]
fn x_talks_while_held_and_after_focus_loss_only_once_released() {
    let mut conversation = joined();
    let mut feedback = felt();
    let mut step = Step::default();
    let now = Instant::now();
    let talk = press(|buttons| buttons.talk = true);
    step.buttons(&talk, true, &mut conversation, &mut feedback, now);
    assert!(conversation.recording().is_some());
    step.blur(&mut conversation);
    assert!(conversation.recording().is_none());
    // Still held through the system menu, it does not record again.
    step.buttons(&talk, true, &mut conversation, &mut feedback, now);
    assert!(conversation.recording().is_none());
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    step.buttons(&talk, true, &mut conversation, &mut feedback, now);
    assert!(conversation.recording().is_some());
    // Without focus, X does nothing.
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    let mut unfocused = Step::default();
    let mut conversation = joined();
    unfocused.buttons(&talk, false, &mut conversation, &mut feedback, now);
    assert!(conversation.recording().is_none());
}

#[test]
fn y_pins_the_menu_in_a_space_until_b_and_lists_again_elsewhere() {
    let mut conversation = joined();
    let mut feedback = felt();
    let mut step = Step::default();
    let now = Instant::now();
    let y = press(|buttons| buttons.y = true);
    step.buttons(&y, true, &mut conversation, &mut feedback, now);
    assert!(step.pinned());
    assert!(
        !felt_on(&mut feedback, Side::Left, now).is_empty(),
        "pinning is felt"
    );
    // B lets go of it before anything else, so the space is not backed out of or left.
    step.buttons(
        &press(|buttons| {
            buttons.back = true;
            buttons.back_held = true;
        }),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert!(!step.pinned());
    step.buttons(&y, true, &mut conversation, &mut feedback, now);
    step.buttons(&y, true, &mut conversation, &mut feedback, now);
    assert!(!step.pinned(), "Y again lets go");
    assert!(felt_on(&mut feedback, Side::Left, now).len() == 1);
    // Leaving the space lets go of a pinned menu.
    step.buttons(&y, true, &mut conversation, &mut feedback, now);
    let mut lobby = lobby(ConnectionRole::Owner);
    step.buttons(&Buttons::default(), true, &mut lobby, &mut feedback, now);
    assert!(!step.pinned());
    // Outside a space, Y lists the sessions again.
    step.buttons(&y, true, &mut lobby, &mut feedback, now);
    assert!(!step.pinned());
    assert!(!lobby.take_actions().is_empty());
}

#[test]
fn mute_and_replay_reach_the_conversation() {
    let mut conversation = joined();
    let mut feedback = felt();
    let mut step = Step::default();
    let now = Instant::now();
    let muted = |conversation: &Conversation| conversation.menu_model().muted;
    let before = muted(&conversation);
    step.buttons(
        &press(|buttons| buttons.mute = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert_ne!(muted(&conversation), before);
    step.buttons(
        &press(|buttons| buttons.replay = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    // This server does not say what it speaks, so it cannot play a reply again, and says so.
    assert_eq!(
        conversation.menu_model().notice.as_deref(),
        Some("This server cannot play replies again.")
    );
}

#[test]
fn the_left_palm_within_45_degrees_of_up_shows_the_menu_after_the_dwell_until_past_55() {
    let mut conversation = joined();
    let mut feedback = felt();
    let start = Instant::now();
    let at = |degrees| Senses {
        left_palm: Some(left_palm_up(degrees)),
        ..senses()
    };
    let mut step = Step::default();
    let frame = step.update(&at(40.0), &mut conversation, &mut feedback, start, FRAME);
    assert!(frame.arrival.is_none(), "not before the dwell");
    let frame = step.update(
        &at(40.0),
        &mut conversation,
        &mut feedback,
        start + HOLD / 2,
        FRAME,
    );
    assert!(frame.arrival.is_none(), "not halfway through it");
    let frame = step.update(
        &at(40.0),
        &mut conversation,
        &mut feedback,
        start + HOLD,
        FRAME,
    );
    let arrival = frame.arrival.expect("the menu");
    assert!(!arrival.scene.objects.is_empty());
    assert_eq!(arrival.scene.viewer, Some(arrival::HOME));
    // The pill hides while a hand's panel shows.
    assert_eq!(frame.panels.pill, Sheet::default());
    // Between the angles it stays, and past 55° it hides.
    let frame = step.update(
        &at(50.0),
        &mut conversation,
        &mut feedback,
        start + HOLD * 2,
        FRAME,
    );
    assert!(frame.arrival.is_some());
    let frame = step.update(
        &at(60.0),
        &mut conversation,
        &mut feedback,
        start + HOLD * 3,
        FRAME,
    );
    assert!(frame.arrival.is_none());
    // 50° from hidden never shows it.
    let mut step = Step::default();
    step.update(&at(50.0), &mut conversation, &mut feedback, start, FRAME);
    let frame = step.update(
        &at(50.0),
        &mut conversation,
        &mut feedback,
        start + HOLD,
        FRAME,
    );
    assert!(frame.arrival.is_none());
    // Without focus it never shows.
    let unfocused = Senses {
        focused: false,
        ..at(0.0)
    };
    let mut step = Step::default();
    step.update(&unfocused, &mut conversation, &mut feedback, start, FRAME);
    let frame = step.update(
        &unfocused,
        &mut conversation,
        &mut feedback,
        start + HOLD,
        FRAME,
    );
    assert!(frame.arrival.is_none());
}

#[test]
fn turning_the_head_does_not_change_whether_either_palm_shows_its_panel() {
    let mut conversation = joined();
    let start = Instant::now();
    let neck = arrival::HOME - Vec3::new(0.0, 0.075, -0.08);
    for yaw in [-90.0, -45.0, 0.0, 45.0, 90.0, 180.0] {
        let head = crate::hand_menu::head_turned(neck, yaw, -20.0);
        let turned = Senses {
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(crate::hand_menu::head_centre(head)),
            ..senses()
        };
        for (degrees, shown) in [(0.0, true), (40.0, true), (-40.0, true), (90.0, false)] {
            let mut step = Step::default();
            let left = Senses {
                left_palm: Some(left_palm_up(degrees)),
                ..turned
            };
            step.update(&left, &mut conversation, &mut felt(), start, FRAME);
            let frame = step.update(&left, &mut conversation, &mut felt(), start + HOLD, FRAME);
            assert_eq!(frame.arrival.is_some(), shown, "left {yaw} {degrees}");
            let mut step = Step::default();
            let right = Senses {
                right_palm: Some(palm_up(degrees)),
                ..turned
            };
            let wrist = wrist_after(&mut step, &mut conversation, &right, start);
            assert_eq!(wrist.model.is_some(), shown, "right {yaw} {degrees}");
        }
    }
}

/// The transcript's sheet after holding the right palm `degrees` from up, from hidden.
fn wrist_after(
    step: &mut Step,
    conversation: &mut Conversation,
    senses: &Senses,
    start: Instant,
) -> Sheet {
    step.update(senses, conversation, &mut felt(), start, FRAME);
    step.update(senses, conversation, &mut felt(), start + HOLD, FRAME)
        .panels
        .wrist
}

#[test]
fn the_right_palm_within_45_degrees_of_up_shows_the_transcript_after_the_dwell_until_past_55() {
    let mut conversation = joined();
    let start = Instant::now();
    let at = |degrees| Senses {
        right_palm: Some(palm_up(degrees)),
        ..senses()
    };
    let mut step = Step::default();
    let mut feedback = felt();
    let frame = step.update(&at(40.0), &mut conversation, &mut feedback, start, FRAME);
    assert!(frame.panels.wrist.model.is_none(), "not before the dwell");
    let frame = step.update(
        &at(40.0),
        &mut conversation,
        &mut feedback,
        start + HOLD / 2,
        FRAME,
    );
    assert!(frame.panels.wrist.model.is_none(), "not halfway through it");
    let frame = step.update(
        &at(40.0),
        &mut conversation,
        &mut feedback,
        start + HOLD,
        FRAME,
    );
    let shown = frame.panels.wrist.clone();
    assert!(shown.model.is_some() && shown.canvas.is_some());
    // Opening is felt lightly on the right hand.
    assert!(!felt_on(&mut feedback, Side::Right, start + HOLD).is_empty());
    // Between the angles it stays.
    let frame = step.update(
        &at(50.0),
        &mut conversation,
        &mut feedback,
        start + HOLD * 2,
        FRAME,
    );
    assert!(frame.panels.wrist.model.is_some());
    // Past 55° it hides.
    let frame = step.update(
        &at(60.0),
        &mut conversation,
        &mut feedback,
        start + HOLD * 3,
        FRAME,
    );
    assert_eq!(frame.panels.wrist, Sheet::default());
    // 50° from hidden never shows it.
    let mut step = Step::default();
    assert!(
        wrist_after(&mut step, &mut conversation, &at(50.0), start)
            .model
            .is_none()
    );
    // Nor does pointing, nor a palm without focus.
    let mut step = Step::default();
    let pointing = Senses {
        pointing: true,
        ..at(0.0)
    };
    assert!(
        wrist_after(&mut step, &mut conversation, &pointing, start)
            .model
            .is_none()
    );
    let mut step = Step::default();
    let unfocused = Senses {
        focused: false,
        ..at(0.0)
    };
    assert!(
        wrist_after(&mut step, &mut conversation, &unfocused, start)
            .model
            .is_none()
    );
    // Outside a space there is no transcript.
    let mut lobby = lobby(ConnectionRole::Owner);
    let mut step = Step::default();
    assert!(
        wrist_after(&mut step, &mut lobby, &at(0.0), start)
            .model
            .is_none()
    );
}

#[test]
fn whichever_of_the_menu_and_the_transcript_shows_first_wins() {
    let mut conversation = joined();
    let start = Instant::now();
    let both = Senses {
        left_palm: Some(left_palm_up(0.0)),
        right_palm: Some(palm_up(0.0)),
        ..senses()
    };
    // The transcript first: the menu then does not show.
    let mut step = Step::default();
    let transcript = Senses {
        left_palm: None,
        ..both
    };
    wrist_after(&mut step, &mut conversation, &transcript, start);
    step.update(
        &both,
        &mut conversation,
        &mut felt(),
        start + HOLD * 2,
        FRAME,
    );
    let frame = step.update(
        &both,
        &mut conversation,
        &mut felt(),
        start + HOLD * 3,
        FRAME,
    );
    assert!(frame.panels.wrist.model.is_some());
    assert!(frame.arrival.is_none(), "no menu");
    // The menu first: the transcript then does not show.
    let mut step = Step::default();
    let menu = Senses {
        right_palm: None,
        ..both
    };
    step.update(&menu, &mut conversation, &mut felt(), start, FRAME);
    step.update(&menu, &mut conversation, &mut felt(), start + HOLD, FRAME);
    step.update(
        &both,
        &mut conversation,
        &mut felt(),
        start + HOLD * 2,
        FRAME,
    );
    let frame = step.update(
        &both,
        &mut conversation,
        &mut felt(),
        start + HOLD * 3,
        FRAME,
    );
    assert!(frame.panels.wrist.model.is_none());
    assert!(frame.arrival.is_some());
}

#[test]
fn the_attractor_is_drawn_as_each_platform_draws_it() {
    // Outside a space, where the trigger still draws the swarm.
    let mut conversation = lobby(ConnectionRole::Owner);
    let mut feedback = felt();
    let now = Instant::now();
    let at = Vec3::new(0.2, 1.0, -0.5);
    for (marker, idle, pulling) in [
        (Marker::Always, Some(at), Some(at)),
        (Marker::WhileAttracting, None, Some(at)),
        (Marker::Hidden, None, None),
    ] {
        let mut step = Step::default();
        let mut senses = Senses {
            attractor: Some(at),
            marker,
            ..senses()
        };
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(!frame.attracting);
        assert_eq!(frame.marker, idle, "{marker:?}");
        senses.trigger = Some(1.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(frame.attracting, "a pull on nothing attracts");
        assert_eq!(frame.marker, pulling, "{marker:?}");
    }
}

/// The [`ClientMessage::PlaceAgent`]s `conversation` sent since it was last asked.
fn placed(conversation: &mut Conversation) -> Vec<Option<tomori_protocol::Pose>> {
    conversation
        .take_actions()
        .into_iter()
        .filter_map(|action| match action {
            Action::Send(tomori_protocol::ClientMessage::PlaceAgent { at }) => Some(at),
            _ => None,
        })
        .collect()
}

#[test]
fn in_a_space_the_trigger_draws_the_swarm_nowhere_and_summons_nothing_from_any_server() {
    let now = Instant::now();
    let at = Vec3::new(0.8, 1.2, -0.6);
    let pulling = Senses {
        attractor: Some(at),
        trigger: Some(1.0),
        ..senses()
    };
    // On a server that keeps summons, as an older one does, and on one that says how the agent moves alike.
    for older in [true, false] {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.summons = older;
            state.movement = (!older).then(tomori_protocol::AgentMovement::default);
        }));
        conversation.take_actions();
        let mut step = Step::default();
        for _ in 0..(1.0 / FRAME) as usize {
            let frame = step.update(&pulling, &mut conversation, &mut felt(), now, FRAME);
            assert!(!frame.attracting, "{older}");
        }
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert!(placed(&mut conversation).is_empty());
        assert_eq!(
            conversation.arrangement().agent,
            crate::anchors::Anchor::AGENT
        );
        // Away, it is sent home from the hand menu instead, from either.
        conversation.apply(space(|state| {
            state.summons = older;
            state.movement = (!older).then(tomori_protocol::AgentMovement::default);
            state.summoned = Some(tomori_protocol::Pose::at([1.0, 1.4, -1.0]));
        }));
        conversation.send_agent_home();
        assert_eq!(placed(&mut conversation), [None]);
    }
}

#[test]
fn on_an_environment_s_selectable_object_the_trigger_selects_it_and_on_a_panel_does_nothing() {
    let now = Instant::now();
    let at = Vec3::new(0.8, 1.2, -0.6);
    let pull = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
        let pulling = Senses {
            attractor: Some(at),
            trigger: Some(1.0),
            ..*senses
        };
        let mut drawn = false;
        for _ in 0..(1.0 / FRAME).ceil() as usize {
            drawn |= step
                .update(&pulling, conversation, &mut felt(), now, FRAME)
                .attracting;
        }
        let released = Senses {
            trigger: Some(0.0),
            ..*senses
        };
        step.update(&released, conversation, &mut felt(), now, FRAME);
        drawn
    };
    let (mut conversation, done) = lamp(Default::default(), true);
    conversation.take_actions();
    let mut step = Step::default();
    let on_lamp = Senses {
        ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
        ..senses()
    };
    grow(&mut step, &mut conversation, &on_lamp, now);
    assert_eq!(step.hands.right.aim, precedence::Aim::Environment);
    assert!(!pull(&mut step, &mut conversation, &on_lamp));
    assert!(!selects(&done).is_empty());
    // On a panel in front of it, Tomori's, the pull draws nothing and summons nothing.
    code_ahead(&mut conversation, 1);
    conversation.apply(space(|state| state.projection = true));
    conversation.take_actions();
    grow(&mut step, &mut conversation, &on_lamp, now);
    assert_eq!(step.hands.right.aim, precedence::Aim::Tomori);
    assert!(!pull(&mut step, &mut conversation, &on_lamp));
    assert!(placed(&mut conversation).is_empty());
}

#[test]
fn the_agent_glides_where_it_goes_by_its_waypoint_turning_to_face_whoever_it_turns_to() {
    let now = Instant::now();
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| {
        state.movement = Some(tomori_protocol::AgentMovement::default());
    }));
    let mut step = Step::default();
    for _ in 0..10 {
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    }
    // It goes 2 m to the side, over someone's head by its waypoint, and settles there, never jumping.
    let to = tomori_protocol::Pose::at([2.0, 1.4, -2.3]).facing_towards([0.0, 1.6, 0.0]);
    conversation.apply(space(|state| {
        state.movement = Some(tomori_protocol::AgentMovement::default());
        state.summoned = Some(to);
        state.via = Some([1.0, 2.6, -2.3]);
    }));
    let mut last = crate::swarm::CENTER;
    let mut highest = 0.0f32;
    for _ in 0..(8.0 / FRAME) as usize {
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        let at = frame.swarm.anchor().position;
        assert!(at.distance(last) <= crate::glide::SPEED * FRAME + 1e-4);
        // Its ring on the floor stays at home as it goes.
        assert_eq!(frame.home_ring, crate::anchors::HomeRing::WHOLE);
        highest = highest.max(at.y);
        last = at;
    }
    assert!(highest > 2.5);
    assert_eq!(last, Vec3::new(2.0, 1.4, -2.3));
    // Turned to face someone, it turns about the vertical to face them, where it stands.
    conversation.apply(space(|state| {
        state.movement = Some(tomori_protocol::AgentMovement::default());
        state.summoned = Some(to);
        state.facing = Some([4.0, 1.6, 0.0]);
    }));
    let facing = conversation.arrangement().agent;
    assert_eq!(facing.position, Vec3::new(2.0, 1.4, -2.3));
    let ahead = facing.orientation * Vec3::Z;
    let towards = Vec3::new(2.0, 0.0, 2.3).normalize();
    assert!(ahead.dot(towards) > 0.999, "{ahead}");
    // It eases round, gently, never at once.
    let start = step
        .update(&senses(), &mut conversation, &mut felt(), now, FRAME)
        .swarm
        .anchor()
        .orientation;
    assert!(start.angle_between(facing.orientation) > 1.0);
    for _ in 0..(2.5 / FRAME) as usize {
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    }
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert!(
        frame
            .swarm
            .anchor()
            .orientation
            .angle_between(facing.orientation)
            < 0.01
    );
    // Somewhere it cannot face, or a waypoint beyond the space, is ignored; and far away it streams.
    conversation.apply(space(|state| {
        state.movement = Some(tomori_protocol::AgentMovement::default());
        state.summoned = Some(to);
        state.facing = Some([f32::NAN, 0.0, 0.0]);
        state.via = Some([0.0, 0.0, 900.0]);
        state.stream = true;
    }));
    let arrangement = conversation.arrangement();
    assert_eq!(
        arrangement.agent.orientation,
        crate::anchors::Anchor::from_pose(&to).unwrap().orientation
    );
    assert_eq!((arrangement.via, arrangement.stream), (None, true));
}

#[test]
fn the_swarm_s_agents_keep_up_with_the_agent_as_it_glides() {
    let now = Instant::now();
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| {
        state.movement = Some(tomori_protocol::AgentMovement::default());
    }));
    let mut step = Step::default();
    for _ in 0..(3.0 / FRAME) as usize {
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    }
    let rest = {
        let swarm = step.coordinator.swarm();
        middle_of(&agents_of(swarm)).distance(swarm.anchor().position)
    };
    // It comes 4 m straight towards whoever it goes to, its agents, flying a quarter faster than it moves, trailing its
    // middle by no more than they trailed at 0.85 m/s for as long, 0.75 s, and gathering about it once there.
    let to = tomori_protocol::Pose::at([0.0, 1.4, 1.7]).facing_towards([0.0, 1.6, 3.7]);
    conversation.apply(space(|state| {
        state.movement = Some(tomori_protocol::AgentMovement::default());
        state.summoned = Some(to);
    }));
    let mut astray = 0.0f32;
    for _ in 0..(8.0 / FRAME) as usize {
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        let middle = middle_of(&agents_of(frame.swarm));
        astray = astray.max(middle.distance(frame.swarm.anchor().position));
    }
    let swarm = step.coordinator.swarm();
    let there = middle_of(&agents_of(swarm)).distance(swarm.anchor().position);
    assert!(astray < crate::glide::SPEED * 0.75 + 0.01, "{astray}");
    assert!(there < rest + 0.05, "{there}");
}

/// The swarm's agents where `swarm` draws them.
fn agents_of(swarm: &crate::swarm::Swarm) -> Vec<Vec3> {
    swarm.agents.iter().map(|agent| agent.position).collect()
}

/// The middle of `agents`.
fn middle_of(agents: &[Vec3]) -> Vec3 {
    agents.iter().sum::<Vec3>() / agents.len() as f32
}

/// How far the agent that moved furthest went between `before` and `after`.
fn furthest(before: &[Vec3], after: &[Vec3]) -> f32 {
    assert_eq!(before.len(), after.len());
    before
        .iter()
        .zip(after)
        .map(|(before, after)| before.distance(*after))
        .fold(0.0, f32::max)
}

/// How far, at most, one of the swarm's agents may fly in a frame of the frame step: as fast as it streams.
const LEAP: f32 = crate::glide::STREAM * FRAME + 1e-4;

/// Whether `conversation` asked the server to move the agent, as joining and leaving never do.
fn moved_the_agent(conversation: &mut Conversation) -> bool {
    use tomori_protocol::ClientMessage;
    conversation.take_actions().into_iter().any(|action| {
        matches!(
            action,
            Action::Send(
                ClientMessage::PlaceAgent { .. }
                    | ClientMessage::CallAgent
                    | ClientMessage::PinAgent { .. }
                    | ClientMessage::WalkAgent { .. }
                    | ClientMessage::SetAgentMovement { .. }
            )
        )
    })
}

/// The swarm's agents as of the last frame, which watches that none of them jumps, nor is jolted beyond what its
/// flock's forces and the easing of its speed after a flight do, and that the server is never asked to move the agent.
struct Watch {
    seen: Vec<Vec3>,
    velocities: Vec<Vec3>,
}

impl Watch {
    fn new(step: &Step) -> Self {
        let swarm = step.coordinator.swarm();
        Self {
            seen: agents_of(swarm),
            velocities: swarm.agents.iter().map(|agent| agent.velocity).collect(),
        }
    }

    /// A frame: where the agent's place is, whether the swarm flies, and its agents' middle.
    fn frame(
        &mut self,
        step: &mut Step,
        conversation: &mut Conversation,
        now: Instant,
    ) -> (crate::anchors::Anchor, bool, Vec3) {
        let frame = step.update(&senses(), conversation, &mut felt(), now, FRAME);
        let agents = agents_of(frame.swarm);
        assert!(furthest(&self.seen, &agents) <= LEAP);
        let velocities: Vec<Vec3> = frame
            .swarm
            .agents
            .iter()
            .map(|agent| agent.velocity)
            .collect();
        assert!(furthest(&self.velocities, &velocities) <= 6.0 * FRAME + 1e-4);
        let (anchor, flying) = (frame.swarm.anchor(), frame.swarm.flying());
        assert!(!moved_the_agent(conversation));
        self.seen = agents;
        self.velocities = velocities;
        (anchor, flying, middle_of(&self.seen))
    }
}

#[test]
fn while_its_speech_plays_the_agent_still_glides_where_it_follows_and_turns_to_whoever_it_turns_to()
{
    use tomori_client::connection::{Report, Session};
    let now = Instant::now();
    let follow = Some(tomori_protocol::AgentMovement::new(
        tomori_protocol::Movement::Follow,
        Some(90),
    ));
    let mut conversation = joined();
    conversation.apply(space(|state| {
        state.movement = follow;
        state.activity = tomori_protocol::Activity::Idle;
    }));
    let mut step = Step::default();
    // Its turn is over, and its voice plays on this headset, each frame, for `seconds` and then this frame.
    let speaking = |step: &mut Step, conversation: &mut Conversation, seconds: f32| {
        for _ in 0..=(seconds / FRAME) as usize {
            Session::report(
                conversation,
                Report::AgentSpeech {
                    playing: true,
                    level: 0.3,
                },
            );
            step.update(&senses(), conversation, &mut felt(), now, FRAME);
        }
        Session::report(
            conversation,
            Report::AgentSpeech {
                playing: true,
                level: 0.3,
            },
        );
    };
    speaking(&mut step, &mut conversation, 0.5);
    // The server follows them on, mid-reply, more than 3 m: it glides there as it speaks.
    let to = tomori_protocol::Pose::at([3.5, 1.2, -4.0]).facing_towards([4.5, 1.1, -5.1]);
    conversation.apply(space(|state| {
        state.movement = follow;
        state.activity = tomori_protocol::Activity::Idle;
        state.summoned = Some(to);
        state.partner = Some([4.5, 1.1, -5.1]);
    }));
    speaking(&mut step, &mut conversation, 12.0);
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.swarm.visual.mode, interaction::Mode::Speaking);
    let arrived = frame.swarm.anchor();
    assert!(
        arrived.position.distance(Vec3::new(3.5, 1.2, -4.0)) < 0.01,
        "{arrived:?}"
    );
    // Then it turns to them as they step aside, still speaking.
    conversation.apply(space(|state| {
        state.movement = follow;
        state.activity = tomori_protocol::Activity::Idle;
        state.summoned = Some(to);
        state.partner = Some([4.5, 1.1, -5.1]);
        state.facing = Some([5.5, 1.1, -4.6]);
    }));
    let turned = conversation.arrangement().agent;
    let towards = Vec3::new(2.0, 0.0, -0.6).normalize();
    assert!((turned.orientation * Vec3::Z).dot(towards) > 0.999);
    speaking(&mut step, &mut conversation, 3.0);
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.swarm.visual.mode, interaction::Mode::Speaking);
    assert!(
        frame
            .swarm
            .anchor()
            .orientation
            .angle_between(turned.orientation)
            < 0.02
    );
}

#[test]
fn joining_the_agent_is_at_its_place_at_once_and_this_clients_swarm_flies_there_from_the_lobby() {
    let now = Instant::now();
    // A home near the lobby's spot and one across the room, and the agent following someone, away from home.
    let near = tomori_protocol::Pose::at([-1.5, 1.6, -3.5]).facing_towards([0.0, 1.6, 0.0]);
    let across = tomori_protocol::Pose::at([-3.0, 1.5, -1.0]).facing_towards([0.0, 1.6, 0.0]);
    let following = tomori_protocol::Pose::at([2.5, 1.5, 1.0]).facing_towards([0.0, 1.6, 0.0]);
    for (home, summoned) in [(near, None), (across, None), (near, Some(following))] {
        let mut conversation = lobby(ConnectionRole::Owner);
        let mut step = Step::default();
        for _ in 0..200 {
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        }
        let mut seen = agents_of(step.coordinator.swarm());
        let lobby = middle_of(&seen);
        assert!(lobby.distance(crate::swarm::CENTER) < 0.5, "{lobby}");
        conversation.apply(space(|state| {
            state.anchors = tomori_protocol::Anchors::new().with_agent(home);
            state.summoned = summoned;
        }));
        assert!(placed(&mut conversation).is_empty());
        let place = conversation.arrangement().agent;
        let distance = lobby.distance(place.position);
        assert!(distance > 1.5, "{distance}");
        let mut revealing = false;
        let mut flown = None;
        for index in 0..72 * 8 {
            let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
            // The agent is where everyone sees it from the first frame, and the server is never asked to move it.
            let at = frame.swarm.anchor();
            assert_eq!(at.position, place.position);
            assert!(at.orientation.dot(place.orientation).abs() > 1.0 - 1e-6);
            assert!(!moved_the_agent(&mut conversation));
            // Every one of the swarm's agents is drawn throughout, and none ever jumps.
            let agents = agents_of(frame.swarm);
            assert_eq!(agents.len(), crate::swarm::AGENT_COUNT);
            assert!(furthest(&seen, &agents) <= LEAP, "{index}");
            seen = agents;
            let middle = middle_of(&seen);
            if index == 0 {
                // It sets off from the lobby as the environment begins to grow in.
                assert!(frame.swarm.flying());
                assert!(middle.distance(lobby) < 0.05, "{middle}");
            }
            if frame.scenery.reveal.is_some() {
                revealing = true;
            } else if revealing {
                // By the time the environment has grown in, it is well on its way, though not yet there.
                revealing = false;
                let gone = lobby.distance(middle);
                assert!(gone > 0.15 * distance, "{gone} of {distance}");
                assert!(middle.distance(place.position) > 0.2 * distance);
                assert!(frame.swarm.flying());
            }
            if !frame.swarm.flying() && flown.is_none() {
                flown = Some(index);
            }
        }
        // It arrives within a few seconds, and stands there.
        let flown = flown.expect("arrived") as f32 * FRAME;
        assert!((1.0..4.0).contains(&flown), "{flown}");
        assert!(middle_of(&seen).distance(place.position) < 0.1);
        // And it stays, through a long idle, with no summons.
        for _ in 0..3000 {
            let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(frame.swarm.anchor().position, place.position);
        }
        assert!(placed(&mut conversation).is_empty());
    }
}

#[test]
fn the_swarm_flies_to_the_agent_as_it_moves_turns_back_on_leaving_and_flies_in_again_on_rejoining()
{
    let now = Instant::now();
    let home = tomori_protocol::Pose::at([-3.0, 1.5, -1.0]).facing_towards([0.0, 1.6, 0.0]);
    let mut conversation = lobby(ConnectionRole::Owner);
    let mut step = Step::default();
    for _ in 0..200 {
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    }
    let mut watch = Watch::new(&step);
    let mut frame =
        |step: &mut Step, conversation: &mut Conversation| watch.frame(step, conversation, now);
    // Joining as the agent follows someone, who walks on: the swarm flies on to it as it glides after them.
    let following =
        |x: f32| Some(tomori_protocol::Pose::at([x, 1.5, 1.0]).facing_towards([0.0, 1.6, 0.0]));
    conversation.apply(space(|state| {
        state.anchors = tomori_protocol::Anchors::new().with_agent(home);
        state.summoned = following(2.5);
    }));
    conversation.take_actions();
    assert!(frame(&mut step, &mut conversation).1, "flying");
    for index in 1..72 * 6 {
        if index < 72 * 2 && index % 12 == 0 {
            conversation.apply(space(|state| {
                state.anchors = tomori_protocol::Anchors::new().with_agent(home);
                state.summoned = following(2.5 - index as f32 / 72.0);
            }));
        }
        frame(&mut step, &mut conversation);
    }
    let (place, flying, middle) = frame(&mut step, &mut conversation);
    assert!(!flying);
    // Where they walked to last.
    let walked = Vec3::new(2.5 - 132.0 / 72.0, 1.5, 1.0);
    assert!(place.position.distance(walked) < 0.05, "{place:?}");
    assert!(middle.distance(place.position) < 0.1);
    // Rejoined from scratch, and left half a second into the flight: it turns back smoothly, to the lobby.
    conversation.hold_back(None);
    assert!(conversation.hold_back(Some(LEAVE_HOLD)));
    let mut lobby_spot = Vec3::ZERO;
    for _ in 0..72 * 6 {
        lobby_spot = frame(&mut step, &mut conversation).2;
    }
    assert!(conversation.joined_space().is_none());
    conversation.join_space(SPACE);
    conversation.apply(space(|state| {
        state.anchors = tomori_protocol::Anchors::new().with_agent(home);
    }));
    conversation.take_actions();
    let home = crate::anchors::Anchor::from_pose(&home).unwrap().position;
    let setting_off = agents_of(step.coordinator.swarm());
    let mut turned = lobby_spot;
    for _ in 0..36 {
        let (_, flying, middle) = frame(&mut step, &mut conversation);
        assert!(flying);
        turned = middle;
    }
    // Its first cubes are well on their way, as the last set off.
    let towards = (home - lobby_spot).normalize();
    let leading = agents_of(step.coordinator.swarm())
        .iter()
        .zip(&setting_off)
        .map(|(agent, before)| (*agent - *before).dot(towards))
        .fold(0.0, f32::max);
    assert!(leading > 0.2, "{leading}");
    conversation.hold_back(None);
    assert!(conversation.hold_back(Some(LEAVE_HOLD)));
    let mut nearest = f32::INFINITY;
    for _ in 0..36 {
        // Near the lobby's spot still, it is soon back.
        let (place, _, middle) = frame(&mut step, &mut conversation);
        assert_eq!(place, crate::anchors::Anchor::AGENT);
        nearest = nearest.min(middle.distance(home));
    }
    // It turned back, getting little nearer the space's agent than it was as it left, as its way on was spent.
    let left = turned.distance(home);
    assert!(nearest > left - 0.4, "{nearest} {left}");
    // Rejoined quickly, as the environment still shrinks away, it flies in again, and arrives.
    assert!(conversation.joined_space().is_none());
    conversation.join_space(SPACE);
    conversation.apply(space(|state| {
        state.anchors = tomori_protocol::Anchors::new()
            .with_agent(tomori_protocol::Pose::at(home.to_array()).facing_towards([0.0, 1.6, 0.0]));
    }));
    conversation.take_actions();
    assert!(frame(&mut step, &mut conversation).1);
    let mut flown = false;
    let mut middle = Vec3::ZERO;
    for _ in 0..72 * 4 {
        let (_, flying, now) = frame(&mut step, &mut conversation);
        flown |= !flying;
        middle = now;
    }
    assert!(flown);
    assert!(middle.distance(home) < 0.1);
}

#[test]
fn an_environment_s_ambience_plays_once_its_scenery_grows_in_carried_into_the_space_until_it_is_left()
 {
    use crate::ambience::{Ambience, Loop, Slot};
    use std::sync::Arc;
    let waterfall =
        Ambience::new().with_loop(Loop::at(Slot::missing("waterfall"), [0.0, 2.0, 0.0]));
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_ambiences(vec![(crate::test_support::SKY, Arc::new(waterfall))]);
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    // None in the lobby.
    step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
    assert!(feedback.take_ambience().is_none());
    // Participants arrive 8 m along +X in the environment, facing its origin, where its waterfall is.
    let street = tomori_protocol::Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0; 3]);
    conversation.apply(space(|state| {
        state.anchors = tomori_protocol::Anchors::new().with_arrival(street);
    }));
    conversation.take_actions();
    // Not while its scenery grows in, and then once, carried: 8 m ahead of home.
    step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
    assert!(feedback.take_ambience().is_none());
    for _ in 0..400 {
        step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
    }
    let playing = feedback.take_ambience().unwrap().unwrap();
    let heard = Vec3::from_array(playing.loops[0].path.points()[0]);
    assert!(heard.distance(Vec3::new(0.0, 2.0, -8.0)) < 1e-4, "{heard}");
    step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
    assert!(feedback.take_ambience().is_none());
    // Leaving, none.
    conversation.apply(tomori_client::Event::Message(
        tomori_protocol::ServerMessage::LEFT,
    ));
    step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
    assert!(feedback.take_ambience().unwrap().is_none());
    assert!(conversation.ambience().is_none());
}

#[test]
fn an_environment_arriving_elsewhere_is_carried_into_the_space_for_its_client_side_and_scenery() {
    use std::sync::{Arc, Mutex};
    // A client side that records the ray and head it is given, and makes a sound at its own origin.
    #[derive(Default)]
    struct Seen {
        rays: Vec<Ray>,
        heads: Vec<Vec3>,
    }
    struct Recording(Arc<Mutex<Seen>>);
    impl crate::client_side::ClientSide for Recording {
        fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
            self.0.lock().unwrap().rays.push(*ray);
            None
        }
        fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
            self.0.lock().unwrap().heads.extend(input.head);
            crate::client_side::Output::new().with_sound(
                crate::client_side::Sound::new(vec![0.0; 4], 48_000),
                [0.0, 0.0, 0.0],
            )
        }
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let factory: crate::client_side::Factory = {
        let seen = seen.clone();
        Arc::new(move |_space: &str| {
            Box::new(Recording(seen.clone())) as Box<dyn crate::client_side::ClientSide>
        })
    };
    // Participants arrive 8 m along +X in the environment, facing its origin.
    let street = tomori_protocol::Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0; 3]);
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    conversation.apply(space(|state| {
        state.anchors = tomori_protocol::Anchors::new().with_arrival(street);
    }));
    let carriage = conversation.arrangement().carriage;
    assert!(
        carriage
            .place(Vec3::ZERO)
            .distance(Vec3::new(0.0, 0.0, -8.0))
            < 1e-4
    );
    let mut step = Step::default();
    let pointing = Senses {
        ray: Some(Ray {
            origin: Vec3::new(0.0, 1.5, 0.0),
            direction: Vec3::NEG_Z,
        }),
        beam: true,
        ..senses()
    };
    let mut feedback = felt();
    let mut frame_carriage = None;
    for _ in 0..2000 {
        let frame = step.update(
            &pointing,
            &mut conversation,
            &mut feedback,
            Instant::now(),
            FRAME,
        );
        frame_carriage = Some(frame.carriage);
    }
    // The scenery is drawn carried, and the client side picks and hears in the environment's own coordinates:
    // from the street, looking at its origin.
    assert_eq!(frame_carriage, Some(carriage));
    let seen = seen.lock().unwrap();
    let ray = seen.rays.last().expect("picked");
    assert!(
        ray.origin.distance(Vec3::new(8.0, 1.5, 0.0)) < 1e-4,
        "{:?}",
        ray.origin
    );
    assert!(
        ray.direction.distance(Vec3::NEG_X) < 1e-4,
        "{:?}",
        ray.direction
    );
    let head = seen.heads.last().expect("told where the head is");
    assert!(head.distance(Vec3::new(8.0, 1.6, 0.0)) < 1e-4, "{head}");
    drop(seen);
    // Its sound, made at its origin, is heard 8 m ahead of home.
    let (_, position) = feedback.take_ambient().next_back().expect("a sound");
    assert!(Vec3::from_array(position).distance(Vec3::new(0.0, 0.0, -8.0)) < 1e-4);
    // On the grid, nothing is carried.
    conversation.apply(space(|state| {
        state.environment = "minimal".into();
        state.anchors = tomori_protocol::Anchors::new().with_arrival(street);
    }));
    assert_eq!(
        conversation.arrangement().carriage,
        crate::anchors::Anchor::HOME
    );
}

#[test]
fn leaving_a_space_keeps_its_environment_where_it_was_seen_until_it_has_gone_as_the_swarm_flies_back()
 {
    use crate::locomotion::{Locomotion, Surface};
    // Participants arrive 8 m along +X, facing the environment's origin, as the farmyard's arrive inside its gate;
    // the agent's home is its own; and travel teleports.
    let street = tomori_protocol::Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0; 3]);
    let home = tomori_protocol::Pose::at([1.0, 1.4, -3.0]).facing_towards([0.0, 1.6, 0.0]);
    // The swarm's agents in the participant's room, the scene placed by `rig`.
    let in_room = |step: &Step, rig: Rig| {
        let back = rig.matrix().inverse();
        agents_of(step.coordinator.swarm())
            .into_iter()
            .map(|agent| back.transform_point3(agent))
            .collect::<Vec<_>>()
    };
    let spot = crate::anchors::Anchor::AGENT.position;
    let close = |a: crate::anchors::Anchor, b: crate::anchors::Anchor| {
        a.position.distance(b.position) < 1e-4
            && a.orientation.dot(b.orientation).abs() > 1.0 - 1e-6
    };
    // Left with B held, as the frame's buttons are read before the platform places its poses, or within a frame
    // after it placed them, as the hand menu leaves.
    for within in [false, true] {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_locomotion(vec![(
            crate::test_support::SKY,
            Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
        )]);
        conversation.apply(space(|state| {
            state.anchors = tomori_protocol::Anchors::new()
                .with_arrival(street)
                .with_agent(home);
        }));
        let arrangement = conversation.arrangement();
        assert_ne!(arrangement.carriage, crate::anchors::Anchor::HOME);
        let mut step = Step::default();
        let now = Instant::now();
        // The platform places the frame's poses with the rig, as it was settled before the frame.
        let placed = |step: &mut Step, conversation: &Conversation, ray: Vec3| {
            let rig = step.rig(conversation).matrix();
            let head = rig * Mat4::from_translation(arrival::HOME);
            Senses {
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(head.w_axis.truncate()),
                ray: Ray::new(
                    rig.transform_point3(arrival::HOME),
                    rig.transform_vector3(ray),
                ),
                ..senses()
            }
        };
        let stick = |step: &mut Step, conversation: &mut Conversation, stick: Vec2| {
            step.buttons(
                &press(|buttons| buttons.left_stick = stick),
                true,
                conversation,
                &mut felt(),
                now,
            );
        };
        // Entering is as it was: the environment is carried, and the agent stands at its home, from the first frame.
        for _ in 0..400 {
            let senses = placed(&mut step, &conversation, -Vec3::Z);
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(frame.carriage, arrangement.carriage);
            assert_eq!(frame.swarm.anchor(), arrangement.agent);
        }
        // Teleported down the yard, the space is drawn from there.
        stick(&mut step, &mut conversation, Vec2::Y);
        let senses = placed(&mut step, &conversation, Vec3::new(0.3, -0.5, -1.0));
        assert!(
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .landing
                .is_some()
        );
        stick(&mut step, &mut conversation, Vec2::ZERO);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let rig = step.rig(&conversation);
        assert_ne!(rig, Rig::default());
        let senses = placed(&mut step, &conversation, -Vec3::Z);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.carriage, arrangement.carriage);
        // Where the environment and the swarm stand in the participant's room.
        let carriage = rig.unplace(arrangement.carriage);
        let ring = rig.unplace(at(arrangement.home_ring().centre)).position;
        assert_eq!(frame.home_ring, arrangement.home_ring());
        let mut swarm = in_room(&step, rig);
        let left = middle_of(&swarm).distance(spot);
        assert!(left > 2.0, "travelled from the lobby's spot: {left}");
        conversation.take_actions();
        // Leaving, the rig goes home at once, while the environment shrinks back into the space's object.
        let mut senses = if within {
            let senses = placed(&mut step, &conversation, -Vec3::Z);
            assert!(conversation.hold_back(Some(LEAVE_HOLD)));
            senses
        } else {
            assert!(conversation.hold_back(Some(LEAVE_HOLD)));
            let senses = placed(&mut step, &conversation, -Vec3::Z);
            assert_eq!(
                step.placed,
                Rig::default(),
                "home before the frame is placed"
            );
            senses
        };
        let mut shrinking = 0;
        loop {
            let rig = step.placed;
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            // The agent's place is the lobby's at once, for this client alone, and the server is never asked to
            // move it.
            assert_eq!(frame.swarm.anchor(), crate::anchors::Anchor::AGENT);
            if frame.scenery.environment == crate::environment::Environment::MINIMAL {
                // Once it has gone, nothing is carried.
                assert_eq!(frame.carriage, crate::anchors::Anchor::HOME);
                assert_eq!(frame.home_ring, crate::anchors::HomeRing::WHOLE);
                break;
            }
            shrinking += 1;
            // While it is drawn, it stays where the participant saw it, in their room.
            assert!(
                close(rig.unplace(frame.carriage), carriage),
                "{within} {shrinking}"
            );
            // The swarm flies back to the lobby's spot from where they saw it, as it shrinks away.
            assert!(frame.swarm.flying(), "{within} {shrinking}");
            // Its ring on the floor with it.
            assert!(
                rig.unplace(at(frame.home_ring.centre))
                    .position
                    .distance(ring)
                    < 1e-4,
                "{within} {shrinking}"
            );
            // None of its agents ever jumps in the room, as the rig goes home or after.
            let seen = in_room(&step, rig);
            let leap = furthest(&swarm, &seen);
            assert!(leap <= LEAP, "{within} {shrinking}: {leap}");
            swarm = seen;
            assert!(!moved_the_agent(&mut conversation));
            senses = placed(&mut step, &conversation, -Vec3::Z);
            assert_eq!(step.placed, Rig::default());
        }
        assert!(shrinking > 10, "it shrinks away over frames: {shrinking}");
        // It set off as the environment shrank away, and flies on to the lobby's spot.
        let gone = middle_of(&swarm).distance(spot);
        assert!(gone < left - 0.15, "{gone} of {left}");
        let mut flying = 0;
        loop {
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(frame.swarm.anchor(), crate::anchors::Anchor::AGENT);
            assert_eq!(frame.carriage, crate::anchors::Anchor::HOME);
            let flown = !frame.swarm.flying();
            let seen = in_room(&step, Rig::default());
            assert!(furthest(&swarm, &seen) <= LEAP, "{within} {flying}");
            swarm = seen;
            if flown {
                break;
            }
            flying += 1;
            assert!(flying < 72 * 4, "it arrives within seconds");
        }
        for _ in 0..360 {
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        }
        // About the lobby's spot, as its flock circles it there.
        let rested = middle_of(&in_room(&step, Rig::default()));
        assert!(rested.distance(spot) < 1.0, "{rested}");
        assert!(!moved_the_agent(&mut conversation));
    }
}

#[test]
fn the_ray_chooses_what_it_points_at_and_draws_the_beam_where_the_platform_does() {
    let now = Instant::now();
    for beam in [true, false] {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![tomori_client::server_choice::Choice::new(
            "Studio",
            "10.0.0.1:8765",
            tomori_client::server_choice::Source::Discovered,
        )]);
        let mut feedback = felt();
        let mut step = Step::default();
        // Arrival's objects move in over a few seconds.
        step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
        let frame = step.update(&senses(), &mut conversation, &mut feedback, now, 5.0);
        let scene = frame.arrival.expect("arrival's objects").scene;
        let target = target(&scene, &arrival::Key::Server("10.0.0.1:8765".into()));
        let aiming = Senses {
            ray: aim_at(target.shape),
            beam,
            ..senses()
        };
        let frame = step.update(&aiming, &mut conversation, &mut feedback, now, FRAME);
        let arrival = frame.arrival.expect("arrival");
        assert_eq!(arrival.hovered, Some(target.key.clone()));
        assert_eq!(arrival.ray.is_some_and(|ray| ray.hit), beam);
        assert!(earcons(&mut feedback).is_empty());
        // Pulling the trigger on it chooses it, and is heard.
        let pull = Senses {
            trigger: Some(1.0),
            ..aiming
        };
        let frame = step.update(&pull, &mut conversation, &mut feedback, now, FRAME);
        assert!(!frame.attracting);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        assert!(
            conversation
                .take_actions()
                .iter()
                .any(|action| matches!(action, Action::Connect(_)))
        );
    }
}

#[test]
fn a_chooses_what_the_ray_met_last_frame_or_else_accepts() {
    let now = Instant::now();
    let mut conversation = Conversation::new("Ada");
    conversation.set_servers(vec![tomori_client::server_choice::Choice::new(
        "Studio",
        "10.0.0.1:8765",
        tomori_client::server_choice::Source::Discovered,
    )]);
    let mut feedback = felt();
    let mut step = Step::default();
    step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
    // With nothing highlighted, A accepts the selected server.
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        now,
    );
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    assert!(
        conversation
            .take_actions()
            .iter()
            .any(|action| matches!(action, Action::Connect(_)))
    );
}

#[test]
fn what_waits_is_cued_on_the_palms_only_where_the_platform_cues() {
    let now = Instant::now();
    // Palms turned down and away from the viewer, so that neither panel shows.
    let away = Palm {
        wrist: arrival::HOME + Vec3::new(0.0, -0.4, -0.3),
        normal: -Vec3::Y,
    };
    let cued = |conversation: &mut Conversation, cues: bool| {
        let senses = Senses {
            left_palm: Some(away),
            right_palm: Some(away),
            cues,
            ..senses()
        };
        Step::default()
            .update(&senses, conversation, &mut felt(), now, FRAME)
            .arrival
            .map_or(0, |arrival| arrival.scene.objects.len())
    };
    // A knock is cued on the left palm, and an approval on the right wrist.
    let mut knocked = joined();
    knock(&mut knocked, "knock-1", "Grace");
    assert_eq!(cued(&mut knocked, true), 1);
    assert_eq!(cued(&mut knocked, false), 0);
    let mut approval = approval_waiting();
    assert_eq!(cued(&mut approval, true), 1);
    assert_eq!(cued(&mut approval, false), 0);
    knock(&mut approval, "knock-1", "Grace");
    assert_eq!(cued(&mut approval, true), 2);
    assert_eq!(cued(&mut joined(), true), 0);
}

#[test]
fn the_swarm_speaks_while_agent_speech_plays_though_the_agent_is_idle() {
    use tomori_client::connection::{Report, Session};
    let now = Instant::now();
    let mut conversation = joined();
    conversation.apply(space(|state| {
        state.activity = tomori_protocol::Activity::Idle;
    }));
    let mut step = Step::default();
    let speech = |conversation: &mut Conversation, playing| {
        Session::report(
            conversation,
            Report::AgentSpeech {
                playing,
                level: if playing { 0.3 } else { 0.0 },
            },
        );
    };
    speech(&mut conversation, true);
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.swarm.visual.mode, interaction::Mode::Speaking);
    // Speech stopping for a moment, as a buffer runs dry, keeps it speaking.
    speech(&mut conversation, false);
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.swarm.visual.mode, interaction::Mode::Speaking);
    // Once it has stopped for the hold, the swarm shows the agent as it is.
    for _ in 0..(interaction::SPEAKING_HOLD / FRAME) as usize + 1 {
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    }
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.swarm.visual.mode, interaction::Mode::Ready);
}

#[test]
fn participants_report_their_head_and_hands_and_hear_from_their_head() {
    let now = Instant::now();
    let controller = Pose {
        position: [0.2, 1.1, -0.3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    let senses = Senses {
        controllers: [None, Some(controller)],
        ..senses()
    };
    let mut conversation = joined();
    let mut step = Step::default();
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let presence = frame.presence.expect("a participant is seen");
    assert_eq!(Some(presence.head), senses.head_pose);
    assert_eq!((presence.left, presence.right), (None, Some(controller)));
    assert_eq!(frame.listener, senses.head_pose);
    // The agent is heard from the swarm's middle.
    assert!(Vec3::from_array(frame.agent).distance(crate::swarm::CENTER) < 1.0);
    // Without a head there is nothing to report, and visitors report nothing.
    let headless = Senses {
        head_pose: None,
        ..senses
    };
    assert!(
        step.update(&headless, &mut conversation, &mut felt(), now, FRAME)
            .presence
            .is_none()
    );
    let mut visitor = lobby(ConnectionRole::Visitor);
    visitor.apply(space(|_| {}));
    let mut step = Step::default();
    assert!(
        step.update(&senses, &mut visitor, &mut felt(), now, FRAME)
            .presence
            .is_none()
    );
}

#[test]
fn the_swarm_and_workers_stand_where_the_space_s_environment_puts_them() {
    use crate::anchors::{Anchor, Placing, WorkerPlacement};
    struct Ahead;
    impl WorkerPlacement for Ahead {
        fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            origin.place(Vec3::new(worker.slot as f32, 1.5, -2.0))
        }
    }
    let agent = tomori_protocol::Pose {
        position: [3.0, 1.6, -5.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    let workers = tomori_protocol::Pose {
        position: [0.0, 0.0, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation
        .set_worker_placements(vec![(crate::test_support::SKY, std::sync::Arc::new(Ahead))]);
    conversation.apply(space(|state| {
        state.anchors = tomori_protocol::Anchors::new()
            .with_agent(agent)
            .with_workers(workers);
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
    }));
    let mut step = Step::default();
    let now = Instant::now();
    // From the first frame in the space, while its scenery still grows in, the placement places the worker.
    let first = step
        .update(&senses(), &mut conversation, &mut felt(), now, FRAME)
        .stations[0]
        .position;
    let scenery = step.motion.scenery(step.arrival_time);
    assert!(
        scenery.reveal.is_some() || scenery.environment == crate::environment::Environment::MINIMAL,
        "still growing in"
    );
    assert_eq!(first, [0.0, 1.5, -6.0]);
    for _ in 0..1000 {
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    }
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.swarm.anchor(), Anchor::from_pose(&agent).unwrap());
    // The agent is heard from the swarm, where it has glided.
    assert!(Vec3::from_array(frame.agent).distance(Vec3::from_array(agent.position)) < 1.0);
    // The worker stands where the placement put it about the workers anchor.
    assert_eq!(frame.stations[0].position, [0.0, 1.5, -6.0]);
}

#[test]
fn the_avatar_the_space_s_backend_names_stands_in_for_the_swarm_from_the_first_frame() {
    use crate::avatar::{Avatar, Factory, Input};
    use std::sync::{Arc, Mutex};
    /// A figure standing at its anchor: heard from its mouth, workers leaving from its hand.
    struct Figure(Arc<Mutex<Vec<Input>>>, crate::anchors::Anchor);
    impl Avatar for Figure {
        fn update(&mut self, input: &Input) {
            self.1 = input.anchor;
            self.0.lock().unwrap().push(*input);
        }
        fn voices(&self, voices: &mut Vec<[f32; 3]>) {
            voices.push((self.1.position + Vec3::Y * 0.3).to_array());
        }
        fn departures(&self, departures: &mut Vec<[f32; 3]>) {
            departures.push((self.1.position + Vec3::X * 0.4).to_array());
        }
        fn bounds(&self) -> crate::arrival_motion::Sphere {
            crate::arrival_motion::Sphere {
                centre: self.1.position,
                radius: 0.5,
            }
        }
        fn drawn(&self) -> Option<crate::client_side::Drawn> {
            Some(Arc::new(self.1.position))
        }
    }
    let told = Arc::new(Mutex::new(Vec::new()));
    let made = Arc::new(Mutex::new(Vec::new()));
    let factory: Factory = {
        let (told, made) = (told.clone(), made.clone());
        Arc::new(move |space: &str| {
            made.lock().unwrap().push(space.to_owned());
            Box::new(Figure(told.clone(), crate::anchors::Anchor::AGENT)) as Box<dyn Avatar>
        })
    };
    let agent = tomori_protocol::Pose {
        position: [3.0, 0.0, -5.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    let figure = |state: &mut tomori_protocol::SpaceState| {
        state.avatar = Some("figure".into());
        state.anchors = tomori_protocol::Anchors::new().with_agent(agent);
    };
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_avatars(vec![("figure".into(), factory)]);
    let mut step = Step::default();
    let now = Instant::now();
    // Outside a space, the swarm.
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert!(frame.avatar.is_none());
    assert_eq!(frame.swarm.agents.len(), crate::swarm::AGENT_COUNT);
    // From the first frame in the space, while its scenery still grows in, the figure stands in for the swarm.
    conversation.apply(space(figure));
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    let shown = frame.avatar.as_ref().expect("the figure shows");
    assert_eq!(shown.avatar, "figure");
    let position = *shown
        .drawn
        .as_ref()
        .unwrap()
        .downcast_ref::<Vec3>()
        .unwrap();
    assert_eq!(position, Vec3::from_array(agent.position));
    // The renderer draws no swarm, though the ring beneath the agent stands where it does.
    assert!(frame.swarm.agents.is_empty());
    assert_eq!(frame.swarm.anchor().position, position);
    // The agent is heard from its mouth, and a worker that arrives flies out from its hand.
    assert_eq!(frame.agent, [3.0, 0.3, -5.0]);
    conversation.apply(space(|state| {
        figure(state);
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
    }));
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    let hand = Vec3::new(3.4, 0.0, -5.0);
    let leaving = frame
        .cubes
        .iter()
        .filter(|cube| Vec3::from_array(cube.position).distance(hand) < 0.2)
        .count();
    assert!(leaving > 0, "{:?}", frame.cubes);
    // It was told the agent's state, where it stands and where the participant's head is.
    let input = *told.lock().unwrap().last().unwrap();
    assert_eq!(input.anchor.position, position);
    assert_eq!(input.head, Some(arrival::HOME));
    assert!(!input.speaking);
    assert_eq!(input.attention, None);
    assert_eq!(input.dt, FRAME);
    assert_eq!(input.attractor, None);
    // The agent at work, and the trigger pulled away from everything, which in a space draws nothing: it is told
    // the one and not the other.
    conversation.apply(space(|state| {
        figure(state);
        state.activity = tomori_protocol::Activity::Running;
    }));
    let pull = Vec3::new(1.0, 1.2, -1.5);
    let pulling = Senses {
        ray: Ray::new(arrival::HOME, Vec3::Y),
        trigger: Some(1.0),
        attractor: Some(pull),
        ..senses()
    };
    for _ in 0..10 {
        step.update(&pulling, &mut conversation, &mut felt(), now, FRAME);
    }
    let input = *told.lock().unwrap().last().unwrap();
    assert_eq!(input.attractor, None);
    assert_eq!(input.visual.mode, interaction::Mode::Working);
    assert!(input.visual.working);
    // Made once, for the space, which it stands in.
    assert_eq!(*made.lock().unwrap(), [SPACE]);
    assert_eq!(step.coordinator.bounds().radius, 0.5);
    // Leaving, the swarm again, where it glided meanwhile.
    conversation.apply(tomori_client::Event::Message(
        tomori_protocol::ServerMessage::LEFT,
    ));
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    assert!(frame.avatar.is_none());
    assert_eq!(frame.swarm.agents.len(), crate::swarm::AGENT_COUNT);
}

#[test]
fn the_pill_stands_ahead_of_the_head_and_a_recentre_places_it_again() {
    let now = Instant::now();
    let mut conversation = joined();
    let mut step = Step::default();
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    let model = frame.panels.pill.model.expect("the pill shows in a space");
    let ahead = model.transform_point3(Vec3::ZERO);
    assert!(ahead.z < arrival::HOME.z - 1.0, "{ahead}");
    // Turned round, the pill follows only lazily, until a recentre places it ahead again.
    let turned = Senses {
        space_head: Mat4::from_rotation_translation(
            glam::Quat::from_rotation_y(std::f32::consts::PI),
            arrival::HOME,
        ),
        ..senses()
    };
    let frame = step.update(&turned, &mut conversation, &mut felt(), now, FRAME);
    let lazily = frame
        .panels
        .pill
        .model
        .unwrap()
        .transform_point3(Vec3::ZERO);
    assert!(lazily.z < arrival::HOME.z, "{lazily}");
    step.recentre();
    let frame = step.update(&turned, &mut conversation, &mut felt(), now, FRAME);
    let placed = frame
        .panels
        .pill
        .model
        .unwrap()
        .transform_point3(Vec3::ZERO);
    assert!(placed.z > arrival::HOME.z + 1.0, "{placed}");
}

#[test]
fn the_transcript_takes_the_right_stick_a_b_and_the_grip_while_it_shows() {
    let mut conversation = approval_waiting();
    let mut step = Step::default();
    let start = Instant::now();
    let open = Senses {
        right_palm: Some(palm_up(0.0)),
        ..senses()
    };
    assert!(
        wrist_after(&mut step, &mut conversation, &open, start)
            .model
            .is_some()
    );
    let mut feedback = felt();
    let clock = std::cell::Cell::new(start + HOLD);
    let tap = |step: &mut Step,
               conversation: &mut Conversation,
               feedback: &mut Feedback,
               buttons: Buttons| {
        let now = clock.get() + Duration::from_millis(50);
        clock.set(now);
        step.buttons(&buttons, true, conversation, feedback, now);
        step.buttons(&Buttons::default(), true, conversation, feedback, now);
        now
    };
    // The stick moves the focus, which ticks on the right hand, instead of paging result cards.
    let moved = tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.right_stick = Vec2::new(0.0, -1.0)),
    );
    assert!(!felt_on(&mut feedback, Side::Right, moved).is_empty());
    assert_eq!(step.presentation_navigation.card, 0);
    // A expands the agent's turn, which is heard.
    let mut conversation = conversed();
    let mut step = Step::default();
    assert!(
        wrist_after(&mut step, &mut conversation, &open, start)
            .model
            .is_some()
    );
    tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.right_stick = Vec2::new(0.0, 1.0)),
    );
    // Sideways, it jumps between turns.
    let turn = conversation.wrist_focus().cloned();
    assert!(turn.is_some());
    tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.right_stick = Vec2::new(-1.0, 0.0)),
    );
    assert_ne!(conversation.wrist_focus().cloned(), turn);
    tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.right_stick = Vec2::new(1.0, 0.0)),
    );
    assert_eq!(conversation.wrist_focus().cloned(), turn);
    earcons(&mut feedback);
    tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.a = true),
    );
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    // The grip detaches it where it stands; with no head to follow, it stays.
    let detach = Senses {
        grip: Some(1.0),
        ..open
    };
    let now = clock.get();
    let frame = step.update(&detach, &mut conversation, &mut feedback, now, FRAME);
    let model = frame.panels.wrist.model;
    assert!(step.wrist.state().detached());
    let headless = Senses {
        head: None,
        grip: Some(0.0),
        right_palm: None,
        ..open
    };
    let frame = step.update(&headless, &mut conversation, &mut feedback, now, FRAME);
    assert_eq!(frame.panels.wrist.model, model);
    // B backs out of the expanded turn first, then returns it to the wrist, rather than backing out of the space.
    tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.back = true),
    );
    assert!(step.wrist.state().detached());
    tap(
        &mut step,
        &mut conversation,
        &mut feedback,
        press(|buttons| buttons.back = true),
    );
    assert!(!step.wrist.state().detached());
    assert!(conversation.joined_space().is_some());
}

#[test]
fn the_ray_and_a_choose_only_in_the_hand_menu_while_it_shows() {
    use crate::hand_menu::MenuKey;
    let start = Instant::now();
    let menu = Senses {
        left_palm: Some(left_palm_up(0.0)),
        ..senses()
    };
    // Y pins the menu open at its Space section, whose rows choose at once.
    let open = |conversation: &mut Conversation, feedback: &mut Feedback| {
        let mut step = Step::default();
        step.buttons(
            &press(|buttons| buttons.y = true),
            true,
            conversation,
            feedback,
            start,
        );
        let frame = step.update(&menu, conversation, feedback, start, FRAME);
        let scene = frame.arrival.expect("the pinned menu").scene;
        (step, scene)
    };
    let mut conversation = joined();
    let mut feedback = felt();
    let (mut step, scene) = open(&mut conversation, &mut feedback);
    let mute = arrival::Key::Menu(MenuKey::Mute);
    let aiming = Senses {
        ray: aim_at(target(&scene, &mute).shape),
        beam: true,
        ..menu
    };
    let frame = step.update(&aiming, &mut conversation, &mut feedback, start, FRAME);
    let arrival = frame.arrival.expect("the menu");
    assert_eq!(arrival.hovered, Some(mute.clone()));
    assert!(arrival.ray.is_some_and(|ray| ray.hit));
    // Pointing at the menu counts as pointing, which keeps the transcript from showing.
    assert!(step.pointing(None));
    earcons(&mut feedback);
    // A chooses the row the ray is on.
    let muted = conversation.menu_model().muted;
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        start,
    );
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    assert_ne!(conversation.menu_model().muted, muted);
    // As does the trigger, which never attracts while the menu shows.
    let mut conversation = joined();
    let (mut step, _) = open(&mut conversation, &mut feedback);
    step.update(&aiming, &mut conversation, &mut feedback, start, FRAME);
    let pull = Senses {
        trigger: Some(1.0),
        ..aiming
    };
    let frame = step.update(&pull, &mut conversation, &mut feedback, start, FRAME);
    assert!(!frame.attracting);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    assert_ne!(conversation.menu_model().muted, muted);
    // The trigger on a section opens it, which is heard too.
    let mut conversation = joined();
    let mut step = Step::default();
    step.update(&menu, &mut conversation, &mut feedback, start, FRAME);
    let frame = step.update(&menu, &mut conversation, &mut feedback, start + HOLD, FRAME);
    let section = arrival::Key::Menu(MenuKey::Section(hand_menu::Section::Sound));
    let aiming = Senses {
        ray: aim_at(target(&frame.arrival.expect("the menu").scene, &section).shape),
        ..menu
    };
    step.update(
        &aiming,
        &mut conversation,
        &mut feedback,
        start + HOLD,
        FRAME,
    );
    let pull = Senses {
        trigger: Some(1.0),
        ..aiming
    };
    let frame = step.update(&pull, &mut conversation, &mut feedback, start + HOLD, FRAME);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
    let rows = frame.arrival.expect("the menu").scene;
    assert!(
        rows.object(&arrival::Key::Menu(MenuKey::Sounds)).is_some(),
        "the Sound section is open"
    );
}

/// Hold the ray on the menu's row `key`, as `menu` shows it, for a second of frames, through which it stays on that row; then pull the trigger, which chooses it, and let go.
fn hold_and_choose(
    step: &mut Step,
    conversation: &mut Conversation,
    menu: &Senses,
    key: hand_menu::MenuKey,
    now: Instant,
) {
    let key = arrival::Key::Menu(key);
    let mut feedback = felt();
    let frame = step.update(menu, conversation, &mut feedback, now, FRAME);
    let scene = frame.arrival.expect("the menu").scene;
    let aiming = Senses {
        ray: aim_at(target(&scene, &key).shape),
        ..*menu
    };
    for frame in 0..72 {
        let hovered = step
            .update(&aiming, conversation, &mut feedback, now, FRAME)
            .arrival
            .expect("the menu")
            .hovered;
        assert_eq!(hovered.as_ref(), Some(&key), "frame {frame}");
    }
    let pull = Senses {
        trigger: Some(1.0),
        ..aiming
    };
    step.update(&pull, conversation, &mut feedback, now, FRAME);
    assert_eq!(earcons(&mut feedback), [Earcon::Choose], "{key:?}");
    step.update(&aiming, conversation, &mut feedback, now, FRAME);
}

#[test]
fn a_steady_ray_stays_on_a_menu_row_and_chooses_it() {
    use crate::hand_menu::{MenuKey, Section};
    use tomori_client::{ConnectionState, Event};
    use tomori_protocol::{ClientMessage, EnvironmentInfo, Mode};
    // An owner, in a space whose agent has three modes, each described, the middle one current, on a server that changes environments.
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Owner,
        certificate: None,
        backends: vec![],
        roots: vec![],
        environments: [("sky", "Sky"), ("minimal", "Grid")]
            .map(|(id, label)| EnvironmentInfo {
                id: id.into(),
                label: label.into(),
                version: 1,
            })
            .to_vec(),
        utterance_limit: None,
        name: None,
    }));
    conversation.apply(space(|state| {
        state.modes = [
            (
                "plan",
                "Plans without changing files, and says how it would.",
            ),
            ("build", "Makes changes."),
            ("review", "Reviews the changes made so far."),
        ]
        .map(|(id, description)| Mode {
            id: id.into(),
            name: id.into(),
            description: Some(description.into()),
        })
        .to_vec();
        state.mode = Some("build".into());
        state.speech = tomori_protocol::SpeechSetting::Brief;
        state.environment_changes = true;
    }));
    conversation.take_actions();
    let start = Instant::now();
    let menu = Senses {
        left_palm: Some(left_palm_up(0.0)),
        ..senses()
    };
    let mut step = Step::default();
    step.buttons(
        &press(|buttons| buttons.y = true),
        true,
        &mut conversation,
        &mut felt(),
        start,
    );
    // The modes above and below the current one, each held for a second, stay highlighted and are chosen; pointing at one shows its description beneath the modes without moving a row.
    let sent = |conversation: &mut Conversation, message: ClientMessage| {
        assert_eq!(conversation.take_actions(), [Action::Send(message)]);
    };
    for mode in ["plan", "review"] {
        hold_and_choose(
            &mut step,
            &mut conversation,
            &menu,
            MenuKey::Mode(mode.into()),
            start,
        );
        sent(
            &mut conversation,
            ClientMessage::SetMode { mode: mode.into() },
        );
    }
    // As do the environment the space is not in, and the space's other settings.
    hold_and_choose(
        &mut step,
        &mut conversation,
        &menu,
        MenuKey::Environment("minimal".into()),
        start,
    );
    sent(
        &mut conversation,
        ClientMessage::SetEnvironment {
            environment: "minimal".into(),
        },
    );
    hold_and_choose(&mut step, &mut conversation, &menu, MenuKey::Speech, start);
    sent(
        &mut conversation,
        ClientMessage::SetSpeech {
            speech: tomori_protocol::SpeechSetting::Full,
        },
    );
    // The Sound section's levels, stepped one after another.
    hold_and_choose(&mut step, &mut conversation, &menu, MenuKey::Back, start);
    hold_and_choose(
        &mut step,
        &mut conversation,
        &menu,
        MenuKey::Section(Section::Sound),
        start,
    );
    let feedback = conversation.menu_model().feedback;
    for key in [MenuKey::Sounds, MenuKey::Sounds, MenuKey::Vibration] {
        hold_and_choose(&mut step, &mut conversation, &menu, key, start);
    }
    let stepped = conversation.menu_model().feedback;
    assert_eq!(stepped.sounds, feedback.sounds.next().next());
    assert_eq!(stepped.vibration, feedback.vibration.next());
}

#[test]
fn while_the_menu_shows_in_arrival_the_ray_meets_only_its_rows() {
    let mut conversation = Conversation::new("Ada");
    conversation.set_servers(vec![tomori_client::server_choice::Choice::new(
        "Studio",
        "10.0.0.1:8765",
        tomori_client::server_choice::Source::Discovered,
    )]);
    let mut step = Step::default();
    let now = Instant::now();
    step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
    let frame = step.update(&senses(), &mut conversation, &mut felt(), now, 5.0);
    let server = target(
        &frame.arrival.expect("arrival").scene,
        &arrival::Key::Server("10.0.0.1:8765".into()),
    );
    let menu = Senses {
        left_palm: Some(left_palm_up(0.0)),
        ray: aim_at(server.shape),
        ..senses()
    };
    step.update(&menu, &mut conversation, &mut felt(), now, FRAME);
    let frame = step.update(&menu, &mut conversation, &mut felt(), now + HOLD, FRAME);
    let hovered = frame.arrival.expect("arrival and the menu").hovered;
    assert!(
        !matches!(hovered, Some(arrival::Key::Server(_))),
        "{hovered:?}"
    );
    // Nor does A choose the server behind it.
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut felt(),
        now + HOLD,
    );
    assert!(conversation.take_actions().is_empty());
}

#[test]
fn a_with_nothing_to_choose_is_not_heard() {
    let mut conversation = joined();
    let mut feedback = felt();
    Step::default().buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut feedback,
        Instant::now(),
    );
    assert!(earcons(&mut feedback).is_empty());
}

#[test]
fn in_a_space_the_beam_ends_on_the_nearest_wall_handle_panel_or_object_it_meets() {
    use tomori_protocol::{Panel, Place, Pose, Wall, WallKind};
    let beam = |frame: &Frame<'_>| {
        frame
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .expect("the beam")
    };
    // An environment's wall ahead, to the right, and a panel floating ahead, to the left, nearer.
    let wall = Wall::new(
        "environment-0",
        WallKind::Environment,
        Pose::upright_facing([1.2, 1.5, -2.5], arrival::HOME.to_array()),
    );
    let floating = arrival::HOME + Vec3::new(-0.8, 0.0, -1.5);
    let panel = Panel::new(
        "panel-1",
        "Notes",
        "Ahead.",
        Place::Free {
            pose: Pose::upright_facing(floating.to_array(), arrival::HOME.to_array()),
        },
    );
    let board = Vec3::new(1.2, 1.5 + 0.42, -2.5);
    let run = |role: ConnectionRole| {
        let mut conversation = lobby(role);
        fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], vec![panel.clone()]);
        (conversation, Step::default())
    };
    let now = Instant::now();
    let mut senses = Senses {
        beam: true,
        grip: Some(0.0),
        ..senses()
    };
    let (mut conversation, mut step) = run(ConnectionRole::Owner);
    // At the wall's board, which a participant may take hold of: it ends there, as it does on a panel.
    senses.ray = Ray::new(arrival::HOME, board - arrival::HOME);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let ends = beam(&frame);
    assert!(
        ends.hit && ends.end.distance(board) < 1e-3,
        "{:?}",
        ends.end
    );
    senses.ray = Ray::new(arrival::HOME, floating - arrival::HOME);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let ends = beam(&frame);
    assert!(
        ends.hit && ends.end.distance(floating) < 1e-3,
        "{:?}",
        ends.end
    );
    // At the wall's bar, below it, which takes hold of it as its board does: it ends there.
    let bar = crate::projection::editing::bar(&wall, crate::board::model(&wall.pose).unwrap());
    senses.ray = Ray::new(arrival::HOME, bar.middle - arrival::HOME);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let ends = beam(&frame);
    assert!(
        ends.hit && ends.end.distance(bar.middle) < 0.01,
        "{:?} {:?}",
        ends.end,
        bar.middle
    );
    // The bar is Tomori's, so the environment takes neither the trigger nor a stick there.
    assert_eq!(step.hands.right.aim, precedence::Aim::Tomori);
    // Taken hold of, at a handle, at the handle, however near the board behind it.
    senses.ray = Ray::new(arrival::HOME, board - arrival::HOME);
    senses.grip = Some(1.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    senses.grip = Some(0.0);
    step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.projector.editing_wall(), Some("environment-0"));
    let frame = crate::board::model(&wall.pose).unwrap();
    let (corner, _) = crate::projection::editing::handle_point(
        &wall,
        frame,
        crate::projection::editing::Handle::Corner(1, 1),
    );
    senses.ray = Ray::new(arrival::HOME, corner - arrival::HOME);
    let drawn = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let ends = beam(&drawn);
    assert!(
        ends.hit && ends.end.distance(corner) < 1e-3,
        "{:?} {corner}",
        ends.end
    );
    // A visitor takes hold of no wall: the ray passes its board and ends short of it.
    let (mut conversation, mut step) = run(ConnectionRole::Visitor);
    senses.ray = Ray::new(arrival::HOME, board - arrival::HOME);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(!beam(&frame).hit);
    // An environment's object a metre away, nearer than the wall, ends it there.
    let (mut conversation, mut step) = run(ConnectionRole::Owner);
    let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let factory: crate::client_side::Factory = std::sync::Arc::new(move |_: &str| {
        Box::new(Fake {
            frames: 0,
            told: told.clone(),
        }) as Box<dyn crate::client_side::ClientSide>
    });
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    conversation.apply(space(|state| {
        state.projection = true;
        state.walls = true;
    }));
    for _ in 0..400 {
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    }
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let ends = beam(&frame);
    let ray = senses.ray.unwrap();
    assert!(
        ends.hit && ends.end.distance(ray.at(1.0)) < 1e-3,
        "{:?}",
        ends.end
    );
}

#[test]
fn each_ray_is_drawn_in_full_while_it_has_something_to_say_a_stub_otherwise_and_hidden_hanging_down_or_palm_up()
 {
    let mut conversation = joined();
    let mut step = Step::default();
    let mut feedback = felt();
    let now = Instant::now();
    let ahead = Ray::new(arrival::HOME, -Vec3::Z);
    let left_ahead = Ray::new(arrival::HOME + Vec3::new(-0.2, 0.0, 0.0), -Vec3::Z);
    let pointing = |trigger: f32, left: Option<Ray>, left_trigger: f32| Senses {
        ray: ahead,
        trigger: Some(trigger),
        left_ray: left,
        left_trigger: Some(left_trigger),
        beam: true,
        ..senses()
    };
    let rays = |frame: &Frame<'_>| {
        frame
            .arrival
            .as_ref()
            .map_or((None, None), |arrival| (arrival.ray, arrival.left_ray))
    };
    // Meeting nothing, each ray is a short stub from its controller, plain.
    let frame = step.update(
        &pointing(0.0, left_ahead, 0.0),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    let (right, left) = rays(&frame);
    let (right, left) = (right.expect("the right ray"), left.expect("the left ray"));
    assert!(!right.hit && !left.hit);
    assert_eq!(right.end, ahead.unwrap().at(STUB));
    assert_eq!(left.end, left_ahead.unwrap().at(STUB));
    assert_eq!(
        frame.presence.map(|presence| presence.rays),
        Some([true, true])
    );
    // A pull of either trigger draws its ray in full, as before, and a second longer once let go.
    let frame = step.update(
        &pointing(1.0, left_ahead, 0.0),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    let (right, left) = rays(&frame);
    assert_eq!(
        right.unwrap().end,
        ahead.unwrap().at(crate::pointing::SPACE_RAY_LENGTH)
    );
    assert_eq!(left.unwrap().end, left_ahead.unwrap().at(STUB));
    let soon = now + LIT / 2;
    let frame = step.update(
        &pointing(0.0, left_ahead, 1.0),
        &mut conversation,
        &mut feedback,
        soon,
        FRAME,
    );
    let (right, left) = rays(&frame);
    assert_eq!(
        right.unwrap().end,
        ahead.unwrap().at(crate::pointing::SPACE_RAY_LENGTH)
    );
    assert_eq!(
        left.unwrap().end,
        left_ahead.unwrap().at(crate::pointing::SPACE_RAY_LENGTH)
    );
    let later = now + LIT * 2;
    let frame = step.update(
        &pointing(0.0, left_ahead, 0.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    let (right, left) = rays(&frame);
    assert_eq!(right.unwrap().end, ahead.unwrap().at(STUB));
    assert_eq!(left.unwrap().end, left_ahead.unwrap().at(STUB));
    // Hanging down, past 50° below the horizontal, the left ray is hidden, and shown as not shown; it shows again
    // only once raised within 45°, so that a hand at the edge does not flicker.
    let down = |degrees: f32| {
        Ray::new(
            arrival::HOME + Vec3::new(-0.2, -0.5, 0.0),
            glam::Quat::from_rotation_x(-degrees.to_radians()) * -Vec3::Z,
        )
    };
    let frame = step.update(
        &pointing(0.0, down(60.0), 0.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    assert_eq!(rays(&frame).1, None);
    assert_eq!(
        frame.presence.map(|presence| presence.rays),
        Some([false, true])
    );
    let frame = step.update(
        &pointing(0.0, down(48.0), 0.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    assert_eq!(rays(&frame).1, None, "within the leeway");
    let frame = step.update(
        &pointing(0.0, down(40.0), 0.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    assert!(rays(&frame).1.is_some());
    // Held down while its trigger pulls, it shows: the hand is busy.
    step.update(
        &pointing(0.0, down(60.0), 0.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    let frame = step.update(
        &pointing(0.0, down(60.0), 1.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    assert!(rays(&frame).1.is_some());
    // Untracked, it is not drawn, nor shown.
    let frame = step.update(
        &pointing(0.0, None, 0.0),
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    assert_eq!(rays(&frame).1, None);
    assert_eq!(
        frame.presence.map(|presence| presence.rays),
        Some([false, true])
    );
    // With the left palm up showing the hand menu, the left ray is hidden, and the right is the menu's.
    let palm = Senses {
        left_palm: Some(left_palm_up(0.0)),
        ..pointing(0.0, left_ahead, 0.0)
    };
    step.update(&palm, &mut conversation, &mut feedback, later, FRAME);
    let shown = later + crate::hand_menu::HOLD;
    let frame = step.update(&palm, &mut conversation, &mut feedback, shown, FRAME);
    let (right, left) = rays(&frame);
    assert!(right.is_some());
    assert_eq!(left, None);
    assert_eq!(
        frame.presence.map(|presence| presence.rays),
        Some([false, true])
    );
    assert!(step.menu_was_shown);
    // The desktop's window, which draws no ray, draws neither.
    let mut step = Step::default();
    let frame = step.update(
        &Senses {
            beam: false,
            ..pointing(0.0, left_ahead, 0.0)
        },
        &mut conversation,
        &mut feedback,
        later,
        FRAME,
    );
    assert_eq!(rays(&frame), (None, None));
}

#[test]
fn the_recording_indicator_and_the_live_badge_show_on_the_left() {
    let mut conversation = joined();
    let mut step = Step::default();
    let now = Instant::now();
    let grip = Pose {
        position: [-0.2, 1.1, -0.35],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    let senses = Senses {
        trackers: Trackers {
            grip: Some(grip),
            ..Default::default()
        },
        ..senses()
    };
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(frame.panels.indicator, [Sheet::default(), Sheet::default()]);
    conversation.toggle_live();
    assert!(conversation.live());
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let [card, badge] = &frame.panels.indicator;
    assert_eq!(card, &Sheet::default());
    assert!(
        badge.canvas.is_some() && badge.model.is_some(),
        "the live badge"
    );
    conversation.stop_live();
    step.buttons(
        &press(|buttons| buttons.talk = true),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let [card, badge] = &frame.panels.indicator;
    assert!(card.canvas.is_some(), "the recording card");
    // It rises from the left controller.
    let at = card.model.expect("drawn").transform_point3(Vec3::ZERO);
    assert!(at.distance(Vec3::from_array(grip.position)) < 0.2, "{at}");
    assert!(at.y > grip.position[1]);
    assert_eq!(badge, &Sheet::default());
    // With the live microphone on too, only the card shows.
    conversation.toggle_live();
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    let [card, badge] = &frame.panels.indicator;
    assert!(card.model.is_some());
    assert_eq!(badge, &Sheet::default());
}

/// The right grip for an object of an environment's that takes it.
mod grab;

/// The right stick's push for an object of an environment's that takes it.
mod scroll;

/// The space's timers, drawn as dials and pointed at.
mod timers;

/// The left hand's ray, trigger and grip, which mirror the right's for Tomori's own things.
mod hands;

/// Volumes, pointed at as words begin.
mod volumes;
