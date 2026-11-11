// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The right grip, and the desktop's G that stands for it, for an object of an environment's client side that takes it:
//! whose a squeeze is, in the grip's order, Tomori's own first, what the client side is told of it, from the hand that
//! holds it, and the others' hands it is given. See `design/environments.md` and `design/responders.md`.

use super::*;
use crate::client_side::{Actions, Grab, Hand, Phase};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What a stick's client side was told each frame: which client side it was, the grip, the others' hands and who this
/// participant is.
type Told = Arc<Mutex<Vec<(usize, Option<Grab>, Vec<(u32, Hand)>, Option<u32>)>>>;

/// Where the stick's handle is, which a hand there reaches.
const HANDLE: Vec3 = Vec3::new(0.3, 1.2, -0.5);

/// A client side whose stick, a metre along any ray ahead while it is there, takes the grip, or not, and is within reach
/// of a hand at its handle, keeping what it was told.
struct Stick {
    made: usize,
    takes: bool,
    there: Arc<AtomicBool>,
    told: Told,
}

impl crate::client_side::ClientSide for Stick {
    fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
        let stick = crate::client_side::Picked::new("stick", "the stick", 1.0);
        (ray.direction.z < 0.0 && self.there.load(Ordering::Relaxed)).then(|| {
            if self.takes {
                stick.handles(Actions::GRAB)
            } else {
                stick
            }
        })
    }

    fn within_reach(&self, hand: Vec3) -> Option<String> {
        (self.takes && hand.distance(HANDLE) < 0.1).then(|| "stick".to_owned())
    }

    fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
        self.told
            .lock()
            .unwrap()
            .push((self.made, input.grab, input.hands, input.participant));
        crate::client_side::Output::new()
    }
}

/// A client side that has nothing within reach, as one from before the grip reached environments.
struct Older;

impl crate::client_side::ClientSide for Older {
    fn pick(&self, _ray: &Ray) -> Option<crate::client_side::Picked> {
        None
    }

    fn update(&mut self, _input: crate::client_side::Input<'_>) -> crate::client_side::Output {
        crate::client_side::Output::new()
    }
}

fn ahead() -> Option<Ray> {
    Ray::new(arrival::HOME, Vec3::NEG_Z)
}

fn behind() -> Option<Ray> {
    Ray::new(arrival::HOME, Vec3::Z)
}

/// A space with a stick grown in, the ray on it, and the frame step, stepping a frame at a time.
struct Rig {
    conversation: Conversation,
    step: Step,
    senses: Senses,
    told: Told,
    there: Arc<AtomicBool>,
    now: Instant,
}

impl Rig {
    /// As `role`, with a stick that takes the grip, or not.
    fn new(role: ConnectionRole, takes: bool) -> Self {
        let told = Told::default();
        let there = Arc::new(AtomicBool::new(true));
        let made = Arc::new(AtomicUsize::new(0));
        let factory: crate::client_side::Factory = {
            let (told, there) = (told.clone(), there.clone());
            Arc::new(move |_: &str| {
                Box::new(Stick {
                    made: made.fetch_add(1, Ordering::Relaxed),
                    takes,
                    there: there.clone(),
                    told: told.clone(),
                }) as Box<dyn crate::client_side::ClientSide>
            })
        };
        let mut conversation = lobby(role);
        conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
        conversation.apply(space(|_| {}));
        conversation.take_actions();
        let mut rig = Self {
            conversation,
            step: Step::default(),
            senses: Senses {
                ray: ahead(),
                grip: Some(0.0),
                ..senses()
            },
            told,
            there,
            now: Instant::now(),
        };
        for _ in 0..400 {
            rig.frame();
        }
        assert!(rig.step.client.is_some());
        rig.told.lock().unwrap().clear();
        rig
    }

    /// A frame, and the grip the stick was told of in it.
    fn frame(&mut self) -> Option<Grab> {
        self.now += Duration::from_secs_f32(FRAME);
        let before = self.told.lock().unwrap().len();
        self.step.update(
            &self.senses,
            &mut self.conversation,
            &mut felt(),
            self.now,
            FRAME,
        );
        let told = self.told.lock().unwrap();
        told[before..]
            .last()
            .and_then(|(_, grab, _, _)| grab.clone())
    }

    /// The grip at `grip`, for a frame.
    fn grip(&mut self, grip: f32) -> Option<Grab> {
        self.senses.grip = Some(grip);
        self.frame()
    }

    /// The phases told, a frame at a time, as the grip goes through `grips`.
    fn phases(&mut self, grips: &[f32]) -> Vec<Option<Phase>> {
        grips
            .iter()
            .map(|grip| self.grip(*grip).map(|grab| grab.phase))
            .collect()
    }
}

#[test]
fn a_squeeze_begun_on_a_stick_that_takes_the_grip_is_its_until_let_go_held_where_the_ray_holds_it()
{
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    use Phase::*;
    // Began as it closes, past three quarters, changed while held, past half, and ended once as it is let go.
    assert_eq!(
        rig.phases(&[0.5, 0.8, 0.6, 0.9, 0.4, 0.0, 0.0]),
        [
            None,
            Some(Began),
            Some(Changed),
            Some(Changed),
            Some(Ended),
            None,
            None
        ]
    );
    assert_eq!(rig.step.last_grab, Some(precedence::Grab::Environment));
    // Without a controller, as on the desktop, the hand is along the ray, as far as it took the stick, facing along it,
    // and moves with the ray, wherever it goes, still the stick's.
    let grab = rig.grip(1.0).unwrap();
    assert_eq!(grab.object, "stick");
    assert!(grab.hand.position.distance(arrival::HOME + Vec3::NEG_Z) < 1e-4);
    assert!((grab.hand.orientation * Vec3::NEG_Z).distance(Vec3::NEG_Z) < 1e-4);
    rig.senses.ray = Ray::new(arrival::HOME, Vec3::new(1.0, 0.0, -1.0));
    let grab = rig.grip(1.0).unwrap();
    assert_eq!((grab.object.as_str(), grab.phase), ("stick", Changed));
    assert!(grab.hand.position.x > 0.5);
    // Whatever the ray points at meanwhile, nothing of Tomori's is given the grip, which it never was.
    assert!(rig.step.projector.held().is_none() && rig.step.board.held().is_none());
    // Losing focus lets it go.
    rig.senses.focused = false;
    assert_eq!(rig.frame().map(|grab| grab.phase), Some(Ended));
    rig.senses.focused = true;
    rig.grip(0.0);
    // Its participant is told who they are, as the server knows them.
    let told = rig.told.lock().unwrap();
    assert!(
        told.iter()
            .all(|(_, _, _, participant)| *participant == Some(1))
    );
}

#[test]
fn a_hand_at_a_stick_takes_it_with_the_grip_from_the_controller_s_grip_pose() {
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    // Pointing behind, at nothing, the hand at the stick's handle: it is within reach.
    rig.senses.ray = behind();
    let turned = glam::Quat::from_rotation_y(0.4);
    rig.senses.controllers[crate::controller::RIGHT] = Some(Pose {
        position: HANDLE.to_array(),
        orientation: turned.to_array(),
    });
    rig.frame();
    let grab = rig.grip(1.0).unwrap();
    assert_eq!((grab.object.as_str(), grab.phase), ("stick", Phase::Began));
    assert!(grab.hand.position.distance(HANDLE) < 1e-4);
    assert!(grab.hand.orientation.abs_diff_eq(turned, 1e-4));
    // The hand moving away, the stick goes with it, still told of from the controller.
    rig.senses.controllers[crate::controller::RIGHT] = Some(Pose {
        position: [0.0, 1.0, 0.0],
        orientation: turned.to_array(),
    });
    let grab = rig.grip(1.0).unwrap();
    assert_eq!(grab.hand.position, Vec3::new(0.0, 1.0, 0.0));
    // Its tracking lost, it is held where it was last.
    rig.senses.controllers[crate::controller::RIGHT] = None;
    rig.senses.ray = None;
    let grab = rig.grip(1.0).unwrap();
    assert_eq!(grab.hand.position, Vec3::new(0.0, 1.0, 0.0));
    assert_eq!(rig.grip(0.0).map(|grab| grab.phase), Some(Phase::Ended));
}

#[test]
fn the_grip_reaches_the_environment_only_where_nothing_of_tomori_s_takes_it() {
    // A squeeze begun on nothing is nobody's, though the ray comes to the stick while it is held.
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    rig.senses.ray = behind();
    rig.frame();
    rig.grip(1.0);
    assert_eq!(rig.step.last_grab, Some(precedence::Grab::Nothing));
    rig.senses.ray = ahead();
    assert!(rig.phases(&[1.0, 1.0, 0.0]).iter().all(Option::is_none));
    // A stick that does not take the grip is not taken.
    let mut rig = Rig::new(ConnectionRole::Owner, false);
    assert!(rig.phases(&[1.0, 0.0]).iter().all(Option::is_none));
    assert_eq!(rig.step.last_grab, Some(precedence::Grab::Nothing));
    // A visitor takes nothing.
    let mut rig = Rig::new(ConnectionRole::Visitor, true);
    assert!(rig.phases(&[1.0, 0.0]).iter().all(Option::is_none));
    // The transcript, showing on the right wrist, takes it first, as it detaches.
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    rig.senses.right_palm = Some(palm_up(0.0));
    for _ in 0..30 {
        rig.frame();
    }
    assert!(rig.step.wrist.state().active());
    assert!(rig.phases(&[1.0, 0.0]).iter().all(Option::is_none));
    assert_eq!(rig.step.last_grab, Some(precedence::Grab::Transcript));
    // A panel nearer than the stick is Tomori's, which the grip pulls off, the stick told nothing; and carried, it
    // keeps the next squeeze too.
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
    fixtures::show_projection(
        &mut rig.conversation,
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
    for _ in 0..10 {
        rig.frame();
    }
    assert!(rig.phases(&[1.0, 1.0]).iter().all(Option::is_none));
    assert_eq!(rig.step.last_grab, Some(precedence::Grab::Tomori));
    assert_eq!(rig.step.projector.held(), Some("panel-1"));
    rig.grip(0.0);
    assert!(rig.phases(&[1.0, 0.0]).iter().all(Option::is_none));
}

#[test]
fn a_squeeze_that_outlives_its_client_side_tells_the_next_nothing_until_let_go() {
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    assert_eq!(rig.grip(1.0).map(|grab| grab.phase), Some(Phase::Began));
    // The space moves to another environment and back, mid-squeeze: the new client side is told nothing of it.
    rig.conversation
        .apply(space(|state| state.environment = "minimal".into()));
    for _ in 0..400 {
        rig.grip(1.0);
    }
    rig.conversation
        .apply(space(|state| state.environment = "sky".into()));
    for _ in 0..400 {
        assert_eq!(rig.grip(1.0), None);
    }
    let told = rig.told.lock().unwrap().clone();
    assert!(told.iter().any(|(made, ..)| *made == 1));
    assert!(
        told.iter()
            .filter(|(made, ..)| *made == 1)
            .all(|(_, grab, ..)| grab.is_none())
    );
    // Let go, the next squeeze is the new one's.
    rig.grip(0.0);
    assert_eq!(rig.grip(1.0).map(|grab| grab.phase), Some(Phase::Began));
    // And the stick gone from under the ray, as its client side says, nothing is taken.
    rig.grip(0.0);
    rig.there.store(false, Ordering::Relaxed);
    rig.frame();
    assert!(rig.phases(&[1.0, 0.0]).iter().all(Option::is_none));
    // A client side that says nothing is within reach takes nothing from a hand anywhere.
    use crate::client_side::ClientSide as _;
    let mut older = Older;
    assert_eq!(older.within_reach(HANDLE), None);
    assert_eq!(older.pick(&ahead().unwrap()), None);
    let output = older.update(crate::client_side::Input::new(Vec::new(), rig.now));
    assert!(output.intents.is_empty());
}

#[test]
fn a_client_side_is_given_the_others_right_hands_in_the_space_but_never_its_own() {
    use tomori_client::Event;
    use tomori_protocol::{Participant, ServerMessage};
    let mut rig = Rig::new(ConnectionRole::Owner, true);
    let with_bea = |state: &mut tomori_protocol::SpaceState| {
        state.participants.push(Participant {
            name: "Bea".into(),
            source: 2,
            engagement: None,
        });
    };
    rig.conversation.apply(space(with_bea));
    let hand = |source: u32, x: f32, right: bool| {
        Event::Message(ServerMessage::Presence {
            source,
            head: tomori_protocol::Pose::HOME,
            left: None,
            right: right.then_some(tomori_protocol::Pose {
                position: [x, 1.0, -1.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            }),
            hands: None,
        })
    };
    // Bea's, and Ada's own never, nor that of Cy, whom the space does not list.
    rig.conversation.apply(hand(2, 0.5, true));
    rig.conversation.apply(hand(1, 0.9, true));
    rig.conversation.apply(hand(3, 0.7, true));
    rig.frame();
    let hands = |rig: &Rig| rig.told.lock().unwrap().last().unwrap().2.clone();
    assert_eq!(hands(&rig).len(), 1);
    let (source, bea) = hands(&rig)[0];
    assert_eq!(source, 2);
    assert!(bea.position.distance(Vec3::new(0.5, 1.0, -1.0)) < 1e-5);
    assert_eq!(rig.conversation.source(), Some(1));
    // Her presence without a right hand, as the desktop's, gives none.
    rig.conversation.apply(hand(2, 0.5, false));
    rig.frame();
    assert!(hands(&rig).is_empty());
    // Leaving the space forgets them all.
    rig.conversation.apply(hand(2, 0.5, true));
    rig.conversation
        .apply(Event::Message(ServerMessage::Left { deleted: None }));
    assert!(rig.conversation.right_hands().is_empty());
    rig.conversation.apply(space(with_bea));
    assert!(rig.conversation.right_hands().is_empty());
}
