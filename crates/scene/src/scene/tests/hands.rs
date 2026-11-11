// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The left hand mirrors the right for Tomori's own things: its ray points at them, its trigger chooses what the
//! right's chooses, and its grip takes and carries what the right's takes, each hand its own, while the buttons act on
//! what the right ray is on, or else the left's, and words begun name what both rays are on. See
//! `design/interaction.md`.

use super::*;
use crate::sides::Side as Hand;
use tomori_protocol::{ClientMessage, Panel, Place, Pose as Placed, Resource, ServerMessage};

/// Where the two floating panels stand: one straight ahead and one ahead to the left.
const AHEAD: Vec3 = Vec3::new(0.0, 1.6, -3.0);
const ASIDE: Vec3 = Vec3::new(-1.6, 1.6, -3.0);

/// A space in which an owner sees two floating panels, `panel-1` ahead and `panel-2` to the left.
fn two_panels() -> Conversation {
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.apply(space(|state| state.projection = true));
    let facing_home = |at: Vec3| Placed::upright_facing(at.to_array(), arrival::HOME.to_array());
    conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
        projection: Box::new(
            tomori_protocol::Projection::new(SPACE, "sky")
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![
                    Panel::new(
                        "panel-1",
                        "Build status",
                        "Green.",
                        Place::Free {
                            pose: facing_home(AHEAD),
                        },
                    ),
                    Panel::new(
                        "panel-2",
                        "Agenda",
                        "Ship it.",
                        Place::Free {
                            pose: facing_home(ASIDE),
                        },
                    ),
                ]),
        ),
    }));
    conversation.take_actions();
    conversation
}

/// The senses of a head at home, the right ray towards `right` and the left towards `left`, either none, with each
/// grip squeezed as far as given.
fn rays(right: Option<Vec3>, left: Option<Vec3>, grips: (f32, f32)) -> Senses {
    let from =
        |towards: Option<Vec3>| towards.and_then(|at| Ray::new(arrival::HOME, at - arrival::HOME));
    Senses {
        beam: true,
        ray: from(right),
        left_ray: from(left),
        grip: Some(grips.0),
        left_grip: Some(grips.1),
        left_trigger: Some(0.0),
        ..senses()
    }
}

/// What words begun now are about, as the conversation sends them.
fn attending(conversation: &mut Conversation) -> Vec<Resource> {
    conversation.talk(true);
    conversation.talk(false);
    let actions = conversation.take_actions();
    let Some(Action::BeginUtterance(utterance)) = actions.first() else {
        unreachable!("recording begins, not {actions:?}")
    };
    utterance.attention.clone()
}

/// The plain text of each sheet of `frame`'s projection that says what letting go does: the hints beside panels
/// carried.
fn hints(frame: &Frame<'_>) -> Vec<String> {
    frame
        .projection
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref().map(crate::canvas::Canvas::plain))
        .filter(|text| text.contains("Let go"))
        .collect()
}

#[test]
fn the_left_grip_pulls_off_carries_and_lets_go_of_a_panel_as_the_right_does_each_hand_its_own() {
    let mut conversation = two_panels();
    let mut step = Step::default();
    let now = Instant::now();
    let mut feedback = felt();
    // The right ray points at the panel ahead; then the left points at the panel to the left, the label on it naming
    // it, as the right's names the one ahead.
    step.update(
        &rays(Some(AHEAD), None, (0.0, 0.0)),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    felt_on(&mut feedback, Side::Right, now);
    let now = now + Duration::from_secs(1);
    let pointing = rays(Some(AHEAD), Some(ASIDE), (0.0, 0.0));
    let frame = step.update(&pointing, &mut conversation, &mut feedback, now, FRAME);
    let labels: Vec<String> = frame
        .projection
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref().map(crate::canvas::Canvas::plain))
        .filter(|text| text.contains("ago, floating"))
        .collect();
    assert_eq!(labels.len(), 2, "{labels:?}");
    let left = frame
        .arrival
        .as_ref()
        .and_then(|arrival| arrival.left_ray)
        .expect("the left ray");
    assert!(left.hit && left.end.distance(ASIDE) < 1e-2, "{left:?}");
    assert_eq!(step.hands[Hand::Left].aim, precedence::Aim::Tomori);
    assert_eq!(step.hands[Hand::Left].panel.as_deref(), Some("panel-2"));
    // Entering it ticks on the left hand, which pointed, and not on the right.
    let ticks = feedback.haptics(now + Duration::from_secs(1));
    assert!(
        ticks.iter().any(|(hand, _)| *hand == Side::Left),
        "{ticks:?}"
    );
    assert!(
        ticks.iter().all(|(hand, _)| *hand == Side::Left),
        "{ticks:?}"
    );
    // The left grip pulls it off, as the right's would, and the right grip pulls off the one ahead.
    step.update(
        &rays(Some(AHEAD), Some(ASIDE), (0.0, 1.0)),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    assert_eq!(step.last_grab, Some((Hand::Left, precedence::Grab::Tomori)));
    assert_eq!(step.projector.held_in(Hand::Left), Some("panel-2"));
    let frame = step.update(
        &rays(Some(AHEAD), Some(ASIDE), (1.0, 1.0)),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    // Both are told to the server, the right's first, the first for older servers.
    let presence = frame.presence.expect("presence");
    assert_eq!(presence.carried, ["panel-1", "panel-2"]);
    assert_eq!(step.projector.held_in(Hand::Right), Some("panel-1"));
    // Carried along the left ray, the left's hint says only what letting go does, and the right's the controls too,
    // which act on what the right hand carries.
    let moved = Vec3::new(-1.0, 1.4, -2.0);
    step.update(
        &rays(Some(AHEAD), Some(moved), (1.0, 1.0)),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    let frame = step.update(
        &rays(Some(AHEAD), Some(moved), (1.0, 1.0)),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    let hints = hints(&frame);
    assert_eq!(hints.len(), 2, "{hints:?}");
    assert_eq!(
        hints
            .iter()
            .filter(|hint| hint.contains("throw away"))
            .count(),
        1
    );
    // B throws away the right hand's, and the left's stays in the left hand.
    step.buttons(
        &press(|buttons| {
            buttons.back = true;
            buttons.back_held = true;
        }),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    step.buttons(
        &Buttons::default(),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    assert_eq!(step.projector.held_in(Hand::Right), None);
    assert_eq!(step.projector.held_in(Hand::Left), Some("panel-2"));
    assert!(conversation.take_actions().iter().any(|action| matches!(
        action,
        Action::Send(ClientMessage::DismissPanel { panel, .. }) if panel == "panel-1"
    )));
    // Let go, the left's floats where the left hand left it, for everyone.
    step.update(
        &rays(Some(AHEAD), Some(moved), (0.0, 0.0)),
        &mut conversation,
        &mut feedback,
        now,
        FRAME,
    );
    assert_eq!(step.projector.held_in(Hand::Left), None);
    assert!(matches!(
        conversation.take_actions().as_slice(),
        [Action::Send(ClientMessage::PlacePanel { panel, wall: None, slot: None, pose: Some(_) })] if panel == "panel-2"
    ));
}

#[test]
fn a_second_grip_on_the_panel_the_other_hand_carries_is_nobody_s() {
    let mut conversation = two_panels();
    let mut step = Step::default();
    let now = Instant::now();
    step.update(
        &rays(Some(AHEAD), None, (0.0, 0.0)),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    step.update(
        &rays(Some(AHEAD), None, (1.0, 0.0)),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(step.projector.held_in(Hand::Right), Some("panel-1"));
    // The left ray on the panel the right carries ends on it; its grip takes nothing, and the panel stays in the right.
    let frame = step.update(
        &rays(Some(AHEAD), Some(AHEAD), (1.0, 0.0)),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert!(
        frame
            .arrival
            .and_then(|arrival| arrival.left_ray)
            .is_some_and(|ray| ray.hit)
    );
    step.update(
        &rays(Some(AHEAD), Some(AHEAD), (1.0, 1.0)),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(step.projector.held_in(Hand::Left), None);
    assert_eq!(step.projector.held_in(Hand::Right), Some("panel-1"));
}

#[test]
fn words_begun_name_what_both_shown_rays_are_on_the_most_recently_pointed_first() {
    let mut conversation = two_panels();
    let mut step = Step::default();
    let now = Instant::now();
    let at = |seconds: u64| now + Duration::from_secs(seconds);
    // The right ray on the panel ahead, the left on nothing: only that one.
    step.update(
        &rays(Some(AHEAD), None, (0.0, 0.0)),
        &mut conversation,
        &mut felt(),
        at(0),
        FRAME,
    );
    assert_eq!(attending(&mut conversation), [Resource::panel("panel-1")]);
    // The left ray moves onto the other one since: it is named first.
    step.update(
        &rays(Some(AHEAD), Some(ASIDE), (0.0, 0.0)),
        &mut conversation,
        &mut felt(),
        at(1),
        FRAME,
    );
    assert_eq!(
        attending(&mut conversation),
        [Resource::panel("panel-2"), Resource::panel("panel-1")]
    );
    // The right ray leaves and comes back: now it is the more recent.
    step.update(
        &rays(None, Some(ASIDE), (0.0, 0.0)),
        &mut conversation,
        &mut felt(),
        at(2),
        FRAME,
    );
    assert_eq!(attending(&mut conversation), [Resource::panel("panel-2")]);
    step.update(
        &rays(Some(AHEAD), Some(ASIDE), (0.0, 0.0)),
        &mut conversation,
        &mut felt(),
        at(3),
        FRAME,
    );
    assert_eq!(
        attending(&mut conversation),
        [Resource::panel("panel-1"), Resource::panel("panel-2")]
    );
    // Both on one panel, it is named once.
    step.update(
        &rays(Some(AHEAD), Some(AHEAD), (0.0, 0.0)),
        &mut conversation,
        &mut felt(),
        at(4),
        FRAME,
    );
    assert_eq!(attending(&mut conversation), [Resource::panel("panel-1")]);
    // A left hand hanging down points at nothing, however its ray would meet the other panel.
    let mut lowered = rays(Some(AHEAD), None, (0.0, 0.0));
    lowered.left_ray = Ray::new(
        arrival::HOME + Vec3::new(-0.2, -0.6, 0.0),
        Vec3::new(0.0, -1.0, -0.2),
    );
    step.update(&lowered, &mut conversation, &mut felt(), at(5), FRAME);
    assert_eq!(attending(&mut conversation), [Resource::panel("panel-1")]);
}

#[test]
fn a_ticks_the_item_the_right_ray_is_on_or_else_the_one_the_left_ray_is_on() {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Collaborator);
    show_checklist(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let now = Instant::now();
    let on_item =
        ray_on_item(&mut step, &mut conversation, ahead, "A  untick").expect("the ticked item");
    let tick = |actions: Vec<Action>| {
        actions.into_iter().any(|action| {
            matches!(
                action,
                Action::Send(ClientMessage::CheckPanelItem { checked: false, .. })
            )
        })
    };
    // The left ray on the item, the right on nothing: A ticks the left's, and its label says so.
    let left = Senses {
        left_ray: on_item.ray,
        ray: Ray::new(arrival::HOME, Vec3::new(1.0, 0.0, -0.2)),
        ..on_item
    };
    let frame = step.update(&left, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(label_hint(&frame.projection).as_deref(), Some("A  untick"));
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    assert!(tick(conversation.take_actions()));
    // The right ray on the panel above its list, which ticks nothing, takes A first: the left's item is not ticked.
    let right_on_title = (0..60)
        .find_map(|step_down| {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                left_ray: None,
                ..left
            };
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            label_hint(&frame.projection)
                .is_some_and(|hint| !hint.contains("tick"))
                .then_some(Senses {
                    left_ray: left.left_ray,
                    ..senses
                })
        })
        .expect("the panel above its list");
    step.update(&right_on_title, &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.hands[Hand::Right].aim, precedence::Aim::Tomori);
    step.buttons(
        &press(|buttons| buttons.a = true),
        true,
        &mut conversation,
        &mut felt(),
        now,
    );
    assert!(!tick(conversation.take_actions()));
}

#[test]
fn the_left_trigger_chooses_a_row_and_a_pad_as_the_right_does() {
    // A choosable row of an environment's list, chosen by a pull of the left trigger begun on it.
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
    let mut conversation = lobby(ConnectionRole::Collaborator);
    show_queue(&mut conversation, ahead, arrival::HOME);
    let mut step = Step::default();
    let [_, choosable, _] = queue_rows(&mut step, &mut conversation, ahead);
    let now = Instant::now();
    let left = |trigger: f32| Senses {
        left_ray: choosable.ray,
        left_trigger: Some(trigger),
        ray: None,
        ..choosable
    };
    step.update(&left(0.0), &mut conversation, &mut felt(), now, FRAME);
    assert_eq!(step.hands[Hand::Left].aim, precedence::Aim::Tomori);
    let mut feedback = felt();
    step.update(&left(1.0), &mut conversation, &mut feedback, now, FRAME);
    assert!(conversation.take_actions().iter().any(|action| matches!(
        action,
        Action::Send(ClientMessage::ChoosePanelRow { row, .. }) if row == "pod112"
    )));
    // With the choosing click, on the left.
    assert!(!felt_on(&mut feedback, Side::Left, now).is_empty());
    assert!(felt_on(&mut feedback, Side::Right, now).is_empty());
    // A teleport pad, which a pull of the left trigger begun on it goes to the pad paired with it.
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_locomotion(vec![(crate::test_support::SKY, storeys())]);
    conversation.apply(space(|_| {}));
    let mut step = Step::default();
    let pad = Vec3::new(0.0, 0.2, -2.0);
    let on_left = |step: &mut Step, conversation: &Conversation, trigger: f32| {
        let senses = on_pads(step, conversation, pad, 0.0);
        Senses {
            left_ray: senses.ray,
            left_trigger: Some(trigger),
            ray: None,
            ..senses
        }
    };
    for _ in 0..400 {
        let senses = on_left(&mut step, &conversation, 0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    }
    assert_eq!(step.hands[Hand::Left].aim, precedence::Aim::Pad);
    let senses = on_left(&mut step, &conversation, 1.0);
    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
    assert!(!frame.attracting);
    assert_eq!(step.rig(&conversation).floor(), -4.5);
}

#[test]
fn the_left_trigger_chooses_nothing_the_environment_shows_and_its_grip_takes_what_its_ray_is_on() {
    use std::sync::{Arc, Mutex};
    /// A client side whose stick, a metre along any ray, is selectable and takes the grip.
    struct Stick {
        told: Arc<Mutex<Vec<(bool, bool, bool)>>>,
    }
    impl crate::client_side::ClientSide for Stick {
        fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
            (ray.direction.x < 0.0).then(|| {
                crate::client_side::Picked::new("stick", "a stick", 1.0)
                    .selectable()
                    .handles(crate::client_side::Actions::GRAB)
            })
        }
        fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
            self.told.lock().unwrap().push((
                input.select.is_some(),
                input.grab.is_some(),
                input.left_grab.is_some(),
            ));
            crate::client_side::Output::new()
        }
    }
    let told = Arc::new(Mutex::new(Vec::new()));
    let made = told.clone();
    let mut conversation = lobby(ConnectionRole::Owner);
    let factory: crate::client_side::Factory = Arc::new(move |_: &str| {
        Box::new(Stick { told: made.clone() }) as Box<dyn crate::client_side::ClientSide>
    });
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    conversation.apply(space(|_| {}));
    conversation.take_actions();
    let mut step = Step::default();
    let now = Instant::now();
    // The left ray points at the stick, for its words, but the right at nothing.
    let senses = |trigger: f32, grip: f32| Senses {
        beam: true,
        ray: Ray::new(arrival::HOME, Vec3::new(1.0, 0.0, -1.0)),
        left_ray: Ray::new(arrival::HOME, Vec3::new(-1.0, 0.0, -1.0)),
        left_trigger: Some(trigger),
        left_grip: Some(grip),
        ..super::senses()
    };
    for _ in 0..400 {
        step.update(
            &senses(0.0, 0.0),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
    }
    assert_eq!(step.hands[Hand::Left].aim, precedence::Aim::Environment);
    assert_eq!(attending(&mut conversation), [Resource::object("stick")]);
    told.lock().unwrap().clear();
    // Its trigger chooses nothing of it; its grip takes it, and the client side is told of the left grip on it.
    step.update(
        &senses(1.0, 1.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    step.update(
        &senses(1.0, 1.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(
        step.last_grab,
        Some((Hand::Left, precedence::Grab::Environment))
    );
    let told = told.lock().unwrap();
    assert!(told.iter().all(|(select, grab, _)| !select && !grab));
    assert!(told.iter().any(|(_, _, left)| *left));
}

#[test]
fn outside_a_space_either_trigger_draws_the_swarm_towards_its_own_hand_the_first_while_both_do() {
    let mut conversation = lobby(ConnectionRole::Owner);
    let mut step = Step::default();
    let now = Instant::now();
    let (right, left) = (Vec3::new(0.3, 1.2, -0.4), Vec3::new(-0.3, 1.2, -0.4));
    // Both rays upward, at nothing arrival shows.
    let up = Ray::new(arrival::HOME, Vec3::Y);
    let pulling = |right_trigger: f32, left_trigger: f32| Senses {
        beam: true,
        ray: up,
        left_ray: up,
        trigger: Some(right_trigger),
        left_trigger: Some(left_trigger),
        attractor: Some(right),
        left_attractor: Some(left),
        ..senses()
    };
    // At rest in arrival, meeting nothing, each ray is a stub.
    let frame = step.update(
        &pulling(0.0, 0.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert!(!frame.attracting);
    let beams = frame
        .arrival
        .as_ref()
        .map(|arrival| (arrival.ray, arrival.left_ray));
    let stub = up.unwrap().at(STUB);
    assert_eq!(
        beams.map(|(right, left)| (right.map(|beam| beam.end), left.map(|beam| beam.end))),
        Some((Some(stub), Some(stub)))
    );
    // The left trigger draws the swarm to the left hand.
    let frame = step.update(
        &pulling(0.0, 1.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert!(frame.attracting);
    assert_eq!(frame.marker, Some(left));
    // The right pulled too, the left, which began first, keeps it.
    let frame = step.update(
        &pulling(1.0, 1.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(frame.marker, Some(left));
    // The left let go, the right has it.
    let frame = step.update(
        &pulling(1.0, 0.0),
        &mut conversation,
        &mut felt(),
        now,
        FRAME,
    );
    assert_eq!(frame.marker, Some(right));
}
