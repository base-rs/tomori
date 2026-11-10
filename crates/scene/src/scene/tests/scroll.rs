// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The right stick's push, and the desktop's keys and wheel that stand for it, for an object of an environment's client
//! side that takes it: whose a push is, in the right stick's order, and that each is told began once and ended once,
//! whatever happens meanwhile. See `design/environments.md` and `design/responders.md`.

use super::*;
use crate::client_side::{Actions, Phase, Scroll};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What a globe's client side was told each frame: which client side it was, the object's push, and the right stick.
type Told = Arc<Mutex<Vec<(usize, Option<Scroll>, Vec2)>>>;

/// A client side whose globe, a metre along any ray ahead while it is there, takes the right stick, or not, keeping what
/// it was told.
struct Globe {
    made: usize,
    takes: bool,
    there: Arc<AtomicBool>,
    told: Told,
}

impl crate::client_side::ClientSide for Globe {
    fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
        let globe = crate::client_side::Picked::new("globe", "the globe", 1.0);
        (ray.direction.z < 0.0 && self.there.load(Ordering::Relaxed)).then(|| {
            if self.takes {
                globe.handles(Actions::SCROLL)
            } else {
                globe
            }
        })
    }

    fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
        self.told
            .lock()
            .unwrap()
            .push((self.made, input.scroll, input.right_stick));
        crate::client_side::Output::new()
    }
}

/// The ray ahead, on the globe, and behind, on nothing.
fn ahead() -> Option<Ray> {
    Ray::new(arrival::HOME, Vec3::NEG_Z)
}

fn behind() -> Option<Ray> {
    Ray::new(arrival::HOME, Vec3::Z)
}

/// A space with a globe grown in, the ray on it, and the frame step, stepping a frame at a time.
struct Rig {
    conversation: Conversation,
    step: Step,
    senses: Senses,
    told: Told,
    there: Arc<AtomicBool>,
    now: Instant,
}

/// Joined to [`SPACE`] with a [`Globe`] that takes the right stick, or not, for Sky, without travel, and what it is told.
fn globe(takes: bool) -> (Conversation, Told, Arc<AtomicBool>) {
    let told = Told::default();
    let there = Arc::new(AtomicBool::new(true));
    let made = Arc::new(AtomicUsize::new(0));
    let factory: crate::client_side::Factory = {
        let (told, there) = (told.clone(), there.clone());
        Arc::new(move |_: &str| {
            Box::new(Globe {
                made: made.fetch_add(1, Ordering::Relaxed),
                takes,
                there: there.clone(),
                told: told.clone(),
            }) as Box<dyn crate::client_side::ClientSide>
        })
    };
    let mut conversation = lobby(ConnectionRole::Owner);
    conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
    conversation.apply(space(|_| {}));
    conversation.take_actions();
    (conversation, told, there)
}

impl Rig {
    fn new(takes: bool) -> Self {
        let (conversation, told, there) = globe(takes);
        let mut rig = Self {
            conversation,
            step: Step::default(),
            senses: Senses {
                ray: ahead(),
                ..senses()
            },
            told,
            there,
            now: Instant::now(),
        };
        for _ in 0..400 {
            rig.update();
        }
        assert!(rig.step.client.is_some());
        rig.told.lock().unwrap().clear();
        rig
    }

    fn update(&mut self) {
        self.now += Duration::from_secs_f32(FRAME);
        self.step.update(
            &self.senses,
            &mut self.conversation,
            &mut felt(),
            self.now,
            FRAME,
        );
    }

    /// A frame with `buttons`, and what the globe was told in it, if it was told anything.
    fn frame_with(&mut self, buttons: Buttons) -> Option<(Option<Scroll>, Vec2)> {
        let focused = self.senses.focused;
        self.step.buttons(
            &buttons,
            focused,
            &mut self.conversation,
            &mut felt(),
            self.now,
        );
        let before = self.told.lock().unwrap().len();
        self.update();
        let told = self.told.lock().unwrap();
        (told.len() > before).then(|| {
            let (_, scroll, right) = told.last().cloned().unwrap();
            (scroll, right)
        })
    }

    /// A frame with the right stick at `right`, and what the globe was told in it.
    fn frame(&mut self, right: Vec2) -> (Option<Scroll>, Vec2) {
        self.frame_with(push(right)).expect("told")
    }

    /// The pushes told since this was last asked, by which client side, with their phase and object.
    fn phases(&self) -> Vec<(usize, Phase, String)> {
        self.told
            .lock()
            .unwrap()
            .drain(..)
            .filter_map(|(made, scroll, _)| {
                scroll.map(|scroll| (made, scroll.phase, scroll.object))
            })
            .collect()
    }
}

fn push(right: Vec2) -> Buttons {
    press(|buttons| buttons.right_stick = right)
}

fn began(axis: Vec2) -> Option<Scroll> {
    Some(Scroll::new("globe", axis, Phase::Began))
}

fn changed(axis: Vec2) -> Option<Scroll> {
    Some(Scroll::new("globe", axis, Phase::Changed))
}

fn ended() -> Option<Scroll> {
    Some(Scroll::new("globe", Vec2::ZERO, Phase::Ended))
}

/// A projected panel floating 0.6 m ahead, nearer than the globe, so that the ray points at it.
fn panel_ahead(conversation: &mut Conversation) {
    let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
    fixtures::show_projection(
        conversation,
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
}

/// Who took a push, as the globe's client side and the frame step see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Taken {
    /// The globe, told as `Input::scroll`.
    Object,
    /// The client side, told as `Input::right_stick`.
    Environment,
    /// Something of Tomori's, or nobody: the client side is told neither.
    Neither,
}

/// A situation, set up in `rig` before a push leaves the middle.
type Situation = fn(&mut Rig);

fn pointing_at_the_globe(_: &mut Rig) {}

fn pointing_at_nothing(rig: &mut Rig) {
    rig.senses.ray = behind();
    rig.update();
}

fn a_panel_with_more_than_it_shows_in_front(rig: &mut Rig) {
    code_ahead(&mut rig.conversation, 200);
    rig.update();
}

fn a_panel_that_shows_everything_in_front(rig: &mut Rig) {
    code_ahead(&mut rig.conversation, 1);
    rig.update();
}

fn a_panel_carried(rig: &mut Rig) {
    panel_ahead(&mut rig.conversation);
    rig.senses.grip = Some(1.0);
    rig.update();
    rig.update();
    assert!(rig.step.projector.held().is_some());
}

fn the_transcript_shows(rig: &mut Rig) {
    rig.senses.right_palm = Some(palm_up(0.0));
    let now = rig.now;
    wrist_after(&mut rig.step, &mut rig.conversation, &rig.senses, now);
    rig.now += HOLD;
    rig.update();
    assert!(rig.step.wrist.state().active());
}

fn the_hand_menu_shows(rig: &mut Rig) {
    rig.senses.left_palm = Some(left_palm_up(0.0));
    rig.update();
    rig.now += HOLD;
    rig.update();
    rig.update();
}

fn without_focus(rig: &mut Rig) {
    rig.senses.focused = false;
}

#[test]
fn a_push_goes_to_the_first_rung_of_the_right_stick_s_order_that_takes_it() {
    // Each situation, the rung of `precedence::Right` it reaches, and who takes the push for a globe that takes the
    // stick, and for one that does not, which is as before.
    let cases: [(&str, Situation, precedence::Right, Taken, Taken); 8] = [
        (
            "pointing at the globe",
            pointing_at_the_globe,
            precedence::Right::Object,
            Taken::Object,
            Taken::Environment,
        ),
        (
            "pointing at nothing",
            pointing_at_nothing,
            precedence::Right::Environment,
            Taken::Environment,
            Taken::Environment,
        ),
        (
            "a panel with more than it shows in front",
            a_panel_with_more_than_it_shows_in_front,
            precedence::Right::Scroll,
            Taken::Neither,
            Taken::Neither,
        ),
        (
            "a panel that shows everything in front",
            a_panel_that_shows_everything_in_front,
            precedence::Right::Tomori,
            Taken::Neither,
            Taken::Neither,
        ),
        (
            "a panel carried",
            a_panel_carried,
            precedence::Right::Carried,
            Taken::Neither,
            Taken::Neither,
        ),
        (
            "the transcript shows",
            the_transcript_shows,
            precedence::Right::Transcript,
            Taken::Neither,
            Taken::Neither,
        ),
        (
            "the hand menu shows",
            the_hand_menu_shows,
            precedence::Right::Tomori,
            Taken::Neither,
            Taken::Neither,
        ),
        (
            "without focus",
            without_focus,
            precedence::Right::Object,
            Taken::Neither,
            Taken::Neither,
        ),
    ];
    let stick = Vec2::new(0.3, -0.8);
    for (name, situation, rung, takes, before) in cases {
        for (handles, expected) in [(true, takes), (false, before)] {
            let mut rig = Rig::new(handles);
            situation(&mut rig);
            rig.told.lock().unwrap().clear();
            let (scroll, right) = rig.frame(stick);
            // Never both: a push told as the globe's tells no right stick.
            let taken = match scroll {
                Some(scroll) => {
                    assert_eq!(
                        (scroll, right),
                        (began(stick).unwrap(), Vec2::ZERO),
                        "{name}"
                    );
                    Taken::Object
                }
                None if right == stick => Taken::Environment,
                None => {
                    assert_eq!(right, Vec2::ZERO, "{name}");
                    Taken::Neither
                }
            };
            assert_eq!(taken, expected, "{name}, takes the stick: {handles}");
            // The rung, for a globe that takes it: as the frame step records it.
            if handles {
                let owner = match rung {
                    precedence::Right::Carried => RightStick::Carried,
                    precedence::Right::Scroll => RightStick::Scrolled("panel-1".into()),
                    precedence::Right::Object if expected == Taken::Object => {
                        RightStick::Object("globe".into(), 1)
                    }
                    precedence::Right::Object => RightStick::Abandoned,
                    precedence::Right::Environment => RightStick::Environment,
                    _ => RightStick::Other,
                };
                assert_eq!(rig.step.right_stick, owner, "{name}");
            }
        }
    }
}

#[test]
fn a_client_side_that_takes_nothing_is_told_the_right_stick_as_before() {
    // Pushes, rests and rolls of the stick, as the old rule tells them: the stick wherever the push is the client
    // side's, and zero at the middle and wherever it is not; never a scroll. The same for a globe that takes the stick
    // where the ray is not on it.
    let sticks = [
        Vec2::ZERO,
        Vec2::new(0.0, 0.5),
        Vec2::Y,
        Vec2::new(-0.4, 0.0),
        Vec2::new(0.1, 0.0),
        Vec2::new(0.9, 0.0),
        Vec2::new(0.0, -0.19),
        Vec2::NEG_Y,
        Vec2::ZERO,
    ];
    let pushed = |stick: Vec2| {
        if stick.length() < 0.2 {
            Vec2::ZERO
        } else {
            stick
        }
    };
    let cases: [(Situation, bool, bool); 7] = [
        (pointing_at_the_globe, false, true),
        (pointing_at_nothing, false, true),
        (pointing_at_nothing, true, true),
        (a_panel_with_more_than_it_shows_in_front, false, false),
        (a_panel_that_shows_everything_in_front, false, false),
        (a_panel_carried, false, false),
        (the_transcript_shows, false, false),
    ];
    for (index, (situation, handles, environment)) in cases.into_iter().enumerate() {
        let mut rig = Rig::new(handles);
        situation(&mut rig);
        let told: Vec<_> = sticks.iter().map(|stick| rig.frame(*stick)).collect();
        let expected: Vec<_> = sticks
            .iter()
            .map(|stick| {
                (
                    None,
                    if environment {
                        pushed(*stick)
                    } else {
                        Vec2::ZERO
                    },
                )
            })
            .collect();
        assert_eq!(told, expected, "case {index}");
    }
}

#[test]
fn a_push_is_told_began_changed_and_ended_and_stays_with_the_object_it_began_on() {
    // Each case: frames of the ray, on the globe or not, and the stick, after a frame at rest with the first frame's
    // ray, and what the globe is told in each: its push, or the right stick.
    let (on, off) = (true, false);
    let full = Vec2::Y;
    let half = Vec2::new(0.5, 0.0);
    type Frames = Vec<(bool, Vec2, Option<Scroll>, Vec2)>;
    let cases: [(&str, Frames); 6] = [
        (
            "a push on the globe",
            vec![
                (on, full, began(full), Vec2::ZERO),
                (on, half, changed(half), Vec2::ZERO),
                (on, Vec2::ZERO, ended(), Vec2::ZERO),
                (on, Vec2::ZERO, None, Vec2::ZERO),
            ],
        ),
        (
            "a push of one frame",
            vec![
                (on, full, began(full), Vec2::ZERO),
                (on, Vec2::new(0.1, 0.0), ended(), Vec2::ZERO),
            ],
        ),
        (
            "a push that leaves the globe stays its own",
            vec![
                (on, full, began(full), Vec2::ZERO),
                (off, full, changed(full), Vec2::ZERO),
                (off, half, changed(half), Vec2::ZERO),
                (off, Vec2::ZERO, ended(), Vec2::ZERO),
            ],
        ),
        (
            "a push that comes to the globe is the client side's",
            vec![
                (off, full, None, full),
                (on, full, None, full),
                (on, Vec2::ZERO, None, Vec2::ZERO),
            ],
        ),
        (
            "two pushes, each its own",
            vec![
                (on, full, began(full), Vec2::ZERO),
                (on, Vec2::ZERO, ended(), Vec2::ZERO),
                (on, half, began(half), Vec2::ZERO),
                (on, Vec2::ZERO, ended(), Vec2::ZERO),
            ],
        ),
        (
            "at rest, nothing",
            vec![
                (on, Vec2::ZERO, None, Vec2::ZERO),
                (on, Vec2::new(0.0, 0.15), None, Vec2::ZERO),
            ],
        ),
    ];
    for (name, frames) in cases {
        let mut rig = Rig::new(true);
        rig.senses.ray = if frames[0].0 { ahead() } else { behind() };
        rig.frame(Vec2::ZERO);
        for (index, (on, stick, scroll, right)) in frames.into_iter().enumerate() {
            rig.senses.ray = if on { ahead() } else { behind() };
            assert_eq!(rig.frame(stick), (scroll, right), "{name}, frame {index}");
        }
    }
}

/// A push begun on the globe, `change` made after two frames of it, held three more frames, then let go and held at
/// the middle three: the client sides that were told began and ended, once each, by which one.
fn pushed_through(rig: &mut Rig, change: impl FnOnce(&mut Rig)) -> Vec<(usize, Phase, String)> {
    rig.frame(Vec2::Y);
    rig.frame(Vec2::Y);
    change(rig);
    for _ in 0..3 {
        rig.frame_with(push(Vec2::Y));
    }
    for _ in 0..3 {
        rig.frame_with(push(Vec2::ZERO));
    }
    rig.phases()
        .into_iter()
        .filter(|(_, phase, _)| *phase != Phase::Changed)
        .collect()
}

#[test]
fn a_push_is_told_ended_once_however_it_ends_and_never_moves_to_anything_else() {
    let once = |made| {
        vec![
            (made, Phase::Began, "globe".to_owned()),
            (made, Phase::Ended, "globe".to_owned()),
        ]
    };
    // Whatever appears or goes meanwhile, the push stays the globe's, named as it began, until the middle.
    let cases: [(&str, Situation); 6] = [
        ("nothing", |_| {}),
        ("the globe goes away", |rig| {
            rig.there.store(false, Ordering::Relaxed)
        }),
        ("the ray leaves it", |rig| rig.senses.ray = behind()),
        ("a panel is carried", |rig| {
            panel_ahead(&mut rig.conversation);
            rig.senses.grip = Some(1.0);
            rig.update();
            rig.update();
            assert!(rig.step.projector.held().is_some());
        }),
        ("the transcript opens", the_transcript_shows),
        ("the hand menu shows", the_hand_menu_shows),
    ];
    for (name, change) in cases {
        let mut rig = Rig::new(true);
        assert_eq!(pushed_through(&mut rig, change), once(0), "{name}");
    }
    // Focus lost, by blurring or by buttons without it, ends the push at once, and the push held on is nobody's.
    let lost: [(&str, Situation); 2] = [
        ("blurred", |rig| {
            rig.step.blur(&mut rig.conversation);
            rig.update();
        }),
        ("unfocused", |rig| {
            rig.senses.focused = false;
            rig.frame_with(push(Vec2::Y));
            rig.senses.focused = true;
        }),
    ];
    for (name, change) in lost {
        let mut rig = Rig::new(true);
        assert_eq!(pushed_through(&mut rig, change), once(0), "{name}");
        assert_eq!(rig.step.right_stick, RightStick::Free, "{name}");
        // The next push on the globe begins anew.
        assert_eq!(rig.frame(Vec2::Y).0, began(Vec2::Y), "{name}");
    }
    // Its client side replaced, as the space changes environment and back, the push goes with the one it began for,
    // without telling the new one anything until it returns to the middle; the next push is the new one's.
    let mut rig = Rig::new(true);
    let told = pushed_through(&mut rig, |rig| {
        for environment in ["minimal", "sky"] {
            rig.conversation
                .apply(space(|state| state.environment = environment.into()));
            for _ in 0..400 {
                rig.step.buttons(
                    &push(Vec2::Y),
                    true,
                    &mut rig.conversation,
                    &mut felt(),
                    rig.now,
                );
                rig.update();
            }
        }
    });
    assert_eq!(told, [(0, Phase::Began, "globe".to_owned())]);
    rig.frame(Vec2::Y);
    rig.frame(Vec2::ZERO);
    assert_eq!(
        rig.phases()
            .into_iter()
            .filter(|(_, phase, _)| *phase != Phase::Changed)
            .collect::<Vec<_>>(),
        once(1)
    );
}

/// A frame with the wheel turned `rows`, down where more than none, as the desktop window gives it.
fn wheel(rows: f32) -> Buttons {
    press(|buttons| buttons.scroll = rows)
}

/// The pushes told since this was last asked, as their phase and forward axis.
fn told(rig: &Rig) -> Vec<(Phase, f32)> {
    rig.told
        .lock()
        .unwrap()
        .drain(..)
        .filter_map(|(_, scroll, _)| scroll)
        .map(|scroll| {
            assert_eq!(scroll.axis.x, 0.0);
            (scroll.phase, scroll.axis.y)
        })
        .collect()
}

/// How far `told` scrolls a panel, forward, as a client side integrating its axis at the stick's rate would.
fn distance(told: &[(Phase, f32)]) -> f32 {
    told.iter()
        .map(|(_, forward)| forward * crate::projection::SCROLL_SPEED * FRAME)
        .sum()
}

#[test]
fn a_notch_of_the_wheel_pushes_the_globe_as_far_as_it_scrolls_a_panel() {
    // A notch away from the participant, three rows up, is a push forward, all the way, until it has scrolled three
    // rows, the last frame only as far as is left; then nothing for a tenth of a second, and its end.
    let mut rig = Rig::new(true);
    rig.frame_with(wheel(-desktop::WHEEL_ROWS));
    for _ in 0..40 {
        rig.frame(Vec2::ZERO);
    }
    let pushed = told(&rig);
    let phases: Vec<_> = pushed.iter().map(|(phase, _)| *phase).collect();
    let mut expected = vec![Phase::Began];
    expected.extend(std::iter::repeat_n(Phase::Changed, 21));
    expected.push(Phase::Ended);
    assert_eq!(phases, expected);
    let forward: Vec<_> = pushed.iter().map(|(_, forward)| *forward).collect();
    assert!(
        forward[..14].iter().all(|forward| *forward == 1.0),
        "{forward:?}"
    );
    assert!((forward[14] - 0.4).abs() < 1e-3, "{forward:?}");
    assert!(
        forward[15..].iter().all(|forward| *forward == 0.0),
        "{forward:?}"
    );
    assert!((distance(&pushed) - desktop::WHEEL_ROWS).abs() < 1e-3);
    // Towards the participant, back; a notch more meanwhile goes on with the same push, as far as both.
    rig.frame_with(wheel(desktop::WHEEL_ROWS));
    for _ in 0..10 {
        rig.frame(Vec2::ZERO);
    }
    rig.frame_with(wheel(desktop::WHEEL_ROWS));
    for _ in 0..60 {
        rig.frame(Vec2::ZERO);
    }
    let pushed = told(&rig);
    let phases: Vec<_> = pushed.iter().map(|(phase, _)| *phase).collect();
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == Phase::Began)
            .count(),
        1
    );
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == Phase::Ended)
            .count(),
        1
    );
    assert!(pushed.iter().all(|(_, forward)| *forward <= 0.0));
    assert!((distance(&pushed) + 2.0 * desktop::WHEEL_ROWS).abs() < 1e-3);
}

#[test]
fn a_slow_scroll_of_a_touchpad_is_one_push_as_slow_and_as_far_as_it_scrolls() {
    // A twentieth of a notch a frame, as a touchpad's pixels give it, for 60 frames: one push, told forward a little
    // less than three quarters of the way, as a panel scrolls 0.15 rows a frame, then its end, as far as it was turned.
    let mut rig = Rig::new(true);
    let rows = desktop::WHEEL_ROWS / 20.0;
    for _ in 0..60 {
        rig.frame_with(wheel(-rows));
    }
    for _ in 0..20 {
        rig.frame(Vec2::ZERO);
    }
    let pushed = told(&rig);
    let phases: Vec<_> = pushed.iter().map(|(phase, _)| *phase).collect();
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == Phase::Began)
            .count(),
        1
    );
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == Phase::Ended)
            .count(),
        1
    );
    assert_eq!(phases.first(), Some(&Phase::Began));
    assert_eq!(phases.last(), Some(&Phase::Ended));
    let expected = rows / (crate::projection::SCROLL_SPEED * FRAME);
    assert!((expected - 0.72).abs() < 1e-3);
    for (_, forward) in &pushed[..60] {
        assert!((forward - expected).abs() < 1e-3, "{forward}");
    }
    let input = 60.0 * rows;
    assert!((distance(&pushed) - input).abs() < input * 0.05);
}

#[test]
fn a_fast_spin_of_the_wheel_keeps_little_and_ends_soon_after_it_stops() {
    // Fifty notches in 0.8 seconds: the push keeps at most what the stick scrolls in 0.4 seconds, never more, and ends
    // within half a second of the last turn.
    let mut rig = Rig::new(true);
    let frames = (0.8 / FRAME).round() as usize;
    let rows = 50.0 * desktop::WHEEL_ROWS / frames as f32;
    for _ in 0..frames {
        rig.frame_with(wheel(-rows));
        let kept = rig.step.wheel_push.as_ref().map_or(0.0, |wheel| wheel.rows);
        assert!(kept.abs() <= WHEEL_BACKLOG, "{kept}");
    }
    let mut after = 0;
    while !told(&rig).iter().any(|(phase, _)| *phase == Phase::Ended) {
        rig.frame(Vec2::ZERO);
        after += 1;
        assert!(after < 1000);
    }
    assert!(after as f32 * FRAME <= 0.5 + FRAME, "{after} frames");
}

#[test]
fn a_turn_of_the_wheel_pushes_nothing_where_the_stick_would_not_push_the_globe() {
    // A turn that comes to nothing pushes nothing, though it lingers as one push, and ends.
    let mut rig = Rig::new(true);
    rig.step.buttons(
        &wheel(3.0),
        true,
        &mut rig.conversation,
        &mut felt(),
        rig.now,
    );
    rig.step.buttons(
        &wheel(-3.0),
        true,
        &mut rig.conversation,
        &mut felt(),
        rig.now,
    );
    for _ in 0..20 {
        rig.frame(Vec2::ZERO);
    }
    let pushed = told(&rig);
    assert_eq!(distance(&pushed), 0.0);
    assert_eq!(pushed.first().map(|(phase, _)| *phase), Some(Phase::Began));
    assert_eq!(pushed.last().map(|(phase, _)| *phase), Some(Phase::Ended));
    // Nor does one without focus, nor at a globe that does not take the stick, nor at a panel in front of it, which it
    // scrolls.
    let cases: [(bool, Situation); 3] = [
        (true, without_focus),
        (false, pointing_at_the_globe),
        (true, a_panel_with_more_than_it_shows_in_front),
    ];
    for (index, (handles, situation)) in cases.into_iter().enumerate() {
        let mut rig = Rig::new(handles);
        situation(&mut rig);
        rig.frame_with(wheel(3.0));
        for _ in 0..20 {
            rig.frame_with(push(Vec2::ZERO));
        }
        assert!(rig.phases().is_empty(), "case {index}");
    }
    // A frame of no time, or of nonsense, pushes nothing and keeps what is left.
    let mut turned = WheelPush::new("globe", 1);
    turned.turn(-3.0);
    assert_eq!(turned.advance(0.0), Some(Vec2::ZERO));
    assert_eq!(turned.advance(f32::NAN), Some(Vec2::ZERO));
    assert_eq!(turned.rows, 3.0);
}

#[test]
fn the_stick_and_the_wheel_take_turns_each_push_told_its_end_before_the_next_begins() {
    // The stick pushed on the globe meanwhile ends the wheel's push and begins its own on the next frame, and the wheel
    // turned during a push of the stick is the stick's.
    let mut rig = Rig::new(true);
    rig.frame_with(wheel(-3.0));
    rig.frame(Vec2::ZERO);
    rig.frame_with(press(|buttons| {
        buttons.right_stick = Vec2::X;
        buttons.scroll = -3.0;
    }));
    rig.frame(Vec2::X);
    rig.frame(Vec2::ZERO);
    for _ in 0..20 {
        rig.frame(Vec2::ZERO);
    }
    let pushed: Vec<_> = rig
        .told
        .lock()
        .unwrap()
        .drain(..)
        .filter_map(|(_, scroll, _)| scroll)
        .map(|scroll| (scroll.phase, scroll.axis))
        .collect();
    assert_eq!(
        pushed,
        [
            (Phase::Began, Vec2::Y),
            (Phase::Changed, Vec2::Y),
            (Phase::Ended, Vec2::ZERO),
            (Phase::Began, Vec2::X),
            (Phase::Ended, Vec2::ZERO),
        ]
    );
    // The wheel turned as a push of the stick returns to the middle waits for the stick's end, then pushes all it was
    // turned, losing nothing to the frame it waited.
    let mut rig = Rig::new(true);
    rig.frame(Vec2::X);
    rig.frame_with(wheel(-3.0));
    for _ in 0..40 {
        rig.frame(Vec2::ZERO);
    }
    let pushed: Vec<_> = rig
        .told
        .lock()
        .unwrap()
        .drain(..)
        .filter_map(|(_, scroll, _)| scroll)
        .collect();
    assert_eq!(
        pushed[..3]
            .iter()
            .map(|scroll| scroll.phase)
            .collect::<Vec<_>>(),
        [Phase::Began, Phase::Ended, Phase::Began]
    );
    assert_eq!(pushed[2].axis, Vec2::Y);
    let wheeled: f32 = pushed[2..]
        .iter()
        .map(|scroll| scroll.axis.y * crate::projection::SCROLL_SPEED * FRAME)
        .sum();
    assert!((wheeled - 3.0).abs() < 1e-3, "{wheeled}");
    // Losing focus ends the wheel's push too.
    let mut rig = Rig::new(true);
    rig.frame_with(wheel(-3.0));
    rig.step.blur(&mut rig.conversation);
    rig.update();
    rig.frame(Vec2::ZERO);
    let phases: Vec<_> = rig
        .phases()
        .into_iter()
        .map(|(_, phase, _)| phase)
        .collect();
    assert_eq!(phases, [Phase::Began, Phase::Ended]);
}

#[test]
fn buttons_read_more_often_than_frames_tell_a_client_side_each_push_it_sees_once() {
    // The headset reads buttons every frame, even one it does not draw, so several may come between two frames of the
    // client side. A push that begins and ends between them tells nothing; one still held tells its beginning; one that
    // ends and another that begins between them tell the first's end, then the second's beginning.
    let mut rig = Rig::new(true);
    let buttons = |rig: &mut Rig, right: Vec2| {
        rig.step.buttons(
            &push(right),
            true,
            &mut rig.conversation,
            &mut felt(),
            rig.now,
        );
    };
    buttons(&mut rig, Vec2::Y);
    buttons(&mut rig, Vec2::Y);
    buttons(&mut rig, Vec2::ZERO);
    rig.update();
    assert!(rig.phases().is_empty());
    for _ in 0..3 {
        buttons(&mut rig, Vec2::Y);
    }
    rig.update();
    buttons(&mut rig, Vec2::ZERO);
    buttons(&mut rig, Vec2::X);
    rig.update();
    rig.update();
    buttons(&mut rig, Vec2::ZERO);
    buttons(&mut rig, Vec2::ZERO);
    rig.update();
    rig.update();
    assert_eq!(
        rig.phases()
            .into_iter()
            .map(|(_, phase, _)| phase)
            .collect::<Vec<_>>(),
        [Phase::Began, Phase::Ended, Phase::Began, Phase::Ended]
    );
}

#[test]
fn the_desktop_s_keys_and_wheel_push_the_globe_the_mouse_points_at() {
    use desktop::{Desktop, Direction, Key};
    const SIZE: Vec2 = Vec2::new(1280.0, 800.0);
    let (mut conversation, told, _) = globe(true);
    let mut desktop = Desktop::new(&mut conversation, Instant::now());
    desktop.set_focused(true, &mut conversation);
    desktop.cursor_moved(SIZE * 0.5);
    let mut now = Instant::now();
    for _ in 0..40 {
        now += Duration::from_millis(100);
        desktop.frame(&mut conversation, &mut felt(), now, SIZE);
    }
    let last = |told: &Told| told.lock().unwrap().last().cloned().unwrap();
    // Page Up, Page Down and the brackets, the right stick's, push the globe the mouse points at.
    for (key, stick) in [
        (Direction::Up, Vec2::Y),
        (Direction::Down, Vec2::NEG_Y),
        (Direction::Left, Vec2::NEG_X),
        (Direction::Right, Vec2::X),
    ] {
        desktop.key(Key::Right(key), true, false, &mut conversation);
        now += Duration::from_millis(16);
        desktop.frame(&mut conversation, &mut felt(), now, SIZE);
        assert_eq!(last(&told).1, began(stick), "{key:?}");
        assert_eq!(last(&told).2, Vec2::ZERO, "{key:?}");
        desktop.key(Key::Right(key), false, false, &mut conversation);
        now += Duration::from_millis(16);
        desktop.frame(&mut conversation, &mut felt(), now, SIZE);
        assert_eq!(last(&told).1, ended(), "{key:?}");
    }
    // A notch of the wheel away from the participant pushes it forward.
    desktop.wheel(1.0);
    now += Duration::from_millis(16);
    desktop.frame(&mut conversation, &mut felt(), now, SIZE);
    assert_eq!(last(&told).1, began(Vec2::Y));
}
