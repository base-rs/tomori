// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An environment's client side as a spectator runs it, so that its audience sees what the environment shows beyond its
//! scenery, such as the farmyard's chickens. It is made and dropped by the frame step's rule, for the joined space while
//! the spectator draws the environment the client side is registered for, and told each frame what the frame step tells
//! it of the space: what its server side sent, the others' hands, and the head, here the spectator's camera. It is told
//! nothing a participant does, pointing, selecting, the sticks or the grip, and as a visitor it is no participant; what
//! it asks its server side is dropped, whatever the spectator's role, its sounds are not played and its passes are not
//! felt. See `design/spectator.md`.
use std::time::Instant;

use glam::Vec3;

use crate::anchors::Anchor;
use crate::client_side::{ClientSide, Drawn, Input};
use crate::conversation::Conversation;
use crate::environment::Environment;

/// The client side running for the joined space, and what it was made for.
struct Running {
    space: String,
    environment: Environment,
    side: Box<dyn ClientSide>,
    /// Whether it has asked its server side anything, which is dropped, as its log says once.
    asked: bool,
}

/// A spectator's environment client side, for whichever space and environment it watches.
#[derive(Default)]
pub struct Spectated {
    running: Option<Running>,
    relayed: crate::sides::Hands<crate::relayed::Relayed>,
    dropped: usize,
}

impl Spectated {
    pub fn new() -> Self {
        Self::default()
    }

    /// One frame, at `now`: run the client side of the joined space's environment, if `conversation` registers one and
    /// the spectator draws `drawn`, the environment it is registered for, carried into the space by `carriage`, seen
    /// from `camera` in the space's coordinates; and say what it shows, for its scenery to draw. None outside a space,
    /// on the grid, or in an environment without one.
    pub fn update(
        &mut self,
        conversation: &mut Conversation,
        drawn: Environment,
        carriage: Anchor,
        camera: Option<Vec3>,
        now: Instant,
    ) -> Option<Drawn> {
        let shown = conversation
            .client_side()
            .filter(|(environment, _)| *environment == drawn)
            .zip(
                conversation
                    .joined_space()
                    .map(|(space, _)| space.id.clone()),
            );
        let Some(((environment, factory), space)) = shown else {
            self.running = None;
            return None;
        };
        let current = self
            .running
            .as_ref()
            .is_some_and(|running| running.space == space && running.environment == environment);
        if !current {
            let side = factory(&space);
            self.running = Some(Running {
                space,
                environment,
                side,
                asked: false,
            });
        }
        let running = self.running.as_mut().expect("made above");
        let updates = conversation.environment_updates(environment.id(), environment.version());
        let (hands, left_hands) =
            crate::relayed::others(&mut self.relayed, conversation, carriage, now);
        let output = running.side.update(
            Input::new(updates, now)
                .pointing(None, camera.map(|camera| carriage.unplace(camera)))
                .with_hands(hands)
                .with_left_hands(left_hands),
        );
        if !output.intents.is_empty() {
            if !running.asked {
                log::info!(
                    "Spectator: dropping what the {} client side asks its server side; a spectator asks nothing.",
                    environment.id()
                );
                running.asked = true;
            }
            self.dropped += output.intents.len();
        }
        output.drawn
    }

    /// How many intents its client sides have asked, all dropped.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::fixtures::{SPACE, lobby, space};
    use std::sync::{Arc, Mutex};
    use tomori_client::Event;
    use tomori_protocol::{ConnectionRole, Participant, ServerMessage};

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

        fn update(&mut self, input: Input<'_>) -> crate::client_side::Output {
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

    fn shown(drawn: Option<Drawn>) -> Option<usize> {
        drawn.map(|drawn| *drawn.downcast_ref::<usize>().unwrap())
    }

    #[test]
    fn a_spectator_runs_the_client_side_of_the_environment_it_draws_and_draws_what_it_shows() {
        let mut conversation = lobby(ConnectionRole::Visitor);
        let (told, made) = registered(&mut conversation);
        let mut spectated = Spectated::new();
        let now = Instant::now();
        let carriage = Anchor::HOME;
        // Outside a space, none.
        assert!(
            spectated
                .update(
                    &mut conversation,
                    crate::test_support::SKY,
                    carriage,
                    None,
                    now
                )
                .is_none()
        );
        conversation.apply(space(|state| {
            state.participants.push(Participant {
                name: "Bea".into(),
                source: 2,
                engagement: None,
            });
        }));
        // On the grid, none.
        assert!(
            spectated
                .update(&mut conversation, Environment::MINIMAL, carriage, None, now)
                .is_none()
        );
        assert_eq!(*made.lock().unwrap(), 0);
        // Drawing the environment, it is made once and shows what it shows each frame.
        let camera = Vec3::new(1.0, 1.6, 2.0);
        let mut frames = Vec::new();
        for _ in 0..3 {
            frames.push(shown(spectated.update(
                &mut conversation,
                crate::test_support::SKY,
                carriage,
                Some(camera),
                now,
            )));
        }
        assert_eq!(frames, [Some(1), Some(2), Some(3)]);
        assert_eq!(*made.lock().unwrap(), 1);
        // What the server side sends reaches it, and the others' hands, from the camera, pointing at nothing.
        conversation.apply(Event::Message(ServerMessage::EnvironmentState {
            environment: "sky".into(),
            version: 1,
            state: tomori_protocol::Payload::new(serde_json::json!({})),
        }));
        conversation.apply(Event::Message(ServerMessage::EnvironmentEvent {
            environment: "sky".into(),
            version: 1,
            event: tomori_protocol::Payload::new(serde_json::json!({"cluck": 1})),
        }));
        conversation.apply(Event::Message(ServerMessage::Presence {
            source: 2,
            head: tomori_protocol::Pose::HOME,
            left: None,
            right: Some(tomori_protocol::Pose::HOME),
            hands: None,
        }));
        let carried = Anchor {
            position: Vec3::new(0.0, 0.0, -1.0),
            ..Anchor::HOME
        };
        spectated.update(
            &mut conversation,
            crate::test_support::SKY,
            carried,
            Some(camera),
            now,
        );
        {
            let told = told.lock().unwrap();
            assert_eq!(told.updates, [0, 0, 0, 2]);
            assert!(told.pointed.iter().all(Option::is_none));
            assert_eq!(told.heads.last(), Some(&Some(Vec3::new(1.0, 1.6, 3.0))));
            assert_eq!(told.hands.last(), Some(&vec![2]));
            assert!(told.participant.iter().all(Option::is_none));
            assert!(!told.controls);
        }
        // Leaving the environment drops it, and coming back makes it anew.
        assert!(
            spectated
                .update(&mut conversation, Environment::MINIMAL, carriage, None, now)
                .is_none()
        );
        assert_eq!(
            shown(spectated.update(
                &mut conversation,
                crate::test_support::SKY,
                carriage,
                None,
                now
            )),
            Some(1)
        );
        assert_eq!(*made.lock().unwrap(), 2);
        // Though its object is there to point at, the spectator pointed at nothing.
        let side = Asking {
            frames: 0,
            told: told.clone(),
        };
        let ray = crate::pointing::Ray::new(camera, Vec3::NEG_Z).unwrap();
        assert!(side.pick(&ray).is_some());
    }

    #[test]
    fn a_spectator_s_client_side_asks_its_server_side_nothing_whatever_its_role() {
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
            let mut spectated = Spectated::new();
            for _ in 0..3 {
                spectated.update(
                    &mut conversation,
                    crate::test_support::SKY,
                    Anchor::HOME,
                    None,
                    Instant::now(),
                );
            }
            assert_eq!(spectated.dropped(), 3);
            let actions = conversation.take_actions();
            assert!(!format!("{actions:?}").contains("EnvironmentIntent"));
        }
    }
}
