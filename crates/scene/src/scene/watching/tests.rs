// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The frame step in watcher mode: its parity with a participant's frame step, and what a watcher does not do.

use super::super::fixtures::*;
use super::*;
use crate::{arrival, client_side::ClientSide, sides::Side};
use glam::Vec3;
use std::sync::{Arc, Mutex};
use tomori_client::Event;
use tomori_protocol::{ConnectionRole, Participant, ServerMessage};

const FRAME: f32 = 1.0 / 72.0;

/// The clock both steps count timers down by, so that their dials agree.
const UNIX: f64 = 1_800_000_000.0;

/// What a client side was told, frame by frame.
#[derive(Debug, Default)]
struct Told {
    updates: Vec<usize>,
    pointed: Vec<Option<String>>,
    heads: Vec<Option<Vec3>>,
    participant: Vec<Option<u32>>,
    hands: Vec<Vec<u32>>,
    controls: bool,
}

/// A client side that asks, sounds and passes every frame, and shows how many frames it has had.
struct Asking {
    frames: usize,
    told: Arc<Mutex<Told>>,
}

impl ClientSide for Asking {
    fn pick(&self, _ray: &crate::pointing::Ray) -> Option<crate::client_side::Picked> {
        Some(crate::client_side::Picked::new("thing", "the thing", 1.0).selectable())
    }

    fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
        self.frames += 1;
        let mut told = self.told.lock().unwrap();
        told.updates.push(input.updates.len());
        told.pointed.push(input.pointed.map(str::to_owned));
        told.heads.push(input.head);
        told.participant.push(input.participant);
        told.hands
            .push(input.hands.iter().map(|(source, _)| *source).collect());
        told.controls |= input.select.is_some()
            || input.back
            || input.scroll.is_some()
            || input.grabs().next().is_some()
            || input.left_stick != glam::Vec2::ZERO
            || input.right_stick != glam::Vec2::ZERO;
        crate::client_side::Output::new()
            .with_intent(tomori_protocol::Payload::new(serde_json::json!(
                self.frames
            )))
            .with_sound(
                crate::client_side::Sound::new(vec![0.0; 4], 48_000),
                [0.0, 1.0, -1.0],
            )
            .with_pass(crate::client_side::Side::Right)
            .drawing(Arc::new(self.frames))
    }
}

/// Register [`Asking`] for Sky with `conversation`, returning what it is told and how many have been made.
fn registered(conversation: &mut Conversation) -> (Arc<Mutex<Told>>, Arc<Mutex<usize>>) {
    let told = Arc::new(Mutex::new(Told::default()));
    let made = Arc::new(Mutex::new(0));
    let factory: crate::client_side::Factory = {
        let (told, made) = (told.clone(), made.clone());
        Arc::new(move |space: &str| {
            assert_eq!(space, SPACE);
            *made.lock().unwrap() += 1;
            Box::new(Asking {
                frames: 0,
                told: told.clone(),
            }) as Box<dyn ClientSide>
        })
    };
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    (told, made)
}

/// A space in Sky, as `role`, with everything a frame can show of it: a client side and an ambience, walls an owner may
/// change, with panels on them, the GitHub environment's cards, workers, a timer and another participant.
fn furnished(role: ConnectionRole) -> Conversation {
    use crate::ambience::{Ambience, Loop, Slot};
    let mut conversation = lobby(role);
    registered(&mut conversation);
    conversation.set_ambiences(vec![(
        crate::test_support::SKY,
        Arc::new(Ambience::new().with_loop(Loop::at(Slot::missing("waterfall"), [0.0, 2.0, 0.0]))),
    )]);
    let panels = (0..3)
        .map(|slot| {
            tomori_protocol::Panel::new(
                format!("panel-{slot}"),
                "Notes",
                "**Published** on the wall.",
                tomori_protocol::Place::Wall {
                    wall: tomori_protocol::TOMORI_WALL.into(),
                    slot,
                },
            )
        })
        .collect();
    show_editable_walls(&mut conversation, Vec::new(), panels);
    show_cards(
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
    conversation.apply(space(|state| {
        state.projection = true;
        state.walls = true;
        state.groups = true;
        state.cards = true;
        state.activity = tomori_protocol::Activity::Running;
        state.participants.push(Participant {
            name: "Bea".into(),
            source: 2,
            engagement: None,
        });
        state.workers = vec![tomori_protocol::Worker {
            id: "ses_a".into(),
            parent: None,
            title: "Review".into(),
            state: tomori_protocol::WorkerState::Running,
        }];
        state.timers = vec![tomori_protocol::Timer::new(
            "standup",
            "weekdays at 09:00",
            "when present",
            Some(UNIX as u64 + 3 * 3600),
            tomori_protocol::TimerState::Paused,
        )];
    }));
    conversation.apply(Event::Message(ServerMessage::Presence {
        source: 2,
        head: tomori_protocol::Pose::HOME,
        left: None,
        right: Some(tomori_protocol::Pose::HOME),
        hands: None,
        pointing: None,
    }));
    conversation.take_actions();
    conversation
}

/// A step whose timers count down by [`UNIX`].
fn clocked() -> Step {
    let mut step = Step {
        controls: crate::projection::Controls::Keyboard,
        ..Step::default()
    };
    step.set_clock(|| UNIX);
    step
}

/// The desktop window's head at home, looking ahead, as a participant's starts.
fn camera() -> Mat4 {
    super::super::desktop::Head::default().transform()
}

/// What the desktop window senses of a participant at home who does nothing: its head, no cursor in the window, the
/// mouse's button and the grip let go, and the recording indicator's place low in the view.
fn participant() -> Senses {
    let head = camera();
    Senses {
        focused: true,
        head: Some(head),
        head_pose: Pose::from_transform(head),
        viewer: Some(arrival::HOME),
        space_head: head,
        trigger: Some(0.0),
        grip: Some(0.0),
        teleport: Side::Right,
        trackers: crate::recording::Trackers {
            view: Some(crate::controller_ui::view_spot(head)),
            ..Default::default()
        },
        marker: Marker::WhileAttracting,
        ..Senses::default()
    }
}

/// The elements of `frame` by kind, for comparing.
fn elements(frame: &Frame) -> std::collections::BTreeMap<Element, String> {
    frame.elements().into_iter().collect()
}

#[test]
fn a_watcher_draws_what_a_participant_does_but_what_is_theirs_alone() {
    // A participant and a visitor in the space, each as the desktop window steps them, and a watcher with an owner's
    // token, all seeing from the same place, at the same instants.
    let mut owner = furnished(ConnectionRole::Owner);
    let mut visitor = furnished(ConnectionRole::Visitor);
    let mut watched = furnished(ConnectionRole::Owner);
    let (mut owner_step, mut visitor_step, mut watcher) = (clocked(), clocked(), clocked());
    let mut watcher_feedback = felt();
    let start = Instant::now();
    let mut compared = 0;
    for index in 1..=480u32 {
        let now = start + std::time::Duration::from_secs_f32(FRAME * index as f32);
        let senses = participant();
        let theirs = owner_step.update(&senses, &mut owner, &mut felt(), now, FRAME);
        let (theirs, presence) = (elements(&theirs), theirs.presence.is_some());
        let visitors =
            elements(&visitor_step.update(&senses, &mut visitor, &mut felt(), now, FRAME));
        let frame = watcher.watch(camera(), &mut watched, &mut watcher_feedback, now, FRAME);
        let seen = elements(&frame);
        // Everything a participant's frame holds the watcher's holds too, but what is personal, which it leaves out.
        for (element, held) in &seen {
            if PERSONAL.contains(element) {
                continue;
            }
            assert_eq!(held, &visitors[element], "{element:?} at frame {index}");
        }
        assert!(frame.presence.is_none() && frame.panels.pill.model.is_none());
        assert!(frame.arrival.is_none() && frame.pads.is_empty() && frame.landing.is_none());
        if index % 120 == 0 {
            compared += 1;
            // The participant reports their presence and sees the pill, as the owner sees their wall's handles, which
            // the watcher, an owner too, does not.
            assert!(presence);
            assert_ne!(theirs[&Element::Pill], seen[&Element::Pill]);
            assert_ne!(theirs[&Element::Projection], seen[&Element::Projection]);
        }
        if index == 480 {
            // So the comparison is of a space in full: walls and panels, cards, workers, a dial and the client side.
            assert!(frame.projection.len() > 3);
            assert!(!frame.cards.is_empty());
            assert!(!frame.cubes.is_empty() && !frame.stations.is_empty());
            assert_eq!(frame.dials.len(), 1);
            assert!(frame.environment.is_some());
            assert_eq!(frame.scenery.environment, crate::test_support::SKY);
            // And it hears from the camera, as the participant does from their head.
            assert_eq!(frame.listener, Pose::from_transform(camera()));
        }
    }
    assert_eq!(compared, 4);
    // It played no ambience, and none of its client side's sounds, and felt none of its passes.
    assert!(watcher_feedback.take_ambience().is_none());
    assert_eq!(watcher_feedback.take_ambient().count(), 0);
    assert!(
        watcher_feedback
            .haptics(start + std::time::Duration::from_secs(7))
            .is_empty()
    );
}

#[test]
fn a_watcher_s_swarm_shows_the_server_s_agent_state_as_a_participant_s_does() {
    use crate::interaction::Mode;
    use tomori_protocol::{AgentState, AgentStatus};
    let mut watched = furnished(ConnectionRole::Owner);
    let mut participant_view = furnished(ConnectionRole::Owner);
    let (mut watcher, mut step) = (clocked(), clocked());
    let start = Instant::now();
    for (sequence, (state, mode)) in (1..).zip([
        (AgentState::Waiting, Mode::Waiting),
        (AgentState::Error, Mode::Unavailable),
        (AgentState::Listening, Mode::Listening),
    ]) {
        // The session says it runs, as an older client would show, while the server says what the agent is doing.
        for conversation in [&mut watched, &mut participant_view] {
            conversation.apply(space(|space| {
                space.activity = tomori_protocol::Activity::Running;
                space.agent = Some(AgentStatus {
                    state,
                    detail: None,
                    sequence,
                });
            }));
        }
        let now = start + std::time::Duration::from_secs(sequence);
        let watched_mode = watcher
            .watch(camera(), &mut watched, &mut felt(), now, FRAME)
            .swarm
            .visual
            .mode;
        let their_mode = step
            .update(
                &participant(),
                &mut participant_view,
                &mut felt(),
                now,
                FRAME,
            )
            .swarm
            .visual
            .mode;
        assert_eq!((watched_mode, their_mode), (mode, mode), "{state:?}");
    }
}

#[test]
fn every_element_of_a_frame_is_shared_or_named_personal() {
    // What a watcher leaves out is exactly this: adding to it hides a scene feature from the spectator, which needs
    // its own review and a change to `design/spectator.md`.
    assert_eq!(
        PERSONAL,
        [
            Element::Marker,
            Element::Attracting,
            Element::Arrival,
            Element::Pill,
            Element::Indicator,
            Element::Wrist,
            Element::Presence,
            Element::Landing,
            Element::Blink,
            Element::Pads,
        ]
    );
    let mut conversation = joined();
    let mut step = Step::default();
    let mut frame = step.watch(
        camera(),
        &mut conversation,
        &mut felt(),
        Instant::now(),
        FRAME,
    );
    let elements = frame.elements().map(|(element, _)| element);
    let mut unique = elements.to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), elements.len());
    // Only the personal elements can be left out, and each of them can.
    for element in elements {
        assert_eq!(
            frame.withhold(element),
            PERSONAL.contains(&element),
            "{element:?}"
        );
    }
}

#[test]
fn a_watcher_runs_the_client_side_of_the_environment_it_draws_and_tells_it_nothing_a_participant_does()
 {
    crate::test_support::log_everything();
    let mut conversation = lobby(ConnectionRole::Visitor);
    let (told, made) = registered(&mut conversation);
    let mut step = Step::default();
    let now = Instant::now();
    // Outside a space, none.
    let camera = Mat4::from_translation(Vec3::new(1.0, 1.6, 2.0));
    assert!(
        step.watch(camera, &mut conversation, &mut felt(), now, FRAME)
            .environment
            .is_none()
    );
    conversation.apply(space(|state| {
        state.participants.push(Participant {
            name: "Bea".into(),
            source: 2,
            engagement: None,
        });
    }));
    // Once its scenery has grown in, it is made once and shows what it shows each frame.
    let mut shown = Vec::new();
    for _ in 0..400 {
        shown.push(
            step.watch(camera, &mut conversation, &mut felt(), now, FRAME)
                .environment
                .map(|drawn| *drawn.downcast_ref::<usize>().unwrap()),
        );
    }
    let frames = shown.last().unwrap().unwrap();
    assert!(frames > 1);
    assert_eq!(*made.lock().unwrap(), 1);
    // What the server side sends reaches it, and the others' hands, from the camera, pointing at nothing.
    conversation.apply(Event::Message(ServerMessage::EnvironmentState {
        environment: "sky".into(),
        version: 1,
        state: tomori_protocol::Payload::new(serde_json::json!({})),
    }));
    conversation.apply(Event::Message(ServerMessage::Presence {
        source: 2,
        head: tomori_protocol::Pose::HOME,
        left: None,
        right: Some(tomori_protocol::Pose::HOME),
        hands: None,
        pointing: None,
    }));
    step.watch(camera, &mut conversation, &mut felt(), now, FRAME);
    {
        let told = told.lock().unwrap();
        assert_eq!(told.updates.last(), Some(&1));
        assert!(told.pointed.iter().all(Option::is_none));
        assert_eq!(told.heads.last(), Some(&Some(Vec3::new(1.0, 1.6, 2.0))));
        assert_eq!(told.hands.last(), Some(&vec![2]));
        assert!(told.participant.iter().all(Option::is_none));
        assert!(!told.controls);
    }
    // Leaving the environment drops it, and coming back makes it anew.
    conversation.apply(space(|state| state.environment = "minimal".into()));
    assert!(
        step.watch(camera, &mut conversation, &mut felt(), now, FRAME)
            .environment
            .is_none()
    );
    conversation.apply(space(|_| {}));
    for _ in 0..400 {
        step.watch(camera, &mut conversation, &mut felt(), now, FRAME);
    }
    assert_eq!(*made.lock().unwrap(), 2);
    // Though its object is there to point at, the watcher pointed at nothing.
    let side = Asking {
        frames: 0,
        told: told.clone(),
    };
    let ray = crate::pointing::Ray::new(Vec3::new(1.0, 1.6, 2.0), Vec3::NEG_Z).unwrap();
    assert!(side.pick(&ray).is_some());
}

#[test]
fn a_watcher_s_client_side_asks_its_server_side_nothing_whatever_its_role() {
    crate::test_support::log_everything();
    for role in [ConnectionRole::Visitor, ConnectionRole::Owner] {
        let mut conversation = lobby(role);
        registered(&mut conversation);
        conversation.apply(space(|_| {}));
        // The server has sent the environment's state, as it does before it takes intents.
        conversation.apply(Event::Message(ServerMessage::EnvironmentState {
            environment: "sky".into(),
            version: 1,
            state: tomori_protocol::Payload::new(serde_json::json!({})),
        }));
        conversation.take_actions();
        let mut step = Step::default();
        let mut shown = 0;
        for _ in 0..400 {
            shown += usize::from(
                step.watch(
                    camera(),
                    &mut conversation,
                    &mut felt(),
                    Instant::now(),
                    FRAME,
                )
                .environment
                .is_some(),
            );
        }
        assert!(shown > 1);
        assert_eq!(step.dropped(), shown);
        let actions = conversation.take_actions();
        assert!(!format!("{actions:?}").contains("EnvironmentIntent"));
    }
}

#[test]
fn a_frame_names_the_avatar_it_shows_among_its_elements() {
    use crate::avatar::{Avatar, Input};
    /// A figure at the agent's home, showing nothing of its own.
    struct Figure;
    impl Avatar for Figure {
        fn update(&mut self, _input: &Input) {}
        fn voices(&self, _voices: &mut Vec<[f32; 3]>) {}
        fn departures(&self, _departures: &mut Vec<[f32; 3]>) {}
        fn bounds(&self) -> crate::arrival_motion::Sphere {
            crate::arrival_motion::Sphere {
                centre: crate::swarm::CENTER,
                radius: 0.5,
            }
        }
    }
    let mut conversation = lobby(ConnectionRole::Visitor);
    conversation.set_avatars(vec![(
        "figure".into(),
        Arc::new(|_: &str| Box::new(Figure) as Box<dyn Avatar>),
    )]);
    conversation.apply(space(|state| state.avatar = Some("figure".into())));
    let mut step = Step::default();
    let frame = step.watch(
        camera(),
        &mut conversation,
        &mut felt(),
        Instant::now(),
        FRAME,
    );
    assert_eq!(
        elements(&frame)[&Element::Avatar],
        format!("{:?}", Some(("figure", SPACE, false)))
    );
}
